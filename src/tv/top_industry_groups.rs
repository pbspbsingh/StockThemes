use crate::tv::{Closeable, Sleepable};
use anyhow::Context;
use chromiumoxide::{Browser, Page};
use indicatif::{ProgressBar, ProgressStyle};

const INDUSTRY_GROUP_HOME: &str =
    "https://www.tradingview.com/markets/stocks-usa/sectorandindustry-industry/";

pub struct TopIndustryGroups {
    page: Page,
    pb: ProgressBar,
}

impl TopIndustryGroups {
    pub async fn new(browser: &Browser) -> anyhow::Result<Self> {
        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::default_spinner()
                .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"])
                .template("{spinner:.cyan} {msg}")?,
        );
        pb.tick();
        pb.set_message(format!("Loading {INDUSTRY_GROUP_HOME:?}"));
        let page = browser.new_page(INDUSTRY_GROUP_HOME).await?;
        page.wait_for_navigation().await?.sleep().await;
        pb.set_message("Done loading");
        pb.tick();
        Ok(Self { page, pb })
    }

    pub async fn fetch_top_industry_groups(
        self,
        sort_by: &str,
        len: usize,
    ) -> anyhow::Result<Vec<String>> {
        self.pb.set_message("Clicking 'Performance' tab");
        self.pb.tick();
        self.page.find_xpath(r#"//div[@id="market-screener-header-columnset-tabs"]//span[normalize-space()="Performance"]"#).await.context("Couldn't find performance tab")?.click().await?;
        self.page.sleep().await;

        self.pb
            .set_message(format!("Sorting industry groups by: {sort_by}"));
        self.pb.tick();
        self.page
            .find_element(format!(r#"th[data-field="Performance|Interval{sort_by}"]"#))
            .await
            .with_context(|| format!("Couldn't find performance tab for: {sort_by}"))?
            .click()
            .await?;
        self.page.sleep().await;

        let mut result = vec![];
        self.pb.set_message("Fetching top industry groups");
        self.pb.tick();
        for element in self
            .page
            .find_elements(
                r#"table tbody[data-testid="selectable-rows-table-body"] tr td:first-child a"#,
            )
            .await
            .context("Couldn't read table's body")?
        {
            let Some(ig) = element.inner_text().await? else {
                continue;
            };
            self.pb.set_message(format!("Parsed '{ig}'"));
            self.pb.tick();
            result.push(ig.trim().to_owned());
            if result.len() >= len {
                break;
            }
        }
        self.pb.finish_and_clear();
        self.page.close_me().await;
        Ok(result)
    }
}
