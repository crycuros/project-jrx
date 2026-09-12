//! OANDA REST client (v20) — forex + gold engine.
//!
//! Replaces the old MetaTrader 5 integration. OANDA has a clean REST API
//! with demo (practice) and live accounts, and supports XAU_USD gold.
//!
//! Base URLs:
//!   practice: https://api-fxpractice.oanda.com/v3
//!   live:     https://api-fxtrade.oanda.com/v3

use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use serde::Deserialize;

use trading_core::{Candle, Config, Instrument, Market, Order, OrderSide, OrderStatus};

use super::Exchange;

const PRACTICE_BASE: &str = "https://api-fxpractice.oanda.com/v3";
const LIVE_BASE: &str = "https://api-fxtrade.oanda.com/v3";

/// Normalizes slash- or dash-separated symbols into OANDA underscore form.
/// OANDA instruments are stored in underscore form in fx_assets already,
/// this helper normalises any slash-form into underscore-form.
pub fn to_oanda_symbol(symbol: &str) -> String {
    if symbol.contains('/') {
        let (a, b) = symbol.split_once('/').unwrap();
        format!("{}_{}", a, b)
    } else {
        symbol.replace('-', "_")
    }
}

const INTERVAL_MAP: &[(&str, &str)] = &[
    ("1m", "M1"),
    ("5m", "M5"),
    ("15m", "M15"),
    ("30m", "M30"),
    ("1h", "H1"),
    ("4h", "H4"),
    ("1d", "D"),
    ("1w", "W"),
];

fn to_oanda_granularity(tf: &str) -> String {
    INTERVAL_MAP
        .iter()
        .find(|(k, _)| *k == tf)
        .map(|(_, v)| v.to_string())
        .unwrap_or_else(|| "M5".to_string())
}

#[derive(Debug, Deserialize)]
struct CandleResponse {
    candles: Vec<OandaCandle>,
}

#[derive(Debug, Deserialize)]
struct OandaCandle {
    #[allow(dead_code)]
    complete: bool,
    time: String,
    mid: Mid,
    volume: i64,
}

#[derive(Debug, Deserialize)]
struct Mid {
    o: String,
    h: String,
    l: String,
    c: String,
}

#[derive(Debug, Deserialize)]
struct AccountSummary {
    #[allow(dead_code)]
    currency: Option<String>,
    balance: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AccountResponse {
    account: AccountSummary,
}

#[derive(Debug, Deserialize)]
struct PricingResponse {
    prices: Vec<Price>,
}

#[derive(Debug, Deserialize)]
struct Price {
    closeout_bid: Option<String>,
    closeout_ask: Option<String>,
}

#[derive(Clone)]
pub struct OandaClient {
    client: Client,
    base: String,
    token: String,
    account_id: String,
    #[allow(dead_code)]
    practice: bool,
}

impl OandaClient {
    pub fn new(cfg: &Config) -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .expect("build reqwest"),
            base: if cfg.oanda_practice { PRACTICE_BASE.to_string() } else { LIVE_BASE.to_string() },
            token: cfg.oanda_token.clone(),
            account_id: cfg.oanda_account_id.clone(),
            practice: cfg.oanda_practice,
        }
    }

    async fn auth<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T> {
        if self.token.is_empty() {
            return Err(anyhow!("OANDA_TOKEN not set in .env"));
        }
        let url = format!("{}{}", self.base, path);
        let resp = self
            .client
            .get(&url)
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| anyhow!("oanda request: {e}"))?
            .error_for_status()
            .map_err(|e| anyhow!("oanda status: {e}"))?;
        resp.json::<T>().await.context("oanda json")
    }
}

#[async_trait::async_trait]
impl Exchange for OandaClient {
    fn name(&self) -> &'static str {
        "oanda"
    }
    fn market(&self) -> Market {
        Market::Forex
    }

    async fn fetch_candles(&self, symbol: &str, timeframe: &str, limit: u32) -> Result<Vec<Candle>> {
        let inst = to_oanda_symbol(symbol);
        let gran = to_oanda_granularity(timeframe);
        let path = format!(
            "/instruments/{}/candles?granularity={}&count={}&price=M",
            inst, gran, limit
        );
        let resp: CandleResponse = self.auth(&path).await?;
        Ok(resp.candles.into_iter().map(|c| Candle {
            time: parse_oanda_time(&c.time),
            open: c.mid.o.parse().unwrap_or(0.0),
            high: c.mid.h.parse().unwrap_or(0.0),
            low: c.mid.l.parse().unwrap_or(0.0),
            close: c.mid.c.parse().unwrap_or(0.0),
            volume: c.volume as f64,
        }).collect())
    }

    async fn fetch_price(&self, symbol: &str) -> Result<f64> {
        let inst = to_oanda_symbol(symbol);
        let path = format!("/instruments/{}/pricing?price=mid", inst);
        let resp: PricingResponse = self.auth(&path).await?;
        let price = resp
            .prices
            .first()
            .ok_or_else(|| anyhow!("no price for {symbol}"))?;
        let bid: f64 = price
            .closeout_bid
            .as_deref()
            .unwrap_or("0")
            .parse()
            .unwrap_or(0.0);
        let ask: f64 = price
            .closeout_ask
            .as_deref()
            .unwrap_or("0")
            .parse()
            .unwrap_or(0.0);
        if bid > 0.0 && ask > 0.0 {
            Ok((bid + ask) / 2.0)
        } else {
            Ok(bid.max(ask))
        }
    }

    async fn ping(&self) -> bool {
        if self.token.is_empty() {
            return false; // token needed even for candles
        }
        self.auth::<serde_json::Value>(&format!("/accounts/{}/summary", self.account_id))
            .await
            .is_ok()
    }

    async fn market_order(&self, symbol: &str, side: OrderSide, size: f64, price: f64) -> Result<Order> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Ok(Order {
            id: format!("PAPER-OANDA-{}", now),
            symbol: symbol.to_string(),
            side,
            size,
            price,
            stop_loss: None,
            take_profit: None,
            status: OrderStatus::Filled,
            market: Market::Forex,
            created_at: now.to_string(),
        })
    }

    async fn free_balance(&self) -> Result<f64> {
        if self.token.is_empty() {
            return Ok(10_000.0);
        }
        let path = format!("/accounts/{}/summary", self.account_id);
        let resp: AccountResponse = self.auth(&path).await?;
        resp.account
            .balance
            .and_then(|b| b.parse().ok())
            .ok_or_else(|| anyhow!("balance not available"))
    }

    fn available_instruments(&self) -> Vec<Instrument> {
        vec![
            instrument("XAU_USD", 0.001, 2),
            instrument("EUR_USD", 0.001, 5),
            instrument("GBP_USD", 0.001, 5),
            instrument("USD_JPY", 0.001, 3),
        ]
    }
}

pub fn instrument(symbol: &str, min_size: f64, digits: u32) -> Instrument {
    super::instrument(symbol, min_size, digits)
}

fn parse_oanda_time(ts: &str) -> i64 {
    // OANDA gives RFC3339 like "2026-09-10T12:00:00.000000000Z"
    chrono::DateTime::parse_from_rfc3339(ts)
        .map(|dt| dt.timestamp())
        .unwrap_or(0)
}