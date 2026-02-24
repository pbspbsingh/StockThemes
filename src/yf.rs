use crate::{Group, Stock, StockInfoFetcher};
use anyhow::Context;
use chrono::{DateTime, Local, NaiveDate, TimeZone, Utc};
use futures::{stream, StreamExt};
use reqwest::{header, Client};
use serde::Deserialize;
use std::{collections::HashMap, fmt, time::Duration};
use tokio::sync::OnceCell;

// ============================================================================
// Candle query types
// ============================================================================

/// The size (and session) of each candle bar.
///
/// `*Ext` variants include pre-market and after-hours data
/// (`includePrePost=true`). Only meaningful for intraday bar sizes —
/// Yahoo ignores the flag for `Daily` and `Weekly`.
#[derive(Debug, Clone, Copy)]
pub enum BarSize {
    Min1,
    Min1Ext,
    Min5,
    Min5Ext,
    Min15,
    Min15Ext,
    Min30,
    Min30Ext,
    Hour1,
    Hour1Ext,
    Daily,
    Weekly,
}

impl BarSize {
    fn as_str(self) -> &'static str {
        match self {
            BarSize::Min1 | BarSize::Min1Ext => "1m",
            BarSize::Min5 | BarSize::Min5Ext => "5m",
            BarSize::Min15 | BarSize::Min15Ext => "15m",
            BarSize::Min30 | BarSize::Min30Ext => "30m",
            BarSize::Hour1 | BarSize::Hour1Ext => "1h",
            BarSize::Daily => "1d",
            BarSize::Weekly => "1wk",
        }
    }

    fn include_pre_post(self) -> bool {
        matches!(
            self,
            BarSize::Min1Ext
                | BarSize::Min5Ext
                | BarSize::Min15Ext
                | BarSize::Min30Ext
                | BarSize::Hour1Ext
        )
    }
}

impl fmt::Display for BarSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A predefined lookback window understood natively by Yahoo Finance.
#[derive(Debug, Clone, Copy)]
pub enum Range {
    OneDay,
    FiveDay,
    OneMonth,
    ThreeMonths,
    SixMonths,
    OneYear,
    TwoYears,
    FiveYears,
    TenYears,
    Ytd,
    Max,
}

impl Range {
    fn as_str(self) -> &'static str {
        match self {
            Range::OneDay => "1d",
            Range::FiveDay => "5d",
            Range::OneMonth => "1mo",
            Range::ThreeMonths => "3mo",
            Range::SixMonths => "6mo",
            Range::OneYear => "1y",
            Range::TwoYears => "2y",
            Range::FiveYears => "5y",
            Range::TenYears => "10y",
            Range::Ytd => "ytd",
            Range::Max => "max",
        }
    }
}

/// How to specify the time window for a candle request.
///
/// - `Range` — a named lookback window (e.g. `Range::OneMonth`).
/// - `Interval` — an explicit `[start, end)` window using `DateTime<Utc>`.
///   Use `TimeSpec::from_dates` for day-precision windows without writing
///   out midnight UTC by hand.
#[derive(Debug, Clone, Copy)]
pub enum TimeSpec {
    Range(Range),
    Interval(DateTime<Utc>, DateTime<Utc>),
}

impl TimeSpec {
    /// Convenience constructor for day-precision windows.
    /// Interprets both dates as midnight UTC.
    pub fn from_dates(start: NaiveDate, end: NaiveDate) -> Self {
        TimeSpec::Interval(
            Utc.from_utc_datetime(&start.and_hms_opt(0, 0, 0).unwrap()),
            Utc.from_utc_datetime(&end.and_hms_opt(0, 0, 0).unwrap()),
        )
    }
}

// ============================================================================
// Output type
// ============================================================================

#[derive(Debug, Clone)]
pub struct Candle {
    pub timestamp: DateTime<Utc>,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: u64,
}

// ============================================================================
// Internal deserialization structs for /v8/finance/chart
// ============================================================================

#[derive(Debug, Deserialize)]
struct ChartResponse {
    chart: ChartResult,
}

#[derive(Debug, Deserialize)]
struct ChartResult {
    result: Option<Vec<ChartData>>,
    error: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct ChartData {
    timestamp: Option<Vec<i64>>,
    indicators: Indicators,
}

#[derive(Debug, Deserialize)]
struct Indicators {
    quote: Vec<QuoteIndicator>,
}

#[derive(Debug, Deserialize)]
struct QuoteIndicator {
    open: Option<Vec<Option<f64>>>,
    high: Option<Vec<Option<f64>>>,
    low: Option<Vec<Option<f64>>>,
    close: Option<Vec<Option<f64>>>,
    volume: Option<Vec<Option<u64>>>,
}

// ============================================================================
// QuoteSummary deserialization (existing)
// ============================================================================

#[derive(Debug, Deserialize)]
struct QuoteSummaryResponse {
    #[serde(rename = "quoteSummary")]
    quote_summary: QuoteSummary,
}

#[derive(Debug, Deserialize)]
struct QuoteSummary {
    result: Vec<QuoteSummaryResult>,
}

#[derive(Debug, Deserialize)]
struct QuoteSummaryResult {
    #[serde(rename = "assetProfile")]
    asset_profile: Option<AssetProfile>,
    price: Option<Price>,
}

#[derive(Debug, Deserialize)]
struct AssetProfile {
    sector: Option<String>,
    industry: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Price {
    #[serde(rename = "exchangeName")]
    exchange_name: Option<String>,
    #[serde(rename = "exchangeCode")]
    exchange_code: Option<String>,
}

// ============================================================================
// Output struct (existing)
// ============================================================================

#[derive(Debug)]
pub struct TickerInfo {
    pub symbol: String,
    pub exchange: Option<String>,
    pub exchange_code: Option<String>,
    pub sector: Option<String>,
    pub industry: Option<String>,
}

// ============================================================================
// YFinance client
// ============================================================================

pub struct YFinance {
    client: Client,
    crumb: OnceCell<String>,
}

impl YFinance {
    pub fn new() -> Self {
        let client = Client::builder()
            .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:148.0) Gecko/20100101 Firefox/148.0")
            .cookie_store(true)
            .gzip(true)
            .deflate(true)
            .timeout(Duration::from_secs(10))
            .build()
            .expect("Failed to build HTTP client");

        Self {
            client,
            crumb: OnceCell::new(),
        }
    }

    async fn crumb(&self) -> anyhow::Result<&str> {
        self.crumb
            .get_or_try_init(|| async {
                let crumb = self
                    .client
                    .get("https://query2.finance.yahoo.com/v1/test/getcrumb")
                    .header(
                        header::ACCEPT,
                        "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
                    )
                    .send()
                    .await?
                    .text()
                    .await?;
                if crumb.is_empty() || crumb.contains("Unauthorized") {
                    anyhow::bail!("Failed to obtain Yahoo Finance crumb");
                }
                Ok(crumb)
            })
            .await
            .map(String::as_str)
    }

    pub async fn fetch_ticker_info(&self, symbol: &str) -> anyhow::Result<TickerInfo> {
        let crumb = self.crumb().await?;

        let url = format!(
            "https://query1.finance.yahoo.com/v10/finance/quoteSummary/{symbol}?modules=assetProfile,price&crumb={crumb}"
        );

        let response = self
            .client
            .get(&url)
            .header(header::ACCEPT, "application/json")
            .send()
            .await?
            .error_for_status()?
            .json::<QuoteSummaryResponse>()
            .await?;

        let result = response
            .quote_summary
            .result
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("Empty result from Yahoo Finance"))?;

        Ok(TickerInfo {
            symbol: symbol.to_string(),
            exchange: result.price.as_ref().and_then(|p| p.exchange_name.clone()),
            exchange_code: result.price.as_ref().and_then(|p| p.exchange_code.clone()),
            sector: result.asset_profile.as_ref().and_then(|a| a.sector.clone()),
            industry: result
                .asset_profile
                .as_ref()
                .and_then(|a| a.industry.clone()),
        })
    }

    /// Fetch OHLCV candles for a single symbol.
    pub async fn fetch_candles(
        &self,
        symbol: &str,
        bar: BarSize,
        time: TimeSpec,
    ) -> anyhow::Result<Vec<Candle>> {
        let crumb = self.crumb().await?;
        let pre_post = bar.include_pre_post();

        let url = match time {
            TimeSpec::Range(range) => format!(
                "https://query1.finance.yahoo.com/v8/finance/chart/{symbol}\
                 ?interval={bar}&range={range}&includePrePost={pre_post}&crumb={crumb}",
                range = range.as_str(),
            ),
            TimeSpec::Interval(start, end) => format!(
                "https://query1.finance.yahoo.com/v8/finance/chart/{symbol}\
                 ?interval={bar}&period1={}&period2={}&includePrePost={pre_post}&crumb={crumb}",
                start.timestamp(),
                end.timestamp(),
            ),
        };

        let resp = self
            .client
            .get(&url)
            .header(header::ACCEPT, "application/json")
            .send()
            .await?
            .error_for_status()?
            .json::<ChartResponse>()
            .await?;

        if let Some(err) = resp.chart.error {
            anyhow::bail!("Yahoo Finance chart error for {symbol}: {err}");
        }

        let data = resp
            .chart
            .result
            .and_then(|mut v| v.pop())
            .ok_or_else(|| anyhow::anyhow!("Empty chart result for {symbol}"))?;

        let timestamps = data
            .timestamp
            .ok_or_else(|| anyhow::anyhow!("No timestamps in chart response for {symbol}"))?;

        let quote = data
            .indicators
            .quote
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("No quote indicators for {symbol}"))?;

        let opens = quote.open.unwrap_or_default();
        let highs = quote.high.unwrap_or_default();
        let lows = quote.low.unwrap_or_default();
        let closes = quote.close.unwrap_or_default();
        let volumes = quote.volume.unwrap_or_default();

        let candles: Vec<Candle> = timestamps
            .into_iter()
            .enumerate()
            .filter_map(|(i, ts)| {
                let open   = opens.get(i)?.as_ref()?;
                let high   = highs.get(i)?.as_ref()?;
                let low    = lows.get(i)?.as_ref()?;
                let close  = closes.get(i)?.as_ref()?;
                let volume = volumes.get(i)?.as_ref()?;

                Some(Candle {
                    timestamp: Utc.timestamp_opt(ts, 0).single()?,
                    open: *open,
                    high: *high,
                    low: *low,
                    close: *close,
                    volume: *volume,
                })
            })
            .collect();

        // When an explicit interval was requested, strip any candles Yahoo
        // appended outside the window (e.g. a "current price" sentinel bar).
        let candles = match time {
            TimeSpec::Interval(start, end) => candles
                .into_iter()
                .filter(|c| c.timestamp >= start && c.timestamp <= end)
                .collect(),
            TimeSpec::Range(_) => candles,
        };

        Ok(candles)
    }

    /// Fetch candles for multiple symbols concurrently, throttled to
    /// `max_concurrent` in-flight requests at a time to avoid rate limiting.
    pub async fn fetch_candles_many(
        &self,
        symbols: &[&str],
        bar: BarSize,
        time: TimeSpec,
        max_concurrent: usize,
    ) -> HashMap<String, anyhow::Result<Vec<Candle>>> {
        stream::iter(symbols)
            .map(|&symbol| async move {
                let result = self.fetch_candles(symbol, bar, time).await;
                (symbol.to_owned(), result)
            })
            .buffer_unordered(max_concurrent)
            .collect()
            .await
    }

    fn exchange_map(exchange: String) -> &'static str {
        match exchange.as_ref() {
            "NYSE" => "NYSE",
            "NYSE American" => "ARCA",
            _ if exchange.starts_with("Nasdaq") => "NASDAQ",
            _ if exchange.starts_with("OTC") => "OTC",
            x => panic!("Unknown exchange: '{x}'"),
        }
    }
}

#[async_trait::async_trait]
impl StockInfoFetcher for YFinance {
    async fn fetch(&self, ticker: &str) -> anyhow::Result<Stock> {
        let ti = self.fetch_ticker_info(ticker).await?;
        let exchange = ti
            .exchange
            .with_context(|| format!("Failed to get exchange {ticker}"))?;
        let exchange = Self::exchange_map(exchange);
        let sector = ti
            .sector
            .with_context(|| format!("Failed to get sector {ticker}"))?;
        let industry = ti
            .industry
            .with_context(|| format!("Failed to get industry {ticker}"))?;
        Ok(Stock {
            ticker: ticker.to_owned(),
            exchange: exchange.to_owned(),
            sector: Group {
                name: sector,
                url: String::from("#"),
            },
            industry: Group {
                name: industry,
                url: String::from("#"),
            },
            last_update: Local::now().date_naive(),
        })
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod test {
    use super::*;
    use chrono::NaiveDate;

    #[tokio::test]
    async fn test_ticker_info() -> anyhow::Result<()> {
        let yf = YFinance::new();
        eprintln!("{:?}", yf.crumb);
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
        }

        eprintln!("AAPL 1mo daily — {} candles, first: {:?}", candles.len(), candles[0]);
        Ok(())
    }

    /// Explicit date-range window using `TimeSpec::from_dates`.
    #[tokio::test]
    async fn test_fetch_candles_period() -> anyhow::Result<()> {
        let yf = YFinance::new();
        let start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2024, 3, 31).unwrap();

        let candles = yf
            .fetch_candles("SPY", BarSize::Daily, TimeSpec::from_dates(start, end))
            .await?;

        assert!(!candles.is_empty());

        // All candles must fall within [start, end].
        let start_dt = Utc.from_utc_datetime(&start.and_hms_opt(0, 0, 0).unwrap());
        let end_dt = Utc.from_utc_datetime(&end.and_hms_opt(23, 59, 59).unwrap());
        for c in &candles {
            assert!(
                c.timestamp >= start_dt && c.timestamp <= end_dt,
                "Candle timestamp {} outside requested window",
                c.timestamp
            );
        }

        eprintln!("SPY Q1-2024 daily — {} candles", candles.len());
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

        eprintln!("QQQ 1d 5m — {} candles, first: {:?}", candles.len(), candles[0]);
        Ok(())
    }

    /// Same as above but with extended hours — should return more candles.
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

        eprintln!("QQQ regular: {} candles, extended: {} candles", regular.len(), extended.len());
        Ok(())
    }

    /// Bulk fetch — all three symbols must succeed.
    #[tokio::test]
    async fn test_fetch_candles_many() -> anyhow::Result<()> {
        let yf = YFinance::new();
        let symbols = ["AAPL", "MSFT", "GOOG"];

        let results = yf
            .fetch_candles_many(&symbols, BarSize::Daily, TimeSpec::Range(Range::OneMonth), 4)
            .await;

        for sym in &symbols {
            let candles = results[*sym].as_ref().expect(&format!("{sym} fetch failed"));
            assert!(!candles.is_empty(), "{sym} returned no candles");
            eprintln!("{sym}: {} candles", candles.len());
        }

        Ok(())
    }

    /// Daily candles for a single clean week (no holidays) — expect exactly 5.
    #[tokio::test]
    async fn test_candle_count_one_week() -> anyhow::Result<()> {
        let yf = YFinance::new();
        // 2024-03-04 (Mon) – 2024-03-08 (Fri): no US market holidays.
        let candles = yf
            .fetch_candles(
                "SPY",
                BarSize::Daily,
                TimeSpec::from_dates(
                    NaiveDate::from_ymd_opt(2024, 3, 4).unwrap(),
                    NaiveDate::from_ymd_opt(2024, 3, 8).unwrap(),
                ),
            )
            .await?;

        assert_eq!(candles.len(), 5, "Expected 5 trading days Mon-Fri, got {}", candles.len());
        Ok(())
    }

    /// Daily candles for all of January 2024 — expect exactly 23.
    /// MLK Day (Jan 15) is the only holiday; weekends account for the rest.
    #[tokio::test]
    async fn test_candle_count_january_2024() -> anyhow::Result<()> {
        let yf = YFinance::new();
        let candles = yf
            .fetch_candles(
                "SPY",
                BarSize::Daily,
                TimeSpec::from_dates(
                    NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                    NaiveDate::from_ymd_opt(2024, 1, 31).unwrap(),
                ),
            )
            .await?;

        // Jan 2024: 23 trading days (31 days - 8 weekend days - MLK Day Jan 15).
        assert_eq!(candles.len(), 23, "Expected 23 trading days in Jan 2024, got {}", candles.len());
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
        let end   = Utc.with_ymd_and_hms(2024, 3, 6, 21,  0, 0).unwrap(); // 16:00 ET

        let candles = yf
            .fetch_candles("SPY", BarSize::Hour1, TimeSpec::Interval(start, end))
            .await?;

        assert_eq!(candles.len(), 7, "Expected 7 hourly bars for a regular session, got {}", candles.len());
        Ok(())
    }

    /// Hourly candles over an explicit intraday DateTime window.
    #[tokio::test]
    async fn test_fetch_candles_datetime_period() -> anyhow::Result<()> {
        let yf = YFinance::new();
        let start = Utc.with_ymd_and_hms(2024, 6, 3, 13, 30, 0).unwrap(); // 9:30 AM ET
        let end   = Utc.with_ymd_and_hms(2024, 6, 7, 20,  0, 0).unwrap(); // 4:00 PM ET

        let candles = yf
            .fetch_candles("TSLA", BarSize::Hour1, TimeSpec::Interval(start, end))
            .await?;

        assert!(!candles.is_empty());
        assert!(candles.first().unwrap().timestamp >= start);
        assert!(candles.last().unwrap().timestamp <= end);

        eprintln!("TSLA hourly over explicit window — {} candles", candles.len());
        Ok(())
    }
}
