use anyhow::Context;
use chromiumoxide::{
    Browser, Element, Page,
    cdp::browser_protocol::{
        input::{DispatchKeyEventParams, DispatchKeyEventType, InsertTextParams},
        target::CloseTargetParams,
    },
};
use chrono::Local;
use log::info;

use super::TV_HOME;

use crate::{Group, Stock, StockInfoFetcher, tv::Sleepable};

pub struct StockInfoLoader {
    _browser: Browser,
    page: Page,
}

impl StockInfoLoader {
    pub async fn load(browser: Browser) -> anyhow::Result<Self> {
        for page in browser.pages().await? {
            if let Some(url) = page.url().await?
                && url.starts_with(TV_HOME)
            {
                info!("Reusing the existing page: {url}");
                return Ok(Self {
                    _browser: browser,
                    page,
                });
            }
        }

        let page = browser.new_page("about:blank").await?;
        page.goto(&format!("{TV_HOME}/markets/usa/"))
            .await?
            .wait_for_navigation()
            .await?
            .sleep()
            .await;

        Ok(Self {
            _browser: browser,
            page,
        })
    }

    pub async fn fetch_stock_info(&self, ticker: &str) -> anyhow::Result<Stock> {
        if let Ok(promo_button) = self
            .page
            .find_element("button[data-qa-id='promo-dialog-close-button']")
            .await
        {
            promo_button.click().await?;
        }

        if !self
            .page
            .url()
            .await?
            .unwrap_or_default()
            .starts_with(&format!("{TV_HOME}/chart/"))
        {
            self.page
                .find_element(r#"button[aria-label="Search"]"#)
                .await?
        } else {
            self.page
                .find_element("button#header-toolbar-symbol-search")
                .await?
        }
        .click()
        .await?;

        self.page
            .sleep()
            .await
            .execute(InsertTextParams::new(ticker))
            .await?;
        self.send_enter().await?;
        self.page.wait_for_navigation().await?.sleep().await;

        let mut error = None;
        for _ in 0..3 {
            match self.parse_ticker_info(ticker).await {
                Ok(res) => return Ok(res),
                Err(e) => {
                    error = Some(e);
                    self.page.sleep().await;
                }
            }
        }
        if let Some(error) = error {
            return Err(error);
        }
        anyhow::bail!("Failed to fetch stock info for {ticker}")
    }

    async fn parse_ticker_info(&self, ticker: &str) -> anyhow::Result<Stock> {
        let detail_widget = self
            .page
            .find_element(r#"div[data-test-id-widget-type="detail"]"#)
            .await
            .context("No detail widget found")?;
        let symbol = detail_widget
            .find_element(r#"span[data-qa-id="details-element symbol"]"#)
            .await
            .context("No exchange info found")?
            .inner_text()
            .await?
            .unwrap_or_default()
            .trim()
            .to_uppercase();
        if symbol != ticker {
            anyhow::bail!(
                "Wrong ticker got loaded in TradingView, expected {ticker:?} found {symbol:?}"
            )
        }

        let exchange = detail_widget
            .find_element(r#"span[data-qa-id="details-element exchange"]"#)
            .await
            .context("No exchange info found")?;
        let sector = detail_widget
            .find_element(r#"a[data-qa-id="details-element sector"]"#)
            .await
            .context("No sector info found")?;
        let industry = detail_widget
            .find_element(r#"a[data-qa-id="details-element industry"]"#)
            .await
            .context("No industry info found")?;

        async fn find_group(element: &Element) -> Option<Group> {
            let name = element.inner_text().await.ok()??.trim().to_owned();
            let url = element.attribute("href").await.ok()??.trim().to_owned();
            Some(Group { name, url })
        }

        Ok(Stock {
            ticker: ticker.to_owned(),
            exchange: exchange
                .inner_text()
                .await?
                .unwrap_or_default()
                .trim()
                .to_uppercase(),
            sector: find_group(&sector).await.context("Couldn't find sector")?,
            industry: find_group(&industry)
                .await
                .context("Couldn't find industry group")?,
            last_update: Local::now().date_naive(),
        })
    }

    async fn send_enter(&self) -> anyhow::Result<()> {
        // 1. KeyDown for Enter
        self.page
            .execute(
                DispatchKeyEventParams::builder()
                    .r#type(DispatchKeyEventType::KeyDown)
                    .key("Enter")
                    .code("Enter")
                    .windows_virtual_key_code(13) // Standard code for Enter
                    .build()
                    .unwrap(),
            )
            .await?;

        // 2. KeyUp for Enter
        self.page
            .execute(
                DispatchKeyEventParams::builder()
                    .r#type(DispatchKeyEventType::KeyUp)
                    .key("Enter")
                    .code("Enter")
                    .windows_virtual_key_code(13)
                    .build()
                    .unwrap(),
            )
            .await?;
        Ok(())
    }

    pub async fn close(&self) {
        let target_id = self.page.target_id().clone();
        self.page
            .execute(CloseTargetParams::new(target_id))
            .await
            .ok();
    }
}

#[async_trait::async_trait]
impl StockInfoFetcher for StockInfoLoader {
    async fn fetch(&self, ticker: &str) -> anyhow::Result<Stock> {
        self.fetch_stock_info(ticker).await
    }

    async fn done(&self) {
        self.close().await;
    }
}
