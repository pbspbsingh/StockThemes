use itertools::Itertools;
use std::collections::HashMap;
use tracing::{info, warn};

use crate::config::APP_CONFIG;
use crate::fetch_candles;
use crate::store::Store;
use crate::util::compute_rs_candles;
use crate::yf::YFinance;
use crate::{Stock, etf_map};

pub type RsMap = HashMap<String, f64>;

#[derive(Debug)]
pub struct RsMaps {
    pub sectors: RsMap,
    pub industries: RsMap,
    pub stocks: RsMap,
}

pub async fn build_rs_maps(
    store: &Store,
    yf: &YFinance,
    stocks: &[Stock],
) -> anyhow::Result<RsMaps> {
    let base_candles = fetch_candles(store, yf, &APP_CONFIG.base_ticker).await?;
    info!("Fetched {} baseline candles", base_candles.len());

    build_etf_rs_maps(stocks, async |ticker| {
        let candles = fetch_candles(store, yf, ticker).await?;
        Ok(compute_rs_candles(&candles, &base_candles))
    })
    .await
}

async fn build_etf_rs_maps(
    stocks: &[Stock],
    mut rs_fn: impl AsyncFnMut(&str) -> anyhow::Result<f64>,
) -> anyhow::Result<RsMaps> {
    let mut sector_rs = HashMap::new();
    let mut industrie_rs = HashMap::new();
    let mut stock_rs = HashMap::new();

    let mapping = etf_map::tv_mapping();
    for sec in stocks.iter().map(|s| &s.sector).unique_by(|sec| &sec.name) {
        let Some(sec) = mapping
            .iter()
            .find(|s| s.sector.eq_ignore_ascii_case(&sec.name))
        else {
            warn!("No ETF mapping found for Sector: {}", sec.name);
            continue;
        };

        sector_rs.insert(sec.sector.clone(), round_rs(rs_fn(&sec.sector_etf).await?));
    }
    for ind in stocks
        .iter()
        .map(|s| &s.industry)
        .unique_by(|ind| &ind.name)
    {
        let Some(ind) = mapping
            .iter()
            .flat_map(|sec| &sec.industries)
            .find(|&i| i.name.eq_ignore_ascii_case(&ind.name))
        else {
            warn!("No ETF mapping found for Industry: {}", ind.name);
            continue;
        };

        industrie_rs.insert(ind.name.clone(), round_rs(rs_fn(&ind.etf).await?));
    }
    for st in stocks {
        stock_rs.insert(st.ticker.clone(), round_rs(rs_fn(&st.ticker).await?));
    }

    Ok(RsMaps {
        sectors: sector_rs,
        industries: industrie_rs,
        stocks: stock_rs,
    })
}

fn round_rs(rs: f64) -> f64 {
    (rs * 100.0).round() / 100.0
}
