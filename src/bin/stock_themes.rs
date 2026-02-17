use anyhow::Context;
use axum::{Router, response::Html, routing};
use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use itertools::Itertools;
use log::{error, info, warn};
use std::time::Duration;
use std::{collections::HashMap, path::PathBuf};
use stock_themes::{
    Stock, StockInfoFetcher, browser, config::APP_CONFIG, store::Store, template::create_html,
    tv::stock_info_loader::StockInfoLoader, util,
};

use stock_themes::yf::YFinance;
use tokio::{fs, net::TcpListener, time};

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
    env_logger::Builder::new()
        .parse_filters(&APP_CONFIG.log_config)
        .init();

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

    let summary = util::summarize(stocks);
    let html = create_html(&summary)?;
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
    let use_yf = !APP_CONFIG.use_tv_for_stock_info;
    let mut store = Store::load_store(if use_yf {
        "stocks_info_yf.json"
    } else {
        "stocks_info_tv.json"
    })?;
    let new_stocks = stocks
        .iter()
        .filter(|&s| store.get(s).is_none())
        .collect_vec();
    info!("New stocks: {}", new_stocks.len());

    if !new_stocks.is_empty() {
        let si_fetcher = if use_yf {
            Box::new(YFinance::new().await?) as Box<dyn StockInfoFetcher + Send + Sync>
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
            if use_yf {
                time::sleep(Duration::from_millis(rand::random_range(100..300))).await;
            }
            let stock = match result {
                Ok(stock) => stock,
                Err(e) => {
                    errors.insert(ticker, e);
                    continue;
                }
            };
            store.add(stock)?;
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

    Ok(stocks
        .into_iter()
        .filter_map(|ticker| store.get(&ticker))
        .cloned()
        .collect())
}

async fn start_http_server(html: String) -> anyhow::Result<()> {
    let addr = "127.0.0.1:8000";
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("Failed to bind at {addr}: e"))?;
    info!("Running http server at: {addr}");
    let app = Router::new().route("/", routing::get(async || Html(html)));
    axum::serve(listener, app).await?;
    Ok(())
}
