use crate::Ticker;
use crate::html_error::HtmlError;
use crate::store::Store;
use crate::tv::fundamentals::{Fundamentals, FundamentalsClient};
use axum::Json;
use axum::extract::{Extension, Path};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::sync::Arc;

#[derive(Serialize)]
pub struct FundamentalsResponse {
    #[serde(flatten)]
    fundamentals: Fundamentals,
    last_updated: DateTime<Utc>,
}

pub async fn get(
    Path((exchange, ticker)): Path<(String, String)>,
    Extension(store): Extension<Arc<Store>>,
    Extension(client): Extension<FundamentalsClient>,
) -> Result<Json<FundamentalsResponse>, HtmlError> {
    let ticker = normalize(exchange, ticker)?;
    if let Some(cached) = store
        .get_fundamentals(&ticker.exchange, &ticker.ticker)
        .await?
    {
        return Ok(Json(FundamentalsResponse {
            fundamentals: serde_json::from_str(&cached.payload)?,
            last_updated: cached.last_updated,
        }));
    }
    fetch_and_cache(&store, &client, ticker).await
}

pub async fn refresh(
    Path((exchange, ticker)): Path<(String, String)>,
    Extension(store): Extension<Arc<Store>>,
    Extension(client): Extension<FundamentalsClient>,
) -> Result<Json<FundamentalsResponse>, HtmlError> {
    fetch_and_cache(&store, &client, normalize(exchange, ticker)?).await
}

async fn fetch_and_cache(
    store: &Store,
    client: &FundamentalsClient,
    ticker: Ticker,
) -> Result<Json<FundamentalsResponse>, HtmlError> {
    let fundamentals = client
        .fetch(std::slice::from_ref(&ticker))
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("TradingView returned no fundamentals"))?;
    if !fundamentals.has_usable_data() {
        return Err(anyhow::anyhow!(
            "TradingView returned no usable fundamentals for {}:{}",
            ticker.exchange,
            ticker.ticker
        )
        .into());
    }
    let last_updated = Utc::now();
    store
        .save_fundamentals(
            &ticker.exchange,
            &ticker.ticker,
            &serde_json::to_string(&fundamentals)?,
            last_updated,
        )
        .await?;
    Ok(Json(FundamentalsResponse {
        fundamentals,
        last_updated,
    }))
}

fn normalize(exchange: String, ticker: String) -> anyhow::Result<Ticker> {
    let exchange = exchange.trim().to_uppercase();
    let ticker = ticker.trim().to_uppercase();
    if exchange.is_empty() || ticker.is_empty() {
        anyhow::bail!("Exchange and ticker are required");
    }
    Ok(Ticker { exchange, ticker })
}
