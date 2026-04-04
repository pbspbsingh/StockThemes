use anyhow::{Context, anyhow};
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use chrono_tz::America::New_York;
use std::collections::HashMap;

use super::{Fill, PosEffect, Side, Trade};

// ── CSV helpers ───────────────────────────────────────────────────────────────

/// Parse a ThinkorSwim datetime string (Eastern Time) and return UTC.
pub fn parse_datetime(s: &str) -> anyhow::Result<DateTime<Utc>> {
    let s = s.trim();
    let (date_str, time_str) = s
        .split_once(' ')
        .ok_or_else(|| anyhow!("Invalid datetime '{s}'"))?;
    let mut parts = date_str.split('/');
    let month: u32 = parts.next().context("missing month")?.trim().parse()?;
    let day: u32 = parts.next().context("missing day")?.trim().parse()?;
    let year: i32 = parts
        .next()
        .context("missing year")?
        .trim()
        .parse::<i32>()?
        + 2000;
    let date = NaiveDate::from_ymd_opt(year, month, day)
        .ok_or_else(|| anyhow!("Invalid date in '{s}'"))?;
    let time = NaiveTime::parse_from_str(time_str.trim(), "%H:%M:%S")
        .with_context(|| format!("Invalid time '{time_str}'"))?;
    let naive = NaiveDateTime::new(date, time);
    New_York
        .from_local_datetime(&naive)
        .single()
        .map(|dt| dt.with_timezone(&Utc))
        .ok_or_else(|| anyhow!("Ambiguous or invalid ET datetime '{s}'"))
}

pub fn parse_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                if in_quotes && chars.peek() == Some(&'"') {
                    chars.next();
                    current.push('"');
                } else {
                    in_quotes = !in_quotes;
                }
            }
            ',' if !in_quotes => fields.push(std::mem::take(&mut current)),
            c => current.push(c),
        }
    }
    fields.push(current);
    fields
}

fn parse_fee(s: &str) -> f64 {
    let s = s.trim();
    if s.is_empty() || s == "--" {
        return 0.0;
    }
    let cleaned: String = s.chars().filter(|&c| c != ',').collect();
    cleaned.parse::<f64>().unwrap_or(0.0).abs()
}

fn ticker_from_description(desc: &str) -> Option<&str> {
    let mut it = desc.split_whitespace();
    match it.next()? {
        "BOT" | "SOLD" => {}
        _ => return None,
    }
    it.next()?;
    it.next()
}

// ── Section parser ────────────────────────────────────────────────────────────

#[derive(PartialEq)]
enum Section {
    Other,
    CashBalance,
    TradeHistory,
}

/// Parse a ThinkorSwim account-statement CSV and return grouped trades,
/// sorted by open time ascending.
pub fn parse_tos_csv(content: &str) -> Vec<Trade> {
    let mut fills: Vec<Fill> = Vec::new();
    let mut fees_by_ticker: HashMap<String, f64> = HashMap::new();

    let mut section = Section::Other;
    let mut skip_next = false;

    for line in content.lines() {
        let line = line.trim();

        if skip_next {
            skip_next = false;
            continue;
        }

        match line {
            "Cash Balance" => {
                section = Section::CashBalance;
                skip_next = true;
                continue;
            }
            "Account Trade History" => {
                section = Section::TradeHistory;
                skip_next = true;
                continue;
            }
            "Futures Statements"
            | "Forex Statements"
            | "Account Order History"
            | "Profits and Losses" => {
                section = Section::Other;
                continue;
            }
            _ if line.is_empty() => continue,
            _ => {}
        }

        match section {
            Section::CashBalance => {
                // DATE,TIME,TYPE,REF #,DESCRIPTION,Misc Fees,Commissions & Fees,...
                let f = parse_csv_line(line);
                if f.len() < 7 || f[2].trim() != "TRD" {
                    continue;
                }
                let Some(ticker) = ticker_from_description(f[4].trim()) else {
                    continue;
                };
                let fee = parse_fee(&f[5]) + parse_fee(&f[6]);
                if fee > 0.0 {
                    *fees_by_ticker.entry(ticker.to_string()).or_default() += fee;
                }
            }
            Section::TradeHistory => {
                // ,Exec Time,Spread,Side,Qty,Pos Effect,Symbol,Exp,Strike,Type,Price,...
                let f = parse_csv_line(line);
                if f.len() < 11 {
                    continue;
                }
                let exec_time = match parse_datetime(f[1].trim()) {
                    Ok(t) => t,
                    Err(_) => continue,
                };
                let side = match f[3].trim() {
                    "BUY" => Side::Buy,
                    "SELL" => Side::Sell,
                    _ => continue,
                };
                let qty: u32 = match f[4].trim().trim_start_matches('+').parse::<i32>() {
                    Ok(q) => q.unsigned_abs(),
                    Err(_) => continue,
                };
                let pos_effect = match f[5].trim() {
                    "TO OPEN" => PosEffect::Open,
                    "TO CLOSE" => PosEffect::Close,
                    _ => continue,
                };
                let symbol = f[6].trim().to_string();
                if symbol.is_empty() {
                    continue;
                }
                let price: f64 = match f[10].trim().parse() {
                    Ok(p) => p,
                    Err(_) => continue,
                };
                fills.push(Fill {
                    exec_time,
                    side,
                    qty,
                    pos_effect,
                    symbol,
                    price,
                });
            }
            Section::Other => {}
        }
    }

    // Trade History is reverse-chronological in the file
    fills.sort_by_key(|f| f.exec_time);

    let mut by_symbol: HashMap<String, Vec<Fill>> = HashMap::new();
    for fill in fills {
        by_symbol.entry(fill.symbol.clone()).or_default().push(fill);
    }

    let mut trades: Vec<Trade> = Vec::new();

    for (symbol, symbol_fills) in &by_symbol {
        let mut net: i64 = 0;
        let mut current: Vec<Fill> = Vec::new();
        let mut open_qty: u32 = 0;

        for fill in symbol_fills {
            // Position opened before the report's date range — infer starting size.
            if fill.pos_effect == PosEffect::Close && net == 0 {
                net = if fill.side == Side::Sell {
                    fill.qty as i64
                } else {
                    -(fill.qty as i64)
                };
            }

            current.push(fill.clone());
            if fill.pos_effect == PosEffect::Open {
                open_qty += fill.qty;
            }

            net += if fill.side == Side::Buy {
                fill.qty as i64
            } else {
                -(fill.qty as i64)
            };

            if net == 0 {
                let open_time = current
                    .iter()
                    .filter(|f| f.pos_effect == PosEffect::Open)
                    .map(|f| f.exec_time)
                    .min()
                    .unwrap_or(current[0].exec_time);
                let close_time = current
                    .iter()
                    .filter(|f| f.pos_effect == PosEffect::Close)
                    .map(|f| f.exec_time)
                    .max();
                if open_qty == 0 {
                    open_qty = current
                        .iter()
                        .filter(|f| f.pos_effect == PosEffect::Close)
                        .map(|f| f.qty)
                        .sum();
                }
                trades.push(Trade {
                    ticker: symbol.clone(),
                    open_time,
                    close_time,
                    qty: open_qty,
                    fills: std::mem::take(&mut current),
                    fees: 0.0,
                });
                open_qty = 0;
            }
        }

        if !current.is_empty() {
            let open_time = current
                .iter()
                .filter(|f| f.pos_effect == PosEffect::Open)
                .map(|f| f.exec_time)
                .min()
                .unwrap_or(current[0].exec_time);
            trades.push(Trade {
                ticker: symbol.clone(),
                open_time,
                close_time: None,
                qty: open_qty,
                fills: current,
                fees: 0.0,
            });
        }
    }

    // Distribute fees proportionally by trade qty
    let mut total_qty: HashMap<String, u32> = HashMap::new();
    for t in &trades {
        *total_qty.entry(t.ticker.clone()).or_default() += t.qty;
    }
    for t in &mut trades {
        let total_fee = fees_by_ticker.get(&t.ticker).copied().unwrap_or(0.0);
        let denom = (*total_qty.get(&t.ticker).unwrap_or(&1)).max(1) as f64;
        t.fees = total_fee * t.qty as f64 / denom;
    }

    trades.sort_by_key(|t| t.open_time);
    trades
}

/// Write the analysis CSV to a string.
pub fn trades_to_csv(trades: &[Trade]) -> String {
    let mut out = String::from("TICKER,OPEN_DATE,QTY,STATUS,DURATION,PNL_PCT,PNL_USD\n");
    for t in trades {
        let status = if t.is_open() { "OPEN" } else { "CLOSED" };
        let pnl_pct = t
            .pnl_pct()
            .map(|p| format!("{:.2}%", p))
            .unwrap_or_else(|| "-".to_string());
        let pnl_usd = t
            .pnl_usd()
            .map(|p| format!("{:.2}", p))
            .unwrap_or_else(|| "-".to_string());
        out.push_str(&format!(
            "{},{},{},{},{},{},{}\n",
            t.ticker,
            t.open_time
                .with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S"),
            t.qty,
            status,
            t.duration_str(),
            pnl_pct,
            pnl_usd,
        ));
    }
    out
}
