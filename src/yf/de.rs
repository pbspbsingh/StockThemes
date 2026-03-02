use serde::Deserialize;

// ============================================================================
// /v8/finance/chart deserialization
// ============================================================================

#[derive(Debug, Deserialize)]
pub(super) struct ChartResponse {
    pub(super) chart: ChartResult,
}

#[derive(Debug, Deserialize)]
pub(super) struct ChartResult {
    pub(super) result: Option<Vec<ChartData>>,
    pub(super) error: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ChartData {
    pub(super) timestamp: Option<Vec<i64>>,
    pub(super) indicators: Indicators,
}

#[derive(Debug, Deserialize)]
pub(super) struct Indicators {
    pub(super) quote: Vec<QuoteIndicator>,
    /// Only present for daily/weekly bars when `includeAdjustedClose=true`.
    pub(super) adjclose: Option<Vec<AdjCloseWrapper>>,
}

#[derive(Debug, Deserialize)]
pub(super) struct AdjCloseWrapper {
    pub(super) adjclose: Option<Vec<Option<f64>>>,
}

#[derive(Debug, Deserialize)]
pub(super) struct QuoteIndicator {
    pub(super) open: Option<Vec<Option<f64>>>,
    pub(super) high: Option<Vec<Option<f64>>>,
    pub(super) low: Option<Vec<Option<f64>>>,
    pub(super) close: Option<Vec<Option<f64>>>,
    pub(super) volume: Option<Vec<Option<u64>>>,
}

// ============================================================================
// /v10/finance/quoteSummary deserialization
// ============================================================================

#[derive(Debug, Deserialize)]
pub(super) struct QuoteSummaryResponse {
    #[serde(rename = "quoteSummary")]
    pub(super) quote_summary: QuoteSummary,
}

#[derive(Debug, Deserialize)]
pub(super) struct QuoteSummary {
    // Yahoo returns `"result": null` on errors — must be Option.
    pub(super) result: Option<Vec<QuoteSummaryResult>>,
}

#[derive(Debug, Deserialize)]
pub(super) struct QuoteSummaryResult {
    #[serde(rename = "assetProfile")]
    pub(super) asset_profile: Option<AssetProfile>,
    pub(super) price: Option<Price>,
}

#[derive(Debug, Deserialize)]
pub(super) struct AssetProfile {
    pub(super) sector: Option<String>,
    pub(super) industry: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct Price {
    #[serde(rename = "exchangeName")]
    pub(super) exchange_name: Option<String>,
    #[serde(rename = "exchangeCode")]
    pub(super) exchange_code: Option<String>,
}
