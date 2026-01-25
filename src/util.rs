use anyhow::Context;
use log::{debug, info};
use std::path::Path;
use tokio::fs;

pub async fn parse_stocks(
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
                .map(|stock| stock.trim().to_uppercase())
        })
        .collect();

    let total_lines = content.lines().count();
    info!(
        "Processed {} lines, found {} stocks",
        total_lines,
        result.len()
    );

    Ok(result)
}
