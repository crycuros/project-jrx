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
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

use trading_core::{Candle, Config, Instrument, Market, Order, OrderSide, OrderStatus};

use super::Exchange;

const MULTIPLIER: u32 = 100;
const TF_SECS: Duration = Duration::from_secs(15);

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

fn as_num(v: &Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse::<f64>().ok()))
}

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
        "VOL_100_1S" | "1HZ100V" | "Volatility 100 (1s) Index" => "1HZ100V".to_string(),
        "VOL_75_1S" | "1HZ75V" | "Volatility 75 (1s) Index" => "1HZ75V".to_string(),
        "VOL_50_1S" | "1HZ50V" | "Volatility 50 (1s) Index" => "1HZ50V".to_string(),
        "VOL_25_1S" | "1HZ25V" | "Volatility 25 (1s) Index" => "1HZ25V".to_string(),
        "VOL_10_1S" | "1HZ10V" | "Volatility 10 (1s) Index" => "1HZ10V".to_string(),
        "VOL_100" | "R_100" | "Volatility 100 Index" => "R_100".to_string(),
        "VOL_75" | "R_75" | "Volatility 75 Index" => "R_75".to_string(),
        "VOL_50" | "R_50" | "Volatility 50 Index" => "R_50".to_string(),
        "VOL_25" | "R_25" | "Volatility 25 Index" => "R_25".to_string(),
        "VOL_10" | "R_10" | "Volatility 10 Index" => "R_10".to_string(),
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
        "1HZ100V" => "1HZ100V".to_string(),
        "1HZ75V" => "1HZ75V".to_string(),
        "1HZ50V" => "1HZ50V".to_string(),
        "1HZ25V" => "1HZ25V".to_string(),
        "1HZ10V" => "1HZ10V".to_string(),
        "R_100" => "R_100".to_string(),
        "R_75" => "R_75".to_string(),
        "R_50" => "R_50".to_string(),
        "R_25" => "R_25".to_string(),
        "R_10" => "R_10".to_string(),
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
    account_id: Arc<Mutex<String>>,
    // Persistent reusable WebSocket connection so we don't spam /otp 20x/sec
    ws_conn: Arc<tokio::sync::Mutex<Option<Ws>>>,
    // live tick prices per symbol (quote), refreshed by continuous tick stream
    tick_cache: Arc<Mutex<HashMap<String, f64>>>,
    // candles cache: symbol:tf -> (fetched_at, candles)
    candle_cache: Arc<Mutex<HashMap<String, (SystemTime, Vec<Candle>)>>>,
    // last known free balance, refreshed by free_balance()
    balance_cache: Mutex<Option<(SystemTime, f64)>>,
    // portfolio & profit_table caches to avoid redundant round-trips every 500ms
    portfolio_cache: Mutex<Option<(SystemTime, Vec<DerivContract>)>>,
    profit_table_cache: Mutex<Option<(SystemTime, Vec<ClosedContract>)>>,
    // statement(500) is heavy (one WS round-trip + 500 rows). Cached for
    // STATEMENT_TTL so the engine loop isn't pulling it twice a second.
    statement_cache: Mutex<Option<(SystemTime, Vec<StatementTx>)>>,
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
            account_id: Arc::new(Mutex::new(cfg.deriv_account_id.clone())),
            ws_conn: Arc::new(tokio::sync::Mutex::new(None)),
            tick_cache: Arc::new(Mutex::new(HashMap::new())),
            candle_cache: Arc::new(Mutex::new(HashMap::new())),
            balance_cache: Mutex::new(None),
            portfolio_cache: Mutex::new(None),
            profit_table_cache: Mutex::new(None),
            statement_cache: Mutex::new(None),
        }
    }

    pub fn is_configured(&self) -> bool {
        !self.token.is_empty()
    }

    /// Fetch an authenticated OTP WebSocket URL (auto-updating account_id if the configured one fails).
    async fn otp_url_static(
        http: &Http,
        token: &str,
        app_id: &str,
        account_id_lock: &Arc<Mutex<String>>,
    ) -> Result<String> {
        if token.is_empty() {
            bail!("deriv: token missing for otp");
        }
        let mut acc_id = account_id_lock.lock().unwrap().clone();
        if acc_id.is_empty() {
            let accounts_url = "https://api.derivws.com/trading/v1/options/accounts";
            if let Ok(resp) = http.get(accounts_url).bearer_auth(token).header("Deriv-App-ID", app_id).send().await {
                if let Ok(v) = resp.json::<Value>().await {
                    if let Some(id) = v["data"][0]["account_id"].as_str() {
                        acc_id = id.to_string();
                        *account_id_lock.lock().unwrap() = acc_id.clone();
                    }
                }
            }
        }
        if acc_id.is_empty() {
            bail!("deriv: account_id missing and could not be auto-discovered");
        }

        let url = format!("https://api.derivws.com/trading/v1/options/accounts/{acc_id}/otp");
        let resp = http
            .post(&url)
            .bearer_auth(token)
            .header("Deriv-App-ID", app_id)
            .send()
            .await
            .map_err(|e| anyhow!("deriv otp request: {e}"))?;

        if !resp.status().is_success() {
            // Try discovering the real account_id from /options/accounts in case .env had a stale ID
            let accounts_url = "https://api.derivws.com/trading/v1/options/accounts";
            if let Ok(acc_resp) = http.get(accounts_url).bearer_auth(token).header("Deriv-App-ID", app_id).send().await {
                if let Ok(v) = acc_resp.json::<Value>().await {
                    if let Some(real_id) = v["data"][0]["account_id"].as_str() {
                        if real_id != acc_id {
                            log::info!("deriv: updated account_id from {} to {}", acc_id, real_id);
                            *account_id_lock.lock().unwrap() = real_id.to_string();
                            let retry_url = format!("https://api.derivws.com/trading/v1/options/accounts/{real_id}/otp");
                            let retry_resp = http
                                .post(&retry_url)
                                .bearer_auth(token)
                                .header("Deriv-App-ID", app_id)
                                .send()
                                .await?
                                .error_for_status()?;
                            let rv: Value = retry_resp.json().await?;
                            return rv["data"]["url"]
                                .as_str()
                                .map(|s| s.to_string())
                                .ok_or_else(|| anyhow!("deriv otp: no ws url in retry response: {rv}"));
                        }
                    }
                }
            }
            bail!("deriv otp HTTP status {}", resp.status());
        }

        let v: Value = resp.json().await?;
        v["data"]["url"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("deriv otp: no ws url in response: {v}"))
    }

    /// Spawns a dedicated background task that connects to Deriv WebSocket
    /// and streams real-time ticks continuously via `{"ticks": sym, "subscribe": 1}`.
    pub fn spawn_tick_stream(
        &self,
        symbols: Vec<String>,
        tx: Option<tokio::sync::mpsc::UnboundedSender<trading_core::LiveTick>>,
    ) -> tokio::task::JoinHandle<()> {
        let http = self.http.clone();
        let token = self.token.clone();
        let app_id = self.app_id.clone();
        let account_id = Arc::clone(&self.account_id);
        let tick_cache = Arc::clone(&self.tick_cache);
        let candle_cache = Arc::clone(&self.candle_cache);

        tokio::spawn(async move {
            loop {
                let ws_url = match Self::otp_url_static(&http, &token, &app_id, &account_id).await {
                    Ok(u) => u,
                    Err(e) => {
                        log::warn!("Deriv tick stream OTP error: {e}");
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        continue;
                    }
                };
                log::info!("⚡ Connecting Deriv continuous tick stream via OTP WS...");
                match connect_async(&ws_url).await {
                    Ok((mut ws, _)) => {
                        log::info!("🟢 Connected to Deriv continuous tick stream for {:?}", symbols);
                        for sym in &symbols {
                            let sub_req = json!({
                                "ticks": deriv_symbol(sym),
                                "subscribe": 1,
                            });
                            let _ = ws.send(Message::Text(sub_req.to_string())).await;
                        }

                        while let Some(msg) = ws.next().await {
                            match msg {
                                Ok(Message::Text(t)) => {
                                    if let Ok(v) = serde_json::from_str::<Value>(&t) {
                                        if v.get("msg_type").and_then(|m| m.as_str()) == Some("tick") {
                                            let tick_obj = &v["tick"];
                                            let raw_sym = tick_obj["symbol"].as_str().unwrap_or_default();
                                            let sym = from_deriv_symbol(raw_sym);
                                            let price = as_num(&tick_obj["quote"]).or_else(|| as_num(&tick_obj["ask"])).unwrap_or(0.0);
                                            let epoch = tick_obj["epoch"].as_i64().unwrap_or_else(|| chrono::Utc::now().timestamp());

                                            if price > 0.0 {
                                                // 1. Update tick cache (value + freshness stamp)
                                                {
                                                    let mut tc = tick_cache.lock().unwrap();
                                                    tc.insert(sym.clone(), price);
                                                    tc.insert(format!("{sym}@ts"), stamp_secs());
                                                }

                                                // 2. Update latest open candle in memory immediately
                                                {
                                                    let mut cc = candle_cache.lock().unwrap();
                                                    for (k, (_t, candles)) in cc.iter_mut() {
                                                        if k.starts_with(&sym) {
                                                            if let Some(last) = candles.last_mut() {
                                                                last.close = price;
                                                                if price > last.high { last.high = price; }
                                                                if price < last.low { last.low = price; }
                                                                last.volume += 1.0;
                                                            }
                                                        }
                                                    }
                                                }

                                                // 3. Broadcast to channel
                                                if let Some(ref sender) = tx {
                                                    let _ = sender.send(trading_core::LiveTick {
                                                        symbol: sym,
                                                        price,
                                                        epoch,
                                                    });
                                                }
                                            }
                                        }
                                    }
                                }
                                Ok(Message::Ping(p)) => {
                                    let _ = ws.send(Message::Pong(p)).await;
                                }
                                Err(e) => {
                                    log::warn!("Deriv tick stream error: {e}");
                                    break;
                                }
                                _ => {}
                            }
                        }
                    }
                    Err(e) => {
                        log::warn!("Deriv tick stream connect error: {e}");
                    }
                }
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        })
    }

    /// POST /accounts/{id}/otp → authenticated Deriv Options WS url for PAT tokens.
    async fn otp_url(&self) -> Result<String> {
        Self::otp_url_static(&self.http, &self.token, &self.app_id, &self.account_id).await
    }

    async fn connect(&self) -> Result<Ws> {
        // Mode 1: If using a Deriv Personal Access Token (pat_...)
        if self.token.starts_with("pat_") {
            let otp_ws_url = self.otp_url().await?;
            let (ws, _) = connect_async(&otp_ws_url)
                .await
                .map_err(|e| anyhow!("deriv otp ws connect: {e}"))?;
            return Ok(ws);
        }

        // Mode 2: Standard Deriv WebSocket v3 connection
        let app_id = if !self.app_id.is_empty() && self.app_id.chars().all(|c| c.is_ascii_digit()) {
            &self.app_id
        } else {
            "1089"
        };
        let ws_url = format!("wss://ws.derivws.com/websockets/v3?app_id={app_id}");
        let (mut ws, _resp) = connect_async(&ws_url)
            .await
            .map_err(|e| anyhow!("deriv ws connect ({ws_url}): {e}"))?;

        if !self.token.is_empty() {
            let auth_req = json!({
                "authorize": self.token,
                "req_id": 9999,
            });
            match Self::request(&mut ws, auth_req).await {
                Ok(auth_resp) => {
                    if let Some(bal) = as_num(&auth_resp["authorize"]["balance"]) {
                        *self.balance_cache.lock().unwrap() = Some((SystemTime::now(), bal));
                    }
                }
                Err(e) => {
                    log::debug!("deriv authorize note: {e}");
                }
            }
        }
        Ok(ws)
    }

    /// Execute a request over the shared persistent WebSocket connection,
    /// automatically reconnecting once if the socket was closed or stale.
    async fn send_req(&self, req: Value) -> Result<Value> {
        let mut guard = self.ws_conn.lock().await;
        if guard.is_none() {
            *guard = Some(self.connect().await?);
        }
        let ws = guard.as_mut().unwrap();
        match Self::request(ws, req.clone()).await {
            Ok(v) => Ok(v),
            Err(e) => {
                let msg = e.to_string();
                // If it's a logical Deriv error (like MarketIsClosed), keep the socket alive!
                if msg.starts_with("deriv ") && !msg.contains("ws send") && !msg.contains("ws recv") && !msg.contains("ws closed") && !msg.contains("timed out") {
                    return Err(e);
                }
                // Socket error — reconnect once and retry
                log::debug!("Deriv WS reconnecting after socket error: {e}");
                *guard = None;
                let mut new_ws = self.connect().await?;
                let res = Self::request(&mut new_ws, req).await;
                if res.is_ok() {
                    *guard = Some(new_ws);
                }
                res
            }
        }
    }

    /// Send a request, return the response matching its req_id.
    async fn request(ws: &mut Ws, req: Value) -> Result<Value> {
        const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

        let req_id = req.get("req_id").and_then(|v| v.as_i64()).unwrap_or(0);
        ws.send(Message::Text(req.to_string()))
            .await
            .map_err(|e| anyhow!("deriv ws send: {e}"))?;

        let reply = async {
            loop {
                match ws.next().await {
                    Some(Ok(Message::Text(t))) => {
                        let v: Value = serde_json::from_str(&t)?;
                        if v.get("req_id").and_then(|r| r.as_i64()) == Some(req_id) {
                            if let Some(e) = v.get("error") {
                                bail!("deriv {}: {}", e["code"].as_str().unwrap_or("error"), e["message"].as_str().unwrap_or_default());
                            }
                            return Ok(v);
                        }
                    }
                    Some(Ok(Message::Ping(p))) => {
                        let _ = ws.send(Message::Pong(p)).await;
                    }
                    // Other frames: keep reading.
                    Some(Ok(_)) => {}
                    Some(Err(e)) => return Err(anyhow!("deriv ws recv: {e}")),
                    None => return Err(anyhow!("deriv ws closed awaiting req_id {req_id}")),
                }
            }
        };

        match tokio::time::timeout(REQUEST_TIMEOUT, reply).await {
            Ok(result) => result,
            Err(_) => Err(anyhow!(
                "deriv request timed out after {}s (req_id {req_id})",
                REQUEST_TIMEOUT.as_secs()
            )),
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
        let r = self.send_req(prop).await?;
        let pid = r["proposal"]["id"]
            .as_str()
            .ok_or_else(|| anyhow!("deriv: no proposal id: {r}"))?
            .to_string();
        let ask = as_num(&r["proposal"]["ask_price"]).unwrap_or(stake);
        let prop_entry = as_num(&r["proposal"]["spot"]).unwrap_or(entry);
        let commission = as_num(&r["proposal"]["commission"]).unwrap_or(0.0);

        let buy = json!({"buy": pid, "price": ask, "req_id": 2});
        let rb = self.send_req(buy).await?;
        let contract_id = rb["buy"]["contract_id"]
            .as_u64()
            .ok_or_else(|| anyhow!("deriv: no contract_id: {rb}"))?;
        let balance_after = as_num(&rb["buy"]["balance_after"]).unwrap_or(0.0);
        // Invalidate portfolio cache on new trade
        *self.portfolio_cache.lock().unwrap() = None;
        Ok(OpenMult {
            contract_id,
            buy_price: as_num(&rb["buy"]["buy_price"]).unwrap_or(ask),
            entry: prop_entry,
            commission,
            balance_after,
        })
    }

    /// Market-close a Deriv contract (sell at best). Returns the sold-for amount.
    pub async fn close_contract(&self, contract_id: u64) -> Result<f64> {
        let r = self.send_req(json!({"sell": contract_id, "price": 0, "req_id": 3})).await?;
        let sold = as_num(&r["sell"]["sold_for"])
            .ok_or_else(|| anyhow!("deriv: no sold_for: {r}"))?;
        if let Some(b) = as_num(&r["sell"]["balance_after"]) {
            *self.balance_cache.lock().unwrap() = Some((SystemTime::now(), b));
        }
        *self.portfolio_cache.lock().unwrap() = None;
        *self.profit_table_cache.lock().unwrap() = None;
        Ok(sold)
    }

    /// Query current open-contract snapshot (spot, bid, profit).
    pub async fn open_contract_info(&self, contract_id: u64) -> Result<Value> {
        let r = self.send_req(json!({"proposal_open_contract": 1, "contract_id": contract_id, "req_id": 4})).await?;
        Ok(r["proposal_open_contract"].clone())
    }

    /// Real free balance from Deriv WS or REST (cached for 5s).
    pub async fn balance(&self) -> Result<f64> {
        const BALANCE_TTL: Duration = Duration::from_secs(5);
        if let Some((t, b)) = *self.balance_cache.lock().unwrap() {
            if t.elapsed().unwrap_or(Duration::MAX) < BALANCE_TTL {
                return Ok(b);
            }
        }

        if self.token.starts_with("pat_") {
            let url = "https://api.derivws.com/trading/v1/options/accounts";
            if let Ok(resp) = self.http.get(url).bearer_auth(&self.token).header("Deriv-App-ID", &self.app_id).send().await {
                if let Ok(v) = resp.json::<Value>().await {
                    if let Some(id) = v["data"][0]["account_id"].as_str() {
                        let mut acc_guard = self.account_id.lock().unwrap();
                        if acc_guard.is_empty() {
                            *acc_guard = id.to_string();
                        }
                    }
                    if let Some(bal) = as_num(&v["data"][0]["balance"]) {
                        *self.balance_cache.lock().unwrap() = Some((SystemTime::now(), bal));
                        return Ok(bal);
                    }
                }
            }
        }

        let r = self.send_req(json!({"balance": 1, "req_id": 9998})).await?;
        let bal = as_num(&r["balance"]["balance"]).ok_or_else(|| anyhow!("deriv: no balance in response: {r}"))?;
        *self.balance_cache.lock().unwrap() = Some((SystemTime::now(), bal));
        Ok(bal)
    }

    /// Open contracts currently on the Deriv account (cached for 5s).
    pub async fn portfolio(&self) -> Result<Vec<DerivContract>> {
        const PORTFOLIO_TTL: Duration = Duration::from_secs(5);
        if let Some((t, v)) = self.portfolio_cache.lock().unwrap().as_ref() {
            if t.elapsed().unwrap_or(Duration::MAX) < PORTFOLIO_TTL {
                return Ok(v.clone());
            }
        }

        let r = self.send_req(json!({"portfolio": 1, "req_id": 7})).await?;
        let mut out = Vec::new();
        if let Some(arr) = r["portfolio"]["contracts"].as_array() {
            for c in arr {
                if let Some(cid) = c["contract_id"].as_u64() {
                    out.push(DerivContract {
                        contract_id: cid,
                        contract_type: c["contract_type"].as_str().unwrap_or_default().to_string(),
                        stake: as_num(&c["buy_price"]).unwrap_or(0.0),
                        underlying: from_deriv_symbol(c["underlying_symbol"].as_str().unwrap_or_default()),
                        purchase_time: c["purchase_time"].as_i64().unwrap_or(0),
                    });
                }
            }
        }
        *self.portfolio_cache.lock().unwrap() = Some((SystemTime::now(), out.clone()));
        Ok(out)
    }

    /// Full snapshot of a single open contract.
    pub async fn contract_info(&self, contract_id: u64) -> Result<ContractInfo> {
        let r = self.send_req(
            json!({"proposal_open_contract": 1, "contract_id": contract_id, "req_id": 8}),
        )
        .await?;
        let p = &r["proposal_open_contract"];
        let f = |k: &str| as_num(&p[k]).unwrap_or(0.0);
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

    /// Recently closed contracts with buy/sell prices (cached for 10s).
    pub async fn profit_table(&self, limit: u32) -> Result<Vec<ClosedContract>> {
        const PROFIT_TABLE_TTL: Duration = Duration::from_secs(10);
        if let Some((t, v)) = self.profit_table_cache.lock().unwrap().as_ref() {
            if t.elapsed().unwrap_or(Duration::MAX) < PROFIT_TABLE_TTL {
                return Ok(v.clone());
            }
        }

        let r = self.send_req(
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
                        buy_price: as_num(&c["buy_price"]).unwrap_or(0.0),
                        sell_price: as_num(&c["sell_price"]).unwrap_or(0.0),
                        sell_time: c["sell_time"].as_i64().unwrap_or(0),
                    });
                }
            }
        }
        *self.profit_table_cache.lock().unwrap() = Some((SystemTime::now(), out.clone()));
        Ok(out)
    }

    /// Account transaction history — same data as the dtrader "Statement" tab
    /// (buy/sell entries with the running balance after each).
    ///
    /// Served from a short-lived cache: the engine loop needs this every tick
    /// to compute NET PROFIT, and each miss is a full WS round-trip.
    pub async fn statement(&self, limit: u32) -> Result<Vec<StatementTx>> {
        const STATEMENT_TTL: Duration = Duration::from_secs(15);

        if let Some((t, v)) = self.statement_cache.lock().unwrap().as_ref() {
            if t.elapsed().unwrap_or(Duration::MAX) < STATEMENT_TTL {
                return Ok(v.clone());
            }
        }

        let r = self.send_req(
            json!({"statement": 1, "limit": limit.clamp(1, 500), "offset": 0, "description": 1, "req_id": 11}),
        )
        .await?;
        let mut out = Vec::new();
        if let Some(arr) = r["statement"]["transactions"].as_array() {
            for t in arr {
                let cid = t["contract_id"].as_u64().unwrap_or(0);
                out.push(StatementTx {
                    transaction_id: t["transaction_id"].as_u64().unwrap_or(0),
                    action_type: t["action_type"].as_str().unwrap_or_default().to_string(),
                    contract_id: cid,
                    contract_type: t["contract_type"].as_str().unwrap_or_default().to_string(),
                    underlying: from_deriv_symbol(t["underlying_symbol"].as_str().unwrap_or_default()),
                    amount: as_num(&t["amount"]).unwrap_or(0.0),
                    balance_after: as_num(&t["balance_after"]).unwrap_or(0.0),
                    transaction_time: t["transaction_time"].as_i64().unwrap_or(0),
                    reference_id: t["reference_id"].as_u64().unwrap_or(0),
                });
            }
        }
        *self.statement_cache.lock().unwrap() = Some((SystemTime::now(), out.clone()));
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

    /// Latest live tick (quote) for a symbol.
    pub async fn tick(&self, symbol: &str) -> Result<f64> {
        const TICK_MAX_STALE: Duration = Duration::from_secs(5);

        // Fast path: a live streamed tick.
        {
            let cache = self.tick_cache.lock().unwrap();
            if let (Some(&price), Some(&stamp)) =
                (cache.get(symbol), cache.get(&format!("{symbol}@ts")))
            {
                let age = stamp_secs() - stamp;
                if price > 0.0 && age >= 0.0 && age < TICK_MAX_STALE.as_secs_f64() {
                    return Ok(price);
                }
            }
        }

        let r = self.send_req(json!({"ticks": deriv_symbol(symbol), "req_id": 10})).await?;
        let q = as_num(&r["tick"]["quote"])
            .or_else(|| as_num(&r["tick"]["ask"]))
            .ok_or_else(|| anyhow!("deriv: no tick quote: {r}"))?;
        {
            let mut cache = self.tick_cache.lock().unwrap();
            cache.insert(symbol.to_string(), q);
            cache.insert(format!("{symbol}@ts"), stamp_secs());
        }

        // Instantly update the latest candle in cache with this new tick
        let mut cc = self.candle_cache.lock().unwrap();
        for (k, (_t, candles)) in cc.iter_mut() {
            if k.starts_with(symbol) {
                if let Some(last) = candles.last_mut() {
                    last.close = q;
                    if q > last.high { last.high = q; }
                    if q < last.low { last.low = q; }
                    last.volume += 1.0;
                }
            }
        }

        Ok(q)
    }

    // ── Deriv-ful price/candle data (replaces Yahoo) ───────────────────
    async fn fetch_candles_from_deriv(&self, symbol: &str, timeframe: &str, limit: u32) -> Result<Vec<Candle>> {
        let sym = deriv_symbol(symbol);
        let r = self.send_req(
            json!({
                "ticks_history": sym,
                "granularity": granularity(timeframe),
                "count": limit.clamp(10, 1200),
                "end": "latest",
                "style": "candles",
                "req_id": 5,
            }),
        )
        .await?;

        if let Some(arr) = r["candles"].as_array() {
            let mut out: Vec<Candle> = arr
                .iter()
                .filter_map(|c| {
                    Some(Candle {
                        time: c["epoch"].as_i64()? * 1000,
                        open: as_num(&c["open"])?,
                        high: as_num(&c["high"])?,
                        low: as_num(&c["low"])?,
                        close: as_num(&c["close"])?,
                        volume: 10.0,
                    })
                })
                .collect();
            out.sort_by_key(|c| c.time);
            if !out.is_empty() {
                return Ok(out);
            }
        }

        if let Some(history) = r.get("history") {
            if let (Some(prices), Some(times)) = (history["prices"].as_array(), history["times"].as_array()) {
                let mut out = Vec::new();
                for (p, t) in prices.iter().zip(times.iter()) {
                    if let (Some(px), Some(epoch)) = (as_num(p), t.as_i64()) {
                        out.push(Candle {
                            time: epoch * 1000,
                            open: px,
                            high: px,
                            low: px,
                            close: px,
                            volume: 1.0,
                        });
                    }
                }
                if !out.is_empty() {
                    return Ok(out);
                }
            }
        }

        bail!("deriv: no candles in response for {sym}: {r}")
    }

    /// Fetch real tick history from Deriv for high-frequency microstructure strategy.
    pub async fn fetch_ticks(&self, symbol: &str, count: u32) -> Result<Vec<trading_core::TickRecord>> {
        let r = self.send_req(
            json!({
                "ticks_history": deriv_symbol(symbol),
                "count": count.clamp(10, 500),
                "end": "latest",
                "style": "ticks",
                "req_id": 12,
            }),
        )
        .await?;
        let mut out = Vec::new();
        if let Some(history) = r.get("history") {
            if let (Some(prices), Some(times)) = (history["prices"].as_array(), history["times"].as_array()) {
                for (p, t) in prices.iter().zip(times.iter()) {
                    if let (Some(price), Some(epoch)) = (as_num(p), t.as_i64()) {
                        out.push(trading_core::TickRecord { price, epoch });
                    }
                }
            }
        }
        Ok(out)
    }
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// Monotonic-ish seconds marker used to age out cached ticks.
fn stamp_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
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

    async fn fetch_ticks(&self, symbol: &str, count: u32) -> Result<Vec<trading_core::TickRecord>> {
        self.fetch_ticks(symbol, count).await
    }

    async fn fetch_price(&self, symbol: &str) -> Result<f64> {
        self.tick(symbol).await
    }

    async fn ping(&self) -> bool {
        self.connect().await.is_ok()
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
        Config::from_env()
    }

    #[tokio::test]
    #[ignore]
    async fn live_open_close_demo_contract() {
        let cfg = test_config();
        println!("CFG: broker={} token_len={} app_id='{}' account_id='{}'", cfg.fx_broker, cfg.deriv_token.len(), cfg.deriv_app_id, cfg.deriv_account_id);
        let fx = DerivFx::new(&cfg);
        let otp = fx.otp_url().await;
        println!("otp_url result: {otp:?}");
        let bal = fx.balance().await;
        println!("balance result: {bal:?}");

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


    #[tokio::test]
    #[ignore]
    async fn check_live_account_pnl() {
        let cfg = test_config();
        let fx = DerivFx::new(&cfg);

        println!("=== DERIV LIVE ACCOUNT PNL REPORT ===");
        let bal = fx.balance().await;
        println!("Account Balance: {:?}", bal);

        // Open Contracts
        println!("\n--- OPEN CONTRACTS ---");
        match fx.portfolio().await {
            Ok(contracts) => {
                println!("Open count: {}", contracts.len());
                let mut total_unrealized = 0.0;
                for c in &contracts {
                    if let Ok(info) = fx.contract_info(c.contract_id).await {
                        let profit = info.bid_price - info.buy_price;
                        total_unrealized += profit;
                        println!(
                            "  ID: {} | Type: {} | Underlying: {} | Stake: ${:.2} | Spot: {:.4} | Bid: ${:.2} | PnL: ${:+.2}",
                            c.contract_id, c.contract_type, c.underlying, info.buy_price, info.current_spot, info.bid_price, profit
                        );
                    } else {
                        println!("  ID: {} | Type: {} | Underlying: {} | Stake: ${:.2}", c.contract_id, c.contract_type, c.underlying, c.stake);
                    }
                }
                println!("Total Unrealized PnL: ${:+.2}", total_unrealized);
            }
            Err(e) => println!("Error fetching portfolio: {e}"),
        }

        // Closed Contracts (Realized PnL)
        println!("\n--- RECENT CLOSED CONTRACTS (PROFIT TABLE) ---");
        match fx.profit_table(50).await {
            Ok(closed) => {
                println!("Recent closed count: {}", closed.len());
                let mut total_realized = 0.0;
                let mut wins = 0;
                let mut losses = 0;
                for c in &closed {
                    let pnl = c.sell_price - c.buy_price;
                    total_realized += pnl;
                    if pnl > 0.0 { wins += 1; } else if pnl < 0.0 { losses += 1; }
                    println!(
                        "  ID: {} | Type: {:<10} | {:<10} | Buy: ${:.2} | Sell: ${:.2} | PnL: ${:+.2}",
                        c.contract_id, c.contract_type, c.underlying, c.buy_price, c.sell_price, pnl
                    );
                }
                println!("\nTotal Realized PnL (Recent {} trades): ${:+.2}", closed.len(), total_realized);
                println!("Wins: {} | Losses: {} | Win Rate: {:.1}%", wins, losses, if wins + losses > 0 { (wins as f64 / (wins + losses) as f64) * 100.0 } else { 0.0 });
            }
            Err(e) => println!("Error fetching profit_table: {e}"),
        }
    }
}
