use std::collections::HashSet;
use std::sync::{Arc, LazyLock};
use std::time::Instant;

use anyhow::anyhow;
use chrono::Local;
use reqwest::Client;
use tokio::sync::{Mutex, mpsc};
use tracing::{debug, error, info, warn};

use crate::config::{TagSuggestionConfig, TagSuggestionProvider};
use crate::store::{CompanyProfile, Store};
use crate::yf::YFinance;

use super::SuggestionInput;
use super::parse::parse_suggested_tags;
use super::prompt::{build_prompt, suggestion_input};
use super::providers::{call_deepseek, call_ollama};
use super::providers::{model_for_config, provider_name, validate_config};

static YF: LazyLock<YFinance> = LazyLock::new(YFinance::new);

#[derive(Clone)]
pub struct TagSuggestionHandle {
    config: Arc<TagSuggestionConfig>,
    store: Arc<Store>,
    sender: mpsc::UnboundedSender<SuggestionJob>,
    queued_tickers: Arc<Mutex<HashSet<String>>>,
}

pub(super) struct SuggestionJob {
    pub(super) ticker: String,
}

struct TagSuggestionActor {
    config: Arc<TagSuggestionConfig>,
    store: Arc<Store>,
    client: Client,
    queued_tickers: Arc<Mutex<HashSet<String>>>,
}

impl TagSuggestionHandle {
    pub fn new(config: TagSuggestionConfig, store: Arc<Store>) -> anyhow::Result<Self> {
        validate_config(&config)?;
        let config = Arc::new(config);
        let (sender, mut receiver) = mpsc::unbounded_channel::<SuggestionJob>();
        let queued_tickers = Arc::new(Mutex::new(HashSet::new()));
        let actor = TagSuggestionActor {
            config: Arc::clone(&config),
            store: Arc::clone(&store),
            client: Client::new(),
            queued_tickers: Arc::clone(&queued_tickers),
        };

        tokio::spawn(async move {
            while let Some(job) = receiver.recv().await {
                actor.process(job).await;
            }
        });

        Ok(Self {
            config,
            store,
            sender,
            queued_tickers,
        })
    }

    pub fn provider_name(&self) -> &'static str {
        provider_name(&self.config.provider)
    }

    pub fn model(&self) -> anyhow::Result<String> {
        model_for_config(&self.config)
    }

    pub async fn enqueue(&self, ticker: String) -> anyhow::Result<bool> {
        let ticker = ticker.trim().to_uppercase();
        let provider = self.provider_name().to_string();
        let model = self.model()?;
        {
            let mut queued = self.queued_tickers.lock().await;
            if !queued.insert(ticker.clone()) {
                debug!("Skipping duplicate in-process tag suggestion for {ticker}");
                return Ok(false);
            }
            if let Err(err) = self
                .store
                .save_pending_tag_suggestion_request(&ticker, &provider, &model)
                .await
            {
                queued.remove(&ticker);
                return Err(err.into());
            }
            if let Err(err) = self.sender.send(SuggestionJob {
                ticker: ticker.clone(),
            }) {
                queued.remove(&ticker);
                return Err(anyhow!("Tag suggestion worker is not running: {err}"));
            }
        }
        Ok(true)
    }
}

impl TagSuggestionActor {
    async fn process(&self, job: SuggestionJob) {
        let started_at = Instant::now();
        let ticker = job.ticker.clone();
        info!("Starting tag suggestion for {ticker}");
        let input = match self.prepare_input(&job).await {
            Ok(Some(input)) => input,
            Ok(None) => {
                warn!("Discarded stale queued tag suggestion for {ticker}");
                self.finish_job(&ticker).await;
                return;
            }
            Err(err) => {
                warn!(
                    "Failed to prepare tag suggestion for {} after {:.2?}s: {err}",
                    ticker,
                    started_at.elapsed(),
                );
                match self
                    .store
                    .save_failed_tag_suggestion(&ticker, &err.to_string())
                    .await
                {
                    Ok(true) => {}
                    Ok(false) => {
                        warn!("Discarded stale tag suggestion preparation failure for {ticker}");
                    }
                    Err(save_err) => {
                        error!("Failed to save tag suggestion error for {ticker}: {save_err}");
                    }
                }
                self.finish_job(&ticker).await;
                return;
            }
        };

        let result = self.suggest(&input).await;
        match result {
            Ok(tags) => {
                let tag_count = tags.len();
                match self
                    .store
                    .save_ready_tag_suggestion(&input.ticker, &tags)
                    .await
                {
                    Ok(true) => {
                        info!(
                            "Completed tag suggestion for {} with {} tags in {:.2?}",
                            ticker,
                            tag_count,
                            started_at.elapsed(),
                        );
                    }
                    Ok(false) => {
                        warn!("Discarded stale tag suggestion result for {ticker}");
                    }
                    Err(err) => {
                        error!("Failed to save tag suggestion for {}: {err}", input.ticker);
                        let error_message = format!("Failed to save tag suggestion: {err}");
                        if let Err(save_err) = self
                            .store
                            .save_failed_tag_suggestion(&input.ticker, &error_message)
                            .await
                        {
                            error!(
                                "Failed to save tag suggestion error for {}: {save_err}",
                                input.ticker
                            );
                        }
                    }
                }
            }
            Err(err) => {
                warn!(
                    "Tag suggestion failed for {} after {:.2?}s: {err}",
                    input.ticker,
                    started_at.elapsed(),
                );
                match self
                    .store
                    .save_failed_tag_suggestion(&input.ticker, &err.to_string())
                    .await
                {
                    Ok(true) => {}
                    Ok(false) => {
                        warn!("Discarded stale tag suggestion failure for {ticker}");
                    }
                    Err(save_err) => {
                        error!(
                            "Failed to save tag suggestion error for {}: {save_err}",
                            input.ticker
                        );
                    }
                }
            }
        }
        self.finish_job(&ticker).await;
    }

    async fn prepare_input(&self, job: &SuggestionJob) -> anyhow::Result<Option<SuggestionInput>> {
        let input = self.build_suggestion_input(&job.ticker).await?;
        let updated = self
            .store
            .update_pending_tag_suggestion_profile(&job.ticker, &input)
            .await?;
        Ok(updated.then_some(input))
    }

    async fn build_suggestion_input(&self, ticker: &str) -> anyhow::Result<SuggestionInput> {
        let profile = match self.store.get_company_profile(ticker).await? {
            Some(profile) => profile,
            None => self.fetch_and_cache_company_profile(ticker).await?,
        };
        let allowed_tags = self
            .store
            .list_tags()
            .await?
            .into_iter()
            .map(|tag| tag.name)
            .collect::<Vec<_>>();
        Ok(suggestion_input(ticker.to_string(), profile, allowed_tags))
    }

    async fn fetch_and_cache_company_profile(&self, ticker: &str) -> anyhow::Result<CompanyProfile> {
        let yf_profile = YF.fetch_company_profile(ticker).await?;
        let profile = CompanyProfile {
            ticker: yf_profile.symbol.trim().to_uppercase(),
            summary: yf_profile
                .summary
                .map(|summary| summary.split_whitespace().collect::<Vec<_>>().join(" "))
                .filter(|summary| !summary.is_empty()),
            sector: yf_profile.sector,
            industry: yf_profile.industry,
            source: "Yahoo Finance".to_string(),
            fetched_at: Local::now(),
        };
        self.store.save_company_profile(&profile).await?;
        Ok(profile)
    }

    async fn finish_job(&self, ticker: &str) {
        self.queued_tickers.lock().await.remove(ticker);
    }

    async fn suggest(&self, input: &SuggestionInput) -> anyhow::Result<Vec<String>> {
        let prompt = build_prompt(input);
        let content = match self.config.provider {
            TagSuggestionProvider::Ollama => {
                let cfg = self
                    .config
                    .ollama
                    .as_ref()
                    .ok_or_else(|| anyhow!("Missing tag_suggestion.ollama config"))?;
                call_ollama(&self.client, cfg, &prompt).await?
            }
            TagSuggestionProvider::Deepseek => {
                let cfg = self
                    .config
                    .deepseek
                    .as_ref()
                    .ok_or_else(|| anyhow!("Missing tag_suggestion.deepseek config"))?;
                call_deepseek(&self.client, cfg, &prompt).await?
            }
        };
        parse_suggested_tags(&content, &input.allowed_tags)
    }
}
