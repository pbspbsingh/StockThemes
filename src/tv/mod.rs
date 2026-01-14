use std::time::Duration;

use headless_chrome::Tab;

pub mod stock_info_loader;
pub mod top_stocks_fetcher;

const TV_HOME: &str = "https://www.tradingview.com";

trait Sleepable {
    fn sleep(&self) -> &Self;
}

impl Sleepable for Tab {
    fn sleep(&self) -> &Self {
        let sleep_time = rand::random_range(500..2000);
        std::thread::sleep(Duration::from_millis(sleep_time));
        self
    }
}
