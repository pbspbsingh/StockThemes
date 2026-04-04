use anyhow::{Context, anyhow};
use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use clap::Parser;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "trade_analyzer")]
#[command(about = "Analyze ThinkorSwim trade export CSV")]
struct Args {
    /// Input ThinkorSwim CSV export file
    pub input: PathBuf,

    /// Output CSV file
    #[arg(short, long, default_value = "trade_analysis.csv")]
    pub output: PathBuf,
}

#[derive(Debug, Clone, PartialEq)]
enum Side {
    Buy,
    Sell,
}

#[derive(Debug, Clone, PartialEq)]
enum PosEffect {
    Open,
    Close,
}

#[derive(Debug, Clone)]
struct Fill {
    exec_time: NaiveDateTime,
    side: Side,
    qty: u32,
    pos_effect: PosEffect,
    symbol: String,
    price: f64,
}

struct Trade {
    ticker: String,
    open_time: NaiveDateTime,
    close_time: Option<NaiveDateTime>,
    qty: u32,
    fills: Vec<Fill>,
    fees: f64,
}

impl Trade {
    fn is_open(&self) -> bool {
        self.close_time.is_none()
    }

    fn duration_str(&self) -> String {
        let close = match self.close_time {
            Some(t) => t,
            None => return "-".to_string(),
        };
        let hours = (close - self.open_time).num_seconds().max(0) as f64 / 3600.0;
        if hours < 24.0 {
            format!("{:.1}h", hours)
        } else if hours < 720.0 {
            format!("{:.1}d", hours / 24.0)
        } else {
            format!("{:.1}mo", hours / 720.0)
        }
    }

    fn pnl_usd(&self) -> Option<f64> {
        if self.is_open() {
            return None;
        }
        let sell: f64 = self.fills.iter()
            .filter(|f| f.side == Side::Sell)
            .map(|f| f.qty as f64 * f.price)
            .sum();
        let buy: f64 = self.fills.iter()
            .filter(|f| f.side == Side::Buy)
            .map(|f| f.qty as f64 * f.price)
            .sum();
        Some(sell - buy - self.fees)
    }

    fn pnl_pct(&self) -> Option<f64> {
        let pnl = self.pnl_usd()?;
        let cost: f64 = self.fills.iter()
            .filter(|f| f.pos_effect == PosEffect::Open)
            .map(|f| f.qty as f64 * f.price)
            .sum();
        if cost == 0.0 { None } else { Some(pnl / cost * 100.0) }
    }
}

fn parse_datetime(s: &str) -> anyhow::Result<NaiveDateTime> {
    let s = s.trim();
    let (date_str, time_str) = s.split_once(' ')
        .ok_or_else(|| anyhow!("Invalid datetime '{s}'"))?;
    let mut parts = date_str.split('/');
    let month: u32 = parts.next().context("missing month")?.trim().parse()?;
    let day: u32 = parts.next().context("missing day")?.trim().parse()?;
    let year: i32 = parts.next().context("missing year")?.trim().parse::<i32>()? + 2000;
    let date = NaiveDate::from_ymd_opt(year, month, day)
        .ok_or_else(|| anyhow!("Invalid date in '{s}'"))?;
    let time = NaiveTime::parse_from_str(time_str.trim(), "%H:%M:%S")
        .with_context(|| format!("Invalid time '{time_str}'"))?;
    Ok(NaiveDateTime::new(date, time))
}

fn parse_csv_line(line: &str) -> Vec<String> {
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
    // "BOT +40 ERAS @10.77" or "SOLD -13 ERAS @10.43"
    let mut it = desc.split_whitespace();
    match it.next()? {
        "BOT" | "SOLD" => {}
        _ => return None,
    }
    it.next()?; // qty with sign
    it.next()   // ticker
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let content = fs::read_to_string(&args.input)
        .with_context(|| format!("Cannot read '{}'", args.input.display()))?;

    let mut fills: Vec<Fill> = Vec::new();
    let mut fees_by_ticker: HashMap<String, f64> = HashMap::new();

    #[derive(PartialEq)]
    enum Section {
        Other,
        CashBalance,
        TradeHistory,
    }

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
                // DATE,TIME,TYPE,REF #,DESCRIPTION,Misc Fees,Commissions & Fees,AMOUNT,BALANCE
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
                fills.push(Fill { exec_time, side, qty, pos_effect, symbol, price });
            }
            Section::Other => {}
        }
    }

    // Trade History is stored in reverse chronological order in the file
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
            // If we see a TO CLOSE on a flat position, the position was opened before
            // the report's date range. Infer the starting size from this fill.
            if fill.pos_effect == PosEffect::Close && net == 0 {
                net = if fill.side == Side::Sell {
                    fill.qty as i64  // was long
                } else {
                    -(fill.qty as i64) // was short
                };
            }

            current.push(fill.clone());
            if fill.pos_effect == PosEffect::Open {
                open_qty += fill.qty;
            }

            net += if fill.side == Side::Buy { fill.qty as i64 } else { -(fill.qty as i64) };

            if net == 0 {
                let open_time = current.iter()
                    .filter(|f| f.pos_effect == PosEffect::Open)
                    .map(|f| f.exec_time)
                    .min()
                    .unwrap_or(current[0].exec_time);
                let close_time = current.iter()
                    .filter(|f| f.pos_effect == PosEffect::Close)
                    .map(|f| f.exec_time)
                    .max();
                // For pre-existing positions qty=0; fall back to total close qty
                if open_qty == 0 {
                    open_qty = current.iter()
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

        // Any remaining fills represent an open position
        if !current.is_empty() {
            let open_time = current.iter()
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

    // Distribute total ticker fees across trades proportionally by open qty
    let mut total_open_qty: HashMap<String, u32> = HashMap::new();
    for trade in &trades {
        *total_open_qty.entry(trade.ticker.clone()).or_default() += trade.qty;
    }
    for trade in &mut trades {
        let total_fee = fees_by_ticker.get(&trade.ticker).copied().unwrap_or(0.0);
        let total_qty = *total_open_qty.get(&trade.ticker).unwrap_or(&1).max(&1) as f64;
        trade.fees = total_fee * trade.qty as f64 / total_qty;
    }

    trades.sort_by_key(|t| t.open_time);

    let mut out = String::from("TICKER,OPEN_DATE,QTY,STATUS,DURATION,PNL_PCT,PNL_USD\n");
    for trade in &trades {
        let status = if trade.is_open() { "OPEN" } else { "CLOSED" };
        let pnl_pct = trade.pnl_pct()
            .map(|p| format!("{:.2}%", p))
            .unwrap_or_else(|| "-".to_string());
        let pnl_usd = trade.pnl_usd()
            .map(|p| format!("{:.2}", p))
            .unwrap_or_else(|| "-".to_string());
        out.push_str(&format!(
            "{},{},{},{},{},{},{}\n",
            trade.ticker,
            trade.open_time.format("%Y-%m-%d %H:%M:%S"),
            trade.qty,
            status,
            trade.duration_str(),
            pnl_pct,
            pnl_usd,
        ));
    }

    fs::write(&args.output, &out)
        .with_context(|| format!("Cannot write '{}'", args.output.display()))?;

    let total = trades.len();
    let open = trades.iter().filter(|t| t.is_open()).count();
    println!("Wrote {total} trades ({open} open) to {}", args.output.display());

    Ok(())
}
