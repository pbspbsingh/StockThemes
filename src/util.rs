use anyhow::Context;
use chrono::{DateTime, Datelike, Local, Months, TimeDelta, Weekday};
use futures::stream;
use itertools::Itertools;
use log::{debug, info};
use std::collections::HashMap;
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};
use tokio::fs;

use futures::{StreamExt, TryStreamExt};

use rand::seq::SliceRandom;

use crate::Performance;
use crate::config::APP_CONFIG;
use crate::yf::Candle;

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
        result.len(),
    );

    Ok(result)
}

pub fn is_upto_date(time: DateTime<Local>) -> bool {
    let now = Local::now();
    let (market_open, market_close) = APP_CONFIG.market_hours;

    let is_market_open = || match now.weekday() {
        Weekday::Sat | Weekday::Sun => false,
        _ => {
            let t = now.time();
            t >= market_open && t < market_close
        }
    };

    let last_market_close = || {
        let mut candidate = now.date_naive();

        loop {
            match candidate.weekday() {
                Weekday::Sat | Weekday::Sun => {
                    candidate -= TimeDelta::days(1);
                }
                _ => {
                    let close_dt = candidate
                        .and_time(market_close)
                        .and_local_timezone(Local)
                        .unwrap();
                    if close_dt <= now {
                        break close_dt;
                    }
                    candidate -= TimeDelta::days(1);
                }
            }
        }
    };

    if is_market_open() {
        return false;
    }
    time >= last_market_close()
}

pub fn normalize(input: &str) -> String {
    // Step 1: Normalize unicode slash lookalikes to ASCII '/'
    let normalized = input.replace(['\u{FF0F}', '\u{2044}', '\u{2215}', '\u{29F8}'], "/");

    // Step 2: Normalize all Unicode whitespace to ASCII space
    let normalized = normalized
        .chars()
        .map(|ch| if ch.is_whitespace() { ' ' } else { ch })
        .collect::<String>();

    // Step 3: Trim and collapse multiple consecutive spaces into one
    let normalized = normalized
        .split(' ')
        .filter(|s| !s.is_empty())
        .join(" ");

    // Step 4: Title-case, treating ' ' and '/' as word boundaries
    let mut result = String::with_capacity(normalized.len());
    let mut capitalize_next = true;

    for ch in normalized.chars() {
        if ch == ' ' || ch == '/' {
            result.push(ch);
            capitalize_next = true;
        } else if capitalize_next {
            result.extend(ch.to_uppercase());
            capitalize_next = false;
        } else {
            result.extend(ch.to_lowercase());
        }
    }

    result
}

pub fn parse_percentage(s: impl AsRef<str>) -> anyhow::Result<f64> {
    let s = s.as_ref();
    let normalized = s
        .trim()
        .replace('−', "-") // U+2212 mathematical minus → ASCII hyphen
        .replace('+', "")
        .replace('%', "")
        .replace(',', "");

    normalized
        .parse::<f64>()
        .with_context(|| format!("Failed to parse percentage: {s:?}"))
}

pub fn compute_perf(candles: &[Candle]) -> HashMap<String, f64> {
    if candles.is_empty() {
        return HashMap::new();
    }

    let latest = candles.last().unwrap();

    let closest_close = |months_ago: u32| -> f64 {
        let target = latest.timestamp - Months::new(months_ago);
        candles
            .iter()
            .min_by_key(|c| (c.timestamp - target).num_seconds().abs())
            .map(|c| (latest.close - c.close) / c.close * 100.0)
            .unwrap_or(0.0)
    };

    HashMap::from([
        ("1M".to_string(), closest_close(1)),
        ("3M".to_string(), closest_close(3)),
        ("6M".to_string(), closest_close(6)),
        ("1Y".to_string(), closest_close(12)),
    ])
}

pub fn compute_rs(perf: &Performance, base: &Performance) -> f64 {
    fn multiplier(p: &Performance) -> f64 {
        1.0 + (p.perf_1m * 0.3 + p.perf_3m * 0.4 + p.perf_6m * 0.2 + p.perf_1y * 0.1) / 100.0
    }

    multiplier(perf) / multiplier(base)
}
