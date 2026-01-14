use std::{sync::Arc, time::Duration};

use super::TV_HOME;
use crate::{Group, Stock, tv::Sleepable};
use anyhow::Context;
use chrono::Local;
use headless_chrome::{Browser, Element, Tab, browser::tab::ModifierKey};
use log::warn;

pub struct StockInfoLoader {
    tab: Arc<Tab>,
}

impl StockInfoLoader {
    pub fn load(browser: &Browser) -> anyhow::Result<Self> {
        let tab = browser.new_tab()?;
        tab.navigate_to(&format!("{TV_HOME}/markets/usa/"))?
            .wait_until_navigated()?;
        Ok(Self { tab })
    }

    pub fn fetch_stock_info(&self, ticker: &str) -> anyhow::Result<Stock> {
        if let Ok(promo_button) = self
            .tab
            .find_element("button[data-qa-id='promo-dialog-close-button']")
        {
            promo_button.click()?;
        }

        if !self.tab.get_url().starts_with(&format!("{TV_HOME}/chart/")) {
            self.tab
                .press_key_with_modifiers("k", Some(&[ModifierKey::Meta]))?
                .sleep();
        }

        self.tab
            .type_str(ticker)?
            .sleep()
            .press_key("Enter")?
            .wait_until_navigated()?
            .sleep();

        let timeout = Duration::from_secs(2);
        let detail_widget = self
            .tab
            .wait_for_element_with_custom_timeout(
                r#"div[data-test-id-widget-type="detail"]"#,
                timeout,
            )
            .context("No detail widget found")?;
        let symbol = detail_widget
            .wait_for_element_with_custom_timeout(
                r#"span[data-qa-id="details-element symbol"]"#,
                timeout,
            )
            .context("No exchange info found")?
            .get_inner_text()?
            .trim()
            .to_uppercase();
        if symbol != ticker {
            anyhow::bail!(
                "Wrong ticker got loaded in TradingView, expected {ticker:?} found {symbol:?}"
            )
        }

        let exchange = detail_widget
            .wait_for_element_with_custom_timeout(
                r#"span[data-qa-id="details-element exchange"]"#,
                timeout,
            )
            .context("No exchange info found")?;
        let sector = detail_widget
            .wait_for_element_with_custom_timeout(
                r#"a[data-qa-id="details-element sector"]"#,
                timeout,
            )
            .context("No sector info found")?;
        let industry = detail_widget
            .wait_for_element_with_custom_timeout(
                r#"a[data-qa-id="details-element industry"]"#,
                timeout,
            )
            .context("No industry info found")?;

        fn find_group(element: &Element) -> Option<Group> {
            let name = element.get_inner_text().ok()?.trim().to_owned();
            let url = element.get_attribute_value("href").ok()??.trim().to_owned();
            Some(Group { name, url })
        }

        Ok(Stock {
            ticker: ticker.to_owned(),
            exchange: exchange.get_inner_text()?.trim().to_uppercase(),
            sector: find_group(&sector).context("Couldn't find sector")?,
            industry: find_group(&industry).context("Couldn't find sector")?,
            last_update: Local::now().date_naive(),
        })
    }
}

impl Drop for StockInfoLoader {
    fn drop(&mut self) {
        if let Err(e) = self.tab.close(false) {
            warn!("Failed to close the TradingView tab properly: {e}");
        }
    }
}
