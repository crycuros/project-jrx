//! Intraday walk-forward backtest on real gold 1-minute history.
//!
//! Streams `XAU_1m_data.jsonl` (6.6M rows, 2004-2025), resamples to the target
//! timeframe, then walks the REAL StrategyEngine through a chronological split.
//!
//! Unlike the daily backtest this models the costs that actually exist on a
//! Deriv multiplier: bid/ask spread and stake commission.
//!
//! Usage:
//!   cargo run -p trading-backtest --release --bin intraday -- --tf 5
//!   cargo run -p trading-backtest --release --bin intraday -- --tf 15 --verbose

use std::io::{BufRead, BufReader};

use anyhow::{Context, Result};
use trading_core::{Candle, Market, OrderSide};
use trading_strategy::StrategyEngine;

// ── Cost model (Deriv frxXAUUSD multiplier) ────────────────────────────────
/// Commission charged on stake by Deriv multipliers.
const COMMISSION_FRAC: f64 = 0.0005;

#[derive(Default)]
struct Leg {
    label: String,
    trades: usize,
    wins: usize,
    gross_win: f64,
    gross_loss: f64,
    peak: f64,
    equity: f64,
    max_dd: f64,
    returns: Vec<f64>,
}

impl Leg {
    fn new(label: &str) -> Self {
        Self {
            label: label.to_string(),
            peak: 1.0,
            equity: 1.0,
            ..Default::default()
        }
    }

    fn record(&mut self, r: f64) {
        self.trades += 1;
        self.equity += r;
        if r > 0.0 {
            self.wins += 1;
            self.gross_win += r;
        } else {
            self.gross_loss += -r;
        }
        if self.equity > self.peak {
            self.peak = self.equity;
        }
        let dd = self.peak - self.equity;
        if dd > self.max_dd {
            self.max_dd = dd;
        }
        self.returns.push(r);
    }

    fn win_rate(&self) -> f64 {
        if self.trades == 0 {
            0.0
        } else {
            self.wins as f64 / self.trades as f64 * 100.0
        }
    }

    fn pf(&self) -> f64 {
        if self.gross_loss <= f64::EPSILON {
            f64::INFINITY
        } else {
            self.gross_win / self.gross_loss
        }
    }

    fn expectancy(&self) -> f64 {
        if self.trades == 0 {
            0.0
        } else {
            self.returns.iter().sum::<f64>() / self.trades as f64
        }
    }

    fn sharpe(&self) -> f64 {
        let n = self.returns.len();
        if n < 2 {
            return 0.0;
        }
        let m = self.expectancy();
        let var = self.returns.iter().map(|r| (r - m).powi(2)).sum::<f64>() / (n - 1) as f64;
        let sd = var.sqrt();
        if sd <= 1e-12 {
            0.0
        } else {
            m / sd * (252.0_f64 * 96.0).sqrt()
        }
    }

    fn max_streak(&self) -> usize {
        let (mut cur, mut best) = (0, 0);
        for r in &self.returns {
            if *r < 0.0 {
                cur += 1;
                best = best.max(cur);
            } else {
                cur = 0;
            }
        }
        best
    }

    fn print(&self) {
        println!("  ┌─ {} ────────────────────────────────", self.label);
        println!("  │ trades        : {}", self.trades);
        println!("  │ win rate      : {:.2}%", self.win_rate());
        println!("  │ expectancy    : {:+.5} R/trade", self.expectancy());
        println!("  │ profit factor : {:.4}", self.pf());
        println!("  │ Sharpe (ann.) : {:+.2}", self.sharpe());
        println!("  │ max drawdown  : {:.3} R", self.max_dd);
        println!("  │ max los streak: {}", self.max_streak());
        println!(
            "  │ net R         : {:+.3}",
            self.returns.iter().sum::<f64>()
        );
        println!("  └───────────────────────────────────────");
    }
}

/// Parse `"2004.06.11 07:18"` into a unix timestamp, treating it as UTC.
fn parse_ts(d: &str) -> Option<i64> {
    let b = d.as_bytes();
    if b.len() < 16 {
        return None;
    }
    let n = |a: usize, z: usize| -> Option<i64> { d.get(a..z)?.trim().parse().ok() };
    let year = n(0, 4)?;
    let month = n(5, 7)?;
    let day = n(8, 10)?;
    let hour = n(11, 13)?;
    let min = n(14, 16)?;
    let (y, m, dd) = (year, month, day);
    // Days-from-civil (Howard Hinnant's algorithm).
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = if y2 >= 0 { y2 } else { y2 - 399 } / 400;
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + dd - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hour * 3600 + min * 60)
}

/// Pull one field out of a tiny JSON object without a full parser.
/// Strips surrounding quotes so string values parse directly.
fn field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let pat = format!("\"{key}\":");
    let i = line.find(&pat)? + pat.len();
    let rest = &line[i..];
    let end = rest
        .find(|c: char| c == ',' || c == '}')
        .unwrap_or(rest.len());
    Some(rest[..end].trim().trim_matches('"'))
}

fn num(line: &str, key: &str) -> Option<f64> {
    field(line, key)?.parse().ok()
}

/// Stream the JSONL file and resample straight into `tf`-minute candles.
fn load_resampled(path: &str, tf_secs: i64) -> Result<Vec<Candle>> {
    let f = std::fs::File::open(path).with_context(|| format!("open {path}"))?;
    let rd = BufReader::with_capacity(1 << 20, f);

    let mut out: Vec<Candle> = Vec::with_capacity(2_000_000);
    let mut cur_bucket = i64::MIN;
    let mut cur = Candle {
        time: 0,
        open: 0.0,
        high: 0.0,
        low: 0.0,
        close: 0.0,
        volume: 0.0,
    };

    for line in rd.lines() {
        let Ok(line) = line else { continue };
        if !line.starts_with('{') {
            continue;
        }
        let (Some(date), Some(o), Some(h), Some(l), Some(c)) = (
            field(&line, "Date"),
            num(&line, "Open"),
            num(&line, "High"),
            num(&line, "Low"),
            num(&line, "Close"),
        ) else {
            continue;
        };
        let Some(ts) = parse_ts(date) else { continue };
        if o <= 0.0 || h <= 0.0 || l <= 0.0 || c <= 0.0 {
            continue;
        }
        let v = num(&line, "Volume").unwrap_or(0.0);

        let bucket = ts - ts.rem_euclid(tf_secs);
        if bucket != cur_bucket {
            if cur_bucket != i64::MIN {
                out.push(cur);
            }
            cur_bucket = bucket;
            cur = Candle {
                time: bucket * 1000,
                open: o,
                high: h,
                low: l,
                close: c,
                volume: v,
            };
        } else {
            if h > cur.high {
                cur.high = h;
            }
            if l < cur.low {
                cur.low = l;
            }
            cur.close = c;
            cur.volume += v;
        }
    }
    if cur_bucket != i64::MIN {
        out.push(cur);
    }
    Ok(out)
}

fn simulate(
    p_entry: f64,
    sl: f64,
    tp: f64,
    side: OrderSide,
    candles: &[Candle],
    entry_idx: usize,
    max_hold: usize,
    spread_frac: f64,
) -> (f64, &'static str) {
    let fill = if candles[entry_idx].open > 0.0 {
        candles[entry_idx].open
    } else {
        p_entry
    };
    let risk = (p_entry - sl).abs();
    if risk <= 0.0 {
        return (0.0, "no-risk");
    }

    // Costs charged once, on entry.
    let cost_r = (fill * spread_frac) / risk + COMMISSION_FRAC;

    let end = (entry_idx + max_hold).min(candles.len());
    let mut last_close = fill;

    for k in entry_idx..end {
        let b = &candles[k];
        last_close = b.close;

        // Stop is checked first — if both print inside one bar we assume the
        // adverse fill, which is the pessimistic assumption.
        let sl_hit = match side {
            OrderSide::Buy => b.low <= sl,
            OrderSide::Sell => b.high >= sl,
        };
        if sl_hit {
            return (-(1.0 + cost_r), "stop");
        }

        let tp_hit = match side {
            OrderSide::Buy => b.high >= tp,
            OrderSide::Sell => b.low <= tp,
        };
        if tp_hit {
            let gross = match side {
                OrderSide::Buy => (tp - fill) / risk,
                OrderSide::Sell => (fill - tp) / risk,
            };
            return ((gross - cost_r).max(-1.5), "target");
        }
    }

    // Neither level hit inside the window — close at the last bar.
    let gross = match side {
        OrderSide::Buy => (last_close - fill) / risk,
        OrderSide::Sell => (fill - last_close) / risk,
    };
    ((gross - cost_r).max(-1.5), "timeout")
}

fn run(
    engine: &StrategyEngine,
    candles: &[Candle],
    symbol: &str,
    market: Market,
    lo: usize,
    hi: usize,
    label: &str,
    verbose: bool,
    max_hold: usize,
    spread_frac: f64,
) -> Leg {
    let mut leg = Leg::new(label);
    let warmup = 40;
    let mut i = lo;
    while i + 1 < hi {
        let w = i.saturating_sub(warmup);
        let window = &candles[w..=i];
        if window.len() >= warmup {
            if let Some(p) = engine.process(window, symbol, market) {
                if p.stop_loss > 0.0 && p.take_profit > 0.0 && p.risk_reward >= 1.5 {
                    let (r, why) = simulate(
                        p.entry,
                        p.stop_loss,
                        p.take_profit,
                        p.action,
                        candles,
                        i + 1,
                        max_hold,
                        spread_frac,
                    );
                    leg.record(r);
                    if verbose {
                        println!(
                            "    {} {:5} conf={:.2} {:9} -> {:5} R={:+.4}",
                            chrono::DateTime::from_timestamp(candles[i].time / 1000, 0)
                                .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
                                .unwrap_or_default(),
                            p.action.as_str().to_uppercase(),
                            p.confidence,
                            p.regime,
                            why,
                            r
                        );
                    }
                }
            }
        }
        i += 1;
    }
    leg
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let tf: u32 = args
        .iter()
        .find_map(|a| a.strip_prefix("--tf="))
        .and_then(|v| v.parse().ok())
        .unwrap_or(5);
    let verbose = args.iter().any(|a| a == "--verbose");
    // Max bars to hold before force-closing. Default 24 bars ≈ 2 hours on 5m.
    let max_hold: usize = args
        .iter()
        .find_map(|a| a.strip_prefix("--hold="))
        .and_then(|v| v.parse().ok())
        .unwrap_or(24);
    let tf_secs = (tf as i64) * 60;
    let spread_frac: f64 = args
        .iter()
        .find_map(|a| a.strip_prefix("--spread="))
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0015);
    let sweep = args.iter().any(|a| a == "--sweep");

    let path = "trading_memory/gold_archive/XAU_1m_data.jsonl";
    if !std::path::Path::new(path).exists() {
        anyhow::bail!("missing {path}");
    }

    eprintln!("streaming + resampling to {tf}m ... (619 MB, one pass)");
    let candles = load_resampled(path, tf_secs)?;
    let n = candles.len();
    if n < 500 {
        anyhow::bail!("only {n} candles after resample");
    }

    let split = (n as f64 * 0.70) as usize;
    let engine = StrategyEngine;

    println!("{}", "═".repeat(72));
    println!("  INTRADAY WALK-FORWARD — XAU/USD  ·  timeframe {tf}m");
    println!("  costs: spread {:.3}% round-trip + commission {:.2}%", spread_frac * 100.0, COMMISSION_FRAC * 100.0);
    println!("{}", "═".repeat(72));
    println!("  candles : {n}");
    println!("  span    : {} .. {}", fmt_ts(candles[0].time), fmt_ts(candles[n - 1].time));
    println!("  split   : {split} train / {} test", n - split);
    println!("  hold    : up to {max_hold} bars (~{} min)", max_hold * tf as usize);

    let is = run(&engine, &candles, "XAU_USD", Market::Gold, 40, split, "IN-SAMPLE (train 70%)", verbose, max_hold, spread_frac);
    let oos = run(&engine, &candles, "XAU_USD", Market::Gold, split, n, "OUT-OF-SAMPLE (test 30%)", verbose, max_hold, spread_frac);

    // Sweep: how much cost can this strategy actually absorb before it dies?
    if sweep {
        println!("\n{}", "=".repeat(72));
        println!("  COST SENSITIVITY — out-of-sample, after each spread level");
        println!("{}", "=".repeat(72));
        println!("  {:>8}  {:>8}  {:>9}  {:>7}  {:>7}", "spread", "trades", "expectancy", "PF", "winrate");
        for pct in [0.0_f64, 0.0005, 0.0010, 0.0015, 0.0020, 0.0030, 0.0050] {
            let l = run(&engine, &candles, "XAU_USD", Market::Gold, split, n, "s", false, max_hold, pct);
            println!(
                "  {:>7.3}%  {:>8}  {:>+9.5}  {:>7.4}  {:>6.2}%",
                pct * 100.0, l.trades, l.expectancy(), l.pf(), l.win_rate()
            );
        }
        println!("\n  Break-even spread is where expectancy crosses zero.");
        println!("  If it only survives below the real spread, the strategy is dead.\n");
    }

    is.print();
    oos.print();

    println!("\n{}", "═".repeat(72));
    println!("  VERDICT");
    println!("{}", "═".repeat(72));

    let e = oos.expectancy();
    let pf = oos.pf();
    if oos.trades < 100 {
        println!("  ⚠  Only {} trades — too few for a verdict.", oos.trades);
    } else if e <= 0.0 {
        println!("  ✗  NEGATIVE expectancy out-of-sample: {e:+.5} R/trade over {} trades.", oos.trades);
        println!("     The edge does not survive unseen data, even after costs.");
    } else if pf < 1.05 {
        println!("  ~  Flat: {e:+.5} R/trade, PF {pf:.4} over {} trades.", oos.trades);
        println!("     Statistically indistinguishable from zero. Not tradeable.");
    } else if pf < 1.20 {
        println!("  ~  Weak positive: {e:+.5} R/trade, PF {pf:.4} over {} trades.", oos.trades);
        println!("     Thin margin — one bad month erases it.");
    } else {
        println!("  ✓  Meaningful: {e:+.5} R/trade, PF {pf:.4} over {} trades.", oos.trades);
        println!("     Re-verify across independent periods before any capital.");
    }
    println!("\n  Worst losing streak: {} consecutive.", oos.max_streak());
    println!("  Every R above is already net of a {:.3}% spread + {:.2}% commission.\n", spread_frac * 100.0, COMMISSION_FRAC * 100.0);

    Ok(())
}

fn fmt_ts(ms: i64) -> String {
    chrono::DateTime::from_timestamp(ms / 1000, 0)
        .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}