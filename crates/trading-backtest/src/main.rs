//! Walk-forward backtest over the mined daily memory.
//!
//! Runs the REAL `StrategyEngine` (no reimplementation) over the daily bars
//! already cached in `data/market_memory_cache.json`, using a chronological
//! split so the reported out-of-sample numbers contain no look-ahead.
//!
//! Usage:
//!   cargo run -p trading-backtest --release
//!   cargo run -p trading-backtest --release -- --symbol EURUSD

use std::collections::HashMap;

const RULE: &str = "═";

use anyhow::{Context, Result};
use trading_core::{Candle, Market, OrderSide};
use trading_strategy::StrategyEngine;

/// Mirrors `trading_ai::history::DailyBar` (same serde shape as the cache file).
#[derive(Debug, Clone, serde::Deserialize)]
struct DailyBar {
    day: i64,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    #[allow(dead_code)]
    volume: f64,
}

/// Cumulative results for one leg of the split.
#[derive(Debug, Default, Clone)]
struct Leg {
    label: String,
    trades: usize,
    wins: usize,
    losses: usize,
    gross_win: f64,
    gross_loss: f64,
    max_drawdown: f64,
    peak_equity: f64,
    equity: f64,
    /// R-multiple of each trade (1.0 = risked 1 unit).
    returns: Vec<f64>,
}

impl Leg {
    fn new(label: &str, start: f64) -> Self {
        Self {
            label: label.to_string(),
            peak_equity: start,
            equity: start,
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
            self.losses += 1;
            self.gross_loss += -r;
        }
        if self.equity > self.peak_equity {
            self.peak_equity = self.equity;
        }
        let dd = self.peak_equity - self.equity;
        if dd > self.max_drawdown {
            self.max_drawdown = dd;
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

    /// Mean win divided by mean loss — >1 means the edge pays for its losses.
    fn profit_factor(&self) -> f64 {
        if self.gross_loss <= f64::EPSILON {
            if self.gross_win > 0.0 {
                f64::INFINITY
            } else {
                0.0
            }
        } else {
            self.gross_win / self.gross_loss
        }
    }

    fn expectancy(&self) -> f64 {
        if self.trades == 0 {
            0.0
        } else {
            let sum: f64 = self.returns.iter().sum();
            sum / self.trades as f64
        }
    }

    fn sharpe(&self) -> f64 {
        let n = self.returns.len();
        if n < 2 {
            return 0.0;
        }
        let mean = self.expectancy();
        let var = self.returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (n - 1) as f64;
        let sd = var.sqrt();
        if sd <= f64::EPSILON {
            0.0
        } else {
            mean / sd * (252.0f64).sqrt()
        }
    }

    /// Longest run of consecutive losses — the kill-switch stress test.
    fn max_losing_streak(&self) -> usize {
        let mut cur = 0;
        let mut best = 0;
        for r in &self.returns {
            if *r < 0.0 {
                cur += 1;
                if cur > best {
                    best = cur;
                }
            } else {
                cur = 0;
            }
        }
        best
    }

    fn print(&self) {
        println!("\n  ┌─ {} ─────────────────────────────", self.label);
        println!("  │ trades          : {}", self.trades);
        println!("  │ wins / losses   : {} / {}", self.wins, self.losses);
        println!("  │ win rate        : {:.1}%", self.win_rate());
        println!("  │ expectancy (R)  : {:+.4}", self.expectancy());
        println!("  │ profit factor   : {:.3}", self.profit_factor());
        println!("  │ max drawdown (R): {:.3}", self.max_drawdown);
        println!("  │ max los streak  : {}", self.max_losing_streak());
        println!("  │ Sharpe (ann.)   : {:+.2}", self.sharpe());
        println!(
            "  │ net R           : {:+.3}  (1.00 = one full stake risked per trade)",
            self.returns.iter().sum::<f64>()
        );
        println!("  └─────────────────────────────────────");
    }
}

fn bars_to_candles(bars: &[DailyBar]) -> Vec<Candle> {
    bars.iter()
        .map(|b| Candle {
            time: b.day * 86_400_000,
            open: b.open,
            high: b.high,
            low: b.low,
            close: b.close,
            volume: b.volume,
        })
        .collect()
}

/// Outcome of one simulated trade expressed in R (risk units).
///
/// Fill is assumed at the NEXT bar's open — you cannot fill at the close that
/// produced the signal. SL/TP are checked against that bar's range; if neither
/// is touched the trade closes at the bar close.
fn simulate(p_entry: f64, sl: f64, tp: f64, side: OrderSide, bar: &Candle) -> (f64, &'static str) {
    let fill = if bar.open > 0.0 { bar.open } else { p_entry };

    // One unit of risk = the distance from entry to the stop. Using this
    // (rather than |fill - entry|, which collapses when the next bar opens
    // near the signal price) keeps R multiples in a sane range.
    let risk = (p_entry - sl).abs();
    if risk <= 0.0 {
        return (0.0, "no-risk-defined");
    }

    let (sl_hit, tp_hit) = match side {
        OrderSide::Buy => (bar.low <= sl, bar.high >= tp),
        OrderSide::Sell => (bar.high >= sl, bar.low <= tp),
    };

    // Both in one bar is ambiguous; assume the stop fills first (pessimistic).
    if sl_hit {
        return (-1.0, "stop-loss");
    }
    if tp_hit {
        let r = match side {
            OrderSide::Buy => (tp - fill) / risk,
            OrderSide::Sell => (fill - tp) / risk,
        };
        return (r.max(-1.0), "take-profit");
    }
    let r = match side {
        OrderSide::Buy => (bar.close - fill) / risk,
        OrderSide::Sell => (fill - bar.close) / risk,
    };
    (r.max(-1.0), "bar-close")
}

/// Walk one slice of candles through the live strategy engine.
fn run_leg(
    engine: &StrategyEngine,
    candles: &[Candle],
    symbol: &str,
    market: Market,
    start_idx: usize,
    end_idx: usize,
    label: &str,
    verbose: bool,
) -> Leg {
    let mut leg = Leg::new(label, 1.0);
    let warmup = 40; // StrategyEngine::process_multi requires >= 40 candles

    let mut i = start_idx;
    while i + 1 < end_idx {
        let lo = i.saturating_sub(warmup);
        let window = &candles[lo..=i];

        if window.len() >= warmup {
            if let Some(p) = engine.process(window, symbol, market) {
                if p.stop_loss > 0.0 && p.take_profit > 0.0 {
                    let next = &candles[i + 1];
                    let (r, why) = simulate(p.entry, p.stop_loss, p.take_profit, p.action, next);
                    leg.record(r);
                    if verbose {
                        println!(
                            "    {} {}  conf={:.2} regime={:<9} -> {:<12} R={:+.3}",
                            ts(i), p.action.as_str().to_uppercase(), p.confidence, p.regime, why, r
                        );
                    }
                }
            }
        }
        i += 1;
    }
    leg
}

fn ts(idx: usize) -> String {
    format!("bar#{}", idx)
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let filter = args.get(1).and_then(|s| s.strip_prefix("--symbol="));
    let verbose = args.iter().any(|a| a == "--verbose");

    let cache_path = std::path::Path::new("data/market_memory_cache.json");
    anyhow::ensure!(
        cache_path.exists(),
        "missing {} — run the desktop app once to build it, or point at the repo root",
        cache_path.display()
    );

    let bytes = std::fs::read(cache_path).context("read market_memory_cache.json")?;
    let daily: HashMap<String, Vec<DailyBar>> =
        serde_json::from_slice(&bytes).context("parse cache as HashMap<String, Vec<DailyBar>>")?;

    println!("{}", RULE.repeat(74));
    println!("  WALK-FORWARD BACKTEST — chronological 70/30 split");
    println!("  engine: the live StrategyEngine (trend/mean_rev/breakout/multi)");
    println!("{}", RULE.repeat(74));

    let engine = StrategyEngine;
    let mut all_is: Leg = Leg::new("IN-SAMPLE (train 70%) — contaminated, reference only", 1.0);
    let mut all_oos: Leg = Leg::new("OUT-OF-SAMPLE (test 30%) — THE NUMBER", 1.0);
    let mut symbols: Vec<&String> = daily.keys().collect();
    symbols.sort();

    for sym in symbols {
        let bars = &daily[sym];
        if filter.map(|f| f != sym).unwrap_or(false) {
            continue;
        }
        if bars.len() < 200 {
            println!("\n  {sym}: only {} bars — skipping", bars.len());
            continue;
        }

        let candles = bars_to_candles(bars);
        let n = candles.len();
        let split = (n as f64 * 0.70) as usize;

        let market = if sym.contains("XAU") { Market::Gold } else { Market::Forex };

        let is_leg = run_leg(
            &engine, &candles, sym, market, 40, split, &format!("{sym} in-sample"), verbose,
        );
        let oos_leg = run_leg(
            &engine, &candles, sym, market, split, n, &format!("{sym} out-of-sample"), verbose,
        );

        println!("\n  ── {sym} ── {} bars ({} train / {} test)",
            n, split - 40, n - split);
        println!("     span: {} .. {}", day_str(bars[0].day), day_str(bars[n - 1].day));
        is_leg.print();
        oos_leg.print();

        for r in &is_leg.returns {
            all_is.record(*r);
        }
        for r in &oos_leg.returns {
            all_oos.record(*r);
        }
    }

    if all_oos.trades == 0 {
        println!("\n  No trades generated — the gate produced zero signals.");
        return Ok(());
    }

    println!("\n{}", RULE.repeat(74));
    println!("  POOLED");
    all_is.print();
    all_oos.print();

    println!("\n{}", RULE.repeat(74));
    println!("  VERDICT");
    println!("{}", RULE.repeat(74));
    let wf = all_oos.win_rate();
    let pf = all_oos.profit_factor();
    let exp = all_oos.expectancy();
    let streak = all_oos.max_losing_streak();

    if all_oos.trades < 30 {
        println!("  ⚠  Only {} out-of-sample trades — too few to conclude anything.", all_oos.trades);
        println!("     Widen the symbol list or accept this is inconclusive.");
    } else if exp <= 0.0 {
        println!("  ✗  NEGATIVE expectancy out-of-sample ({:+.4} R/trade).", exp);
        println!("     The edge does not survive unseen data. Do not trade this.");
    } else if pf < 1.0 {
        println!("  ✗  Profit factor {:.3} < 1.0 — losing after costs.", pf);
    } else if wf < 55.0 {
        println!("  ~  Positive but weak: {:.1}% win rate, PF {:.3}, {exp:+.4} R/trade.", wf, pf);
        println!("     Viable in theory, far too thin to fund real capital.");
    } else {
        println!("  ✓  Positive out-of-sample: {:.1}% win rate, PF {:.3}, {exp:+.4} R/trade.", wf, pf);
        println!("     Still verify across more periods before risking money.");
    }

    println!("\n  Worst case: {} consecutive losses in the test window.", streak);
    if streak >= 5 {
        println!("     ⚠  Your MAX_CONSECUTIVE_LOSSES=5 kill switch would trip.");
        println!("        At $50 stake that is -${} before the bot stops.", streak as f64 * 50.0);
    }
    println!("\n  Reminder: no fees, no slippage, no spread modelled. Real results are worse.\n");

    Ok(())
}

fn day_str(day: i64) -> String {
    chrono::DateTime::from_timestamp(day * 86_400, 0)
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}