use crate::{Group, Performance, Stock};
use anyhow::Context;
use chrono::{DateTime, Local, TimeDelta};
use sqlx::sqlite::{SqliteAutoVacuum, SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous};
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use std::collections::HashMap;

use crate::util::is_upto_date;
use log::warn;
use std::sync::{Arc, LazyLock, Weak};
use tokio::sync::Mutex;

const DB_FILE: &str = "database.sqlite";

static INSTANCE: LazyLock<Mutex<Weak<Store>>> = LazyLock::new(|| Mutex::new(Weak::new()));

pub struct Store {
    pool: SqlitePool,
}

impl Store {
    pub async fn load_store() -> anyhow::Result<Arc<Store>> {
        let mut weak = INSTANCE.lock().await;
        if let Some(arc) = weak.upgrade() {
            return Ok(arc);
        }

        let options = SqliteConnectOptions::new()
            .filename(DB_FILE)
            .create_if_missing(true)
            .auto_vacuum(SqliteAutoVacuum::Full)
            .journal_mode(SqliteJournalMode::Wal) // concurrent reads + writes
            .synchronous(SqliteSynchronous::Normal) // safe with WAL, much faster than Full
            .pragma("cache_size", "-65536") // 64MB page cache (negative = KiB)
            .pragma("temp_store", "memory") // temp tables in RAM
            .pragma("mmap_size", "268435456") // 256MB memory-mapped I/O
            .pragma("busy_timeout", "5000"); // wait 5s instead of immediately erroring on lock

        let pool = SqlitePoolOptions::new()
            .max_connections(4) // WAL supports multiple readers
            .connect_with(options)
            .await
            .with_context(|| format!("Failed to open SQLite database: {DB_FILE}"))?;

        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .context("Failed to run database migrations")?;

        // Evict stale stock rows for this source (older than 30 days)
        let cutoff = Local::now().date_naive() - TimeDelta::days(30);
        sqlx::query!("DELETE FROM stocks WHERE last_update < $1", cutoff)
            .execute(&pool)
            .await
            .context("Failed to evict stale stock rows")?;

        let store = Arc::new(Store { pool });
        *weak = Arc::downgrade(&store);

        Ok(store)
    }

    fn source(use_tv: bool) -> &'static str {
        if use_tv {
            "TradingView"
        } else {
            "YahooFinance"
        }
    }
    // ── Stock methods ────────────────────────────────────────────────────────

    pub async fn get_stock(
        &self,
        ticker: impl AsRef<str>,
        use_tv: bool,
    ) -> anyhow::Result<Option<Stock>> {
        let ticker = ticker.as_ref();
        let source = Self::source(use_tv);

        let row = sqlx::query!(
            "SELECT ticker, exchange, sector_name, sector_url, industry_name, industry_url, last_update
             FROM stocks
             WHERE source = $1 AND ticker = $2",
            source,
            ticker,
        )
        .fetch_optional(&self.pool)
        .await
        .with_context(|| format!("Failed to query stock: {ticker}"))?;

        Ok(row.map(|r| Stock {
            ticker: r.ticker,
            exchange: r.exchange,
            sector: Group {
                name: r.sector_name,
                url: r.sector_url,
            },
            industry: Group {
                name: r.industry_name,
                url: r.industry_url,
            },
            last_update: r.last_update,
        }))
    }

    pub async fn add_stocks(&self, stocks: &[Stock], is_tv: bool) -> anyhow::Result<()> {
        let mut tx = self
            .pool
            .begin()
            .await
            .context("Failed to begin transaction")?;
        for stock in stocks {
            let source = Self::source(is_tv);
            sqlx::query!(
                r"INSERT INTO stocks
                    (source, ticker, exchange, sector_name, sector_url,
                     industry_name, industry_url, last_update)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                 ON CONFLICT(source, ticker) DO UPDATE SET
                    exchange      = $3,
                    sector_name   = $4,
                    sector_url    = $5,
                    industry_name = $6,
                    industry_url  = $7,
                    last_update   = $8
                ",
                source,
                stock.ticker,
                stock.exchange,
                stock.sector.name,
                stock.sector.url,
                stock.industry.name,
                stock.industry.url,
                stock.last_update,
            )
            .execute(&mut *tx)
            .await
            .with_context(|| format!("Failed to upsert stock: {}", stock.ticker))?;
        }
        tx.commit()
            .await
            .context("Failed to commit stock transaction")
    }

    pub async fn save_performance(&self, perf: &Performance) -> sqlx::Result<()> {
        sqlx::query!(
                r#"
                INSERT INTO performance (ticker, perf_1m, perf_3m, perf_6m, perf_1y, last_updated, extra_info)
                VALUES ($1, $2, $3, $4, $5, $6, $7)
                ON CONFLICT(ticker) DO UPDATE SET
                    perf_1m      = excluded.perf_1m,
                    perf_3m      = excluded.perf_3m,
                    perf_6m      = excluded.perf_6m,
                    perf_1y      = excluded.perf_1y,
                    last_updated = excluded.last_updated,
                    extra_info   = excluded.extra_info
                "#,
                perf.ticker,
                perf.perf_1m,
                perf.perf_3m,
                perf.perf_6m,
                perf.perf_1y,
                perf.last_updated,
                perf.extra_info,
            )
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_performance(&self, ticker: &str) -> sqlx::Result<Option<Performance>> {
        let result = sqlx::query_as!(
            Performance,
            r#"
                SELECT
                    ticker,
                    perf_1m,
                    perf_3m,
                    perf_6m,
                    perf_1y,
                    last_updated as "last_updated: DateTime<Local>",
                    extra_info as "extra_info: sqlx::types::Json<HashMap<String, f64>>"
                FROM performance
                WHERE ticker = $1
            "#,
            ticker,
        )
        .fetch_optional(&self.pool)
        .await?;
        if let Some(perf) = result
            && is_upto_date(perf.last_updated)
        {
            return Ok(Some(perf));
        }
        Ok(None)
    }

    pub async fn get_all_performances(&self) -> sqlx::Result<Vec<Performance>> {
        let result = sqlx::query_as!(
            Performance,
            r#"
                SELECT
                    ticker,
                    perf_1m,
                    perf_3m,
                    perf_6m,
                    perf_1y,
                    last_updated as "last_updated: DateTime<Local>",
                    extra_info as "extra_info: sqlx::types::Json<HashMap<String, f64>>"
                FROM performance
                ORDER BY ticker
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(result
            .into_iter()
            .filter(|p| is_upto_date(p.last_updated))
            .collect())
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        let pool = self.pool.clone();
        tokio::spawn(async move {
            if let Err(e) = sqlx::query!("VACUUM").execute(&pool).await {
                warn!("Failed to VACUUM on Store drop: {e}");
            }
        });
    }
}
