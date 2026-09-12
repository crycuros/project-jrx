//! Technical indicators implemented in pure Rust.

/// Simple moving average over a window.
/// Returns a Vec aligned with input; leading values are None.
pub fn sma(values: &[f64], period: usize) -> Vec<Option<f64>> {
    let mut out = vec![None; values.len()];
    if period == 0 {
        return out;
    }
    let mut sum = 0.0;
    for i in 0..values.len() {
        sum += values[i];
        if i >= period {
            sum -= values[i - period];
        }
        if i >= period - 1 {
            out[i] = Some(sum / period as f64);
        }
    }
    out
}

/// Exponential moving average.
pub fn ema(values: &[f64], period: usize) -> Vec<Option<f64>> {
    let mut out = vec![None; values.len()];
    if period == 0 || values.is_empty() {
        return out;
    }
    let alpha = 2.0 / (period as f64 + 1.0);
    let mut prev = values[0];
    out[0] = Some(prev);
    for i in 1..values.len() {
        prev = alpha * values[i] + (1.0 - alpha) * prev;
        out[i] = Some(prev);
    }
    out
}

/// Relative Strength Index (Wilder's smoothing).
pub fn rsi(values: &[f64], period: usize) -> Vec<Option<f64>> {
    let mut out = vec![None; values.len()];
    if values.len() < 2 {
        return out;
    }
    // RSI needs `period` deltas before it can start.
    let mut gains: f64 = 0.0;
    let mut losses: f64 = 0.0;
    for i in 1..=period.min(values.len() - 1) {
        let change = values[i] - values[i - 1];
        if change >= 0.0 {
            gains += change;
        } else {
            losses -= change;
        }
    }
    if period > 0 && values.len() > period {
        gains /= period as f64;
        losses /= period as f64;
        let rs = if losses == 0.0 { 100.0 } else { gains / losses };
        out[period] = Some(100.0 - (100.0 / (1.0 + rs)));
    }
    for i in (period + 1)..values.len() {
        let change = values[i] - values[i - 1];
        let gain = if change > 0.0 { change } else { 0.0 };
        let loss = if change < 0.0 { -change } else { 0.0 };
        gains = (gains * (period as f64 - 1.0) + gain) / period as f64;
        losses = (losses * (period as f64 - 1.0) + loss) / period as f64;
        let rs = if losses == 0.0 { 100.0 } else { gains / losses };
        out[i] = Some(100.0 - (100.0 / (1.0 + rs)));
    }
    out
}

/// Average True Range (Wilder's smoothing).
pub fn atr(highs: &[f64], lows: &[f64], closes: &[f64], period: usize) -> Vec<Option<f64>> {
    let mut out = vec![None; closes.len()];
    if closes.len() < period + 1 {
        return out;
    }
    let mut trs: Vec<f64> = Vec::with_capacity(closes.len());
    trs.push(highs[0] - lows[0]);
    for i in 1..closes.len() {
        let tr = (highs[i] - lows[i])
            .max((highs[i] - closes[i - 1]).abs())
            .max((lows[i] - closes[i - 1]).abs());
        trs.push(tr);
    }
    let mut avg: f64 = trs[..period].iter().sum::<f64>() / period as f64;
    out[period - 1] = Some(avg);
    for i in period..trs.len() {
        avg = (avg * (period as f64 - 1.0) + trs[i]) / period as f64;
        out[i] = Some(avg);
    }
    out
}

/// MACD (12,26,9).
pub fn macd(closes: &[f64]) -> (Vec<Option<f64>>, Vec<Option<f64>>, Vec<Option<f64>>) {
    let ema12 = ema(closes, 12);
    let ema26 = ema(closes, 26);
    let mut line = vec![None; closes.len()];
    for i in 0..closes.len() {
        if let (Some(a), Some(b)) = (ema12[i], ema26[i]) {
            line[i] = Some(a - b);
        }
    }
    // signal = EMA9 of macd line
    let flat: Vec<f64> = line.iter().map(|v| v.unwrap_or(0.0)).collect();
    let ema9 = ema(&flat, 9);
    let mut signal = vec![None; closes.len()];
    let mut hist = vec![None; closes.len()];
    for i in 0..closes.len() {
        if let (Some(l), Some(s)) = (line[i], ema9[i]) {
            signal[i] = Some(s);
            hist[i] = Some(l - s);
        }
    }
    (line, signal, hist)
}

/// Standard deviation (population) over window, used for volatility filter.
pub fn stddev(values: &[f64], period: usize) -> Vec<Option<f64>> {
    let mut out = vec![None; values.len()];
    if period == 0 {
        return out;
    }
    for i in 0..values.len() {
        if i + 1 < period {
            continue;
        }
        let start = i + 1 - period;
        let window = &values[start..=i];
        let mean = window.iter().sum::<f64>() / period as f64;
        let var = window.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / period as f64;
        out[i] = Some(var.sqrt());
    }
    out
}