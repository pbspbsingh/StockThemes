use clap::Parser;
use itertools::Itertools;
use log::info;
use std::collections::HashMap;
use stock_themes::store::Store;
use stock_themes::summary::Summary;
use stock_themes::tv::top_industry_groups::TopIndustryGroups;
use stock_themes::tv::top_stocks_fetcher::TopStocksFetcher;
use stock_themes::{browser, start_http_server};
use stock_themes::{init_logger, time_frames};

#[derive(Parser, Debug)]
#[command(name = "top_industries")]
#[command(about = "Fetches the top performing stocks from top performing industry groups")]
struct TopInsArgs {
    /// Input files to process
    #[arg(
        required = false,
        default_value = "https://www.tradingview.com/screener/"
    )]
    pub base_screen_url: String,

    /// Time frames to order the industry groups by
    #[arg(short = 'i', long, default_value = "1M,3M")]
    pub industry_group_strength: String,

    /// Number of top industry groups to pick
    #[arg(short = 'n', long, default_value_t = 50)]
    pub industry_group_count: usize,

    /// Numbers of top stocks to pick
    #[arg(short = 'c', long, default_value_t = 100)]
    pub top_count: usize,

    /// Time frames from which top stocks to pick from
    #[arg(short = 't', long, default_value = "1M,3M,6M,1Y")]
    pub time_frames: String,
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> anyhow::Result<()> {
    init_logger();

    let args = TopInsArgs::parse();
    info!("Using args: {args:#?}");

    let browser = browser::init_browser().await?;
    let mut industries = Vec::new();
    let tig = TopIndustryGroups::new(&browser).await?;
    for tf in time_frames(&args.industry_group_strength) {
        let next = tig
            .fetch_top_industry_groups(&tf, args.industry_group_count)
            .await?;
        industries = merge(industries, next);
    }
    tig.close().await;
    info!("Fetched {} industry groups", industries.len());
    if industries.is_empty() {
        anyhow::bail!("No industry groups found");
    }

    let mut stocks = HashMap::new();
    let fetcher = TopStocksFetcher::load_screen_with_industries(
        &browser,
        &args.base_screen_url,
        args.top_count,
        &industries,
    )
    .await?;
    for sort_by in time_frames(&args.time_frames) {
        for stock in fetcher.fetch_stocks(&sort_by).await? {
            stocks.insert(stock.ticker.clone(), stock);
        }
    }
    fetcher.close().await;
    info!("Total {} unique stocks fetched", stocks.len());

    Store::load_store(true)
        .await?
        .add_stocks(&stocks.values().cloned().collect_vec())
        .await?;

    let summary = Summary::summarize(stocks.values().cloned().collect());
    let html = summary.render(industries);
    start_http_server(html).await
}

fn merge(primary: Vec<String>, secondary: Vec<String>) -> Vec<String> {
    let mut order = HashMap::new();
    let mut idx = 0;
    for key in primary.into_iter().chain(secondary) {
        order.entry(key).or_insert_with(|| {
            idx += 1;
            idx
        });
    }
    order.keys().cloned().sorted_by_key(|s| order[s]).collect()
}
