use askama::Template;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing,
};
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};
use tokio::sync::Mutex;
use tracing::info;

use crate::config::APP_CONFIG;
use crate::html_error::HtmlError;
use crate::store::{CompanyProfile, DeleteTagResult, Store, Tag};
use crate::tags::import::{ImportError, TagAssignment, normalize_assignments, parse_import};
use crate::tags::suggest::{SuggestionStatus, TagSuggestionHandle};
use crate::yf::YFinance;

static YF: LazyLock<YFinance> = LazyLock::new(YFinance::new);

#[derive(Clone)]
struct TagState {
    store: Arc<Store>,
    tag_suggestions: Option<TagSuggestionHandle>,
    suggestion_action_lock: Arc<Mutex<()>>,
}

#[derive(Template)]
#[template(path = "tags_mgmt.html")]
struct TagMgmtTemplate {
    tags_json: String,
    categories_json: String,
    stocks_json: String,
    untagged_json: String,
    tag_suggestion_enabled: bool,
}

#[derive(Debug, Deserialize)]
struct TagNameRequest {
    name: String,
    category_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct CategoryNameRequest {
    name: String,
}

#[derive(Debug, Deserialize)]
struct SetStockTagsRequest {
    ticker: String,
    tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct SuggestTagsRequest {
    tickers: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct SuggestTagsStatusRequest {
    tickers: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ApplySuggestionsRequest {
    tickers: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct IgnoreSuggestionsRequest {
    tickers: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ImportRequest {
    content: String,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

#[derive(Debug, Serialize)]
struct TagStockView {
    ticker: String,
    tags: Vec<Tag>,
}

#[derive(Debug, Serialize)]
struct SetStockTagsResponse {
    ticker: String,
    set_tags: Vec<String>,
    removed_tags: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ImportPreviewRow {
    ticker: String,
    tags: Vec<String>,
    unknown_ticker: bool,
    new_tags: Vec<String>,
    mappings_to_set: usize,
    mappings_to_remove: usize,
}

#[derive(Debug, Serialize)]
struct ImportPreviewResponse {
    rows_parsed: usize,
    new_tags: Vec<String>,
    mappings_to_set: usize,
    mappings_to_remove: usize,
    unknown_tickers: Vec<String>,
    errors: Vec<ImportError>,
    rows: Vec<ImportPreviewRow>,
}

#[derive(Debug, Serialize)]
struct ImportApplyResponse {
    rows_parsed: usize,
    mappings_set: usize,
    mappings_removed: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum BatchSuggestionStatus {
    NotRequested,
    Pending,
    Ready,
    Failed,
    Ignored,
}

#[derive(Debug, Serialize)]
struct BatchSuggestionItem {
    ticker: String,
    status: BatchSuggestionStatus,
    suggested_tags: Vec<String>,
    error: Option<String>,
    generated_at: Option<chrono::DateTime<Local>>,
    requested_at: Option<chrono::DateTime<Local>>,
    provider: Option<String>,
    model: Option<String>,
}

#[derive(Debug, Serialize)]
struct BatchSuggestionsResponse {
    items: Vec<BatchSuggestionItem>,
}

#[derive(Debug, Serialize)]
struct ApplySuggestionItem {
    ticker: String,
    applied: bool,
    set_tags: Vec<String>,
    removed_tags: Vec<String>,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct ApplySuggestionsResponse {
    items: Vec<ApplySuggestionItem>,
}

struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: message.into(),
        }
    }
}

impl<E> From<E> for ApiError
where
    E: Into<anyhow::Error>,
{
    fn from(error: E) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: error.into().to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorResponse {
                error: self.message,
            }),
        )
            .into_response()
    }
}

pub fn router(store: Arc<Store>) -> Router {
    let tag_suggestions = APP_CONFIG.tag_suggestion.clone().and_then(|config| {
        match TagSuggestionHandle::new(config, store.clone()) {
            Ok(handle) => {
                info!(
                    "Tag suggestions enabled with provider={} model={}",
                    handle.provider_name(),
                    handle.model().unwrap_or_else(|_| "unknown".to_string())
                );
                Some(handle)
            }
            Err(err) => {
                tracing::error!("Tag suggestion config is invalid: {err}");
                None
            }
        }
    });
    let state = TagState {
        store,
        tag_suggestions,
        suggestion_action_lock: Arc::new(Mutex::new(())),
    };
    Router::new()
        .route("/tags_mgmt.html", routing::get(tag_mgmt_home))
        .route("/api/tags", routing::get(list_tags).post(create_tag))
        .route(
            "/api/tag-categories",
            routing::get(list_tag_categories).post(create_tag_category),
        )
        .route(
            "/api/tags/{id}",
            routing::put(rename_tag).delete(delete_tag),
        )
        .route("/api/stock-tags", routing::get(list_stock_tags))
        .route("/api/stock-tags/tags", routing::put(set_stock_tags))
        .route(
            "/api/stock-tags/suggest",
            routing::post(queue_tag_suggestion),
        )
        .route(
            "/api/stock-tags/suggest/status",
            routing::post(get_tag_suggestion_statuses),
        )
        .route(
            "/api/stock-tags/suggest/apply",
            routing::post(apply_tag_suggestions),
        )
        .route(
            "/api/stock-tags/suggest/ignore",
            routing::post(ignore_tag_suggestions),
        )
        .route(
            "/api/stock-tags/suggest/{ticker}",
            routing::delete(delete_tag_suggestion),
        )
        .route(
            "/api/stock-tags/untagged",
            routing::get(list_untagged_stocks),
        )
        .route(
            "/api/company-profiles/{ticker}",
            routing::get(get_company_profile).post(refresh_company_profile),
        )
        .route("/api/tag-import/preview", routing::post(preview_import))
        .route("/api/tag-import", routing::post(apply_import))
        .with_state(state)
}

async fn tag_mgmt_home(State(state): State<TagState>) -> Result<Html<String>, HtmlError> {
    let tags = state.store.list_tags().await?;
    let categories = state.store.list_tag_categories().await?;
    let stocks = stock_views(&state.store).await?;
    let untagged = state.store.list_untagged_stocks().await?;

    let html = TagMgmtTemplate {
        tags_json: serde_json::to_string(&tags)?,
        categories_json: serde_json::to_string(&categories)?,
        stocks_json: serde_json::to_string(&stocks)?,
        untagged_json: serde_json::to_string(&untagged)?,
        tag_suggestion_enabled: state.tag_suggestions.is_some(),
    }
    .render()?;

    Ok(Html(html))
}

async fn list_tags(State(state): State<TagState>) -> Result<impl IntoResponse, ApiError> {
    Ok(Json(state.store.list_tags().await?))
}

async fn list_tag_categories(State(state): State<TagState>) -> Result<impl IntoResponse, ApiError> {
    Ok(Json(state.store.list_tag_categories().await?))
}

async fn create_tag_category(
    State(state): State<TagState>,
    Json(req): Json<CategoryNameRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if req.name.trim().is_empty() {
        return Err(ApiError::bad_request("Category name is required"));
    }

    Ok(Json(state.store.create_tag_category(&req.name).await?))
}

async fn create_tag(
    State(state): State<TagState>,
    Json(req): Json<TagNameRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if req.name.trim().is_empty() {
        return Err(ApiError::bad_request("Tag name is required"));
    }
    let Some(category_id) = req.category_id else {
        return Err(ApiError::bad_request("Category is required"));
    };
    if !state.store.category_exists(category_id).await? {
        return Err(ApiError::bad_request("Category does not exist"));
    }

    Ok(Json(
        state
            .store
            .create_tag_in_category(&req.name, Some(category_id))
            .await?,
    ))
}

async fn rename_tag(
    State(state): State<TagState>,
    Path(id): Path<i64>,
    Json(req): Json<TagNameRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let name = req.name.trim();
    if let Some(category_id) = req.category_id {
        if !state.store.category_exists(category_id).await? {
            return Err(ApiError::bad_request("Category does not exist"));
        }
        if !name.is_empty() {
            state.store.rename_tag(id, name).await?;
        }
        return Ok(Json(
            state.store.move_tag_to_category(id, category_id).await?,
        ));
    };

    if name.is_empty() {
        return Err(ApiError::bad_request("Tag name is required"));
    }

    Ok(Json(state.store.rename_tag(id, name).await?))
}

async fn delete_tag(
    State(state): State<TagState>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, ApiError> {
    match state.store.delete_tag(id).await? {
        DeleteTagResult::Deleted => Ok(StatusCode::NO_CONTENT.into_response()),
        DeleteTagResult::InUse(count) => {
            Err(ApiError::conflict(format!("Tag is used by {count} stocks")))
        }
    }
}

async fn list_stock_tags(State(state): State<TagState>) -> Result<impl IntoResponse, ApiError> {
    Ok(Json(stock_views(&state.store).await?))
}

async fn list_untagged_stocks(
    State(state): State<TagState>,
) -> Result<impl IntoResponse, ApiError> {
    Ok(Json(state.store.list_untagged_stocks().await?))
}

async fn set_stock_tags(
    State(state): State<TagState>,
    Json(req): Json<SetStockTagsRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if req.ticker.trim().is_empty() {
        return Err(ApiError::bad_request("Ticker is required"));
    }

    let allowed_tags = state
        .store
        .list_tags()
        .await?
        .into_iter()
        .map(|tag| tag.name.to_lowercase())
        .collect::<HashSet<_>>();
    let unknown_tags = req
        .tags
        .iter()
        .map(|tag| tag.trim())
        .filter(|tag| !tag.is_empty())
        .filter(|tag| !allowed_tags.contains(&tag.to_lowercase()))
        .map(str::to_string)
        .collect::<Vec<_>>();
    if !unknown_tags.is_empty() {
        return Err(ApiError::bad_request(format!(
            "Unknown tags: {}",
            unknown_tags.join(", ")
        )));
    }

    let result = state
        .store
        .set_tags_for_stock(&req.ticker, &req.tags)
        .await?;
    Ok(Json(SetStockTagsResponse {
        ticker: req.ticker.trim().to_uppercase(),
        set_tags: result.set_tags,
        removed_tags: result.removed_tags,
    }))
}

async fn queue_tag_suggestion(
    State(state): State<TagState>,
    Json(req): Json<SuggestTagsRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let Some(handle) = state.tag_suggestions.as_ref() else {
        return Err(ApiError::bad_request("Tag suggestions are not configured"));
    };
    let tickers = normalize_tickers(&req.tickers)?;
    let mut items = Vec::with_capacity(tickers.len());

    for ticker in tickers {
        match queue_one_tag_suggestion(&state, handle, &ticker).await {
            Ok(item) => items.push(item),
            Err(err) => items.push(BatchSuggestionItem::request_error(ticker, err.message)),
        }
    }

    Ok(Json(BatchSuggestionsResponse { items }))
}

async fn queue_one_tag_suggestion(
    state: &TagState,
    handle: &TagSuggestionHandle,
    ticker: &str,
) -> Result<BatchSuggestionItem, ApiError> {
    let enqueued = handle.enqueue(ticker.to_string()).await?;
    if enqueued {
        info!("Queued tag suggestion for {ticker}");
    } else {
        info!("Tag suggestion for {ticker} is already queued");
    }

    let suggestion = state
        .store
        .get_tag_suggestion(&ticker)
        .await?
        .ok_or_else(|| ApiError::bad_request("Failed to queue tag suggestion"))?;
    Ok(BatchSuggestionItem::from(suggestion))
}

async fn get_tag_suggestion_statuses(
    State(state): State<TagState>,
    Json(req): Json<SuggestTagsStatusRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let tickers = normalize_tickers(&req.tickers)?;
    let mut items = Vec::with_capacity(tickers.len());
    for ticker in tickers {
        items.push(suggestion_status_item(&state.store, &ticker).await?);
    }
    Ok(Json(BatchSuggestionsResponse { items }))
}

async fn apply_tag_suggestions(
    State(state): State<TagState>,
    Json(req): Json<ApplySuggestionsRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let _action_guard = state.suggestion_action_lock.lock().await;
    let tickers = normalize_tickers(&req.tickers)?;
    let mut items = Vec::with_capacity(tickers.len());

    for ticker in tickers {
        let Some(suggestion) = state.store.get_tag_suggestion(&ticker).await? else {
            items.push(ApplySuggestionItem {
                ticker,
                applied: false,
                set_tags: Vec::new(),
                removed_tags: Vec::new(),
                error: Some("No suggestion requested".to_string()),
            });
            continue;
        };
        if suggestion.status != SuggestionStatus::Ready {
            items.push(ApplySuggestionItem {
                ticker,
                applied: false,
                set_tags: Vec::new(),
                removed_tags: Vec::new(),
                error: Some("Suggestion is not ready".to_string()),
            });
            continue;
        }
        if suggestion.suggested_tags.is_empty() {
            items.push(ApplySuggestionItem {
                ticker,
                applied: false,
                set_tags: Vec::new(),
                removed_tags: Vec::new(),
                error: Some("Suggestion has no tags".to_string()),
            });
            continue;
        }
        let result = match state
            .store
            .set_tags_for_stock(&ticker, &suggestion.suggested_tags)
            .await
        {
            Ok(result) => result,
            Err(err) => {
                items.push(ApplySuggestionItem {
                    ticker,
                    applied: false,
                    set_tags: Vec::new(),
                    removed_tags: Vec::new(),
                    error: Some(err.to_string()),
                });
                continue;
            }
        };
        items.push(ApplySuggestionItem {
            ticker,
            applied: true,
            set_tags: result.set_tags,
            removed_tags: result.removed_tags,
            error: None,
        });
    }

    Ok(Json(ApplySuggestionsResponse { items }))
}

async fn ignore_tag_suggestions(
    State(state): State<TagState>,
    Json(req): Json<IgnoreSuggestionsRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let _action_guard = state.suggestion_action_lock.lock().await;
    let tickers = normalize_tickers(&req.tickers)?;
    let mut items = Vec::with_capacity(tickers.len());

    for ticker in tickers {
        if !state.store.ignore_tag_suggestion(&ticker).await? {
            return Err(ApiError::bad_request(format!(
                "Only unapplied ready suggestions can be ignored: {ticker}"
            )));
        }
        items.push(suggestion_status_item(&state.store, &ticker).await?);
    }

    Ok(Json(BatchSuggestionsResponse { items }))
}

async fn delete_tag_suggestion(
    State(state): State<TagState>,
    Path(ticker): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let ticker = normalize_ticker(&ticker)?;
    state.store.delete_tag_suggestion(&ticker).await?;
    info!("Deleted cached tag suggestion for {ticker}");
    Ok(StatusCode::NO_CONTENT)
}

async fn get_company_profile(
    State(state): State<TagState>,
    Path(ticker): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let ticker = normalize_ticker(&ticker)?;
    let profile = match state.store.get_company_profile(&ticker).await? {
        Some(profile) => profile,
        None => fetch_and_cache_company_profile(&state.store, &ticker).await?,
    };
    Ok(Json(profile))
}

async fn refresh_company_profile(
    State(state): State<TagState>,
    Path(ticker): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let ticker = normalize_ticker(&ticker)?;
    Ok(Json(
        fetch_and_cache_company_profile(&state.store, &ticker).await?,
    ))
}

async fn fetch_and_cache_company_profile(
    store: &Store,
    ticker: &str,
) -> Result<CompanyProfile, ApiError> {
    let yf_profile = YF.fetch_company_profile(ticker).await?;
    let profile = CompanyProfile {
        ticker: yf_profile.symbol.trim().to_uppercase(),
        summary: yf_profile
            .summary
            .map(|summary| summary.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|summary| !summary.is_empty()),
        sector: yf_profile.sector,
        industry: yf_profile.industry,
        source: "Yahoo Finance".to_string(),
        fetched_at: Local::now(),
    };
    store.save_company_profile(&profile).await?;
    Ok(profile)
}

fn normalize_ticker(ticker: &str) -> Result<String, ApiError> {
    let ticker = ticker.trim().to_uppercase();
    if ticker.is_empty() {
        return Err(ApiError::bad_request("Ticker is required"));
    }
    Ok(ticker)
}

fn normalize_tickers(tickers: &[String]) -> Result<Vec<String>, ApiError> {
    let mut normalized = Vec::new();
    let mut seen = HashSet::new();
    for ticker in tickers {
        let ticker = normalize_ticker(ticker)?;
        if seen.insert(ticker.clone()) {
            normalized.push(ticker);
        }
    }
    if normalized.is_empty() {
        return Err(ApiError::bad_request("At least one ticker is required"));
    }
    Ok(normalized)
}

async fn suggestion_status_item(
    store: &Store,
    ticker: &str,
) -> Result<BatchSuggestionItem, ApiError> {
    match store.get_tag_suggestion(ticker).await? {
        Some(suggestion) => Ok(BatchSuggestionItem::from(suggestion)),
        None => Ok(BatchSuggestionItem {
            ticker: ticker.to_string(),
            status: BatchSuggestionStatus::NotRequested,
            suggested_tags: Vec::new(),
            error: None,
            generated_at: None,
            requested_at: None,
            provider: None,
            model: None,
        }),
    }
}

impl From<crate::tags::suggest::CachedTagSuggestion> for BatchSuggestionItem {
    fn from(suggestion: crate::tags::suggest::CachedTagSuggestion) -> Self {
        let status = match suggestion.status {
            SuggestionStatus::Pending => BatchSuggestionStatus::Pending,
            SuggestionStatus::Ready => BatchSuggestionStatus::Ready,
            SuggestionStatus::Failed => BatchSuggestionStatus::Failed,
            SuggestionStatus::Ignored => BatchSuggestionStatus::Ignored,
        };
        Self {
            ticker: suggestion.ticker,
            status,
            suggested_tags: suggestion.suggested_tags,
            error: suggestion.error,
            generated_at: suggestion.generated_at,
            requested_at: Some(suggestion.requested_at),
            provider: Some(suggestion.provider),
            model: Some(suggestion.model),
        }
    }
}

impl BatchSuggestionItem {
    fn request_error(ticker: String, error: String) -> Self {
        Self {
            ticker,
            status: BatchSuggestionStatus::Failed,
            suggested_tags: Vec::new(),
            error: Some(error.clone()),
            generated_at: None,
            requested_at: None,
            provider: None,
            model: None,
        }
    }
}

async fn preview_import(
    State(state): State<TagState>,
    Json(req): Json<ImportRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let assignments = parse_request_import(&req)?;
    let tags = state.store.list_tags().await?;
    let errors = validate_import_assignments(&assignments, &tags);
    let preview = build_preview(&state.store, assignments, errors).await?;
    Ok(Json(preview))
}

async fn apply_import(
    State(state): State<TagState>,
    Json(req): Json<ImportRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let assignments = parse_request_import(&req)?;
    let tags = state.store.list_tags().await?;
    let errors = validate_import_assignments(&assignments, &tags);
    if !errors.is_empty() {
        return Err(ApiError::bad_request("Import has validation errors"));
    }

    let import_rows = assignments
        .into_iter()
        .map(|assignment| (assignment.ticker, assignment.tags))
        .collect::<Vec<_>>();
    let result = state.store.replace_tags_for_stocks(&import_rows).await?;

    Ok(Json(ImportApplyResponse {
        rows_parsed: import_rows.len(),
        mappings_set: result.mappings_set,
        mappings_removed: result.mappings_removed,
    }))
}

fn parse_request_import(req: &ImportRequest) -> Result<Vec<TagAssignment>, ApiError> {
    if req.content.trim().is_empty() {
        return Err(ApiError::bad_request("Import content is required"));
    }

    parse_import(&req.content)
        .map(normalize_assignments)
        .map_err(|err| ApiError::bad_request(err.to_string()))
}

fn validate_import_assignments(assignments: &[TagAssignment], tags: &[Tag]) -> Vec<ImportError> {
    let allowed_tags = tags
        .iter()
        .map(|tag| tag.name.to_lowercase())
        .collect::<HashSet<_>>();
    let mut errors = crate::tags::import::validate(assignments);

    for (idx, assignment) in assignments.iter().enumerate() {
        let unknown_tags = assignment
            .tags
            .iter()
            .filter(|tag| !allowed_tags.contains(&tag.to_lowercase()))
            .cloned()
            .collect::<Vec<_>>();
        if !unknown_tags.is_empty() {
            errors.push(ImportError {
                row: Some(idx + 1),
                message: format!("Unknown tags: {}", unknown_tags.join(", ")),
            });
        }
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(name: &str) -> Tag {
        Tag {
            id: 1,
            name: name.to_string(),
            category_id: 1,
            stock_count: 0,
            assigned_at: None,
        }
    }

    #[test]
    fn import_validation_rejects_unknown_tags() {
        let assignments = vec![TagAssignment {
            ticker: "NVDA".to_string(),
            tags: vec![
                "AI Infrastructure".to_string(),
                "AI Infrastucture".to_string(),
            ],
        }];
        let tags = vec![tag("AI Infrastructure")];

        let errors = validate_import_assignments(&assignments, &tags);

        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].row, Some(1));
        assert_eq!(errors[0].message, "Unknown tags: AI Infrastucture");
    }

    #[test]
    fn import_validation_allows_existing_tags_case_insensitively() {
        let assignments = vec![TagAssignment {
            ticker: "NVDA".to_string(),
            tags: vec!["ai infrastructure".to_string()],
        }];
        let tags = vec![tag("AI Infrastructure")];

        let errors = validate_import_assignments(&assignments, &tags);

        assert!(errors.is_empty());
    }
}

async fn stock_views(store: &Store) -> sqlx::Result<Vec<TagStockView>> {
    Ok(store
        .list_stock_tags()
        .await?
        .into_iter()
        .map(|stock| TagStockView {
            ticker: stock.ticker,
            tags: stock.tags,
        })
        .collect())
}

async fn build_preview(
    store: &Store,
    assignments: Vec<TagAssignment>,
    errors: Vec<ImportError>,
) -> sqlx::Result<ImportPreviewResponse> {
    let tags = store.list_tags().await?;
    let existing_tags = tags
        .iter()
        .map(|tag| tag.name.to_lowercase())
        .collect::<HashSet<_>>();
    let stocks = store.list_stock_tags().await?;
    let untagged = store.list_untagged_stocks().await?;
    let known_tickers = stocks
        .iter()
        .map(|stock| stock.ticker.clone())
        .chain(untagged)
        .collect::<HashSet<_>>();
    let existing_mappings = stocks
        .into_iter()
        .map(|stock| {
            (
                stock.ticker,
                stock
                    .tags
                    .into_iter()
                    .map(|tag| tag.name.to_lowercase())
                    .collect::<HashSet<_>>(),
            )
        })
        .collect::<HashMap<_, _>>();

    let mut seen_new_tags = HashSet::<String>::new();
    let mut new_tags = Vec::new();
    let mut unknown_tickers = Vec::new();
    let mut mappings_to_set = 0;
    let mut mappings_to_remove = 0;
    let mut rows = Vec::new();

    for assignment in assignments {
        let mut row_new_tags = Vec::new();
        let mut row_mappings_to_set = 0;
        let mut row_mappings_to_remove = 0;
        let existing_for_ticker = existing_mappings.get(&assignment.ticker);

        for tag in &assignment.tags {
            let tag_key = tag.to_lowercase();
            if !existing_tags.contains(&tag_key) && seen_new_tags.insert(tag_key.clone()) {
                new_tags.push(tag.clone());
                row_new_tags.push(tag.clone());
            } else if !existing_tags.contains(&tag_key) {
                row_new_tags.push(tag.clone());
            }

            row_mappings_to_set += 1;
        }

        let unknown_ticker = !known_tickers.contains(&assignment.ticker);
        if let Some(existing_tags) = existing_for_ticker {
            let desired = assignment
                .tags
                .iter()
                .map(|tag| tag.to_lowercase())
                .collect::<HashSet<_>>();
            row_mappings_to_remove = existing_tags
                .iter()
                .filter(|tag| !desired.contains(*tag))
                .count();
        }
        if unknown_ticker {
            unknown_tickers.push(assignment.ticker.clone());
        }
        mappings_to_set += row_mappings_to_set;
        mappings_to_remove += row_mappings_to_remove;
        rows.push(ImportPreviewRow {
            ticker: assignment.ticker,
            tags: assignment.tags,
            unknown_ticker,
            new_tags: row_new_tags,
            mappings_to_set: row_mappings_to_set,
            mappings_to_remove: row_mappings_to_remove,
        });
    }

    new_tags.sort_by_key(|tag| tag.to_lowercase());
    unknown_tickers.sort();
    Ok(ImportPreviewResponse {
        rows_parsed: rows.len(),
        new_tags,
        mappings_to_set,
        mappings_to_remove,
        unknown_tickers,
        errors,
        rows,
    })
}
