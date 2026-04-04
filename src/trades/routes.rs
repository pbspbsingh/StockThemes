use anyhow::Context;
use askama::Template;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    response::{Html, IntoResponse},
    routing,
};
use chrono::DateTime;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::info;

use super::candles::fetch_hourly_candles;
use crate::config::APP_CONFIG;
use crate::html_error::HtmlError;
use crate::store::Store;
use crate::trades::TradeView;
use crate::yf::YFinance;

// ── Shared state ──────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<Store>,
    pub yf: Arc<YFinance>,
    pub html: Arc<String>,
}

// ── Query params ──────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CandleQuery {
    /// Unix timestamp (seconds)
    from: i64,
    /// Unix timestamp (seconds)
    to: i64,
}

// ── Candle response ───────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct CandlePoint {
    time: i64,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: u64,
}

// ── Handlers ──────────────────────────────────────────────────────────────────

pub async fn home(State(state): State<AppState>) -> impl IntoResponse {
    Html(state.html.as_ref().clone())
}

pub async fn daily_candles(
    State(state): State<AppState>,
    Path(ticker): Path<String>,
    Query(q): Query<CandleQuery>,
) -> Result<impl IntoResponse, HtmlError> {
    let from = DateTime::from_timestamp(q.from, 0)
        .ok_or_else(|| anyhow::anyhow!("Invalid from timestamp"))?;
    let to =
        DateTime::from_timestamp(q.to, 0).ok_or_else(|| anyhow::anyhow!("Invalid to timestamp"))?;

    let candles = crate::fetch_candles(&state.store, &state.yf, &ticker).await?;
    let points: Vec<CandlePoint> = candles
        .into_iter()
        .filter(|c| c.timestamp >= from && c.timestamp <= to)
        .map(|c| CandlePoint {
            time: c.timestamp.timestamp(),
            open: c.open,
            high: c.high,
            low: c.low,
            close: c.close,
            volume: c.volume,
        })
        .collect();

    Ok(Json(points))
}

pub async fn hourly_candles(
    State(state): State<AppState>,
    Path(ticker): Path<String>,
    Query(q): Query<CandleQuery>,
) -> Result<impl IntoResponse, HtmlError> {
    let from = DateTime::from_timestamp(q.from, 0)
        .ok_or_else(|| anyhow::anyhow!("Invalid from timestamp"))?;
    let to =
        DateTime::from_timestamp(q.to, 0).ok_or_else(|| anyhow::anyhow!("Invalid to timestamp"))?;

    let tz_offset = chrono::Local::now().offset().local_minus_utc() as i64;
    let candles = fetch_hourly_candles(&state.store, &state.yf, &ticker, from, to).await?;
    let points: Vec<CandlePoint> = candles
        .into_iter()
        .map(|c| CandlePoint {
            time: c.timestamp.timestamp() + tz_offset,
            open: c.open,
            high: c.high,
            low: c.low,
            close: c.close,
            volume: c.volume,
        })
        .collect();

    Ok(Json(points))
}

// ── Server startup ────────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "trade_analyzer.html")]
struct TradeAnalyzerTemplate {
    trades_json: String,
    benchmark_json: String,
    min_hourly_candles: f64,
    tz_offset_secs: i32,
}

pub async fn start_server(
    store: Arc<Store>,
    yf: Arc<YFinance>,
    trade_views: Vec<TradeView>,
    benchmark: &str,
) -> anyhow::Result<()> {
    let cfg = &APP_CONFIG.trade_analysis;
    // Expected hourly candles: calendar days → trading days (×5/7), 6.5h/day, 10% holiday buffer
    let min_hourly_candles =
        (cfg.hourly_chart_days + cfg.hourly_chart_post_days) as f64 * (5.0 / 7.0) * 6.5 * 0.9;
    let tz_offset_secs = chrono::Local::now().offset().local_minus_utc();
    let html = TradeAnalyzerTemplate {
        trades_json: serde_json::to_string(&trade_views)?,
        benchmark_json: serde_json::to_string(benchmark)?,
        min_hourly_candles,
        tz_offset_secs,
    }
    .render()?;

    let state = AppState {
        store,
        yf,
        html: Arc::new(html),
    };

    let app = Router::new()
        .route("/", routing::get(home))
        .route("/api/candles/daily/{ticker}", routing::get(daily_candles))
        .route("/api/candles/hourly/{ticker}", routing::get(hourly_candles))
        .with_state(state);

    let addr = format!("127.0.0.1:{}", APP_CONFIG.http_port);
    let listener = TcpListener::bind(&addr)
        .await
        .with_context(|| format!("Failed to bind at {addr}"))?;

    info!("Trade analyzer running at http://{addr}");

    axum::serve(listener, app).await?;
    Ok(())
}
