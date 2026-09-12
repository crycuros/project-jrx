//! Core data models shared across the whole system.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Market type discriminator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Market {
    #[default]
    Forex,
    Gold,
}

impl Market {
    pub fn from_symbol(symbol: &str) -> Self {
        if symbol == "XAU_USD" || symbol == "XAUUSD" || symbol == "XAU/USD" {
            Market::Gold
        } else if symbol.contains('_') {
            Market::Forex
        } else {
            Market::Forex
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Market::Gold => "gold",
            Market::Forex => "forex",
        }
    }

    pub fn contract_size(&self) -> f64 {
        match self {
            Market::Forex => 100_000.0,
            Market::Gold => 100.0,
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
            strategies_saved: 0,
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
}

// Convenience alias for JSON maps used by UI snapshots.
pub type JsonMap = HashMap<String, serde_json::Value>;