//! Core domain models for the Trading AI system.

pub mod config;
pub mod models;

pub use config::Config;
pub use models::{
    AiMemoryNode, AiVerdict, Candle, EngineMode, Instrument, JsonMap, Market, MemoryOrbit,
    NetworkNode, NewsItem, NodeVerdict, Order, OrderBook, OrderBookLevel, OrderSide, OrderStatus,
    Position, Proposal, Status, SystemHealth, Trade, TradeStatus, UiSnapshot,
};