//! Strategy engine: multi-strategy quantitative pipeline.
//!
//! Modules:
//!   indicators   — technical indicators (SMA, EMA, RSI, ATR, MACD, BB, ADX, Stoch, Donchian, Z-score)
//!   regime       — market regime classifier (Trending / Ranging / Volatile / Breakout)
//!   strategies   — sub-strategy modules + multi-strategy combiner

pub mod engine;
pub mod indicators;
pub mod regime;
pub mod strategies;

pub use engine::{SignalQuality, StrategyEngine};
pub use indicators::{atr, bollinger_bands, donchian, ema, macd, rsi, sma, zscore};
pub use regime::RegimeDetector;
pub use strategies::multi::{combine as combine_signals, kelly_fraction};
