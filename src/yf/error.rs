use std::fmt;

/// Errors that callers may want to handle explicitly.
#[derive(Debug)]
pub enum YfError {
    /// Yahoo Finance returned HTTP 404 — retrying will not help.
    NotFound { url: String },
    /// Yahoo Finance returned HTTP 429 — caller should back off and retry.
    RateLimited,
}

impl fmt::Display for YfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            YfError::NotFound { url } => {
                write!(f, "Yahoo Finance resource not found (HTTP 404): {url}")
            }
            YfError::RateLimited => write!(f, "Yahoo Finance rate limit exceeded (HTTP 429)"),
        }
    }
}

impl std::error::Error for YfError {}
