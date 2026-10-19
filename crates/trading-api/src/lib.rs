//! Axum HTTP + WebSocket API for the desktop UI.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use tokio::sync::broadcast;

use trading_core::{NewsItem, Status, SystemHealth, Trade, UiSnapshot};

#[derive(Debug, Clone)]
pub struct ManualOrderRequest {
    pub symbol: String,
    pub side: trading_core::OrderSide,
    pub contract_mode: trading_core::ContractMode,
    pub stake: f64,
    pub duration_ticks: u32,
}

/// Live mutable UI-facing model, updated by the trading loop each tick.
#[derive(Debug, Clone, Default)]
pub struct AppModel {
    pub status: Status,
    pub positions: Vec<Trade>,
    pub recent_trades: Vec<Trade>,
    pub news: Vec<NewsItem>,
    pub system: SystemHealth,
    pub equity_curve: Vec<(f64, f64)>,
    pub candles: std::collections::HashMap<String, Vec<trading_core::Candle>>,
    pub order_books: std::collections::HashMap<String, trading_core::OrderBook>,
    pub historical: std::collections::HashMap<String, String>,
    pub contract_mode: trading_core::ContractMode,
    pub rise_fall_stake: f64,
    pub tick_duration: u32,
    pub pending_orders: Vec<ManualOrderRequest>,
}

/// Shared server state.
#[derive(Clone)]
pub struct AppState {
    pub model: Arc<Mutex<AppModel>>,
    pub tx: broadcast::Sender<UiSnapshot>,
    /// Emergency stop. Read by the engine loop every tick; when true, no new
    /// positions are opened and open ones are managed (not opened).
    pub kill_switch: Arc<AtomicBool>,
}

impl AppState {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(128);
        Self {
            model: Arc::new(Mutex::new(AppModel {
                contract_mode: trading_core::ContractMode::Multiplier,
                rise_fall_stake: 0.0,
                tick_duration: 5,
                ..Default::default()
            })),
            tx,
            kill_switch: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Trip or release the emergency stop.
    pub fn set_kill_switch(&self, on: bool) {
        self.kill_switch.store(on, Ordering::SeqCst);
        log::warn!("KILL SWITCH {}", if on { "ENGAGED — no new trades" } else { "released" });
    }

    pub fn kill_switch_engaged(&self) -> bool {
        self.kill_switch.load(Ordering::SeqCst)
    }

    /// Queue a manual user order for immediate background execution.
    pub fn queue_order(&self, req: ManualOrderRequest) {
        if let Ok(mut m) = self.model.lock() {
            m.pending_orders.push(req);
        }
    }

    /// Drain all queued manual orders for execution.
    pub fn take_pending_orders(&self) -> Vec<ManualOrderRequest> {
        if let Ok(mut m) = self.model.lock() {
            std::mem::take(&mut m.pending_orders)
        } else {
            Vec::new()
        }
    }

    /// Update user-selected contract mode, stake, and duration.
    pub fn set_trading_params(&self, mode: trading_core::ContractMode, stake: f64, duration: u32) {
        if let Ok(mut m) = self.model.lock() {
            m.contract_mode = mode;
            m.rise_fall_stake = stake;
            m.tick_duration = duration;
        }
    }

    /// Get current trading parameters.
    pub fn get_trading_params(&self) -> (trading_core::ContractMode, f64, u32) {
        if let Ok(m) = self.model.lock() {
            (m.contract_mode, m.rise_fall_stake, m.tick_duration)
        } else {
            (trading_core::ContractMode::Multiplier, 0.0, 5)
        }
    }

    /// Update model + broadcast a snapshot to all WS clients.
    pub fn publish(&self, snapshot: UiSnapshot) {
        if let Ok(mut m) = self.model.lock() {
            m.status = snapshot.status.clone();
            m.positions = snapshot.positions.clone();
            m.recent_trades = snapshot.recent_trades.clone();
            m.news = snapshot.news.clone();
            m.system = snapshot.system.clone();
            m.candles = snapshot.candles.clone();
            m.order_books = snapshot.order_books.clone();
            m.historical = snapshot.historical.clone();
        }
        let _ = self.tx.send(snapshot);
    }

    pub fn model(&self) -> AppModel {
        self.model.lock().unwrap().clone()
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

pub async fn serve(state: AppState, addr: &str) -> Result<()> {
    let app = api_router(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    log::info!("API listening on http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

pub fn api_router(state: AppState) -> Router {
    Router::new()
        .route("/api/status", get(get_status))
        .route("/api/positions", get(get_positions))
        .route("/api/trades", get(get_trades))
        .route("/api/news", get(get_news))
        .route("/api/health", get(get_health))
        .route("/api/equity", get(get_equity))
        .route("/api/engine", post(set_engine))
        .route("/api/killswitch", post(set_killswitch))
        .route("/api/order", post(place_manual_order))
        .route("/apis", get(|| async { "trading-ai-rust api ok" }))
        .route("/ws", get(ws_handler))
        .with_state(state)
}

// ── REST handlers ───────────────────────────────────────────

async fn get_status(State(st): State<AppState>) -> impl IntoResponse {
    Json(st.model().status)
}

async fn get_positions(State(st): State<AppState>) -> impl IntoResponse {
    Json(st.model().positions)
}

async fn get_trades(State(st): State<AppState>) -> impl IntoResponse {
    Json(st.model().recent_trades)
}

async fn get_news(State(st): State<AppState>) -> impl IntoResponse {
    Json(st.model().news)
}

async fn get_health(State(st): State<AppState>) -> impl IntoResponse {
    Json(st.model().system)
}

async fn get_equity(State(st): State<AppState>) -> impl IntoResponse {
    Json(st.model().equity_curve)
}

#[derive(Deserialize)]
struct EngineReq {
    engine: String,
}

async fn set_engine(State(st): State<AppState>, Json(req): Json<EngineReq>) -> impl IntoResponse {
    let mut m = st.model.lock().unwrap();
    m.status.active_engine = req.engine.clone();
    m.status.engine = req.engine;
    (StatusCode::OK, Json(serde_json::json!({"ok": true})))
}

#[derive(Deserialize)]
struct KillSwitchReq {
    enabled: bool,
}

async fn set_killswitch(State(st): State<AppState>, Json(req): Json<KillSwitchReq>) -> impl IntoResponse {
    st.set_kill_switch(req.enabled);
    (StatusCode::OK, Json(serde_json::json!({"ok": true, "kill_switch": req.enabled})))
}

#[derive(Deserialize)]
struct OrderReq {
    symbol: String,
    side: String,
    stake: Option<f64>,
}

async fn place_manual_order(State(st): State<AppState>, Json(req): Json<OrderReq>) -> impl IntoResponse {
    let side = if req.side.eq_ignore_ascii_case("sell") {
        trading_core::OrderSide::Sell
    } else {
        trading_core::OrderSide::Buy
    };
    let stake = req.stake.unwrap_or(2.0).clamp(1.0, 2000.0);
    st.queue_order(ManualOrderRequest {
        symbol: req.symbol.clone(),
        side,
        contract_mode: trading_core::ContractMode::Multiplier,
        stake,
        duration_ticks: 5,
    });
    log::info!("Queued manual order: {} {} stake ${:.2}", side.as_str().to_uppercase(), req.symbol, stake);
    (StatusCode::OK, Json(serde_json::json!({
        "ok": true,
        "symbol": req.symbol,
        "side": side.as_str(),
        "stake": stake
    })))
}

// ── WebSocket ───────────────────────────────────────────────

async fn ws_handler(ws: WebSocketUpgrade, State(st): State<AppState>) -> Response {
    ws.on_upgrade(move |socket| ws_loop(socket, st))
}

async fn ws_loop(mut socket: WebSocket, st: AppState) {
    let mut rx = st.tx.subscribe();
    // Initial catch-up payload (drop the guard before awaiting I/O).
    let initial_payload = {
        let model = st.model.lock().unwrap();
        let snap = UiSnapshot {
            status: model.status.clone(),
            positions: model.positions.clone(),
            recent_trades: model.recent_trades.clone(),
            news: model.news.clone(),
            system: model.system.clone(),
            candles: model.candles.clone(),
            order_books: model.order_books.clone(),
            historical: model.historical.clone(),
        };
        serde_json::to_string(&snap).unwrap_or_default()
    };
    let _ = socket.send(Message::Text(initial_payload.into())).await;
    loop {
        tokio::select! {
            msg = rx.recv() => {
                match msg {
                    Ok(snapshot) => {
                        if let Ok(text) = serde_json::to_string(&snapshot) {
                            if socket.send(Message::Text(text.into())).await.is_err() {
                                break;
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(_)) => break,
                }
            }
        }
    }
}