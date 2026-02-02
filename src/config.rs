use std::{
    path::{Path, PathBuf},
    sync::LazyLock,
};

use anyhow::Context;

use serde::Deserialize;

const CONFIG_FILE: &str = "config.toml";

#[derive(Debug, Deserialize)]
pub struct Config {
    pub log_config: String,
    pub chrome_path: String,
    pub user_data_dir: PathBuf,
    #[serde(default)]
    pub chrome_args: Vec<String>,
    pub kill_chrome_on_exit: bool,
    pub stock_store_file: PathBuf,
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
        .with_context(|| format!("Couldn't parse into config\n:{content}"))?;
    Ok(config)
}
