use std::collections::HashMap;

use anyhow::Context;
use tracing::info;

use crate::config::{APP_CONFIG, PerfMode};
use crate::etf_map;
use crate::store::Store;
use crate::tv::tv_manager::TvManager;
use crate::util::{compute_rs, compute_rs_candles};
use crate::yf::YFinance;
use crate::{Performance, TickerType, fetch_candles, fetch_stock_perf};

pub type RsMap = HashMap<String, f64>;

pub struct RsMaps {
    pub sectors: RsMap,
    pub industries: RsMap,
    pub stocks: RsMap,
}

pub async fn build_rs_maps(
    store: &Store,
    yf: &YFinance,
    tv_manager: &mut TvManager,
    stock_perfs: &[Performance],
) -> anyhow::Result<RsMaps> {
    let base_candles = fetch_candles(store, yf, &APP_CONFIG.base_ticker).await?;
    info!("Fetched {} baseline candles", base_candles.len());
    let base_perf = Performance::compute(&APP_CONFIG.base_ticker, TickerType::Stock, &base_candles);

    match APP_CONFIG.perf_mode {
        PerfMode::TradingView => {
            let sectors = tv_manager.fetch_sectors().await?;
            info!("Fetched {} sectors", sectors.len());
            let industries = tv_manager.fetch_industries().await?;
            info!("Fetched {} industry groups", industries.len());
            Ok(RsMaps {
                sectors: rs_map_from_perfs(sectors, &base_perf),
                industries: rs_map_from_perfs(industries, &base_perf),
                stocks: rs_map_from_perfs(stock_perfs.iter().cloned(), &base_perf),
            })
        }
        PerfMode::EtfBuckets => {
            build_etf_rs_maps(stock_perfs, async |ticker| {
                let p = fetch_stock_perf(store, yf, ticker).await?;
                Ok(compute_rs(&p, &base_perf))
            })
            .await
        }
        PerfMode::EtfCandles => {
            build_etf_rs_maps(stock_perfs, async |ticker| {
                let c = fetch_candles(store, yf, ticker).await?;
                compute_rs_candles(&c, &base_candles)
                    .with_context(|| format!("IBD RS for {ticker}"))
            })
            .await
        }
    }
}

async fn build_etf_rs_maps(
    stock_perfs: &[Performance],
    mut rs_fn: impl AsyncFnMut(&str) -> anyhow::Result<f64>,
) -> anyhow::Result<RsMaps> {
    let mapping = etf_map::tv_mapping();
    let mut sectors = HashMap::new();
    let mut industries = HashMap::new();
    let mut stocks = HashMap::new();

    for s in &mapping {
        sectors.insert(s.sector.clone(), round_rs(rs_fn(&s.sector_etf).await?));
        for ind in &s.industries {
            industries.insert(ind.name.clone(), round_rs(rs_fn(&ind.etf).await?));
        }
    }
    for p in stock_perfs {
        stocks.insert(p.ticker.clone(), round_rs(rs_fn(&p.ticker).await?));
    }
    Ok(RsMaps {
        sectors,
        industries,
        stocks,
    })
}

fn rs_map_from_perfs(perfs: impl IntoIterator<Item = Performance>, base: &Performance) -> RsMap {
    perfs
        .into_iter()
        .map(|p| {
            let rs = round_rs(compute_rs(&p, base));
            (p.ticker, rs)
        })
        .collect()
}

fn round_rs(rs: f64) -> f64 {
    (rs * 100.0).round() / 100.0
}
