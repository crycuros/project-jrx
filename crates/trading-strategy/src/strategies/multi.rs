//! Multi-strategy combiner — Stage 3 (Allocate) of the quant pipeline.
//!
//! Implements signal voting and IC-weighted confidence:
//!   - Collect proposals from all sub-strategies
//!   - Group by (symbol, direction)
//!   - If 2+ strategies agree → boost confidence, increment vote_count
//!   - If strategies conflict on direction → drop to avoid whipsaw
//!   - Apply Grinold-Kahn IC weighting: confidence *= sqrt(vote_count)

use trading_core::{OrderSide, Proposal};

/// Combine proposals from multiple sub-strategies using signal voting.
/// Conflicting proposals (same symbol, opposite side) are dropped.
/// Agreeing proposals are merged into a single higher-confidence proposal.
pub fn combine(mut proposals: Vec<Proposal>) -> Vec<Proposal> {
    if proposals.is_empty() {
        return proposals;
    }

    // Sort for determinism
    proposals.sort_by(|a, b| a.symbol.cmp(&b.symbol)
        .then(a.strategy_source.cmp(&b.strategy_source)));

    let mut out: Vec<Proposal> = Vec::new();

    // Group by symbol
    let mut symbols: Vec<String> = proposals.iter().map(|p| p.symbol.clone()).collect();
    symbols.dedup();

    for sym in symbols {
        let sym_props: Vec<&Proposal> = proposals.iter()
            .filter(|p| p.symbol == sym)
            .collect();

        let buys:  Vec<&&Proposal> = sym_props.iter().filter(|p| p.action == OrderSide::Buy).collect();
        let sells: Vec<&&Proposal> = sym_props.iter().filter(|p| p.action == OrderSide::Sell).collect();

        // Conflict: both sides present → skip this symbol
        if !buys.is_empty() && !sells.is_empty() {
            log::debug!("multi: conflicting signals for {} — skipping", sym);
            continue;
        }

        let winners = if !buys.is_empty() { &buys } else { &sells };
        if winners.is_empty() { continue; }

        // Best single proposal (highest confidence)
        let best = winners.iter().max_by(|a, b| {
            a.confidence.partial_cmp(&b.confidence).unwrap_or(std::cmp::Ordering::Equal)
        }).unwrap();

        let vote_count = winners.len() as u8;

        // IC-weighted confidence boost: conf *= sqrt(votes) / sqrt(1) capped at 0.95
        // Rationale: multiple independent signals → higher information coefficient
        let raw_conf = best.confidence;
        let boosted  = (raw_conf * (vote_count as f64).sqrt()).min(0.95);

        // Merge sources
        let sources: Vec<String> = winners.iter()
            .map(|p| p.strategy_source.clone())
            .collect();

        // Best risk-reward among agreeing signals
        let best_rr = winners.iter()
            .map(|p| p.risk_reward)
            .fold(0.0f64, f64::max);

        let merged = Proposal {
            symbol: sym.clone(),
            action: best.action,
            entry: best.entry,
            stop_loss: best.stop_loss,
            take_profit: best.take_profit,
            risk_reward: best_rr,
            confidence: boosted,
            regime: best.regime.clone(),
            reason: sources.join(" + "),
            strategy_source: sources.join("|"),
            vote_count,
            contract_mode: best.contract_mode,
            duration_ticks: best.duration_ticks,
        };

        out.push(merged);
    }

    // Sort final output by confidence descending — best opportunities first
    out.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence)
        .unwrap_or(std::cmp::Ordering::Equal));

    out
}

/// Kelly criterion fraction for optimal position sizing.
/// Uses half-Kelly for safety: kelly_f * 0.5
///
/// kelly_f = (win_rate * avg_win_rr - (1 - win_rate)) / avg_win_rr
///
/// Returns a fraction of equity to risk [0.01 .. 0.15]
pub fn kelly_fraction(win_rate: f64, avg_win_rr: f64) -> f64 {
    if avg_win_rr <= 0.0 || win_rate <= 0.0 {
        return 0.01;
    }
    let lose_rate = 1.0 - win_rate;
    let kelly_f = (win_rate * avg_win_rr - lose_rate) / avg_win_rr;
    // Half-Kelly, clamped to [1%, 15%] of equity
    (kelly_f * 0.5).clamp(0.01, 0.15)
}

#[cfg(test)]
mod tests {
    use super::*;
    use trading_core::{ContractMode, Proposal};

    fn prop(sym: &str, side: OrderSide, conf: f64, src: &str) -> Proposal {
        Proposal {
            symbol: sym.to_string(),
            action: side,
            confidence: conf,
            strategy_source: src.to_string(),
            vote_count: 1,
            risk_reward: 2.0,
            entry: 100.0,
            stop_loss: 98.0,
            take_profit: 104.0,
            regime: "trending".to_string(),
            reason: src.to_string(),
            contract_mode: ContractMode::Multiplier,
            duration_ticks: 0,
        }
    }

    #[test]
    fn agrees_boost_confidence() {
        let props = vec![
            prop("BTC/USDT", OrderSide::Buy, 0.70, "trend::ema"),
            prop("BTC/USDT", OrderSide::Buy, 0.68, "mean_rev::rsi"),
        ];
        let result = combine(props);
        assert_eq!(result.len(), 1);
        assert!(result[0].confidence > 0.70, "vote boost expected");
        assert_eq!(result[0].vote_count, 2);
    }

    #[test]
    fn conflicts_dropped() {
        let props = vec![
            prop("ETH/USDT", OrderSide::Buy, 0.70, "trend::ema"),
            prop("ETH/USDT", OrderSide::Sell, 0.72, "mean_rev::bb"),
        ];
        let result = combine(props);
        assert!(result.is_empty(), "conflicting signals should be dropped");
    }

    #[test]
    fn kelly_typical() {
        // 0.55 WR @ 2.0 RR -> full Kelly 0.325, half-Kelly 0.1625,
        // clamped down to the documented 0.15 ceiling.
        let f = kelly_fraction(0.55, 2.0);
        assert!(f > 0.01 && f <= 0.15, "kelly fraction {f} out of expected range");
    }

    #[test]
    fn kelly_clamped_high() {
        // A very strong edge still cannot exceed the 15% ceiling.
        let f = kelly_fraction(0.80, 3.0);
        assert_eq!(f, 0.15);
    }

    #[test]
    fn kelly_no_edge_floors_at_min() {
        // 30% WR @ 1.0 RR is a losing edge -> negative Kelly -> floor at 1%.
        let f = kelly_fraction(0.30, 1.0);
        assert_eq!(f, 0.01);
    }
}
