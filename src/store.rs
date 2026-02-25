use crate::{Group, Stock, yf::Candle};
use anyhow::Context;
use chrono::{DateTime, Local, TimeDelta, Utc};
use sqlx::sqlite::{SqliteAutoVacuum, SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous};
use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};

const DB_FILE: &str = "database.sqlite";

pub struct Store {
    pool: SqlitePool,
    source: String,
}

impl Store {
    pub async fn load_store(use_tv: bool) -> anyhow::Result<Store> {
        let source = if use_tv {
            String::from("TradingView")
        } else {
            String::from("YahooFinance")
        };

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
        sqlx::query!("DELETE FROM stocks WHERE last_update < ?", cutoff)
            .execute(&pool)
            .await
            .context("Failed to evict stale stock rows")?;

        Ok(Store { pool, source })
    }

    // ── Stock methods ────────────────────────────────────────────────────────

    pub async fn get_stock(&self, ticker: impl AsRef<str>) -> anyhow::Result<Option<Stock>> {
        let ticker = ticker.as_ref();
        let source = self.source.as_str();

        let row = sqlx::query!(
            "SELECT ticker, exchange, sector_name, sector_url, industry_name, industry_url, last_update
             FROM stocks
             WHERE source = ? AND ticker = ?",
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

    pub async fn add_stocks(&self, stocks: &[Stock]) -> anyhow::Result<()> {
        let mut tx = self
            .pool
            .begin()
            .await
            .context("Failed to begin transaction")?;
        for stock in stocks {
            let source = &self.source;
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

    // ── Candle methods ───────────────────────────────────────────────────────

    /// Insert or replace candles for a given ticker.
    pub async fn add_candles(
        &self,
        ticker: impl AsRef<str>,
        candles: &[Candle],
    ) -> anyhow::Result<()> {
        let ticker = ticker.as_ref();
        let mut tx = self
            .pool
            .begin()
            .await
            .context("Failed to begin transaction")?;

        for candle in candles {
            let volume = candle.volume as i64;
            sqlx::query!(
                r"INSERT INTO candles (ticker, timestamp, open, high, low, close, volume)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)
                 ON CONFLICT(ticker, timestamp) DO UPDATE SET
                    open   = $3,
                    high   = $4,
                    low    = $5,
                    close  = $6,
                    volume = $7
                ",
                ticker,
                candle.timestamp,
                candle.open,
                candle.high,
                candle.low,
                candle.close,
                volume
            )
            .execute(&mut *tx)
            .await
            .with_context(|| {
                format!(
                    "Failed to upsert candle for {} at {}",
                    ticker, candle.timestamp,
                )
            })?;
        }

        tx.commit()
            .await
            .context("Failed to commit candle transaction")
    }

    /// Fetch candles for a ticker within [from, to] (inclusive).
    pub async fn get_candles(
        &self,
        ticker: impl AsRef<str>,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> anyhow::Result<Vec<Candle>> {
        let ticker = ticker.as_ref();
        let from_ts = from.timestamp();
        let to_ts = to.timestamp();

        let rows = sqlx::query!(
            "SELECT timestamp, open, high, low, close, volume
             FROM candles
             WHERE ticker = ? AND timestamp BETWEEN ? AND ?
             ORDER BY timestamp ASC",
            ticker,
            from_ts,
            to_ts
        )
        .fetch_all(&self.pool)
        .await
        .with_context(|| format!("Failed to fetch candles for {ticker}"))?;

        Ok(rows
            .into_iter()
            .map(|r| Candle {
                timestamp: r.timestamp.and_utc(),
                open: r.open,
                high: r.high,
                low: r.low,
                close: r.close,
                volume: r.volume as u64,
            })
            .collect())
    }

    /// Fetch the most recent `limit` candles for a ticker.
    pub async fn get_latest_candles(
        &self,
        ticker: impl AsRef<str>,
        limit: i64,
    ) -> anyhow::Result<Vec<Candle>> {
        let ticker = ticker.as_ref();

        let mut candles = sqlx::query!(
            "SELECT timestamp, open, high, low, close, volume
             FROM candles
             WHERE ticker = ?
             ORDER BY timestamp DESC
             LIMIT ?",
            ticker,
            limit
        )
        .fetch_all(&self.pool)
        .await
        .with_context(|| format!("Failed to fetch latest candles for {ticker}"))?
        .into_iter()
        .map(|r| Candle {
            timestamp: r.timestamp.and_utc(),
            open: r.open,
            high: r.high,
            low: r.low,
            close: r.close,
            volume: r.volume as u64,
        })
        .collect::<Vec<_>>();

        // Re-sort ascending after the DESC fetch used for LIMIT efficiency
        candles.sort_unstable_by_key(|c| c.timestamp);
        Ok(candles)
    }
}
