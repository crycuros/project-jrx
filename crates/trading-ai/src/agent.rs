//! The ReAct agent client — the bot's trade gate.
//!
//! The strategy engine proposes trades; this module asks the ReAct agent
//! (the Next.js terminal) whether they are worth taking, and the answer is a
//! hard gate.
//!
//! The gate **fails closed**. If the agent cannot be reached, times out,
//! answers something unreadable, or says HOLD, no trade happens. A bot that
//! quietly trades without the agent is exactly the failure mode this gate
//! exists to prevent.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;

use trading_core::{Config, OrderSide, Proposal};

/// The live readings the agent reasons about.
#[derive(Debug, Clone, Serialize, Default)]
pub struct AgentIndicators {
    pub rsi: f64,
    pub atr: f64,
    pub atr_pct: f64,
    pub ema20: f64,
    pub ema50: f64,
    pub ema200: f64,
    pub adx: f64,
    pub trend: String,
    pub range_position_pct: f64,
}

/// The candidate, as the agent sees it.
#[derive(Debug, Clone, Serialize)]
pub struct AgentRequest {
    pub symbol: String,
    pub timeframe: String,
    pub side: String,
    pub entry: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub balance: f64,
    pub equity: f64,
    pub daily_pnl: f64,
    pub total_pnl: f64,
    pub open_positions: i32,
    pub consecutive_losses: i32,
    pub strategy_source: String,
    pub rationale: String,
    pub indicators: AgentIndicators,
}

impl AgentRequest {
    /// Build a request from a strategy proposal and the live account.
    #[allow(clippy::too_many_arguments)]
    pub fn from_proposal(
        proposal: &Proposal,
        timeframe: &str,
        balance: f64,
        equity: f64,
        daily_pnl: f64,
        total_pnl: f64,
        open_positions: i32,
        consecutive_losses: i32,
        indicators: AgentIndicators,
    ) -> Self {
        Self {
            symbol: proposal.symbol.clone(),
            timeframe: timeframe.to_string(),
            side: match proposal.action {
                OrderSide::Buy => "BUY".to_string(),
                OrderSide::Sell => "SELL".to_string(),
            },
            entry: proposal.entry,
            stop_loss: proposal.stop_loss,
            take_profit: proposal.take_profit,
            balance,
            equity,
            daily_pnl,
            total_pnl,
            open_positions,
            consecutive_losses,
            strategy_source: proposal.strategy_source.clone(),
            rationale: proposal.reason.clone(),
            indicators,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct AgentDecision {
    action: String,
    #[serde(default)]
    confidence: f64,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    stake_usd: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
struct Outcome {
    decision: AgentDecision,
}

#[derive(Debug, Clone, Deserialize)]
struct AgentResponse {
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    outcome: Option<Outcome>,
    #[serde(default)]
    error: Option<String>,
}

/// What the gate decided.
#[derive(Debug, Clone)]
pub enum GateVerdict {
    /// The agent approved the trade.
    Approve {
        confidence: f64,
        reason: String,
        /// An upper bound the agent put on the stake, if it said anything.
        max_stake_usd: Option<f64>,
    },
    /// The agent said no.
    Reject {
        reason: String,
        confidence: f64,
    },
    /// The agent could not be reached or understood.
    Unavailable { reason: String },
    /// The gate is switched off.
    NotRequired,
}

/// The HTTP client for the ReAct agent.
pub struct AgentClient {
    http: reqwest::Client,
    base_url: String,
    api_secret: String,
    timeout: Duration,
    min_confidence: f64,
    required: bool,
}

impl AgentClient {
    pub fn new(cfg: &Config) -> Self {
        let timeout = Duration::from_secs(cfg.agent_timeout_secs.clamp(5, 600));
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .expect("build reqwest");
        Self {
            http,
            base_url: cfg.agent_api_url.trim_end_matches('/').to_string(),
            api_secret: cfg.agent_api_secret.clone(),
            timeout,
            min_confidence: cfg.agent_min_confidence,
            required: cfg.agent_required,
        }
    }

    pub fn is_required(&self) -> bool {
        self.required
    }

    pub fn min_confidence(&self) -> f64 {
        self.min_confidence
    }

    /// Ask the agent about a candidate.
    ///
    /// Never returns an error: every failure becomes `Unavailable`, and the
    /// caller decides what to do with that.
    pub async fn evaluate(&self, request: &AgentRequest) -> GateVerdict {
        if !self.required {
            return GateVerdict::NotRequired;
        }
        if self.base_url.is_empty() {
            return GateVerdict::Unavailable {
                reason: "AGENT_API_URL is not set".to_string(),
            };
        }

        let url = format!("{}/api/agent", self.base_url);
        let mut builder = self.http.post(&url).json(request).timeout(self.timeout);
        if !self.api_secret.is_empty() {
            builder = builder.header("x-jrx-secret", &self.api_secret);
        }

        let response = match builder.send().await {
            Ok(r) => r,
            Err(e) => {
                return GateVerdict::Unavailable {
                    reason: format!("the agent did not answer: {e}"),
                }
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return GateVerdict::Unavailable {
                reason: format!("the agent answered HTTP {status}: {}", body.chars().take(160).collect::<String>()),
            };
        }

        let parsed: AgentResponse = match response.json().await {
            Ok(v) => v,
            Err(e) => {
                return GateVerdict::Unavailable {
                    reason: format!("the agent's answer was unreadable: {e}"),
                }
            }
        };

        if !parsed.ok {
            return GateVerdict::Unavailable {
                reason: parsed
                    .error
                    .unwrap_or_else(|| "the agent reported a failure".to_string()),
            };
        }

        let Some(outcome) = parsed.outcome else {
            return GateVerdict::Unavailable {
                reason: "the agent's answer had no decision".to_string(),
            };
        };

        let decision = outcome.decision;
        let action = decision.action.trim().to_uppercase();
        if action == "HOLD" {
            return GateVerdict::Reject {
                reason: if decision.reason.is_empty() {
                    "the agent said HOLD".to_string()
                } else {
                    decision.reason
                },
                confidence: decision.confidence,
            };
        }

        if decision.confidence < self.min_confidence {
            return GateVerdict::Reject {
                reason: format!(
                    "the agent's confidence {:.2} is below the {:.2} gate",
                    decision.confidence, self.min_confidence
                ),
                confidence: decision.confidence,
            };
        }

        GateVerdict::Approve {
            confidence: decision.confidence,
            reason: decision.reason,
            max_stake_usd: decision.stake_usd,
        }
    }

    /// Is the agent reachable right now? Used for the health panel.
    pub async fn ping(&self) -> Result<()> {
        if self.base_url.is_empty() {
            return Err(anyhow!("AGENT_API_URL is not set"));
        }
        let url = format!("{}/api/health", self.base_url);
        let mut builder = self.http.get(&url).timeout(Duration::from_secs(5));
        if !self.api_secret.is_empty() {
            builder = builder.header("x-jrx-secret", &self.api_secret);
        }
        let response = builder.send().await.context("the agent did not answer")?;
        if !response.status().is_success() {
            return Err(anyhow!("the agent answered HTTP {}", response.status()));
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "agent_tests.rs"]
mod tests;
