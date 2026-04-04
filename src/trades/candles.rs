use anyhow::Context;
use chrono::{DateTime, TimeDelta, Utc};
use tracing::info;

use crate::store::Store;
use crate::yf::{BarSize, TimeSpec, YFinance};

/// Maximum look-back Yahoo Finance supports for hourly candles (~60 days).
const HOURLY_MAX_LOOKBACK_DAYS: i64 = 60;

/// Lazily loads hourly candles for `ticker` within `[from, to]`.
/// Returns cached candles if already present, otherwise fetches from YF and saves.
/// Skips ranges older than Yahoo's hourly look-back limit (returns empty vec).
pub async fn fetch_hourly_candles(
    store: &Store,
    yf: &YFinance,
    ticker: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> anyhow::Result<Vec<crate::yf::Candle>> {
    let hourly_limit = Utc::now() - TimeDelta::days(HOURLY_MAX_LOOKBACK_DAYS);
    let effective_from = from.max(hourly_limit);
    if effective_from >= to {
        return Ok(vec![]);
    }

    let stored = store.get_hourly_candles(ticker, effective_from, to).await?;
    let covered = stored
        .first()
        .map(|c| c.timestamp <= effective_from + TimeDelta::days(1))
        .unwrap_or(false);
    if covered {
        return Ok(stored);
    }

    info!(
        "Fetching hourly candles for {ticker} [{} → {}]",
        effective_from.format("%Y-%m-%d %H:%M"),
        to.format("%Y-%m-%d %H:%M")
    );
    let candles = yf
        .fetch_candles(
            ticker,
            BarSize::Hour1,
            TimeSpec::Interval(effective_from, to),
        )
        .await
        .with_context(|| format!("Failed to fetch hourly candles for {ticker}"))?;
    info!("Fetched {} hourly candles for {ticker}", candles.len());
    store
        .save_hourly_candles(ticker, &candles)
        .await
        .with_context(|| format!("Failed to save hourly candles for {ticker}"))?;

    Ok(candles)
}
