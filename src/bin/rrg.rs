use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use axum::{Extension, Router, middleware, routing};
use clap::Parser;
use stock_themes::config::APP_CONFIG;
use stock_themes::rrg_util::RrgMode;
use stock_themes::{etf_map, init_logger, no_cache, rrg_util, util};
use tokio::net::TcpListener;
use tracing::info;

#[derive(Parser, Debug)]
#[command(name = "rrg")]
#[command(
    about = "Relative Rotation Graph server. With CSV files, plots those tickers' rotation instead of sector/industry ETFs."
)]
pub struct RrgArgs {
    /// Optional input CSV files of tickers to plot (same format as stock_themes).
    /// When omitted, the sector/industry ETF rotation is shown.
    pub files: Vec<PathBuf>,

    /// Number of header lines to skip in each CSV
    #[arg(short = 'n', long, default_value_t = 4)]
    pub skip_lines: usize,

    /// Comma separated list of stocks to skip
    #[arg(short = 's', long, default_value = "")]
    pub skip_stocks: String,
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> anyhow::Result<()> {
    init_logger();

    let args = RrgArgs::parse();

    let mode = if args.files.is_empty() {
        RrgMode::Sectors(etf_map::tv_mapping())
    } else {
        let tickers = util::read_stocks(&args.files, args.skip_lines, &args.skip_stocks).await?;
        if tickers.is_empty() {
            anyhow::bail!(
                "CSV files {:?} produced no tickers after filtering (check --skip-lines / --skip-stocks)",
                args.files
            );
        }
        RrgMode::Tickers(tickers)
    };
    match &mode {
        RrgMode::Sectors(_) => info!("No ticker files — serving sector/industry rotation"),
        RrgMode::Tickers(t) => info!("Serving ticker rotation for {} tickers", t.len()),
    }
    let mode = Arc::new(mode);

    let addr = format!("127.0.0.1:{}", APP_CONFIG.http_port);
    let listener = TcpListener::bind(&addr)
        .await
        .with_context(|| format!("Failed to bind at {addr}"))?;

    info!("Running http server at: {addr}");
    let app = Router::new()
        .route("/", routing::get(rrg_util::rrg_home))
        .route("/rrg.html", routing::get(rrg_util::rrg_home))
        .route("/api/rrg/{ticker}", routing::get(rrg_util::rrg_handler))
        .layer(Extension(mode))
        .layer(middleware::from_fn(no_cache));
    axum::serve(listener, app).await?;

    Ok(())
}
