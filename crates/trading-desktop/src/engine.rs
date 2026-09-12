//! Background engine: market polling → strategy → risk → paper execution → publish.

use std::time::Duration;

use log::{debug, info, warn};
use tokio::task::JoinHandle;

use trading_ai::AiClient;
use trading_api::AppState;
use trading_core::{
    AiVerdict, Candle, Market, NewsItem, OrderSide, Proposal, Status, SystemHealth, Trade,
    TradeStatus, UiSnapshot,
};
use trading_db::Db;
use trading_exchange::derivfx::DerivFx;
use trading_exchange::oanda::OandaClient;
use trading_exchange::yahoofx::YahooFx;
use trading_exchange::Exchange;
use trading_risk::RiskManager;
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
            for c in &contracts {
                let ext = c.contract_id.to_string();
                if open_trades.iter().any(|t| t.external_id.as_deref() == Some(&ext))
                    || closed_trades.iter().any(|t| t.external_id.as_deref() == Some(&ext))
                {
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
                let trade = Trade {
                    external_id: Some(ext),
                    symbol: c.underlying.clone(),
                    side,
                    entry_price: info.entry_spot,
                    amount: c.stake,
                    current_price: if info.current_spot != 0.0 { Some(info.current_spot) } else { None },
                    unrealized_pnl: None,
                    status: TradeStatus::Open,
                    broker: "deriv".to_string(),
                    entry_time: fmt_time(c.purchase_time),
                    market,
                    ..Default::default()
                };
                info!("reconcile: pulled Deriv open {} {} contract_id={} stake=${:.2}", c.underlying, c.contract_type, c.contract_id, c.stake);
                /* avoid borrow issues by inserting after the struct capture */
                let mut head = trade;
                head.reason = "synced from Deriv portfolio".to_string();
                open_trades.push(head);
            }
        }
        Err(e) => warn!("deriv portfolio sync failed: {e}"),
    }
    if let Ok(closed) = deriv.profit_table(20).await {
        for cc in &closed {
            let ext = cc.contract_id.to_string();
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
    let strategy = StrategyEngine;
    let mut risk = RiskManager::new(&config);
    let ai = AiClient::new(&config);

    // Broad probe of connectivity.
    let oanda_ok = oanda.ping().await;
    let yahoo_ok = yahoo.ping().await;
    let deriv_ok = deriv.ping().await;
    let fx_ok = if use_oanda { oanda_ok } else if use_deriv { deriv_ok } else { yahoo_ok };
    let ollama_ok = ai.ping().await;
    info!("fx_broker={} oanda_ok={oanda_ok} yahoo_ok={yahoo_ok} deriv_ok={deriv_ok} ollama_ok={ollama_ok}", config.fx_broker);
    let mem = ai.memory();
    let loaded_symbols = if mem.is_empty() { 0 } else { mem.total_bars() };
    info!(
        "market memory strategy loaded ({} bars, {} patterns) -> see data/market_memory_strategy.json",
        loaded_symbols,
        mem.total_patterns()
    );

    let mut open_trades: Vec<Trade> = Vec::new();
    let mut closed_trades: Vec<Trade> = Vec::new();
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
    let mut daily_pnl = db.load_app_state().unwrap_or_default().daily_pnl;
    let mut total_pnl = 0.0;
    let mut consecutive_losses = 0;
    let kill_switch = false;
    let forex_enabled = config.forex_enabled;
    let gold_enabled = config.gold_enabled;
    let active_engine = config.default_engine.clone();

    // Pull existing Deriv positions/history into the local view so trades
    // opened outside this session (or before a restart) show up too.
    if use_deriv {
        net_deposits = reconcile_deriv(&deriv, &mut open_trades, &mut closed_trades, &mut deriv_entries).await;
    }

    let loop_interval = Duration::from_secs(config.interval_seconds);
    let mut tick = tokio::time::interval(loop_interval);

    loop {
        tick.tick().await;

        // ── 1. Status/system health ───────────────────────────────
        let market_status = if use_deriv {
            deriv.market_status().await
        } else if use_oanda {
            "UNKNOWN".to_string()
        } else {
            "UNKNOWN".to_string()
        };
        if market_status != "OPEN" {
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

        // ── 2. Build memo list of assets to poll ──────────────────
        #[allow(clippy::type_complexity)]
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

        // ── 3. Poll prices/candles ────────────────────────────────
        let mut memos: Vec<MarketMemo> = Vec::new();
        for (symbol, market) in &targets {
            let price = if use_oanda {
                oanda.fetch_price(symbol).await.ok().unwrap_or(0.0)
            } else if use_deriv {
                deriv.fetch_price(symbol).await.ok().unwrap_or(0.0)
            } else {
                yahoo.fetch_price(symbol).await.ok().unwrap_or(0.0)
            };
            memos.push(MarketMemo { symbol: symbol.clone(), market: *market, candles: vec![], price });
        }

        // ── 4. Strategy signals → risk → (AI) → paper execute ─────
        let mut order_books: std::collections::HashMap<String, trading_core::OrderBook> = std::collections::HashMap::new();
        if !kill_switch && !memos.is_empty() {
            // Fetch candles for the candidates that have a price.
            for memo in memos.iter_mut() {
                let mut candles: Option<Vec<trading_core::Candle>> = match memo.market {
                    Market::Forex | Market::Gold => {
                        if use_oanda {
                            oanda.fetch_candles(&memo.symbol, &config.default_timeframe, 120).await.ok()
                        } else if use_deriv {
                            deriv.fetch_candles(&memo.symbol, &config.default_timeframe, 120).await.ok()
                        } else {
                            yahoo.fetch_candles(&memo.symbol, &config.default_timeframe, 120).await.ok()
                        }
                    }
                };
                // Some timeframes return no bars while the market is closed
                // (e.g. 5m). Fall back to a timeframe that does, so the chart
                // never stalls in "INITIALIZING" during the weekend.
                if use_deriv && candles.as_ref().map(|c| c.len()).unwrap_or(0) < 2 {
                    candles = deriv.fetch_candles(&memo.symbol, "1m", 120).await.ok();
                    if candles.as_ref().map(|c| c.len()).unwrap_or(0) < 2 {
                        candles = deriv.fetch_candles(&memo.symbol, "1h", 120).await.ok();
                    }
                }
                memo.candles = candles.unwrap_or_default();
                if memo.price <= 0.0 {
                    memo.price = memo.candles.last().map(|c| c.close).unwrap_or(0.0);
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
                if memo.candles.len() >= 40 {
                    if let Some(p) = strategy.process(&memo.candles, &memo.symbol, memo.market) {
                        info!("signal: {} {} conf={:.2} regime={}", p.action.as_str().to_uppercase(), p.symbol, p.confidence, p.regime);
                        proposals.push(p);
                    }
                }
            }

            for proposal in proposals {
                if open_trades.len() as i32 >= config.max_open_positions {
                    break;
                }
                // One position per symbol at a time.
                if open_trades.iter().any(|t| t.symbol == proposal.symbol) {
                    continue;
                }
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

                // AI gate (best-effort; skip when Ollama offline).
                let verdict = if ollama_ok {
                    ai.evaluate_signal(&proposal, &format!("regime {}", proposal.regime))
                        .await
                        .unwrap_or(AiVerdict {
                            decision: "WATCH".to_string(),
                            reason: "cache miss".to_string(),
                            confidence: 0.5,
                        })
                } else {
                    AiVerdict { decision: "GO".to_string(), reason: "no-ai paper".to_string(), confidence: proposal.confidence }
                };
                if verdict.decision == "NO-GO" {
                    continue;
                }

                // Quality gate.
                let quality = strategy.score_quality(proposal.action, &memos.iter().find(|m| m.symbol == proposal.symbol).map(|m| m.candles.clone()).unwrap_or_default(), &proposal.symbol, memos.iter().find(|m| m.symbol == proposal.symbol).map(|m| m.market).unwrap_or(Market::Forex));
                if !quality.approved {
                    log::debug!("quality block: {} score={:.1} reasons={:?}", proposal.symbol, quality.score, quality.reasons);
                    continue;
                }

                // Open paper position.
                let entry_time = now_iso();
                let mut trade = Trade {
                    symbol: proposal.symbol.clone(),
                    side: proposal.action,
                    entry_price: proposal.entry,
                    amount: decision.suggested_size,
                    stop_loss: proposal.stop_loss,
                    take_profit: proposal.take_profit,
                    status: TradeStatus::Open,
                    regime: proposal.regime.clone(),
                    confidence: quality.score / 100.0,
                    reason: proposal.reason.clone(),
                    ai_verdict: verdict.decision.clone(),
                    ai_reason: verdict.reason.clone(),
                    quality_score: quality.score,
                    market: memos.iter().find(|m| m.symbol == proposal.symbol).map(|m| m.market).unwrap_or(Market::Forex),
                    broker: if use_deriv { "deriv".to_string() } else if use_oanda { "oanda".to_string() } else { "yahoo".to_string() },
                    entry_time: entry_time.clone(),
                    current_price: Some(proposal.entry),
                    ..Default::default()
                };

                // Real Deriv demo order (non-blocking; falls back to paper on failure).
                if use_deriv && deriv_ok {
                    let stake = decision.suggested_size.clamp(1.0, 10.0);
                    let opened_on_deriv = match deriv.open_multiplier(&proposal.symbol, proposal.action, stake, proposal.entry, proposal.stop_loss, proposal.take_profit).await {
                        Ok(open) => {
                            trade.external_id = Some(open.contract_id.to_string());
                            trade.entry_price = open.entry;
                            deriv_entries.insert(open.contract_id, (open.entry, open.commission));
                            info!("⚡ DERIV OPEN {} {} @ {:.4} contract_id={} stake=${:.2} commission=${:.2}", proposal.action.as_str().to_uppercase(), proposal.symbol, open.entry, open.contract_id, stake, open.commission);
                            toast(
                                "🟢 TRADE OPENED",
                                &format!("{} {} @ {:.5} · stake ${:.2}\ncontract #{}", proposal.action.as_str().to_uppercase(), proposal.symbol, open.entry, stake, open.contract_id),
                            );
                            true
                        }
                        Err(e) => {
                            warn!("derive open failed: {e}");
                            false
                        }
                    };
                    // No paper fallback while trading on Deriv: a "position"
                    // that exists only in memory (market closed / rejected)
                    // would show as an open trade that isn't real.
                    if !opened_on_deriv {
                        continue;
                    }
                } else {
                    info!("{} {} {} @ {:.4}", proposal.action.as_str().to_uppercase(), proposal.symbol, decision.suggested_size, proposal.entry);
                }

                let db_id = db.save_trade(&trade).ok();
                let mut t = trade;
                t.id = db_id;
                open_trades.push(t.clone());
                closed_trades.push(t);
            }
        }

        // ── 5. Update open positions (mark-to-market) ─────────────
        let mut realized: Vec<(Trade, f64, f64, String)> = Vec::new();
        for pos in open_trades.iter_mut() {
            let current = memos
                .iter()
                .find(|m| m.symbol == pos.symbol)
                .map(|m| m.price)
                .unwrap_or(pos.entry_price);
            pos.current_price = Some(current);
            pos.unrealized_pnl = Some(paper_pnl(pos, current));
            // Keep the recent-trades view of this open position live too.
            if let Some(id) = pos.id {
                if let Some(entry) = closed_trades.iter_mut().find(|t| t.id == Some(id)) {
                    entry.current_price = Some(current);
                    entry.unrealized_pnl = pos.unrealized_pnl;
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
                if use_deriv {
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
                            if let Some(id) = pos.id {
                                if let Some(entry2) = closed_trades.iter_mut().find(|t| t.id == Some(id)) {
                                    entry2.unrealized_pnl = pos.unrealized_pnl;
                                }
                            }
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

        let status = Status::build(
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

        let news = synth_news(&memos);
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

/// Fire a Windows toast notification (best-effort; silent on failure).
fn toast(title: &str, body: &str) {
    use notify_rust::Notification;
    let _ = Notification::new()
        .summary(title)
        .body(body)
        .appname("JRX AI Terminal")
        .show();
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