use crate::{Group, Stock, StockInfoFetcher};
use anyhow::Context;
use chrono::Local;
use reqwest::{Client, header};
use serde::Deserialize;
use std::time::Duration;

// --- Response structs ---

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
    #[serde(rename = "exchangeCode")] // short code e.g. "NMS", "NYQ"
    exchange_code: Option<String>,
}

// --- Output struct ---

#[derive(Debug)]
pub struct TickerInfo {
    pub symbol: String,
    pub exchange: Option<String>,
    pub exchange_code: Option<String>,
    pub sector: Option<String>,
    pub industry: Option<String>,
}

pub struct YFinance {
    client: Client,
    crumb: String,
}

impl YFinance {
    pub async fn new() -> anyhow::Result<Self> {
        let client = Client::builder()
            .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:148.0) Gecko/20100101 Firefox/148.0")
            .cookie_store(true)
            .gzip(true)
            .deflate(true)
            .timeout(Duration::from_secs(10))
            .build()
            .expect("Failed to build HTTP client");
        let crumb = Self::crumb(&client).await?;
        Ok(Self { client, crumb })
    }

    async fn crumb(client: &Client) -> anyhow::Result<String> {
        let crumb = client
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
    }

    pub async fn fetch_ticker_info(&self, symbol: &str) -> anyhow::Result<TickerInfo> {
        let url = format!(
            "https://query1.finance.yahoo.com/v10/finance/quoteSummary/{}?modules=assetProfile,price&crumb={}",
            symbol, self.crumb
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

#[cfg(test)]
mod test {
    use super::YFinance;

    #[tokio::test]
    async fn test_ticker_info() -> anyhow::Result<()> {
        let yf = YFinance::new().await?;
        eprintln!("{:?}", yf.crumb);
        Ok(())
    }

    #[tokio::test]
    async fn test_crumb_is_reused() -> anyhow::Result<()> {
        let yf = YFinance::new().await?;
        // Both calls should use the same crumb (only one network handshake)
        eprintln!("{:?}", yf.fetch_ticker_info("AAPL").await?);
        eprintln!("{:?}", yf.fetch_ticker_info("JPM").await?);
        Ok(())
    }
}
