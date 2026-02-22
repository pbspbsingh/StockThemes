use std::{collections::HashMap, path::PathBuf};

use crate::Stock;
use anyhow::Context;
use chrono::{Local, TimeDelta};
use log::error;
use serde::{Deserialize, Serialize};
use tokio::fs;

#[derive(Debug, Serialize, Deserialize)]
pub struct Store {
    store_file: PathBuf,
    info: HashMap<String, Stock>,
}

impl Store {
    pub async fn load_store(use_tv: bool) -> anyhow::Result<Store> {
        let store_file = if use_tv {
            "stocks_info_tv.json"
        } else {
            "stocks_info_yf.json"
        }
        .into();
        let Ok(content) = fs::read_to_string(&store_file).await else {
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

    pub async fn add(&mut self, stock: &[Stock]) -> anyhow::Result<()> {
        for stock in stock {
            self.info.insert(stock.ticker.clone(), stock.clone());
        }
        self.save().await
    }

    pub async fn save(&mut self) -> anyhow::Result<()> {
        let content = serde_json::to_string_pretty(&self.info)?;
        fs::write(&self.store_file, content)
            .await
            .with_context(|| format!("Failed to write stock info to {:?}", self.store_file))
    }
}
