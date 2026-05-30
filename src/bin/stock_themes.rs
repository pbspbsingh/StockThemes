use clap::Parser;

use tracing::{info, warn};

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;
use stock_themes::{Stock, init_logger, metrics, rs, start_http_server, store::Store, util};

use stock_themes::summary::Summary;
use stock_themes::tv::screener_api::ScreenerApi;
use stock_themes::yf::YFinance;

#[derive(Parser, Debug)]
#[command(name = "stock_themes")]
#[command(about = "Process csv files with stocks to find the common themes among them")]
pub struct StockThemesArgs {
    /// Input files to process
    #[arg(required = true)]
    pub files: Vec<PathBuf>,

    /// Number of items to skip
    #[arg(short = 'n', long, default_value_t = 4)]
    pub skip_lines: usize,

    /// Comma seperated list of Stocks to skip
    #[arg(short = 's', long, default_value = "")]
    pub skip_stocks: String,
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> anyhow::Result<()> {
    init_logger();

    let args = StockThemesArgs::parse();
    info!("args: {args:#?}");

    let yf = YFinance::new();
    let store = Store::load_store().await?;

    let tickers = util::read_stocks(&args.files, args.skip_lines, &args.skip_stocks).await?;
    info!("Total unique stocks: {}", tickers.len());

    let stocks = fetch_stock_info(&store, tickers).await?;
    let rs_maps = rs::build_rs_maps(&store, &yf, &stocks).await?;

    let stock_metrics = metrics::build_stock_metrics(&store, &yf, &stocks).await?;
    info!("Computed metrics for {} stocks", stock_metrics.len());

    let summary = Summary::summarize(stocks);
    let html = summary.render(
        rs_maps.sectors,
        rs_maps.industries,
        rs_maps.stocks,
        stock_metrics,
    );

    start_http_server(html).await
}

async fn fetch_stock_info(store: &Store, tickers: Vec<String>) -> anyhow::Result<Vec<Stock>> {
    let start = Instant::now();
    let mut cached_stocks = HashMap::new();
    let mut missing_tickers = Vec::new();
    for ticker in &tickers {
        match store.get_stock(ticker).await? {
            Some(stock) => {
                cached_stocks.insert(ticker.clone(), stock);
            }
            None => missing_tickers.push(ticker.clone()),
        }
    }

    if !missing_tickers.is_empty() {
        info!(
            "Fetching {} stocks info from TradingView API",
            missing_tickers.len()
        );
        let stock_info_fetcher = ScreenerApi::new()?;
        let fetched_stocks = stock_info_fetcher.fetch_stocks(&missing_tickers).await?;
        if !fetched_stocks.is_empty() {
            let fetched = fetched_stocks.values().cloned().collect::<Vec<_>>();
            store.add_stocks(&fetched).await?;
            cached_stocks.extend(fetched_stocks);
        }
    }

    let missing_tickers = tickers
        .into_iter()
        .filter(|t| !cached_stocks.contains_key(t))
        .collect::<Vec<_>>();
    if !missing_tickers.is_empty() {
        warn!(
            "Failed to fetch stock info for {} tickers",
            missing_tickers.len()
        );
        warn!("Failed tickers: '{}'", missing_tickers.join(","));
        anyhow::bail!(
            "Couldn't fetch stock info for '{}'",
            missing_tickers.join(",")
        );
    }
    info!(
        "Finished processing {} tickers in {:?}",
        cached_stocks.len(),
        start.elapsed(),
    );

    Ok(cached_stocks.into_values().collect())
}
