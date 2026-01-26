use anyhow::Context;

use chromiumoxide::browser::HeadlessMode;
use chromiumoxide::{Browser, BrowserConfig, Handler};
use log::{debug, info, warn};

use sysinfo::{RefreshKind, System};

use futures::StreamExt;

use crate::config::APP_CONFIG;

const REMOTE_DEBUG_ARG: &str = "--remote-debugging-port";

pub async fn init_browser() -> anyhow::Result<Browser> {
    let (browser, mut handler) = match try_connect_existing_session().await {
        Ok(browser) => browser,
        Err(e) => {
            warn!("Error connecting to existing session: '{e}', starting new session...");
            start_new_session().await?
        }
    };

    tokio::spawn(async move {
        while let Some(h) = handler.next().await {
            if let Err(e) = h {
                warn!("Chrome handler error: {e}");
                break;
            }
        }
    });

    info!(
        "Browser: {} => {}",
        browser.version().await?.product,
        browser.websocket_address()
    );
    Ok(browser)
}

async fn try_connect_existing_session() -> anyhow::Result<(Browser, Handler)> {
    let sys_info = System::new_with_specifics(RefreshKind::everything());
    let chrome_process = sys_info
        .processes()
        .values()
        .filter(|&p| {
            p.cmd()
                .first()
                .map(|process| process.to_string_lossy().to_lowercase().contains("chrome"))
                .unwrap_or_default()
        })
        .find(|p| {
            p.cmd()
                .iter()
                .any(|arg| arg.to_string_lossy().starts_with(REMOTE_DEBUG_ARG))
        })
        .ok_or_else(|| anyhow::anyhow!("No Chrome process with remote debug port found"))?;
    debug!(
        "Found a chrome process with debug enabled: {:?}",
        chrome_process.cmd()
    );
    let debug_str = chrome_process
        .cmd()
        .iter()
        .map(|arg| arg.to_string_lossy())
        .find(|arg| arg.starts_with(REMOTE_DEBUG_ARG))
        .ok_or_else(|| anyhow::anyhow!("Oops didn't find debug argument"))?;
    let debug_port = debug_str
        .split('=')
        .nth(1)
        .map(|s| s.trim())
        .map(|s| {
            s.parse::<u16>()
                .map_err(|e| anyhow::anyhow!("Failed to parse {s} into u16: {e}"))
        })
        .ok_or_else(|| anyhow::anyhow!("Didn't find debug port in {debug_str}"))??;
    let debug_url = format!("http://localhost:{debug_port}");
    info!("Found debug port: {debug_port}, url: {debug_url}");

    Browser::connect(&debug_url)
        .await
        .with_context(|| format!("Failed to connect to existing session at {debug_url}"))
}

async fn start_new_session() -> anyhow::Result<(Browser, Handler)> {
    let config = BrowserConfig::builder()
        .chrome_executable(&APP_CONFIG.chrome_path)
        .user_data_dir(&APP_CONFIG.user_data_dir)
        .headless_mode(HeadlessMode::False)
        .enable_cache()
        .window_size(1920, 1080)
        .viewport(None)
        .args(&APP_CONFIG.chrome_args)
        .build()
        .map_err(|e| anyhow::anyhow!("BrowserConfig error: {e}"))?;

    Browser::launch(config)
        .await
        .with_context(|| format!("Failed to start a new browser session"))
}
