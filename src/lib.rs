use std::{collections::HashSet, path::PathBuf};

use chrono::NaiveDate;
use clap::Parser;
use itertools::Itertools;
use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};

pub mod browser;
pub mod config;
pub mod store;
pub mod template;
pub mod tv;
pub mod util;

#[derive(Parser, Debug)]
#[command(name = "stock_themes")]
#[command(about = "Process csv files with stocks to find the common themes among them")]
pub struct StockThemesArgs {
    /// Input files to process
    #[arg(required = true)]
    pub files: Vec<PathBuf>,

    /// Number of items to skip
    #[arg(short = 'n', long, default_value_t = 4)]
    pub skip_lines: usize,

    /// Comma seperated list of Stocks to skip
    #[arg(short = 's', long, default_value = "")]
    pub skip_stocks: String,

    /// Stock store file to save stock info
    #[arg(long, default_value = "stock_store.toml")]
    pub stock_store: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Stock {
    pub ticker: String,
    pub exchange: String,
    pub sector: Group,
    pub industry: Group,
    pub last_update: NaiveDate,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Group {
    pub name: String,
    pub url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Summary {
    pub size: usize,
    pub sectors: Vec<SummarySector>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SummarySector {
    pub name: String,
    pub url: String,
    pub size: usize,
    pub industries: Vec<SummaryIndustry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SummaryIndustry {
    pub name: String,
    pub url: String,
    pub size: usize,
    pub tickers: Vec<Ticker>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Ticker {
    pub exchange: String,
    pub ticker: String,
}

pub fn read_stocks(args: &StockThemesArgs) -> anyhow::Result<Vec<String>> {
    let skips = args
        .skip_stocks
        .split(',')
        .map(str::trim)
        .map(str::to_uppercase)
        .collect::<HashSet<_>>();
    let mut stocks = args
        .files
        .iter()
        .map(|file| util::parse_stocks(file, args.skip_lines))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .filter(|s| !skips.contains(s))
        .unique()
        .collect_vec();
    stocks.shuffle(&mut rand::rng());
    Ok(stocks)
}

pub fn summarize(stocks: Vec<Stock>) -> Summary {
    let mut size = 0;
    let mut sectors = Vec::new();

    let sectors_map = stocks
        .into_iter()
        .into_group_map_by(|stock| stock.sector.name.clone());
    for (sector_name, stocks) in sectors_map {
        if stocks.is_empty() {
            continue;
        }

        let mut sector_summary = SummarySector {
            name: sector_name,
            url: stocks[0].sector.url.clone(),
            size: 0,
            industries: Vec::new(),
        };

        let industry_map = stocks
            .into_iter()
            .into_group_map_by(|stock| stock.industry.name.clone());
        for (industry_name, stocks) in industry_map {
            if stocks.is_empty() {
                continue;
            }

            let industry_summary = SummaryIndustry {
                name: industry_name,
                url: stocks[0].industry.url.clone(),
                size: stocks.len(),
                tickers: stocks
                    .into_iter()
                    .map(|s| Ticker {
                        exchange: s.exchange,
                        ticker: s.ticker,
                    })
                    .collect(),
            };
            sector_summary.size += industry_summary.size;
            sector_summary.industries.push(industry_summary);
        }
        sector_summary
            .industries
            .sort_by_key(|si| -(si.size as isize));
        if sector_summary.size > 0 {
            size += sector_summary.size;
            sectors.push(sector_summary);
        }
    }
    sectors.sort_by_key(|ss| -(ss.size as isize));
    Summary { size, sectors }
}
