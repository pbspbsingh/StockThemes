use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagAssignment {
    pub ticker: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportError {
    pub row: Option<usize>,
    pub message: String,
}

pub fn parse_import(content: &str) -> anyhow::Result<Vec<TagAssignment>> {
    parse_json(content)
}

pub fn validate(assignments: &[TagAssignment]) -> Vec<ImportError> {
    assignments
        .iter()
        .enumerate()
        .filter_map(|(idx, assignment)| {
            if assignment.ticker.trim().is_empty() {
                Some(ImportError {
                    row: Some(idx + 1),
                    message: "Missing ticker".to_string(),
                })
            } else {
                None
            }
        })
        .collect()
}

pub fn normalize_assignments(assignments: Vec<TagAssignment>) -> Vec<TagAssignment> {
    let mut merged = HashMap::<String, Vec<String>>::new();
    for assignment in assignments {
        let ticker = assignment.ticker.trim().to_uppercase();
        if ticker.is_empty() {
            continue;
        }
        let entry = merged.entry(ticker).or_default();
        for tag in assignment.tags {
            let tag = normalize_tag_name(&tag);
            if tag.is_empty() {
                continue;
            }
            if !entry
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(&tag))
            {
                entry.push(tag);
            }
        }
    }

    let mut assignments = merged
        .into_iter()
        .map(|(ticker, tags)| TagAssignment { ticker, tags })
        .collect::<Vec<_>>();
    assignments.sort_by(|a, b| a.ticker.cmp(&b.ticker));
    assignments
}

fn parse_json(content: &str) -> anyhow::Result<Vec<TagAssignment>> {
    let parsed: HashMap<String, Vec<String>> =
        serde_json::from_str(content).context("Failed to parse JSON")?;
    let assignments = parsed
        .into_iter()
        .map(|(ticker, tags)| TagAssignment { ticker, tags })
        .collect();
    Ok(normalize_assignments(assignments))
}

fn normalize_tag_name(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_json_map() {
        let rows = parse_import(r#"{"nvda":["AI Infrastructure","Semiconductors"]}"#).unwrap();

        assert_eq!(rows[0].ticker, "NVDA");
        assert_eq!(rows[0].tags.len(), 2);
    }

    #[test]
    fn allows_empty_tag_arrays() {
        let rows = parse_import(r#"{"nvda":[]}"#).unwrap();

        assert_eq!(rows[0].ticker, "NVDA");
        assert!(rows[0].tags.is_empty());
        assert!(validate(&rows).is_empty());
    }
}
