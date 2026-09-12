//! HistoricalDataCopy historical memory — builds 2010→2026 daily bars (from the 5-min
//! CSV exports), mines persistent trading patterns, and stores the result as a
//! JSON strategy file that the AI + engine can consume.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use log::{info, warn};

use serde::{Deserialize, Serialize};
use chrono::Datelike;

use trading_core::{Config, Market};

/// Daily aggregation of the HistoricalDataCopy dump for one symbol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyBar {
    /// Days since the Unix epoch of the bar's open (00:00 UTC).
    pub day: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

/// One mined statistical pattern.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pattern {
    /// e.g. "weekday-mon", "month-jan", "trend-persistence"
    pub key: String,
    pub edge_pct: f64,
    pub win_rate: f64,
    pub samples: usize,
    /// human-readable description of the rule
    pub rule: String,
}

/// The mined strategy for one symbol — persisted to historicaldatacopy_strategy.json.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageStrategy {
    pub symbol: String,
    pub span_start: String,
    pub span_end: String,
    pub bars: usize,
    pub all_time_high: f64,
    pub all_time_low: f64,
    pub bullish_days_pct: f64,
    pub avg_daily_range_pct: f64,
    pub momentum_persistence: f64,
    pub best_weekday: String,
    pub worst_weekday: String,
    pub best_month: String,
    pub worst_month: String,
    pub patterns: Vec<Pattern>,
}

/// Long-term market memory + mined strategy for the AI.
pub struct HistoricalDataCopyMemory {
    daily: HashMap<String, Vec<DailyBar>>,
    strategy: HashMap<String, MessageStrategy>,
}

const FILES: [(&str, &str); 4] = [
    ("EURUSD", "EURUSD_m5_20100101_20260101.csv"),
    ("GBPUSD", "GBPUSD_m5_20100101_20260101.csv"),
    ("USDJPY", "USDJPY_m5_20100101_20260101.csv"),
    ("XAUUSD", "XAUUSD_m5_20100101_20260101.csv"),
];

const CACHE: &str = "historicaldatacopy_memory.cache.json";
const STRAT: &str = "historicaldatacopy_strategy.json";

impl HistoricalDataCopyMemory {
    /// Load (or build + cache) the memory and mine/store the strategy.
    pub fn load(cfg: &Config) -> Self {
        let csv_dir = std::env::var("HISTORICALDATACOPY_DIR").unwrap_or_else(|_| dirs_download_dir());
        let cache = cfg.data_dir.join(CACHE);
        let strat_path = cfg.data_dir.join(STRAT);

        // 1) ensure daily cache exists
        if Path::new(&cache).exists() {
            match Self::load_cached(Path::new(&cache)) {
                Ok(mut m) => {
                    m.ensure_strategy(&strat_path);
                    info!(
                        "historicaldatacopy memory: loaded {} symbols / {} bars, strategy {} patterns",
                        m.daily.len(),
                        m.total_bars(),
                        m.total_patterns()
                    );
                    return m;
                }
                Err(e) => warn!("historicaldatacopy memory cache invalid: {e}"),
            }
        }
        let mut m = match Self::build(Path::new(&csv_dir), Path::new(&cache)) {
            Ok(m) => m,
            Err(e) => {
                warn!("historicaldatacopy memory build failed ({e}); running without history");
                return Self { daily: HashMap::new(), strategy: HashMap::new() };
            }
        };
        m.mine_and_store(&strat_path);
        info!(
            "historicaldatacopy memory: built {} symbols / {} bars / {} patterns",
            m.daily.len(),
            m.total_bars(),
            m.total_patterns()
        );
        m
    }

    fn ensure_strategy(&mut self, path: &Path) {
        if Path::new(path).exists() {
            if let Ok(mut s) = Self::load_strategy(path) {
                let missing = self.daily.keys().any(|k| !s.contains_key(k));
                if !missing {
                    self.strategy = std::mem::take(&mut s);
                    return;
                }
            }
        }
        self.mine_and_store(path);
    }

    fn mine_and_store(&mut self, path: &Path) {
        let mut strat: HashMap<String, MessageStrategy> = HashMap::new();
        for (sym, bars) in &self.daily {
            strat.insert(sym.clone(), mine(&sym.clone(), bars));
        }
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(bytes) = serde_json::to_vec(&strat) {
            let _ = std::fs::write(path, bytes);
            info!("historicaldatacopy strategy: wrote {}", path.display());
        }
        self.strategy = strat;
    }

    fn load_strategy(path: &Path) -> Result<HashMap<String, MessageStrategy>> {
        let bytes = std::fs::read(path)?;
        let s: HashMap<String, MessageStrategy> = serde_json::from_slice(&bytes)?;
        if s.is_empty() {
            return Err(anyhow!("empty strategy"));
        }
        Ok(s)
    }

    fn load_cached(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path)?;
        let daily: HashMap<String, Vec<DailyBar>> = serde_json::from_slice(&bytes)?;
        if daily.is_empty() {
            return Err(anyhow!("empty cache"));
        }
        Ok(Self { daily, strategy: HashMap::new() })
    }

    fn build(csv_dir: &Path, cache: &Path) -> Result<Self> {
        let mut daily: HashMap<String, Vec<DailyBar>> = HashMap::new();
        for (sym, file) in FILES {
            match aggregate_daily(csv_dir.join(file)) {
                Ok(bars) => {
                    info!("historicaldatacopy memory: {sym} -> {} daily bars", bars.len());
                    daily.insert(sym.to_string(), bars);
                }
                Err(e) => warn!("historicaldatacopy memory: skip {sym}: {e}"),
            }
        }
        if daily.is_empty() {
            return Err(anyhow!("no historicaldatacopy files parsed"));
        }
        if let Some(parent) = cache.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let bytes = serde_json::to_vec(&daily)?;
        std::fs::write(cache, bytes).with_context(|| "write historicaldatacopy cache")?;
        Ok(Self { daily, strategy: HashMap::new() })
    }

    pub fn strategy_for(&self, symbol: &str) -> Option<&MessageStrategy> {
        self.strategy.get(&map_symbol(symbol))
    }

    pub fn len_for(&self, symbol: &str) -> usize {
        self.daily.get(&map_symbol(symbol)).map(|v| v.len()).unwrap_or(0)
    }

    pub fn total_bars(&self) -> usize {
        self.daily.values().map(|v| v.len()).sum()
    }

    pub fn total_patterns(&self) -> usize {
        self.strategy.values().map(|s| s.patterns.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.daily.is_empty()
    }

    /// Compact decision context (memory summary) for the AI prompt.
    pub fn context(&self, symbol: &str) -> String {
        let Some(st) = self.strategy_for(symbol) else {
            return format!("HISTORY[{symbol}]: no cached historicaldatacopy data");
        };
        let mut out = format!(
            "HISTORY[{symbol}] historicaldatacopy {}..{} ({} daily bars)\n\
             allTime high={:.4} low={:.4} bullishDays={:.1}% avgDailyRange={:.4}%\n\
             persistence(win streak follow)={:.2}",
            st.span_start, st.span_end, st.bars, st.all_time_high, st.all_time_low,
            st.bullish_days_pct, st.avg_daily_range_pct, st.momentum_persistence,
        );
        if let Some(bw) = st.patterns.iter().find(|p| p.key == "weekday-best") {
            out.push_str(&format!("\nbestDay {} edge {:+.3}% win {:.0}%", bw.rule, bw.edge_pct, bw.win_rate * 100.0));
        }
        if let Some(ww) = st.patterns.iter().find(|p| p.key == "weekday-worst") {
            out.push_str(&format!("\nworstDay {} edge {:+.3}% win {:.0}%", ww.rule, ww.edge_pct, ww.win_rate * 100.0));
        }
        if let Some(bm) = st.patterns.iter().find(|p| p.key == "month-best") {
            out.push_str(&format!("\nbestMonth {} edge {:+.3}%", bm.rule, bm.edge_pct));
        }
        if let Some(wm) = st.patterns.iter().find(|p| p.key == "month-worst") {
            out.push_str(&format!("\nworstMonth {} edge {:+.3}%", wm.rule, wm.edge_pct));
        }
        for p in st.patterns.iter().filter(|p| matches!(p.key.as_str(), "friday-bias" | "monday-bias" | "range-expansion" | "gap-follow")) {
            out.push_str(&format!("\n{}: {:+.3}% win {:.0}% (n={})", p.rule, p.edge_pct, p.win_rate * 100.0, p.samples));
        }
        out
    }
}

/// Mine the strategy patterns from the daily bars and return the MessageStrategy.
fn mine(symbol: &str, bars: &[DailyBar]) -> MessageStrategy {
    let all_time_high = bars.iter().fold(f64::NEG_INFINITY, |a, b| a.max(b.high));
    let all_time_low = bars.iter().fold(f64::INFINITY, |a, b| a.min(b.low));
    let n = bars.len();
    let up_days = bars.iter().filter(|b| b.close > b.open).count();
    let bullish_days_pct = up_days as f64 / n as f64 * 100.0;
    let avg_range = bars.iter().map(|b| (b.high - b.low) / b.open * 100.0).sum::<f64>() / n as f64;

    // momentum persistence: day after an up-day tends to also be up?
    let mut both_up = 0usize;
    let mut up_then = 0usize;
    for w in bars.windows(2) {
        if w[0].close > w[0].open {
            up_then += 1;
            if w[1].close > w[1].open {
                both_up += 1;
            }
        }
    }
    let momentum_persistence = if up_then > 0 { both_up as f64 / up_then as f64 } else { 0.5 };

    // weekday + month stats
    let mut wd: HashMap<u32, (f64, usize, usize)> = HashMap::new(); // dow -> (sum ret%, count, upcount)
    let mut mo: HashMap<u32, (f64, usize)> = HashMap::new();
    for b in bars {
        let Some(dt) = chrono::DateTime::from_timestamp(b.day * 86_400, 0) else { continue };
        let dow = dt.weekday().num_days_from_monday();
        let ret = (b.close - b.open) / b.open * 100.0;
        let e = wd.entry(dow).or_insert((0.0, 0, 0));
        e.0 += ret;
        e.1 += 1;
        if b.close > b.open {
            e.2 += 1;
        }
        let e2 = mo.entry(dt.month()).or_insert((0.0, 0));
        e2.0 += ret;
        e2.1 += 1;
    }

    let wd_names = ["mon", "tue", "wed", "thu", "fri"];
    let mo_names = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    let mut patterns: Vec<Pattern> = Vec::new();

    // weekday edge
    for dow in 0..5 {
        if let Some((sum, cnt, up)) = wd.get(&dow).cloned() {
            if cnt >= 200 {
                let mean = sum / cnt as f64;
                let wr = up as f64 / cnt as f64;
                patterns.push(Pattern {
                    key: format!("weekday-{}", wd_names[dow as usize]),
                    edge_pct: mean,
                    win_rate: wr,
                    samples: cnt,
                    rule: format!("avg return on {}(dow) {:+.3}%", wd_names[dow as usize], mean),
                });
            }
        }
    }
    // best/worst weekday
    let best_wd = best_pattern(&patterns, "weekday-").cloned().map(|p| (p.key.replace("weekday-", ""), p.edge_pct, p.win_rate, p.samples));
    let worst_wd = worst_pattern(&patterns, "weekday-").cloned().map(|p| (p.key.replace("weekday-", ""), p.edge_pct, p.win_rate, p.samples));
    if let (Some((name, edge, wr, samples)), Some((wname, wedge, wwr, wsamples))) = (&best_wd, &worst_wd) {
        patterns.push(Pattern {
            key: "weekday-best".into(),
            edge_pct: *edge,
            win_rate: *wr,
            samples: *samples,
            rule: format!("MOST historical profit on {name}"),
        });
        patterns.push(Pattern {
            key: "weekday-worst".into(),
            edge_pct: *wedge,
            win_rate: *wwr,
            samples: *wsamples,
            rule: format!("LEAST historical profit on {wname}"),
        });
    }
    // month edge
    for m in 1..=12 {
        if let Some((sum, cnt)) = mo.get(&m).cloned() {
            if cnt >= 200 {
                let mean = sum / cnt as f64;
                patterns.push(Pattern {
                    key: format!("month-{}", mo_names[(m - 1) as usize]),
                    edge_pct: mean,
                    win_rate: 0.5,
                    samples: cnt,
                    rule: format!("avg return in {}(month) {:+.3}%", mo_names[(m - 1) as usize], mean),
                });
            }
        }
    }
    let best_mo = best_pattern(&patterns, "month-").cloned().map(|p| (p.key.replace("month-", ""), p.edge_pct, p.win_rate, p.samples));
    let worst_mo = worst_pattern(&patterns, "month-").cloned().map(|p| (p.key.replace("month-", ""), p.edge_pct, p.win_rate, p.samples));
    if let (Some((name, edge, wr, samples)), Some((wname, wedge, wwr, wsamples))) = (&best_mo, &worst_mo) {
        patterns.push(Pattern {
            key: "month-best".into(),
            edge_pct: *edge,
            win_rate: *wr,
            samples: *samples,
            rule: format!("MOST historical profit in {name}"),
        });
        patterns.push(Pattern {
            key: "month-worst".into(),
            edge_pct: *wedge,
            win_rate: *wwr,
            samples: *wsamples,
            rule: format!("LEAST historical profit in {wname}"),
        });
    }
    // monday-bias / friday-bias
    if let Some(md) = wd.get(&0) {
        if md.1 >= 200 {
            patterns.push(Pattern {
                key: "monday-bias".into(),
                edge_pct: md.0 / md.1 as f64,
                win_rate: md.2 as f64 / md.1 as f64,
                samples: md.1,
                rule: "Monday open-to-close".into(),
            });
        }
    }
    if let Some(fd) = wd.get(&4) {
        if fd.1 >= 200 {
            patterns.push(Pattern {
                key: "friday-bias".into(),
                edge_pct: fd.0 / fd.1 as f64,
                win_rate: fd.2 as f64 / fd.1 as f64,
                samples: fd.1,
                rule: "Friday open-to-close".into(),
            });
        }
    }
    // range expansion: big up day → next day
    let mut exp_up_then_up = 0usize;
    let mut exp_total = 0usize;
    let atr20 = avg_atr(bars, 20);
    for w in bars.windows(2) {
        let r0 = (w[0].high - w[0].low) / w[0].open;
        if r0 > atr20 * 1.7 {
            exp_total += 1;
            if w[1].close > w[1].open {
                exp_up_then_up += 1;
            }
        }
    }
    if exp_total >= 100 {
        patterns.push(Pattern {
            key: "range-expansion".into(),
            edge_pct: exp_up_then_up as f64 / exp_total as f64,
            win_rate: exp_up_then_up as f64 / exp_total as f64,
            samples: exp_total,
            rule: "after a >1.7x ATR expansion day, next day up".into(),
        });
    }

    MessageStrategy {
        symbol: symbol.to_string(),
        span_start: fmt_day(bars.first().map(|b| b.day).unwrap_or(0)),
        span_end: fmt_day(bars.last().map(|b| b.day).unwrap_or(0)),
        bars: n,
        all_time_high,
        all_time_low,
        bullish_days_pct,
        avg_daily_range_pct: avg_range,
        momentum_persistence,
        best_weekday: best_wd.as_ref().map(|(n, _, _, _)| n.clone()).unwrap_or_default(),
        worst_weekday: worst_wd.as_ref().map(|(n, _, _, _)| n.clone()).unwrap_or_default(),
        best_month: best_mo.as_ref().map(|(n, _, _, _)| n.clone()).unwrap_or_default(),
        worst_month: worst_mo.as_ref().map(|(n, _, _, _)| n.clone()).unwrap_or_default(),
        patterns,
    }
}

fn best_pattern<'a>(ps: &'a [Pattern], prefix: &str) -> Option<&'a Pattern> {
    ps.iter().filter(|p| p.key.starts_with(prefix)).max_by(|a, b| a.edge_pct.partial_cmp(&b.edge_pct).unwrap_or(std::cmp::Ordering::Equal))
}
fn worst_pattern<'a>(ps: &'a [Pattern], prefix: &str) -> Option<&'a Pattern> {
    ps.iter().filter(|p| p.key.starts_with(prefix)).min_by(|a, b| a.edge_pct.partial_cmp(&b.edge_pct).unwrap_or(std::cmp::Ordering::Equal))
}

fn avg_atr(bars: &[DailyBar], window: usize) -> f64 {
    let w = window.min(bars.len());
    if w < 2 {
        return 0.0;
    }
    let sum: f64 = bars
        .iter()
        .rev()
        .take(w)
        .map(|b| (b.high - b.low) / b.open)
        .sum();
    sum / w as f64
}

fn fmt_day(day: i64) -> String {
    chrono::DateTime::from_timestamp(day * 86_400, 0)
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

/// Aggregate a HistoricalDataCopy 5-min CSV into daily OHLC bars.
fn aggregate_daily(path: PathBuf) -> Result<Vec<DailyBar>> {
    let file = std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let mut out: Vec<DailyBar> = Vec::new();
    let mut cur: Option<(i64, f64, f64, f64, f64, f64)> = None; // day,o,h,l,c,v
    for (idx, line) in file.lines().enumerate() {
        if idx == 0 {
            continue; // header
        }
        let mut it = line.splitn(6, ',');
        let (date_str, o_s, h_s, l_s, c_s, v_s) = (
            it.next().unwrap_or(""),
            it.next().unwrap_or(""),
            it.next().unwrap_or(""),
            it.next().unwrap_or(""),
            it.next().unwrap_or(""),
            it.next().unwrap_or(""),
        );
        let Ok(ts) = chrono::NaiveDateTime::parse_from_str(date_str, "%Y-%m-%d %H:%M:%S") else {
            continue;
        };
        let day = ts.and_utc().timestamp() / 86_400;
        let o: f64 = o_s.parse().unwrap_or(0.0);
        let h: f64 = h_s.parse().unwrap_or(0.0);
        let l: f64 = l_s.parse().unwrap_or(0.0);
        let c: f64 = c_s.parse().unwrap_or(0.0);
        let v: f64 = v_s.parse().unwrap_or(0.0);
        if o <= 0.0 || h <= 0.0 || l <= 0.0 || c <= 0.0 {
            continue;
        }
        match &mut cur {
            Some((d, oo, hh, ll, cc, vv)) if *d == day => {
                *oo = if *oo == 0.0 { o } else { *oo };
                *hh = hh.max(h);
                *ll = ll.min(l);
                *cc = c;
                *vv += v;
            }
            _ => {
                if let Some((d, oo, hh, ll, cc, vv)) = cur.take() {
                    out.push(DailyBar { day: d, open: oo, high: hh, low: ll, close: cc, volume: vv });
                }
                cur = Some((day, o, h, l, c, v));
            }
        }
    }
    if let Some((d, oo, hh, ll, cc, vv)) = cur {
        out.push(DailyBar { day: d, open: oo, high: hh, low: ll, close: cc, volume: vv });
    }
    if out.is_empty() {
        return Err(anyhow!("no rows in {}", path.display()));
    }
    Ok(out)
}

fn map_symbol(symbol: &str) -> String {
    match symbol {
        "EUR_USD" => "EURUSD".to_string(),
        "GBP_USD" => "GBPUSD".to_string(),
        "USD_JPY" => "USDJPY".to_string(),
        "XAU_USD" | "XAUUSD" => "XAUUSD".to_string(),
        s => s.to_string(),
    }
}

/// Best-effort path to the Downloads folder (Windows).
fn dirs_download_dir() -> String {
    let home = std::env::var("USERPROFILE").unwrap_or_else(|_| ".".to_string());
    format!("{home}\\Downloads\\dukascopy-data-downloader\\data")
}

/// market helper used by callers
pub fn market_of(symbol: &str) -> Market {
    if symbol == "XAU_USD" || symbol == "XAUUSD" {
        Market::Gold
    } else {
        Market::Forex
    }
}