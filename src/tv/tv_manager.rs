use crate::config::APP_CONFIG;
use crate::store::Store;
use crate::tv::stock_info_loader::StockInfoLoader;
use crate::tv::top_industry_groups::TopIndustryGroups;
use crate::tv::top_stocks_fetcher::TopStocksFetcher;
use crate::{Performance, Stock, TickerType};
use chrome_driver::{Browser, ChromeDriverConfig, Page, PageFeatures};
use itertools::Itertools;
use std::collections::HashMap;
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

    pub async fn fetch_stock_info(&mut self, ticker: &str) -> anyhow::Result<Stock> {
        if let Some(stock) = self.store.get_stock(ticker).await? {
            return Ok(stock);
        }

        let si_loader = StockInfoLoader::new(self.get_or_init_page().await?).await?;
        let stock = si_loader.fetch_stock_info(ticker).await?;

        self.store.add_stocks(slice::from_ref(&stock)).await?;

        Ok(stock)
    }

    pub async fn fetch_top_stocks(
        &mut self,
        screen_url: &str,
        top_count: usize,
        is_desc: bool,
        time_frames: impl Iterator<Item = String>,
    ) -> anyhow::Result<(Vec<Stock>, Vec<Performance>)> {
        let store = self.store.clone();

        let (screener_stocks, perfs) = {
            let fetcher = TopStocksFetcher::load_screen_url(
                self.get_or_init_page().await?,
                screen_url,
                top_count,
                is_desc,
            )
            .await?;

            let mut stocks_map: HashMap<String, Stock> = HashMap::new();
            let mut perf_map: HashMap<String, Performance> = HashMap::new();
            for sort_by in time_frames {
                let (stocks, perfs) = fetcher.fetch_stocks(&sort_by).await?;

                // Persist perfs only — screener sector/industry is unreliable, don't store it.
                store.save_performances(&perfs).await?;

                for stock in stocks {
                    stocks_map.insert(stock.ticker.clone(), stock);
                }
                for perf in perfs {
                    perf_map.insert(perf.ticker.clone(), perf);
                }
            }

            let stocks: Vec<Stock> = stocks_map
                .into_values()
                .sorted_by_key(|s| s.ticker.clone())
                .collect();
            let perfs: Vec<Performance> = perf_map
                .into_values()
                .sorted_by_key(|p| p.ticker.clone())
                .collect();
            (stocks, perfs)
        };

        let total = screener_stocks.len();
        info!("Validating stock info for {total} tickers via detail page");
        let mut stocks = Vec::with_capacity(total);
        for (i, screener) in screener_stocks.into_iter().enumerate() {
            info!("[{}/{total}] Validating {}", i + 1, screener.ticker);
            let stock = match self.fetch_stock_info(&screener.ticker).await {
                Ok(detail) => {
                    if screener.exchange != detail.exchange
                        || screener.sector.name != detail.sector.name
                        || screener.industry.name != detail.industry.name
                    {
                        warn!(
                            "Mismatch for {}: screener=(exchange={}, sector={}, industry={}) detail=(exchange={}, sector={}, industry={}); using detail",
                            screener.ticker,
                            screener.exchange,
                            screener.sector.name,
                            screener.industry.name,
                            detail.exchange,
                            detail.sector.name,
                            detail.industry.name,
                        );
                    }
                    detail
                }
                Err(e) => {
                    warn!(
                        "fetch_stock_info failed for {}: {e}; falling back to screener data",
                        screener.ticker
                    );
                    screener
                }
            };
            stocks.push(stock);
        }

        Ok((stocks, perfs))
    }

    async fn industry_groups(&mut self) -> anyhow::Result<TopIndustryGroups<'_>> {
        let page = self.get_or_init_page().await?;
        TopIndustryGroups::new(page).await
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
