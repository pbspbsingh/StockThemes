use super::*;
use crate::{Performance, TickerType};
use chrono::{TimeZone, Utc};

#[tokio::test]
async fn test_crumb() -> anyhow::Result<()> {
    let yf = YFinance::new();
    let crumb = yf.crumb().await?;
    assert!(!crumb.is_empty(), "Expected a non-empty crumb");
    eprintln!("crumb = {crumb:?}");
    Ok(())
}

#[tokio::test]
async fn test_crumb_is_reused() -> anyhow::Result<()> {
    let yf = YFinance::new();
    eprintln!("{:?}", yf.fetch_ticker_info("AAPL").await?);
    eprintln!("{:?}", yf.fetch_ticker_info("JPM").await?);
    Ok(())
}

/// Daily candles via a named range — basic sanity check.
#[tokio::test]
async fn test_fetch_candles_range() -> anyhow::Result<()> {
    let yf = YFinance::new();
    let candles = yf
        .fetch_candles("AAPL", BarSize::Daily, TimeSpec::Range(Range::OneMonth))
        .await?;

    assert!(!candles.is_empty(), "Expected at least one candle");

    // Timestamps must be strictly ascending.
    for w in candles.windows(2) {
        assert!(w[0].timestamp < w[1].timestamp, "Timestamps out of order");
    }

    // Spot-check field ranges.
    for c in &candles {
        assert!(c.open > 0.0);
        assert!(c.high >= c.open.max(c.close));
        assert!(c.low <= c.open.min(c.close));
        assert!(c.volume > 0);
        // Daily bars must always have an adj_close.
        assert!(c.adj_close.is_some(), "Expected adj_close for daily bar");
    }

    eprintln!(
        "AAPL 1mo daily — {} candles, first: {:?}",
        candles.len(),
        candles[0]
    );
    Ok(())
}

/// adj_close is None for intraday bars.
#[tokio::test]
async fn test_adj_close_absent_for_intraday() -> anyhow::Result<()> {
    let yf = YFinance::new();
    let candles = yf
        .fetch_candles("AAPL", BarSize::Min5, TimeSpec::Range(Range::OneDay))
        .await?;

    assert!(!candles.is_empty());
    for c in &candles {
        assert!(
            c.adj_close.is_none(),
            "Expected no adj_close for intraday bar, got {:?}",
            c.adj_close
        );
    }
    Ok(())
}

/// Intraday candles — 5-minute bars over the last day, regular session.
#[tokio::test]
async fn test_fetch_candles_intraday() -> anyhow::Result<()> {
    let yf = YFinance::new();
    let candles = yf
        .fetch_candles("QQQ", BarSize::Min5, TimeSpec::Range(Range::OneDay))
        .await?;

    assert!(!candles.is_empty(), "Expected intraday candles");

    for w in candles.windows(2) {
        assert!(w[0].timestamp < w[1].timestamp);
    }

    eprintln!(
        "QQQ 1d 5m — {} candles, first: {:?}",
        candles.len(),
        candles[0]
    );
    Ok(())
}

/// Extended hours should return at least as many candles as regular session.
#[tokio::test]
async fn test_fetch_candles_intraday_extended() -> anyhow::Result<()> {
    let yf = YFinance::new();
    let regular = yf
        .fetch_candles("QQQ", BarSize::Min5, TimeSpec::Range(Range::OneDay))
        .await?;
    let extended = yf
        .fetch_candles("QQQ", BarSize::Min5Ext, TimeSpec::Range(Range::OneDay))
        .await?;

    assert!(
        extended.len() >= regular.len(),
        "Extended session ({}) should have at least as many candles as regular ({})",
        extended.len(),
        regular.len(),
    );

    eprintln!(
        "QQQ regular: {} candles, extended: {} candles",
        regular.len(),
        extended.len()
    );
    Ok(())
}

/// Hourly candles for one regular trading day — expect exactly 7.
/// NYSE regular session 9:30–16:00 produces bars at :30 past each hour,
/// last bar at 15:30. That's 7 bars: 9:30 10:30 11:30 12:30 13:30 14:30 15:30.
#[tokio::test]
async fn test_candle_count_hourly_one_day() -> anyhow::Result<()> {
    let yf = YFinance::new();
    // 2024-03-06 is a plain Wednesday with no early close, before DST (Mar 10).
    let start = Utc.with_ymd_and_hms(2024, 3, 6, 14, 30, 0).unwrap(); // 9:30 ET
    let end = Utc.with_ymd_and_hms(2024, 3, 6, 21, 0, 0).unwrap(); // 16:00 ET

    let candles = yf
        .fetch_candles("SPY", BarSize::Hour1, TimeSpec::Interval(start, end))
        .await?;

    assert_eq!(
        candles.len(),
        7,
        "Expected 7 hourly bars for a regular session, got {}",
        candles.len()
    );
    Ok(())
}

/// Hourly candles over an explicit intraday DateTime window.
#[tokio::test]
async fn test_fetch_candles_datetime_period() -> anyhow::Result<()> {
    let yf = YFinance::new();
    let start = Utc.with_ymd_and_hms(2024, 6, 3, 13, 30, 0).unwrap(); // 9:30 AM ET
    let end = Utc.with_ymd_and_hms(2024, 6, 7, 20, 0, 0).unwrap(); // 4:00 PM ET

    let candles = yf
        .fetch_candles("TSLA", BarSize::Hour1, TimeSpec::Interval(start, end))
        .await?;

    assert!(!candles.is_empty());
    assert!(candles.first().unwrap().timestamp >= start);
    assert!(candles.last().unwrap().timestamp <= end);

    eprintln!(
        "TSLA hourly over explicit window — {} candles",
        candles.len()
    );
    Ok(())
}

#[tokio::test]
async fn test_spy_candles_with_adj_close() -> anyhow::Result<()> {
    let yf = YFinance::new();
    let ticker = "IWM";
    let candles = yf
        .fetch_candles(ticker, BarSize::Daily, TimeSpec::Range(Range::TwoYears))
        .await?;
    eprintln!("Total {ticker} candles: {}", candles.len());
    eprintln!("First: {}", candles.first().unwrap());
    eprintln!("Last: {}", candles.last().unwrap());

    let perf = Performance::compute(ticker, TickerType::Stock, &candles);
    eprintln!("Performance: {perf}");
    Ok(())
}

/// Verify that 429 surfaces as YfError::RateLimited and is downcasable.
#[test]
fn test_rate_limit_error_is_downcatable() {
    let err: anyhow::Error = YfError::RateLimited.into();
    assert!(err.downcast_ref::<YfError>().is_some());
}
