use anyhow::Context;

use clap::Parser;
use futures::{stream, StreamExt, TryStreamExt};
use indicatif::{ProgressBar, ProgressStyle};
use itertools::Itertools;
use log::{error, info, warn};
use std::sync::Arc;
use std::time::Duration;
use std::{collections::HashMap, path::PathBuf};
use stock_themes::{
    browser, config::APP_CONFIG, init_logger, start_http_server, store::Store, tv::stock_info_loader::StockInfoLoader,
    util, Stock, StockInfoFetcher,
};

use stock_themes::summary::Summary;
use stock_themes::yf::YFinance;
use tokio::{fs, time};

const HTML_FILE: &str = "stocks_themes.html";

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
    info!(
        "Reading {} csv files, skipping {} lines",
        args.files.len(),
        args.skip_lines,
    );

    let stocks = util::read_stocks(&args.files, args.skip_lines, &args.skip_stocks).await?;
    info!("Total unique stocks: {}", stocks.len());

    let stocks = fetch_stock_info(stocks).await?;
    info!("Fetched stock info of {} stocks", stocks.len());

    let summary = Summary::summarize(stocks);
    let html = summary.render(vec![]);
    fs::write(HTML_FILE, &html)
        .await
        .with_context(|| format!("Failed to write {HTML_FILE}"))?;
    info!(
        "Done! Wrote html to {:?}",
        fs::canonicalize(HTML_FILE).await?
    );

    start_http_server(html).await
}

async fn fetch_stock_info(stocks: Vec<String>) -> anyhow::Result<Vec<Stock>> {
    let use_tv = APP_CONFIG.use_tv_for_stock_info;
    let store = Store::load_store().await?;

    let new_stocks: Vec<_> = stream::iter(stocks.iter())
        .filter(|&ticker| {
            let value = store.clone();
            async move { value.get_stock(ticker, use_tv).await.ok().flatten().is_none() }
        })
        .collect()
        .await;
    info!("New stocks: {}", new_stocks.len());

    if !new_stocks.is_empty() {
        let si_fetcher = if !use_tv {
            Box::new(YFinance::new()) as Box<dyn StockInfoFetcher + Send + Sync>
        } else {
            let browser = browser::init_browser().await?;
            info!("Starting fetching of stock info...");

            let tv = StockInfoLoader::load(browser).await?;
            Box::new(tv) as Box<dyn StockInfoFetcher + Send + Sync>
        };

        let pb = ProgressBar::new(new_stocks.len() as u64);
        pb.set_style(ProgressStyle::default_bar().template(
            "{spinner:.green} [{elapsed_precise}] [{bar:60.cyan/blue}] {pos}/{len} {msg}",
        )?);
        let mut errors = HashMap::new();
        for ticker in new_stocks {
            pb.set_message(format!("[{ticker}]"));
            pb.inc(1);
            let result = si_fetcher.fetch(ticker).await;
            if !use_tv {
                time::sleep(Duration::from_millis(rand::random_range(100..300))).await;
            }
            let stock = match result {
                Ok(stock) => stock,
                Err(e) => {
                    errors.insert(ticker, e);
                    continue;
                }
            };
            store.add_stocks(&[stock], use_tv).await?;
        }
        if !errors.is_empty() {
            error!("Error while fetching info for {} tickers", errors.len());
            for (ticker, error) in &errors {
                warn!("\t{ticker} -> {error}");
            }
            anyhow::bail!(
                "Fetching stock info failed for '{}'",
                errors.keys().join(",")
            )
        }
        pb.finish_with_message("Done fetching all the tickers!");
        si_fetcher.done().await;
    }

    stream::iter(&stocks)
        .then(async |ticker| store.get_stock(ticker, use_tv).await)
        .try_filter_map(async |opt| Ok(opt))
        .try_collect()
        .await
}
