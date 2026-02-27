use crate::store::Store;
use crate::tv::Closeable;
use crate::tv::top_industry_groups::TopIndustryGroups;
use crate::tv::top_stocks_fetcher::TopStocksFetcher;
use crate::{Performance, Stock, TickerType, browser};
use chromiumoxide::{Browser, Page};
use itertools::Itertools;
use log::info;
use std::collections::HashMap;
use std::sync::Arc;

pub struct TvManager {
    store: Arc<Store>,
    browser: Option<Browser>,
    page: Option<Page>,
}

impl TvManager {
    pub fn new(store: Arc<Store>) -> Self {
        Self {
            store,
            browser: None,
            page: None,
        }
    }

    pub async fn fetch_sectors(&mut self) -> anyhow::Result<Vec<Performance>> {
        let cached = self
            .store
            .get_performances_by_type(TickerType::Sector)
            .await?;
        if !cached.is_empty() {
            info!("Sectors loaded from store ({} entries)", cached.len());
            return Ok(cached);
        }

        let tig = self.industry_groups().await?;
        let sectors = tig.fetch_sectors().await?;
        self.store.save_performances(&sectors).await?;

        Ok(sectors)
    }

    pub async fn fetch_industries(&mut self) -> anyhow::Result<Vec<Performance>> {
        let cached = self
            .store
            .get_performances_by_type(TickerType::Industry)
            .await?;
        if !cached.is_empty() {
            info!("Industries loaded from store ({} entries)", cached.len());
            return Ok(cached);
        }

        let tig = self.industry_groups().await?;
        let industries = tig.fetch_industries().await?;
        self.store.save_performances(&industries).await?;

        Ok(industries)
    }

    pub async fn fetch_top_stocks(
        &mut self,
        screen_url: &str,
        top_count: usize,
        is_desc: bool,
        time_frames: impl Iterator<Item = String>,
    ) -> anyhow::Result<(Vec<Stock>, Vec<Performance>)> {
        let store = self.store.clone();

        let fetcher = TopStocksFetcher::load_screen_url(
            self.get_or_init_page().await?,
            screen_url,
            top_count,
            is_desc,
        )
        .await?;

        Self::fetch_stocks(store, fetcher, time_frames).await
    }

    pub async fn fetch_top_stocks_with_industries_filter(
        &mut self,
        base_screen_url: &str,
        top_count: usize,
        industries: &[String],
        time_frames: impl Iterator<Item = String>,
    ) -> anyhow::Result<(Vec<Stock>, Vec<Performance>)> {
        let store = self.store.clone();

        let fetcher = TopStocksFetcher::load_screen_with_industries(
            self.get_or_init_page().await?,
            base_screen_url,
            top_count,
            industries,
        )
        .await?;

        Self::fetch_stocks(store, fetcher, time_frames).await
    }

    async fn fetch_stocks<'a>(
        store: Arc<Store>,
        fetcher: TopStocksFetcher<'a>,
        time_frames: impl Iterator<Item = String>,
    ) -> anyhow::Result<(Vec<Stock>, Vec<Performance>)> {
        let mut stocks_map = HashMap::new();
        let mut perf_map = HashMap::new();
        for sort_by in time_frames {
            let (stocks, perfs) = fetcher.fetch_stocks(&sort_by).await?;

            store.add_stocks(&stocks, true).await?;
            store.save_performances(&perfs).await?;

            for stock in stocks {
                stocks_map.insert(stock.ticker.clone(), stock);
            }
            for perf in perfs {
                perf_map.insert(perf.ticker.clone(), perf);
            }
        }

        Ok((
            stocks_map
                .into_values()
                .sorted_by_key(|s| s.ticker.clone())
                .collect(),
            perf_map
                .into_values()
                .sorted_by_key(|s| s.ticker.clone())
                .collect(),
        ))
    }

    async fn industry_groups(&mut self) -> anyhow::Result<TopIndustryGroups<'_>> {
        let page = self.get_or_init_page().await?;
        TopIndustryGroups::new(page).await
    }

    async fn get_or_init_page(&mut self) -> anyhow::Result<&Page> {
        if self.page.is_none() {
            info!("TvFetcher: cache miss — launching browser");
            let browser = browser::init_browser().await?;
            let page = browser.new_page("about:blank").await?;
            self.browser = Some(browser);
            self.page = Some(page);
        }
        Ok(self.page.as_ref().unwrap())
    }
}

impl Drop for TvManager {
    fn drop(&mut self) {
        if let Some(browser) = self.browser.take()
            && let Some(page) = self.page.take()
        {
            tokio::spawn(async move {
                page.close_me().await;
                drop(browser);
            });
        }
    }
}
