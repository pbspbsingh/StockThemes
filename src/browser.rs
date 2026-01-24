use anyhow::Context;

use headless_chrome::Browser;
use log::{debug, info, warn};
use serde::Deserialize;
use std::io::{BufRead, BufReader};
use std::net::{Ipv4Addr, TcpListener};
use std::ops::Deref;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;
use sysinfo::{Pid, RefreshKind, System};
use ureq::http::header;

use crate::config::APP_CONFIG;

const REMOTE_DEBUG_ARG: &str = "--remote-debugging-port";

pub struct KillableBrowser {
    pid: u32,
    browser: Browser,
}

impl Deref for KillableBrowser {
    type Target = Browser;

    fn deref(&self) -> &Self::Target {
        &self.browser
    }
}

impl Drop for KillableBrowser {
    fn drop(&mut self) {
        if !APP_CONFIG.kill_chrome_on_exit {
            info!("No need to kill the Browser");
            return;
        }

        let sys = System::new_with_specifics(RefreshKind::everything());
        let Some(process) = sys.process(Pid::from_u32(self.pid)) else {
            warn!("Didn't find any process with pid: {}", self.pid);
            return;
        };
        if process.kill() {
            info!(
                "Killed process: {:?} (PID: {})",
                process.name(),
                process.pid(),
            );
        } else {
            warn!(
                "Failed to kill the process: {:?} (PID: {})",
                process.name(),
                process.pid(),
            );
        }
    }
}

impl KillableBrowser {
    fn new(pid: u32, browser: Browser) -> Self {
        Self { pid, browser }
    }
}

pub fn init_browser() -> anyhow::Result<KillableBrowser> {
    try_connect_existing_session().or_else(|e| {
        warn!("Couldn't resume previous session '{e}', try start new session");
        start_new_session()
    })
}

fn try_connect_existing_session() -> anyhow::Result<KillableBrowser> {
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
    info!("Found debug port: {debug_port}");

    let ws_url = fetch_debug_info(debug_port)?;
    info!("Successfully fetched debug ws url: {ws_url}");

    Ok(KillableBrowser::new(
        chrome_process.pid().as_u32(),
        connect(&ws_url)?,
    ))
}

fn start_new_session() -> anyhow::Result<KillableBrowser> {
    fn start_chrome_process() -> anyhow::Result<(u32, String)> {
        let port = quick_port()?;
        debug!("Starting new chrome session with remote debugging port at: {port}");
        let mut process = Command::new(&APP_CONFIG.chrome_path)
            .arg(format!("{REMOTE_DEBUG_ARG}={port}"))
            .args(&APP_CONFIG.chrome_args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()?;
        debug!("Started a chrome instance with pid: {}", process.id());
        if let Some(output) = process.stderr.take() {
            let mut reader = BufReader::new(output);
            let mut buff = String::new();
            loop {
                reader.read_line(&mut buff)?;
                if buff.starts_with("DevTools listening on") {
                    let ws_url = buff.trim_start_matches("DevTools listening on").trim();
                    return Ok((process.id(), ws_url.to_owned()));
                }

                buff.clear();
                thread::sleep(Duration::from_millis(200));
            }
        }

        warn!("Couldn't get the stdout of child process");
        process.kill()?;
        anyhow::bail!("Failed to get stdout of child process")
    }

    let (id, ws_url) = start_chrome_process()?;
    Ok(KillableBrowser::new(id, connect(&ws_url)?))
}

fn connect(ws_url: impl Into<String>) -> anyhow::Result<Browser> {
    let url = ws_url.into();
    Browser::connect_with_timeout(url.clone(), Duration::from_secs(10))
        .with_context(|| format!("Failed to connect to {url:?}"))
}

fn quick_port() -> anyhow::Result<u16> {
    Ok(TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?
        .local_addr()?
        .port())
}

fn fetch_debug_info(debug_port: u16) -> anyhow::Result<String> {
    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct DebugInfo {
        web_socket_debugger_url: String,
    }

    let mut response = ureq::get(format!("http://localhost:{debug_port}/json/version"))
        .header(header::ACCEPT, "application/json")
        .call()?;
    let info = response.body_mut().read_json::<DebugInfo>()?;
    Ok(info.web_socket_debugger_url)
}
