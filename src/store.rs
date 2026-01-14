use std::collections::HashMap;

use anyhow::Context;
use chrono::{Local, TimeDelta};
use log::error;
use serde::{Deserialize, Serialize};

use crate::{Stock, config::APP_CONFIG};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Store {
    industry_info: HashMap<String, Stock>,
}

impl Store {
    pub fn load_store() -> anyhow::Result<Store> {
        let store_file = &APP_CONFIG.stock_store_file;
        let Ok(content) = std::fs::read_to_string(store_file) else {
            error!("Stock info file: {store_file:?} not found");
            return Ok(Default::default());
        };

        let today = Local::now().date_naive();
        let mut store = toml::from_str::<Store>(&content)
            .with_context(|| format!("Failed to parse:\n{content}"))?;
        store
            .industry_info
            .retain(|_ticker, stock| today - stock.last_update <= TimeDelta::days(30));
        Ok(store)
    }

    pub fn get(&self, ticker: impl AsRef<str>) -> Option<&Stock> {
        self.industry_info.get(ticker.as_ref())
    }

    pub fn add(&mut self, stock: Stock) -> anyhow::Result<()> {
        self.industry_info.insert(stock.ticker.clone(), stock);
        self.save()
    }

    pub fn save(&mut self) -> anyhow::Result<()> {
        let store_file = &APP_CONFIG.stock_store_file;
        let content = toml::to_string_pretty(self)?;
        std::fs::write(store_file, content)
            .with_context(|| format!("Failed to write stock info to {store_file:?}"))?;
        Ok(())
    }
}
