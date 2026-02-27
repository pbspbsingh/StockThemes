use clap::Parser;
use itertools::Itertools;
use log::info;
use std::collections::HashMap;
use std::sync::Arc;
use stock_themes::config::APP_CONFIG;
use stock_themes::store::Store;
use stock_themes::summary::Summary;
use stock_themes::tv::Closeable;
use stock_themes::tv::top_industry_groups::TopIndustryGroups;
use stock_themes::tv::top_stocks_fetcher::TopStocksFetcher;
use stock_themes::util::compute_rs;
use stock_themes::yf::YFinance;
use stock_themes::{Performance, TickerType, browser, fetch_stock_perf, start_http_server};
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

    let browser = browser::init_browser().await?;
    let page = browser.new_page("about:blank").await?;

    let base_perf = fetch_stock_perf(store.clone(), &yf, &APP_CONFIG.base_ticker).await?;
    info!("Fetched baseline: {base_perf:?}");

    let tig = TopIndustryGroups::new(&page).await?;
    let sectors = fetch_sectors(store.clone(), &tig).await?;
    info!("Fetched {} sectors", sectors.len());

    let industries = fetch_industries(store.clone(), &tig).await?;
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

    let fetcher = TopStocksFetcher::load_screen_with_industries(
        &page,
        &args.base_screen_url,
        args.top_count,
        &industries
            .iter()
            .map(|p| p.ticker.clone())
            .take(args.industry_group_count)
            .collect_vec(),
    )
    .await?;

    let mut stocks_map = HashMap::new();
    let mut perf_map = HashMap::new();

    for sort_by in time_frames(&args.time_frames) {
        let (stocks, perfs) = fetcher.fetch_stocks(&sort_by).await?;
        store.add_stocks(&stocks, true).await?;
        store.save_performances(&perfs).await?;

        for stock in stocks {
            stocks_map.insert(stock.ticker.clone(), stock);
        }
        for perf in perfs {
            perf_map.insert(perf.ticker.clone(), perf);
        }
    }
    info!("Total {} unique stocks fetched", stocks_map.len());
    page.close_me().await;

    let summary = Summary::summarize(stocks_map.into_values());
    let html = summary.render(
        create_rs_map(sectors.into_iter(), &base_perf),
        create_rs_map(industries.into_iter(), &base_perf),
        create_rs_map(perf_map.into_values(), &base_perf),
    );

    start_http_server(html).await
}

async fn fetch_sectors<'a>(
    store: Arc<Store>,
    tig: &TopIndustryGroups<'a>,
) -> anyhow::Result<Vec<Performance>> {
    let mut sectors = store.get_performances_by_type(TickerType::Sector).await?;
    if sectors.is_empty() {
        sectors = tig.fetch_sectors().await?;
        store.save_performances(&sectors).await?;
    }
    Ok(sectors)
}

async fn fetch_industries<'a>(
    store: Arc<Store>,
    tig: &TopIndustryGroups<'a>,
) -> anyhow::Result<Vec<Performance>> {
    let mut industries = store.get_performances_by_type(TickerType::Industry).await?;
    if industries.is_empty() {
        industries = tig.fetch_industries().await?;
        store.save_performances(&industries).await?;
    }
    Ok(industries)
}

fn create_rs_map(
    perfs: impl Iterator<Item = Performance>,
    base: &Performance,
) -> HashMap<String, f64> {
    perfs
        .map(|p| {
            let rs = (compute_rs(&p, base) * 100.0).round() / 100.0;
            (p.ticker, rs)
        })
        .collect()
}
