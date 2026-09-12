//! Strategy engine: technical indicators + regime detection + signal generation.

pub mod indicators;
pub mod engine;

pub use engine::{SignalQuality, StrategyEngine};
pub use indicators::{atr, ema, macd, rsi, sma};