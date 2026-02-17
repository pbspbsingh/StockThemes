use anyhow::Context;
use futures::stream;
use itertools::Itertools;
use log::{debug, info};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};
use tokio::fs;

use futures::{StreamExt, TryStreamExt};

use rand::seq::SliceRandom;

use crate::{Stock, Summary, SummaryIndustry, SummarySector, Ticker, config::APP_CONFIG};

pub async fn read_stocks(
    files: &[PathBuf],
    skip_lines: usize,
    skip_stocks: &str,
) -> anyhow::Result<Vec<String>> {
    let skips = skip_stocks
        .split(',')
        .chain(APP_CONFIG.ignored_stocks.iter().map(|s| s.as_str()))
        .map(str::trim)
        .filter(|&s| !s.is_empty())
        .map(str::to_uppercase)
        .collect::<HashSet<_>>();
    if !skips.is_empty() {
        if skips.len() <= 10 {
            info!("Skipping: [{}]", skips.iter().sorted().join(","));
        } else {
            info!("Skipping {} stocks", skips.len());
        }
    }

    let mut stocks = stream::iter(files)
        .then(|file| parse_stocks(file, skip_lines))
        .try_collect::<Vec<_>>()
        .await?
        .into_iter()
        .flatten()
        .filter(|s| !skips.contains(s))
        .unique()
        .collect_vec();
    stocks.shuffle(&mut rand::rng());
    Ok(stocks)
}

async fn parse_stocks(
    csv_file: impl AsRef<Path>,
    skip_lines: usize,
) -> anyhow::Result<Vec<String>> {
    let csv_file = csv_file.as_ref();
    let csv_file = fs::canonicalize(csv_file)
        .await
        .with_context(|| format!("Failed to canonicalize {csv_file:?}"))?;
    debug!("Reading {csv_file:?}");

    let content = fs::read_to_string(&csv_file)
        .await
        .with_context(|| format!("Couldn't read {csv_file:?}"))?;

    let result: Vec<String> = content
        .lines()
        .skip(skip_lines)
        .filter_map(|line| {
            line.trim()
                .split(',')
                .next()
                .map(str::trim)
                .map(|stock| stock.trim().to_uppercase())
        })
        .filter(|s| !s.is_empty())
        .collect();

    let total_lines = content.lines().count();
    info!(
        "Processed {} lines, found {} stocks",
        total_lines,
        result.len()
    );

    Ok(result)
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
