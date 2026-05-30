use anyhow::Context;
use chrome_driver::{Element, Page, PageFeatures};

use tracing::{debug, info, trace};

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

    pub async fn fetch_stocks(&self, sort_by: &str) -> anyhow::Result<Vec<(String, String)>> {
        self.sort_stocks(sort_by).await?;
        self.page.sleep().await;

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
            let stock = Self::parse_stock(row).await?;
            trace!("[{sort_by}] Parsed stock: {}", stock.1);
            result.push(stock);

            if result.len() >= self.count {
                break;
            }
        }

        info!("[{sort_by}] Fetched {}/{} stocks", result.len(), self.count);
        Ok(result)
    }

    async fn parse_stock(row: Element) -> anyhow::Result<(String, String)> {
        let row_key = row
            .attribute("data-rowkey")
            .await?
            .context("No data rowkey")?;
        let (exchange, ticker) = row_key
            .split_once(':')
            .map(|(exchange, stock)| (exchange.trim().to_uppercase(), stock.trim().to_uppercase()))
            .with_context(|| format!("Couldn't extract exchange & ticker from {row_key}"))?;
        Ok((exchange, ticker))
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
}
