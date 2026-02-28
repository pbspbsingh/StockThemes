use anyhow::Context;
use chromiumoxide::{
    Element, Page,
    cdp::browser_protocol::input::{
        DispatchKeyEventParams, DispatchKeyEventType, InsertTextParams,
    },
};
use chrono::Local;

use super::TV_HOME;

use crate::{Group, Stock, StockInfoFetcher, tv::Sleepable};
use crate::util::normalize;

pub struct StockInfoLoader<'a> {
    page: &'a Page,
}

impl<'a> StockInfoLoader<'a> {
    pub async fn new(page: &'a Page) -> anyhow::Result<Self> {
        let url = page.url().await?.unwrap_or_default();
        let tv_home = format!("{TV_HOME}/markets/usa/");
        if !(url == tv_home || url.starts_with(&format!("{TV_HOME}/chart/"))) {
            page.goto(tv_home)
                .await?
                .wait_for_navigation()
                .await?
                .sleep()
                .await;
        }
        Ok(Self { page })
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
            let name = normalize(element.inner_text().await.ok()??.trim());
            let url = normalize(element.attribute("href").await.ok()??.trim());
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
}

#[async_trait::async_trait]
impl<'a> StockInfoFetcher for StockInfoLoader<'a> {
    async fn fetch(&self, ticker: &str) -> anyhow::Result<Stock> {
        self.fetch_stock_info(ticker).await
    }
}
