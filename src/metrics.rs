use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::warn;

use crate::config::APP_CONFIG;
use crate::store::Store;
use crate::yf::{Candle, YFinance};
use crate::{Stock, fetch_candles};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct StockMetrics {
    pub adr_pct: f64,
    pub avg_volume: u64,
}

pub type MetricsMap = HashMap<String, StockMetrics>;

pub async fn build_stock_metrics(
    store: &Store,
    yf: &YFinance,
    stocks: &[Stock],
) -> anyhow::Result<MetricsMap> {
    let adr_days = APP_CONFIG.metrics.adr_days;
    let vol_days = APP_CONFIG.metrics.avg_volume_days;

    let mut map = HashMap::with_capacity(stocks.len());
    for stock in stocks {
        let candles = fetch_candles(store, yf, &stock.ticker).await?;
        match compute_metrics(&candles, adr_days, vol_days) {
            Some(metrics) => {
                map.insert(stock.ticker.clone(), metrics);
            }
            None => warn!(
                "Skipping metrics for {}: only {} candles available",
                stock.ticker,
                candles.len()
            ),
        }
    }
    Ok(map)
}

fn compute_metrics(candles: &[Candle], adr_days: usize, vol_days: usize) -> Option<StockMetrics> {
    fn tail(candles: &[Candle], days: usize) -> &[Candle] {
        let start = candles.len().saturating_sub(days);
        &candles[start..]
    }

    if candles.is_empty() {
        return None;
    }

    let adr_window = tail(candles, adr_days);
    let adr_pct = adr_window
        .iter()
        .filter(|c| c.low > 0.0)
        .map(|c| (c.high / c.low) - 1.0)
        .sum::<f64>()
        / adr_window.len() as f64
        * 100.0;

    let vol_window = tail(candles, vol_days);
    let avg_volume =
        (vol_window.iter().map(|c| c.volume).sum::<u64>() as f64 / vol_window.len() as f64) as u64;

    Some(StockMetrics {
        adr_pct,
        avg_volume,
    })
}
