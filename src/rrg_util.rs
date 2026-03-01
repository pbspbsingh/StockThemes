use crate::config::APP_CONFIG;
use crate::html_error::HtmlError;
use crate::store::Store;
use crate::yf::{Candle, YFinance};
use crate::{etf_map, fetch_candles};
use anyhow::Context;
use askama::Template;
use axum::response::{Html, IntoResponse};
use axum::{
    Json,
    extract::{Path, Query},
};
use chrono::Datelike;
use log::debug;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

static YF: LazyLock<YFinance> = LazyLock::new(|| YFinance::new());

pub async fn rrg_home() -> Result<impl IntoResponse, HtmlError> {
    #[derive(Template)]
    #[template(path = "rrg.html")]
    struct Home {
        benchmark: String,
        sectors: Vec<etf_map::Sector>,
    }

    let home = Home {
        benchmark: APP_CONFIG.base_ticker.to_uppercase(),
        sectors: etf_map::tv_mapping(),
    };

    Ok(Html(home.render()?))
}

// ── Query params & response types ───────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct RrgQuery {
    /// "daily" or "weekly" — defaults to "weekly"
    timeframe: String,

    /// Number of historical tail points to return (oldest → newest).
    /// Typical values: daily 20/50/100/200, weekly 4/12/26/52.
    tail: usize,

    /// Number of RS-Ratio history points for the bottom chart.
    history: usize,
}

// ────────────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct TailPoint {
    rs_ratio: f64,
    rs_momentum: f64,
}

#[derive(Serialize)]
struct HistoryPoint {
    date: String, // "YYYY-MM-DD"
    value: f64,   // RS-Ratio at that period
}

#[derive(Serialize)]
pub struct RrgResponse {
    ticker: String,
    rs_ratio: f64,
    rs_momentum: f64,
    tail: Vec<TailPoint>,
    rs_history: Vec<HistoryPoint>,
}
// ── Axum handler ─────────────────────────────────────────────────────────────

/// GET /api/rrg/:ticker?timeframe=weekly&tail=12&history=52
pub async fn rrg_handler(
    Path(ticker): Path<String>,
    Query(params): Query<RrgQuery>,
) -> Result<Json<RrgResponse>, HtmlError> {
    debug!("Ticker: {ticker}, params: {params:?}");
    let store = Store::load_store().await?;
    let etf_candles = fetch_candles(&store, &YF, &ticker).await?;
    let bmk_candles = fetch_candles(&store, &YF, &APP_CONFIG.base_ticker).await?;

    if etf_candles.is_empty() || bmk_candles.is_empty() {
        return Err(anyhow::anyhow!(
            "{}/{} candles fetched",
            etf_candles.len(),
            bmk_candles.len(),
        )
        .into());
    }

    let response = compute_rrg(
        &ticker,
        &etf_candles,
        &bmk_candles,
        &params.timeframe,
        params.tail,
        params.history,
    )
    .context("Failed to compute rrg")?;

    Ok(Json(response))
}

// ── Core computation ─────────────────────────────────────────────────────────

/// A single period's worth of data after optional weekly aggregation.
struct PeriodClose {
    date: chrono::NaiveDate,
    close: f64,
}

/// Aggregate daily candles to weekly closes (last trading day of each ISO week).
fn to_weekly(candles: &[Candle]) -> Vec<PeriodClose> {
    use std::collections::BTreeMap;

    // Group by ISO year+week, keep last close per week.
    let mut weeks: BTreeMap<(i32, u32), PeriodClose> = BTreeMap::new();
    for c in candles {
        let date = c.timestamp.date_naive();
        let key = (date.iso_week().year(), date.iso_week().week());
        // Overwrite → last candle in the week wins (latest date = weekly close).
        weeks.insert(
            key,
            PeriodClose {
                date,
                close: c.close,
            },
        );
    }

    weeks.into_values().collect()
}

/// Convert daily candles to PeriodClose (trivial, just extracts date + close).
fn to_daily(candles: &[Candle]) -> Vec<PeriodClose> {
    candles
        .iter()
        .map(|c| PeriodClose {
            date: c.timestamp.date_naive(),
            close: c.close,
        })
        .collect()
}

/// Simple Moving Average.  Returns a vec the same length as `src`.
/// The first `period - 1` values use a shorter window (expanding SMA).
fn sma(src: &[f64], period: usize) -> Vec<f64> {
    src.iter()
        .enumerate()
        .map(|(i, _)| {
            let start = i.saturating_sub(period - 1);
            let window = &src[start..=i];
            window.iter().sum::<f64>() / window.len() as f64
        })
        .collect()
}

/// Round to 3 decimal places — keeps JSON tidy.
#[inline]
fn r3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// Align two period series by date (inner join on date), returning
/// `(etf_closes, bmk_closes)` in chronological order.
fn align(etf: &[PeriodClose], bmk: &[PeriodClose]) -> (Vec<f64>, Vec<chrono::NaiveDate>, Vec<f64>) {
    use std::collections::HashMap;

    let bmk_map: HashMap<chrono::NaiveDate, f64> = bmk.iter().map(|p| (p.date, p.close)).collect();

    let mut etf_closes: Vec<f64> = Vec::new();
    let mut dates: Vec<chrono::NaiveDate> = Vec::new();
    let mut bmk_closes: Vec<f64> = Vec::new();

    for p in etf {
        if let Some(&b) = bmk_map.get(&p.date) {
            if p.close > 0.0 && b > 0.0 {
                etf_closes.push(p.close);
                bmk_closes.push(b);
                dates.push(p.date);
            }
        }
    }

    (etf_closes, dates, bmk_closes)
}

/// JdK RS-Ratio / RS-Momentum computation.
///
/// Formula (Julius de Kempenaer, "Relative Rotation Graphs"):
///
///   1.  rs[i]         = etf_close[i] / benchmark_close[i]
///                       — raw relative strength ratio
///
///   2.  rs_smooth[i]  = SMA(rs, 10)[i]
///                       — reduces daily noise
///
///   3.  rs_ratio[i]   = (rs_smooth[i] / SMA(rs_smooth, 10)[i]) × 100
///                       — normalises around 100 (= benchmark parity)
///
///   4.  rs_momentum[i]= (rs_ratio[i]  / SMA(rs_ratio,  10)[i]) × 100
///                       — rate-of-change of RS-Ratio, also centred at 100
///
/// `tail_len`    — how many historical (rs_ratio, rs_momentum) pairs to return
///                 in the "tail" array (oldest → newest, excludes current point)
/// `history_len` — how many (date, rs_ratio) pairs to return for the bottom chart
fn compute_rrg(
    ticker: &str,
    etf_candles: &[Candle],
    bmk_candles: &[Candle],
    timeframe: &str,
    tail_len: usize,
    history_len: usize,
) -> Option<RrgResponse> {
    // ── 1. Resample ──────────────────────────────────────────────────────────
    let etf_periods = match timeframe {
        "daily" => to_daily(etf_candles),
        _ => to_weekly(etf_candles), // "weekly" is the default
    };
    let bmk_periods = match timeframe {
        "daily" => to_daily(bmk_candles),
        _ => to_weekly(bmk_candles),
    };

    // ── 2. Align by date ─────────────────────────────────────────────────────
    let (etf_close, dates, bmk_close) = align(&etf_periods, &bmk_periods);
    let n = etf_close.len();
    if n < 20 {
        return None; // not enough data to compute meaningful SMAs
    }

    // ── 3. Raw RS ────────────────────────────────────────────────────────────
    let rs: Vec<f64> = etf_close
        .iter()
        .zip(bmk_close.iter())
        .map(|(e, b)| e / b)
        .collect();

    // ── 4. RS-Ratio: smooth RS, then normalise against its own SMA ───────────
    let sma_period = match timeframe {
        "daily" => 50, // 10 weeks × 5 days
        _ => 10,       // 10 weeks (weekly default)
    };

    let rs_smooth = sma(&rs, sma_period);
    let rs_smooth_sma = sma(&rs_smooth, sma_period);

    let rs_ratio: Vec<f64> = rs_smooth
        .iter()
        .zip(rs_smooth_sma.iter())
        .map(|(s, m)| if *m != 0.0 { (s / m) * 100.0 } else { 100.0 })
        .collect();

    // ── 5. RS-Momentum: normalise RS-Ratio against its own SMA ───────────────
    let rs_ratio_sma = sma(&rs_ratio, sma_period);

    let rs_momentum: Vec<f64> = rs_ratio
        .iter()
        .zip(rs_ratio_sma.iter())
        .map(|(r, m)| if *m != 0.0 { (r / m) * 100.0 } else { 100.0 })
        .collect();

    // ── 6. Current values (last data point) ──────────────────────────────────
    let current_rs_ratio = r3(*rs_ratio.last()?);
    let current_rs_momentum = r3(*rs_momentum.last()?);

    // ── 7. Tail (tail_len points immediately before the current point) ────────
    //
    // Layout of the arrays (length = n):
    //   index:  0 … (n-1-tail_len) … (n-2)  (n-1)
    //                                 ^tail   ^current
    //
    let tail_start = n.saturating_sub(tail_len + 1);
    let tail: Vec<TailPoint> = (tail_start..n.saturating_sub(1))
        .map(|i| TailPoint {
            rs_ratio: r3(rs_ratio[i]),
            rs_momentum: r3(rs_momentum[i]),
        })
        .collect();

    // ── 8. RS history for the bottom chart ───────────────────────────────────
    let hist_start = n.saturating_sub(history_len);
    let rs_history: Vec<HistoryPoint> = (hist_start..n)
        .map(|i| HistoryPoint {
            date: dates[i].format("%Y-%m-%d").to_string(),
            value: r3(rs_ratio[i]),
        })
        .collect();

    Some(RrgResponse {
        ticker: ticker.to_uppercase(),
        rs_ratio: current_rs_ratio,
        rs_momentum: current_rs_momentum,
        tail,
        rs_history,
    })
}
