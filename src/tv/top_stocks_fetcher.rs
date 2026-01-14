use std::sync::Arc;

use anyhow::Ok;
use headless_chrome::{Browser, Tab};
use indicatif::ProgressBar;
use log::warn;

use crate::tv::Sleepable;

pub struct TopStocksFetcher<'a> {
    tab: Arc<Tab>,
    count: usize,
    pb: &'a ProgressBar,
}

impl<'a> TopStocksFetcher<'a> {
    pub fn load(
        browser: &Browser,
        screen_url: &str,
        count: usize,
        pb: &'a ProgressBar,
    ) -> anyhow::Result<Self> {
        pb.set_message("Opening new tab");
        let tab = browser.new_tab()?;

        pb.set_message(format!("Loading {screen_url}"));
        tab.navigate_to(screen_url)?.wait_until_navigated()?.sleep();

        Ok(Self { tab, count, pb })
    }

    pub fn fetch_stocks(&self, sort_by: &str) -> anyhow::Result<Vec<String>> {
        self.pb
            .set_message(format!("[{sort_by}] Clicking performance tab"));
        self.tab
            .wait_for_xpath(r"//button[@role='tab'][contains(., 'Performance')]")?
            .click()?;

        self.pb
            .set_message(format!("[{sort_by}] Sorting table by colum"));
        self.tab
            .sleep()
            .wait_for_xpath(&format!(
                r#"//table//thead//th//div[contains(@class, 'bottomLine-')][.//div[text()='{}']]"#,
                sort_by,
            ))?
            .click()?;

        self.pb
            .set_message(format!("[{sort_by}] Clicking sort by popup button"));
        self.tab
            .sleep()
            .wait_for_xpath(r#"//div[@data-qa-id="column-menu"]//div[@data-qa-id="column-menu-item"]//div[.//*[contains(., 'Sort descending')]]"#)?
            .click()?;

        self.pb
            .set_message(format!("[{sort_by}] Quering rows from the table"));
        let rows = self.tab.sleep().wait_for_elements(
            r#"table tbody[data-testid="selectable-rows-table-body"] tr.listRow"#,
        )?;

        let mut result = Vec::new();
        for row in rows {
            let Some(row_key) = row.get_attribute_value("data-rowkey")? else {
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

impl<'a> Drop for TopStocksFetcher<'a> {
    fn drop(&mut self) {
        if let Err(e) = self.tab.close(false) {
            warn!("Failed to close the TradingView tab properly: {e}");
        }
    }
}
