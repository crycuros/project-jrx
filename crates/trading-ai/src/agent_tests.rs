//! Tests for the ReAct agent gate.
//!
//! The gate is the thing that stands between a signal and real money, so the
//! failure modes are the interesting part: an unreachable agent, a garbled
//! answer, a HOLD, a low confidence, and the side the agent disagrees with.

use super::{AgentClient, AgentIndicators, AgentRequest, GateVerdict};
use trading_core::{Config, OrderSide, Proposal};

fn proposal(side: OrderSide) -> Proposal {
    Proposal {
        symbol: "XAU_USD".to_string(),
        action: side,
        entry: 4000.0,
        stop_loss: 3992.0,
        take_profit: 4016.0,
        risk_reward: 2.0,
        confidence: 0.72,
        regime: "trending".to_string(),
        reason: "trend::ema_cross_up".to_string(),
        strategy_source: "trend::ema_cross_up".to_string(),
        vote_count: 1,
        ..Default::default()
    }
}

fn request() -> AgentRequest {
    AgentRequest::from_proposal(
        &proposal(OrderSide::Buy),
        "15m",
        500.0,
        500.0,
        0.0,
        0.0,
        1,
        0,
        AgentIndicators::default(),
    )
}

fn config(agent_api_url: &str, required: bool) -> Config {
    let mut cfg = Config::from_env();
    cfg.agent_api_url = agent_api_url.to_string();
    cfg.agent_required = required;
    cfg.agent_min_confidence = 0.6;
    cfg
}

#[tokio::test]
async fn an_unreachable_agent_is_unavailable_not_approved() {
    // Port 1 is reserved and nothing listens there.
    let client = AgentClient::new(&config("http://127.0.0.1:1", true));
    let verdict = client.evaluate(&request()).await;
    match verdict {
        GateVerdict::Unavailable { reason } => {
            assert!(!reason.is_empty(), "the reason has to say something");
        }
        other => panic!("an unreachable agent must never approve: {other:?}"),
    }
}

#[tokio::test]
async fn the_gate_can_be_switched_off() {
    let client = AgentClient::new(&config("http://127.0.0.1:1", false));
    assert!(matches!(
        client.evaluate(&request()).await,
        GateVerdict::NotRequired
    ));
}

#[tokio::test]
async fn an_empty_url_is_unavailable() {
    let client = AgentClient::new(&config("", true));
    assert!(matches!(
        client.evaluate(&request()).await,
        GateVerdict::Unavailable { .. }
    ));
}

#[test]
fn the_request_carries_the_live_readings() {
    let mut indicators = AgentIndicators::default();
    indicators.rsi = 61.5;
    indicators.atr = 5.25;
    indicators.trend = "uptrend".to_string();
    let req = AgentRequest::from_proposal(
        &proposal(OrderSide::Sell),
        "15m",
        500.0,
        500.0,
        -2.0,
        10.0,
        3,
        1,
        indicators,
    );
    assert_eq!(req.side, "SELL");
    assert_eq!(req.symbol, "XAU_USD");
    assert_eq!(req.entry, 4000.0);
    assert_eq!(req.indicators.rsi, 61.5);
    assert_eq!(req.daily_pnl, -2.0);
    assert_eq!(req.open_positions, 3);
    assert_eq!(req.consecutive_losses, 1);
}
