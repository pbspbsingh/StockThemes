use crate::config::APP_CONFIG;
use crate::store::Store;
use crate::util::{compute_perf, is_upto_date};
use crate::yf::{BarSize, Candle, Range, TimeSpec, YFinance};
use anyhow::Context;
use axum::response::Html;
use axum::{Router, routing};
use chrono::{DateTime, Local, NaiveDate, TimeDelta, Utc};
use log::{debug, info};
use serde::{Deserialize, Serialize};
use sqlx::types::Json;
use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use tokio::net::TcpListener;

pub mod browser;
pub mod config;
mod etf_map;
mod html_error;
pub mod rrg_util;
pub mod store;
pub mod summary;
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
pub struct Ticker {
    pub exchange: String,
    pub ticker: String,
}

#[derive(Debug, Clone, Copy)]
pub enum TickerType {
    Sector,
    Industry,
    Stock,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Performance {
    pub ticker: String,
    pub ticker_type: TickerType,
    pub perf_1m: f64,
    pub perf_3m: f64,
    pub perf_6m: f64,
    pub perf_1y: f64,
    pub extra_info: Json<HashMap<String, f64>>,
    pub last_updated: DateTime<Local>,
}

#[async_trait::async_trait]
pub trait StockInfoFetcher {
    async fn fetch(&self, ticker: &str) -> anyhow::Result<Stock>;

    async fn done(&self) {}
}

pub fn init_logger() {
    env_logger::Builder::new()
        .parse_filters(&APP_CONFIG.log_config)
        .init();
}

pub fn time_frames(input: &str) -> impl Iterator<Item = String> {
    input.split(',').map(str::trim).map(str::to_uppercase)
}

pub async fn start_http_server(html: String) -> anyhow::Result<()> {
    let addr = format!("127.0.0.1:{}", APP_CONFIG.http_port);
    let listener = TcpListener::bind(&addr)
        .await
        .with_context(|| format!("Failed to bind at {addr}: e"))?;

    info!("Running http server at: {addr}");
    let app = Router::new().route("/", routing::get(async || Html(html)));
    axum::serve(listener, app).await?;

    Ok(())
}

pub async fn fetch_candles(
    store: &Store,
    yf: &YFinance,
    ticker: &str,
) -> anyhow::Result<Vec<Candle>> {
    let mut candles = store.get_candles(ticker).await?;
    if candles.is_empty() {
        let candles = yf
            .fetch_candles(ticker, BarSize::Daily, TimeSpec::Range(Range::TwoYears))
            .await?;
        info!("Fetched {} candles for {} from yfinance", candles.len(), ticker);
        store.save_candles(ticker, &candles).await?;
        return Ok(candles);
    }

    if is_upto_date(candles.last().unwrap().last_updated) {
        debug!("Candles for {ticker} is up to date, no need to fetch it");
        return Ok(candles);
    }

    let last_updated = candles.pop().unwrap().last_updated;
    info!("Last candle of {ticker} is from {last_updated}, hence requires updating");

    let start = candles
        .last()
        .map(|c| c.timestamp - TimeDelta::days(1))
        .unwrap_or_else(|| Utc::now() - TimeDelta::days(2 * 365));
    let end = Utc::now();
    let new_candles = yf
        .fetch_candles(ticker, BarSize::Daily, TimeSpec::Interval(start, end))
        .await?;
    info!("Fetched {} new candles for {}", new_candles.len(), ticker);

    candles.extend(new_candles);
    store.save_candles(ticker, &candles).await?;

    Ok(store.get_candles(ticker).await?)
}

pub async fn fetch_stock_perf(
    store: &Store,
    yf: &YFinance,
    ticker: &str,
) -> anyhow::Result<Performance> {
    match store.get_performance(ticker, TickerType::Stock).await? {
        Some(perf) => Ok(perf),
        None => {
            let candles = fetch_candles(store, yf, ticker).await?;
            Ok(Performance::new(
                ticker,
                TickerType::Stock,
                compute_perf(&candles),
            ))
        }
    }
}

impl Performance {
    pub fn new(
        ticker: impl Into<String>,
        ticker_type: TickerType,
        perf_map: HashMap<String, f64>,
    ) -> Self {
        Self {
            ticker: ticker.into(),
            ticker_type,
            perf_1m: perf_map.get("1M").copied().unwrap_or_default(),
            perf_3m: perf_map.get("3M").copied().unwrap_or_default(),
            perf_6m: perf_map.get("6M").copied().unwrap_or_default(),
            perf_1y: perf_map.get("1Y").copied().unwrap_or_default(),
            extra_info: Json(perf_map),
            last_updated: Local::now(),
        }
    }
}

impl Display for Performance {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}={{", self.ticker,)?;
        write!(f, "1M={:.2}%,", self.perf_1m)?;
        write!(f, "3M={:.2}%,", self.perf_3m)?;
        write!(f, "6M={:.2}%,", self.perf_6m)?;
        write!(f, "1Y={:.2}%", self.perf_1y)?;
        writeln!(f, "}}")
    }
}
