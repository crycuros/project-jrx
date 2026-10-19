//! Ask the ReAct agent about a candidate trade.
//!
//! This is the same call the bot makes on every signal, so it doubles as the
//! end-to-end check of the wiring:
//!
//! ```text
//! AGENT_API_URL=http://127.0.0.1:3000 cargo run -p trading-ai --example ask_agent
//! ```
//!
//! It prints the agent's verdict, the reason it gave, and the tools it used.

use trading_ai::agent::{AgentClient, AgentIndicators, AgentRequest, GateVerdict};
use trading_core::{Config, OrderSide, Proposal};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = Config::from_env();
    let client = AgentClient::new(&cfg);

    let side = std::env::var("SIDE").unwrap_or_else(|_| "BUY".to_string());
    let side = if side.eq_ignore_ascii_case("sell") {
        OrderSide::Sell
    } else {
        OrderSide::Buy
    };

    let proposal = Proposal {
        symbol: std::env::var("SYMBOL").unwrap_or_else(|_| "XAU_USD".to_string()),
        action: side,
        entry: 4167.24,
        stop_loss: 4159.22,
        take_profit: 4179.26,
        risk_reward: 1.5,
        confidence: 0.72,
        regime: "trending".to_string(),
        reason: "trend::ema_cross_up".to_string(),
        strategy_source: "trend::ema_cross_up".to_string(),
        vote_count: 1,
        ..Default::default()
    };

    let indicators = AgentIndicators {
        rsi: 43.3,
        atr: 5.05,
        atr_pct: 0.1212,
        ema20: 4170.04,
        ema50: 4165.2,
        ema200: 4183.55,
        adx: 28.3,
        trend: "mixed".to_string(),
        range_position_pct: 58.0,
    };

    let request = AgentRequest::from_proposal(
        &proposal,
        "15m",
        500.0,
        500.0,
        0.0,
        0.0,
        1,
        0,
        indicators,
    );

    println!("asking {} …", cfg.agent_api_url);
    let verdict = client.evaluate(&request).await;
    match verdict {
        GateVerdict::Approve {
            confidence,
            reason,
            max_stake_usd,
        } => {
            println!("APPROVE  confidence {confidence:.2}");
            println!("  reason: {reason}");
            if let Some(cap) = max_stake_usd {
                println!("  stake cap: ${cap:.2}");
            }
        }
        GateVerdict::Reject { reason, confidence } => {
            println!("REJECT   confidence {confidence:.2}");
            println!("  reason: {reason}");
        }
        GateVerdict::Unavailable { reason } => {
            println!("UNAVAILABLE - the gate fails closed, so this is a NO TRADE");
            println!("  reason: {reason}");
        }
        GateVerdict::NotRequired => {
            println!("the gate is switched off (AGENT_REQUIRED=false)");
        }
    }
    Ok(())
}
