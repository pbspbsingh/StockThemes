use crate::util::parse_percentage;
use crate::{Performance, TickerType};
use anyhow::Context;
use chromiumoxide::{Element, Page};
use std::collections::HashMap;

pub async fn parse_performances(
    page: &Page,
    ticker_type: TickerType,
) -> anyhow::Result<Vec<Performance>> {
    let indices = find_perf_cols(&page).await?;
    if indices.is_empty() {
        anyhow::bail!("Performance information didn't load in time for {ticker_type:?}");
    }

    let mut result = Vec::new();
    for row in page
        .find_elements(r#"table tbody[data-testid="selectable-rows-table-body"] tr"#)
        .await?
    {
        result.push(parse_perf_info(&indices, &row, ticker_type).await?);
    }
    Ok(result)
}

async fn parse_perf_info(
    indices: &HashMap<String, usize>,
    row: &Element,
    ticker_type: TickerType,
) -> anyhow::Result<Performance> {
    let cells = row.find_elements("td").await?;
    let mut name = cells
        .get(0)
        .context("No cells returned")?
        .inner_text()
        .await?
        .context("No sector name")?;
    if let Some((ticker, _detail)) = name.split_once('\n') {
        name = ticker.trim().to_owned();
    }

    let mut perf_map = HashMap::new();
    for (perf_name, perf_idx) in indices {
        let perf = cells
            .get(*perf_idx)
            .with_context(|| format!("No cell for {perf_name} at {perf_idx}"))?
            .inner_text()
            .await?
            .with_context(|| format!("No inner text for {perf_name} at {perf_idx}"))?;
        perf_map.insert(perf_name.clone(), parse_percentage(perf)?);
    }

    Ok(Performance::new(name, ticker_type, perf_map))
}

async fn find_perf_cols(page: &Page) -> anyhow::Result<HashMap<String, usize>> {
    const PERF_COL_NAME: [&str; 4] = ["1M", "3M", "6M", "1Y"];

    let mut indices = HashMap::with_capacity(PERF_COL_NAME.len());
    for (idx, element) in page
        .find_elements("table thead tr th")
        .await
        .context("Couldn't table headers")?
        .iter()
        .enumerate()
    {
        for col_name in PERF_COL_NAME {
            if let Some(data_field) = element.attribute("data-field").await?
                && data_field == format!("Performance|Interval{col_name}")
                && !indices.contains_key(col_name)
            {
                indices.insert(col_name.to_owned(), idx);
            }
        }
        if indices.len() == PERF_COL_NAME.len() {
            break;
        }
    }
    Ok(indices)
}
