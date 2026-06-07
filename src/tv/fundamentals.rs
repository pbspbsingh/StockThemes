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

const WEBSOCKET_URL: &str = "wss://data.tradingview.com/socket.io/websocket";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
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
/// concurrent fetches, and closes it when no fetches remain.
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
    /// Returns an error unless every symbol has eight complete historical
    /// quarters and both next-quarter EPS and revenue forecasts.
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
    pub currency: String,
    pub quarters: Vec<QuarterFundamentals>,
    pub next_quarter: Forecast,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QuarterFundamentals {
    pub fiscal_period: String,
    pub earnings_release_date: DateTime<Utc>,
    pub earnings_per_share: f64,
    pub earnings_per_share_estimate: f64,
    pub earnings_surprise: f64,
    pub earnings_surprise_percent: Option<f64>,
    pub revenue: f64,
    pub revenue_estimate: f64,
    pub revenue_surprise: f64,
    pub revenue_surprise_percent: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Forecast {
    pub earnings_per_share: f64,
    pub revenue: f64,
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
                let deadline = self
                    .requests
                    .values()
                    .map(|request| request.deadline)
                    .min()
                    .unwrap_or_else(Instant::now);

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
            let _ = response.send(Err(error));
            return;
        }

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

    async fn handle_socket_message(
        &mut self,
        message: Option<Result<Message, async_tungstenite::tungstenite::Error>>,
    ) {
        let Some(message) = message else {
            self.fail_all("TradingView fundamentals connection closed");
            self.drop_socket();
            return;
        };

        let message = match message {
            Ok(message) => message,
            Err(error) => {
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
            self.finish_request(id);
        }
    }

    fn finish_request(&mut self, id: u64) {
        let Some(request) = self.requests.remove(&id) else {
            return;
        };
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

    async fn close_if_idle(&mut self) {
        if !self.requests.is_empty() {
            return;
        }
        if let Some(mut socket) = self.socket.take() {
            let _ = socket.close(None).await;
        }
        self.drop_socket();
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
    let symbol = qualified_symbol(&ticker);
    let invalid_fields = FIELDS
        .iter()
        .filter(|field| field_missing_or_invalid(fields, field))
        .copied()
        .collect::<Vec<_>>();
    if !invalid_fields.is_empty() {
        anyhow::bail!(
            "{symbol} is missing required TradingView fundamentals fields: {}",
            invalid_fields.join(", ")
        );
    }

    let eps = number_array(fields.get("earnings_per_share_fq_h"));
    let eps_estimates = number_array(fields.get("earnings_per_share_forecast_fq_h"));
    let revenue = number_array(fields.get("revenue_fq_h"));
    let revenue_estimates = number_array(fields.get("revenue_forecast_fq_h"));
    let release_dates = timestamp_array(fields.get("earnings_release_date_fq_h"));
    let fiscal_periods = string_array(fields.get("fiscal_period_fq_h"));

    let historical_lengths = [
        eps.len(),
        eps_estimates.len(),
        revenue.len(),
        revenue_estimates.len(),
    ];
    if historical_lengths
        .iter()
        .any(|length| *length != historical_lengths[0])
    {
        anyhow::bail!(
            "{symbol} returned mismatched historical array lengths: EPS={}, EPS estimates={}, revenue={}, revenue estimates={}",
            eps.len(),
            eps_estimates.len(),
            revenue.len(),
            revenue_estimates.len()
        );
    }
    let quarter_count = historical_lengths[0];
    if quarter_count < 8 {
        anyhow::bail!(
            "{symbol} returned only {quarter_count} complete historical quarters; expected at least 8"
        );
    }
    if release_dates.len() < quarter_count || fiscal_periods.len() < quarter_count {
        anyhow::bail!(
            "{symbol} returned insufficient quarter metadata: history={quarter_count}, release dates={}, fiscal periods={}",
            release_dates.len(),
            fiscal_periods.len()
        );
    }

    let mut latest_indices = (0..quarter_count)
        .map(|index| {
            required_at(&release_dates, index, &symbol, "earnings release date")
                .map(|release_date| (index, release_date))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    latest_indices.sort_unstable_by(|(_, left), (_, right)| right.cmp(left));
    latest_indices.truncate(8);

    let quarters = latest_indices
        .into_iter()
        .map(|(index, earnings_release_date)| {
            let fiscal_period = required_at(&fiscal_periods, index, &symbol, "fiscal period")?;
            let earnings_per_share = required_at(&eps, index, &symbol, "EPS")?;
            let earnings_per_share_estimate =
                required_at(&eps_estimates, index, &symbol, "EPS estimate")?;
            let revenue = required_at(&revenue, index, &symbol, "revenue")?;
            let revenue_estimate =
                required_at(&revenue_estimates, index, &symbol, "revenue estimate")?;
            let (earnings_surprise, earnings_surprise_percent) =
                surprise(earnings_per_share, earnings_per_share_estimate);
            let (revenue_surprise, revenue_surprise_percent) = surprise(revenue, revenue_estimate);

            Ok(QuarterFundamentals {
                fiscal_period,
                earnings_release_date,
                earnings_per_share,
                earnings_per_share_estimate,
                earnings_surprise,
                earnings_surprise_percent,
                revenue,
                revenue_estimate,
                revenue_surprise,
                revenue_surprise_percent,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    Ok(Fundamentals {
        ticker,
        currency: fields
            .get("fundamental_currency_code")
            .and_then(Value::as_str)
            .context("validated currency is missing")?
            .to_owned(),
        quarters,
        next_quarter: Forecast {
            earnings_per_share: fields
                .get("earnings_per_share_forecast_next_fq")
                .and_then(Value::as_f64)
                .context("validated next-quarter EPS forecast is missing")?,
            revenue: fields
                .get("revenue_forecast_next_fq")
                .and_then(Value::as_f64)
                .context("validated next-quarter revenue forecast is missing")?,
        },
    })
}

fn field_missing_or_invalid(fields: &Map<String, Value>, field: &str) -> bool {
    let Some(value) = fields.get(field) else {
        return true;
    };
    if value.is_null() {
        return true;
    }

    match field {
        "earnings_per_share_fq_h"
        | "earnings_per_share_forecast_fq_h"
        | "revenue_fq_h"
        | "revenue_forecast_fq_h"
        | "earnings_release_date_fq_h"
        | "fiscal_period_fq_h" => !value.is_array(),
        "earnings_per_share_forecast_next_fq" | "revenue_forecast_next_fq" => !value.is_number(),
        "fundamental_currency_code" => !value.is_string(),
        _ => false,
    }
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

fn required_at<T: Clone>(
    values: &[Option<T>],
    index: usize,
    symbol: &str,
    field: &str,
) -> anyhow::Result<T> {
    option_at(values, index)
        .with_context(|| format!("{symbol} has no {field} for historical quarter index {index}"))
}

fn surprise(actual: f64, estimate: f64) -> (f64, Option<f64>) {
    let absolute = actual - estimate;
    let percent = (estimate != 0.0).then_some(absolute / estimate.abs() * 100.0);
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
    fn returns_latest_eight_complete_quarters_in_descending_date_order() {
        let fields = complete_fields();
        let result = fundamentals_from_fields(test_ticker(), fields.as_object().unwrap()).unwrap();

        assert_eq!(result.currency, "USD");
        assert_eq!(result.next_quarter.earnings_per_share, 2.2);
        assert_eq!(result.next_quarter.revenue, 120.0);
        assert_eq!(result.quarters.len(), 8);
        assert_eq!(result.quarters[0].fiscal_period, "Q9");
        assert_eq!(result.quarters[7].fiscal_period, "Q2");
        assert!((result.quarters[0].earnings_surprise - 0.5).abs() < f64::EPSILON);
        assert!((result.quarters[0].earnings_surprise_percent.unwrap() - 25.0).abs() < 1e-10);
        assert!((result.quarters[0].revenue_surprise_percent.unwrap() - 10.0).abs() < 1e-10);
    }

    #[test]
    fn rejects_mismatched_historical_arrays() {
        let mut fields = complete_fields();
        fields["revenue_fq_h"].as_array_mut().unwrap().pop();

        let error =
            fundamentals_from_fields(test_ticker(), fields.as_object().unwrap()).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("mismatched historical array lengths")
        );
    }

    #[test]
    fn rejects_fewer_than_eight_historical_quarters() {
        let mut fields = complete_fields();
        for field in [
            "earnings_per_share_fq_h",
            "earnings_per_share_forecast_fq_h",
            "revenue_fq_h",
            "revenue_forecast_fq_h",
        ] {
            fields[field].as_array_mut().unwrap().truncate(7);
        }

        let error =
            fundamentals_from_fields(test_ticker(), fields.as_object().unwrap()).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("only 7 complete historical quarters")
        );
    }

    #[test]
    fn rejects_incomplete_historical_quarter() {
        let mut fields = complete_fields();
        fields["revenue_forecast_fq_h"][3] = Value::Null;

        let error =
            fundamentals_from_fields(test_ticker(), fields.as_object().unwrap()).unwrap_err();

        assert!(error.to_string().contains("no revenue estimate"));
    }

    #[test]
    fn rejects_missing_next_quarter_forecast() {
        let mut fields = complete_fields();
        fields["revenue_forecast_next_fq"] = Value::Null;

        let error =
            fundamentals_from_fields(test_ticker(), fields.as_object().unwrap()).unwrap_err();

        assert!(error.to_string().contains("revenue_forecast_next_fq"));
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
                result.next_quarter.earnings_per_share.is_finite(),
                "{}",
                result.ticker.ticker
            );
            assert!(
                result.next_quarter.revenue.is_finite(),
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
