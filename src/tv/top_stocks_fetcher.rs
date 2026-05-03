use crate::tv::TV_HOME;
use crate::tv::perf_util::parse_performances;
use crate::util::normalize;
use crate::{Group, Performance, Stock, TickerType};
use anyhow::Context;
use chrome_driver::{Element, Page, PageFeatures};
use chrono::Local;
use tracing::{debug, info, trace};
use url::Url;

pub struct TopStocksFetcher<'a> {
    page: &'a Page,
    count: usize,
    descending: bool,
}

impl<'a> TopStocksFetcher<'a> {
    pub async fn load_screen_url(
        page: &'a Page,
        screen_url: &str,
        count: usize,
        descending: bool,
    ) -> anyhow::Result<Self> {
        info!("Loading screen URL: {screen_url} (top {count})");

        page.goto(screen_url)
            .await?
            .wait_for_navigation()
            .await
            .with_context(|| format!("Navigating to {screen_url} failed"))?
            .sleep()
            .await;

        Ok(Self {
            page,
            count,
            descending,
        })
    }

    pub async fn fetch_stocks(
        &self,
        sort_by: &str,
    ) -> anyhow::Result<(Vec<Stock>, Vec<Performance>)> {
        self.sort_stocks(sort_by).await?;
        self.page.sleep().await;

        let sector_idx = self
            .add_sector_industry_columns("Sector")
            .await
            .context("Failed to add sector column")?;
        let industry_idx = self
            .add_sector_industry_columns("Industry")
            .await
            .context("Failed to add IndustryGroup column")?;

        info!("[{sort_by}] Querying rows from the table");
        let mut result = Vec::new();
        for row in self
            .page
            .sleep()
            .await
            .find_elements(r#"table tbody[data-testid="selectable-rows-table-body"] tr.listRow"#)
            .await
            .context("Failed to find stock rows")?
        {
            let stock = Self::parse_stock(row, sector_idx, industry_idx).await?;

            trace!("[{sort_by}] Parsed stock: {}", stock.ticker);

            result.push(stock);
            if result.len() >= self.count {
                break;
            }
        }

        info!("[{sort_by}] Fetched {}/{} stocks", result.len(), self.count);
        let perfs = parse_performances(self.page, TickerType::Stock).await?;

        Ok((result, perfs))
    }

    async fn parse_stock(
        row: Element,
        sector_idx: usize,
        industry_idx: usize,
    ) -> anyhow::Result<Stock> {
        async fn parse_group(cell: &Element) -> anyhow::Result<Group> {
            let anchor = cell.find_element("a").await?;
            let name = normalize(
                anchor
                    .inner_text()
                    .await?
                    .context("No inner html found in cell")?
                    .trim(),
            );
            let mut url = anchor
                .attribute("href")
                .await?
                .context("No link found in cell's anchor")?;
            if Url::parse(&url).is_err() {
                url = Url::parse(TV_HOME).unwrap().join(&url)?.to_string();
            }
            Ok(Group { name, url })
        }

        let row_key = row
            .attribute("data-rowkey")
            .await?
            .context("No data rowkey")?;
        let (exchange, ticker) = row_key
            .split_once(':')
            .map(|(exchange, stock)| (exchange.trim().to_uppercase(), stock.trim().to_uppercase()))
            .with_context(|| format!("Couldn't extract exchange & ticker from {row_key}"))?;

        let cells = row.find_elements("td").await?;
        let sector = parse_group(
            cells
                .get(sector_idx)
                .with_context(|| format!("Failed to get sector from column {sector_idx}"))?,
        )
        .await
        .context("Failed to parse sector")?;
        let industry = parse_group(
            cells
                .get(industry_idx)
                .with_context(|| format!("Failed to get sector from industry {industry_idx}"))?,
        )
        .await
        .context("Failed to parse industry group")?;

        Ok(Stock {
            ticker,
            exchange,
            sector,
            industry,
            last_update: Local::now().date_naive(),
        })
    }

    async fn sort_stocks(&self, sort_by: &str) -> anyhow::Result<()> {
        let direction = if self.descending {
            "descending"
        } else {
            "ascending"
        };
        info!("[{sort_by}] Sorting table ({direction})");

        debug!("[{sort_by}] Clicking performance tab");
        self.page
            .find_xpath(r"//button[@role='tab'][contains(., 'Performance')]")
            .await?
            .click()
            .await?;

        debug!("[{sort_by}] Clicking sort column header");
        self.page
            .sleep()
            .await
            .find_xpath(&format!(
                r#"//table//thead//th//div[contains(@class, 'bottomLine-')][.//div[text()='{}']]"#,
                sort_by,
            ))
            .await?
            .click()
            .await?;

        debug!("[{sort_by}] Selecting sort direction: {direction}");
        let sort_selector = format!(
            r#"//div[@data-qa-id="column-menu"]//div[@data-qa-id="column-menu-item"]//div[.//*[contains(., 'Sort {direction}')]]"#,
        );
        self.page
            .sleep()
            .await
            .find_xpath(sort_selector)
            .await
            .context("Failed to select sort direction")?
            .click()
            .await?;
        Ok(())
    }

    async fn add_sector_industry_columns(&self, col_name: &str) -> anyhow::Result<usize> {
        assert!(col_name == "Sector" || col_name == "Industry");

        if self
            .page
            .find_element(format!(r#"table thead th[data-field="{col_name}"]"#))
            .await
            .with_context(|| format!("Query to search {col_name} column failed"))
            .is_err()
        {
            info!("{col_name} column not present, adding it");
            self.page
                .find_element("table thead th#columns-plus-btn")
                .await
                .context("Couldn't find '+' button in the table header")?
                .click()
                .await?;
            self.page.sleep().await;

            debug!("Searching for {col_name} column");
            let input_field = self
                .page
                .find_element(r#"#overlap-manager-root input[aria-label="Search"]"#)
                .await
                .context("Couldn't find column input field")?;
            input_field.type_str(col_name).await?;
            self.page.sleep().await;

            debug!("Adding {col_name} column");
            self.page
                .find_element(
                    format!(r#"#overlap-manager-root div[data-qa-id="screener-add-filter-option-{col_name}"]"#),
                )
                .await
                .with_context(|| format!("Couldn't find {col_name} column in the add form"))?
                .click()
                .await?;

            if let Ok(confirm_btn) = self
                .page
                .find_element(
                    r#"div[data-qa-id="overlap-manager-root"] button[data-qa-id="apply-btn"]"#,
                )
                .await
            {
                info!("Clicking on confirmation button");
                confirm_btn.click().await?;
            }
            self.page.sleep().await;
        }
        for (idx, column) in self
            .page
            .find_elements(r#"table thead th"#)
            .await
            .with_context(|| format!("Couldn't find {col_name} column in the result table"))?
            .iter()
            .enumerate()
        {
            if let Some(data_field) = column
                .attribute("data-field")
                .await
                .context("Failed to read data-field on table header")?
                && data_field == col_name
            {
                return Ok(idx);
            }
        }

        anyhow::bail!("Couldn't add {col_name} column");
    }
}
