use std::time::Duration;

use anyhow::{anyhow, bail};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::config::{
    DeepseekTagSuggestionConfig, OllamaTagSuggestionConfig, TagSuggestionConfig,
    TagSuggestionProvider,
};

const PROVIDER_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

pub(super) fn provider_name(provider: &TagSuggestionProvider) -> &'static str {
    match provider {
        TagSuggestionProvider::Ollama => "ollama",
        TagSuggestionProvider::Deepseek => "deepseek",
    }
}

pub(super) fn model_for_config(config: &TagSuggestionConfig) -> anyhow::Result<String> {
    match config.provider {
        TagSuggestionProvider::Ollama => config
            .ollama
            .as_ref()
            .map(|cfg| cfg.model.clone())
            .ok_or_else(|| anyhow!("Missing tag_suggestion.ollama config")),
        TagSuggestionProvider::Deepseek => config
            .deepseek
            .as_ref()
            .map(|cfg| cfg.model.clone())
            .ok_or_else(|| anyhow!("Missing tag_suggestion.deepseek config")),
    }
}

pub(super) fn validate_config(config: &TagSuggestionConfig) -> anyhow::Result<()> {
    match config.provider {
        TagSuggestionProvider::Ollama => {
            let cfg = config
                .ollama
                .as_ref()
                .ok_or_else(|| anyhow!("Missing tag_suggestion.ollama config"))?;
            if cfg.base_url.trim().is_empty() || cfg.model.trim().is_empty() {
                bail!("Ollama tag suggestion config requires base_url and model");
            }
        }
        TagSuggestionProvider::Deepseek => {
            let cfg = config
                .deepseek
                .as_ref()
                .ok_or_else(|| anyhow!("Missing tag_suggestion.deepseek config"))?;
            if cfg.base_url.trim().is_empty()
                || cfg.model.trim().is_empty()
                || cfg.api_key.trim().is_empty()
            {
                bail!("DeepSeek tag suggestion config requires base_url, model, and api_key");
            }
        }
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct OllamaChatRequest<'a> {
    model: &'a str,
    messages: Vec<OllamaMessage<'a>>,
    stream: bool,
}

#[derive(Debug, Serialize)]
struct OllamaMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Debug, Deserialize)]
struct OllamaChatResponse {
    message: OllamaResponseMessage,
}

#[derive(Debug, Deserialize)]
struct OllamaResponseMessage {
    content: String,
}

pub(super) async fn call_ollama(
    client: &Client,
    config: &OllamaTagSuggestionConfig,
    prompt: &str,
) -> anyhow::Result<String> {
    let url = format!("{}/api/chat", config.base_url.trim_end_matches('/'));
    let response = client
        .post(url)
        .timeout(PROVIDER_REQUEST_TIMEOUT)
        .json(&OllamaChatRequest {
            model: &config.model,
            messages: vec![OllamaMessage {
                role: "user",
                content: prompt,
            }],
            stream: false,
        })
        .send()
        .await?
        .error_for_status()?
        .json::<OllamaChatResponse>()
        .await?;
    Ok(response.message.content)
}

#[derive(Debug, Serialize)]
struct DeepseekChatRequest<'a> {
    model: &'a str,
    messages: Vec<DeepseekMessage<'a>>,
    temperature: f32,
    response_format: DeepseekResponseFormat<'a>,
}

#[derive(Debug, Serialize)]
struct DeepseekMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Debug, Serialize)]
struct DeepseekResponseFormat<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
}

#[derive(Debug, Deserialize)]
struct DeepseekChatResponse {
    choices: Vec<DeepseekChoice>,
}

#[derive(Debug, Deserialize)]
struct DeepseekChoice {
    message: DeepseekResponseMessage,
}

#[derive(Debug, Deserialize)]
struct DeepseekResponseMessage {
    content: String,
}

pub(super) async fn call_deepseek(
    client: &Client,
    config: &DeepseekTagSuggestionConfig,
    prompt: &str,
) -> anyhow::Result<String> {
    let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));
    let response = client
        .post(url)
        .timeout(PROVIDER_REQUEST_TIMEOUT)
        .bearer_auth(config.api_key.trim())
        .json(&DeepseekChatRequest {
            model: &config.model,
            messages: vec![DeepseekMessage {
                role: "user",
                content: prompt,
            }],
            temperature: 0.0,
            response_format: DeepseekResponseFormat {
                kind: "json_object",
            },
        })
        .send()
        .await?
        .error_for_status()?
        .json::<DeepseekChatResponse>()
        .await?;
    response
        .choices
        .into_iter()
        .next()
        .map(|choice| choice.message.content)
        .ok_or_else(|| anyhow!("DeepSeek returned no choices"))
}
