use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::sync::{Arc, LazyLock};

use askama::Template;
use axum::{
    Extension,
    body::{Body, Bytes},
    extract::Query,
    http::header::CONTENT_TYPE,
    response::{Html, IntoResponse, Response},
};
use futures::{StreamExt, stream};
use serde::{Deserialize, Serialize};

use crate::config::APP_CONFIG;
use crate::fetch_candles;
use crate::html_error::HtmlError;
use crate::metrics;
use crate::store::{StockTags, Store, Tag, TagCategory};
use crate::util::compute_rs_candles;
use crate::yf::YFinance;
use tracing::warn;

static YF: LazyLock<YFinance> = LazyLock::new(YFinance::new);
const METRIC_STREAM_CONCURRENCY: usize = 2;

#[derive(Template)]
#[template(path = "stock_tags.html")]
struct StockTagsTemplate {
    benchmark_symbol: String,
    real_tag_count: usize,
    tagged_stock_count: usize,
    untagged_stock_count: usize,
    tag_categories: Vec<TagCategoryView>,
    tag_groups: Vec<TagGroupView>,
    ticker_info: HashMap<String, TickerInfoView>,
}

#[derive(Debug, Clone, Serialize)]
struct TagCategoryView {
    id: i64,
    name: String,
    count: usize,
}

#[derive(Debug, Clone, Serialize)]
struct TagGroupView {
    name: String,
    category_id: Option<i64>,
    count: usize,
    tickers: Vec<TagTickerView>,
    is_untagged: bool,
}

#[derive(Debug, Clone, Serialize)]
struct TagTickerView {
    ticker: String,
    exchange: String,
}

#[derive(Debug, Clone, Serialize)]
struct TickerInfoView {
    exchange: String,
    tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct MetricsQuery {
    tickers: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct StockTagMetricView {
    rs: f64,
    adr_pct: Option<f64>,
    avg_volume: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StockTagMetricStreamRow {
    ticker: String,
    metric: Option<StockTagMetricView>,
    error: Option<String>,
}

pub async fn stock_tags_home(
    Extension(store): Extension<Arc<Store>>,
) -> Result<Html<String>, HtmlError> {
    let html = build_template(&store).await?.render()?;
    Ok(Html(html))
}

pub async fn stock_tag_metrics_stream(
    Extension(store): Extension<Arc<Store>>,
    Query(query): Query<MetricsQuery>,
) -> Result<Response, HtmlError> {
    let tickers = parse_metric_tickers(&query.tickers);
    if tickers.is_empty() {
        return Ok((
            [(CONTENT_TYPE, "application/x-ndjson; charset=utf-8")],
            Body::empty(),
        )
            .into_response());
    }

    let base_candles = Arc::new(fetch_candles(&store, &YF, &APP_CONFIG.base_ticker).await?);

    let rows = stream::iter(tickers)
        .map(move |ticker| {
            let store = Arc::clone(&store);
            let base_candles = Arc::clone(&base_candles);
            async move {
                let row = metric_stream_row(store, base_candles, ticker).await;
                let line = match serde_json::to_string(&row) {
                    Ok(json) => json + "\n",
                    Err(err) => format!(
                        "{{\"ticker\":\"{}\",\"metric\":null,\"error\":\"{}\"}}\n",
                        escape_json_string(&row.ticker),
                        escape_json_string(&err.to_string())
                    ),
                };
                Ok::<Bytes, Infallible>(Bytes::from(line))
            }
        })
        .buffer_unordered(METRIC_STREAM_CONCURRENCY);

    Ok((
        [(CONTENT_TYPE, "application/x-ndjson; charset=utf-8")],
        Body::from_stream(rows),
    )
        .into_response())
}

async fn build_template(store: &Store) -> anyhow::Result<StockTagsTemplate> {
    let tags = store.list_tags().await?;
    let categories = store.list_tag_categories().await?;
    let stock_tags = store.list_stock_tags().await?;
    let untagged = store.list_untagged_stocks().await?;

    let all_tickers = collect_unique_tickers(&stock_tags, &untagged);
    let exchange_map = load_exchange_map(store, &all_tickers).await?;
    let tag_groups = build_tag_groups(&tags, &stock_tags, &exchange_map, &untagged);
    let ticker_info = build_ticker_info(&stock_tags, &untagged, &exchange_map);

    let real_tag_count = tags.len();
    let tagged_stock_count = stock_tags.len();
    let untagged_stock_count = untagged.len();

    Ok(StockTagsTemplate {
        benchmark_symbol: APP_CONFIG.base_ticker.to_uppercase(),
        real_tag_count,
        tagged_stock_count,
        untagged_stock_count,
        tag_categories: build_tag_categories(&categories),
        tag_groups,
        ticker_info,
    })
}

fn collect_unique_tickers(stock_tags: &[StockTags], untagged: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut tickers = Vec::new();

    for stock in stock_tags {
        if seen.insert(stock.ticker.clone()) {
            tickers.push(stock.ticker.clone());
        }
    }
    for ticker in untagged {
        if seen.insert(ticker.clone()) {
            tickers.push(ticker.clone());
        }
    }

    tickers
}

async fn metric_stream_row(
    store: Arc<Store>,
    base_candles: Arc<Vec<crate::yf::Candle>>,
    ticker: String,
) -> StockTagMetricStreamRow {
    match stock_tag_metric(&store, &ticker, &base_candles).await {
        Ok(metric) => StockTagMetricStreamRow {
            ticker,
            metric: Some(metric),
            error: None,
        },
        Err(err) => {
            warn!("Skipping lazy stock tag metrics for {ticker}: {err}");
            StockTagMetricStreamRow {
                ticker,
                metric: None,
                error: Some(err.to_string()),
            }
        }
    }
}

async fn stock_tag_metric(
    store: &Store,
    ticker: &str,
    base_candles: &[crate::yf::Candle],
) -> anyhow::Result<StockTagMetricView> {
    let candles = fetch_candles(store, &YF, ticker).await?;
    let metrics = metrics::compute_metrics(
        &candles,
        APP_CONFIG.metrics.adr_days,
        APP_CONFIG.metrics.avg_volume_days,
    );
    Ok(StockTagMetricView {
        rs: round_rs(compute_rs_candles(&candles, base_candles)),
        adr_pct: metrics.map(|m| m.adr_pct),
        avg_volume: metrics.map(|m| m.avg_volume),
    })
}

fn escape_json_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

async fn load_exchange_map(
    store: &Store,
    tickers: &[String],
) -> anyhow::Result<HashMap<String, String>> {
    let mut exchange_map = HashMap::with_capacity(tickers.len());

    for ticker in tickers {
        if let Some(stock) = store.get_stock(ticker).await? {
            exchange_map.insert(ticker.clone(), stock.exchange);
        }
    }

    Ok(exchange_map)
}

fn parse_metric_tickers(input: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut tickers = Vec::new();

    for ticker in input.split(',') {
        let ticker = ticker.trim().to_uppercase();
        if ticker.is_empty() {
            continue;
        }
        if seen.insert(ticker.clone()) {
            tickers.push(ticker);
        }
    }

    tickers
}

fn build_tag_groups(
    tags: &[Tag],
    stock_tags: &[StockTags],
    exchange_map: &HashMap<String, String>,
    untagged: &[String],
) -> Vec<TagGroupView> {
    let mut groups = tags
        .iter()
        .map(|tag| {
            (
                tag.name.to_lowercase(),
                TagGroupView {
                    name: tag.name.clone(),
                    category_id: Some(tag.category_id),
                    count: tag.stock_count as usize,
                    tickers: Vec::new(),
                    is_untagged: false,
                },
            )
        })
        .collect::<HashMap<_, _>>();

    for stock in stock_tags {
        let exchange = exchange_map.get(&stock.ticker).cloned().unwrap_or_default();
        for tag in &stock.tags {
            if let Some(group) = groups.get_mut(&tag.name.to_lowercase()) {
                group.tickers.push(TagTickerView {
                    ticker: stock.ticker.clone(),
                    exchange: exchange.clone(),
                });
            }
        }
    }

    let mut rows = groups.into_values().collect::<Vec<_>>();
    rows.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    let mut result = Vec::with_capacity(rows.len() + 1);
    let mut tickers = Vec::with_capacity(untagged.len());
    for ticker in untagged {
        tickers.push(TagTickerView {
            ticker: ticker.clone(),
            exchange: exchange_map.get(ticker).cloned().unwrap_or_default(),
        });
    }
    result.push(TagGroupView {
        name: "Untagged".to_string(),
        category_id: None,
        count: untagged.len(),
        tickers,
        is_untagged: true,
    });
    result.extend(rows);
    result
}

fn build_tag_categories(categories: &[TagCategory]) -> Vec<TagCategoryView> {
    categories
        .iter()
        .map(|category| TagCategoryView {
            id: category.id,
            name: category.name.clone(),
            count: category.stock_count as usize,
        })
        .collect()
}

fn build_ticker_info(
    stock_tags: &[StockTags],
    untagged: &[String],
    exchange_map: &HashMap<String, String>,
) -> HashMap<String, TickerInfoView> {
    let mut map = HashMap::new();

    for stock in stock_tags {
        map.insert(
            stock.ticker.clone(),
            TickerInfoView {
                exchange: exchange_map.get(&stock.ticker).cloned().unwrap_or_default(),
                tags: stock.tags.iter().map(|tag| tag.name.clone()).collect(),
            },
        );
    }

    for ticker in untagged {
        map.entry(ticker.clone()).or_insert_with(|| TickerInfoView {
            exchange: exchange_map.get(ticker).cloned().unwrap_or_default(),
            tags: Vec::new(),
        });
    }

    map
}

fn round_rs(rs: f64) -> f64 {
    (rs * 100.0).round() / 100.0
}
