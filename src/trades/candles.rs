use anyhow::Context;
use chrono::{DateTime, Datelike, NaiveTime, TimeDelta, Utc, Weekday};
use tracing::{info, warn};

use crate::store::Store;
use crate::yf::{BarSize, TimeSpec, YFinance};

/// Maximum look-back Yahoo Finance supports for hourly candles
pub const HOURLY_MAX_LOOKBACK_DAYS: i64 = 200;
const TIME_FMT: &str = "%Y-%m-%d %H:%M";
const MID_NIGHT: NaiveTime = NaiveTime::from_hms_opt(0, 0, 0).unwrap();

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
    let from = from.with_time(MID_NIGHT).unwrap();
    let to = (to + TimeDelta::days(1)).with_time(MID_NIGHT).unwrap();
    let hourly_limit =
        Utc::now().with_time(MID_NIGHT).unwrap() - TimeDelta::days(HOURLY_MAX_LOOKBACK_DAYS);

    let stored = store.get_hourly_candles(ticker, from, to).await?;
    if has_enough_candles(from.max(hourly_limit), to, stored.len()) {
        return Ok(stored);
    }

    if !stored.is_empty() {
        warn!(ticker=%ticker, "[{} -> {}] candles found {}", from.format(TIME_FMT), to.format(TIME_FMT), stored.len());
    }
    let effective_from = stored
        .last()
        .map(|c| c.timestamp.with_time(MID_NIGHT).unwrap() - TimeDelta::days(3))
        .unwrap_or(hourly_limit)
        .max(hourly_limit);
    let effective_to = Utc::now();
    info!(ticker=%ticker, "Fetching hourly candles [{} → {}]", effective_from.format(TIME_FMT), effective_to.format(TIME_FMT));
    let candles = yf
        .fetch_candles(
            ticker,
            BarSize::Hour1,
            TimeSpec::Interval(effective_from, effective_to),
        )
        .await
        .with_context(|| format!("Failed to fetch hourly candles for {ticker}"))?;
    info!("Fetched {} hourly candles for {ticker}", candles.len());
    store
        .save_hourly_candles(ticker, &candles)
        .await
        .with_context(|| format!("Failed to save hourly candles for {ticker}"))?;

    Ok(store.get_hourly_candles(ticker, from, to).await?)
}

fn has_enough_candles(start: DateTime<Utc>, end: DateTime<Utc>, candles: usize) -> bool {
    let today = Utc::now().date_naive();
    let mut day = start.date_naive();
    let mut work_days = 0;
    while day < end.date_naive() && day <= today {
        if !matches!(day.weekday(), Weekday::Sat | Weekday::Sun) {
            work_days += 1;
        }
        day += TimeDelta::days(1);
    }

    let expected_candles = work_days * 7;
    candles as f64 >= (expected_candles as f64) * 0.9
}
