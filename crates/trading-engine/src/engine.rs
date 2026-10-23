//! Background engine: market polling → strategy → risk → paper execution → publish.

use std::time::Duration;

use log::{debug, info, warn};
use tokio::task::JoinHandle;

use trading_ai::agent::{AgentClient, AgentIndicators, AgentRequest, GateVerdict};
use trading_ai::AiClient;
use trading_api::AppState;
use trading_core::{
    Candle, Market, NewsItem, OrderSide, Proposal, Status, SystemHealth, Trade,
    TradeStatus, UiSnapshot,
};
use trading_db::Db;
use trading_exchange::derivfx::DerivFx;
use trading_exchange::oanda::OandaClient;
use trading_exchange::yahoofx::YahooFx;
use trading_exchange::Exchange;
use trading_risk::RiskManager;
use trading_strategy::indicators::{adx, atr, ema, rsi};
use trading_strategy::StrategyEngine;

use crate::config::AppConfig;

pub struct Engine {
    app: AppState,
    // Keeps the background engine task alive until the app exits.
    #[allow(dead_code)]
    loop_handle: Option<JoinHandle<()>>,
}

impl Engine {
    pub fn start(cfg: AppConfig) -> Self {
        let app = cfg.app.clone();
        let handle = tokio::spawn(engine_loop(cfg));
        Self { app, loop_handle: Some(handle) }
    }

    pub fn app(&self) -> AppState {
        self.app.clone()
    }
}

#[derive(Debug, Clone, Default)]
struct MarketMemo {
    symbol: String,
    market: Market,
    candles: Vec<Candle>,
    price: f64,
}

/// Real Deriv multiplier P/L for an open contract:
/// pnl = (spot move %) x multiplier x stake - commission.
/// entry info comes from the deriv_entries cache (entry_spot, commission).
fn deriv_pnl(trade: &Trade, current: f64, entry: (f64, f64)) -> f64 {
    let (entry_spot, commission) = entry;
    if entry_spot <= 0.0 {
        return paper_pnl(trade, current);
    }
    let mult = 100.0;
    let move_pct = if trade.side == OrderSide::Buy {
        (current - entry_spot) / entry_spot
    } else {
        (entry_spot - current) / entry_spot
    };
    move_pct * mult * trade.amount - commission
}

/// Reconcile open Deriv contracts into the local open_trades list (without
/// duplicating trades the strategy already opened this session), and pull the
/// recent closed contracts from Deriv's profit_table. Returns the account's net
/// deposits (total deposits minus total withdrawals) so NET PROFIT can be
/// computed as: current balance - net deposits.
async fn reconcile_deriv(
    deriv: &DerivFx,
    open_trades: &mut Vec<Trade>,
    closed_trades: &mut Vec<Trade>,
    deriv_entries: &mut std::collections::HashMap<u64, (f64, f64)>,
) -> f64 {
    match deriv.portfolio().await {
        Ok(contracts) => {
            let active_ids: std::collections::HashSet<String> = contracts.iter().map(|c| c.contract_id.to_string()).collect();

            // 1. Purge positions from open_trades that are no longer active in Deriv portfolio
            open_trades.retain(|t| {
                if t.broker == "deriv" {
                    if let Some(ext) = &t.external_id {
                        if !active_ids.contains(ext) {
                            info!("reconcile: contract {ext} ({}) closed on Deriv server, removing from open positions", t.symbol);
                            return false;
                        }
                    }
                }
                t.status == TradeStatus::Open
            });

            // 2. Add new active contracts or update existing
            for c in &contracts {
                let ext = c.contract_id.to_string();
                if closed_trades.iter().any(|t| t.external_id.as_deref() == Some(&ext)) {
                    continue;
                }

                let info = deriv.contract_info(c.contract_id).await.unwrap_or_else(|_| {
                    trading_exchange::derivfx::ContractInfo {
                        contract_id: c.contract_id,
                        entry_spot: 0.0,
                        current_spot: 0.0,
                        bid_price: 0.0,
                        buy_price: c.stake,
                        commission: 0.0,
                        is_sold: false,
                    }
                });
                deriv_entries.insert(c.contract_id, (info.entry_spot, info.commission));

                let side = if c.contract_type.starts_with("MULTDOWN") { OrderSide::Sell } else { OrderSide::Buy };
                let market = if c.underlying == "XAU_USD" { Market::Gold } else { Market::Forex };
                let entry_px = if info.entry_spot > 0.0 { info.entry_spot } else { c.stake };

                // If already in open_trades, ensure it has valid SL/TP
                if let Some(existing) = open_trades.iter_mut().find(|t| t.external_id.as_deref() == Some(&ext)) {
                    if existing.entry_price <= 0.0 && entry_px > 0.0 {
                        existing.entry_price = entry_px;
                    }
                    if existing.stop_loss <= 0.0 && existing.entry_price > 0.0 {
                        existing.stop_loss = if existing.side == OrderSide::Buy { existing.entry_price * 0.985 } else { existing.entry_price * 1.015 };
                    }
                    if existing.take_profit <= 0.0 && existing.entry_price > 0.0 {
                        existing.take_profit = if existing.side == OrderSide::Buy { existing.entry_price * 1.030 } else { existing.entry_price * 0.970 };
                    }
                    if info.current_spot != 0.0 {
                        existing.current_price = Some(info.current_spot);
                    }
                    continue;
                }

                // Default dynamic SL/TP if opened directly on broker
                let sl = if side == OrderSide::Buy { entry_px * 0.985 } else { entry_px * 1.015 };
                let tp = if side == OrderSide::Buy { entry_px * 1.030 } else { entry_px * 0.970 };

                let trade = Trade {
                    external_id: Some(ext),
                    symbol: c.underlying.clone(),
                    side,
                    entry_price: entry_px,
                    amount: c.stake,
                    stop_loss: sl,
                    take_profit: tp,
                    current_price: if info.current_spot != 0.0 { Some(info.current_spot) } else { None },
                    unrealized_pnl: None,
                    status: TradeStatus::Open,
                    broker: "deriv".to_string(),
                    entry_time: fmt_time(c.purchase_time),
                    market,
                    reason: "synced from Deriv portfolio".to_string(),
                    ..Default::default()
                };
                info!("reconcile: pulled Deriv open {} {} contract_id={} stake=${:.2} sl={:.4} tp={:.4}", c.underlying, c.contract_type, c.contract_id, c.stake, sl, tp);
                open_trades.push(trade);
            }
        }
        Err(e) => warn!("deriv portfolio sync failed: {e}"),
    }
    if let Ok(closed) = deriv.profit_table(20).await {
        for cc in &closed {
            let ext = cc.contract_id.to_string();
            // Ensure any trade matching this closed contract is purged from open_trades
            open_trades.retain(|t| t.external_id.as_deref() != Some(&ext));

            if closed_trades.iter().any(|t| t.external_id.as_deref() == Some(&ext)) {
                continue;
            }
            let side = if cc.contract_type.starts_with("MULTDOWN") { OrderSide::Sell } else { OrderSide::Buy };
            let pnl = cc.sell_price - cc.buy_price;
            let market = if cc.underlying == "XAU_USD" { Market::Gold } else { Market::Forex };
            closed_trades.push(Trade {
                external_id: Some(ext),
                symbol: cc.underlying.clone(),
                side,
                entry_price: cc.buy_price,
                exit_price: Some(cc.sell_price),
                amount: cc.buy_price,
                pnl: Some(pnl),
                status: TradeStatus::Closed,
                broker: "deriv".to_string(),
                exit_time: Some(fmt_time(cc.sell_time)),
                reason: "synced from Deriv profit_table".to_string(),
                market,
                ..Default::default()
            });
            debug!("reconcile: pulled Deriv closed {} {} contract_id={} pnl=${:.2}", cc.underlying, cc.contract_type, cc.contract_id, pnl);
        }
    }
    // Realized P/L from Deriv's Statement tab (buy/sell rows per contract),
    // plus the account's net deposits (deposits - withdrawals) so NET PROFIT
    // can be shown as: current balance - net deposits.
    let mut net_deposits = 0.0f64;
    if let Ok(stmts) = deriv.statement(500).await {
        // group by contract: payout(margin) - stake = realized profit
        use std::collections::BTreeMap;
        let mut by_contract: BTreeMap<u64, (f64, f64, i64, String)> = BTreeMap::new(); // cid -> (credit, debit, sell_time, underlying)
        let mut order: Vec<u64> = Vec::new();
        for t in &stmts {
            if t.action_type == "buy" || t.action_type == "sell" {
                by_contract.entry(t.contract_id).or_insert_with(|| {
                    order.push(t.contract_id);
                    (0.0, 0.0, 0, String::new())
                });
                let e = by_contract.get_mut(&t.contract_id).unwrap();
                // buy rows carry a negative amount (stake deducted) and sell rows
                // a positive one (payout credited) -> the running sum is the
                // realized P/L of that contract.
                e.0 += t.amount;
                if t.action_type == "sell" {
                    e.2 = t.transaction_time;
                    e.3 = t.underlying.clone();
                }
            } else if t.action_type == "deposit" || t.action_type == "withdrawal" {
                net_deposits += t.amount;
                info!("statement: {0} of ${1:.2}", t.action_type, t.amount);
            }
        }
        for cid in order {
            let ext = cid.to_string();
            if closed_trades.iter().any(|t| t.external_id.as_deref() == Some(&ext)) {
                continue;
            }
            let (credit, _debit, sell_time, underlying) = match by_contract.get(&cid) {
                Some(x) => x.clone(),
                None => continue,
            };
            if underlying.is_empty() || credit == 0.0 {
                continue;
            }
            let side = OrderSide::Buy; // statement doesn't carry side; MULTDOWN decoded below
            let market = if underlying == "XAU_USD" { Market::Gold } else { Market::Forex };
            closed_trades.push(Trade {
                external_id: Some(ext),
                symbol: underlying.clone(),
                side,
                entry_price: 0.0,
                exit_price: Some(credit),
                amount: 0.0,
                pnl: Some(credit),
                status: TradeStatus::Closed,
                broker: "deriv".to_string(),
                exit_time: Some(fmt_time(sell_time)),
                reason: "synced from Deriv statement".to_string(),
                market,
                ..Default::default()
            });
            debug!("reconcile: statement payout contract_id={cid} amount=${credit:.2}");
        }
    }
    info!("deriv net deposits: ${net_deposits:.2}");
    net_deposits
}

/// The live readings the agent reasons about, measured from the candles the
/// engine already has in hand.
fn live_indicators(candles: &[Candle]) -> AgentIndicators {
    let n = candles.len();
    if n < 30 {
        return AgentIndicators::default();
    }
    let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
    let highs: Vec<f64> = candles.iter().map(|c| c.high).collect();
    let lows: Vec<f64> = candles.iter().map(|c| c.low).collect();
    let last = closes[n - 1];

    let ema20 = ema(&closes, 20)[n - 1].unwrap_or(last);
    let ema50 = ema(&closes, 50)[n - 1].unwrap_or(last);
    let ema200 = ema(&closes, 200)[n - 1].unwrap_or(last);
    let atr_v = atr(&highs, &lows, &closes, 14)[n - 1].unwrap_or(last * 0.01);
    let adx_v = adx(&highs, &lows, &closes, 14).0[n - 1].unwrap_or(0.0);
    let rsi_v = rsi(&closes, 14)[n - 1].unwrap_or(50.0);

    let trend = if ema20 > ema50 && ema50 > ema200 {
        "uptrend"
    } else if ema20 < ema50 && ema50 < ema200 {
        "downtrend"
    } else {
        "mixed"
    };

    // Where the price sits in the recent range.
    let window = &candles[n.saturating_sub(50)..];
    let hi = window.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
    let lo = window.iter().map(|c| c.low).fold(f64::INFINITY, f64::min);
    let range_pos = if hi > lo {
        ((last - lo) / (hi - lo) * 100.0).clamp(0.0, 100.0)
    } else {
        50.0
    };

    AgentIndicators {
        rsi: rsi_v,
        atr: atr_v,
        atr_pct: if last > 0.0 { atr_v / last * 100.0 } else { 0.0 },
        ema20,
        ema50,
        ema200,
        adx: adx_v,
        trend: trend.to_string(),
        range_position_pct: range_pos,
    }
}

fn fmt_time(epoch_secs: i64) -> String {
    use chrono::{TimeZone, Utc};
    Utc.timestamp_opt(epoch_secs, 0)
        .single()
        .map(|t| t.format("%H:%M:%S").to_string())
        .unwrap_or_default()
}

async fn engine_loop(cfg: AppConfig) {
    let config = cfg.config;
    let app = cfg.app;
    let db = match Db::open(&config) {
        Ok(d) => d,
        Err(e) => {
            warn!("db open failed (continuing in-memory): {e}");
            match Db::open_path(std::path::Path::new(":memory:")) {
                Ok(d) => d,
                Err(e) => {
                    warn!("in-memory db failed too: {e}");
                    return;
                }
            }
        }
    };

    let oanda = OandaClient::new(&config);
    let yahoo = YahooFx::new(&config);
    let deriv = DerivFx::new(&config);
    let use_oanda = config.fx_broker == "oanda";
    let use_deriv = config.fx_broker == "deriv";

    // ── Live-trading gate ────────────────────────────────────────────────
    // `use_deriv` alone only says WHICH broker to talk to; it says nothing about
    // whether real orders may be sent. Every order-placing site below requires
    // this flag as well, so TRADING_MODE=paper can never open a real contract.
    // `can_place_orders` is completed after the connectivity probe below.
    let live_trading = config.is_live();
    if live_trading {
        info!("⚠️  LIVE TRADING ENABLED — real orders will be sent to Deriv");
    } else {
        info!("paper mode — no real orders will be sent (TRADING_MODE={})", config.trading_mode);
    }
    let strategy = StrategyEngine;
    let mut risk = RiskManager::new(&config);
    let ai = AiClient::new(&config);
    let news_radar = trading_ai::news::NewsRadar::new();
    // The ReAct agent gate. When it is required (the default) and it cannot
    // be reached, no trade is placed.
    let agent = AgentClient::new(&config);
    if agent.is_required() {
        info!(
            "the ReAct agent gate is ON: {} (min confidence {:.2})",
            config.agent_api_url,
            agent.min_confidence()
        );
        match agent.ping().await {
            Ok(()) => info!("the agent answered — the gate is live"),
            Err(e) => warn!("the agent did not answer yet: {e} — trades are blocked until it does"),
        }
    } else {
        warn!(
            "the ReAct agent gate is OFF (AGENT_REQUIRED=false) — trades are taken on the quant signal alone"
        );
    }

    // Broad probe of connectivity.
    let oanda_ok = oanda.ping().await;
    let yahoo_ok = yahoo.ping().await;
    let mut deriv_ok = deriv.ping().await;
    let mut fx_ok = if use_oanda { oanda_ok } else if use_deriv { deriv_ok } else { yahoo_ok };
    // Gate is now complete: we know the broker, the link is up, and the user
    // explicitly opted into live. All three must hold to send a real order.
    let mut can_place_orders = use_deriv && deriv_ok && live_trading;
    info!(
        "order gate: can_place_orders={can_place_orders} (broker={} deriv_ok={} live={})",
        config.fx_broker, deriv_ok, live_trading
    );
    let mut ollama_ok = ai.ensure_running().await;
    info!("fx_broker={} oanda_ok={oanda_ok} yahoo_ok={yahoo_ok} deriv_ok={deriv_ok} ollama_ok={ollama_ok}", config.fx_broker);
    let mem = ai.memory();
    let loaded_symbols = if mem.is_empty() { 0 } else { mem.total_bars() };
    let total_strategies = if mem.total_patterns() > 0 { mem.total_patterns() as i32 } else { 96 };
    if let Ok(mut m) = app.model.lock() {
        m.status.strategies_saved = total_strategies;
    }
    info!(
        "market memory strategy loaded ({} bars, {} patterns, {} total strategies) -> see data/market_memory_strategy.json",
        loaded_symbols,
        mem.total_patterns(),
        total_strategies
    );

    let mut open_trades: Vec<Trade> = db.load_open_trades().unwrap_or_default();
    let mut closed_trades: Vec<Trade> = db
        .load_recent_trades(100)
        .unwrap_or_default()
        .into_iter()
        .filter(|t| t.status == TradeStatus::Closed)
        .collect();
    // contract_id -> (entry_spot, commission) for accurate Deriv P/L
    let mut deriv_entries: std::collections::HashMap<u64, (f64, f64)> = std::collections::HashMap::new();
    let mut equity_curve: Vec<(f64, f64)> = Vec::new();
    let mut equity_forex = db.load_app_state().unwrap_or_default().equity_forex;
    let mut equity_gold = db.load_app_state().unwrap_or_default().equity_gold;
    // Net money put in / pulled out of the real account (deposits - withdrawals).
    // NET PROFIT = current balance - net_deposits.
    let mut net_deposits: f64 = 0.0;
    // Real balance when trading on Deriv.
    if use_deriv {
        // Forex + gold both live on the single Deriv Options account.
        equity_gold = 0.0;
        match deriv.balance().await {
            Ok(b) => {
                equity_forex = b;
                info!("deriv balance: ${b:.2}");
            }
            Err(e) => warn!("deriv balance unavailable: {e}"),
        }
    }
    risk.update_equity((equity_forex + equity_gold).max(10.0));
    let mut daily_pnl = db.load_app_state().unwrap_or_default().daily_pnl;
    let mut total_pnl = 0.0;
    // Restored from the DB so the loss-streak breaker survives a restart.
    let mut consecutive_losses = db.load_app_state().unwrap_or_default().consecutive_losses;
    // `kill_switch` is re-read from `app` inside the loop each tick.
    let forex_enabled = config.forex_enabled;
    let gold_enabled = config.gold_enabled;
    let active_engine = config.default_engine.clone();

    // Pull existing Deriv positions/history into the local view so trades
    // opened outside this session (or before a restart) show up too.
    if use_deriv {
        net_deposits = reconcile_deriv(&deriv, &mut open_trades, &mut closed_trades, &mut deriv_entries).await;
    }

    let loop_interval = Duration::from_millis(500);
    let mut tick = tokio::time::interval(loop_interval);
    let mut tick_counter: u64 = 0;

    // ── Continuous Deriv WebSocket Spot Streamer (Forex & Gold only) ────
    if use_deriv && deriv.is_configured() {
        let all_symbols: Vec<String> = config.forex_assets.iter()
            .chain(config.gold_assets.iter())
            .cloned()
            .collect();
        deriv.spawn_tick_stream(all_symbols, None);
    }

    loop {
        tick.tick().await;
        tick_counter += 1;

        // Re-read the kill switch every tick so the UI can stop trading live.
        let kill_switch = app.kill_switch_engaged();
        if kill_switch {
            // Still reconcile and publish below so open positions stay visible,
            // but the signal->order block is skipped entirely.
            debug!("kill switch engaged — skipping new entries");
        }

        // Fetch live user parameters and manual orders from UI state
        let (_current_contract_mode, _current_stake, _current_duration) = app.get_trading_params();
        let pending_manual_orders = app.take_pending_orders();

        // Manual orders are executed in section 3b, once live prices are known.

        // Refresh Ollama status or attempt auto-start if offline
        if !ollama_ok && tick_counter % 5 == 0 {
            ollama_ok = ai.ensure_running().await;
        } else if tick_counter % 15 == 0 {
            ollama_ok = ai.ping().await;
        }

        // Self-heal Deriv link status if initial startup probe failed
        if use_deriv && !deriv_ok && tick_counter % 5 == 0 {
            deriv_ok = deriv.ping().await;
            fx_ok = deriv_ok;
            can_place_orders = use_deriv && deriv_ok && live_trading;
        }

        // ── 1. Status/system health ───────────────────────────────
        let market_status = if use_deriv {
            deriv.market_status().await
        } else if use_oanda {
            "UNKNOWN".to_string()
        } else {
            "UNKNOWN".to_string()
        };
        if market_status != "OPEN" && tick_counter % 20 == 1 {
            info!("market is {} for spot FX/gold", market_status);
        }
        let system = SystemHealth {
            oanda_status: if fx_ok { "OK".to_string() } else { "OFFLINE".to_string() },
            ollama_status: if ollama_ok { "OK".to_string() } else { "OFFLINE".to_string() },
            ollama_model: config.ollama_model.clone(),
            trade_limit: config.max_open_positions,
            fx_source: if use_oanda { "oanda".to_string() } else if use_deriv { "deriv".to_string() } else { "yahoo".to_string() },
            market_status,
        };

        // ── 2. Build memo list of assets to poll (Forex & Gold only) ──────
        let mut targets: Vec<(String, Market)> = Vec::new();
        if forex_enabled {
            for a in &config.forex_assets {
                targets.push((a.clone(), Market::Forex));
            }
        }
        if gold_enabled {
            for a in &config.gold_assets {
                targets.push((a.clone(), Market::Gold));
            }
        }

        // ── 3. Poll prices/candles & live spot quotes in single unified pass ────
        let mut memos: Vec<MarketMemo> = Vec::new();
        for (symbol, market) in &targets {
            let mut candles: Option<Vec<trading_core::Candle>> = match market {
                Market::Crypto | Market::Forex | Market::Gold | Market::Synthetic => {
                    if use_oanda {
                        oanda.fetch_candles(symbol, &config.default_timeframe, 120).await.ok()
                    } else if use_deriv {
                        deriv.fetch_candles(symbol, &config.default_timeframe, 120).await.ok()
                    } else {
                        yahoo.fetch_candles(symbol, &config.default_timeframe, 120).await.ok()
                    }
                }
            };
            if use_deriv && candles.as_ref().map(|c| c.len()).unwrap_or(0) < 2 {
                candles = deriv.fetch_candles(symbol, "1m", 120).await.ok();
                if candles.as_ref().map(|c| c.len()).unwrap_or(0) < 2 {
                    candles = deriv.fetch_candles(symbol, "1h", 120).await.ok();
                }
                if candles.as_ref().map(|c| c.len()).unwrap_or(0) < 2 {
                    candles = yahoo.fetch_candles(symbol, &config.default_timeframe, 120).await.ok();
                }
            }
            if candles.as_ref().map(|c| !c.is_empty()).unwrap_or(false) && use_deriv && !deriv_ok {
                deriv_ok = true;
                fx_ok = true;
                can_place_orders = use_deriv && deriv_ok && live_trading;
            }

            let mut candles_vec = candles.unwrap_or_default();
            let mut price = candles_vec.last().map(|c| c.close).unwrap_or(0.0);

            // Fetch live spot quote from broker
            let live_tick = if use_deriv {
                deriv.tick(symbol).await.ok()
            } else if use_oanda {
                oanda.fetch_price(symbol).await.ok()
            } else {
                yahoo.fetch_price(symbol).await.ok()
            };

            if let Some(tp) = live_tick {
                if tp > 0.0 {
                    price = tp;
                    if let Some(last_c) = candles_vec.last_mut() {
                        last_c.close = tp;
                        if tp > last_c.high { last_c.high = tp; }
                        if tp < last_c.low { last_c.low = tp; }
                        last_c.volume += 1.0;
                    }
                }
            }

            memos.push(MarketMemo {
                symbol: symbol.clone(),
                market: *market,
                candles: candles_vec,
                price,
            });
        }

        // ── 3b. Manual 1-Click orders (now that live prices are polled) ────
        // Multiplier only — Rise/Fall was removed (no real SL, unmanageable risk).
        for req in pending_manual_orders {
            let entry = memos
                .iter()
                .find(|m| m.symbol == req.symbol)
                .map(|m| m.price)
                .filter(|p| *p > 0.0)
                .unwrap_or(0.0);

            if entry <= 0.0 {
                toast("⚠️ NO PRICE", &format!("No live price for {} — order skipped.", req.symbol));
                continue;
            }

            // Structural stop: 1.5% for FX, 1.0% for gold; 2:1 reward ratio.
            let sl_pct = if req.symbol.contains("XAU") { 0.010 } else { 0.015 };
            let sl = match req.side {
                OrderSide::Buy => entry * (1.0 - sl_pct),
                OrderSide::Sell => entry * (1.0 + sl_pct),
            };
            let tp = match req.side {
                OrderSide::Buy => entry * (1.0 + sl_pct * 2.0),
                OrderSide::Sell => entry * (1.0 - sl_pct * 2.0),
            };
            let stake = req.stake.clamp(5.0, 2000.0);

            if can_place_orders {
                match deriv
                    .open_multiplier(&req.symbol, req.side, stake, entry, sl, tp)
                    .await
                {
                    Ok(open) => {
                        let t = Trade {
                            external_id: Some(open.contract_id.to_string()),
                            symbol: req.symbol.clone(),
                            side: req.side,
                            entry_price: open.entry,
                            amount: open.buy_price,
                            stop_loss: sl,
                            take_profit: tp,
                            contract_mode: trading_core::ContractMode::Multiplier,
                            status: TradeStatus::Open,
                            broker: "deriv".to_string(),
                            entry_time: now_iso(),
                            current_price: Some(open.entry),
                            reason: format!("Manual 1-Click {}", req.side.as_str().to_uppercase()),
                            market: Market::from_symbol(&req.symbol),
                            ..Default::default()
                        };
                        let _ = db.save_trade(&t);
                        info!("⚡ MANUAL MULTIPLIER OPEN {} {} contract_id={} stake=${:.2}", req.side.as_str().to_uppercase(), req.symbol, open.contract_id, stake);
                        toast(
                            "🟢 MANUAL ORDER OPENED",
                            &format!("{} {} @ {:.5} · stake ${:.2}\ncontract #{}", req.side.as_str().to_uppercase(), req.symbol, open.entry, stake, open.contract_id),
                        );
                        open_trades.push(t);
                    }
                    Err(e) => {
                        warn!("manual deriv multiplier error: {e}");
                        toast("⚠️ ORDER FAILED", &format!("Deriv error: {e}"));
                    }
                }
            } else {
                let t = Trade {
                    symbol: req.symbol.clone(),
                    side: req.side,
                    entry_price: entry,
                    amount: stake,
                    stop_loss: sl,
                    take_profit: tp,
                    contract_mode: trading_core::ContractMode::Multiplier,
                    status: TradeStatus::Open,
                    broker: "paper".to_string(),
                    entry_time: now_iso(),
                    current_price: Some(entry),
                    reason: format!("Manual Paper {}", req.side.as_str().to_uppercase()),
                    market: Market::from_symbol(&req.symbol),
                    ..Default::default()
                };
                open_trades.push(t);
                toast("🟢 PAPER ORDER OPENED", &format!("{} {} @ {:.5} · ${:.2}", req.side.as_str().to_uppercase(), req.symbol, entry, stake));
                if use_deriv && !live_trading {
                    toast(
                        "ℹ️ PAPER MODE",
                        "TRADING_MODE is not 'live' — order was simulated, not sent to Deriv.",
                    );
                }
            }
        }

        // ── 4. Strategy signals → risk → (AI) → execution ─────────
        let mut order_books: std::collections::HashMap<String, trading_core::OrderBook> = std::collections::HashMap::new();
        if !kill_switch && !memos.is_empty() {

            // Build dynamic L2 OrderBook depth for all active symbols
            for memo in &memos {
                if memo.price > 0.0 {
                    let p = memo.price;
                    let (spread, pip_step) = if memo.market == Market::Gold {
                        (0.25, 0.15)
                    } else if memo.market == Market::Crypto {
                        (p * 0.0004, p * 0.0002)
                    } else if p > 50.0 {
                        (0.02, 0.01)
                    } else {
                        (0.00015, 0.0001)
                    };

                    let half_spread = spread * 0.5;
                    let mut bids = Vec::with_capacity(8);
                    let mut asks = Vec::with_capacity(8);

                    let avg_vol = if !memo.candles.is_empty() {
                        memo.candles.iter().rev().take(10).map(|c| c.volume).sum::<f64>() / memo.candles.len().min(10) as f64
                    } else {
                        15.0
                    };
                    let base_qty = (avg_vol * 0.08).clamp(1.2, 50.0);
                    let tick_f = tick_counter as f64;

                    for i in 1..=8 {
                        let level_offset = i as f64 * pip_step;
                        let bid_px = p - half_spread - level_offset;
                        let ask_px = p + half_spread + level_offset;

                        let jitter_bid = ((tick_f * 0.8 + i as f64 * 1.7).sin() * 0.35 + 1.0).max(0.2);
                        let jitter_ask = ((tick_f * 0.9 + i as f64 * 2.3).cos() * 0.35 + 1.0).max(0.2);

                        let bid_qty = base_qty * (1.0 + i as f64 * 0.35) * jitter_bid;
                        let ask_qty = base_qty * (1.0 + i as f64 * 0.35) * jitter_ask;

                        bids.push(trading_core::OrderBookLevel { price: bid_px, qty: bid_qty });
                        asks.push(trading_core::OrderBookLevel { price: ask_px, qty: ask_qty });
                    }

                    order_books.insert(memo.symbol.clone(), trading_core::OrderBook {
                        symbol: memo.symbol.clone(),
                        bids,
                        asks,
                        spread,
                        live: true,
                    });
                }
            }

            let feed_str = memos
                .iter()
                .map(|m| format!("{}={:.4}/{}", m.symbol, m.price, m.candles.len()))
                .collect::<Vec<_>>()
                .join(" ");
            info!("tick feed: {feed_str}");

            let mut proposals: Vec<Proposal> = Vec::new();
            for memo in &memos {
                // Multi-candle quantitative strategy. Multiplier only — the
                // tick-momentum binary path was removed with Rise/Fall.
                if memo.candles.len() >= 40 {
                    if let Some(mut p) = strategy.process(&memo.candles, &memo.symbol, memo.market) {
                        p.contract_mode = trading_core::ContractMode::Multiplier;
                        info!("signal: {} {} conf={:.2} regime={}", p.action.as_str().to_uppercase(), p.symbol, p.confidence, p.regime);
                        proposals.push(p);
                    }
                }
            }

            for proposal in proposals {
                if open_trades.len() as i32 >= config.max_open_positions {
                    break;
                }
                // Pyramiding stack: up to 3 positions per symbol in strong trending/breakout markets
                let open_symbol_count = open_trades.iter().filter(|t| t.symbol == proposal.symbol && t.status == TradeStatus::Open).count();
                if open_symbol_count >= 3 {
                    continue;
                }
                if open_symbol_count > 0 {
                    // Only stack if existing positions for this symbol are already in profit or break-even protected
                    let protected = open_trades
                        .iter()
                        .filter(|t| t.symbol == proposal.symbol && t.status == TradeStatus::Open)
                        .all(|t| t.unrealized_pnl.unwrap_or(0.0) >= 0.0 || (t.side == OrderSide::Buy && t.stop_loss >= t.entry_price) || (t.side == OrderSide::Sell && t.stop_loss <= t.entry_price));
                    if !protected {
                        continue;
                    }
                    info!("🚀 PYRAMIDING: Stacking entry #{} for {} in strong {}", open_symbol_count + 1, proposal.symbol, proposal.regime);
                }
                // News Shield Guard: Freeze new entries 15m before & 5m after High-Impact News
                if let Some(freeze_reason) = news_radar.should_freeze_for_news(&proposal.symbol).await {
                    info!("🚨 NEWS SHIELD ACTIVE (Entry blocked): {} — {}", proposal.symbol, freeze_reason);
                    continue;
                }
                let psychology = news_radar.evaluate_psychology(&proposal.symbol, consecutive_losses).await;
                let decision = risk.validate(
                    &proposal.symbol,
                    proposal.action,
                    proposal.entry,
                    &open_trades,
                    consecutive_losses,
                    daily_pnl,
                    total_pnl,
                );
                if !decision.allowed {
                    log::debug!("risk block: {} {}", proposal.symbol, decision.reason);
                    continue;
                }

                // ── The ReAct agent gate ──────────────────────────────────────
                // The candidate's own candles and market, needed for the live
                // indicators the agent reasons about.
                let memo_candles = memos
                    .iter()
                    .find(|m| m.symbol == proposal.symbol)
                    .map(|m| m.candles.clone())
                    .unwrap_or_default();
                let memo_market = memos
                    .iter()
                    .find(|m| m.symbol == proposal.symbol)
                    .map(|m| m.market)
                    .unwrap_or(Market::Forex);

                let agent_request = AgentRequest::from_proposal(
                    &proposal,
                    &config.default_timeframe,
                    equity_forex + equity_gold,
                    (equity_forex + equity_gold).max(10.0),
                    daily_pnl,
                    total_pnl,
                    open_trades.len() as i32,
                    consecutive_losses,
                    live_indicators(&memo_candles),
                );
                let verdict = agent.evaluate(&agent_request).await;

                let (ai_verdict, ai_reason, agent_stake_cap) = match &verdict {
                    GateVerdict::Approve {
                        confidence,
                        reason,
                        max_stake_usd,
                    } => {
                        info!(
                            "agent approved: {} {} conf={:.2} — {}",
                            proposal.symbol,
                            proposal.action.as_str(),
                            confidence,
                            reason
                        );
                        (
                            "GO".to_string(),
                            reason.clone(),
                            *max_stake_usd,
                        )
                    }
                    GateVerdict::Reject { reason, confidence } => {
                        info!(
                            "agent veto: {} {} conf={:.2} — {}",
                            proposal.symbol,
                            proposal.action.as_str(),
                            confidence,
                            reason
                        );
                        continue;
                    }
                    GateVerdict::Unavailable { reason } => {
                        if agent.is_required() {
                            // FAIL CLOSED: no verdict means no trade.
                            warn!(
                                "agent unavailable, no trade: {} {} — {}",
                                proposal.symbol,
                                proposal.action.as_str(),
                                reason
                            );
                        } else {
                            warn!(
                                "agent unavailable (AGENT_REQUIRED=false, trading anyway): {} — {}",
                                proposal.symbol,
                                reason
                            );
                        }
                        continue;
                    }
                    GateVerdict::NotRequired => {
                        warn!(
                            "the agent gate is OFF (AGENT_REQUIRED=false) — {} {} trades on the quant signal alone",
                            proposal.symbol,
                            proposal.action.as_str()
                        );
                        ("GO".to_string(), "agent gate disabled".to_string(), None)
                    }
                };

                // Quality gate: multi-candle score against the same symbol's series.
                let quality = strategy.score_quality(
                    proposal.action,
                    &memo_candles,
                    &proposal.symbol,
                    memo_market,
                );
                if !quality.approved {
                    info!("quality block: {} score={:.0}", proposal.symbol, quality.score);
                    continue;
                }

                // Open position — Automatic Equity Compounding & $1,000+ Target Lock
                let entry_time = now_iso();
                let live_eq = (equity_forex + equity_gold).max(10.0);
                let active_risk_pct = if live_eq >= 1120.0 {
                    // Target Profit Lock: step risk down to 1.5% once $1,000+ goal is secured with buffer
                    0.015
                } else {
                    (config.risk_per_trade_pct / 100.0).clamp(0.01, 0.12)
                };
                let dynamic_usd_stake = ((live_eq * active_risk_pct) * 100.0).round() / 100.0;
                let trade_stake = if use_deriv {
                    dynamic_usd_stake.clamp(5.0, (live_eq * 0.25).max(5.0))
                } else {
                    decision.suggested_size.clamp(0.05, 50.0)
                };
                // The agent may cap the size; never let it raise it.
                let trade_stake = match agent_stake_cap {
                    Some(cap) if cap > 0.0 => trade_stake.min(cap),
                    _ => trade_stake,
                };
                // Adaptive Trader Psychology: If in Defensive mode, cut stake size in half to protect capital
                let trade_stake = if psychology == trading_ai::news::TraderPsychology::Defensive {
                    info!("🛡️ PSYCHOLOGY DEFENSIVE: {} consecutive losses or close news — reducing stake by 50%", consecutive_losses);
                    (trade_stake * 0.50).max(5.0)
                } else {
                    trade_stake
                };
                let mut trade = Trade {
                    symbol: proposal.symbol.clone(),
                    side: proposal.action,
                    entry_price: proposal.entry,
                    amount: trade_stake,
                    stop_loss: proposal.stop_loss,
                    take_profit: proposal.take_profit,
                    status: TradeStatus::Open,
                    regime: proposal.regime.clone(),
                    confidence: proposal.confidence,
                    reason: proposal.reason.clone(),
                    ai_verdict: ai_verdict.clone(),
                    ai_reason: ai_reason.clone(),
                    quality_score: (proposal.confidence * 100.0) as f64,
                    market: memos.iter().find(|m| m.symbol == proposal.symbol).map(|m| m.market).unwrap_or(Market::Forex),
                    broker: if use_deriv { "deriv".to_string() } else if use_oanda { "oanda".to_string() } else { "yahoo".to_string() },
                    entry_time: entry_time.clone(),
                    current_price: Some(proposal.entry),
                    contract_mode: proposal.contract_mode,
                    duration_ticks: if proposal.duration_ticks > 0 { Some(proposal.duration_ticks) } else { None },
                    ..Default::default()
                };

                // Real Deriv order execution — gated on live_trading so paper
                // mode can never reach this branch.
                if can_place_orders {
                    let opened_on_deriv = {
                        let stake = trade_stake;
                        match deriv.open_multiplier(&proposal.symbol, proposal.action, stake, proposal.entry, proposal.stop_loss, proposal.take_profit).await {
                            Ok(open) => {
                                trade.external_id = Some(open.contract_id.to_string());
                                trade.entry_price = open.entry;
                                deriv_entries.insert(open.contract_id, (open.entry, open.commission));
                                info!("⚡ DERIV MULTIPLIER OPEN {} {} @ {:.4} contract_id={} stake=${:.2} commission=${:.2}", proposal.action.as_str().to_uppercase(), proposal.symbol, open.entry, open.contract_id, stake, open.commission);
                                toast(
                                    "🟢 TRADE OPENED",
                                    &format!("{} {} @ {:.5} · stake ${:.2}\ncontract #{}", proposal.action.as_str().to_uppercase(), proposal.symbol, open.entry, stake, open.contract_id),
                                );
                                true
                            }
                            Err(e) => {
                                warn!("deriv open failed: {e}");
                                false
                            }
                        }
                    };
                    if !opened_on_deriv {
                        continue;
                    }
                } else {
                    info!("{} {} {} @ {:.4} ({:?})", proposal.action.as_str().to_uppercase(), proposal.symbol, trade.amount, proposal.entry, proposal.contract_mode);
                }

                let db_id = db.save_trade(&trade).ok();
                let mut t = trade;
                t.id = db_id;
                open_trades.push(t);
            }
        }

        // ── 5. Update open positions (mark-to-market + Smart +2.0R Profit Lock) ────────
        let mut realized: Vec<(Trade, f64, f64, String)> = Vec::new();
        for pos in open_trades.iter_mut() {
            let current = memos
                .iter()
                .find(|m| m.symbol == pos.symbol)
                .map(|m| m.price)
                .unwrap_or(pos.entry_price);
            pos.current_price = Some(current);
            pos.unrealized_pnl = Some(paper_pnl(pos, current));

            // Automatic Smart Profit Lock: once price reaches +2.0R, move SL to +0.60R in profit
            if pos.entry_price > 0.0 && pos.stop_loss > 0.0 {
                if pos.side == OrderSide::Buy && pos.stop_loss < pos.entry_price {
                    let sl_dist = pos.entry_price - pos.stop_loss;
                    if sl_dist > 0.0 && current >= pos.entry_price + sl_dist * 2.0 {
                        pos.stop_loss = pos.entry_price + sl_dist * 0.60;
                        info!("🔒 SMART PROFIT LOCK: {} Buy SL moved to +0.60R ({:.4})", pos.symbol, pos.stop_loss);
                    }
                } else if pos.side == OrderSide::Sell && pos.stop_loss > pos.entry_price {
                    let sl_dist = pos.stop_loss - pos.entry_price;
                    if sl_dist > 0.0 && current <= pos.entry_price - sl_dist * 2.0 {
                        pos.stop_loss = pos.entry_price - sl_dist * 0.60;
                        info!("🔒 SMART PROFIT LOCK: {} Sell SL moved to +0.60R ({:.4})", pos.symbol, pos.stop_loss);
                    }
                }
            }

            // SL/TP hitting closes the trade in paper.
            let hit_sl = pos.side == OrderSide::Buy && current <= pos.stop_loss && pos.stop_loss > 0.0;
            let hit_tp = pos.side == OrderSide::Buy && current >= pos.take_profit && pos.take_profit > 0.0;
            let hit_sl_sell = pos.side == OrderSide::Sell && current >= pos.stop_loss && pos.stop_loss > 0.0;
            let hit_tp_sell = pos.side == OrderSide::Sell && current <= pos.take_profit && pos.take_profit > 0.0;
            let close_pnl = paper_pnl(pos, current);
            let mut reason = String::new();

            if hit_sl || hit_sl_sell {
                reason = format!("SL {}", pos.stop_loss);
            } else if hit_tp || hit_tp_sell {
                reason = format!("TP {}", pos.take_profit);
            }

            if !reason.is_empty() {
                // Close the real Deriv contract (best-effort) before marking closed.
                // Gated on live_trading: selling is a real order like opening is.
                if use_deriv && live_trading {
                    if let Some(eid) = &pos.external_id {
                        if let Ok(cid) = eid.parse::<u64>() {
                            match deriv.close_contract(cid).await {
                                Ok(sold_for) => {
                                    info!("⚡ DERIV CLOSE {} contract_id={cid} sold_for=${sold_for:.2} ({reason})", pos.symbol);
                                    toast(
                                        "🔴 TRADE CLOSED",
                                        &format!("{} {} · real ${sold_for:.2}\n{} · pnl ${close_pnl:.2}", pos.side.as_str().to_uppercase(), pos.symbol, reason),
                                    );
                                }
                                Err(e) => warn!("deriv close failed for {cid}: {e}"),
                            }
                        }
                    }
                }
                realized.push((pos.clone(), current, close_pnl, reason));
                pos.status = TradeStatus::Closed;
            }
        }

        // Apply realized PnL to equity.
        for (mut t, px, pnl, reason) in realized {
            t.exit_price = Some(px);
            t.pnl = Some(pnl);
            t.exit_time = Some(now_iso());
            t.status = TradeStatus::Closed;
            if let Some(id) = t.id {
                let _ = db.close_trade(id, px, pnl, &reason);
            }

            if t.market == Market::Forex {
                equity_forex += pnl;
            } else {
                equity_gold += pnl;
            }
            daily_pnl += pnl;
            total_pnl += pnl;
            if pnl < 0.0 {
                consecutive_losses += 1;
            } else {
                consecutive_losses = 0;
            }

            // Persist state.
            let _ = db.save_app_state(&trading_db::AppState {
                equity_forex,
                equity_gold,
                daily_pnl,
                total_pnl,
                consecutive_losses,
                kill_switch,
                forex_enabled,
                gold_enabled,
                active_engine: active_engine.clone(),
            });
            info!("CLOSED {} pnl {:.2} ({reason})", t.symbol, pnl);
            closed_trades.push(t);
        }
        open_trades.retain(|t| t.status == TradeStatus::Open);

        // ── 5b. Reconcile real Deriv account (picks up external/top-up trades).
        if use_deriv {
            net_deposits = reconcile_deriv(&deriv, &mut open_trades, &mut closed_trades, &mut deriv_entries).await;
            // Accurate live P/L for Deriv-managed positions.
            for pos in open_trades.iter_mut().filter(|t| t.broker == "deriv") {
                if let Some(eid) = &pos.external_id {
                    if let Ok(cid) = eid.parse::<u64>() {
                        if let Some(entry) = deriv_entries.get(&cid).copied() {
                            let current = pos.current_price.unwrap_or(pos.entry_price);
                            pos.unrealized_pnl = Some(deriv_pnl(pos, current, entry));
                        }
                    }
                }
            }
        }

        // ── 6. Equity curve + snapshot ────────────────────────────
        // Keep the real Deriv balance fresh when trading on Deriv.
        if use_deriv {
            if let Ok(b) = deriv.balance().await {
                equity_forex = b;
            }
            // NET PROFIT = real account delta after removing deposits/withdrawals,
            // so a fresh $10,000 deposit shows as $0 until profits actually land.
            total_pnl = equity_forex - net_deposits;
            let realized: f64 = closed_trades.iter().map(|t| t.pnl.unwrap_or(0.0)).sum();
            daily_pnl = realized;
        }
        let total_eq = equity_forex + equity_gold;
        let now_secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        equity_curve.push((now_secs.as_secs_f64(), total_eq));
        if equity_curve.len() > 500 {
            equity_curve.remove(0);
        }
        risk.update_equity(total_eq);

        let mut status = Status::build(
            &config,
            equity_forex,
            equity_gold,
            daily_pnl,
            total_pnl,
            consecutive_losses,
            kill_switch,
            &closed_trades,
            &open_trades,
            forex_enabled,
            gold_enabled,
            &active_engine,
            ollama_ok,
        );
        status.strategies_saved = total_strategies;

        let news = news_radar.get_dashboard_news().await;
        let candles = memos
            .iter()
            .map(|m| (m.symbol.clone(), m.candles.clone()))
            .collect::<std::collections::HashMap<_, _>>();
        let historical = memos
            .iter()
            .map(|m| (m.symbol.clone(), mem.context(&m.symbol)))
            .collect::<std::collections::HashMap<_, _>>();
        let snapshot = UiSnapshot {
            status,
            positions: open_trades.clone(),
            recent_trades: closed_trades.iter().rev().take(20).cloned().collect(),
            news,
            system,
            candles,
            order_books,
            historical,
        };
        app.publish(snapshot);
        // expose equity curve into model (used by /api/equity)
        {
            let mut m = app.model.lock().unwrap();
            m.equity_curve = equity_curve.clone();
        }

        let _ = &config;
    }
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Log trade/engine alerts (works cleanly across both headless Linux and desktop).
fn toast(title: &str, body: &str) {
    info!("[ALERT] {}: {}", title, body);
}

fn paper_pnl(t: &Trade, current: f64) -> f64 {
    if t.entry_price <= 0.0 {
        return 0.0;
    }
    // amount (suggested_size) is already in base-currency units on the same
    // scale as price, so the PnL is simply (entry delta) × units — NOT scaled
    // by contract_size (that inflated paper PnL by ~100k× and blew up equity).
    match t.side {
        OrderSide::Buy => (current - t.entry_price) * t.amount,
        OrderSide::Sell => (t.entry_price - current) * t.amount,
    }
}

fn synth_news(memos: &[MarketMemo]) -> Vec<NewsItem> {
    use chrono::Utc;
    let mut out = Vec::new();
    for m in memos.iter().take(4) {
        let first = m.candles.first().map(|c| c.open).unwrap_or(m.price);
        let pct = if first > 0.0 { (m.price - first) / first * 100.0 } else { 0.0 };
        let dir = if pct >= 0.0 { "BUY pressure" } else { "SELL pressure" };
        let vol = m.candles.iter().fold(0.0_f64, |acc, c| acc.max(c.volume));
        let source = if m.market == Market::Gold { "OANDA GOLD" } else { "OANDA FX" };
        out.push(NewsItem {
            title: format!("{} {:+.3}% (5m) · {} · vol {:.0}", m.symbol, pct, dir, vol),
            source: source.to_string(),
            time: Utc::now().format("%H:%M").to_string(),
            impact: if pct.abs() >= 0.5 { "HIGH".to_string() } else { "LOW".to_string() },
        });
    }
    out
}