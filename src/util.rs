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

use crate::config::APP_CONFIG;

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
