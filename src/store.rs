use crate::{Group, Performance, Stock, TickerType};
use anyhow::Context;
use chrono::{DateTime, Local, TimeDelta, Utc};
use sqlx::sqlite::{SqliteAutoVacuum, SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous};
use sqlx::{
    Decode, Encode, Sqlite, SqlitePool, Type, encode::IsNull, error::BoxDynError,
    sqlite::SqlitePoolOptions,
};
use std::collections::HashMap;

use crate::util::is_upto_date;
use crate::yf::Candle;
use std::sync::{Arc, LazyLock, Weak};
use tokio::sync::Mutex;

const DB_FILE: &str = "database.sqlite";

static INSTANCE: LazyLock<Mutex<Weak<Store>>> = LazyLock::new(|| Mutex::new(Weak::new()));

// ── Store ────────────────────────────────────────────────────────────────────

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

        // Evict stale stock rows older than 30 days
        let cutoff = Local::now().date_naive() - TimeDelta::days(30);
        sqlx::query!("DELETE FROM stocks WHERE last_update < $1", cutoff)
            .execute(&pool)
            .await
            .context("Failed to evict stale stock rows")?;

        let store = Arc::new(Store { pool });
        *weak = Arc::downgrade(&store);

        Ok(store)
    }

    // ── Stock methods ────────────────────────────────────────────────────────

    pub async fn get_stock(&self, ticker: impl AsRef<str>) -> anyhow::Result<Option<Stock>> {
        let ticker = ticker.as_ref();
        let row = sqlx::query!(
            "SELECT ticker, exchange, sector_name, sector_url, industry_name, industry_url, last_update
             FROM stocks
             WHERE ticker = $1",
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

    pub async fn add_stocks(&self, stocks: &[Stock]) -> sqlx::Result<()> {
        let mut tx = self.pool.begin().await?;

        for stock in stocks {
            sqlx::query!(
                r"INSERT INTO stocks
                    (ticker, exchange, sector_name, sector_url,
                     industry_name, industry_url, last_update)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)
                 ON CONFLICT(ticker) DO UPDATE SET
                    exchange      = excluded.exchange,
                    sector_name   = excluded.sector_name,
                    sector_url    = excluded.sector_url,
                    industry_name = excluded.industry_name,
                    industry_url  = excluded.industry_url,
                    last_update   = excluded.last_update",
                stock.ticker,
                stock.exchange,
                stock.sector.name,
                stock.sector.url,
                stock.industry.name,
                stock.industry.url,
                stock.last_update,
            )
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await
    }

    // ── Performance methods ──────────────────────────────────────────────────

    pub async fn save_performances(&self, perfs: &[Performance]) -> sqlx::Result<()> {
        let mut tx = self.pool.begin().await?;
        for perf in perfs {
            sqlx::query!(
                r#"
                INSERT INTO performance (ticker, ticker_type, perf_1m, perf_3m, perf_6m, perf_1y, last_updated, extra_info)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                ON CONFLICT(ticker, ticker_type) DO UPDATE SET
                    perf_1m      = excluded.perf_1m,
                    perf_3m      = excluded.perf_3m,
                    perf_6m      = excluded.perf_6m,
                    perf_1y      = excluded.perf_1y,
                    extra_info   = excluded.extra_info,
                    last_updated = excluded.last_updated
                "#,
                perf.ticker,
                perf.ticker_type,
                perf.perf_1m,
                perf.perf_3m,
                perf.perf_6m,
                perf.perf_1y,
                perf.last_updated,
                perf.extra_info,
                )
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await
    }

    pub async fn get_performance(
        &self,
        ticker: &str,
        ticker_type: TickerType,
    ) -> sqlx::Result<Option<Performance>> {
        let result = sqlx::query_as!(
            Performance,
            r#"
            SELECT
                ticker,
                ticker_type  as "ticker_type: TickerType",
                perf_1m,
                perf_3m,
                perf_6m,
                perf_1y,
                last_updated as "last_updated: DateTime<Local>",
                extra_info   as "extra_info: sqlx::types::Json<HashMap<String, f64>>"
            FROM performance
            WHERE ticker = $1 AND ticker_type = $2
            "#,
            ticker,
            ticker_type,
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
                ticker_type  as "ticker_type: TickerType",
                perf_1m,
                perf_3m,
                perf_6m,
                perf_1y,
                last_updated as "last_updated: DateTime<Local>",
                extra_info   as "extra_info: sqlx::types::Json<HashMap<String, f64>>"
            FROM performance
            ORDER BY ticker_type, ticker
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(result
            .into_iter()
            .filter(|p| is_upto_date(p.last_updated))
            .collect())
    }

    pub async fn get_performances_by_type(
        &self,
        ticker_type: TickerType,
    ) -> sqlx::Result<Vec<Performance>> {
        let result = sqlx::query_as!(
            Performance,
            r#"
            SELECT
                ticker,
                ticker_type  as "ticker_type: TickerType",
                perf_1m,
                perf_3m,
                perf_6m,
                perf_1y,
                last_updated as "last_updated: DateTime<Local>",
                extra_info   as "extra_info: sqlx::types::Json<HashMap<String, f64>>"
            FROM performance
            WHERE ticker_type = $1
            ORDER BY ticker
            "#,
            ticker_type,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(result
            .into_iter()
            .filter(|p| is_upto_date(p.last_updated))
            .collect())
    }

    pub async fn get_candles(&self, ticker: &str) -> sqlx::Result<Vec<Candle>> {
        let one_year_ago = Utc::now() - TimeDelta::days(2 * 365);
        let rows = sqlx::query!(
            r#"
                SELECT ds as "ds: DateTime<Utc>",
                       open,
                       high,
                       low,
                       close,
                       volume,
                       last_updated as "last_updated: DateTime<Local>"
                FROM daily_candles
                WHERE ticker = $1 AND ds >= $2
                ORDER BY ds ASC
            "#,
            ticker,
            one_year_ago,
        )
        .map(|row| Candle {
            timestamp: row.ds,
            open: row.open,
            high: row.high,
            low: row.low,
            close: row.close,
            volume: row.volume as u64,
            last_updated: row.last_updated,
        })
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn save_candles(&self, ticker: &str, candles: &[Candle]) -> sqlx::Result<()> {
        let mut tx = self.pool.begin().await?;
        for candle in candles {
            let volume = candle.volume as i64;
            sqlx::query!(
                r#"
                    INSERT INTO daily_candles (ticker, ds, open, high, low, close, volume, last_updated)
                    VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                    ON CONFLICT(ticker, ds) DO UPDATE SET
                        open = excluded.open,
                        high = excluded.high,
                        low = excluded.low,
                        close = excluded.close,
                        volume = excluded.volume,
                        last_updated = excluded.last_updated
                "#,
                ticker,
                candle.timestamp,
                candle.open,
                candle.high,
                candle.low,
                candle.close,
                volume,
                candle.last_updated,
            )
            .execute(&mut *tx) // Execute on the transaction
            .await?;
        }
        tx.commit().await
    }
}

// ── TickerType <-> SQLite ────────────────────────────────────────────────────

impl Type<Sqlite> for TickerType {
    fn type_info() -> sqlx::sqlite::SqliteTypeInfo {
        <str as Type<Sqlite>>::type_info()
    }
}

impl Encode<'_, Sqlite> for TickerType {
    fn encode_by_ref(
        &self,
        buf: &mut Vec<sqlx::sqlite::SqliteArgumentValue<'_>>,
    ) -> Result<IsNull, BoxDynError> {
        let s = match self {
            TickerType::Sector => "Sector",
            TickerType::Industry => "Industry",
            TickerType::Stock => "Stock",
        };
        Encode::<Sqlite>::encode_by_ref(&s, buf)
    }
}

impl<'r> Decode<'r, Sqlite> for TickerType {
    fn decode(value: sqlx::sqlite::SqliteValueRef<'r>) -> Result<Self, BoxDynError> {
        let s = <&str as Decode<Sqlite>>::decode(value)?;
        match s {
            "Sector" => Ok(TickerType::Sector),
            "Industry" => Ok(TickerType::Industry),
            "Stock" => Ok(TickerType::Stock),
            other => Err(format!("Unknown TickerType: {other}").into()),
        }
    }
}
