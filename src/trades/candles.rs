use anyhow::Context;
use chrono::{DateTime, NaiveDate, TimeDelta, Utc};
use tracing::{info, warn};
use std::sync::Arc;

use crate::store::Store;
use crate::yf::{BarSize, Range, TimeSpec, YFinance};

/// Maximum look-back Yahoo Finance supports for hourly candles (~60 days).
const HOURLY_MAX_LOOKBACK_DAYS: i64 = 60;

/// Calendar-day gap threshold above which we consider data missing.
/// Accounts for weekends (2 days) + public holidays (1 day buffer).
const DAILY_GAP_THRESHOLD_DAYS: i64 = 5;
const HOURLY_GAP_THRESHOLD_DAYS: i64 = 3;

// ── Daily candles ─────────────────────────────────────────────────────────────

/// Ensures contiguous daily candles exist in the DB for `[from, to]`.
/// Detects internal gaps and fetches only the missing segments.
pub async fn ensure_daily_candles(
    store: &Arc<Store>,
    yf: &Arc<YFinance>,
    ticker: &str,
    from: NaiveDate,
    to: NaiveDate,
) -> anyhow::Result<()> {
    let stored = store
        .daily_candle_dates(ticker, from, to)
        .await
        .with_context(|| format!("Failed to query daily candle dates for {ticker}"))?;

    let gaps = detect_date_gaps(&stored, from, to, DAILY_GAP_THRESHOLD_DAYS);

    if gaps.is_empty() {
        return Ok(());
    }

    for (gap_start, gap_end) in gaps {
        fetch_and_save_daily(store, yf, ticker, gap_start, gap_end).await?;
    }

    Ok(())
}

async fn fetch_and_save_daily(
    store: &Arc<Store>,
    yf: &Arc<YFinance>,
    ticker: &str,
    from: NaiveDate,
    to: NaiveDate,
) -> anyhow::Result<()> {
    // Overlap by 1 day on each side to avoid edge gaps
    let start = (from - TimeDelta::days(1))
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc();
    let end = (to + TimeDelta::days(1))
        .and_hms_opt(23, 59, 59)
        .unwrap()
        .and_utc();

    info!("Fetching daily candles for {ticker} [{from} → {to}]");

    let candles = yf
        .fetch_candles(ticker, BarSize::Daily, TimeSpec::Interval(start, end))
        .await
        .with_context(|| format!("Failed to fetch daily candles for {ticker}"))?;

    info!("Fetched {} daily candles for {ticker}", candles.len());
    store
        .save_candles(ticker, &candles)
        .await
        .with_context(|| format!("Failed to save daily candles for {ticker}"))?;

    Ok(())
}

// ── Hourly candles ────────────────────────────────────────────────────────────

/// Ensures contiguous hourly candles exist in the DB for `[from, to]`.
/// Skips gaps older than Yahoo's hourly look-back limit.
pub async fn ensure_hourly_candles(
    store: &Arc<Store>,
    yf: &Arc<YFinance>,
    ticker: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> anyhow::Result<()> {
    let hourly_limit = Utc::now() - TimeDelta::days(HOURLY_MAX_LOOKBACK_DAYS);

    // Clamp the requested range to what Yahoo can provide
    let effective_from = from.max(hourly_limit);
    if effective_from >= to {
        warn!(
            "Hourly candles for {ticker} requested before Yahoo's {HOURLY_MAX_LOOKBACK_DAYS}-day limit — skipping"
        );
        return Ok(());
    }

    let stored = store
        .hourly_candle_hours(ticker, effective_from, to)
        .await
        .with_context(|| format!("Failed to query hourly candle hours for {ticker}"))?;

    let gaps = detect_datetime_gaps(
        &stored,
        effective_from,
        to,
        HOURLY_GAP_THRESHOLD_DAYS,
        hourly_limit,
    );

    if gaps.is_empty() {
        return Ok(());
    }

    for (gap_start, gap_end) in gaps {
        fetch_and_save_hourly(store, yf, ticker, gap_start, gap_end).await?;
    }

    Ok(())
}

async fn fetch_and_save_hourly(
    store: &Arc<Store>,
    yf: &Arc<YFinance>,
    ticker: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> anyhow::Result<()> {
    let start = from - TimeDelta::hours(1);
    let end = to + TimeDelta::hours(1);

    info!(
        "Fetching hourly candles for {ticker} [{} → {}]",
        start.format("%Y-%m-%d %H:%M"),
        end.format("%Y-%m-%d %H:%M")
    );

    let candles = yf
        .fetch_candles(ticker, BarSize::Hour1, TimeSpec::Interval(start, end))
        .await
        .with_context(|| format!("Failed to fetch hourly candles for {ticker}"))?;

    info!("Fetched {} hourly candles for {ticker}", candles.len());
    store
        .save_hourly_candles(ticker, &candles)
        .await
        .with_context(|| format!("Failed to save hourly candles for {ticker}"))?;

    Ok(())
}

// ── Gap detection ─────────────────────────────────────────────────────────────

/// Returns `(gap_start, gap_end)` pairs for missing date ranges within `[from, to]`.
fn detect_date_gaps(
    stored: &[NaiveDate],
    from: NaiveDate,
    to: NaiveDate,
    threshold_days: i64,
) -> Vec<(NaiveDate, NaiveDate)> {
    let mut gaps = Vec::new();

    if stored.is_empty() {
        gaps.push((from, to));
        return gaps;
    }

    // Gap at the start
    if (stored[0] - from).num_days() > threshold_days {
        gaps.push((from, stored[0]));
    }

    // Internal gaps
    for window in stored.windows(2) {
        let (prev, next) = (window[0], window[1]);
        if (next - prev).num_days() > threshold_days {
            gaps.push((prev, next));
        }
    }

    // Gap at the end
    let last = *stored.last().unwrap();
    if (to - last).num_days() > threshold_days {
        gaps.push((last, to));
    }

    gaps
}

/// Returns `(gap_start, gap_end)` pairs for missing datetime ranges within `[from, to]`,
/// skipping any gap that starts before `hourly_limit`.
fn detect_datetime_gaps(
    stored: &[DateTime<Utc>],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    threshold_days: i64,
    hourly_limit: DateTime<Utc>,
) -> Vec<(DateTime<Utc>, DateTime<Utc>)> {
    let threshold = TimeDelta::days(threshold_days);
    let mut gaps = Vec::new();

    if stored.is_empty() {
        gaps.push((from, to));
        return gaps;
    }

    if stored[0] - from > threshold {
        gaps.push((from, stored[0]));
    }

    for window in stored.windows(2) {
        let (prev, next) = (window[0], window[1]);
        if next - prev > threshold {
            if next > hourly_limit {
                gaps.push((prev, next));
            }
        }
    }

    let last = *stored.last().unwrap();
    if to - last > threshold {
        gaps.push((last, to));
    }

    gaps
}

// ── Bulk prefetch ─────────────────────────────────────────────────────────────

/// Prefetch daily + hourly candles for all tickers (including benchmark),
/// covering every trade's chart window. Called once on startup.
pub async fn prefetch_all(
    store: &Arc<Store>,
    yf: &Arc<YFinance>,
    tickers: &[String],
    daily_windows: &[(String, NaiveDate, NaiveDate)],
    hourly_windows: &[(String, DateTime<Utc>, DateTime<Utc>)],
) -> anyhow::Result<()> {
    // Use Range::SixMonths for daily if we have no data at all (faster than many interval calls)
    for ticker in tickers {
        let existing = store.get_candles(ticker).await?;
        if existing.is_empty() {
            info!("No daily candles for {ticker}, fetching SixMonths baseline");
            let candles = yf
                .fetch_candles(ticker, BarSize::Daily, TimeSpec::Range(Range::SixMonths))
                .await
                .with_context(|| format!("Failed to fetch baseline daily candles for {ticker}"))?;
            store.save_candles(ticker, &candles).await?;
        }
    }

    // Now fill any gaps for the precise windows needed
    for (ticker, from, to) in daily_windows {
        ensure_daily_candles(store, yf, ticker, *from, *to).await?;
    }

    for (ticker, from, to) in hourly_windows {
        ensure_hourly_candles(store, yf, ticker, *from, *to).await?;
    }

    Ok(())
}
