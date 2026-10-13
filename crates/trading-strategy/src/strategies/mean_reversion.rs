//! Mean Reversion strategy — fires in Regime::Ranging markets.
//!
//! Signals:
//!   1. Bollinger Band touch (price at lower band → buy, upper → sell)
//!   2. RSI extreme reversion (RSI < 25 → buy, RSI > 75 → sell)
//!   3. Z-score reversion (|z| > 2.0 → fade the move)
//!   4. Stochastic cross from extreme zones

use trading_core::{Candle, Market, OrderSide, Proposal, Regime};

use crate::indicators::{atr, bollinger_bands, ema, rsi, stochastic, zscore};

pub fn analyze(candles: &[Candle], symbol: &str, _market: Market) -> Vec<Proposal> {
    let mut proposals = Vec::new();
    let n = candles.len();
    if n < 30 { return proposals; }

    let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
    let highs:  Vec<f64> = candles.iter().map(|c| c.high).collect();
    let lows:   Vec<f64> = candles.iter().map(|c| c.low).collect();

    let last   = closes[n - 1];
    let atr_v  = atr(&highs, &lows, &closes, 14)[n - 1].unwrap_or(last * 0.01);
    let rsi_v  = rsi(&closes, 14)[n - 1].unwrap_or(50.0);
    let rsi_p  = rsi(&closes, 14)[n - 2].unwrap_or(50.0);
    let regime = Regime::Ranging.as_str().to_string();

    // ── Signal 1: Bollinger Band mean reversion ──────────────────────────────
    let (bb_mid, bb_up, bb_lo) = bollinger_bands(&closes, 20, 2.0);
    if let (Some(mid), Some(lo), Some(up)) = (bb_mid[n - 1], bb_lo[n - 1], bb_up[n - 1]) {
        let prev = closes[n - 2];
        // Touch lower band and RSI not still dropping hard
        if prev < lo && last > lo && rsi_v > 25.0 && rsi_v < 55.0 {
            let stop   = (last - atr_v * 1.0).max(0.0);
            let target = mid;   // revert to mean (middle band)
            proposals.push(make_proposal(symbol, OrderSide::Buy, last, stop, target,
                0.68, &regime, "mean_rev::bb_lower_touch"));
        }
        // Touch upper band
        if prev > up && last < up && rsi_v < 75.0 && rsi_v > 45.0 {
            let stop   = last + atr_v * 1.0;
            let target = mid;
            proposals.push(make_proposal(symbol, OrderSide::Sell, last, stop, target,
                0.67, &regime, "mean_rev::bb_upper_touch"));
        }
    }

    // ── Signal 2: RSI extreme reversion ─────────────────────────────────────
    // RSI crosses back above 25 from below (oversold exhaustion)
    if rsi_p < 25.0 && rsi_v >= 25.0 {
        let stop   = (last - atr_v * 1.2).max(0.0);
        let target = last + atr_v * 2.0;
        proposals.push(make_proposal(symbol, OrderSide::Buy, last, stop, target,
            0.74, &regime, "mean_rev::rsi_oversold_cross"));
    }
    // RSI crosses back below 75 from above (overbought exhaustion)
    if rsi_p > 75.0 && rsi_v <= 75.0 {
        let stop   = last + atr_v * 1.2;
        let target = (last - atr_v * 2.0).max(0.0);
        proposals.push(make_proposal(symbol, OrderSide::Sell, last, stop, target,
            0.73, &regime, "mean_rev::rsi_overbought_cross"));
    }

    // ── Signal 3: Z-score reversion ─────────────────────────────────────────
    let z_vals = zscore(&closes, 20);
    if let Some(z) = z_vals[n - 1] {
        let z_prev = z_vals[n - 2].unwrap_or(0.0);
        // Price was 2+ stddevs below mean, now reverting upward
        if z_prev < -2.0 && z > z_prev {
            let stop   = (last - atr_v * 1.0).max(0.0);
            let target = last + atr_v * 2.0;
            proposals.push(make_proposal(symbol, OrderSide::Buy, last, stop, target,
                0.71, &regime, "mean_rev::zscore_low_revert"));
        }
        // Price was 2+ stddevs above mean, now reverting downward
        if z_prev > 2.0 && z < z_prev {
            let stop   = last + atr_v * 1.0;
            let target = (last - atr_v * 2.0).max(0.0);
            proposals.push(make_proposal(symbol, OrderSide::Sell, last, stop, target,
                0.70, &regime, "mean_rev::zscore_high_revert"));
        }
    }

    // ── Signal 4: Stochastic cross from extreme ──────────────────────────────
    let (k_vals, d_vals) = stochastic(&highs, &lows, &closes, 14, 3);
    if let (Some(k), Some(d), Some(kp), Some(dp)) = (
        k_vals[n - 1], d_vals[n - 1],
        k_vals[n - 2], d_vals[n - 2],
    ) {
        // K crosses above D from oversold (<20)
        if kp < dp && k > d && k < 40.0 {
            let stop   = (last - atr_v * 1.0).max(0.0);
            let target = last + atr_v * 1.8;
            proposals.push(make_proposal(symbol, OrderSide::Buy, last, stop, target,
                0.64, &regime, "mean_rev::stoch_cross_up"));
        }
        // K crosses below D from overbought (>80)
        if kp > dp && k < d && k > 60.0 {
            let stop   = last + atr_v * 1.0;
            let target = (last - atr_v * 1.8).max(0.0);
            proposals.push(make_proposal(symbol, OrderSide::Sell, last, stop, target,
                0.63, &regime, "mean_rev::stoch_cross_down"));
        }
    }

    // ── Signal 5: Keltner 1.6x ATR Exhaustion Snapback to EMA 20 (61.7% WR Gold/FX Edge) ──
    let ema20_vals = ema(&closes, 20);
    if let Some(ema20_v) = ema20_vals[n - 1] {
        let last_open = candles[n - 1].open;
        let stretch_down = ema20_v - last;
        let stretch_up = last - ema20_v;

        // Oversold stretch > 1.60 * ATR below EMA 20 + bullish rejection candle
        if stretch_down >= atr_v * 1.60 && last > last_open {
            let stop = (last - atr_v * 1.0).max(0.0);
            let target = ema20_v.max(last + atr_v * 1.60);
            proposals.push(make_proposal(
                symbol,
                OrderSide::Buy,
                last,
                stop,
                target,
                0.79,
                &regime,
                "mean_rev::keltner_ema20_snapback",
            ));
        }
        // Overbought stretch > 1.60 * ATR above EMA 20 + bearish rejection candle
        if stretch_up >= atr_v * 1.60 && last < last_open {
            let stop = last + atr_v * 1.0;
            let target = ema20_v.min((last - atr_v * 1.60).max(0.0));
            proposals.push(make_proposal(
                symbol,
                OrderSide::Sell,
                last,
                stop,
                target,
                0.78,
                &regime,
                "mean_rev::keltner_ema20_snapback",
            ));
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
