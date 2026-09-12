//! Deriv Options demo adapter — REAL multiplier contracts (MULTUP/MULTDOWN) with SL/TP.
//!
//! Auth flow:
//!   1. REST OTP on the Options WS URL using the Bearer PAT + app id.
//!   2. Authenticated WS session for proposal -> buy -> sell.
//!
//! Order model (Multiliplier): stake in USD, no fixed expiry, PnL = move% * multiplier * stake.
//! SL/TP are attached to the contract server-side as USD amounts and are validated against
//! stake/maximums — we clamp before sending.

use anyhow::{anyhow, bail, Result};
use futures_util::{SinkExt, StreamExt};
use log::info;
use reqwest::Client as Http;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

use trading_core::{Candle, Config, Instrument, Market, Order, OrderSide, OrderStatus};

use super::yahoofx::YahooFx;
use super::Exchange;

const REST_BASE: &str = "https://api.derivws.com/trading/v1/options";
const MULTIPLIER: u32 = 100;
const TF_SECS: Duration = Duration::from_secs(60);

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

fn deriv_symbol(symbol: &str) -> String {
    match symbol {
        "EUR_USD" => "frxEURUSD".to_string(),
        "GBP_USD" => "frxGBPUSD".to_string(),
        "USD_JPY" => "frxUSDJPY".to_string(),
        "USD_CHF" => "frxUSDCHF".to_string(),
        "AUD_USD" => "frxAUDUSD".to_string(),
        "NZD_USD" => "frxNZDUSD".to_string(),
        "USD_CAD" => "frxUSDCAD".to_string(),
        "XAU_USD" | "XAUUSD" => "frxXAUUSD".to_string(),
        s => s.to_string(),
    }
}

fn from_deriv_symbol(symbol: &str) -> String {
    match symbol {
        "frxEURUSD" => "EUR_USD".to_string(),
        "frxGBPUSD" => "GBP_USD".to_string(),
        "frxUSDJPY" => "USD_JPY".to_string(),
        "frxUSDCHF" => "USD_CHF".to_string(),
        "frxAUDUSD" => "AUD_USD".to_string(),
        "frxNZDUSD" => "NZD_USD".to_string(),
        "frxUSDCAD" => "USD_CAD".to_string(),
        "frxXAUUSD" => "XAU_USD".to_string(),
        s => s.to_string(),
    }
}

/// An open Deriv contract as returned by `portfolio`.
#[derive(Debug, Clone)]
pub struct DerivContract {
    pub contract_id: u64,
    pub contract_type: String,
    pub stake: f64,
    pub underlying: String,
    pub purchase_time: i64,
}

/// Live snapshot of one open contract (`proposal_open_contract`).
#[derive(Debug, Clone)]
pub struct ContractInfo {
    pub contract_id: u64,
    pub entry_spot: f64,
    pub current_spot: f64,
    pub bid_price: f64,
    pub buy_price: f64,
    pub commission: f64,
    pub is_sold: bool,
}

/// A closed contract from `profit_table`.
#[derive(Debug, Clone)]
pub struct ClosedContract {
    pub contract_id: u64,
    pub contract_type: String,
    pub underlying: String,
    pub buy_price: f64,
    pub sell_price: f64,
    pub sell_time: i64,
}

/// A single account transaction from `statement` (the Deriv "Statement" tab).
#[derive(Debug, Clone)]
pub struct StatementTx {
    pub transaction_id: u64,
    pub action_type: String, // "buy" | "sell"
    pub contract_id: u64,
    pub contract_type: String,
    pub underlying: String,
    pub amount: f64,
    pub balance_after: f64,
    pub transaction_time: i64,
    pub reference_id: u64,
}

fn granularity(tf: &str) -> u64 {
    match tf {
        "1m" => 60,
        "5m" => 300,
        "15m" => 900,
        "1h" => 3600,
        "4h" => 14400,
        "1d" => 86400,
        _ => 300,
    }
}

fn clampi(v: f64, lo: f64, hi: f64) -> f64 {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

/// Result of a successful multiplier open.
#[derive(Debug, Clone)]
pub struct OpenMult {
    pub contract_id: u64,
    pub buy_price: f64,
    pub entry: f64,
    pub commission: f64,
    pub balance_after: f64,
}

pub struct DerivFx {
    http: Http,
    token: String,
    app_id: String,
    account_id: String,
    // live tick prices per symbol (quote), refreshed by fetch_price
    tick_cache: Mutex<HashMap<String, f64>>,
    // candles cache: symbol:tf -> (fetched_at, candles)
    candle_cache: Mutex<HashMap<String, (SystemTime, Vec<Candle>)>>,
    // last known free balance, refreshed by free_balance()
    balance_cache: Mutex<Option<f64>>,
}

impl DerivFx {
    pub fn new(cfg: &Config) -> Self {
        Self {
            http: Http::builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .expect("build reqwest"),
            token: cfg.deriv_token.clone(),
            app_id: cfg.deriv_app_id.clone(),
            account_id: cfg.deriv_account_id.clone(),
            tick_cache: Mutex::new(HashMap::new()),
            candle_cache: Mutex::new(HashMap::new()),
            balance_cache: Mutex::new(None),
        }
    }

    pub fn is_configured(&self) -> bool {
        !self.token.is_empty() && !self.app_id.is_empty() && !self.account_id.is_empty()
    }

    /// POST /accounts/{id}/otp → authed WS url.
    async fn otp_url(&self) -> Result<String> {
        if !self.is_configured() {
            bail!("deriv: token/app/account not configured");
        }
        let url = format!("{REST_BASE}/accounts/{}/otp", self.account_id);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.token)
            .header("Deriv-App-ID", &self.app_id)
            .send()
            .await
            .map_err(|e| anyhow!("deriv otp request: {e}"))?
            .error_for_status()
            .map_err(|e| anyhow!("deriv otp status: {e}"))?;
        let v: Value = resp.json().await?;
        v["data"]["url"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("deriv otp: no ws url in response: {v}"))
    }

    async fn connect(&self) -> Result<Ws> {
        let url = self.otp_url().await?;
        let (ws, _resp) = connect_async(&url)
            .await
            .map_err(|e| anyhow!("deriv ws connect: {e}"))?;
        Ok(ws)
    }

    /// Send a request, return the response matching its req_id.
    async fn request(ws: &mut Ws, req: Value) -> Result<Value> {
        let req_id = req.get("req_id").and_then(|v| v.as_i64()).unwrap_or(0);
        ws.send(Message::Text(req.to_string()))
            .await
            .map_err(|e| anyhow!("deriv ws send: {e}"))?;
        loop {
            if let Some(Ok(msg)) = ws.next().await {
                match msg {
                    Message::Text(t) => {
                        let v: Value = serde_json::from_str(&t)?;
                        if v.get("req_id").and_then(|r| r.as_i64()) == Some(req_id) {
                            if let Some(e) = v.get("error") {
                                bail!("deriv {}: {}", e["code"].as_str().unwrap_or("error"), e["message"].as_str().unwrap_or_default());
                            }
                            return Ok(v);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// Compute USD SL/TP amounts for a price-based stop/target on a multiplier.
    fn limit_amounts(&self, side: OrderSide, entry: f64, sl_price: f64, tp_price: f64, stake: f64) -> Option<(f64, f64)> {
        if entry <= 0.0 || sl_price <= 0.0 || tp_price <= 0.0 {
            return None;
        }
        let (loss_dist, profit_dist) = match side {
            OrderSide::Buy => (entry - sl_price, tp_price - entry),
            OrderSide::Sell => (sl_price - entry, entry - tp_price),
        };
        if loss_dist <= 0.0 || profit_dist <= 0.0 {
            return None;
        }
        let sl_usd = (loss_dist / entry) * MULTIPLIER as f64 * stake;
        let tp_usd = (profit_dist / entry) * MULTIPLIER as f64 * stake;
        // Server caps: SL <= stake, TP <= 50*stake, both >= 0.10
        let sl_usd = clampi(sl_usd, 0.10, stake);
        let tp_usd = clampi(tp_usd, 0.10, 50.0 * stake);
        Some((round2(sl_usd), round2(tp_usd)))
    }

    /// Open a real Deriv multiplier contract.
    pub async fn open_multiplier(
        &self,
        symbol: &str,
        side: OrderSide,
        stake: f64,
        entry: f64,
        sl_price: f64,
        tp_price: f64,
    ) -> Result<OpenMult> {
        let mut ws = self.connect().await?;
        let ct = match side {
            OrderSide::Buy => "MULTUP",
            OrderSide::Sell => "MULTDOWN",
        };
        let stake = clampi(stake, 1.0, 2000.0);

        let mut limit = json!({});
        if let Some((sl_usd, tp_usd)) = self.limit_amounts(side, entry, sl_price, tp_price, stake) {
            limit = json!({"stop_loss": sl_usd, "take_profit": tp_usd});
        }
        let prop = json!({
            "proposal": 1,
            "amount": stake,
            "basis": "stake",
            "contract_type": ct,
            "currency": "USD",
            "multiplier": MULTIPLIER,
            "underlying_symbol": deriv_symbol(symbol),
            "limit_order": limit,
            "req_id": 1,
        });
        let r = Self::request(&mut ws, prop).await?;
        let pid = r["proposal"]["id"]
            .as_str()
            .ok_or_else(|| anyhow!("deriv: no proposal id: {r}"))?
            .to_string();
        let ask = r["proposal"]["ask_price"].as_f64().unwrap_or(stake);
        let prop_entry = r["proposal"]["spot"].as_f64().unwrap_or(entry);
        let commission = r["proposal"]["commission"].as_f64().unwrap_or(0.0);

        let buy = json!({"buy": pid, "price": ask, "req_id": 2});
        let rb = Self::request(&mut ws, buy).await?;
        let contract_id = rb["buy"]["contract_id"]
            .as_u64()
            .ok_or_else(|| anyhow!("deriv: no contract_id: {rb}"))?;
        let balance_after = rb["buy"]["balance_after"].as_f64().unwrap_or(0.0);
        Ok(OpenMult {
            contract_id,
            buy_price: rb["buy"]["buy_price"].as_f64().unwrap_or(ask),
            entry: prop_entry,
            commission,
            balance_after,
        })
    }

    /// Market-close a Deriv contract (sell at best). Returns the sold-for amount.
    pub async fn close_contract(&self, contract_id: u64) -> Result<f64> {
        let mut ws = self.connect().await?;
        let r = Self::request(&mut ws, json!({"sell": contract_id, "price": 0, "req_id": 3})).await?;
        let sold = r["sell"]["sold_for"]
            .as_f64()
            .ok_or_else(|| anyhow!("deriv: no sold_for: {r}"))?;
        if let Some(b) = r["sell"]["balance_after"].as_f64() {
            *self.balance_cache.lock().unwrap() = Some(b);
        }
        Ok(sold)
    }

    /// Query current open-contract snapshot (spot, bid, profit).
    pub async fn open_contract_info(&self, contract_id: u64) -> Result<Value> {
        let mut ws = self.connect().await?;
        let r = Self::request(&mut ws, json!({"proposal_open_contract": 1, "contract_id": contract_id, "req_id": 4})).await?;
        Ok(r["proposal_open_contract"].clone())
    }

    /// Real free balance from the accounts endpoint.
    pub async fn balance(&self) -> Result<f64> {
        let url = format!("{REST_BASE}/accounts");
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&self.token)
            .header("Deriv-App-ID", &self.app_id)
            .send()
            .await
            .map_err(|e| anyhow!("deriv balance request: {e}"))?
            .error_for_status()
            .map_err(|e| anyhow!("deriv balance status: {e}"))?;
        let v: Value = resp.json().await?;
        let b = &v["data"][0]["balance"];
        let bal = b
            .as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .or_else(|| b.as_f64())
            .ok_or_else(|| anyhow!("deriv: no balance in response"))?;
        *self.balance_cache.lock().unwrap() = Some(bal);
        Ok(bal)
    }

    /// Open contracts currently on the Deriv account.
    pub async fn portfolio(&self) -> Result<Vec<DerivContract>> {
        let mut ws = self.connect().await?;
        let r = Self::request(&mut ws, json!({"portfolio": 1, "req_id": 7})).await?;
        let mut out = Vec::new();
        if let Some(arr) = r["portfolio"]["contracts"].as_array() {
            for c in arr {
                if let Some(cid) = c["contract_id"].as_u64() {
                    out.push(DerivContract {
                        contract_id: cid,
                        contract_type: c["contract_type"].as_str().unwrap_or_default().to_string(),
                        stake: c["buy_price"].as_f64().unwrap_or(0.0),
                        underlying: from_deriv_symbol(c["underlying_symbol"].as_str().unwrap_or_default()),
                        purchase_time: c["purchase_time"].as_i64().unwrap_or(0),
                    });
                }
            }
        }
        Ok(out)
    }

    /// Full snapshot of a single open contract.
    pub async fn contract_info(&self, contract_id: u64) -> Result<ContractInfo> {
        let mut ws = self.connect().await?;
        let r = Self::request(
            &mut ws,
            json!({"proposal_open_contract": 1, "contract_id": contract_id, "req_id": 8}),
        )
        .await?;
        let p = &r["proposal_open_contract"];
        let f = |k: &str| p[k].as_str().and_then(|s| s.parse::<f64>().ok()).or_else(|| p[k].as_f64()).unwrap_or(0.0);
        Ok(ContractInfo {
            contract_id,
            entry_spot: f("entry_spot"),
            current_spot: f("current_spot"),
            bid_price: f("bid_price"),
            buy_price: f("buy_price"),
            commission: f("commission"),
            is_sold: p["is_sold"].as_i64().unwrap_or(0) != 0,
        })
    }

    /// Recently closed contracts with buy/sell prices.
    pub async fn profit_table(&self, limit: u32) -> Result<Vec<ClosedContract>> {
        let mut ws = self.connect().await?;
        let r = Self::request(
            &mut ws,
            json!({"profit_table": 1, "limit": limit.clamp(1, 100), "offset": 0, "description": 1, "req_id": 9}),
        )
        .await?;
        let mut out = Vec::new();
        if let Some(arr) = r["profit_table"]["transactions"].as_array() {
            for c in arr {
                if let Some(cid) = c["contract_id"].as_u64() {
                    out.push(ClosedContract {
                        contract_id: cid,
                        contract_type: c["contract_type"].as_str().unwrap_or_default().to_string(),
                        underlying: from_deriv_symbol(c["underlying_symbol"].as_str().unwrap_or_default()),
                        buy_price: c["buy_price"].as_f64().unwrap_or(0.0),
                        sell_price: c["sell_price"].as_f64().unwrap_or(0.0),
                        sell_time: c["sell_time"].as_i64().unwrap_or(0),
                    });
                }
            }
        }
        Ok(out)
    }

    /// Account transaction history — same data as the dtrader "Statement" tab
    /// (buy/sell entries with the running balance after each).
    pub async fn statement(&self, limit: u32) -> Result<Vec<StatementTx>> {
        let mut ws = self.connect().await?;
        let r = Self::request(
            &mut ws,
            json!({"statement": 1, "limit": limit.clamp(1, 500), "offset": 0, "description": 1, "req_id": 11}),
        )
        .await?;
        let mut out = Vec::new();
        if let Some(arr) = r["statement"]["transactions"].as_array() {
            for t in arr {
                let cid = t["contract_id"].as_u64().unwrap_or(0);
                if cid == 0 {
                    continue; // skip non-contract rows (deposits etc.)
                }
                out.push(StatementTx {
                    transaction_id: t["transaction_id"].as_u64().unwrap_or(0),
                    action_type: t["action_type"].as_str().unwrap_or_default().to_string(),
                    contract_id: cid,
                    contract_type: t["contract_type"].as_str().unwrap_or_default().to_string(),
                    underlying: from_deriv_symbol(t["underlying_symbol"].as_str().unwrap_or_default()),
                    amount: t["amount"].as_f64().unwrap_or(0.0),
                    balance_after: t["balance_after"].as_f64().unwrap_or(0.0),
                    transaction_time: t["transaction_time"].as_i64().unwrap_or(0),
                    reference_id: t["reference_id"].as_u64().unwrap_or(0),
                });
            }
        }
        Ok(out)
    }

    /// "OPEN", "CLOSED", or "UNKNOWN" for the FX/gold spot market based on a
    /// live ticks probe (Deriv replies `MarketIsClosed` when shut).
    pub async fn market_status(&self) -> String {
        match self.tick("EUR_USD").await {
            Ok(p) if p > 0.0 => "OPEN".to_string(),
            Err(e) => {
                let m = e.to_string();
                if m.contains("MarketIsClosed") {
                    "CLOSED".to_string()
                } else {
                    info!("market_status: unknown probe result: {m}");
                    "UNKNOWN".to_string()
                }
            }
            _ => "UNKNOWN".to_string(),
        }
    }

    /// Latest live tick (quote) for a symbol, via ticks_history style tick.
    pub async fn tick(&self, symbol: &str) -> Result<f64> {
        let mut ws = self.connect().await?;
        let r = Self::request(&mut ws, json!({"ticks": deriv_symbol(symbol), "req_id": 10})).await?;
        let q = r["tick"]["quote"]
            .as_f64()
            .or_else(|| r["tick"]["ask"].as_f64())
            .ok_or_else(|| anyhow!("deriv: no tick quote: {r}"))?;
        self.tick_cache.lock().unwrap().insert(symbol.to_string(), q);
        Ok(q)
    }

    // ── Deriv-ful price/candle data (replaces Yahoo) ───────────────────
    async fn fetch_candles_from_deriv(&self, symbol: &str, timeframe: &str, limit: u32) -> Result<Vec<Candle>> {
        let mut ws = self.connect().await?;
        let r = Self::request(
            &mut ws,
            json!({
                "ticks_history": deriv_symbol(symbol),
                "granularity": granularity(timeframe),
                "count": limit.clamp(10, 1200),
                "end": "latest",
                "style": "candles",
                "req_id": 5,
            }),
        )
        .await?;
        let arr = r["candles"]
            .as_array()
            .ok_or_else(|| anyhow!("deriv: no candles in response: {r}"))?;
        let mut out: Vec<Candle> = arr
            .iter()
            .filter_map(|c| {
                Some(Candle {
                    time: c["epoch"].as_i64()? * 1000,
                    open: c["open"].as_f64()?,
                    high: c["high"].as_f64()?,
                    low: c["low"].as_f64()?,
                    close: c["close"].as_f64()?,
                    volume: 0.0,
                })
            })
            .collect();
        out.sort_by_key(|c| c.time);
        Ok(out)
    }
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

#[async_trait::async_trait]
impl Exchange for DerivFx {
    fn name(&self) -> &'static str {
        "deriv"
    }
    fn market(&self) -> Market {
        Market::Forex
    }

    async fn fetch_candles(&self, symbol: &str, timeframe: &str, limit: u32) -> Result<Vec<Candle>> {
        let key = format!("{symbol}:{timeframe}");
        if let Some((t, v)) = self.candle_cache.lock().unwrap().get(&key) {
            if t.elapsed().unwrap_or(Duration::from_secs(u64::MAX)) < TF_SECS {
                return Ok(v.clone());
            }
        }
        let candles = self.fetch_candles_from_deriv(symbol, timeframe, limit).await?;
        self.candle_cache.lock().unwrap().insert(key, (SystemTime::now(), candles.clone()));
        Ok(candles)
    }

    async fn fetch_price(&self, symbol: &str) -> Result<f64> {
        self.tick(symbol).await
    }

    async fn ping(&self) -> bool {
        self.otp_url().await.is_ok()
    }

    async fn market_order(&self, symbol: &str, side: OrderSide, size: f64, price: f64) -> Result<Order> {
        let stake = clampi(size, 1.0, 2000.0);
        let open = self
            .open_multiplier(symbol, side, stake, price, 0.0, 0.0)
            .await?;
        Ok(Order {
            id: format!("deriv-{}", open.contract_id),
            symbol: symbol.to_string(),
            side,
            size: stake,
            price: open.entry,
            stop_loss: None,
            take_profit: None,
            status: OrderStatus::Filled,
            market: Market::from_symbol(symbol),
            created_at: chrono::Utc::now().to_rfc3339(),
        })
    }

    async fn free_balance(&self) -> Result<f64> {
        self.balance().await
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
        Config {
            oanda_token: "".into(),
            oanda_account_id: "".into(),
            oanda_practice: true,
            fx_broker: "deriv".into(),
            deriv_token: std::env::var("DERIV_TOKEN").unwrap_or_default(),
            deriv_app_id: std::env::var("DERIV_APP_ID").unwrap_or_default(),
            deriv_account_id: std::env::var("DERIV_ACCOUNT_ID").unwrap_or_default(),
            ollama_base_url: "http://localhost:11434".into(),
            ollama_model: "llama3.2:3b".into(),
            default_asset: "EUR_USD".into(),
            trading_mode: "paper".into(),
            forex_enabled: true,
            gold_enabled: true,
            default_engine: "forex".into(),
            interval_seconds: 5,
            default_timeframe: "5m".into(),
            forex_assets: vec!["EUR_USD".into(), "GBP_USD".into(), "USD_JPY".into()],
            gold_assets: vec!["XAU_USD".into()],
            max_daily_loss: 5.0,
            max_consecutive_losses: 5,
            max_open_positions: 3,
            risk_per_trade_pct: 0.5,
            data_dir: "data".into(),
            db_path: "data/trading.db".into(),
            log_dir: "logs".into(),
        }
    }

    #[tokio::test]
    #[ignore]
    async fn live_open_close_demo_contract() {
        let fx = DerivFx::new(&test_config());
        let bal = fx.balance().await;
        println!("balance: {bal:?}");
        assert!(bal.is_ok());

        let open = fx
            .open_multiplier("EUR_USD", OrderSide::Buy, 2.0, 1.16, 1.14, 1.20)
            .await
            .expect("open multiplier");
        println!("opened: {open:?}");

        let info = fx.open_contract_info(open.contract_id).await.unwrap();
        println!("info: status={} is_sold={}", info["status"], info["is_sold"]);

        let sold = fx.close_contract(open.contract_id).await;
        println!("sold: {sold:?}");
        assert!(sold.is_ok());

        let bal2 = fx.balance().await.unwrap();
        println!("balance after: {bal2}");
    }
}