use chrono::{DateTime, Local, Utc};
use std::fmt;
use std::fmt::{Display, Formatter};
// ============================================================================
// Candle query types
// ============================================================================

/// The size (and session) of each candle bar.
///
/// `*Ext` variants include pre-market and after-hours data
/// (`includePrePost=true`). Only meaningful for intraday bar sizes —
/// Yahoo ignores the flag for `Daily` and `Weekly`.
#[derive(Debug, Clone, Copy)]
pub enum BarSize {
    Min1,
    Min1Ext,
    Min5,
    Min5Ext,
    Min15,
    Min15Ext,
    Min30,
    Min30Ext,
    Hour1,
    Hour1Ext,
    Daily,
    Weekly,
}

impl BarSize {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            BarSize::Min1 | BarSize::Min1Ext => "1m",
            BarSize::Min5 | BarSize::Min5Ext => "5m",
            BarSize::Min15 | BarSize::Min15Ext => "15m",
            BarSize::Min30 | BarSize::Min30Ext => "30m",
            BarSize::Hour1 | BarSize::Hour1Ext => "1h",
            BarSize::Daily => "1d",
            BarSize::Weekly => "1wk",
        }
    }

    pub(super) fn include_pre_post(self) -> bool {
        matches!(
            self,
            BarSize::Min1Ext
                | BarSize::Min5Ext
                | BarSize::Min15Ext
                | BarSize::Min30Ext
                | BarSize::Hour1Ext
        )
    }
}

impl fmt::Display for BarSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ============================================================================

/// A predefined lookback window understood natively by Yahoo Finance.
#[derive(Debug, Clone, Copy)]
pub enum Range {
    OneDay,
    FiveDay,
    OneMonth,
    ThreeMonths,
    SixMonths,
    OneYear,
    TwoYears,
    FiveYears,
    TenYears,
    Ytd,
    Max,
}

impl Range {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Range::OneDay => "1d",
            Range::FiveDay => "5d",
            Range::OneMonth => "1mo",
            Range::ThreeMonths => "3mo",
            Range::SixMonths => "6mo",
            Range::OneYear => "1y",
            Range::TwoYears => "2y",
            Range::FiveYears => "5y",
            Range::TenYears => "10y",
            Range::Ytd => "ytd",
            Range::Max => "max",
        }
    }
}

// ============================================================================

/// How to specify the time window for a candle request.
///
/// - `Range` — a named lookback window (e.g. `Range::OneMonth`).
/// - `Interval` — an explicit `[start, end]` window using `DateTime<Utc>`.
#[derive(Debug, Clone, Copy)]
pub enum TimeSpec {
    Range(Range),
    Interval(DateTime<Utc>, DateTime<Utc>),
}

// ============================================================================
// Output types
// ============================================================================

#[derive(Debug)]
pub struct TickerInfo {
    pub symbol: String,
    pub exchange: Option<String>,
    pub exchange_code: Option<String>,
    pub sector: Option<String>,
    pub industry: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Candle {
    pub timestamp: DateTime<Utc>,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: u64,
    pub adj_close: Option<f64>, // Only available on daily/weekly candles
    pub last_updated: DateTime<Local>,
}

impl Candle {
    pub fn adj_close(&self) -> f64 {
        self.adj_close.unwrap_or(self.close)
    }
}

impl Display for Candle {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "[O=${:.2}, ", self.open)?;
        write!(f, "L=${:.2}, ", self.low)?;
        write!(f, "H=${:.2}, ", self.high)?;
        write!(f, "C=${:.2}, ", self.close)?;
        if let Some(adj_close) = self.adj_close {
            write!(f, "AC=${:.2}, ", adj_close)?;
        }
        write!(f, "V={}, ", self.volume)?;
        write!(f, "T={}],", self.timestamp)
    }
}
