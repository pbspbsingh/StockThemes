use crate::Summary;

static HTML: &str = include_str!("./template.html");
const TEMPLATE_TEXT: &str = r#""__REPLACE_ME__""#;

pub fn create_html(summary: &Summary) -> anyhow::Result<String> {
    let data = serde_json::to_string(&summary)?;
    Ok(HTML.replace(TEMPLATE_TEXT, &data))
}
