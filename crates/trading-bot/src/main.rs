//! JRX AI 1.0 — 24/7 Cloud Headless Trading Bot for Ubuntu/EC2 Linux.
//!
//! Runs the entire trading engine (market stream, strategies, risk management,
//! auto-compounding, Deriv execution) headlessly without any display server or GPU.
//! Also hosts the Axum REST & WebSocket API on port 8080 for remote status monitoring.

use std::sync::Arc;
use std::time::{Duration, Instant};

use log::{info, warn};
use trading_api::AppState;
use trading_core::Config;
use trading_engine::{AppConfig, Engine};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let start_time = Instant::now();
    let config = Arc::new(Config::from_env());
    let app_state = AppState::new();
    let app_cfg = AppConfig {
        config: config.clone(),
        app: app_state.clone(),
    };

    println!("\n=======================================================================");
    println!("  JRX AI 1.0 — 24/7 CLOUD HEADLESS TRADING DAEMON (LINUX EC2)");
    println!("=======================================================================");
    println!("  Broker        : {}", config.fx_broker.to_uppercase());
    println!("  Mode          : {}", config.trading_mode.to_uppercase());
    println!("  Deriv Account : {}", if config.deriv_account_id.is_empty() { "NOT SET" } else { &config.deriv_account_id });
    println!("  Gold Enabled  : {}", config.gold_enabled);
    println!("  Forex Enabled : {}", config.forex_enabled);
    println!("  Risk / Trade  : {:.2}% (Auto-Compounding Enabled)", config.risk_per_trade_pct);
    println!("  Max Positions : {}", config.max_open_positions);
    println!("  Max Daily Loss: ${:.2}", config.max_daily_loss);
    println!("  Profit Lock   : +0.60R locked @ +2.0R gain");
    println!("=======================================================================\n");

    // 1. Start the Live Background Trading Engine
    info!("Starting background trading engine loop...");
    let engine = Engine::start(app_cfg.clone());
    let st = engine.app();

    // 2. Start the Axum Web/REST API in the background
    let port = std::env::var("PORT").unwrap_or_else(|_| "8080".to_string());
    let bind_addr = format!("0.0.0.0:{}", port);
    let api_state = st.clone();

    tokio::spawn(async move {
        info!("REST & WebSocket API listening on http://{}", bind_addr);
        info!("Endpoints: /api/status, /api/positions, /api/trades, /api/health, /ws");
        if let Err(e) = trading_api::serve(api_state, &bind_addr).await {
            warn!("API server ended or could not bind to {}: {}", bind_addr, e);
        }
    });

    info!("Bot is live and running. Press Ctrl+C to terminate cleanly.");

    // 3. Status monitor heartbeat loop
    let mut interval = tokio::time::interval(Duration::from_secs(15));
    loop {
        tokio::select! {
            _ = interval.tick() => {
                let uptime_secs = start_time.elapsed().as_secs();
                let hrs = uptime_secs / 3600;
                let mins = (uptime_secs % 3600) / 60;
                let secs = uptime_secs % 60;

                let model = st.model();
                let bal = model.status.balance;
                let pnl = model.status.total_pnl;
                let open_count = model.positions.len();
                let closed_count = model.recent_trades.len();
                let engine_status = if model.status.engine.is_empty() { "ONLINE" } else { &model.status.engine };

                info!(
                    "[HEARTBEAT {:02}:{:02}:{:02}] BAL: ${:.2} | PNL: {:+.2} | OPEN: {} | CLOSED: {} | ENGINE: {}",
                    hrs, mins, secs, bal, pnl, open_count, closed_count, engine_status
                );
            }
            _ = tokio::signal::ctrl_c() => {
                info!("SIGINT / Ctrl+C received. Gracefully shutting down JRX AI Bot...");
                st.set_kill_switch(true);
                tokio::time::sleep(Duration::from_millis(500)).await;
                info!("Shutdown complete. Goodbye!");
                break;
            }
        }
    }

    Ok(())
}
