use std::{collections::HashMap, path::PathBuf};

use anyhow::Context;
use chrono::{Local, TimeDelta};
use log::error;
use serde::{Deserialize, Serialize};

use crate::Stock;

#[derive(Debug, Serialize, Deserialize)]
pub struct Store {
    store_file: PathBuf,
    info: HashMap<String, Stock>,
}

impl Store {
    pub fn load_store(store_file: impl Into<PathBuf>) -> anyhow::Result<Store> {
        let store_file = store_file.into();
        let Ok(content) = std::fs::read_to_string(&store_file) else {
            error!("Stock info file: {store_file:?} not found");
            return Ok(Self {
                store_file,
                info: HashMap::new(),
            });
        };

        let today = Local::now().date_naive();
        let mut info = serde_json::from_str::<HashMap<String, Stock>>(&content)
            .with_context(|| format!("Failed to parse:\n{content}"))?;

        info.retain(|_, stock| today - stock.last_update <= TimeDelta::days(30));
        Ok(Store { store_file, info })
    }

    pub fn get(&self, ticker: impl AsRef<str>) -> Option<&Stock> {
        self.info.get(ticker.as_ref())
    }

    pub fn add(&mut self, stock: Stock) -> anyhow::Result<()> {
        self.info.insert(stock.ticker.clone(), stock);
        self.save()
    }

    pub fn save(&mut self) -> anyhow::Result<()> {
        let content = serde_json::to_string_pretty(&self.info)?;
        std::fs::write(&self.store_file, content)
            .with_context(|| format!("Failed to write stock info to {:?}", self.store_file))
    }
}
