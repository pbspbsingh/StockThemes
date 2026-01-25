use anyhow::Ok;
use chromiumoxide::{Browser, Page};
use indicatif::ProgressBar;

use crate::tv::Sleepable;

pub struct TopStocksFetcher<'a> {
    page: Page,
    count: usize,
    pb: &'a ProgressBar,
}

impl<'a> TopStocksFetcher<'a> {
    pub async fn load(
        browser: &Browser,
        screen_url: &str,
        count: usize,
        pb: &'a ProgressBar,
    ) -> anyhow::Result<Self> {
        pb.set_message("Opening new tab");
        let page = browser.new_page("about:blank").await?;

        pb.set_message(format!("Loading {screen_url}"));
        page.goto(screen_url)
            .await?
            .wait_for_navigation()
            .await?
            .sleep()
            .await;

        Ok(Self { page, count, pb })
    }

    pub async fn fetch_stocks(&self, sort_by: &str) -> anyhow::Result<Vec<String>> {
        self.pb
            .set_message(format!("[{sort_by}] Clicking performance tab"));
        self.page
            .find_xpath(r"//button[@role='tab'][contains(., 'Performance')]")
            .await?
            .click()
            .await?;

        self.pb
            .set_message(format!("[{sort_by}] Sorting table by colum"));
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

        self.pb
            .set_message(format!("[{sort_by}] Clicking sort by popup button"));
        self.page
            .sleep().await
            .find_xpath(r#"//div[@data-qa-id="column-menu"]//div[@data-qa-id="column-menu-item"]//div[.//*[contains(., 'Sort descending')]]"#).await?
            .click().await?;

        self.pb
            .set_message(format!("[{sort_by}] Quering rows from the table"));
        let rows = self
            .page
            .sleep()
            .await
            .find_elements(r#"table tbody[data-testid="selectable-rows-table-body"] tr.listRow"#)
            .await?;

        let mut result = Vec::new();
        for row in rows {
            let Some(row_key) = row.attribute("data-rowkey").await? else {
                continue;
            };
            let Some(stock) = row_key.split_once(':').map(|(_, stock)| stock.trim()) else {
                continue;
            };

            self.pb.set_message(format!("[{stock}]"));
            self.pb.inc(1);

            result.push(stock.to_uppercase());
            if result.len() >= self.count {
                break;
            }
        }
        Ok(result)
    }
}
