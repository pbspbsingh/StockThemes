use chrome_driver::Page;
use chrome_driver::chromiumoxide::page::ScreenshotParams;
use chrono::Local;
use std::path::Path;
use tracing::warn;

const SCREENSHOTS_DIR: &str = "screenshots";

pub(super) trait SnapOnErr<T> {
    /// On `Err`, logs the failure and captures a screenshot labeled `label`,
    /// then returns the result unchanged.
    async fn snap_on_err(self, page: &Page, label: &str) -> anyhow::Result<T>;
}

impl<T> SnapOnErr<T> for anyhow::Result<T> {
    async fn snap_on_err(self, page: &Page, label: &str) -> anyhow::Result<T> {
        if let Err(e) = &self {
            warn!("{label} failed: {e}; capturing screenshot");
            save_screenshot(page, label).await;
        }
        self
    }
}

async fn save_screenshot(page: &Page, label: &str) {
    let dir = Path::new(SCREENSHOTS_DIR);
    if let Err(e) = tokio::fs::create_dir_all(dir).await {
        warn!("Could not create {}: {e}", dir.display());
        return;
    }
    let safe_label: String = label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let filename = format!(
        "{}_{}.png",
        Local::now().format("%Y%m%d_%H%M%S"),
        safe_label
    );
    let path = dir.join(filename);
    match page
        .save_screenshot(ScreenshotParams::default(), &path)
        .await
    {
        Ok(_) => warn!("Saved error screenshot: {}", path.display()),
        Err(e) => warn!("Failed to save screenshot {}: {e}", path.display()),
    }
}
