use crate::{Group, Stock, StockInfoFetcher, util};
use anyhow::Context;
use chrono::Local;
use futures::{StreamExt, TryStreamExt, stream};
use reqwest::{Client, header};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

use url::Url;

const SCANNER_URL: &str = "https://scanner.tradingview.com/global/scan";
const SYMBOL_SEARCH_URL: &str = "https://symbol-search.tradingview.com/symbol_search/v3/";
const FIELDS: [&str; 4] = ["name", "exchange", "sector", "industry"];
const SYMBOL_SEARCH_CONCURRENCY: usize = 8;

#[derive(Clone)]
pub struct ScreenerApi {
    client: &'static Client,
}

impl Default for ScreenerApi {
    fn default() -> Self {
        Self::new()
    }
}

impl ScreenerApi {
    pub fn new() -> Self {
        Self {
            client: &util::HTTP_CLIENT,
        }
    }

    pub async fn fetch_stocks(&self, tickers: &[String]) -> anyhow::Result<HashMap<String, Stock>> {
        let resolved = self.resolve_tickers(tickers).await?;
        let qualified = resolved
            .iter()
            .map(|(_, qualified)| qualified.clone())
            .collect::<Vec<_>>();
        let stocks = self.scan_qualified(&qualified).await?;
        let qualified_to_input = resolved
            .into_iter()
            .map(|(input, qualified)| (qualified, input))
            .collect::<HashMap<_, _>>();

        let mut result = HashMap::with_capacity(stocks.len());
        for stock in stocks {
            let Some(input) = qualified_to_input.get(&stock.ticker) else {
                continue;
            };
            result.insert(input.clone(), stock_from_scanner(stock, input));
        }

        Ok(result)
    }

    async fn resolve_tickers(&self, tickers: &[String]) -> anyhow::Result<Vec<(String, String)>> {
        stream::iter(tickers.iter().cloned())
            .map(|ticker| async move {
                self.resolve_ticker(&ticker)
                    .await
                    .map(|qualified| (ticker, qualified))
            })
            .buffer_unordered(SYMBOL_SEARCH_CONCURRENCY)
            .try_collect()
            .await
    }

    async fn resolve_ticker(&self, ticker: &str) -> anyhow::Result<String> {
        if ticker.contains(':') {
            return Ok(ticker.to_uppercase());
        }

        let response = match self.search_symbol(ticker, Some("stocks")).await {
            Ok(response) if !response.symbols.is_empty() => response,
            _ => self.search_symbol(ticker, None).await?,
        };

        let best = response
            .symbols
            .iter()
            .find(|m| {
                m.symbol.eq_ignore_ascii_case(ticker) && !m.exchange.eq_ignore_ascii_case("BOATS")
            })
            .or_else(|| {
                response
                    .symbols
                    .iter()
                    .find(|m| m.symbol.eq_ignore_ascii_case(ticker))
            })
            .or_else(|| response.symbols.first())
            .with_context(|| format!("Could not resolve TradingView symbol {ticker}"))?;

        Ok(best.qualified())
    }

    async fn search_symbol(
        &self,
        ticker: &str,
        search_type: Option<&str>,
    ) -> anyhow::Result<SymbolSearchResponse> {
        let mut url = Url::parse(SYMBOL_SEARCH_URL)?;
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("text", ticker).append_pair("lang", "en");
            if let Some(search_type) = search_type {
                pairs.append_pair("search_type", search_type);
            }
        }

        self.client
            .get(url)
            .header(header::ORIGIN, "https://www.tradingview.com")
            .header(header::REFERER, "https://www.tradingview.com/")
            .send()
            .await?
            .error_for_status()?
            .json::<SymbolSearchResponse>()
            .await
            .map_err(Into::into)
    }

    async fn scan_qualified(&self, qualified: &[String]) -> anyhow::Result<Vec<ScannerStock>> {
        if qualified.is_empty() {
            return Ok(Vec::new());
        }

        let payload = ScanRequest {
            symbols: SymbolsBlock {
                tickers: qualified.to_vec(),
                query: SymbolQuery { types: Vec::new() },
            },
            columns: FIELDS.iter().map(|field| (*field).to_owned()).collect(),
        };

        let response = self
            .client
            .post(SCANNER_URL)
            .header(header::ORIGIN, "https://www.tradingview.com")
            .header(header::REFERER, "https://www.tradingview.com/")
            .json(&payload)
            .send()
            .await?
            .error_for_status()?
            .json::<ScanResponse>()
            .await?;

        response
            .data
            .into_iter()
            .map(scanner_stock_from_row)
            .collect()
    }
}

#[async_trait::async_trait]
impl StockInfoFetcher for ScreenerApi {
    async fn fetch(&self, ticker: &str) -> anyhow::Result<Stock> {
        let stocks = self.fetch_stocks(&[ticker.to_owned()]).await?;
        stocks
            .into_values()
            .next()
            .with_context(|| format!("TradingView screener returned no stock info for {ticker}"))
    }
}

fn scanner_stock_from_row(row: SymbolRow) -> anyhow::Result<ScannerStock> {
    let get = |idx: usize| -> String {
        row.values
            .get(idx)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned()
    };

    Ok(ScannerStock {
        ticker: row.symbol,
        name: get(0),
        exchange: get(1),
        sector: get(2),
        industry: get(3),
    })
}

fn stock_from_scanner(stock: ScannerStock, input_ticker: &str) -> Stock {
    let ticker = input_ticker
        .split_once(':')
        .map(|(_, ticker)| ticker)
        .unwrap_or(input_ticker)
        .to_uppercase();

    Stock {
        ticker,
        exchange: stock.exchange,
        sector: Group {
            name: stock.sector,
            url: String::from("#"),
        },
        industry: Group {
            name: stock.industry,
            url: String::from("#"),
        },
        last_update: Local::now().date_naive(),
    }
}

#[derive(Debug)]
struct ScannerStock {
    ticker: String,
    #[allow(dead_code)]
    name: String,
    exchange: String,
    sector: String,
    industry: String,
}

#[derive(Debug, Serialize)]
struct ScanRequest {
    symbols: SymbolsBlock,
    columns: Vec<String>,
}

#[derive(Debug, Serialize)]
struct SymbolsBlock {
    tickers: Vec<String>,
    query: SymbolQuery,
}

#[derive(Debug, Serialize)]
struct SymbolQuery {
    types: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ScanResponse {
    data: Vec<SymbolRow>,
}

#[derive(Debug, Deserialize)]
struct SymbolRow {
    #[serde(rename = "s")]
    symbol: String,
    #[serde(rename = "d")]
    values: Vec<Value>,
}

#[derive(Debug, Deserialize)]
struct SymbolSearchResponse {
    symbols: Vec<SymbolMatch>,
}

#[derive(Debug, Deserialize)]
struct SymbolMatch {
    symbol: String,
    exchange: String,
    #[serde(default)]
    prefix: Option<String>,
}

impl SymbolMatch {
    fn qualified(&self) -> String {
        let prefix = self
            .prefix
            .as_deref()
            .filter(|prefix| !prefix.is_empty())
            .unwrap_or(&self.exchange);
        format!("{}:{}", prefix.to_uppercase(), self.symbol.to_uppercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn symbol_match_uses_prefix_when_available() {
        let matched = SymbolMatch {
            symbol: "aapl".to_owned(),
            exchange: "XNGS".to_owned(),
            prefix: Some("NASDAQ".to_owned()),
        };

        assert_eq!(matched.qualified(), "NASDAQ:AAPL");
    }

    #[test]
    fn scanner_row_maps_to_stock_with_existing_domain_types() {
        let scanner_stock = scanner_stock_from_row(SymbolRow {
            symbol: "NASDAQ:AAPL".to_owned(),
            values: vec![
                json!("AAPL"),
                json!("NASDAQ"),
                json!("Electronic Technology"),
                json!("Telecommunications Equipment"),
            ],
        })
        .unwrap();
        let stock = stock_from_scanner(scanner_stock, "aapl");

        assert_eq!(stock.ticker, "AAPL");
        assert_eq!(stock.exchange, "NASDAQ");
        assert_eq!(stock.sector.name, "Electronic Technology");
        assert_eq!(stock.industry.name, "Telecommunications Equipment");
        assert_eq!(stock.sector.url, "#");
        assert_eq!(stock.industry.url, "#");
    }
}
