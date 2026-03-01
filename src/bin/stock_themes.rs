use anyhow::Context;

use clap::Parser;

use indicatif::{ProgressBar, ProgressStyle};

use log::info;

use std::path::PathBuf;
use stock_themes::{
    Performance, Stock, config::APP_CONFIG, fetch_stock_perf, init_logger, start_http_server,
    store::Store, util,
};

use stock_themes::summary::Summary;
use stock_themes::tv::tv_manager::TvManager;
use stock_themes::yf::YFinance;
use tokio::fs;

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
    info!("args: {args:#?}");

    let yf = YFinance::new();
    let store = Store::load_store().await?;

    let base_perf = fetch_stock_perf(&store, &yf, &APP_CONFIG.base_ticker).await?;
    info!("Fetched baseline: {base_perf}");

    let mut tv_manager = TvManager::new(store.clone());

    let sectors = tv_manager.fetch_sectors().await?;
    info!("Fetched {} sectors", sectors.len());

    let industries = tv_manager.fetch_industries().await?;
    info!("Fetched {} industry groups", industries.len());

    let tickers = util::read_stocks(&args.files, args.skip_lines, &args.skip_stocks).await?;
    info!("Total unique stocks: {}", tickers.len());

    let (stocks, stock_perfs) = fetch_stock_info(&mut tv_manager, &yf, tickers).await?;
    drop(tv_manager);

    let summary = Summary::summarize(stocks);
    let html = summary.render(sectors, industries, stock_perfs, &base_perf);

    fs::write(HTML_FILE, &html)
        .await
        .with_context(|| format!("Failed to write {HTML_FILE}"))?;
    info!(
        "Done! Wrote html to {:?}",
        fs::canonicalize(HTML_FILE).await?
    );

    start_http_server(html).await
}

async fn fetch_stock_info(
    tv_manager: &mut TvManager,
    yf: &YFinance,
    tickers: Vec<String>,
) -> anyhow::Result<(Vec<Stock>, Vec<Performance>)> {
    let pb = ProgressBar::new(tickers.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar().template(
            "{spinner:.green} [{elapsed_precise}] [{bar:60.cyan/blue}] {pos}/{len} {msg}",
        )?,
    );

    let store = Store::load_store().await?;
    let mut stocks = Vec::with_capacity(tickers.len());
    let mut perfs = Vec::with_capacity(tickers.len());

    for ticker in tickers {
        pb.set_message(format!("[{ticker}] info..."));
        pb.tick();
        stocks.push(tv_manager.fetch_stock_info(&ticker).await?);

        pb.set_message(format!("[{ticker}] performance..."));
        pb.inc(1);
        perfs.push(fetch_stock_perf(&store, yf, &ticker).await?);
    }

    pb.finish_with_message(format!("Finished processing {} tickers", stocks.len()));

    Ok((stocks, perfs))
}
