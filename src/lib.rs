use std::collections::HashMap;
use crate::config::APP_CONFIG;
use anyhow::Context;
use axum::response::Html;
use axum::{Router, routing};
use chrono::{DateTime, Local, NaiveDate};
use log::info;
use serde::{Deserialize, Serialize};
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

#[derive(Debug, sqlx::FromRow)]
pub struct Performance {
    pub ticker: String,
    pub perf_1m: f64,
    pub perf_3m: f64,
    pub perf_6m: f64,
    pub perf_1y: f64,
    pub last_updated: DateTime<Local>,
    pub extra_info: sqlx::types::Json<HashMap<String, f64>>,
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
