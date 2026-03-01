use clap::Parser;
use itertools::Itertools;
use log::info;

use stock_themes::config::APP_CONFIG;
use stock_themes::store::Store;
use stock_themes::summary::Summary;

use stock_themes::tv::tv_manager::TvManager;
use stock_themes::util::compute_rs;
use stock_themes::yf::YFinance;
use stock_themes::{Performance, fetch_stock_perf, start_http_server};
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

    let yf = YFinance::new();
    let store = Store::load_store().await?;

    let base_perf = fetch_stock_perf(&store, &yf, &APP_CONFIG.base_ticker).await?;
    info!("Fetched baseline: {base_perf}");

    let mut tv_manager = TvManager::new(store.clone());

    let sectors = tv_manager.fetch_sectors().await?;
    info!("Fetched {} sectors", sectors.len());

    let industries = tv_manager.fetch_industries().await?;
    info!("Fetched {} industry groups", industries.len());
    let industries: Vec<Performance> = industries
        .into_iter()
        .map(|p| {
            let rs = compute_rs(&p, &base_perf);
            (p, rs)
        })
        .sorted_by(|a, b| b.1.partial_cmp(&a.1).unwrap())
        .map(|(p, _)| p)
        .collect();
    if industries.is_empty() {
        anyhow::bail!("No industry groups found");
    }

    let (stocks, stock_perfs) = tv_manager
        .fetch_top_stocks_with_industries_filter(
            &args.base_screen_url,
            args.top_count,
            &industries
                .iter()
                .map(|p| p.ticker.clone())
                .take(args.industry_group_count)
                .collect_vec(),
            time_frames(&args.time_frames),
        )
        .await?;
    drop(tv_manager);
    info!("Total {} unique stocks fetched", stocks.len());

    let summary = Summary::summarize(stocks);
    let html = summary.render(sectors, industries, stock_perfs, &base_perf);

    start_http_server(html).await
}
