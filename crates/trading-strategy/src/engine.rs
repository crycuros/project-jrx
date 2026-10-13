//! Strategy engine: multi-strategy quantitative pipeline.
//!
//! Pipeline:
//!   1. Ingest candles
//!   2. Regime classification (Condition gate)
//!   3. Run regime-appropriate strategy modules (Predict)
//!   4. Combine signals via voting combiner (Allocate)
//!   5. Return best proposal(s) for risk evaluation
//!
//! Based on the 7-stage quant framework: Ingest→Predict→Condition→Allocate→Constrain→Execute→Evaluate

use trading_core::{Candle, Market, OrderSide, Proposal, Regime};

use crate::indicators::{atr, ema, rsi};
use crate::regime::RegimeDetector;
use crate::strategies::{breakout, mean_reversion, multi, trend};

/// Result of the quality gate used before AI evaluation.
#[derive(Debug, Clone)]
pub struct SignalQuality {
    pub score: f64,
    pub grade: String,
    pub approved: bool,
    pub reasons: Vec<String>,
}

impl SignalQuality {
    pub fn new(score: f64) -> Self {
        let grade = match score as i32 {
            80.. => "A",
            70.. => "B",
            60.. => "C",
            _ => "D",
        };
        Self {
            score,
            grade: grade.to_string(),
            approved: score >= 55.0,
            reasons: Vec::new(),
        }
    }
}

pub struct StrategyEngine;

impl StrategyEngine {
    /// Full multi-strategy pipeline:
    /// classifies regime → runs matching strategies → combines signals.
    /// Returns the best merged proposal or None.
    pub fn process(&self, candles: &[Candle], symbol: &str, market: Market) -> Option<Proposal> {
        let proposals = self.process_multi(candles, symbol, market);
        proposals.into_iter().next()
    }

    /// Run full pipeline and return all combined proposals (sorted by confidence).
    pub fn process_multi(
        &self,
        candles: &[Candle],
        symbol: &str,
        market: Market,
    ) -> Vec<Proposal> {
        if candles.len() < 40 {
            return Vec::new();
        }

        // ── Stage 2: Regime Classification (Condition gate) ─────────────────
        let (regime, adx_val, atr_pct, regime_reason) =
            RegimeDetector::classify(candles, market);

        log::debug!(
            "strategy: {} regime={} ADX={:.1} ATR%={:.2} — {}",
            symbol, regime.as_str(), adx_val, atr_pct, regime_reason
        );

        // ── Stage 3: Run regime-appropriate strategies (Predict) ─────────────
        let mut all_proposals: Vec<Proposal> = Vec::new();

        match regime {
            Regime::Trending => {
                all_proposals.extend(trend::analyze(candles, symbol, market));
            }
            Regime::Ranging => {
                all_proposals.extend(mean_reversion::analyze(candles, symbol, market));
                // Shallow pullbacks to EMA-20 in an established macro trend (EMA50 > EMA200)
                // often temporarily compress ADX into Ranging; allow 5R shallow reclaim signals:
                let t = trend::analyze(candles, symbol, market);
                all_proposals.extend(
                    t.into_iter()
                        .filter(|p| p.strategy_source == "trend::ema20_shallow_reclaim_5r"),
                );
            }
            Regime::Breakout => {
                all_proposals.extend(breakout::analyze(candles, symbol, market));
                // Also allow trend signals during breakout (momentum is valid)
                all_proposals.extend(trend::analyze(candles, symbol, market));
            }
            Regime::Volatile => {
                // High volatility: reduce to only the highest-confidence trend signals
                let t = trend::analyze(candles, symbol, market);
                all_proposals.extend(t.into_iter().filter(|p| p.confidence >= 0.75));
            }
        }

        if all_proposals.is_empty() {
            return Vec::new();
        }

        // ── Stage 4: Signal voting + IC-weighted combination (Allocate) ──────
        let combined = multi::combine(all_proposals);

        // Minimum R:R filter — only pass proposals with RR > 1.5
        combined.into_iter().filter(|p| p.risk_reward >= 1.5).collect()
    }

    /// Market-aware quality scoring (Evaluate gate before AI).
    pub fn score_quality(&self, side: OrderSide, candles: &[Candle], symbol: &str, market: Market) -> SignalQuality {
        let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
        if closes.len() < 30 {
            return SignalQuality::new(0.0);
        }
        let n = closes.len();
        let r = rsi(&closes, 14)[n - 1].unwrap_or(50.0);

        let mut score: f64 = 50.0;
        let mut reasons = Vec::new();

        // Trend alignment (EMA5 vs EMA20)
        let ema5  = ema(&closes, 5)[n - 1].unwrap_or(closes[n - 1]);
        let ema20 = ema(&closes, 20)[n - 1].unwrap_or(closes[n - 1]);
        let trending_up   = ema5 > ema20;
        let trending_down = ema5 < ema20;

        match side {
            OrderSide::Buy => {
                if trending_up {
                    score += 15.0;
                    reasons.push("trend align".to_string());
                }
                if (40.0..70.0).contains(&r) {
                    score += 10.0;
                    reasons.push("rsi zone".to_string());
                }
            }
            OrderSide::Sell => {
                if trending_down {
                    score += 15.0;
                    reasons.push("trend align".to_string());
                }
                if (30.0..60.0).contains(&r) {
                    score += 10.0;
                    reasons.push("rsi zone".to_string());
                }
            }
        }

        // Regime awareness bonus: if regime matches strategy direction, reward
        let (regime, _, _, _) = RegimeDetector::classify(candles, market);
        match (side, regime) {
            (OrderSide::Buy,  Regime::Trending | Regime::Breakout) => {
                score += 10.0;
                reasons.push("regime match".to_string());
            }
            (OrderSide::Sell, Regime::Ranging | Regime::Volatile) => {
                score += 8.0;
                reasons.push("regime match".to_string());
            }
            _ => {}
        }

        // Volatility sanity (ATR% bounds)
        let last = closes[n - 1];
        let highs: Vec<f64> = candles.iter().map(|c| c.high).collect();
        let lows:  Vec<f64> = candles.iter().map(|c| c.low).collect();
        let atr_val = atr(&highs, &lows, &closes, 14)[n - 1].unwrap_or(0.0);
        let atr_pct = if last > 0.0 { atr_val / last } else { 0.0 };
        if !(0.0005..0.08).contains(&atr_pct) {
            score -= 15.0;
            reasons.push("volatility out of range".to_string());
        }

        // Gold boost
        if market == Market::Gold && matches!(symbol, "XAU_USD" | "XAUUSD" | "XAU/USD") {
            score += 10.0;
            reasons.push("gold hft".to_string());
        }

        SignalQuality {
            score: score.clamp(0.0, 100.0),
            grade: grade_of(score),
            approved: score >= 55.0,
            reasons,
        }
    }
}

fn grade_of(score: f64) -> String {
    match score as i32 {
        80.. => "A".to_string(),
        70..  => "B".to_string(),
        60..  => "C".to_string(),
        _     => "D".to_string(),
    }
}
