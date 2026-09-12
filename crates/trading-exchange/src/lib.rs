//! Exchange clients for OANDA forex and gold trading.

pub mod derivfx;
pub mod oanda;
pub mod yahoofx;

use anyhow::Result;
use trading_core::{Candle, Instrument, Market, Order, OrderSide, Config};

/// Trait implemented by every broker adapter.
#[async_trait::async_trait]
pub trait Exchange: Send + Sync {
    fn name(&self) -> &'static str;
    fn market(&self) -> Market;

    /// Fetch recent OHLCV candles for `symbol`.
    async fn fetch_candles(&self, symbol: &str, timeframe: &str, limit: u32) -> Result<Vec<Candle>>;

    /// Current mid price for a symbol.
    async fn fetch_price(&self, symbol: &str) -> Result<f64>;

    /// Check backend connectivity.
    async fn ping(&self) -> bool;

    /// Place a market order (paper by default).
    async fn market_order(&self, symbol: &str, side: OrderSide, size: f64, price: f64) -> Result<Order>;

    /// Return base currency balance available for trading.
    async fn free_balance(&self) -> Result<f64>;

    /// Available trading instruments.
    fn available_instruments(&self) -> Vec<Instrument>;
}

/// Utility: build an Instrument from a symbol + price prefix info.
pub fn instrument(symbol: &str, min_size: f64, digits: u32) -> Instrument {
    Instrument {
        symbol: symbol.to_string(),
        display_name: symbol.to_string(),
        market: Market::from_symbol(symbol),
        min_size,
        digits,
    }
}

pub fn asset_pair_from_cfg(cfg: &Config) -> Vec<(String, Market)> {
    let mut pairs = Vec::new();
    for s in &cfg.forex_assets {
        pairs.push((s.clone(), Market::Forex));
    }
    for s in &cfg.gold_assets {
        pairs.push((s.clone(), Market::Gold));
    }
    pairs
}