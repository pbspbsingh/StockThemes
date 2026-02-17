use chrono::NaiveDate;

use serde::{Deserialize, Serialize};

pub mod browser;
pub mod config;
pub mod store;
pub mod template;
pub mod tv;
pub mod util;
pub mod yf;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Stock {
    pub ticker: String,
    pub exchange: String,
    pub sector: Group,
    pub industry: Group,
    pub last_update: NaiveDate,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Group {
    pub name: String,
    pub url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Summary {
    pub size: usize,
    pub sectors: Vec<SummarySector>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SummarySector {
    pub name: String,
    pub url: String,
    pub size: usize,
    pub industries: Vec<SummaryIndustry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SummaryIndustry {
    pub name: String,
    pub url: String,
    pub size: usize,
    pub tickers: Vec<Ticker>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Ticker {
    pub exchange: String,
    pub ticker: String,
}

#[async_trait::async_trait]
pub trait StockInfoFetcher {
    async fn fetch(&self, ticker: &str) -> anyhow::Result<Stock>;

    async fn done(&self) {}
}
