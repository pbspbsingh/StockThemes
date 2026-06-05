use crate::store::CompanyProfile;

use super::SuggestionInput;

pub fn prompt_hash(input: &SuggestionInput, provider: &str, model: &str) -> String {
    let payload = serde_json::json!({
        "ticker": input.ticker,
        "summary": input.profile.summary,
        "sector": input.profile.sector,
        "industry": input.profile.industry,
        "profile_fetched_at": input.profile.fetched_at.timestamp_millis(),
        "allowed_tags": input.allowed_tags,
        "provider": provider,
        "model": model,
    });
    stable_hash(payload.to_string().as_bytes())
}

pub fn input_for_hash(
    ticker: String,
    profile: CompanyProfile,
    allowed_tags: Vec<String>,
) -> SuggestionInput {
    SuggestionInput {
        ticker,
        profile,
        allowed_tags,
        prompt_hash: String::new(),
    }
}

pub(super) fn build_prompt(input: &SuggestionInput) -> String {
    let profile = &input.profile;
    let allowed_tags = serde_json::to_string_pretty(&input.allowed_tags).unwrap_or_default();
    format!(
        r#"You are assigning stock theme tags.

Return JSON only in this exact shape:
{{"tags":["Tag Name"]}}

Rules:
- Use only tags from the allowed tags JSON.
- Do not create new tags, variants, synonyms, or near-duplicates.
- Assign all tags that are central to the company's business model.
- Avoid peripheral or minor activities.
- If no allowed tag fits, return {{"tags":[]}}.

Ticker: {ticker}
Sector: {sector}
Industry: {industry}
Company profile:
{summary}

Allowed tags JSON:
{allowed_tags}
"#,
        ticker = input.ticker,
        sector = profile.sector.as_deref().unwrap_or("Unknown"),
        industry = profile.industry.as_deref().unwrap_or("Unknown"),
        summary = profile.summary.as_deref().unwrap_or("No summary available"),
        allowed_tags = allowed_tags,
    )
}

fn stable_hash(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}
