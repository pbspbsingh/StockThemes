use crate::config::APP_CONFIG;
use crate::store::Store;
use crate::util::is_upto_date;
use crate::yf::{BarSize, Candle, Range, TimeSpec, YFinance, YfError};
use anyhow::Context;
use axum::extract::{Extension, Path};
use axum::http::StatusCode;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, HeaderValue};
use axum::middleware::Next;
use axum::response::Html;
use axum::response::Response;
use axum::{Router, middleware, routing};
use chrono::{DateTime, Local, Months, NaiveDate, TimeDelta, Utc};
#[cfg(not(debug_assertions))]
use include_dir::{Dir, include_dir};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::Mutex as AsyncMutex;
use tracing::{info, trace, warn};

pub mod config;
pub mod etf_map;
pub mod html_error;
pub mod metrics;
pub mod rrg_util;
pub mod rs;
pub mod store;
pub mod summary;
pub mod tags;
pub mod trades;
pub mod tv;
pub mod util;
pub mod yf;

#[cfg(not(debug_assertions))]
static ASSETS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/assets");

const TWO_YEARS: TimeDelta = TimeDelta::days(2 * 365);

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
    pub last_updated: DateTime<Local>,
}

#[async_trait::async_trait]
pub trait StockInfoFetcher {
    async fn fetch(&self, ticker: &str) -> anyhow::Result<Stock>;

    async fn done(&self) {}
}

pub fn init_logger() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_new(&APP_CONFIG.log_config)
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
}

pub async fn start_http_server(store: Arc<Store>, home: String) -> anyhow::Result<()> {
    let addr = format!("127.0.0.1:{}", APP_CONFIG.http_port);
    let listener = TcpListener::bind(&addr)
        .await
        .with_context(|| format!("Failed to bind at {addr}"))?;

    info!("Running http server at: {addr}");
    let untagged = store.list_untagged_stocks().await?;
    if !untagged.is_empty() {
        warn!(
            "Tickers without tags ({}): [{}]",
            untagged.len(),
            untagged.join(",")
        );
    }

    let fundamentals_client = tv::fundamentals::FundamentalsClient::new();
    let app = Router::new()
        .route("/", routing::get(async || Html(home)))
        .route("/rrg.html", routing::get(rrg_util::rrg_home))
        .route(
            "/stock_tags.html",
            routing::get(tags::stock_tags::stock_tags_home),
        )
        .route(
            "/api/stock-tags/metrics/stream",
            routing::get(tags::stock_tags::stock_tag_metrics_stream),
        )
        .route("/api/rrg/{ticker}", routing::get(rrg_util::rrg_handler))
        .route(
            "/api/fundamentals/{exchange}/{ticker}",
            routing::get(tv::fundamentals_api::get),
        )
        .route(
            "/api/fundamentals/{exchange}/{ticker}/refresh",
            routing::post(tv::fundamentals_api::refresh),
        )
        .route("/assets/{*path}", routing::get(static_asset))
        .merge(tags::router(store.clone()))
        .layer(Extension(fundamentals_client))
        .layer(Extension(store))
        .layer(middleware::from_fn(no_cache));
    axum::serve(listener, app).await?;

    Ok(())
}

pub async fn static_asset(Path(path): Path<String>) -> Result<Response, StatusCode> {
    if path.is_empty() || path.starts_with('/') || path.contains("..") || path.contains('\\') {
        return Err(StatusCode::BAD_REQUEST);
    }

    #[cfg(debug_assertions)]
    let bytes = {
        let full_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("assets")
            .join(&path);
        tokio::fs::read(&full_path)
            .await
            .map_err(|_| StatusCode::NOT_FOUND)?
    };

    #[cfg(not(debug_assertions))]
    let bytes = ASSETS
        .get_file(&path)
        .ok_or(StatusCode::NOT_FOUND)?
        .contents()
        .to_vec();

    let content_type = match PathBuf::from(&path)
        .extension()
        .and_then(|extension| extension.to_str())
    {
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        _ => "application/octet-stream",
    };

    Response::builder()
        .header(CONTENT_TYPE, content_type)
        .body(bytes.into())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

pub async fn no_cache(request: axum::extract::Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

static FETCH_LOCKS: LazyLock<Mutex<HashMap<String, Arc<AsyncMutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub async fn fetch_candles(
    store: &Store,
    yf: &YFinance,
    ticker: &str,
) -> anyhow::Result<Vec<Candle>> {
    let lock = {
        let mut map = FETCH_LOCKS.lock().expect("lock poison");
        Arc::clone(map.entry(ticker.to_string()).or_default())
    };
    let _guard = lock.lock().await;

    let mut candles = store.get_candles(ticker).await?;
    if candles.is_empty() {
        let candles = fetch_with_backoff(|| {
            yf.fetch_candles(ticker, BarSize::Daily, TimeSpec::Range(Range::TwoYears))
        })
        .await?;
        info!(
            "Fetched {} candles for {:?} from yfinance",
            candles.len(),
            ticker,
        );
        store.save_candles(ticker, &candles).await?;
        return Ok(candles);
    }

    if is_upto_date(candles.last().unwrap().last_updated) {
        trace!("Candles for {ticker} is up to date, no need to fetch it");
        return Ok(candles);
    }

    let last_updated = candles.pop().unwrap().last_updated;
    trace!("Last candle of {ticker} was updated {last_updated}, hence requires updating");

    let start = candles
        .last()
        .map(|c| c.timestamp - TimeDelta::days(1))
        .unwrap_or_else(|| Utc::now() - TWO_YEARS);
    let end = Utc::now();
    let new_candles = fetch_with_backoff(|| {
        yf.fetch_candles(ticker, BarSize::Daily, TimeSpec::Interval(start, end))
    })
    .await?;
    info!("Fetched {} new candles for {:?}", new_candles.len(), ticker);

    candles.extend(new_candles);
    store.save_candles(ticker, &candles).await?;

    Ok(store.get_candles(ticker).await?)
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
            last_updated: Local::now(),
        }
    }

    pub fn compute(ticker: impl Into<String>, ticker_type: TickerType, candles: &[Candle]) -> Self {
        let latest = candles.last().unwrap();
        let latest_date = latest.timestamp.date_naive();

        let target_close = |months_ago: u32| -> f64 {
            let target_date = latest_date - Months::new(months_ago);
            let idx = candles.partition_point(|c| c.timestamp.date_naive() < target_date);
            let target = &candles[idx];
            ((latest.close - target.close) * 100.0) / target.close
        };

        Self {
            ticker: ticker.into(),
            ticker_type,
            perf_1m: target_close(1),
            perf_3m: target_close(3),
            perf_6m: target_close(6),
            perf_1y: target_close(12),
            last_updated: Local::now(),
        }
    }
}

impl Display for Performance {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}={{", self.ticker,)?;
        write!(f, "1M={:.2}%, ", self.perf_1m)?;
        write!(f, "3M={:.2}%, ", self.perf_3m)?;
        write!(f, "6M={:.2}%, ", self.perf_6m)?;
        write!(f, "1Y={:.2}%", self.perf_1y)?;
        writeln!(f, "}}")
    }
}

const MAX_RETRIES: u32 = 3;
const BASE_DELAY_MS: u64 = 500;

async fn fetch_with_backoff<F, Fut, T>(mut f: F) -> anyhow::Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = anyhow::Result<T>>,
{
    let mut attempt = 0;
    loop {
        match f().await {
            Ok(val) => break Ok(val),
            Err(e) if matches!(e.downcast_ref::<YfError>(), Some(YfError::NotFound { .. })) => {
                break Err(e);
            }
            Err(e) if attempt < MAX_RETRIES => {
                let delay = BASE_DELAY_MS * 2u64.pow(attempt);
                warn!(
                    "Yahoo Finance request failed (attempt {}/{}), retrying in {}ms: {e}",
                    attempt + 1,
                    MAX_RETRIES,
                    delay,
                );
                tokio::time::sleep(Duration::from_millis(delay)).await;
                attempt += 1;
            }
            Err(e) => break Err(e),
        }
    }
}
