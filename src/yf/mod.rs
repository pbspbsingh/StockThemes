mod de;
mod error;
mod types;

#[cfg(test)]
mod tests;

pub use error::YfError;
pub use types::{BarSize, Candle, Range, TickerInfo, TimeSpec};

use crate::{Group, Stock, StockInfoFetcher};
use anyhow::Context;
use chrono::{Local, TimeZone, Utc};
use de::{ChartResponse, QuoteSummaryResponse};
use log::warn;
use reqwest::{Client, StatusCode, header};
use std::time::Duration;
use tokio::sync::OnceCell;

// ============================================================================
// YFinance client
// ============================================================================

pub struct YFinance {
    client: Client,
    crumb: OnceCell<String>,
}

impl Default for YFinance {
    fn default() -> Self {
        Self::new()
    }
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

    // -------------------------------------------------------------------------
    // Cookie / crumb handling — two-strategy approach mirroring yfinance
    // -------------------------------------------------------------------------

    /// Strategy 1 ("basic"): seed the cookie jar via `fc.yahoo.com`, then
    /// fetch the crumb from `query2`. Fast path — works most of the time.
    async fn fetch_crumb_basic(&self) -> anyhow::Result<String> {
        // fc.yahoo.com seeds the session cookie. The response is usually 404
        // but the Set-Cookie header is what matters; ignore the body/error.
        let _ = self.client.get("https://fc.yahoo.com/").send().await;

        let crumb = self
            .client
            .get("https://query2.finance.yahoo.com/v1/test/getcrumb")
            .header(header::ACCEPT, "text/plain")
            .send()
            .await?
            .text()
            .await?;

        if crumb.is_empty() || crumb.contains("Unauthorized") || crumb.contains("Too Many") {
            anyhow::bail!("basic crumb strategy failed: '{crumb}'");
        }
        Ok(crumb)
    }

    /// Strategy 2 ("csrf fallback"): visit `finance.yahoo.com` to obtain a
    /// full browser-like session cookie, then re-fetch the crumb from `query1`.
    /// Slower but more reliable when Yahoo's bot detection blocks strategy 1.
    async fn fetch_crumb_csrf(&self) -> anyhow::Result<String> {
        self.client
            .get("https://finance.yahoo.com/")
            .header(
                header::ACCEPT,
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            )
            .header(header::ACCEPT_LANGUAGE, "en-US,en;q=0.5")
            .send()
            .await
            .context("csrf: failed to fetch finance.yahoo.com")?;

        let crumb = self
            .client
            .get("https://query1.finance.yahoo.com/v1/test/getcrumb")
            .header(header::ACCEPT, "text/plain")
            .send()
            .await?
            .text()
            .await?;

        if crumb.is_empty() || crumb.contains("Unauthorized") || crumb.contains("Too Many") {
            anyhow::bail!("csrf crumb strategy failed: '{crumb}'");
        }
        Ok(crumb)
    }

    /// Returns the crumb, fetching it on first call (cached via `OnceCell`).
    /// Tries the basic strategy first; falls back to csrf if that fails.
    pub(crate) async fn crumb(&self) -> anyhow::Result<&str> {
        self.crumb
            .get_or_try_init(|| async {
                match self.fetch_crumb_basic().await {
                    Ok(c) => Ok(c),
                    Err(e) => {
                        warn!("Basic cookie strategy failed ({e}), retrying with csrf fallback");
                        self.fetch_crumb_csrf().await
                    }
                }
            })
            .await
            .map(String::as_str)
    }

    // -------------------------------------------------------------------------
    // Helpers
    // -------------------------------------------------------------------------

    /// Maps HTTP status to an explicit `YfError::RateLimited` on 429,
    /// or a generic error for any other non-2xx status.
    fn check_status(status: StatusCode, url: &str) -> anyhow::Result<()> {
        if status == StatusCode::TOO_MANY_REQUESTS {
            return Err(YfError::RateLimited.into());
        }
        if !status.is_success() {
            anyhow::bail!("HTTP {} fetching {url}", status.as_u16());
        }
        Ok(())
    }

    fn exchange_map(exchange: &str) -> anyhow::Result<&'static str> {
        match exchange {
            "NYSE" => Ok("NYSE"),
            "NYSE American" => Ok("ARCA"),
            _ if exchange.starts_with("Nasdaq") => Ok("NASDAQ"),
            _ if exchange.starts_with("OTC") => Ok("OTC"),
            x => anyhow::bail!("Unknown exchange: '{x}'"),
        }
    }

    // -------------------------------------------------------------------------
    // Public API
    // -------------------------------------------------------------------------

    pub async fn fetch_ticker_info(&self, symbol: &str) -> anyhow::Result<TickerInfo> {
        let crumb = self.crumb().await?;

        let url = format!(
            "https://query1.finance.yahoo.com/v10/finance/quoteSummary/{symbol}\
             ?modules=assetProfile,price&crumb={crumb}"
        );

        let response = self
            .client
            .get(&url)
            .header(header::ACCEPT, "application/json")
            .send()
            .await?;

        Self::check_status(response.status(), &url)?;

        let response = response.json::<QuoteSummaryResponse>().await?;

        let result = response
            .quote_summary
            .result
            .and_then(|v| v.into_iter().next())
            .ok_or_else(|| anyhow::anyhow!("Empty result from Yahoo Finance for {symbol}"))?;

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
    ///
    /// `adj_close` is populated for `Daily` and `Weekly` bars.
    /// It will be `None` for all intraday bar sizes — Yahoo does not provide
    /// adjusted prices at intraday granularity.
    pub async fn fetch_candles(
        &self,
        symbol: &str,
        bar: BarSize,
        time: TimeSpec,
    ) -> anyhow::Result<Vec<Candle>> {
        // No crumb required for v8/finance/chart — it's an open endpoint.
        let pre_post = bar.include_pre_post();

        let url = match time {
            TimeSpec::Range(range) => format!(
                "https://query1.finance.yahoo.com/v8/finance/chart/{symbol}\
                 ?interval={bar}&range={range}&includePrePost={pre_post}&includeAdjustedClose=true",
                range = range.as_str(),
            ),
            TimeSpec::Interval(start, end) => format!(
                "https://query1.finance.yahoo.com/v8/finance/chart/{symbol}\
                 ?interval={bar}&period1={}&period2={}&includePrePost={pre_post}&includeAdjustedClose=true",
                start.timestamp(),
                end.timestamp(),
            ),
        };

        // Random jitter to avoid throttling
        tokio::time::sleep(Duration::from_millis(rand::random_range(10..=100))).await;

        let response = self
            .client
            .get(&url)
            .header(header::ACCEPT, "application/json")
            .send()
            .await?;

        Self::check_status(response.status(), &url)?;

        let resp = response.json::<ChartResponse>().await?;

        if let Some(err) = resp.chart.error {
            anyhow::bail!("Yahoo Finance chart error for {symbol}: {err}");
        }

        let data = resp
            .chart
            .result
            .and_then(|v| v.into_iter().next())
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

        // Extract adjclose array — absent for intraday bars.
        let adj_closes: Vec<Option<f64>> = data
            .indicators
            .adjclose
            .and_then(|v| v.into_iter().next())
            .and_then(|w| w.adjclose)
            .unwrap_or_default();

        let opens = quote.open.unwrap_or_default();
        let highs = quote.high.unwrap_or_default();
        let lows = quote.low.unwrap_or_default();
        let closes = quote.close.unwrap_or_default();
        let volumes = quote.volume.unwrap_or_default();
        let last_updated = Local::now();

        let candles = timestamps.into_iter().enumerate().filter_map(|(i, ts)| {
            let open = opens.get(i)?.as_ref()?;
            let high = highs.get(i)?.as_ref()?;
            let low = lows.get(i)?.as_ref()?;
            let close = closes.get(i)?.as_ref()?;
            let volume = volumes.get(i)?.as_ref()?;
            let adj_close = adj_closes.get(i).copied().flatten();
            Some(Candle {
                timestamp: Utc.timestamp_opt(ts, 0).single()?,
                open: *open,
                high: *high,
                low: *low,
                close: *close,
                volume: *volume,
                adj_close,
                last_updated,
            })
        });

        // When an explicit interval was requested, strip any candles Yahoo
        // appended outside the window (e.g. a "current price" sentinel bar).
        let mut candles: Vec<_> = match time {
            TimeSpec::Interval(start, end) => candles
                .filter(|c| c.timestamp >= start && c.timestamp <= end)
                .collect(),
            TimeSpec::Range(_) => candles.collect(),
        };

        candles.sort_unstable_by_key(|candle| candle.timestamp);

        Ok(candles)
    }
}

#[async_trait::async_trait]
impl StockInfoFetcher for YFinance {
    async fn fetch(&self, ticker: &str) -> anyhow::Result<Stock> {
        let ti = self.fetch_ticker_info(ticker).await?;
        let exchange = ti
            .exchange
            .with_context(|| format!("Failed to get exchange {ticker}"))?;
        let exchange = Self::exchange_map(&exchange)?;
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
