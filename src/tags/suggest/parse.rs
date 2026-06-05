use std::collections::HashMap;

use anyhow::{Context, anyhow};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct SuggestedTagsPayload {
    tags: Vec<String>,
}

pub(super) fn parse_suggested_tags(
    content: &str,
    allowed_tags: &[String],
) -> anyhow::Result<Vec<String>> {
    let json = extract_json_object(content).ok_or_else(|| anyhow!("Model did not return JSON"))?;
    let payload = serde_json::from_str::<SuggestedTagsPayload>(json)
        .with_context(|| format!("Model returned invalid JSON: {json}"))?;
    let allowed_by_lower = allowed_tags
        .iter()
        .map(|tag| (tag.to_lowercase(), tag.clone()))
        .collect::<HashMap<_, _>>();
    let mut result = Vec::new();
    for tag in payload.tags {
        let key = tag.trim().to_lowercase();
        let Some(canonical) = allowed_by_lower.get(&key) else {
            continue;
        };
        if !result
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(canonical))
        {
            result.push(canonical.clone());
        }
    }
    Ok(result)
}

fn extract_json_object(content: &str) -> Option<&str> {
    let start = content.find('{')?;
    let end = content.rfind('}')?;
    (start <= end).then_some(&content[start..=end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_json_tag_response_with_canonical_names() {
        let tags = parse_suggested_tags(
            r#"{"tags":["ai infrastructure","Unknown","Semiconductors"]}"#,
            &[
                "AI Infrastructure".to_string(),
                "Semiconductors".to_string(),
            ],
        )
        .unwrap();
        assert_eq!(tags, vec!["AI Infrastructure", "Semiconductors"]);
    }

    #[test]
    fn extracts_json_from_wrapped_content() {
        let tags = parse_suggested_tags(
            "Here:\n```json\n{\"tags\":[\"AI\"]}\n```",
            &["AI".to_string()],
        )
        .unwrap();
        assert_eq!(tags, vec!["AI"]);
    }
}
