use anyhow::Context;
use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs;
use tracing::info;

use stock_themes::config::APP_CONFIG;
use stock_themes::init_logger;
use stock_themes::store::Store;
use stock_themes::trades::build_views;
use stock_themes::trades::parser::{parse_tos_csv, trades_to_csv};
use stock_themes::trades::routes::start_server;
use stock_themes::yf::YFinance;

#[derive(Parser, Debug)]
#[command(name = "trade_analyzer")]
#[command(about = "Analyze ThinkorSwim trade export CSV and launch web UI")]
struct Args {
    /// Input ThinkorSwim CSV export file
    pub input: PathBuf,

    /// Output analysis CSV file
    #[arg(short, long, default_value = "trade_analysis.csv")]
    pub output: PathBuf,
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> anyhow::Result<()> {
    init_logger();
    let args = Args::parse();

    // Parse CSV
    let content = fs::read_to_string(&args.input)
        .await
        .with_context(|| format!("Cannot read '{}'", args.input.display()))?;
    let trades = parse_tos_csv(&content);
    info!(
        "Parsed {} trades from {}",
        trades.len(),
        args.input.display()
    );

    // Write analysis CSV
    let csv = trades_to_csv(&trades);
    fs::write(&args.output, &csv)
        .await
        .with_context(|| format!("Cannot write '{}'", args.output.display()))?;
    info!(
        "Wrote {} trades ({} open) to {}",
        trades.len(),
        trades.iter().filter(|t| t.is_open()).count(),
        args.output.display()
    );

    // Build views
    let benchmark = APP_CONFIG.base_ticker.to_uppercase();
    let views = build_views(&trades, &APP_CONFIG.trade_analysis);

    let store = Store::load_store().await?;
    let yf = Arc::new(YFinance::new());

    // Start web server
    start_server(store, yf, views.trade_views, &benchmark).await
}
