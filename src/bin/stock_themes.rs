use std::collections::HashMap;

use anyhow::Context;
use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use itertools::Itertools;
use log::{debug, error, info, warn};
use stock_themes::{
    Stock, StockThemesArgs, browser, config::APP_CONFIG, read_stocks, store::Store, summarize,
    template::create_html, tv::stock_info_loader::StockInfoLoader,
};
use tiny_http::{Header, Response, Server, StatusCode};

const HTML_FILE: &str = "stocks_themes.html";

fn main() -> anyhow::Result<()> {
    env_logger::Builder::new()
        .parse_filters(&APP_CONFIG.log_config)
        .init();

    let args = StockThemesArgs::parse();
    info!(
        "Reading {} csv files, skipping {} lines",
        args.files.len(),
        args.skip_lines,
    );

    let stocks = read_stocks(&args)?;
    info!("Total unique stocks: {}", stocks.len());

    let stocks = fetch_stock_info(stocks)?;
    info!("Fetched stock info of {} stocks", stocks.len());

    let summary = summarize(stocks);
    let html = create_html(&summary)?;
    std::fs::write(HTML_FILE, &html).with_context(|| format!("Failed to write {HTML_FILE}"))?;
    info!(
        "Done! Wrote html to {:?}",
        std::fs::canonicalize(HTML_FILE)?
    );

    start_http_server(html)
}

fn fetch_stock_info(stocks: Vec<String>) -> anyhow::Result<Vec<Stock>> {
    let mut store = Store::load_store()?;
    let new_stocks = stocks
        .iter()
        .filter(|&s| store.get(s).is_none())
        .collect_vec();
    info!("New stocks: {}", new_stocks.len());

    if !new_stocks.is_empty() {
        let browser = browser::init_browser()?;

        debug!("Browser info: {:?}", browser.get_version()?);
        info!("Starting fetching of stock info...");

        let tv = StockInfoLoader::load(&browser)?;
        let pb = ProgressBar::new(new_stocks.len() as u64);
        pb.set_style(ProgressStyle::default_bar().template(
            "{spinner:.green} [{elapsed_precise}] [{bar:60.cyan/blue}] {pos}/{len} {msg}",
        )?);
        let mut errors = HashMap::new();
        for ticker in new_stocks {
            pb.set_message(format!("[{ticker}]"));
            pb.inc(1);
            let stock = match tv.fetch_stock_info(ticker) {
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
    }

    Ok(stocks
        .into_iter()
        .filter_map(|ticker| store.get(&ticker))
        .cloned()
        .collect())
}

fn start_http_server(html: String) -> anyhow::Result<()> {
    let addr = "127.0.0.1:8000";
    let server = Server::http(addr)
        .map_err(|e| anyhow::anyhow!("Failed to start http server at {addr}: {e}"))?;
    info!("Http server: http://{addr}/");

    let header = Header::from_bytes(&b"Content-Type"[..], &b"text/html"[..])
        .map_err(|_| anyhow::anyhow!("Header parsing error"))?;
    for request in server.incoming_requests() {
        let response = if request.url().ends_with("favicon.ico") {
            Response::from_string("").with_status_code(StatusCode(404))
        } else {
            Response::from_string(&html).with_header(header.clone())
        };

        if let Err(e) = request.respond(response) {
            warn!("Failed to send html to the client: {e}");
        }
    }
    Ok(())
}
