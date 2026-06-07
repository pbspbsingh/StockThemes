mod handle;
mod parse;
mod prompt;
mod providers;
mod store;

pub use handle::TagSuggestionHandle;
pub use prompt::suggestion_input;

use chrono::{DateTime, Local};
use serde::Serialize;

use crate::store::CompanyProfile;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionStatus {
    Pending,
    Ready,
    Failed,
    Ignored,
}

#[derive(Debug, Clone, Serialize)]
pub struct CachedTagSuggestion {
    pub ticker: String,
    pub status: SuggestionStatus,
    pub suggested_tags: Vec<String>,
    pub error: Option<String>,
    pub generated_at: Option<DateTime<Local>>,
    pub requested_at: DateTime<Local>,
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Clone)]
pub struct SuggestionInput {
    pub ticker: String,
    pub profile: CompanyProfile,
    pub allowed_tags: Vec<String>,
}
