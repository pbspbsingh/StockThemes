use crate::config::APP_CONFIG;
use crate::store::Store;
use crate::util::compute_perf;
use crate::yf::{BarSize, Range, TimeSpec, YFinance};
use anyhow::Context;
use axum::response::Html;
use axum::{Router, routing};
use chrono::{DateTime, Local, NaiveDate};
use log::info;
use serde::{Deserialize, Serialize};
use sqlx::types::Json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::TcpListener;

pub mod browser;
pub mod config;
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
    let addr = "127.0.0.1:8000";
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("Failed to bind at {addr}: e"))?;

    info!("Running http server at: {addr}");
    let app = Router::new().route("/", routing::get(async || Html(html)));
    axum::serve(listener, app).await?;

    Ok(())
}

pub async fn fetch_stock_perf(
    store: Arc<Store>,
    yf: &YFinance,
    ticker: &str,
) -> anyhow::Result<Performance> {
    match store.get_performance(ticker, TickerType::Stock).await? {
        Some(perf) => Ok(perf),
        None => {
            let candles = yf
                .fetch_candles(ticker, BarSize::Daily, TimeSpec::Range(Range::OneYear))
                .await?;
            let perf = Performance::new(ticker, TickerType::Stock, compute_perf(&candles));
            store.save_performances(&[perf.clone()]).await?;
            Ok(perf)
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
