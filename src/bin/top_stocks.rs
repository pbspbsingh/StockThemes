use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use anyhow::Context;
use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use itertools::Itertools;
use log::info;
use stock_themes::{browser, config::APP_CONFIG, tv::top_stocks_fetcher::TopStocksFetcher};

#[derive(Parser, Debug)]
#[command(name = "top_stocks")]
#[command(about = "Fetches the top performing stocks using trading view stocks screen")]
struct TopStocksArgs {
    /// Trading view screen url
    #[arg(required = true)]
    pub tv_screen_url: String,

    /// Time frames from which top stocks to pick from
    #[arg(short = 't', long, default_value = "1W,1M,3M,6M")]
    pub time_frames: String,

    /// Numbers of top stocks to pick
    #[arg(short = 'c', long, default_value_t = 50)]
    pub top_count: usize,

    /// Output CSV File
    #[arg(short = 'o', long, default_value = "top_performers.csv")]
    pub output_file: PathBuf,
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> anyhow::Result<()> {
    env_logger::Builder::new()
        .parse_filters(&APP_CONFIG.log_config)
        .init();

    let args = TopStocksArgs::parse();
    info!("Screen url: {}", args.tv_screen_url);

    let browser = browser::init_browser().await?;

    let tf = args.time_frames.split(',').map(str::trim).collect_vec();
    let mut stocks = HashSet::new();

    let pb = ProgressBar::new((tf.len() * args.top_count) as u64);
    pb.set_style(
        ProgressStyle::default_bar().template(
            "{spinner:.green} [{elapsed_precise}] [{bar:50.cyan/blue}] {pos}/{len} {msg}",
        )?,
    );

    let fetcher =
        TopStocksFetcher::load(&browser, &args.tv_screen_url, args.top_count, &pb).await?;
    for sort_by in tf {
        stocks.extend(fetcher.fetch_stocks(sort_by).await?);
    }
    pb.finish_with_message("Done fetching top stocks");
    info!("Total {} unique stocks fetched", stocks.len());
    save_csv(&args.output_file, &args.tv_screen_url, stocks).await
}

async fn save_csv(file: &Path, source: &str, stocks: HashSet<String>) -> anyhow::Result<()> {
    use std::fmt::Write;

    let mut content = String::new();
    writeln!(content, "======= Top Performing Stocks ======")?;
    writeln!(content, "Source: {source}")?;
    writeln!(content, "Count: {}", stocks.len())?;
    writeln!(content)?;
    for stock in stocks {
        writeln!(content, "{stock}")?;
    }
    tokio::fs::write(file, content)
        .await
        .with_context(|| format!("Error writing output to {file:?}"))?;
    info!("Saved the output to {:?}\n", file.canonicalize()?);
    Ok(())
}
