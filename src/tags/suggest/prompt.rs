use crate::store::CompanyProfile;

use super::SuggestionInput;

pub fn suggestion_input(
    ticker: String,
    profile: CompanyProfile,
    allowed_tags: Vec<String>,
) -> SuggestionInput {
    SuggestionInput {
        ticker,
        profile,
        allowed_tags,
    }
}

pub(super) fn build_prompt(input: &SuggestionInput) -> String {
    let profile = &input.profile;
    let allowed_tags = serde_json::to_string_pretty(&input.allowed_tags).unwrap_or_default();
    format!(
        r#"You need to assign thematic tags to stocks. These tags must reflect the core business of the company.

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
