use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};

use askama::Template;
use axum::{
    Extension,
    response::Html,
};
use chrono::Utc;
use futures::future::join_all;
use serde::Serialize;

use crate::config::APP_CONFIG;
use crate::html_error::HtmlError;
use crate::metrics;
use crate::store::{Store, StockTags, Tag};
use crate::util::compute_rs_candles;
use crate::yf::YFinance;
use crate::{Group, Stock, fetch_candles};

static YF: LazyLock<YFinance> = LazyLock::new(YFinance::new);

#[derive(Template)]
#[template(path = "stock_tags.html")]
struct StockTagsTemplate {
    benchmark_symbol: String,
    real_tag_count: usize,
    tagged_stock_count: usize,
    untagged_stock_count: usize,
    tag_groups_json: String,
    ticker_info_json: String,
    stock_rs_json: String,
    stock_metrics_json: String,
}

#[derive(Debug, Clone, Serialize)]
struct TagGroupView {
    name: String,
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

pub async fn stock_tags_home(Extension(store): Extension<Arc<Store>>) -> Result<Html<String>, HtmlError> {
    let html = build_template(&store).await?.render()?;
    Ok(Html(html))
}

async fn build_template(store: &Store) -> anyhow::Result<StockTagsTemplate> {
    let tags = store.list_tags().await?;
    let stock_tags = store.list_stock_tags().await?;
    let untagged = store.list_untagged_stocks().await?;

    let all_tickers = collect_unique_tickers(&stock_tags, &untagged);
    let stocks = load_stocks(store, &all_tickers).await?;
    let exchange_map = stocks
        .iter()
        .map(|stock| (stock.ticker.clone(), stock.exchange.clone()))
        .collect::<HashMap<_, _>>();

    let stock_rs = build_stock_rs(store, &YF, &stocks).await?;
    let stock_metrics = metrics::build_stock_metrics(store, &YF, &stocks).await?;

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
        tag_groups_json: serde_json::to_string(&tag_groups)?,
        ticker_info_json: serde_json::to_string(&ticker_info)?,
        stock_rs_json: serde_json::to_string(&stock_rs)?,
        stock_metrics_json: serde_json::to_string(&stock_metrics)?,
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

async fn load_stocks(store: &Store, tickers: &[String]) -> anyhow::Result<Vec<Stock>> {
    let fetches = tickers.iter().cloned().map(|ticker| async move {
        match store.get_stock(&ticker).await? {
            Some(stock) => Ok::<Stock, anyhow::Error>(stock),
            None => Ok::<Stock, anyhow::Error>(Stock {
                ticker,
                exchange: String::new(),
                sector: unknown_group(),
                industry: unknown_group(),
                last_update: Utc::now().date_naive(),
            }),
        }
    });

    let mut stocks = Vec::with_capacity(tickers.len());
    for result in join_all(fetches).await {
        stocks.push(result?);
    }
    Ok(stocks)
}

async fn build_stock_rs(
    store: &Store,
    yf: &YFinance,
    stocks: &[Stock],
) -> anyhow::Result<HashMap<String, f64>> {
    let base_candles = fetch_candles(store, yf, &APP_CONFIG.base_ticker).await?;
    let mut map = HashMap::with_capacity(stocks.len());

    for stock in stocks {
        let candles = fetch_candles(store, yf, &stock.ticker).await?;
        map.insert(stock.ticker.clone(), round_rs(compute_rs_candles(&candles, &base_candles)));
    }

    Ok(map)
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
    rows.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));

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
        count: untagged.len(),
        tickers,
        is_untagged: true,
    });
    result.extend(rows);
    result
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

fn unknown_group() -> Group {
    Group {
        name: "Unknown".to_string(),
        url: String::new(),
    }
}

fn round_rs(rs: f64) -> f64 {
    (rs * 100.0).round() / 100.0
}
