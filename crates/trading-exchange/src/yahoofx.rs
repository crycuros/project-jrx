//! Yahoo Finance FX/gold data feed — free, no API key.
//!
//! Uses the public `v8/finance/chart` endpoint for intraday candles.
//! Results are cached per symbol for a short TTL so we stay well under
//! Yahoo's rate limits (the engine polls every few seconds).

use anyhow::{anyhow, Result};
use reqwest::Client;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use trading_core::{Candle, Config, Instrument, Market, Order, OrderSide, OrderStatus};

use super::Exchange;

const BASE: &str = "https://query1.finance.yahoo.com/v8/finance/chart";
const TTL: Duration = Duration::from_secs(300);

fn yahoo_symbol(symbol: &str) -> String {
    match symbol {
        "EUR_USD" => "EURUSD=X".to_string(),
        "GBP_USD" => "GBPUSD=X".to_string(),
        "USD_JPY" => "USDJPY=X".to_string(),
        "USD_CHF" => "USDCHF=X".to_string(),
        "AUD_USD" => "AUDUSD=X".to_string(),
        "NZD_USD" => "NZDUSD=X".to_string(),
        "USD_CAD" => "USDCAD=X".to_string(),
        "XAU_USD" | "XAUUSD" => "GC=F".to_string(),
        s => s.to_string(),
    }
}

fn to_interval(timeframe: &str) -> &str {
    match timeframe {
        "1m" => "1m",
        "15m" => "15m",
        "1h" => "60m",
        _ => "5m",
    }
}

#[derive(Debug, Deserialize)]
struct ChartResponse {
    chart: Chart,
}
#[derive(Debug, Deserialize)]
struct Chart {
    result: Vec<ChartResult>,
}
#[derive(Debug, Deserialize)]
struct ChartResult {
    #[serde(rename = "meta")]
    meta: ChartMeta,
    timestamp: Option<Vec<i64>>,
    indicators: Indicators,
}
#[derive(Debug, Deserialize)]
struct ChartMeta {
    #[serde(rename = "regularMarketPrice")]
    regular_market_price: Option<f64>,
}
#[derive(Debug, Deserialize)]
struct Indicators {
    quote: Vec<QuoteSeries>,
}
#[derive(Debug, Deserialize)]
struct QuoteSeries {
    open: Option<Vec<Option<f64>>>,
    high: Option<Vec<Option<f64>>>,
    low: Option<Vec<Option<f64>>>,
    close: Option<Vec<Option<f64>>>,
}

pub struct YahooFx {
    client: Client,
    cache: Mutex<HashMap<String, (SystemTime, Vec<Candle>)>>,
}

impl YahooFx {
    pub fn new(_cfg: &Config) -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .expect("build reqwest"),
            cache: Mutex::new(HashMap::new()),
        }
    }

    async fn fetch_raw(&self, symbol: &str, timeframe: &str) -> Result<Vec<Candle>> {
        let iv = to_interval(timeframe);
        let url = format!("{BASE}/{}?range=1d&interval={iv}", yahoo_symbol(symbol));
        let resp = self
            .client
            .get(&url)
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .send()
            .await
            .map_err(|e| anyhow!("yahoo request: {e}"))?
            .error_for_status()
            .map_err(|e| anyhow!("yahoo status: {e}"))?;
        let body: ChartResponse = resp.json().await?;
        let r = body
            .chart
            .result
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("yahoo: empty chart result"))?;
        let quote = r
            .indicators
            .quote
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("yahoo: empty quote"))?;

        let mut candles = Vec::new();
        if let (Some(ts), Some(o), Some(h), Some(l), Some(c)) =
            (r.timestamp, quote.open, quote.high, quote.low, quote.close)
        {
            let n = ts.len().min(o.len()).min(h.len()).min(l.len()).min(c.len());
            for i in 0..n {
                let (o, h, l, c) = match (o[i], h[i], l[i], c[i]) {
                    (Some(o), Some(h), Some(l), Some(c)) => (o, h, l, c),
                    _ => continue, // market-closed slots come back null
                };
                candles.push(Candle { time: ts[i], open: o, high: h, low: l, close: c, volume: 0.0 });
            }
        }

        if candles.len() < 2 && r.meta.regular_market_price.is_some() {
            let p = r.meta.regular_market_price.unwrap_or(0.0);
            let now = SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            candles.push(Candle { time: now, open: p, high: p, low: p, close: p, volume: 0.0 });
        }
        if candles.is_empty() {
            return Err(anyhow!("yahoo: no candle data for {symbol}"));
        }
        Ok(candles)
    }

    async fn fetch_candles_cached(&self, symbol: &str, timeframe: &str) -> Result<Vec<Candle>> {
        let key = format!("{symbol}:{timeframe}");
        if let Some((t, v)) = self.cache.lock().unwrap().get(&key) {
            if t.elapsed().unwrap_or(Duration::from_secs(u64::MAX)) < TTL {
                return Ok(v.clone());
            }
        }
        let candles = self.fetch_raw(symbol, timeframe).await?;
        self.cache.lock().unwrap().insert(key, (SystemTime::now(), candles.clone()));
        Ok(candles)
    }
}

#[async_trait::async_trait]
impl Exchange for YahooFx {
    fn name(&self) -> &'static str {
        "yahoo"
    }
    fn market(&self) -> Market {
        Market::Forex
    }

    async fn fetch_candles(&self, symbol: &str, timeframe: &str, _limit: u32) -> Result<Vec<Candle>> {
        self.fetch_candles_cached(symbol, timeframe).await
    }

    async fn fetch_price(&self, symbol: &str) -> Result<f64> {
        let candles = self.fetch_candles_cached(symbol, "5m").await?;
        candles.last().map(|c| c.close).ok_or_else(|| anyhow!("yahoo: no price"))
    }

    async fn ping(&self) -> bool {
        self.fetch_price("EUR_USD").await.is_ok()
    }

    async fn market_order(&self, symbol: &str, side: OrderSide, size: f64, price: f64) -> Result<Order> {
        let now = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Ok(Order {
            id: format!("PAPER-YAHOO-{}", now),
            symbol: symbol.to_string(),
            side,
            size,
            price,
            stop_loss: None,
            take_profit: None,
            status: OrderStatus::Filled,
            market: Market::from_symbol(symbol),
            created_at: now.to_string(),
        })
    }

    async fn free_balance(&self) -> Result<f64> {
        Ok(10_000.0)
    }

    fn available_instruments(&self) -> Vec<Instrument> {
        let mut out = Vec::new();
        for s in ["EUR_USD", "GBP_USD", "USD_JPY", "XAU_USD"] {
            out.push(super::instrument(s, 0.0, 4));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> Config {
        Config::from_env()
    }

    #[tokio::test]
    #[ignore]
    async fn live_fetch_forex_and_gold() {
        let fx = YahooFx::new(&test_config());
        for sym in ["EUR_USD", "GBP_USD", "USD_JPY", "XAU_USD"] {
            let price = fx.fetch_price(sym).await;
            assert!(price.is_ok(), "{sym} price failed: {price:?}");
            let candles = fx.fetch_candles(sym, "5m", 120).await;
            assert!(candles.is_ok(), "{sym} candles failed: {candles:?}");
            let n = candles.unwrap().len();
            assert!(n >= 40, "{sym} only {n} candles");
            println!("{sym} price={:?} candles={n}", price.unwrap());
        }
        assert!(fx.ping().await);
    }
}