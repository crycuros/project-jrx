//! Market regime classifier — Stage 2 (Condition) of the quant pipeline.
//!
//! Classifies the current market state BEFORE running strategy signals.
//! This prevents trend strategies from firing in ranging markets and vice versa.
//!
//! ADX > 25  + EMA spread wide  → Trending
//! ADX < 20  + narrow ATR%      → Ranging
//! ATR% > 4% (crypto) / 1.5% (FX) → Volatile
//! Donchian breakout            → Breakout

use trading_core::{Candle, Market, Regime};

use crate::indicators::{adx, atr, donchian, ema};

pub struct RegimeDetector;

impl RegimeDetector {
    /// Classify the regime from the last N candles.
    /// Returns (regime, adx_value, atr_pct, reasoning)
    pub fn classify(
        candles: &[Candle],
        market: Market,
    ) -> (Regime, f64, f64, String) {
        let n = candles.len();
        if n < 30 {
            return (Regime::Trending, 0.0, 0.0, "insufficient data".to_string());
        }

        let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
        let highs:  Vec<f64> = candles.iter().map(|c| c.high).collect();
        let lows:   Vec<f64> = candles.iter().map(|c| c.low).collect();

        let last = closes[n - 1];

        // ── ADX for trend strength ──────────────────────────────────────────
        let (adx_vals, pdi, mdi) = adx(&highs, &lows, &closes, 14);
        let adx_val = adx_vals[n - 1].unwrap_or(20.0);
        let pdi_val = pdi[n - 1].unwrap_or(20.0);
        let mdi_val = mdi[n - 1].unwrap_or(20.0);

        // ── ATR% for volatility ─────────────────────────────────────────────
        let atr_vals = atr(&highs, &lows, &closes, 14);
        let atr_val  = atr_vals[n - 1].unwrap_or(last * 0.01);
        let atr_pct  = if last > 0.0 { atr_val / last * 100.0 } else { 1.0 };

        // Market-specific volatility threshold
        let vol_threshold = match market {
            Market::Crypto | Market::Forex => 0.8,
            Market::Gold                   => 1.2,
            Market::Synthetic              => 1.5,
        };

        // ── EMA spread (trend direction breadth) ───────────────────────────
        let ema_fast = ema(&closes, 12);
        let ema_slow = ema(&closes, 50);
        let ef = ema_fast[n - 1].unwrap_or(last);
        let es = ema_slow[n - 1].unwrap_or(last);
        let ema_spread_pct = if es > 0.0 { (ef - es).abs() / es * 100.0 } else { 0.0 };

        // ── Donchian breakout check (prev bar vs 20-bar channel) ───────────
        let (don_upper, don_lower) = donchian(&highs, &lows, 20);
        let prev_close   = closes[n - 2];
        let prev_upper   = don_upper[n - 2].unwrap_or(f64::MAX);
        let prev_lower   = don_lower[n - 2].unwrap_or(f64::MIN);
        let curr_upper   = don_upper[n - 1].unwrap_or(f64::MAX);
        let curr_lower   = don_lower[n - 1].unwrap_or(f64::MIN);
        // Breakout: yesterday was inside, today closed ABOVE channel high or BELOW channel low
        let is_breakout =
            (prev_close < prev_upper && last > curr_upper) ||
            (prev_close > prev_lower && last < curr_lower);

        // ── Classification rules (in priority order) ───────────────────────
        let regime;
        let reason;

        if atr_pct > vol_threshold * 1.8 {
            regime = Regime::Volatile;
            reason = format!("ATR% {:.2} far above threshold {:.1}", atr_pct, vol_threshold * 1.8);
        } else if is_breakout && adx_val > 18.0 {
            regime = Regime::Breakout;
            reason = format!("Donchian breakout, ADX {:.1}", adx_val);
        } else if adx_val >= 25.0 && ema_spread_pct >= 0.5 {
            regime = Regime::Trending;
            let dir = if pdi_val > mdi_val { "bullish" } else { "bearish" };
            reason = format!("ADX {:.1} trending {}, EMA spread {:.2}%", adx_val, dir, ema_spread_pct);
        } else if adx_val < 20.0 && atr_pct < vol_threshold {
            regime = Regime::Ranging;
            reason = format!("ADX {:.1} below 20, ATR% {:.2} calm", adx_val, atr_pct);
        } else {
            // Ambiguous zone (ADX 20–25): default to trend-following (safer)
            regime = Regime::Trending;
            reason = format!("ADX {:.1} ambiguous zone → default trending", adx_val);
        }

        (regime, adx_val, atr_pct, reason)
    }
}
