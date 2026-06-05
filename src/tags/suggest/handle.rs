use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use anyhow::anyhow;
use reqwest::Client;
use tokio::sync::{Mutex, mpsc};
use tracing::{debug, error, info, warn};

use crate::config::{TagSuggestionConfig, TagSuggestionProvider};
use crate::store::Store;

use super::SuggestionInput;
use super::parse::parse_suggested_tags;
use super::prompt::build_prompt;
use super::providers::{call_deepseek, call_ollama};
use super::providers::{model_for_config, provider_name, validate_config};

#[derive(Clone)]
pub struct TagSuggestionHandle {
    config: Arc<TagSuggestionConfig>,
    store: Arc<Store>,
    sender: mpsc::UnboundedSender<SuggestionJob>,
    queued_tickers: Arc<Mutex<HashSet<String>>>,
}

pub(super) struct SuggestionJob {
    pub(super) input: SuggestionInput,
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

    pub async fn enqueue(
        &self,
        input: SuggestionInput,
        provider: &str,
        model: &str,
    ) -> anyhow::Result<bool> {
        let ticker = input.ticker.clone();
        {
            let mut queued = self.queued_tickers.lock().await;
            if !queued.insert(ticker.clone()) {
                debug!("Skipping duplicate in-process tag suggestion for {ticker}");
                return Ok(false);
            }
            if let Err(err) = self
                .store
                .save_pending_tag_suggestion(&input, provider, model)
                .await
            {
                queued.remove(&ticker);
                return Err(err.into());
            }
            if let Err(err) = self.sender.send(SuggestionJob { input }) {
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
        let ticker = job.input.ticker.clone();
        info!("Starting tag suggestion for {ticker}");
        let result = self.suggest(&job.input).await;
        match result {
            Ok(tags) => {
                let tag_count = tags.len();
                match self
                    .store
                    .save_ready_tag_suggestion(&job.input.ticker, &job.input.prompt_hash, &tags)
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
                        error!(
                            "Failed to save tag suggestion for {}: {err}",
                            job.input.ticker
                        );
                        let error_message = format!("Failed to save tag suggestion: {err}");
                        if let Err(save_err) = self
                            .store
                            .save_failed_tag_suggestion(
                                &job.input.ticker,
                                &job.input.prompt_hash,
                                &error_message,
                            )
                            .await
                        {
                            error!(
                                "Failed to save tag suggestion error for {}: {save_err}",
                                job.input.ticker
                            );
                        }
                    }
                }
            }
            Err(err) => {
                warn!(
                    "Tag suggestion failed for {} after {:.2?}s: {err}",
                    job.input.ticker,
                    started_at.elapsed(),
                );
                match self
                    .store
                    .save_failed_tag_suggestion(
                        &job.input.ticker,
                        &job.input.prompt_hash,
                        &err.to_string(),
                    )
                    .await
                {
                    Ok(true) => {}
                    Ok(false) => {
                        warn!("Discarded stale tag suggestion failure for {ticker}");
                    }
                    Err(save_err) => {
                        error!(
                            "Failed to save tag suggestion error for {}: {save_err}",
                            job.input.ticker
                        );
                    }
                }
            }
        }
        self.queued_tickers.lock().await.remove(&ticker);
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
