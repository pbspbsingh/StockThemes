use std::{
    collections::HashSet,
    fs::File,
    io::BufWriter,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::Context;
use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use log::info;
use stock_themes::{browser, config::APP_CONFIG, tv::top_stocks_fetcher::TopStocksFetcher};

#[derive(Parser, Debug)]
#[command(name = "top_stocks")]
#[command(about = "Fetches the top performing stocks using trading view stocks screen")]
struct TopStocksArgs {
    /// Trading view screen url
    #[arg(required = true)]
    pub tv_screen_url: String,

    /// Numbers of top stocks to pick
    #[arg(short = 'c', long, default_value_t = 30)]
    pub top_count: usize,

    /// Output CSV File
    #[arg(short = 'o', long, default_value = "top_performers.csv")]
    pub output_file: PathBuf,
}

const SORT_BY_KEYS: &[&str] = &["1W", "1M", "3M", "6M"];

fn main() -> anyhow::Result<()> {
    env_logger::Builder::new()
        .parse_filters(&APP_CONFIG.log_config)
        .init();

    let args = TopStocksArgs::parse();
    info!("Screen url: {}", args.tv_screen_url);

    let browser = browser::init_browser()?;

    let mut stocks = HashSet::new();

    let pb = ProgressBar::new((SORT_BY_KEYS.len() * args.top_count) as u64);
    pb.set_style(
        ProgressStyle::default_bar().template(
            "{spinner:.green} [{elapsed_precise}] [{bar:50.cyan/blue}] {pos}/{len} {msg}",
        )?,
    );

    let fetcher = TopStocksFetcher::load(&browser, &args.tv_screen_url, args.top_count, &pb)?;
    for &sort_by in SORT_BY_KEYS {
        stocks.extend(fetcher.fetch_stocks(sort_by)?);
    }
    pb.finish_with_message("Done fetching top stocks");
    info!("Total {} unique stocks fetched", stocks.len());
    save_csv(&args.output_file, &args.tv_screen_url, stocks)
}

fn save_csv(file: &Path, source: &str, stocks: HashSet<String>) -> anyhow::Result<()> {
    let f = File::create(file).with_context(|| format!("Failed to write to {file:?}"))?;
    let mut writer = BufWriter::new(f);
    writeln!(writer, "======= Top Performing Stocks ======")?;
    writeln!(writer, "Source: {source}")?;
    writeln!(writer, "Count: {}", stocks.len())?;
    writeln!(writer)?;
    for stock in stocks {
        writeln!(writer, "{stock}")?;
    }
    info!("Saved the output to {:?}\n", file.canonicalize()?);
    Ok(())
}
