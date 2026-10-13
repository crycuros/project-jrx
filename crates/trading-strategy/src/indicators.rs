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

/// Bollinger Bands (middle = SMA, upper/lower = middle ± k*stddev).
/// Returns (middle, upper, lower) each aligned with input.
pub fn bollinger_bands(
    values: &[f64],
    period: usize,
    k: f64,
) -> (Vec<Option<f64>>, Vec<Option<f64>>, Vec<Option<f64>>) {
    let mid = sma(values, period);
    let sd  = stddev(values, period);
    let mut upper = vec![None; values.len()];
    let mut lower = vec![None; values.len()];
    for i in 0..values.len() {
        if let (Some(m), Some(s)) = (mid[i], sd[i]) {
            upper[i] = Some(m + k * s);
            lower[i] = Some(m - k * s);
        }
    }
    (mid, upper, lower)
}

/// Z-score of price relative to rolling mean and stddev.
/// z = (price - mean) / stddev
pub fn zscore(values: &[f64], period: usize) -> Vec<Option<f64>> {
    let means = sma(values, period);
    let sds   = stddev(values, period);
    let mut out = vec![None; values.len()];
    for i in 0..values.len() {
        if let (Some(m), Some(s)) = (means[i], sds[i]) {
            if s > 1e-12 {
                out[i] = Some((values[i] - m) / s);
            }
        }
    }
    out
}

/// Average Directional Index (Wilder's ADX, period typically 14).
/// Returns (adx, +di, -di) each aligned with input (leading values are None).
/// ADX > 25 = trending, ADX < 20 = ranging.
pub fn adx(
    highs:  &[f64],
    lows:   &[f64],
    closes: &[f64],
    period: usize,
) -> (Vec<Option<f64>>, Vec<Option<f64>>, Vec<Option<f64>>) {
    let n = closes.len();
    let mut adx_out  = vec![None; n];
    let mut pdi_out  = vec![None; n];
    let mut mdi_out  = vec![None; n];

    if n < period + 1 || period == 0 {
        return (adx_out, pdi_out, mdi_out);
    }

    let mut tr_arr  = vec![0.0f64; n];
    let mut pdm_arr = vec![0.0f64; n];
    let mut mdm_arr = vec![0.0f64; n];

    for i in 1..n {
        let tr = (highs[i] - lows[i])
            .max((highs[i] - closes[i - 1]).abs())
            .max((lows[i]  - closes[i - 1]).abs());
        tr_arr[i] = tr;
        let up   = highs[i] - highs[i - 1];
        let down = lows[i - 1] - lows[i];
        pdm_arr[i] = if up > down && up > 0.0 { up } else { 0.0 };
        mdm_arr[i] = if down > up && down > 0.0 { down } else { 0.0 };
    }

    // Wilder smoothing seed
    let mut atr_s  = tr_arr[1..=period].iter().sum::<f64>();
    let mut pdm_s  = pdm_arr[1..=period].iter().sum::<f64>();
    let mut mdm_s  = mdm_arr[1..=period].iter().sum::<f64>();

    let pf = period as f64;
    let pdi_seed = if atr_s > 0.0 { 100.0 * pdm_s / atr_s } else { 0.0 };
    let mdi_seed = if atr_s > 0.0 { 100.0 * mdm_s / atr_s } else { 0.0 };
    pdi_out[period] = Some(pdi_seed);
    mdi_out[period] = Some(mdi_seed);

    let dx_seed = if pdi_seed + mdi_seed > 0.0 {
        100.0 * (pdi_seed - mdi_seed).abs() / (pdi_seed + mdi_seed)
    } else { 0.0 };

    let mut dx_sum = dx_seed;
    let mut prev_adx = 0.0f64;

    for i in (period + 1)..n {
        atr_s = atr_s - atr_s / pf + tr_arr[i];
        pdm_s = pdm_s - pdm_s / pf + pdm_arr[i];
        mdm_s = mdm_s - mdm_s / pf + mdm_arr[i];

        let pdi = if atr_s > 0.0 { 100.0 * pdm_s / atr_s } else { 0.0 };
        let mdi = if atr_s > 0.0 { 100.0 * mdm_s / atr_s } else { 0.0 };
        pdi_out[i] = Some(pdi);
        mdi_out[i] = Some(mdi);

        let dx = if pdi + mdi > 0.0 { 100.0 * (pdi - mdi).abs() / (pdi + mdi) } else { 0.0 };

        // After first full period, switch to Wilder's EMA for ADX
        if i == period + 1 {
            dx_sum += dx;
            prev_adx = dx_sum / (period as f64 + 1.0);
        } else {
            prev_adx = (prev_adx * (pf - 1.0) + dx) / pf;
        }
        adx_out[i] = Some(prev_adx);
    }

    (adx_out, pdi_out, mdi_out)
}

/// Stochastic Oscillator (%K, %D).
/// %K = 100 * (close - lowest_low_k) / (highest_high_k - lowest_low_k)
/// %D = SMA(%K, d_period)
pub fn stochastic(
    highs:  &[f64],
    lows:   &[f64],
    closes: &[f64],
    k_period: usize,
    d_period: usize,
) -> (Vec<Option<f64>>, Vec<Option<f64>>) {
    let n = closes.len();
    let mut k_out = vec![None; n];
    if n < k_period {
        return (k_out, vec![None; n]);
    }
    for i in (k_period - 1)..n {
        let slice_h = &highs[i + 1 - k_period..=i];
        let slice_l = &lows[i + 1 - k_period..=i];
        let hh = slice_h.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let ll = slice_l.iter().cloned().fold(f64::INFINITY, f64::min);
        let range = hh - ll;
        k_out[i] = Some(if range > 0.0 { 100.0 * (closes[i] - ll) / range } else { 50.0 });
    }
    let k_flat: Vec<f64> = k_out.iter().map(|v| v.unwrap_or(50.0)).collect();
    let d_out = sma(&k_flat, d_period);
    (k_out, d_out)
}

/// Donchian Channel — highest high and lowest low over `period` bars.
/// Returns (upper, lower) — breakout signal when price closes above upper or below lower.
pub fn donchian(
    highs: &[f64],
    lows:  &[f64],
    period: usize,
) -> (Vec<Option<f64>>, Vec<Option<f64>>) {
    let n = highs.len();
    let mut upper = vec![None; n];
    let mut lower = vec![None; n];
    if period == 0 || n < period {
        return (upper, lower);
    }
    for i in (period - 1)..n {
        let slice_h = &highs[i + 1 - period..=i];
        let slice_l = &lows[i + 1 - period..=i];
        upper[i] = Some(slice_h.iter().cloned().fold(f64::NEG_INFINITY, f64::max));
        lower[i] = Some(slice_l.iter().cloned().fold(f64::INFINITY, f64::min));
    }
    (upper, lower)
}

/// Rolling Pearson correlation between two series over `period` bars.
/// Returns values in [-1, 1]. Used for pairs trading and correlation penalty in risk.
pub fn rolling_correlation(a: &[f64], b: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = a.len().min(b.len());
    let mut out = vec![None; n];
    if period < 2 || n < period {
        return out;
    }
    for i in (period - 1)..n {
        let sa = &a[i + 1 - period..=i];
        let sb = &b[i + 1 - period..=i];
        let ma: f64 = sa.iter().sum::<f64>() / period as f64;
        let mb: f64 = sb.iter().sum::<f64>() / period as f64;
        let mut cov = 0.0f64;
        let mut va  = 0.0f64;
        let mut vb  = 0.0f64;
        for j in 0..period {
            let da = sa[j] - ma;
            let db = sb[j] - mb;
            cov += da * db;
            va  += da * da;
            vb  += db * db;
        }
        let denom = (va * vb).sqrt();
        out[i] = Some(if denom > 1e-12 { (cov / denom).clamp(-1.0, 1.0) } else { 0.0 });
    }
    out
}

/// Highest high over rolling window (used for breakout detection).
pub fn rolling_high(highs: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = highs.len();
    let mut out = vec![None; n];
    for i in (period.saturating_sub(1))..n {
        let start = i + 1 - period.min(i + 1);
        out[i] = Some(highs[start..=i].iter().cloned().fold(f64::NEG_INFINITY, f64::max));
    }
    out
}

/// Lowest low over rolling window (used for breakout detection).
pub fn rolling_low(lows: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = lows.len();
    let mut out = vec![None; n];
    for i in (period.saturating_sub(1))..n {
        let start = i + 1 - period.min(i + 1);
        out[i] = Some(lows[start..=i].iter().cloned().fold(f64::INFINITY, f64::min));
    }
    out
}