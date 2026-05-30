use anyhow::Context;
use clap::Parser;

use tracing::info;

use std::path::{Path, PathBuf};

use stock_themes::store::Store;

use stock_themes::tv::tv_manager::TvManager;

use stock_themes::{init_logger};

#[derive(Parser, Debug)]
#[command(name = "top_stocks")]
#[command(about = "Fetches the top performing stocks using trading view stocks screen")]
struct TopStocksArgs {
    /// Trading view screen url
    #[arg(required = true)]
    pub tv_screen_url: String,

    /// Time frames from which top stocks to pick from
    #[arg(short = 't', long, default_value = "1M,3M,6M,1Y")]
    pub time_frames: String,

    /// Numbers of top stocks to pick
    #[arg(short = 'c', long, default_value_t = 100)]
    pub top_count: usize,

    /// Fetch the stocks which are gainers or losers
    #[arg(short = 'l', long, default_value_t = false)]
    pub fetch_losers: bool,

    /// Output CSV File
    #[arg(short = 'o', long, default_value = "watchlist.csv")]
    pub output_file: PathBuf,
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> anyhow::Result<()> {
    init_logger();

    let args = TopStocksArgs::parse();
    info!("Using args: {args:#?}");

    let mut tv_manager = TvManager::new(Store::load_store().await?);
    let stocks = tv_manager
        .fetch_top_stocks(
            &args.tv_screen_url,
            args.top_count,
            !args.fetch_losers,
            time_frames(&args.time_frames),
        )
        .await?;
    info!("Total {} unique stocks fetched", stocks.len());

    save_csv(&args.output_file, &args.tv_screen_url, &stocks).await
}

async fn save_csv(file: &Path, source: &str, stocks: &[String]) -> anyhow::Result<()> {
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

pub fn time_frames(input: &str) -> impl Iterator<Item = String> {
    input.split(',').map(str::trim).map(str::to_uppercase)
}
