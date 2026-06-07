use super::*;
use crate::{Performance, TickerType};

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

#[tokio::test]
async fn test_fetch_company_profile() -> anyhow::Result<()> {
    let yf = YFinance::new();
    let profile = yf.fetch_company_profile("AAPL").await?;

    assert_eq!(profile.symbol, "AAPL");
    assert!(profile.summary.is_some());
    eprintln!("{profile:?}");

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

/// Verify that 404 surfaces as YfError::NotFound and is downcasable.
#[test]
fn test_not_found_error_is_downcatable() {
    let err: anyhow::Error = YfError::NotFound {
        url: "https://query1.finance.yahoo.com/v8/finance/chart/MISSING".to_string(),
    }
    .into();
    assert!(matches!(
        err.downcast_ref::<YfError>(),
        Some(YfError::NotFound { .. })
    ));
}
