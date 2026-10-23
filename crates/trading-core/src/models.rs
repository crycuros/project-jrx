//! Core data models shared across the whole system.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Market type discriminator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Market {
    Crypto,
    #[default]
    Forex,
    Gold,
    Synthetic,
}

impl Market {
    pub fn from_symbol(symbol: &str) -> Self {
        let sym_up = symbol.to_uppercase();
        if sym_up.contains("1HZ") || sym_up.starts_with("R_") || sym_up.contains("VOL") || sym_up.contains("SYNTH") {
            Market::Synthetic
        } else if sym_up.contains("XAU") {
            Market::Gold
        } else if symbol.contains('/') {
            let base = symbol.split('/').next().unwrap_or("");
            let crypto_bases = ["BTC","ETH","SOL","BNB","XRP","ADA","DOT","AVAX","MATIC","LINK","UNI","ATOM"];
            if crypto_bases.contains(&base) {
                Market::Crypto
            } else {
                Market::Forex
            }
        } else {
            Market::Forex
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Market::Crypto => "crypto",
            Market::Gold   => "gold",
            Market::Forex  => "forex",
            Market::Synthetic => "synthetic",
        }
    }

    pub fn contract_size(&self) -> f64 {
        match self {
            Market::Crypto => 1.0,
            Market::Forex  => 100_000.0,
            Market::Gold   => 100.0,
            Market::Synthetic => 1.0,
        }
    }
}

/// One OHLCV candle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candle {
    pub time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

/// A market order.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Order {
    pub id: String,
    pub symbol: String,
    pub side: OrderSide,
    pub size: f64,
    pub price: f64,
    pub stop_loss: Option<f64>,
    pub take_profit: Option<f64>,
    pub status: OrderStatus,
    pub market: Market,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum OrderSide {
    #[default]
    Buy,
    Sell,
}

impl OrderSide {
    pub fn as_str(&self) -> &'static str {
        match self {
            OrderSide::Buy => "buy",
            OrderSide::Sell => "sell",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderStatus {
    Pending,
    Filled,
    Rejected,
}

/// Contract type for execution: Multiplier (standard leveraged position) vs Rise/Fall (binary options).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ContractMode {
    #[default]
    Multiplier,
    RiseFall,
}

impl ContractMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ContractMode::Multiplier => "multiplier",
            ContractMode::RiseFall => "rise_fall",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "rise_fall" | "risefall" | "binary" | "options" => ContractMode::RiseFall,
            _ => ContractMode::Multiplier,
        }
    }
}

/// A single real-time market tick from live WebSocket stream.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct TickRecord {
    pub price: f64,
    pub epoch: i64,
}

/// An incoming live tick payload with symbol and timestamp.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveTick {
    pub symbol: String,
    pub price: f64,
    pub epoch: i64,
}

/// An open or closed trade position.
/// Mirrors `positions` + `trades` in the original DB.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trade {
    pub id: Option<i64>,
    pub symbol: String,
    pub side: OrderSide,
    pub entry_price: f64,
    pub exit_price: Option<f64>,
    pub amount: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub pnl: Option<f64>,
    pub status: TradeStatus,
    pub regime: String,
    pub confidence: f64,
    pub reason: String,
    pub ai_verdict: String,
    pub ai_reason: String,
    pub quality_score: f64,
    pub market: Market,
    pub broker: String,
    pub entry_time: String,
    pub exit_time: Option<String>,
    pub external_id: Option<String>,
    #[serde(default)]
    pub contract_mode: ContractMode,
    #[serde(default)]
    pub duration_ticks: Option<u32>,
    #[serde(default)]
    pub payout: Option<f64>,
    /// unrealized PnL (live tick), not persisted
    #[serde(default)]
    pub unrealized_pnl: Option<f64>,
    #[serde(default)]
    pub current_price: Option<f64>,
}

impl Default for Trade {
    fn default() -> Self {
        Self {
            id: None,
            symbol: String::new(),
            side: OrderSide::Buy,
            entry_price: 0.0,
            exit_price: None,
            amount: 0.0,
            stop_loss: 0.0,
            take_profit: 0.0,
            pnl: None,
            status: TradeStatus::Open,
            regime: String::new(),
            confidence: 0.0,
            reason: String::new(),
            ai_verdict: String::new(),
            ai_reason: String::new(),
            quality_score: 0.0,
            market: Market::Forex,
            broker: "oanda".to_string(),
            entry_time: String::new(),
            exit_time: None,
            external_id: None,
            contract_mode: ContractMode::Multiplier,
            duration_ticks: None,
            payout: None,
            unrealized_pnl: None,
            current_price: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TradeStatus {
    Open,
    Closed,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Position {
    pub symbol: String,
    pub side: OrderSide,
    pub amount: f64,
    pub entry_price: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub unrealized_pnl: f64,
    pub current_price: f64,
    pub regime: String,
    pub market: Market,
}

/// App-wide status snapshot sent to the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Status {
    pub running: bool,
    pub mode: String,
    pub engine: String,
    pub asset: String,
    pub balance: f64,
    pub balance_total: f64,
    pub equity_forex: f64,
    pub equity_gold: f64,
    pub daily_pnl: f64,
    pub total_pnl: f64,
    pub win_rate: f64,
    pub consecutive_losses: i32,
    pub kill_switch: bool,
    pub open_positions: i32,
    pub total_trades: i32,
    pub strategies_saved: i32,
    pub ai_available: bool,
    pub forex_enabled: bool,
    pub gold_enabled: bool,
    pub active_engine: String,
    pub assets: Vec<String>,
}

impl Default for Status {
    fn default() -> Self {
        Self {
            running: false,
            mode: "paper".to_string(),
            engine: "hybrid".to_string(),
            asset: "EUR_USD".to_string(),
            balance: 0.0,
            balance_total: 0.0,
            equity_forex: 10_000.0,
            equity_gold: 10_000.0,
            daily_pnl: 0.0,
            total_pnl: 0.0,
            win_rate: 0.0,
            consecutive_losses: 0,
            kill_switch: false,
            open_positions: 0,
            total_trades: 0,
            strategies_saved: 96,
            ai_available: false,
            forex_enabled: true,
            gold_enabled: true,
            active_engine: "hybrid".to_string(),
            assets: vec![],
        }
    }
}

impl Status {
    /// Rebuild a Status from runtime state collections.
    pub fn build(
        cfg: &crate::Config,
        equity_forex: f64,
        equity_gold: f64,
        daily_pnl: f64,
        total_pnl: f64,
        consecutive_losses: i32,
        kill_switch: bool,
        trades: &[Trade],
        positions: &[Trade],
        forex_enabled: bool,
        gold_enabled: bool,
        active_engine: &str,
        ai_available: bool,
    ) -> Self {
        let closed: Vec<&Trade> = trades
            .iter()
            .filter(|t| t.status == TradeStatus::Closed && t.pnl.is_some())
            .collect();
        let wins = closed.iter().filter(|t| t.pnl.unwrap_or(0.0) > 0.0).count();
        let win_rate = if closed.is_empty() {
            0.0
        } else {
            (wins as f64 / closed.len() as f64) * 100.0
        };

        let mut assets = cfg.forex_assets.clone();
        assets.extend(cfg.gold_assets.clone());

        Self {
            running: true,
            mode: cfg.trading_mode.clone(),
            engine: active_engine.to_string(),
            asset: cfg.default_asset.clone(),
            balance: equity_forex + equity_gold,
            balance_total: equity_forex + equity_gold,
            equity_forex,
            equity_gold,
            daily_pnl,
            total_pnl,
            win_rate,
            consecutive_losses,
            kill_switch,
            open_positions: positions.len() as i32,
            total_trades: trades.len() as i32,
            strategies_saved: 0,
            ai_available,
            forex_enabled,
            gold_enabled,
            active_engine: active_engine.to_string(),
            assets,
        }
    }
}

/// Market metadata for an asset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Instrument {
    pub symbol: String,
    pub display_name: String,
    pub market: Market,
    pub min_size: f64,
    pub digits: u32,
}

/// Network-graph node: a token / asset cluster representation.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NetworkNode {
    pub id: String,
    pub label: String,
    pub cluster: String,
    pub market_cap: f64,
    pub sentiment: f64,
    pub x: f32,
    pub y: f32,
}

/// AI verdict for a memory node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum NodeVerdict {
    Go,
    Watch,
    #[default]
    NoGo,
    Neutral,
}

/// Orbital tier of a node in the galaxy AI memory graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum MemoryOrbit {
    /// Inner ring — live strategy signals / proposals
    Signal,
    /// Mid ring — regime / pattern memory
    Regime,
    /// Outer ring — asset universe
    #[default]
    Asset,
}

/// A node in the AI Memory Graph.
/// Three orbital tiers: Signal (inner) → Regime (mid) → Asset (outer).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AiMemoryNode {
    pub id: String,
    pub label: String,
    pub orbit: MemoryOrbit,
    pub verdict: NodeVerdict,
    pub confidence: f64,  // 0.0 – 1.0
    pub market: Market,
    pub active: bool,     // currently selected / executing
    /// Tooltip text shown when hovering the node (e.g. mined historical patterns).
    pub detail: String,
}

/// Latest data snapshot broadcast to the UI every tick.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UiSnapshot {
    pub status: Status,
    pub positions: Vec<Trade>,
    pub recent_trades: Vec<Trade>,
    pub news: Vec<NewsItem>,
    pub system: SystemHealth,
    pub candles: std::collections::HashMap<String, Vec<Candle>>,
    pub order_books: std::collections::HashMap<String, OrderBook>,
    /// Per-symbol mined strategy context (from the historical data copy memory).
    pub historical: std::collections::HashMap<String, String>,
}

/// A single live level in an order book.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct OrderBookLevel {
    pub price: f64,
    pub qty: f64,
}

/// Live depth for one symbol (from the real broker; never synthesised).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OrderBook {
    pub symbol: String,
    pub bids: Vec<OrderBookLevel>,
    pub asks: Vec<OrderBookLevel>,
    pub spread: f64,
    pub live: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NewsItem {
    pub title: String,
    pub source: String,
    pub time: String,
    pub impact: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SystemHealth {
    pub oanda_status: String,
    pub ollama_status: String,
    pub ollama_model: String,
    pub trade_limit: i32,
    pub fx_source: String,
    /// "OPEN", "CLOSED", or "UNKNOWN" (from the live broker feed).
    pub market_status: String,
}

/// Trading engine selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EngineMode {
    Forex,
    Gold,
}

impl EngineMode {
    pub fn from_str(s: &str) -> Self {
        match s {
            "gold" => EngineMode::Gold,
            _ => EngineMode::Forex,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            EngineMode::Forex => "forex",
            EngineMode::Gold => "gold",
        }
    }
}

/// AI evaluation verdict for a candidate trade.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AiVerdict {
    pub decision: String, // GO | WATCH | NO-GO
    pub reason: String,
    pub confidence: f64,
}

/// Market regime — classified from price structure before strategy signals run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Regime {
    /// Strong directional trend (ADX > 25, EMA spread wide)
    #[default]
    Trending,
    /// Sideways / mean-reverting (ADX < 20, price oscillates)
    Ranging,
    /// Elevated volatility, wide ATR% — caution zone
    Volatile,
    /// Price breaking out of consolidation range
    Breakout,
}

impl Regime {
    pub fn as_str(&self) -> &'static str {
        match self {
            Regime::Trending  => "trending",
            Regime::Ranging   => "ranging",
            Regime::Volatile  => "volatile",
            Regime::Breakout  => "breakout",
        }
    }
    pub fn from_str(s: &str) -> Self {
        match s {
            "ranging"   => Regime::Ranging,
            "volatile"  => Regime::Volatile,
            "breakout"  => Regime::Breakout,
            _           => Regime::Trending,
        }
    }
}

/// Strategy signal proposal.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Proposal {
    pub symbol: String,
    pub action: OrderSide,
    pub entry: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub risk_reward: f64,
    pub confidence: f64,
    pub regime: String,
    pub reason: String,
    /// Which sub-strategy produced this signal (e.g. "trend::ema_cross", "mean_rev::bb_touch", "tick::momentum")
    pub strategy_source: String,
    /// Number of strategies that agreed on this signal (for voting combiner)
    pub vote_count: u8,
    /// Execution contract mode (Multiplier vs Rise/Fall)
    pub contract_mode: ContractMode,
    /// Contract duration in ticks for Rise/Fall binary options
    pub duration_ticks: u32,
}

// Convenience alias for JSON maps used by UI snapshots.
pub type JsonMap = HashMap<String, serde_json::Value>;