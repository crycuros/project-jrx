//! Breakout strategy — fires in Regime::Breakout markets.
//!
//! Signals:
//!   1. Donchian channel breakout (20-bar high/low)
//!   2. Volatility squeeze breakout (BB width contracted then expanded)
//!   3. ATR expansion on breakout bar (confirms genuine move, not a fake-out)

use trading_core::{Candle, Market, OrderSide, Proposal, Regime};

use crate::indicators::{atr, bollinger_bands, donchian, ema};

pub fn analyze(candles: &[Candle], symbol: &str, _market: Market) -> Vec<Proposal> {
    let mut proposals = Vec::new();
    let n = candles.len();
    if n < 35 { return proposals; }

    let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
    let highs:  Vec<f64> = candles.iter().map(|c| c.high).collect();
    let lows:   Vec<f64> = candles.iter().map(|c| c.low).collect();

    let last   = closes[n - 1];
    let atr_v  = atr(&highs, &lows, &closes, 14)[n - 1].unwrap_or(last * 0.01);
    let atr_p  = atr(&highs, &lows, &closes, 14)[n - 2].unwrap_or(last * 0.01);
    let regime = Regime::Breakout.as_str().to_string();

    // ── Signal 1: Donchian channel breakout ─────────────────────────────────
    let (don_up, don_lo) = donchian(&highs, &lows, 20);
    if n >= 3 {
        let prev_close  = closes[n - 2];
        let prev2_close = closes[n - 3];
        // Confirm: two bars ago was inside channel, previous bar touched/broke, today confirms close above
        if let (Some(pu2), Some(pu1)) = (don_up[n - 3], don_up[n - 2]) {
            let expansion = atr_v > atr_p * 1.1;  // ATR expanding = real move
            if prev2_close < pu2 && prev_close >= pu1 && last > prev_close && expansion {
                // Bull breakout confirmed by ATR expansion
                let stop   = don_lo[n - 1].unwrap_or(last - atr_v * 2.0).max(0.0);
                let target = last + atr_v * 3.5;
                proposals.push(make_proposal(symbol, OrderSide::Buy, last, stop, target,
                    0.80, &regime, "breakout::donchian_up_confirmed"));
            }
        }
        if let (Some(pl2), Some(pl1)) = (don_lo[n - 3], don_lo[n - 2]) {
            let expansion = atr_v > atr_p * 1.1;
            if prev2_close > pl2 && prev_close <= pl1 && last < prev_close && expansion {
                // Bear breakout confirmed
                let stop   = don_up[n - 1].unwrap_or(last + atr_v * 2.0);
                let target = (last - atr_v * 3.5).max(0.0);
                proposals.push(make_proposal(symbol, OrderSide::Sell, last, stop, target,
                    0.79, &regime, "breakout::donchian_down_confirmed"));
            }
        }
    }

    // ── Signal 2: Volatility squeeze breakout ───────────────────────────────
    // BB width contracts (squeeze) then expands sharply → breakout
    let (_, bb_up, bb_lo) = bollinger_bands(&closes, 20, 2.0);
    let width_now  = match (bb_up[n - 1], bb_lo[n - 1]) {
        (Some(u), Some(l)) => u - l,
        _ => 0.0,
    };
    let width_prev = match (bb_up[n - 5], bb_lo[n - 5]) {
        (Some(u), Some(l)) => u - l,
        _ => 0.0,
    };
    let squeeze_then_expand = width_prev > 0.0 && width_now > width_prev * 1.4;

    if squeeze_then_expand {
        let ef = ema(&closes, 10)[n - 1].unwrap_or(last);
        let es = ema(&closes, 30)[n - 1].unwrap_or(last);
        if last > ef && ef > es {
            // Expanding up + trend alignment
            let stop   = (last - atr_v * 1.8).max(0.0);
            let target = last + atr_v * 3.0;
            proposals.push(make_proposal(symbol, OrderSide::Buy, last, stop, target,
                0.75, &regime, "breakout::bb_squeeze_up"));
        } else if last < ef && ef < es {
            let stop   = last + atr_v * 1.8;
            let target = (last - atr_v * 3.0).max(0.0);
            proposals.push(make_proposal(symbol, OrderSide::Sell, last, stop, target,
                0.74, &regime, "breakout::bb_squeeze_down"));
        }
    }

    proposals
}

fn make_proposal(
    symbol: &str, action: OrderSide, entry: f64,
    stop_loss: f64, take_profit: f64, confidence: f64,
    regime: &str, source: &str,
) -> Proposal {
    let rr = if action == OrderSide::Buy {
        if entry - stop_loss > 0.0 { (take_profit - entry) / (entry - stop_loss) } else { 0.0 }
    } else {
        if stop_loss - entry > 0.0 { (entry - take_profit) / (stop_loss - entry) } else { 0.0 }
    };
    Proposal {
        symbol: symbol.to_string(),
        action,
        entry,
        stop_loss,
        take_profit,
        risk_reward: rr,
        confidence,
        regime: regime.to_string(),
        reason: source.to_string(),
        strategy_source: source.to_string(),
        vote_count: 1,
        ..Default::default()
    }
}
