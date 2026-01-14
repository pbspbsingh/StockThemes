use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};

use anyhow::Context;
use log::{debug, info};

pub fn parse_stocks(csv_file: &Path, skip_lines: usize) -> anyhow::Result<Vec<String>> {
    let csv_file = csv_file
        .canonicalize()
        .with_context(|| format!("Failed to read {csv_file:?}"))?;
    debug!("Reading {csv_file:?}");

    let file = File::open(&csv_file).with_context(|| format!("Couldn't read {csv_file:?}"))?;
    let mut reader = BufReader::new(file);
    let mut buff = String::new();
    let mut lines = 0;
    let mut result = Vec::new();
    while reader.read_line(&mut buff)? > 0 {
        lines += 1;
        if lines <= skip_lines {
            buff.clear();
            continue;
        }

        if let Some(stock) = buff.trim().split(',').next() {
            result.push(stock.trim().to_uppercase());
        }
        buff.clear();
    }
    info!("processed {} lines, found {} stocks", lines, result.len());
    Ok(result)
}
