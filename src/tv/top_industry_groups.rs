use crate::tv::Sleepable;
use crate::{Performance, TickerType};
use anyhow::Context;
use chromiumoxide::Page;
use indicatif::{ProgressBar, ProgressStyle};

use crate::tv::perf_util::parse_performances;

const SECTORS_HOME: &str =
    "https://www.tradingview.com/markets/stocks-usa/sectorandindustry-sector/";

pub struct TopIndustryGroups<'a> {
    page: &'a Page,
    pb: ProgressBar,
}

impl<'a> TopIndustryGroups<'a> {
    pub async fn new(page: &'a Page) -> anyhow::Result<Self> {
        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::default_spinner()
                .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"])
                .template("{spinner:.cyan} {msg}")?,
        );

        pb.tick();
        pb.set_message(format!("Loading {SECTORS_HOME:?}"));

        page.goto(SECTORS_HOME)
            .await?
            .wait_for_navigation()
            .await?
            .sleep()
            .await;

        pb.tick();
        pb.set_message("Done loading");

        Ok(Self { page, pb })
    }

    pub async fn fetch_sectors(&self) -> anyhow::Result<Vec<Performance>> {
        self.set_message("Clicking 'Sector' tab");
        self.page
            .find_xpath("a#sector")
            .await
            .context("Couldn't find Sector tab")?
            .click()
            .await?;
        self.page.sleep().await;

        self.click_perf_tab().await?;

        parse_performances(&self.page, TickerType::Sector).await
    }

    pub async fn fetch_industries(&self) -> anyhow::Result<Vec<Performance>> {
        self.set_message("Clicking 'Industry' tab");
        self.page
            .find_xpath("a#industry")
            .await
            .context("Couldn't find Industry tab")?
            .click()
            .await?;
        self.page.sleep().await;

        self.click_perf_tab().await?;

        if let Ok(load_more) = self
            .page
            .find_element(r#"button[data-overflow-tooltip-text="Load More"]"#)
            .await
        {
            self.set_message("Loading more industries");
            load_more.click().await?;
            self.page.sleep().await;
        }

        parse_performances(&self.page, TickerType::Industry).await
    }

    fn set_message(&self, msg: impl Into<String>) {
        self.pb.tick();
        self.pb.set_message(msg.into());
    }

    async fn click_perf_tab(&self) -> anyhow::Result<()> {
        self.set_message("Clicking 'Performance' tab");
        self.page
            .find_xpath(r#"//div[@id="market-screener-header-columnset-tabs"]//span[normalize-space()="Performance"]"#)
            .await
            .context("Couldn't find performance tab")?
            .click()
            .await?;
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        Ok(())
    }
}
