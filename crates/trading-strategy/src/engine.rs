//! Strategy engine: combines indicators into trade proposals.

use trading_core::{Candle, Market, OrderSide, Proposal};

use crate::indicators::{atr, ema, macd, rsi};

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
    /// Analyze a candle series for one asset and produce a proposal if a signal fires.
    pub fn process(&self, candles: &[Candle], symbol: &str, _market: Market) -> Option<Proposal> {
        let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
        let highs: Vec<f64> = candles.iter().map(|c| c.high).collect();
        let lows: Vec<f64> = candles.iter().map(|c| c.low).collect();

        if closes.len() < 40 {
            return None;
        }

        let n = closes.len();
        let last = closes[n - 1];
        let rsi_vals = rsi(&closes, 14);
        let ema_fast = ema(&closes, 12);
        let ema_slow = ema(&closes, 26);
        let (macd_line, macd_signal, macd_hist) = macd(&closes);
        let atr_vals = atr(&highs, &lows, &closes, 14);

        let r = rsi_vals[n - 1].unwrap_or(50.0);
        let prev_r = rsi_vals[n - 2].unwrap_or(50.0);
        let ef = ema_fast[n - 1].unwrap_or(last);
        let es = ema_slow[n - 1].unwrap_or(last);
        let _macd_line = macd_line[n - 1].unwrap_or(0.0);
        let m_hist = macd_hist[n - 1].unwrap_or(0.0);
        let prev_hist = macd_hist[n - 2].unwrap_or(0.0);
        let atr_val = atr_vals[n - 1].unwrap_or(last * 0.01);
        let macd_signal_last = macd_signal[n - 1].unwrap_or(0.0);

        // Regime detection
        let regime = if ef > es { "bullish_trend" } else { "bearish_trend" };

        // Buy signal
        if (prev_r < 30.0 && r > 30.0)
            || (m_hist > 0.0 && prev_hist <= 0.0 && r < 65.0)
            || (m_hist > 0.0 && macd_signal_last < 0.0)
        {
            let entry = last;
            // Price-based stop: 1.5x ATR below entry
            let stop = (entry - atr_val * 1.5).max(0.0).min(entry);
            let target = entry + atr_val * 2.5;
            let confidence = confidence_score(r, 50.0, true, m_hist);
            return Some(Proposal {
                symbol: symbol.to_string(),
                action: OrderSide::Buy,
                entry,
                stop_loss: stop,
                take_profit: target,
                risk_reward: if entry - stop > 0.0 { (target - entry) / (entry - stop) } else { 0.0 },
                confidence,
                regime: regime.to_string(),
                reason: format!("MACD/RSI cross + {}", regime),
            });
        }

        // Sell signal (crossover)
        if (prev_r > 70.0 && r < 70.0) || (m_hist < 0.0 && prev_hist >= 0.0 && r > 35.0) {
            let entry = last;
            let stop = entry + atr_val * 1.5;
            let target = entry - atr_val * 2.5;
            let confidence = confidence_score(r, 50.0, false, m_hist);
            return Some(Proposal {
                symbol: symbol.to_string(),
                action: OrderSide::Sell,
                entry,
                stop_loss: stop,
                take_profit: target,
                risk_reward: if stop - entry > 0.0 { (entry - target) / (stop - entry) } else { 0.0 },
                confidence,
                regime: regime.to_string(),
                reason: format!("MACD/RSI cross + {}", regime),
            });
        }

        // Persistent-strong momentum fallback: fires far more often than pure
        // crossovers, so the engine actually opens positions in trending markets.
        if ef > es && m_hist > 0.0 && (45.0..72.0).contains(&r) {
            let entry = last;
            let stop = (entry - atr_val * 1.5).max(0.0).min(entry);
            let target = entry + atr_val * 2.5;
            let confidence = confidence_score(r, 50.0, true, m_hist);
            return Some(Proposal {
                symbol: symbol.to_string(),
                action: OrderSide::Buy,
                entry,
                stop_loss: stop,
                take_profit: target,
                risk_reward: if entry - stop > 0.0 { (target - entry) / (entry - stop) } else { 0.0 },
                confidence,
                regime: regime.to_string(),
                reason: format!("trend momentum {} (buy)", regime),
            });
        }
        if ef < es && m_hist < 0.0 && (28.0..55.0).contains(&r) {
            let entry = last;
            let stop = entry + atr_val * 1.5;
            let target = entry - atr_val * 2.5;
            let confidence = confidence_score(r, 50.0, false, m_hist);
            return Some(Proposal {
                symbol: symbol.to_string(),
                action: OrderSide::Sell,
                entry,
                stop_loss: stop,
                take_profit: target,
                risk_reward: if stop - entry > 0.0 { (entry - target) / (stop - entry) } else { 0.0 },
                confidence,
                regime: regime.to_string(),
                reason: format!("trend momentum {} (sell)", regime),
            });
        }

        None
    }

    /// Market-aware quality scoring (lightweight port of TradeQualityScorer).
    pub fn score_quality(&self, side: OrderSide, candles: &[Candle], symbol: &str, market: Market) -> SignalQuality {
        let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
        if closes.len() < 30 {
            return SignalQuality::new(0.0);
        }
        let n = closes.len();
        let r = rsi(&closes, 14)[n - 1].unwrap_or(50.0);

        let mut score: f64 = 50.0;
        let mut reasons = Vec::new();

        // Trend alignment (multi-timeframe proxy: EMA slope)
        let ema5 = ema(&closes, 5)[n - 1].unwrap_or(closes[n - 1]);
        let ema20 = ema(&closes, 20)[n - 1].unwrap_or(closes[n - 1]);
        let trending_up = ema5 > ema20;
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

        // Volatility sanity (ATR% bounds)
        let last = closes[n - 1];
        let atr_val = atr(
            &candles.iter().map(|c| c.high).collect::<Vec<_>>(),
            &candles.iter().map(|c| c.low).collect::<Vec<_>>(),
            &closes,
            14,
        )[n - 1].unwrap_or(0.0);
        let atr_pct = if last > 0.0 { atr_val / last } else { 0.0 };
        if !(0.0005..0.08).contains(&atr_pct) {
            score -= 15.0;
            reasons.push("volatility".to_string());
        }

        // Gold scalper boost
        if market == Market::Gold && scale_gold(symbol) {
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

fn confidence_score(rsi_val: f64, center: f64, buying: bool, macd_hist: f64) -> f64 {
    let mut c = 0.60 + (macd_hist.abs() / 1000.0).min(0.25);
    // RSI momentum
    let dist = (rsi_val - center).abs() / 50.0;
    c += dist * 0.15;
    if buying && rsi_val > 70.0 {
        c *= 0.8; // overbought penalty
    }
    if !buying && rsi_val < 30.0 {
        c *= 0.8;
    }
    c.clamp(0.1, 0.95)
}

fn grade_of(score: f64) -> String {
    match score as i32 {
        80.. => "A".to_string(),
        70.. => "B".to_string(),
        60.. => "C".to_string(),
        _ => "D".to_string(),
    }
}

fn scale_gold(symbol: &str) -> bool {
    matches!(symbol, "XAU_USD" | "XAUUSD" | "XAU/USD")
}