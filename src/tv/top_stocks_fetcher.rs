use crate::tv::perf_util::parse_performances;
use crate::tv::{Sleepable, TV_HOME};
use crate::{Group, Performance, Stock, TickerType};
use anyhow::{Context, Ok};
use chromiumoxide::{Element, Page};
use chrono::Local;
use indicatif::{ProgressBar, ProgressStyle};
use url::Url;

pub struct TopStocksFetcher<'a> {
    page: &'a Page,
    count: usize,
    descending: bool,
    pb: ProgressBar,
}

impl<'a> TopStocksFetcher<'a> {
    pub async fn load_screen_url(
        page: &'a Page,
        screen_url: &str,
        count: usize,
        descending: bool,
    ) -> anyhow::Result<Self> {
        let pb = ProgressBar::new(count as u64);
        pb.set_style(ProgressStyle::default_bar().template(
            "{spinner:.green} [{elapsed_precise}] [{bar:50.cyan/blue}] {pos}/{len} {msg}",
        )?);
        pb.set_message(format!("Loading {screen_url}"));

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
            pb,
        })
    }

    pub async fn load_screen_with_industries(
        page: &'a Page,
        base_url: &str,
        count: usize,
        industries: &[String],
    ) -> anyhow::Result<Self> {
        let pb = ProgressBar::new(industries.len() as u64);
        pb.set_style(ProgressStyle::default_bar().template(
            "{spinner:.green} [{elapsed_precise}] [{bar:50.cyan/blue}] {pos}/{len} {msg}",
        )?);
        pb.set_message(format!("Loading {base_url}"));

        page.goto(base_url)
            .await?
            .wait_for_navigation()
            .await
            .with_context(|| format!("Navigating to {base_url} failed"))?
            .sleep()
            .await;

        let industry_filter_selector = r#"button[data-qa-id="ui-lib-multiselect-filter-pill screener-pills-checkbox-pill-Industry"]"#;
        if page.find_element(industry_filter_selector).await.is_err() {
            pb.set_message("Clicking 'Add Filter' button");
            page.find_element(r#"button[data-qa-id="screener-add-new-filter"]"#)
                .await
                .context("Failed to find AddFilter button")?
                .click()
                .await?;
            page.sleep().await;

            pb.set_message("Searching for industry filter");
            page.find_element(r#"input[aria-label="Type filter name"]"#)
                .await
                .context("Failed to find Add filter input")?
                .type_str("Industry")
                .await?;
            page.sleep().await;

            pb.set_message("Clicking the Industry filter");
            page.find_element(r#"div[data-qa-id="screener-add-filter-option__Industry"]"#)
                .await
                .context("Failed to find Industry button in filter list")?
                .click()
                .await?;
            page.sleep().await;
        }

        pb.set_message("Clicking on Industry filter");
        page.find_element(industry_filter_selector)
            .await
            .context("Failed to find Industry filter")?
            .click()
            .await?;
        page.sleep().await;

        pb.set_message("Resetting industry filter");
        page.find_xpath(r#"//div[@id='overlap-manager-root']//button[.//*[contains(text(),'Reset')] or contains(text(),'Reset')]"#)
            .await
            .context("Failed to find Reset button in Industry filter pane")?
            .click()
            .await?;

        for industry in industries {
            pb.inc(1);
            pb.set_message(format!("Selecting {industry}"));
            page.find_xpath(format!(r#"//div[@id='overlap-manager-root']//div[@role='listbox']//div[contains(@id, '{industry}')]"#))
                .await
                .with_context(|| format!("Failed to find industry group '{industry}' in Industry filter dropdown"))?
                .scroll_into_view()
                .await?
                .click()
                .await?;
            page.nap().await;
        }
        pb.set_length(count as u64);
        pb.reset();
        Ok(Self {
            page,
            count,
            descending: true,
            pb,
        })
    }

    pub async fn fetch_stocks(
        &self,
        sort_by: &str,
    ) -> anyhow::Result<(Vec<Stock>, Vec<Performance>)> {
        self.pb.reset();
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

        self.pb
            .set_message(format!("[{sort_by}] Quering rows from the table"));
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

            self.pb.set_message(format!("[{}]", stock.ticker));
            self.pb.inc(1);

            result.push(stock);
            if result.len() >= self.count {
                break;
            }
        }

        let perfs = parse_performances(&self.page, TickerType::Stock).await?;

        Ok((result, perfs))
    }

    async fn parse_stock(
        row: Element,
        sector_idx: usize,
        industry_idx: usize,
    ) -> anyhow::Result<Stock> {
        async fn parse_group(cell: &Element) -> anyhow::Result<Group> {
            let anchor = cell.find_element("a").await?;
            let name = anchor
                .inner_text()
                .await?
                .context("No inner html found in cell")?
                .trim()
                .to_owned();
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
        let sort_selector = format!(
            r#"//div[@data-qa-id="column-menu"]//div[@data-qa-id="column-menu-item"]//div[.//*[contains(., 'Sort {}')]]"#,
            if self.descending {
                "descending"
            } else {
                "ascending"
            }
        );
        self.page
            .sleep()
            .await
            .find_xpath(sort_selector)
            .await?
            .click()
            .await?;
        Ok(())
    }

    async fn add_sector_industry_columns(&self, col_name: &str) -> anyhow::Result<usize> {
        assert!(col_name == "Sector" || col_name == "Industry");

        self.pb.set_message("Clicking Custom tab");
        self.page
            .find_xpath(r"//button[@role='tab'][contains(., 'Custom')]")
            .await?
            .click()
            .await?;
        self.page.sleep().await;

        if self
            .page
            .find_element(format!(r#"table thead th[data-field="{col_name}"]"#))
            .await
            .is_err()
        {
            self.pb
                .set_message(format!("{col_name} column is not present, adding it."));
            self.page
                .find_element("table thead th#columns-plus-btn")
                .await
                .context("Couldn't find '+' button in the table header")?
                .click()
                .await?;
            self.page.sleep().await;

            self.pb
                .set_message(format!("Searching for {col_name} column"));
            let input_field = self
                .page
                .find_element(r#"#overlap-manager-root input[aria-label="Type column name"]"#)
                .await
                .context("Couldn't find column input field")?;
            input_field.type_str(col_name).await?;
            self.page.sleep().await;

            self.pb.set_message(format!("Adding {col_name} column"));
            self.page
                .find_element(
                    format!(r#"#overlap-manager-root div[data-qa-id="screener-add-filter-option__{col_name}"]"#),
                )
                .await
                .with_context(|| format!("Couldn't find {col_name} column in the add form"))?
                .click()
                .await?;
            self.page.sleep().await;
        }
        for (idx, column) in self
            .page
            .find_elements(r#"table thead th"#)
            .await?
            .iter()
            .enumerate()
        {
            if let Some(data_field) = column.attribute("data-field").await?
                && data_field == col_name
            {
                return Ok(idx);
            }
        }

        anyhow::bail!("Couldn't add {col_name} column");
    }
}
