pub mod candles;
pub mod parser;
pub mod routes;

use chrono::{DateTime, Local, NaiveDate, TimeDelta, Timelike, Utc};
use serde::Serialize;

// ── Core types (used by parser and routes) ───────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Side {
    Buy,
    Sell,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum PosEffect {
    Open,
    Close,
}

#[derive(Debug, Clone, Serialize)]
pub struct Fill {
    pub exec_time: DateTime<Utc>,
    pub side: Side,
    pub qty: u32,
    pub pos_effect: PosEffect,
    pub symbol: String,
    pub price: f64,
}

#[derive(Debug, Clone)]
pub struct Trade {
    pub ticker: String,
    pub open_time: DateTime<Utc>,
    pub close_time: Option<DateTime<Utc>>,
    pub qty: u32,
    pub fills: Vec<Fill>,
    pub fees: f64,
}

impl Trade {
    pub fn is_open(&self) -> bool {
        self.close_time.is_none()
    }

    pub fn is_long(&self) -> bool {
        self.fills
            .iter()
            .find(|f| f.pos_effect == PosEffect::Open)
            .map(|f| f.side == Side::Buy)
            .unwrap_or(true)
    }

    pub fn duration_str(&self) -> String {
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

    pub fn pnl_usd(&self) -> Option<f64> {
        if self.is_open() {
            return None;
        }
        let has_open_fills = self.fills.iter().any(|f| f.pos_effect == PosEffect::Open);
        if !has_open_fills {
            return None;
        }
        let sell: f64 = self
            .fills
            .iter()
            .filter(|f| f.side == Side::Sell)
            .map(|f| f.qty as f64 * f.price)
            .sum();
        let buy: f64 = self
            .fills
            .iter()
            .filter(|f| f.side == Side::Buy)
            .map(|f| f.qty as f64 * f.price)
            .sum();
        Some(sell - buy - self.fees)
    }

    pub fn pnl_pct(&self) -> Option<f64> {
        let pnl = self.pnl_usd()?;
        let cost: f64 = self
            .fills
            .iter()
            .filter(|f| f.pos_effect == PosEffect::Open)
            .map(|f| f.qty as f64 * f.price)
            .sum();
        if cost == 0.0 {
            None
        } else {
            Some(pnl / cost * 100.0)
        }
    }

    /// Individual entry fill markers with UTC timestamps (for the daily chart).
    pub fn entry_markers(&self) -> Vec<FillMarker> {
        self.fills
            .iter()
            .filter(|f| f.pos_effect == PosEffect::Open)
            .map(|f| FillMarker {
                time: f.exec_time.timestamp(),
                price: f.price,
                qty: f.qty,
            })
            .collect()
    }

    /// Individual entry fill markers with local-time timestamps (for the hourly chart).
    pub fn entry_markers_hourly(&self) -> Vec<FillMarker> {
        let tz_offset = Local::now().offset().local_minus_utc() as i64;
        self.fills
            .iter()
            .filter(|f| f.pos_effect == PosEffect::Open)
            .map(|f| FillMarker {
                time: f.exec_time.timestamp() + tz_offset,
                price: f.price,
                qty: f.qty,
            })
            .collect()
    }

    /// Exit markers deduplicated by calendar day (for the daily chart).
    pub fn exit_markers_daily(&self) -> Vec<FillMarker> {
        use std::collections::HashMap;
        let mut by_day: HashMap<NaiveDate, (f64, u32)> = HashMap::new();
        for f in self
            .fills
            .iter()
            .filter(|f| f.pos_effect == PosEffect::Close)
        {
            let day = f.exec_time.date_naive();
            let entry = by_day.entry(day).or_default();
            entry.0 += f.price * f.qty as f64;
            entry.1 += f.qty;
        }
        let mut markers: Vec<FillMarker> = by_day
            .into_iter()
            .map(|(day, (total, qty))| FillMarker {
                time: day.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp(),
                price: total / qty as f64,
                qty,
            })
            .collect();
        markers.sort_by_key(|m| m.time);
        markers
    }

    /// Exit markers deduplicated by clock hour with local-time timestamps (for the hourly chart).
    pub fn exit_markers_hourly(&self) -> Vec<FillMarker> {
        use std::collections::HashMap;
        let tz_offset = Local::now().offset().local_minus_utc() as i64;
        let mut by_hour: HashMap<DateTime<Utc>, (f64, u32)> = HashMap::new();
        for f in self
            .fills
            .iter()
            .filter(|f| f.pos_effect == PosEffect::Close)
        {
            let hour = f.exec_time.with_minute(0).unwrap().with_second(0).unwrap();
            let entry = by_hour.entry(hour).or_default();
            entry.0 += f.price * f.qty as f64;
            entry.1 += f.qty;
        }
        let mut markers: Vec<FillMarker> = by_hour
            .into_iter()
            .map(|(hour, (total, qty))| FillMarker {
                time: hour.timestamp() + tz_offset,
                price: total / qty as f64,
                qty,
            })
            .collect();
        markers.sort_by_key(|m| m.time);
        markers
    }
}

// ── View builder ─────────────────────────────────────────────────────────────

pub struct ViewsResult {
    pub trade_views: Vec<TradeView>,
}

pub fn build_views(trades: &[Trade], cfg: &crate::config::TradeAnalysisConfig) -> ViewsResult {
    let daily_days = cfg.daily_chart_days as i64;
    let daily_post = cfg.daily_chart_post_days as i64;
    let hourly_days = cfg.hourly_chart_days as i64;
    let hourly_post = cfg.hourly_chart_post_days as i64;

    let trade_views = trades
        .iter()
        .map(|trade| {
            let exit = trade.close_time.unwrap_or_else(Utc::now);

            let daily_from = (trade.open_time - TimeDelta::days(daily_days)).date_naive();
            let daily_to = (exit + TimeDelta::days(daily_post)).date_naive();
            let hourly_from = trade.open_time - TimeDelta::days(hourly_days);
            let hourly_to = exit + TimeDelta::days(hourly_post);

            TradeView {
                ticker: trade.ticker.clone(),
                open_date: trade
                    .open_time
                    .with_timezone(&Local)
                    .format("%H:%M")
                    .to_string(),
                month_label: trade
                    .open_time
                    .with_timezone(&Local)
                    .format("%B %Y")
                    .to_string(),
                day_label: trade
                    .open_time
                    .with_timezone(&Local)
                    .format("%a %b %-d")
                    .to_string(),
                qty: trade.qty,
                status: if trade.is_open() { "OPEN" } else { "CLOSED" }.to_string(),
                duration: trade.duration_str(),
                pnl_usd: trade.pnl_usd(),
                pnl_pct: trade.pnl_pct(),
                is_long: trade.is_long(),
                daily_from: daily_from
                    .and_hms_opt(0, 0, 0)
                    .unwrap()
                    .and_utc()
                    .timestamp(),
                daily_to: daily_to
                    .and_hms_opt(23, 59, 59)
                    .unwrap()
                    .and_utc()
                    .timestamp(),
                hourly_from: hourly_from.timestamp(),
                hourly_to: hourly_to.timestamp(),
                entry_markers: trade.entry_markers(),
                entry_markers_hourly: trade.entry_markers_hourly(),
                exit_markers_daily: trade.exit_markers_daily(),
                exit_markers_hourly: trade.exit_markers_hourly(),
            }
        })
        .collect();

    ViewsResult { trade_views }
}

// ── Serialisable view sent to the frontend ───────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct FillMarker {
    pub time: i64,
    pub price: f64,
    pub qty: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct TradeView {
    pub ticker: String,
    pub open_date: String,
    pub month_label: String,
    pub day_label: String,
    pub qty: u32,
    pub status: String,
    pub duration: String,
    pub pnl_usd: Option<f64>,
    pub pnl_pct: Option<f64>,
    pub is_long: bool,
    /// Unix timestamps bounding the daily chart window
    pub daily_from: i64,
    pub daily_to: i64,
    /// Unix timestamps bounding the hourly chart window
    pub hourly_from: i64,
    pub hourly_to: i64,
    pub entry_markers: Vec<FillMarker>,
    pub entry_markers_hourly: Vec<FillMarker>,
    pub exit_markers_daily: Vec<FillMarker>,
    pub exit_markers_hourly: Vec<FillMarker>,
}
