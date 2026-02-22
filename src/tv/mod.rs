use chromiumoxide::Page;
use chromiumoxide::cdp::browser_protocol::target::CloseTargetParams;
use std::time::Duration;
use tokio::time;

pub mod stock_info_loader;
pub mod top_industry_groups;
pub mod top_stocks_fetcher;

const TV_HOME: &str = "https://www.tradingview.com";

trait Sleepable {
    async fn nap(&self) -> &Self;
    async fn sleep(&self) -> &Self;
}

impl Sleepable for Page {
    async fn nap(&self) -> &Self {
        let sleep_time = rand::random_range(50..250);
        time::sleep(Duration::from_millis(sleep_time)).await;
        self
    }

    async fn sleep(&self) -> &Self {
        let sleep_time = rand::random_range(500..2500);
        time::sleep(Duration::from_millis(sleep_time)).await;
        self
    }
}

trait Closeable {
    async fn close_me(&self);
}

impl Closeable for Page {
    async fn close_me(&self) {
        let target_id = self.target_id().clone();
        self.execute(CloseTargetParams::new(target_id)).await.ok();
    }
}
