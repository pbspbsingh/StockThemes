use anyhow::Context;
use axum::{Router, middleware, routing};
use stock_themes::config::APP_CONFIG;
use stock_themes::{init_logger, no_cache, rrg_util};
use tokio::net::TcpListener;
use tracing::info;

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> anyhow::Result<()> {
    init_logger();

    let addr = format!("127.0.0.1:{}", APP_CONFIG.http_port);
    let listener = TcpListener::bind(&addr)
        .await
        .with_context(|| format!("Failed to bind at {addr}"))?;

    info!("Running http server at: {addr}");
    let app = Router::new()
        .route("/", routing::get(rrg_util::rrg_home))
        .route("/rrg.html", routing::get(rrg_util::rrg_home))
        .route("/api/rrg/{ticker}", routing::get(rrg_util::rrg_handler))
        .layer(middleware::from_fn(no_cache));
    axum::serve(listener, app).await?;

    Ok(())
}
