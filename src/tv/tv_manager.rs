use crate::config::APP_CONFIG;
use crate::store::Store;
use crate::tv::screenshot::SnapOnErr;
use crate::tv::stock_info_loader::StockInfoLoader;
use crate::tv::top_industry_groups::TopIndustryGroups;
use crate::tv::top_stocks_fetcher::TopStocksFetcher;
use crate::{Performance, Stock, TickerType};
use chrome_driver::chromiumoxide::cdp::browser_protocol::page::{
    EventJavascriptDialogOpening, HandleJavaScriptDialogParams,
};
use chrome_driver::{Browser, ChromeDriverConfig, Page, PageFeatures};
use futures::StreamExt;

use std::collections::HashSet;
use std::slice;
use std::sync::Arc;
use tracing::{info, warn};

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

        let page = self.get_or_init_page().await?.clone();
        let tig = TopIndustryGroups::new(&page)
            .await
            .snap_on_err(&page, "industry_groups_init")
            .await?;
        let sectors = tig
            .fetch_sectors()
            .await
            .snap_on_err(&page, "fetch_sectors")
            .await?;
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

        let page = self.get_or_init_page().await?.clone();
        let tig = TopIndustryGroups::new(&page)
            .await
            .snap_on_err(&page, "industry_groups_init")
            .await?;
        let industries = tig
            .fetch_industries()
            .await
            .snap_on_err(&page, "fetch_industries")
            .await?;
        self.store.save_performances(&industries).await?;

        Ok(industries)
    }

    pub async fn fetch_stock_info(&mut self, ticker: &str) -> anyhow::Result<Stock> {
        if let Some(stock) = self.store.get_stock(ticker).await? {
            return Ok(stock);
        }

        let page = self.get_or_init_page().await?.clone();
        let si_loader = StockInfoLoader::new(&page)
            .await
            .snap_on_err(&page, &format!("stock_info_init_{ticker}"))
            .await?;
        let stock = si_loader
            .fetch_stock_info(ticker)
            .await
            .snap_on_err(&page, &format!("stock_info_{ticker}"))
            .await?;

        self.store.add_stocks(slice::from_ref(&stock)).await?;

        Ok(stock)
    }

    pub async fn fetch_top_stocks(
        &mut self,
        screen_url: &str,
        top_count: usize,
        is_desc: bool,
        time_frames: impl Iterator<Item = String>,
    ) -> anyhow::Result<Vec<String>> {
        let mut unique_stocks = HashSet::new();

        let page = self.get_or_init_page().await?;
        let fetcher = TopStocksFetcher::load_screen_url(page, screen_url, top_count, is_desc)
            .await
            .snap_on_err(page, "load_screen_url")
            .await?;
        for sort_by in time_frames {
            let stocks = fetcher
                .fetch_stocks(&sort_by)
                .await
                .snap_on_err(page, &format!("fetch_stocks_{sort_by}"))
                .await?;
            unique_stocks.extend(stocks.into_iter().map(|s| s.1));
        }

        Ok(unique_stocks.into_iter().collect())
    }

    async fn get_or_init_page(&mut self) -> anyhow::Result<&Page> {
        if self.page.is_none() {
            info!("TvFetcher: cache miss — launching browser");
            let browser = ChromeDriverConfig::new(&APP_CONFIG.chrome_path)
                .user_data_dir(&APP_CONFIG.user_data_dir)
                .args(&APP_CONFIG.chrome_args)
                .launch_if_needed(APP_CONFIG.launch_chrome_if_needed)
                .connect()
                .await?;
            let page = browser.new_page("about:blank").await?;
            Self::auto_accept_dialogs(&page).await?;
            self.browser = Some(browser);
            self.page = Some(page);
        }
        Ok(self.page.as_ref().unwrap())
    }

    async fn auto_accept_dialogs(page: &Page) -> anyhow::Result<()> {
        let mut dialogs = page
            .event_listener::<EventJavascriptDialogOpening>()
            .await?;
        let page = page.clone();
        tokio::spawn(async move {
            while let Some(ev) = dialogs.next().await {
                info!("Auto-accepting JS dialog ({:?}): {}", ev.r#type, ev.message);
                if let Err(e) = page.execute(HandleJavaScriptDialogParams::new(true)).await {
                    warn!("Failed to auto-accept JS dialog: {e}");
                }
            }
        });
        Ok(())
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
