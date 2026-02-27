use std::{
    path::{Path, PathBuf},
    sync::LazyLock,
};

use anyhow::Context;
use chrono::NaiveTime;
use serde::{Deserialize, Serialize};

const CONFIG_FILE: &str = "config.toml";

#[derive(Debug, Deserialize, Serialize)]
pub struct Config {
    pub log_config: String,
    pub chrome_path: String,
    pub user_data_dir: PathBuf,
    #[serde(default)]
    pub chrome_args: Vec<String>,
    #[serde(default)]
    pub launch_chrome_if_needed: bool,
    pub market_hours: (NaiveTime, NaiveTime),
    pub base_ticker: String,
    #[serde(default)]
    pub ignored_stocks: Vec<String>,
}

pub static APP_CONFIG: LazyLock<Config> = LazyLock::new(|| {
    parse_config(Path::new(CONFIG_FILE))
        .unwrap_or_else(|e| panic!("Failed to parse {}: {}", CONFIG_FILE, e))
});

fn parse_config(file: &Path) -> anyhow::Result<Config> {
    let content =
        std::fs::read_to_string(file).with_context(|| format!("Couldn't read {file:?}"))?;
    let config = toml::from_str(&content)
        .with_context(|| format!("Couldn't parse into config:\n{content}"))?;
    Ok(config)
}

#[cfg(test)]
mod test {
    use crate::config::Config;
    use chrono::NaiveTime;

    #[test]
    fn print_config() {
        let config = Config {
            log_config: "".into(),
            chrome_path: "".into(),
            user_data_dir: "".into(),
            chrome_args: Vec::new(),
            launch_chrome_if_needed: false,
            market_hours: (
                NaiveTime::from_hms_opt(6, 30, 0).unwrap(),
                NaiveTime::from_hms_opt(11, 00, 0).unwrap(),
            ),
            base_ticker: "QQQ".into(),
            ignored_stocks: Vec::new(),
        };
        eprintln!("Config:\n:{}", toml::to_string_pretty(&config).unwrap());
    }
}
