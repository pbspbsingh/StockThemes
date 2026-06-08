use crate::Ticker;
use anyhow::Context;
use async_tungstenite::WebSocketStream;
use async_tungstenite::tokio::{ConnectStream, connect_async};
use async_tungstenite::tungstenite::{Message, client::IntoClientRequest, http::header::ORIGIN};
use chrono::{DateTime, Utc};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;
use tracing::{debug, info, warn};

const WEBSOCKET_URL: &str = "wss://data.tradingview.com/socket.io/websocket";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);
const IDLE_SOCKET_TIMEOUT: Duration = Duration::from_secs(60);
const ACTOR_CHANNEL_CAPACITY: usize = 64;

const FIELDS: [&str; 9] = [
    "earnings_per_share_fq_h",
    "earnings_per_share_forecast_fq_h",
    "revenue_fq_h",
    "revenue_forecast_fq_h",
    "earnings_release_date_fq_h",
    "fiscal_period_fq_h",
    "earnings_per_share_forecast_next_fq",
    "revenue_forecast_next_fq",
    "fundamental_currency_code",
];

type Socket = WebSocketStream<ConnectStream>;
type FetchResult = anyhow::Result<Vec<Fundamentals>>;

/// A clonable handle to a single actor-owned TradingView connection.
///
/// The actor opens the WebSocket on the first active fetch, shares it across
/// concurrent fetches, and retains the idle connection briefly for reuse.
#[derive(Clone)]
pub struct FundamentalsClient {
    commands: mpsc::Sender<Command>,
    timeout: Duration,
}

impl Default for FundamentalsClient {
    fn default() -> Self {
        Self::new()
    }
}

impl FundamentalsClient {
    pub fn new() -> Self {
        Self::with_timeout(DEFAULT_TIMEOUT)
    }

    pub fn with_timeout(timeout: Duration) -> Self {
        let (commands, receiver) = mpsc::channel(ACTOR_CHANNEL_CAPACITY);
        tokio::spawn(Actor::new(receiver).run());
        Self { commands, timeout }
    }

    /// Fetches fundamentals while preserving the order of `tickers`.
    ///
    /// Missing fundamentals are returned as `None` so callers can render the
    /// partial data TradingView has available for a symbol.
    pub async fn fetch(&self, tickers: &[Ticker]) -> FetchResult {
        if tickers.is_empty() {
            return Ok(Vec::new());
        }

        let (response, receiver) = oneshot::channel();
        self.commands
            .send(Command::Fetch {
                tickers: tickers.to_vec(),
                deadline: Instant::now() + self.timeout,
                response,
            })
            .await
            .context("TradingView fundamentals actor stopped")?;

        receiver
            .await
            .context("TradingView fundamentals actor dropped the request")?
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Fundamentals {
    pub ticker: Ticker,
    pub currency: Option<String>,
    pub quarters: Vec<QuarterFundamentals>,
    pub next_quarter: Forecast,
}

impl Fundamentals {
    pub fn has_usable_data(&self) -> bool {
        self.next_quarter.earnings_per_share.is_some()
            || self.next_quarter.revenue.is_some()
            || self.quarters.iter().any(|quarter| {
                quarter.earnings_per_share.is_some()
                    || quarter.earnings_per_share_estimate.is_some()
                    || quarter.revenue.is_some()
                    || quarter.revenue_estimate.is_some()
            })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QuarterFundamentals {
    pub fiscal_period: Option<String>,
    pub earnings_release_date: Option<DateTime<Utc>>,
    pub earnings_per_share: Option<f64>,
    pub earnings_per_share_estimate: Option<f64>,
    pub earnings_surprise: Option<f64>,
    pub earnings_surprise_percent: Option<f64>,
    pub revenue: Option<f64>,
    pub revenue_estimate: Option<f64>,
    pub revenue_surprise: Option<f64>,
    pub revenue_surprise_percent: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Forecast {
    pub earnings_per_share: Option<f64>,
    pub revenue: Option<f64>,
}

enum Command {
    Fetch {
        tickers: Vec<Ticker>,
        deadline: Instant,
        response: oneshot::Sender<FetchResult>,
    },
}

struct Request {
    tickers: Vec<Ticker>,
    qualified: Vec<String>,
    deadline: Instant,
    response: oneshot::Sender<FetchResult>,
}

#[derive(Default)]
struct SymbolState {
    fields: Map<String, Value>,
}

impl SymbolState {
    fn complete(&self) -> bool {
        FIELDS.iter().all(|field| self.fields.contains_key(*field))
    }

    fn merge(&mut self, fields: Map<String, Value>) {
        self.fields.extend(fields);
    }
}

struct Actor {
    commands: mpsc::Receiver<Command>,
    commands_closed: bool,
    socket: Option<Socket>,
    session: String,
    requests: HashMap<u64, Request>,
    symbols: HashMap<String, SymbolState>,
    subscribed: HashSet<String>,
    next_request_id: u64,
    idle_deadline: Option<Instant>,
}

impl Actor {
    fn new(commands: mpsc::Receiver<Command>) -> Self {
        Self {
            commands,
            commands_closed: false,
            socket: None,
            session: String::new(),
            requests: HashMap::new(),
            symbols: HashMap::new(),
            subscribed: HashSet::new(),
            next_request_id: 0,
            idle_deadline: None,
        }
    }

    async fn run(mut self) {
        loop {
            if self.socket.is_none() {
                if self.commands_closed {
                    return;
                }
                match self.commands.recv().await {
                    Some(command) => self.handle_command(command).await,
                    None => {
                        self.commands_closed = true;
                        continue;
                    }
                }
            } else {
                let request_deadline = self.requests.values().map(|request| request.deadline).min();
                let deadline = request_deadline
                    .into_iter()
                    .chain(self.idle_deadline)
                    .min()
                    .expect("connected socket has an active or idle deadline");

                tokio::select! {
                    command = self.commands.recv(), if !self.commands_closed => {
                        match command {
                            Some(command) => self.handle_command(command).await,
                            None => self.commands_closed = true,
                        }
                    }
                    message = async { self.socket.as_mut().expect("socket checked").next().await } => {
                        self.handle_socket_message(message).await;
                    }
                    _ = tokio::time::sleep_until(deadline) => {
                        self.finish_expired();
                    }
                }
            }

            self.finish_ready();
            self.prune_cancelled();
            self.drain_commands().await;
            self.release_unused_symbols().await;
            self.close_if_idle().await;
        }
    }

    async fn handle_command(&mut self, command: Command) {
        let Command::Fetch {
            tickers,
            deadline,
            response,
        } = command;

        if deadline <= Instant::now() {
            let _ = response.send(Err(anyhow::anyhow!(
                "TradingView fundamentals request expired before it was processed"
            )));
            return;
        }

        if self.socket.is_none()
            && let Err(error) = self.open_socket().await
        {
            warn!("Failed to open TradingView fundamentals WebSocket: {error}");
            let _ = response.send(Err(error));
            return;
        }
        self.idle_deadline = None;

        let qualified = tickers.iter().map(qualified_symbol).collect::<Vec<_>>();
        let new_symbols = qualified
            .iter()
            .filter(|symbol| self.subscribed.insert((*symbol).clone()))
            .cloned()
            .collect::<Vec<_>>();

        if let Err(error) = self.subscribe(&new_symbols).await {
            let _ = response.send(Err(
                error.context("Failed to subscribe to TradingView symbols")
            ));
            self.fail_all("TradingView fundamentals connection failed");
            self.drop_socket();
            return;
        }

        for symbol in &qualified {
            self.symbols.entry(symbol.clone()).or_default();
        }

        let request_id = self.next_request_id;
        self.next_request_id += 1;
        debug!(
            request_id,
            symbols = ?qualified,
            "Fetching TradingView fundamentals"
        );
        self.requests.insert(
            request_id,
            Request {
                tickers,
                qualified,
                deadline,
                response,
            },
        );
    }

    async fn open_socket(&mut self) -> anyhow::Result<()> {
        let mut request = WEBSOCKET_URL.into_client_request()?;
        request.headers_mut().insert(
            ORIGIN,
            "https://www.tradingview.com".parse().expect("valid origin"),
        );
        let (mut socket, _) = connect_async(request)
            .await
            .context("Failed to connect to TradingView fundamentals WebSocket")?;

        self.session = format!("qs_{:016x}", rand::random::<u64>());
        send_method(
            &mut socket,
            "set_auth_token",
            vec![json!("unauthorized_user_token")],
        )
        .await?;
        send_method(
            &mut socket,
            "quote_create_session",
            vec![json!(self.session)],
        )
        .await?;

        let mut params = vec![json!(self.session)];
        params.extend(FIELDS.iter().map(|field| json!(field)));
        send_method(&mut socket, "quote_set_fields", params).await?;
        self.socket = Some(socket);
        info!("Opened TradingView fundamentals WebSocket");
        Ok(())
    }

    async fn subscribe(&mut self, symbols: &[String]) -> anyhow::Result<()> {
        if symbols.is_empty() {
            return Ok(());
        }

        let mut params = vec![json!(self.session)];
        params.extend(symbols.iter().map(|symbol| json!(symbol)));
        send_method(
            self.socket.as_mut().context("WebSocket is not connected")?,
            "quote_add_symbols",
            params,
        )
        .await
    }

    async fn unsubscribe(&mut self, symbols: &[String]) -> anyhow::Result<()> {
        if symbols.is_empty() {
            return Ok(());
        }

        let mut params = vec![json!(self.session)];
        params.extend(symbols.iter().map(|symbol| json!(symbol)));
        send_method(
            self.socket.as_mut().context("WebSocket is not connected")?,
            "quote_remove_symbols",
            params,
        )
        .await
    }

    async fn handle_socket_message(
        &mut self,
        message: Option<Result<Message, async_tungstenite::tungstenite::Error>>,
    ) {
        let Some(message) = message else {
            warn!("TradingView fundamentals WebSocket closed unexpectedly");
            self.fail_all("TradingView fundamentals connection closed");
            self.drop_socket();
            return;
        };

        let message = match message {
            Ok(message) => message,
            Err(error) => {
                warn!("TradingView fundamentals WebSocket error: {error}");
                self.fail_all(&format!(
                    "TradingView fundamentals WebSocket error: {error}"
                ));
                self.drop_socket();
                return;
            }
        };

        match message {
            Message::Text(text) => {
                for payload in parse_frames(text.as_ref()) {
                    if payload.starts_with("~h~") {
                        if let Some(socket) = self.socket.as_mut() {
                            let _ = socket.send(Message::text(frame(payload))).await;
                        }
                        continue;
                    }
                    self.merge_qsd(payload);
                }
            }
            Message::Close(_) => {
                warn!("TradingView fundamentals WebSocket closed unexpectedly");
                self.fail_all("TradingView fundamentals connection closed");
                self.drop_socket();
            }
            _ => {}
        }
    }

    fn merge_qsd(&mut self, payload: &str) {
        let Ok(message) = serde_json::from_str::<ProtocolMessage>(payload) else {
            return;
        };
        if message.method != "qsd" {
            return;
        }

        let Some(update) = message.params.get(1).and_then(Value::as_object) else {
            return;
        };
        let Some(symbol) = update.get("n").and_then(Value::as_str) else {
            return;
        };
        let Some(fields) = update.get("v").and_then(Value::as_object) else {
            return;
        };

        self.symbols
            .entry(symbol.to_owned())
            .or_default()
            .merge(fields.clone());
    }

    fn finish_ready(&mut self) {
        let ready = self
            .requests
            .iter()
            .filter(|(_, request)| {
                request
                    .qualified
                    .iter()
                    .all(|symbol| self.symbols.get(symbol).is_some_and(SymbolState::complete))
            })
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();

        for id in ready {
            self.finish_request(id);
        }
    }

    fn finish_expired(&mut self) {
        let now = Instant::now();
        let expired = self
            .requests
            .iter()
            .filter(|(_, request)| request.deadline <= now)
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();

        for id in expired {
            if let Some(request) = self.requests.get(&id) {
                warn!(
                    request_id = id,
                    symbols = ?request.qualified,
                    "TradingView fundamentals fetch timed out"
                );
            }
            self.finish_request(id);
        }
    }

    fn finish_request(&mut self, id: u64) {
        let Some(request) = self.requests.remove(&id) else {
            return;
        };
        debug!(
            request_id = id,
            symbols = ?request.qualified,
            "Completed TradingView fundamentals fetch"
        );
        let results = request
            .tickers
            .into_iter()
            .zip(request.qualified)
            .map(|(ticker, symbol)| {
                fundamentals_from_fields(
                    ticker,
                    self.symbols
                        .get(&symbol)
                        .map(|state| &state.fields)
                        .unwrap_or(&Map::new()),
                )
            })
            .collect();
        let _ = request.response.send(results);
    }

    fn prune_cancelled(&mut self) {
        self.requests
            .retain(|_, request| !request.response.is_closed());
    }

    async fn drain_commands(&mut self) {
        // Admit a concurrent burst before reading updates, but keep socket
        // processing responsive if producers continuously fill the channel.
        for _ in 0..ACTOR_CHANNEL_CAPACITY {
            match self.commands.try_recv() {
                Ok(command) => self.handle_command(command).await,
                Err(mpsc::error::TryRecvError::Empty) => return,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    self.commands_closed = true;
                    return;
                }
            }
        }
    }

    async fn release_unused_symbols(&mut self) {
        if self.socket.is_none() {
            return;
        }

        let unused = self.unused_symbols();

        if let Err(error) = self.unsubscribe(&unused).await {
            warn!("Failed to unsubscribe from TradingView fundamentals symbols: {error}");
            self.fail_all(&format!(
                "Failed to unsubscribe from TradingView symbols: {error}"
            ));
            self.drop_socket();
            return;
        }
        for symbol in unused {
            self.subscribed.remove(&symbol);
            self.symbols.remove(&symbol);
        }
    }

    fn unused_symbols(&self) -> Vec<String> {
        let active = self
            .requests
            .values()
            .flat_map(|request| request.qualified.iter().cloned())
            .collect::<HashSet<_>>();
        self.subscribed.difference(&active).cloned().collect()
    }

    async fn close_if_idle(&mut self) {
        if !self.requests.is_empty() {
            self.idle_deadline = None;
            return;
        }
        if !self.idle_close_due(Instant::now()) {
            return;
        }
        if let Some(mut socket) = self.socket.take() {
            let _ = socket.close(None).await;
        }
        info!("Closed TradingView fundamentals WebSocket");
        self.drop_socket();
    }

    fn idle_close_due(&mut self, now: Instant) -> bool {
        if self.commands_closed {
            return true;
        }
        *self.idle_deadline.get_or_insert(now + IDLE_SOCKET_TIMEOUT) <= now
    }

    fn fail_all(&mut self, message: &str) {
        for (_, request) in self.requests.drain() {
            let _ = request
                .response
                .send(Err(anyhow::anyhow!(message.to_owned())));
        }
    }

    fn drop_socket(&mut self) {
        self.socket = None;
        self.session.clear();
        self.symbols.clear();
        self.subscribed.clear();
        self.idle_deadline = None;
    }
}

#[derive(Deserialize)]
struct ProtocolMessage {
    #[serde(rename = "m")]
    method: String,
    #[serde(rename = "p")]
    params: Vec<Value>,
}

async fn send_method(socket: &mut Socket, method: &str, params: Vec<Value>) -> anyhow::Result<()> {
    let payload = serde_json::to_string(&json!({ "m": method, "p": params }))?;
    socket.send(Message::text(frame(&payload))).await?;
    Ok(())
}

fn frame(payload: &str) -> String {
    format!("~m~{}~m~{payload}", payload.len())
}

fn parse_frames(mut message: &str) -> Vec<&str> {
    let mut frames = Vec::new();
    while let Some(rest) = message.strip_prefix("~m~") {
        let Some((length, payload)) = rest.split_once("~m~") else {
            break;
        };
        let Ok(length) = length.parse::<usize>() else {
            break;
        };
        if payload.len() < length || !payload.is_char_boundary(length) {
            break;
        }
        let (frame, remainder) = payload.split_at(length);
        frames.push(frame);
        message = remainder;
    }
    frames
}

fn qualified_symbol(ticker: &Ticker) -> String {
    format!(
        "{}:{}",
        ticker.exchange.trim().to_uppercase(),
        ticker.ticker.trim().to_uppercase()
    )
}

fn fundamentals_from_fields(
    ticker: Ticker,
    fields: &Map<String, Value>,
) -> anyhow::Result<Fundamentals> {
    let eps = number_array(fields.get("earnings_per_share_fq_h"));
    let eps_estimates = number_array(fields.get("earnings_per_share_forecast_fq_h"));
    let revenue = number_array(fields.get("revenue_fq_h"));
    let revenue_estimates = number_array(fields.get("revenue_forecast_fq_h"));
    let release_dates = timestamp_array(fields.get("earnings_release_date_fq_h"));
    let fiscal_periods = string_array(fields.get("fiscal_period_fq_h"));

    let quarter_count = [
        eps.len(),
        eps_estimates.len(),
        revenue.len(),
        revenue_estimates.len(),
        release_dates.len(),
        fiscal_periods.len(),
    ]
    .into_iter()
    .max()
    .unwrap_or_default();

    let mut latest_indices = (0..quarter_count).collect::<Vec<_>>();
    latest_indices.sort_unstable_by(|left, right| {
        option_at(&release_dates, *right)
            .cmp(&option_at(&release_dates, *left))
            .then_with(|| right.cmp(left))
    });
    latest_indices.truncate(8);

    let quarters = latest_indices
        .into_iter()
        .map(|index| {
            let earnings_per_share = option_at(&eps, index);
            let earnings_per_share_estimate = option_at(&eps_estimates, index);
            let revenue = option_at(&revenue, index);
            let revenue_estimate = option_at(&revenue_estimates, index);
            let (earnings_surprise, earnings_surprise_percent) =
                surprise(earnings_per_share, earnings_per_share_estimate);
            let (revenue_surprise, revenue_surprise_percent) = surprise(revenue, revenue_estimate);

            QuarterFundamentals {
                fiscal_period: option_at(&fiscal_periods, index),
                earnings_release_date: option_at(&release_dates, index),
                earnings_per_share,
                earnings_per_share_estimate,
                earnings_surprise,
                earnings_surprise_percent,
                revenue,
                revenue_estimate,
                revenue_surprise,
                revenue_surprise_percent,
            }
        })
        .collect();

    Ok(Fundamentals {
        ticker,
        currency: fields
            .get("fundamental_currency_code")
            .and_then(Value::as_str)
            .map(str::to_owned),
        quarters,
        next_quarter: Forecast {
            earnings_per_share: fields
                .get("earnings_per_share_forecast_next_fq")
                .and_then(Value::as_f64),
            revenue: fields
                .get("revenue_forecast_next_fq")
                .and_then(Value::as_f64),
        },
    })
}

fn number_array(value: Option<&Value>) -> Vec<Option<f64>> {
    value
        .and_then(Value::as_array)
        .map(|values| values.iter().map(Value::as_f64).collect())
        .unwrap_or_default()
}

fn string_array(value: Option<&Value>) -> Vec<Option<String>> {
    value
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .map(|value| value.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn timestamp_array(value: Option<&Value>) -> Vec<Option<DateTime<Utc>>> {
    value
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .map(|value| {
                    value
                        .as_i64()
                        .and_then(|timestamp| DateTime::from_timestamp(timestamp, 0))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn option_at<T: Clone>(values: &[Option<T>], index: usize) -> Option<T> {
    values.get(index).cloned().flatten()
}

fn surprise(actual: Option<f64>, estimate: Option<f64>) -> (Option<f64>, Option<f64>) {
    let absolute = actual
        .zip(estimate)
        .map(|(actual, estimate)| actual - estimate);
    let percent = actual.zip(estimate).and_then(|(actual, estimate)| {
        (estimate != 0.0).then_some((actual - estimate) / estimate.abs() * 100.0)
    });
    (absolute, percent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_multiple_frames_and_ignores_incomplete_tail() {
        let first = r#"{"m":"qsd","p":[]}"#;
        let second = "~h~123";
        let message = format!("{}{}~m~10~m~short", frame(first), frame(second));

        assert_eq!(parse_frames(&message), vec![first, second]);
    }

    #[test]
    fn merges_partial_qsd_updates() {
        let mut actor = Actor::new(mpsc::channel(1).1);
        actor.merge_qsd(
            r#"{"m":"qsd","p":["qs_test",{"n":"NASDAQ:AAPL","v":{"revenue_fq_h":[1,2]}}]}"#,
        );
        actor.merge_qsd(
            r#"{"m":"qsd","p":["qs_test",{"n":"NASDAQ:AAPL","v":{"fundamental_currency_code":"USD"}}]}"#,
        );

        let fields = &actor.symbols["NASDAQ:AAPL"].fields;
        assert_eq!(fields["revenue_fq_h"], json!([1, 2]));
        assert_eq!(fields["fundamental_currency_code"], json!("USD"));
    }

    #[test]
    fn idle_socket_closes_after_timeout_or_client_shutdown() {
        let mut actor = Actor::new(mpsc::channel(1).1);
        let now = Instant::now();

        assert!(!actor.idle_close_due(now));
        assert_eq!(actor.idle_deadline, Some(now + IDLE_SOCKET_TIMEOUT));
        assert!(!actor.idle_close_due(now + IDLE_SOCKET_TIMEOUT - Duration::from_millis(1)));
        assert!(actor.idle_close_due(now + IDLE_SOCKET_TIMEOUT));

        actor.commands_closed = true;
        actor.idle_deadline = None;
        assert!(actor.idle_close_due(now));
    }

    #[test]
    fn only_releases_symbols_unused_by_active_requests() {
        let mut actor = Actor::new(mpsc::channel(1).1);
        actor.subscribed = ["NASDAQ:AAPL", "NASDAQ:MSFT"]
            .map(str::to_owned)
            .into_iter()
            .collect();
        let (response, _) = oneshot::channel();
        actor.requests.insert(
            0,
            Request {
                tickers: vec![nasdaq_ticker("AAPL")],
                qualified: vec!["NASDAQ:AAPL".to_owned()],
                deadline: Instant::now() + DEFAULT_TIMEOUT,
                response,
            },
        );

        assert_eq!(actor.unused_symbols(), vec!["NASDAQ:MSFT"]);
    }

    #[test]
    fn returns_latest_eight_complete_quarters_in_descending_date_order() {
        let fields = complete_fields();
        let result = fundamentals_from_fields(test_ticker(), fields.as_object().unwrap()).unwrap();

        assert_eq!(result.currency.as_deref(), Some("USD"));
        assert_eq!(result.next_quarter.earnings_per_share, Some(2.2));
        assert_eq!(result.next_quarter.revenue, Some(120.0));
        assert_eq!(result.quarters.len(), 8);
        assert_eq!(result.quarters[0].fiscal_period.as_deref(), Some("Q9"));
        assert_eq!(result.quarters[7].fiscal_period.as_deref(), Some("Q2"));
        assert!((result.quarters[0].earnings_surprise.unwrap() - 0.5).abs() < f64::EPSILON);
        assert!((result.quarters[0].earnings_surprise_percent.unwrap() - 25.0).abs() < 1e-10);
        assert!((result.quarters[0].revenue_surprise_percent.unwrap() - 10.0).abs() < 1e-10);
    }

    #[test]
    fn preserves_mismatched_historical_arrays() {
        let mut fields = complete_fields();
        fields["revenue_fq_h"].as_array_mut().unwrap().pop();

        let result = fundamentals_from_fields(test_ticker(), fields.as_object().unwrap()).unwrap();

        assert_eq!(result.quarters.len(), 8);
        assert_eq!(result.quarters[0].revenue, None);
        assert_eq!(result.quarters[0].earnings_per_share, Some(2.5));
    }

    #[test]
    fn preserves_fewer_than_eight_historical_quarters() {
        let mut fields = complete_fields();
        for field in [
            "earnings_per_share_fq_h",
            "earnings_per_share_forecast_fq_h",
            "revenue_fq_h",
            "revenue_forecast_fq_h",
        ] {
            fields[field].as_array_mut().unwrap().truncate(7);
        }

        let result = fundamentals_from_fields(test_ticker(), fields.as_object().unwrap()).unwrap();

        assert_eq!(result.quarters.len(), 8);
        assert_eq!(result.quarters[0].earnings_per_share, None);
        assert_eq!(result.quarters[2].earnings_per_share, Some(1.6));
    }

    #[test]
    fn preserves_incomplete_historical_quarter() {
        let mut fields = complete_fields();
        fields["revenue_forecast_fq_h"][3] = Value::Null;

        let result = fundamentals_from_fields(test_ticker(), fields.as_object().unwrap()).unwrap();

        let quarter = result
            .quarters
            .iter()
            .find(|quarter| quarter.fiscal_period.as_deref() == Some("Q4"))
            .unwrap();
        assert_eq!(quarter.revenue_estimate, None);
        assert_eq!(quarter.revenue_surprise, None);
        assert_eq!(quarter.revenue_surprise_percent, None);
    }

    #[test]
    fn preserves_missing_next_quarter_forecast() {
        let mut fields = complete_fields();
        fields["revenue_forecast_next_fq"] = Value::Null;

        let result = fundamentals_from_fields(test_ticker(), fields.as_object().unwrap()).unwrap();

        assert_eq!(result.next_quarter.revenue, None);
        assert_eq!(result.next_quarter.earnings_per_share, Some(2.2));
    }

    #[test]
    fn preserves_entirely_missing_fields() {
        let mut fields = complete_fields();
        let fields = fields.as_object_mut().unwrap();
        fields.remove("earnings_per_share_forecast_fq_h");
        fields.remove("earnings_per_share_forecast_next_fq");
        fields.remove("fundamental_currency_code");

        let result = fundamentals_from_fields(test_ticker(), fields).unwrap();

        assert_eq!(result.currency, None);
        assert_eq!(result.next_quarter.earnings_per_share, None);
        assert!(
            result
                .quarters
                .iter()
                .all(|quarter| quarter.earnings_per_share_estimate.is_none())
        );
    }

    #[test]
    fn empty_or_metadata_only_fundamentals_are_not_usable() {
        let empty = fundamentals_from_fields(test_ticker(), &Map::new()).unwrap();
        assert!(!empty.has_usable_data());

        let metadata_only = json!({
            "earnings_release_date_fq_h": [1780000000],
            "fiscal_period_fq_h": ["Q1"],
            "fundamental_currency_code": "USD"
        });
        let metadata_only =
            fundamentals_from_fields(test_ticker(), metadata_only.as_object().unwrap()).unwrap();
        assert!(!metadata_only.has_usable_data());
    }

    #[test]
    fn forecast_or_partial_quarter_fundamentals_are_usable() {
        let forecast_only = json!({ "revenue_forecast_next_fq": 120.0 });
        let forecast_only =
            fundamentals_from_fields(test_ticker(), forecast_only.as_object().unwrap()).unwrap();
        assert!(forecast_only.has_usable_data());

        let partial_quarter = json!({ "earnings_per_share_fq_h": [1.0] });
        let partial_quarter =
            fundamentals_from_fields(test_ticker(), partial_quarter.as_object().unwrap()).unwrap();
        assert!(partial_quarter.has_usable_data());
    }

    #[tokio::test]
    #[ignore]
    async fn live_concurrent_fetch() {
        let client = FundamentalsClient::new();
        let first = ["AAPL", "MSFT", "NVDA", "AMZN", "GOOGL"].map(nasdaq_ticker);
        let second = ["TSLA", "PLTR", "RKLB", "SOUN", "WMT"].map(nasdaq_ticker);

        let (first, second) = tokio::join!(client.fetch(&first), client.fetch(&second));
        let results = first
            .unwrap()
            .into_iter()
            .chain(second.unwrap())
            .collect::<Vec<_>>();
        assert_eq!(results.len(), 10);
        for result in results {
            assert_eq!(result.quarters.len(), 8, "{}", result.ticker.ticker);
            assert!(
                result
                    .next_quarter
                    .earnings_per_share
                    .is_some_and(f64::is_finite),
                "{}",
                result.ticker.ticker
            );
            assert!(
                result.next_quarter.revenue.is_some_and(f64::is_finite),
                "{}",
                result.ticker.ticker
            );
            assert!(
                result.quarters.windows(2).all(|quarters| {
                    quarters[0].earnings_release_date >= quarters[1].earnings_release_date
                }),
                "{}",
                result.ticker.ticker
            );
        }
    }

    #[tokio::test]
    #[ignore]
    async fn live_partial_fetch() {
        let result = FundamentalsClient::new()
            .fetch(&[nasdaq_ticker("NBIS")])
            .await
            .unwrap()
            .pop()
            .unwrap();

        assert!(!result.quarters.is_empty());
    }

    #[tokio::test]
    #[ignore]
    async fn live_sequential_fetch_reuses_idle_session() {
        let client = FundamentalsClient::new();

        let first = client.fetch(&[nasdaq_ticker("AAPL")]).await.unwrap();
        let second = client.fetch(&[nasdaq_ticker("MSFT")]).await.unwrap();

        assert_eq!(first[0].ticker.ticker, "AAPL");
        assert_eq!(second[0].ticker.ticker, "MSFT");
    }

    fn test_ticker() -> Ticker {
        nasdaq_ticker("AAPL")
    }

    fn nasdaq_ticker(ticker: &str) -> Ticker {
        Ticker {
            exchange: "NASDAQ".to_owned(),
            ticker: ticker.to_owned(),
        }
    }

    fn complete_fields() -> Value {
        json!({
            "earnings_per_share_fq_h": [1.0, 1.1, 1.2, 1.3, 1.4, 1.5, 1.6, 1.7, 2.5],
            "earnings_per_share_forecast_fq_h": [0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 1.1, 1.2, 2.0],
            "revenue_fq_h": [110.0, 110.0, 110.0, 110.0, 110.0, 110.0, 110.0, 110.0, 110.0],
            "revenue_forecast_fq_h": [100.0, 100.0, 100.0, 100.0, 100.0, 100.0, 100.0, 100.0, 100.0],
            "earnings_release_date_fq_h": [
                1700000000, 1710000000, 1720000000, 1730000000, 1740000000,
                1750000000, 1760000000, 1770000000, 1780000000
            ],
            "fiscal_period_fq_h": ["Q1", "Q2", "Q3", "Q4", "Q5", "Q6", "Q7", "Q8", "Q9"],
            "earnings_per_share_forecast_next_fq": 2.2,
            "revenue_forecast_next_fq": 120.0,
            "fundamental_currency_code": "USD"
        })
    }
}
