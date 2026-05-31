use askama::Template;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::html_error::HtmlError;
use crate::store::{DeleteTagResult, Store, Tag};
use crate::tags::import::{ImportError, TagAssignment, normalize_assignments, parse_import};

#[derive(Clone)]
struct TagState {
    store: Arc<Store>,
}

#[derive(Template)]
#[template(path = "tag_mgmt.html")]
struct TagMgmtTemplate {
    tags_json: String,
    stocks_json: String,
    untagged_json: String,
}

#[derive(Debug, Deserialize)]
struct TagNameRequest {
    name: String,
}

#[derive(Debug, Deserialize)]
struct AddStockTagsRequest {
    ticker: String,
    tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RemoveStockTagRequest {
    ticker: String,
    tag_id: i64,
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
struct AddStockTagsResponse {
    ticker: String,
    created_tags: Vec<String>,
    added_tags: Vec<String>,
    duplicates_skipped: Vec<String>,
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
    created_tags: Vec<String>,
    mappings_set: usize,
    mappings_removed: usize,
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
    let state = TagState { store };
    Router::new()
        .route("/tag_mgmt.html", routing::get(tag_mgmt_home))
        .route("/api/tags", routing::get(list_tags).post(create_tag))
        .route(
            "/api/tags/{id}",
            routing::put(rename_tag).delete(delete_tag),
        )
        .route("/api/stock-tags", routing::get(list_stock_tags))
        .route(
            "/api/stock-tags/tags",
            routing::post(add_stock_tags).delete(remove_stock_tag),
        )
        .route(
            "/api/stock-tags/untagged",
            routing::get(list_untagged_stocks),
        )
        .route("/api/tag-import/preview", routing::post(preview_import))
        .route("/api/tag-import", routing::post(apply_import))
        .with_state(state)
}

async fn tag_mgmt_home(State(state): State<TagState>) -> Result<Html<String>, HtmlError> {
    let tags = state.store.list_tags().await?;
    let stocks = stock_views(&state.store).await?;
    let untagged = state.store.list_untagged_stocks().await?;

    let html = TagMgmtTemplate {
        tags_json: serde_json::to_string(&tags)?,
        stocks_json: serde_json::to_string(&stocks)?,
        untagged_json: serde_json::to_string(&untagged)?,
    }
    .render()?;

    Ok(Html(html))
}

async fn list_tags(State(state): State<TagState>) -> Result<impl IntoResponse, ApiError> {
    Ok(Json(state.store.list_tags().await?))
}

async fn create_tag(
    State(state): State<TagState>,
    Json(req): Json<TagNameRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if req.name.trim().is_empty() {
        return Err(ApiError::bad_request("Tag name is required"));
    }

    Ok(Json(state.store.create_tag(&req.name).await?))
}

async fn rename_tag(
    State(state): State<TagState>,
    Path(id): Path<i64>,
    Json(req): Json<TagNameRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if req.name.trim().is_empty() {
        return Err(ApiError::bad_request("Tag name is required"));
    }

    Ok(Json(state.store.rename_tag(id, &req.name).await?))
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

async fn add_stock_tags(
    State(state): State<TagState>,
    Json(req): Json<AddStockTagsRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if req.ticker.trim().is_empty() {
        return Err(ApiError::bad_request("Ticker is required"));
    }
    if req.tags.iter().all(|tag| tag.trim().is_empty()) {
        return Err(ApiError::bad_request("At least one tag is required"));
    }

    let result = state
        .store
        .add_tags_to_stock(&req.ticker, &req.tags)
        .await?;
    Ok(Json(AddStockTagsResponse {
        ticker: req.ticker.trim().to_uppercase(),
        created_tags: result.created_tags,
        added_tags: result.added_tags,
        duplicates_skipped: result.duplicates_skipped,
    }))
}

async fn remove_stock_tag(
    State(state): State<TagState>,
    Json(req): Json<RemoveStockTagRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if req.ticker.trim().is_empty() {
        return Err(ApiError::bad_request("Ticker is required"));
    }

    state
        .store
        .remove_tag_from_stock(&req.ticker, req.tag_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
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
        created_tags: result.created_tags,
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
            stock_count: 0,
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
