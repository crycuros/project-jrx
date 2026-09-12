//! AI evaluation layer — interfaces with local Ollama.

pub mod history;

use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;

use trading_core::{AiVerdict, Config, Proposal};

const GENERATE_ENDPOINT: &str = "/api/generate";
const TAGS_ENDPOINT: &str = "/api/tags";

#[derive(Debug, Deserialize)]
struct GenerateResponse {
    response: String,
    #[serde(default)]
    #[allow(dead_code)]
    done: bool,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct TagsResponse {
    models: Vec<TagModel>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct TagModel {
    name: String,
}

/// Client for Ollama's local model server.
pub struct AiClient {
    client: Client,
    base_url: String,
    model: String,
    memory: history::HistoricalDataCopyMemory,
}

impl AiClient {
    pub fn new(cfg: &Config) -> Self {
        let memory = history::HistoricalDataCopyMemory::load(cfg);
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(120))
                .build()
                .expect("build reqwest"),
            base_url: cfg.ollama_base_url.trim_end_matches('/').to_string(),
            model: cfg.ollama_model.clone(),
            memory,
        }
    }

    /// Check whether Ollama is up and the model exists.
    pub async fn ping(&self) -> bool {
        let url = format!("{}{}", self.base_url, TAGS_ENDPOINT);
        match self.client.get(&url).timeout(Duration::from_secs(3)).send().await {
            Ok(resp) => {
                if !resp.status().is_success() {
                    return false;
                }
                match resp.json::<TagsResponse>().await {
                    Ok(tags) => tags.models.iter().any(|m| m.name.starts_with(&self.model)),
                    Err(_) => true, // unreachable details; model check relaxed
                }
            }
            Err(_) => false,
        }
    }

    /// Evaluate a signal via the LLM and return a structured verdict.
    pub async fn evaluate_signal(&self, proposal: &Proposal, context: &str) -> Result<AiVerdict> {
        let history_ctx = self.memory.context(&proposal.symbol);
        let prompt = format!(
            "You are a risk-aware trading assistant. Evaluate this signal.\n\
             Signal: {:?} {} at {:.4}, SL {:.4}, TP {:.4}, RR {:.2}\n\
             {}\n\
             {}\n\
             Reply strictly as JSON: {{\"decision\":\"GO|WATCH|NO-GO\",\"reason\":\"<short>\",\"confidence\":0.0-1.0}}",
            proposal.action,
            proposal.symbol,
            proposal.entry,
            proposal.stop_loss,
            proposal.take_profit,
            proposal.risk_reward,
            history_ctx,
            context
        );

        let url = format!("{}{}", self.base_url, GENERATE_ENDPOINT);
        let body = json!({
            "model": self.model,
            "prompt": prompt,
            "stream": false,
            "format": "json"
        });

        let resp: GenerateResponse = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .context("ollama request")?
            .error_for_status()
            .context("ollama status")?
            .json()
            .await
            .context("ollama json")?;

        parse_verdict(&resp.response).ok_or_else(|| anyhow!("unparseable verdict"))
    }

    /// Generic chat helper (spare capacity / future prompts).
    pub async fn chat(&self, prompt: &str) -> Result<String> {
        let url = format!("{}{}", self.base_url, GENERATE_ENDPOINT);
        let body = json!({
            "model": self.model,
            "prompt": prompt,
            "stream": false
        });
        let resp: GenerateResponse = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .context("ollama chat")?
            .error_for_status()
            .context("ollama chat status")?
            .json()
            .await
            .context("ollama chat json")?;
        Ok(resp.response)
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    /// Long-term historical data copy memory (2010→2026) with mined patterns.
    pub fn memory(&self) -> &history::HistoricalDataCopyMemory {
        &self.memory
    }
}

/// Conservative parser that never lets a bad model output crash the loop.
fn parse_verdict(raw: &str) -> Option<AiVerdict> {
    let trimmed = raw.trim();
    let without_fences = trimmed
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    if let Ok(v) = serde_json::from_str::<Value>(without_fences) {
        if let (Some(decision), Some(reason)) = (
            v.get("decision").and_then(|d| d.as_str()),
            v.get("reason").and_then(|d| d.as_str()),
        ) {
            let confidence = v
                .get("confidence")
                .and_then(|c| c.as_f64())
                .unwrap_or(0.5)
                .clamp(0.0, 1.0);
            let decision = normalize_decision(decision);
            return Some(AiVerdict {
                decision: decision.to_string(),
                reason: reason.to_string(),
                confidence,
            });
        }
    }
    // Failed to parse JSON: default to WATCH (safe no-trade).
    Some(AiVerdict {
        decision: "WATCH".to_string(),
        reason: "AI response unparseable — defaulting to watch".to_string(),
        confidence: 0.3,
    })
}

fn normalize_decision(d: &str) -> &str {
    let up = d.to_uppercase();
    if up.contains("NO") || up.contains("NOT") {
        "NO-GO"
    } else if up.contains("GO") {
        "GO"
    } else {
        "WATCH"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fenced_json() {
        let v = parse_verdict("```json\n{\"decision\":\"GO\",\"reason\":\"momentum\",\"confidence\":0.8}\n```");
        assert_eq!(v.unwrap().decision, "GO");
    }

    #[test]
    fn defaults_on_garbage() {
        let v = parse_verdict("well i think maybe buy?");
        assert_eq!(v.unwrap().decision, "WATCH");
    }
}