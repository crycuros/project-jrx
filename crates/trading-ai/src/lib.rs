//! AI evaluation layer — interfaces with local Ollama.
//!
//! Two AI paths live here:
//!
//! * [`AiClient`] — the original single-shot Ollama gate used by the desktop
//!   app.
//! * [`agent::AgentClient`] — the ReAct decision gate: the bot asks the
//!   Next.js terminal for a verdict before every trade, and the gate fails
//!   closed.

pub mod agent;
pub mod history;
pub mod news;

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
    memory: history::MarketMemory,
}

impl AiClient {
    pub fn new(cfg: &Config) -> Self {
        let memory = history::MarketMemory::load(cfg);
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

    /// Check whether Ollama is up. If offline, automatically attempts to spawn `ollama serve` and wait for it.
    pub async fn ensure_running(&self) -> bool {
        if self.ping().await {
            return true;
        }

        log::info!("Ollama not responding at {}. Auto-starting 'ollama serve'...", self.base_url);
        if let Err(e) = Self::spawn_ollama_process() {
            log::warn!("Auto-start of ollama process encountered: {e}");
        }

        // Retry probe up to 10 seconds (20 iterations * 500ms)
        for _ in 0..20 {
            tokio::time::sleep(Duration::from_millis(500)).await;
            if self.ping().await {
                log::info!("Ollama server auto-started successfully and is now ONLINE.");
                return true;
            }
        }

        log::warn!("Timed out waiting for auto-started Ollama server.");
        false
    }

    /// Spawns `ollama serve` detached in the background without creating a console window.
    pub fn spawn_ollama_process() -> std::io::Result<()> {
        let mut cmd = std::process::Command::new("ollama");
        cmd.arg("serve");
        cmd.stdin(std::process::Stdio::null());
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            const DETACHED_PROCESS: u32 = 0x00000008;
            cmd.creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS);
        }

        match cmd.spawn() {
            Ok(_) => Ok(()),
            Err(e) => {
                // Try standard Windows install location if PATH lookup fails
                #[cfg(target_os = "windows")]
                if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
                    let exe = std::path::PathBuf::from(local_app_data)
                        .join("Programs")
                        .join("Ollama")
                        .join("ollama.exe");
                    if exe.exists() {
                        use std::os::windows::process::CommandExt;
                        const CREATE_NO_WINDOW: u32 = 0x08000000;
                        const DETACHED_PROCESS: u32 = 0x00000008;
                        let mut fallback_cmd = std::process::Command::new(exe);
                        fallback_cmd.arg("serve");
                        fallback_cmd.stdin(std::process::Stdio::null());
                        fallback_cmd.stdout(std::process::Stdio::null());
                        fallback_cmd.stderr(std::process::Stdio::null());
                        fallback_cmd.creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS);
                        return fallback_cmd.spawn().map(|_| ());
                    }
                }
                Err(e)
            }
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

    /// Long-term market memory with mined patterns.
    pub fn memory(&self) -> &history::MarketMemory {
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