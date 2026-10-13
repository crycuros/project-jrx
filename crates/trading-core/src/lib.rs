//! Core domain models for the Trading AI system.

pub mod config;
pub mod models;

pub use config::Config;
pub use models::{
    AiMemoryNode, AiVerdict, Candle, ContractMode, EngineMode, Instrument, JsonMap, LiveTick, Market, MemoryOrbit,
    NetworkNode, NewsItem, NodeVerdict, Order, OrderBook, OrderBookLevel, OrderSide, OrderStatus,
    Position, Proposal, Regime, Status, SystemHealth, TickRecord, Trade, TradeStatus, UiSnapshot,
};