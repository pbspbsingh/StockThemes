use std::time::Duration;

use chromiumoxide::Page;
use tokio::time;

pub mod stock_info_loader;
pub mod top_stocks_fetcher;

const TV_HOME: &str = "https://www.tradingview.com";

trait Sleepable {
    async fn sleep(&self) -> &Self;
}

impl Sleepable for Page {
    async fn sleep(&self) -> &Self {
        let sleep_time = rand::random_range(500..2500);
        time::sleep(Duration::from_millis(sleep_time)).await;
        self
    }
}
