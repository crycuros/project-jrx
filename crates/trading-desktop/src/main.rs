//! Trading AI desktop — DirectX 11/12 accelerated egui UI + in-process engine.
//!
//! Features:
//!   • DirectX 11 / 12 GPU acceleration via wgpu
//!   • 60–144+ FPS high-precision immediate-mode animations
//!   • Interactive Candlestick Chart with crosshair HUD & zoom/pan
//!   • AI Neural Memory Topology (multi-tier orbital particle graph)
//!   • Animated Pipeline DAG Console (cubic bezier packet flow)
//!   • Real-time Depth Orderbook, Risk Controls & Execution Cards

mod config;
mod engine;
mod theme;
mod ui;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;
use egui::{pos2, vec2, Color32, FontFamily, FontId, Rect, Rounding, Ui};
use trading_core::UiSnapshot;

use crate::config::AppConfig;
use crate::engine::Engine;
use crate::theme::C;
use crate::ui::chart::ChartState;
use crate::ui::ai_memory_graph::NeuralMemoryState;
use crate::ui::pipeline_console::PipelineState;
use crate::ui::toast::ToastManager;

fn main() -> eframe::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    log::info!("JRX AI 1.0 — Trading AI Terminal starting (DirectX 11/12 GPU)...");

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([1100.0, 700.0])
            .with_title("JRX AI 1.0 — Trading AI Terminal"),
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            supported_backends: wgpu::Backends::PRIMARY,
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        },
        ..Default::default()
    };

    eframe::run_native(
        "JRX AI 1.0 — Trading AI Terminal",
        native_options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}

const ONBOARD_STEPS: usize = 6;
const STEP_TITLES: [&str; ONBOARD_STEPS] = [
    "WELCOME",
    "MODE & ENGINE",
    "FOREX DATA — OANDA",
    "GOLD DATA — OANDA",
    "DERIV FX & SYNTHETICS",
    "RISK & LIMITS",
];

#[allow(dead_code)]
#[derive(Clone)]
struct OnboardState {
    step: usize,
    paper: bool,
    engine: String,
    oanda_token: String,
    oanda_account: String,
    practice: bool,
    deriv_token: String,
    deriv_app_id: String,
    deriv_account: String,
    risk_pct: String,
    max_open: String,
    max_daily: String,
    max_loss: String,
}

impl OnboardState {
    fn new(cfg: &AppConfig) -> Self {
        let c = cfg.config.as_ref();
        Self {
            step: 0,
            paper: c.trading_mode != "live",
            engine: c.default_engine.clone(),
            oanda_token: String::new(),
            oanda_account: String::new(),
            practice: c.oanda_practice,
            deriv_token: String::new(),
            deriv_app_id: String::new(),
            deriv_account: String::new(),
            risk_pct: format!("{:.1}", c.risk_per_trade_pct),
            max_open: c.max_open_positions.to_string(),
            max_daily: format!("{:.0}", c.max_daily_loss),
            max_loss: c.max_consecutive_losses.to_string(),
        }
    }
}

enum Phase {
    Setup(OnboardState),
    Live,
}

struct App {
    cfg: AppConfig,
    #[allow(dead_code)]
    rt: Option<tokio::runtime::Runtime>,
    phase: Phase,
    snapshot: UiSnapshot,
    equity: Vec<(f64, f64)>,
    sl_weight: f32,
    factor_weight: f32,
    contract_mode: trading_core::ContractMode,
    rise_fall_stake: f64,
    tick_duration: u32,
    trade_action: String,
    trade_symbol: String,
    chart_symbol: String,
    data_ready: bool,
    show_pipeline: bool,
    last_poll: Instant,

    // Component View States
    chart_state: ChartState,
    neural_state: NeuralMemoryState,
    pipeline_state: PipelineState,
    toast_manager: ToastManager,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Configure Custom JetBrains Mono Fonts
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "JetBrainsMono-Regular".to_owned(),
            egui::FontData::from_static(include_bytes!("../fonts/JetBrainsMono-Regular.ttf")),
        );
        fonts.font_data.insert(
            "JetBrainsMono-Bold".to_owned(),
            egui::FontData::from_static(include_bytes!("../fonts/JetBrainsMono-Bold.ttf")),
        );

        if let Some(vec) = fonts.families.get_mut(&FontFamily::Monospace) {
            vec.insert(0, "JetBrainsMono-Regular".to_owned());
        }
        if let Some(vec) = fonts.families.get_mut(&FontFamily::Proportional) {
            vec.insert(0, "JetBrainsMono-Regular".to_owned());
        }
        cc.egui_ctx.set_fonts(fonts);
        C::apply_visuals(&cc.egui_ctx);

        let cfg = AppConfig::new();
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        let setup = OnboardState::new(&cfg);

        let mut app = Self {
            cfg,
            rt: Some(rt),
            phase: Phase::Setup(setup),
            snapshot: UiSnapshot::default(),
            equity: Vec::new(),
            sl_weight: 2.0,
            factor_weight: 30.0,
            contract_mode: trading_core::ContractMode::Multiplier,
            rise_fall_stake: 0.0,
            tick_duration: 5,
            trade_action: "buy".to_string(),
            trade_symbol: "1HZ100V".to_string(),
            chart_symbol: "1HZ100V".to_string(),
            data_ready: false,
            show_pipeline: true,
            last_poll: Instant::now() - Duration::from_secs(5),
            chart_state: ChartState::default(),
            neural_state: NeuralMemoryState::default(),
            pipeline_state: PipelineState::default(),
            toast_manager: ToastManager::default(),
        };

        if std::path::Path::new(".env").exists() {
            app.finish_setup();
        }

        app
    }

    fn poll_snapshot(&mut self) {
        let m = self.cfg.app.model();
        self.snapshot.status = m.status;
        self.snapshot.positions = m.positions.clone();
        self.snapshot.recent_trades = m.recent_trades.clone();
        self.snapshot.news = m.news.clone();
        self.snapshot.system = m.system.clone();
        self.snapshot.candles = m.candles.clone();
        self.snapshot.order_books = m.order_books.clone();
        self.snapshot.historical = m.historical.clone();
        self.equity = m.equity_curve;

        // Transition to live terminal immediately when data or status is received
        if !self.snapshot.candles.is_empty() || self.snapshot.status.running || self.snapshot.status.balance > 0.0 || self.last_poll.elapsed() > Duration::from_millis(1200) {
            self.data_ready = true;
        }
    }

    #[allow(dead_code)]
    fn real_sl_weight(positions: &[trading_core::Trade], risk_pct: f64) -> f32 {
        let mut total = 0.0;
        let mut cnt = 0.0;
        for p in positions {
            if p.entry_price > 0.0 {
                let d = if p.stop_loss > 0.0 {
                    (p.entry_price - p.stop_loss).abs() / p.entry_price * 100.0
                } else {
                    risk_pct
                };
                total += d;
                cnt += 1.0;
            }
        }
        if cnt == 0.0 { risk_pct as f32 } else { (total / cnt).clamp(1.0, 100.0) as f32 }
    }

    #[allow(dead_code)]
    fn real_factor_weight(positions: &[trading_core::Trade], balance: f64) -> f32 {
        if balance <= 0.0 {
            return 5.0;
        }
        let committed = positions.iter().map(|p| p.amount).sum::<f64>();
        let pct = committed / balance * 100.0;
        if pct > 0.0 { pct.clamp(1.0, 100.0) as f32 } else { 5.0 }
    }

    fn finish_setup(&mut self) {
        let st = match &self.phase {
            Phase::Setup(o) => o.clone(),
            Phase::Live => return,
        };

        let mut c = self.cfg.config.as_ref().clone();
        c.default_engine = st.engine.clone();
        if !std::path::Path::new(".env").exists() {
            c.forex_enabled = st.engine != "gold";
            c.gold_enabled = st.engine == "gold";
        }
        c.trading_mode = if st.paper { "paper".to_string() } else { "live".to_string() };
        c.oanda_token = st.oanda_token.trim().to_string();
        c.oanda_account_id = st.oanda_account.trim().to_string();
        c.oanda_practice = st.practice && st.paper;

        let booted = std::path::Path::new(".env").exists();
        if !booted {
            c.fx_broker = if c.oanda_token.is_empty() { "yahoo".to_string() } else { "oanda".to_string() };
        }
        c.risk_per_trade_pct = st.risk_pct.trim().parse().unwrap_or(c.risk_per_trade_pct);
        c.max_open_positions = st.max_open.trim().parse().unwrap_or(c.max_open_positions);
        c.max_daily_loss = st.max_daily.trim().parse().unwrap_or(c.max_daily_loss);
        c.max_consecutive_losses = st.max_loss.trim().parse().unwrap_or(c.max_consecutive_losses);

        write_env(&st, &c);

        let ac = AppConfig { config: Arc::new(c), app: self.cfg.app.clone() };
        let _guard = self.rt.as_ref().expect("runtime").enter();
        let engine = Engine::start(ac.clone());
        let _app_state = engine.app();
        self.cfg = ac;
        self.phase = Phase::Live;
    }

    fn current_candles(&self) -> Vec<trading_core::Candle> {
        if let Some(c) = self.snapshot.candles.get(&self.chart_symbol) {
            if c.len() >= 2 {
                return c.clone();
            }
        }
        self.snapshot
            .candles
            .values()
            .find(|c| c.len() >= 2)
            .cloned()
            .unwrap_or_default()
    }

    fn neural_nodes(&self) -> Vec<trading_core::AiMemoryNode> {
        use trading_core::{AiMemoryNode, Market, MemoryOrbit, NodeVerdict};
        let mut nodes = Vec::new();
        let status = &self.snapshot.status;

        // Signal ring
        for t in self.snapshot.recent_trades.iter().take(6) {
            let verdict = match t.ai_verdict.as_str() {
                "GO" => NodeVerdict::Go,
                "WATCH" => NodeVerdict::Watch,
                "NO-GO" => NodeVerdict::NoGo,
                _ => NodeVerdict::Neutral,
            };
            nodes.push(AiMemoryNode {
                id: format!("sig_{}", t.symbol),
                label: format!("{} {}", t.symbol.split('/').next().unwrap_or(&t.symbol), t.side.as_str().to_uppercase()),
                orbit: MemoryOrbit::Signal,
                verdict,
                confidence: t.confidence,
                market: t.market,
                active: t.confidence > 0.65,
                detail: format!("{} {} · {}", t.side.as_str().to_uppercase(), t.symbol, t.ai_verdict),
            });
        }

        // Regime ring — driven by the mined long-term memory (weekday/month edge,
        // momentum persistence) rather than fixed placeholder confidences.
        let regime = status.active_engine.as_str();
        let memory_ctx = self
            .snapshot
            .historical
            .get(&status.asset)
            .cloned()
            .unwrap_or_default();

        // Parse the compact memory blob for a usable bullish-days% figure.
        let bullish_pct = memory_ctx
            .split("bullishDays=")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
            .and_then(|s| s.trim_end_matches('%').parse::<f64>().ok());

        let persistence = memory_ctx
            .split("persistence(win streak follow)=")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
            .and_then(|s| s.parse::<f64>().ok());

        // bullish_pct: 50% is neutral, >55% favours longs, <45% favours shorts.
        let bull_conf = bullish_pct.map(|p| ((p - 50.0) / 20.0 + 0.5).clamp(0.05, 0.95));
        let bear_conf = bullish_pct.map(|p| ((50.0 - p) / 20.0 + 0.5).clamp(0.05, 0.95));
        let persist_conf = persistence.map(|p| p.clamp(0.05, 0.95));

        let has_memory = bullish_pct.is_some();
        for (label, verdict, conf, active) in [
            ("BULL", NodeVerdict::Go, bull_conf.unwrap_or(0.50), has_memory),
            ("BEAR", NodeVerdict::NoGo, bear_conf.unwrap_or(0.50), false),
            ("PERSIST", NodeVerdict::Watch, persist_conf.unwrap_or(0.50), false),
            ("BREAKOUT", NodeVerdict::Go, 0.75, regime == "forex"),
            ("FOREX", NodeVerdict::Neutral, 0.50, status.forex_enabled),
            ("GOLD", NodeVerdict::Neutral, 0.60, status.gold_enabled),
        ] {
            nodes.push(AiMemoryNode {
                id: format!("regime_{label}"),
                label: label.to_string(),
                orbit: MemoryOrbit::Regime,
                verdict,
                confidence: conf,
                market: Market::Forex,
                active,
                detail: if has_memory {
                    format!("mined from {}+ daily bars — {}", status.asset, memory_ctx.lines().next().unwrap_or(""))
                } else {
                    format!("regime {} — no long-term memory for {}", label, status.asset)
                },
            });
        }

        // Asset universe ring
        for (symbol, candles) in self.snapshot.candles.iter() {
            let market = Market::from_symbol(symbol);
            let confidence = candles.last()
                .filter(|c| c.open > 0.0)
                .map(|c| {
                    let drift = (c.close - c.open).abs() / c.open;
                    (0.35 + drift * 12.0).clamp(0.0, 1.0)
                })
                .unwrap_or(0.35);
            let verdict = candles.last()
                .filter(|c| c.open > 0.0)
                .map(|c| if c.close > c.open { NodeVerdict::Go } else { NodeVerdict::NoGo })
                .unwrap_or(NodeVerdict::Neutral);
            let short_label = symbol.split('/').next()
                .unwrap_or(symbol)
                .split('_').next()
                .unwrap_or(symbol)
                .to_string();

            nodes.push(AiMemoryNode {
                id: format!("asset_{symbol}"),
                label: short_label,
                orbit: MemoryOrbit::Asset,
                verdict,
                confidence,
                market,
                active: symbol == &status.asset,
                detail: self.snapshot.historical.get(symbol).cloned().unwrap_or_default(),
            });
        }

        nodes
    }

    fn neural_feed(&self) -> Vec<String> {
        let mut feed = Vec::new();
        let status = &self.snapshot.status;
        if status.running {
            feed.push(format!("REGIME :: {} · AI {} · CONF {:.0}%",
                status.active_engine.to_uppercase(), status.ai_available, status.win_rate));

            let mut best: Option<(String, f64)> = None;
            for (sym, cs) in self.snapshot.candles.iter() {
                if cs.len() >= 2 {
                    let first = cs.first().unwrap().close;
                    let last = cs.last().unwrap().close;
                    if first > 0.0 {
                        let pct = (last - first) / first * 100.0;
                        if best.as_ref().map(|(_, b)| pct.abs() > b.abs()).unwrap_or(true) {
                            best = Some((sym.clone(), pct));
                        }
                    }
                }
            }
            if let Some((sym, pct)) = best.as_ref() {
                feed.push(format!("MOMENTUM LEADER {sym} {:+.2}% (5m)", pct));
            }

            if let Some(b) = self.snapshot.order_books.get(&status.asset).filter(|b| b.live) {
                let d = if b.spread < 0.01 { 8 } else if b.spread < 1.0 { 6 } else { 2 };
                feed.push(format!("SPREAD {} {:>.*} · QTY {}", b.symbol, d, b.spread, b.bids.len() + b.asks.len()));
            }

            feed.push(format!("OPEN {}/{} · DAILY {:+.2} · MARGIN {:.2}%",
                status.open_positions, self.snapshot.system.trade_limit, status.daily_pnl,
                if status.balance_total > 0.0 { status.equity_forex / status.balance_total * 100.0 } else { 0.0 }));

            feed.push(format!("WINS {:.0}% · LOSS SEQ {} · STRATEGIES SAVED {}",
                status.win_rate, status.consecutive_losses, status.strategies_saved));
        } else {
            feed.push("ENGINE IDLE — START PENDING".to_string());
        }
        feed
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // High-precision 60-120+ FPS continuous animation request
        ctx.request_repaint_after(Duration::from_millis(16));

        // Periodically poll engine model (Fast 100ms tick update)
        if matches!(self.phase, Phase::Live) && self.last_poll.elapsed() >= Duration::from_millis(100) {
            self.poll_snapshot();
            self.last_poll = Instant::now();
        }

        // Sync & auto-trigger toasts on newly opened positions
        self.toast_manager.sync_positions(&self.snapshot.positions);

        // Configure custom monochrome carbon-black background with 8px outer margin
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(C::BG).inner_margin(egui::Margin::same(8.0)))
            .show(ctx, |ui| {
                match &self.phase {
                    Phase::Setup(_) => self.setup_view(ui),
                    Phase::Live => {
                        if !self.data_ready {
                            self.loading_view(ui);
                        } else {
                            self.terminal_view(ui);
                        }
                    }
                }
            });

        // Floating toast overlay in bottom-right corner
        self.toast_manager.render(ctx);
    }
}

impl App {
    /// Full-screen high-tech boot sequence screen
    fn loading_view(&self, ui: &mut Ui) {
        let status = &self.snapshot.status;
        let health = &self.snapshot.system;
        let time = ui.input(|i| i.time) as f32;

        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() / 4.0);

            C::panel_frame().show(ui, |ui| {
                ui.set_width(520.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        egui::RichText::new("J R X   A I   T E R M I N A L")
                            .font(FontId::new(22.0, FontFamily::Monospace))
                            .color(C::WHITE)
                            .strong(),
                    );
                    ui.label(
                        egui::RichText::new("DIRECTX 11/12 GPU ACCELERATED · INITIALIZING LIVE FEED")
                            .font(FontId::new(11.0, FontFamily::Monospace))
                            .color(C::TEXT_MUTED),
                    );

                    ui.add_space(10.0);

                    // Scanning Bar
                    let (scan_rect, _) = ui.allocate_exact_size(vec2(ui.available_width() - 20.0, 3.0), egui::Sense::hover());
                    if ui.is_rect_visible(scan_rect) {
                        ui.painter().rect_filled(scan_rect, Rounding::same(2.0), C::BG_CARD);
                        let sweep = (time * 2.0).sin() * 0.5 + 0.5;
                        let scan_w = 60.0;
                        let scan_x = scan_rect.min.x + sweep * (scan_rect.width() - scan_w);
                        let laser_rect = Rect::from_min_size(pos2(scan_x, scan_rect.min.y), vec2(scan_w, 3.0));
                        ui.painter().rect_filled(laser_rect, Rounding::same(2.0), C::WHITE);
                    }

                    ui.add_space(10.0);
                    ui::components::hline(ui);
                    ui.add_space(10.0);

                    let mut stat_row = |lbl: &str, val: &str, col: Color32| {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(format!("{:<14}:", lbl)).font(FontId::new(10.0, FontFamily::Monospace)).color(C::TEXT_DIM));
                            ui.label(egui::RichText::new(val).font(FontId::new(10.0, FontFamily::Monospace)).color(col).strong());
                        });
                    };

                    stat_row("ACTIVE ENGINE", &status.active_engine.to_uppercase(), C::WHITE);
                    stat_row("TRADING MODE", &status.mode.to_uppercase(), C::TEXT_MUTED);
                    stat_row("BALANCE", &format!("${:.2}", status.balance), C::GREEN);
                    stat_row("FX FEED", if health.oanda_status == "OK" { "OK (CONNECTED)" } else { &health.oanda_status }, C::TEXT_MUTED);
                    stat_row("AI ENGINE", if status.ai_available { "ONLINE (OLLAMA)" } else { "OFFLINE" }, if status.ai_available { C::GREEN } else { C::AMBER });
                });
            });
        });
    }

    /// Main Terminal Layout (Topbar, Sidebar, Center Chart/DAG, Right Feeds, Bottom Curves)
    fn terminal_view(&mut self, ui: &mut Ui) {
        let status = self.snapshot.status.clone();
        let health = self.snapshot.system.clone();
        let depth = self.snapshot.order_books.get(&self.chart_symbol).cloned();
        let latest_pos = self.snapshot.positions.last().cloned();

        let chart_assets: Vec<String> = self
            .cfg
            .config
            .synthetic_assets
            .iter()
            .chain(self.cfg.config.forex_assets.iter())
            .chain(self.cfg.config.gold_assets.iter())
            .cloned()
            .collect();

        ui.vertical(|ui| {
            // ── 1. Top Status Header ──────────────────────────────────────────
            {
                let app_state = self.cfg.app.clone();
                ui::topbar::view(ui, &status, &health, move |engaged| {
                    app_state.set_kill_switch(engaged);
                });
            }
            ui.add_space(6.0);

            // ── 2. Main Center Body (Sidebar | Center | Right Feeds) ──────────
            let total_h = (ui.available_height() - 204.0).max(300.0);
            ui.allocate_ui(vec2(ui.available_width(), total_h), |ui| {
                ui.horizontal(|ui| {
                    // Left Sidebar (Portfolio & Risk)
                    ui.allocate_ui(vec2(280.0, total_h), |ui| {
                        ui::sidebar::view(
                            ui,
                            &status,
                            &self.snapshot.positions,
                            &mut self.sl_weight,
                            &mut self.factor_weight,
                            &mut self.contract_mode,
                            &mut self.rise_fall_stake,
                            &mut self.tick_duration,
                            self.snapshot.system.trade_limit,
                            self.cfg.config.max_daily_loss,
                            self.cfg.config.risk_per_trade_pct,
                        );
                        self.cfg.app.set_trading_params(self.contract_mode, self.rise_fall_stake, self.tick_duration);
                    });

                    ui.add_space(6.0);

                    // Center (Chart + Orderbook + View Switcher)
                    let center_w = (ui.available_width() - 314.0).max(400.0);
                    ui.allocate_ui(vec2(center_w, total_h), |ui| {
                        ui.vertical(|ui| {
                            // Asset Selector Row
                            ui.horizontal(|ui| {
                                ui::components::mono(ui, "INSTRUMENT:", C::TEXT_DIM, 9.0);
                                for s in &chart_assets {
                                    let active = s == &self.chart_symbol;
                                    let label = match s.as_str() {
                                        "1HZ100V" => "VOL 100 (1s)",
                                        "R_100" => "VOL 100",
                                        "1HZ75V" => "VOL 75 (1s)",
                                        "1HZ50V" => "VOL 50 (1s)",
                                        other => other,
                                    };
                                    if ui::components::cyber_btn(ui, label, active, C::WHITE).clicked() {
                                        self.chart_symbol = s.clone();
                                        self.trade_symbol = s.clone();
                                    }
                                }
                            });

                            ui.add_space(6.0);

                            // Chart & Orderbook Split
                            let top_split_h = (total_h * 0.58).max(220.0);
                            let candles = self.current_candles();
                            let chart_sym_disp = format!("{} · 5M", self.chart_symbol);
                            let chart_sym = self.chart_symbol.clone();
                            let chart_state = &mut self.chart_state;
                            let positions = self.snapshot.positions.clone();

                            ui.allocate_ui(vec2(center_w, top_split_h), |ui| {
                                ui.horizontal(|ui| {
                                    let chart_w = (center_w - 204.0).max(200.0);
                                    ui.allocate_ui(vec2(chart_w, top_split_h), |ui| {
                                        ui::components::panel(
                                            ui,
                                            "PRICE / LIQUIDITY",
                                            |ui| {
                                                ui::components::mono_bold(ui, chart_sym_disp, C::TEXT_MUTED, 9.0);
                                            },
                                            |ui| {
                                                ui::chart::view(
                                                    ui,
                                                    chart_state,
                                                    &candles,
                                                    &chart_sym,
                                                    "5m",
                                                    &positions,
                                                );
                                            },
                                        );
                                    });

                                    ui.add_space(6.0);

                                    ui.allocate_ui(vec2(196.0, top_split_h), |ui| {
                                        ui::orderbook::view(ui, depth.as_ref());
                                    });
                                });
                            });

                            ui.add_space(6.0);

                            // Pipeline Console vs Neural Memory Graph
                            let lower_split_h = ui.available_height().max(150.0);
                            let is_pipe = self.show_pipeline;
                            let mut pipe_toggle = None;
                            let neural_nodes = if !is_pipe { self.neural_nodes() } else { Vec::new() };
                            let pipeline_state = &mut self.pipeline_state;
                            let neural_state = &mut self.neural_state;
                            let snap_ref = &self.snapshot;
                            let status_ref = &status;

                            ui.allocate_ui(vec2(center_w, lower_split_h), |ui| {
                                ui::components::panel(
                                    ui,
                                    if is_pipe { "PIPELINE — DATA FLOW" } else { "AI NEURAL TOPOLOGY" },
                                    |ui| {
                                        ui.horizontal(|ui| {
                                            if ui::components::cyber_btn(ui, "PIPELINE", is_pipe, C::WHITE).clicked() {
                                                pipe_toggle = Some(true);
                                            }
                                            if ui::components::cyber_btn(ui, "NEURAL MEMORY", !is_pipe, C::WHITE).clicked() {
                                                pipe_toggle = Some(false);
                                            }
                                        });
                                    },
                                    |ui| {
                                        if is_pipe {
                                            ui::pipeline_console::view(ui, pipeline_state, snap_ref);
                                        } else {
                                            ui::ai_memory_graph::view(ui, neural_state, &neural_nodes, status_ref);
                                        }
                                    },
                                );
                            });
                            if let Some(show) = pipe_toggle {
                                self.show_pipeline = show;
                            }
                        });
                    });

                    ui.add_space(6.0);

                    // Right Sidebar (News + Neural Feed + Execution Box)
                    ui.allocate_ui(vec2(300.0, total_h), |ui| {
                        let neural_lines = self.neural_feed();
                        let clicked_order = ui::news_panel::view(
                            ui,
                            &self.snapshot.news,
                            &neural_lines,
                            &self.trade_action,
                            &self.trade_symbol,
                            Some("ENGINE ACTIVE"),
                            latest_pos,
                            self.contract_mode,
                            self.rise_fall_stake,
                            self.tick_duration,
                        );
                        if let Some(req) = clicked_order {
                            self.cfg.app.queue_order(req);
                        }
                    });
                });
            });

            ui.add_space(6.0);

            // ── 3. Bottom Row (Equity Curve + Recent Trades) ───────────────────
            let bottom_h = ui.available_height().max(180.0);
            ui.allocate_ui(vec2(ui.available_width(), bottom_h), |ui| {
                ui.horizontal(|ui| {
                    let half_w = (ui.available_width() - 6.0) / 2.0;
                    ui.allocate_ui(vec2(half_w, bottom_h), |ui| {
                        ui::equity_curve::view(ui, &self.equity, status.daily_pnl);
                    });

                    ui.add_space(6.0);

                    ui.allocate_ui(vec2(half_w, bottom_h), |ui| {
                        ui::trades_table::view(ui, &self.snapshot.recent_trades);
                    });
                });
            });
        });
    }

    /// Setup / Onboarding Wizard
    fn setup_view(&mut self, ui: &mut Ui) {
        let mut next_step = false;
        let mut back_step = false;
        let mut finish = false;

        let st = match &mut self.phase {
            Phase::Setup(o) => o,
            Phase::Live => return,
        };

        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() / 6.0);

            C::panel_frame().show(ui, |ui| {
                ui.set_width(680.0);
                ui.vertical(|ui| {
                    ui.label(
                        egui::RichText::new("JRX AI 1.0 — SETUP WIZARD")
                            .font(FontId::new(18.0, FontFamily::Monospace))
                            .color(C::CYAN)
                            .strong(),
                    );
                    ui.label(
                        egui::RichText::new(format!("STEP {} / {} — {}", st.step + 1, ONBOARD_STEPS, STEP_TITLES[st.step]))
                            .font(FontId::new(10.0, FontFamily::Monospace))
                            .color(C::TEXT_DIM),
                    );

                    ui::components::hline(ui);
                    ui.add_space(8.0);

                    match st.step {
                        0 => {
                            ui.label("• Forex and gold prices/candles come from live feeds.");
                            ui.label("• All order execution is PAPER (simulated credits, no real funds).");
                            ui.label("• OANDA is optional — only needed to place REAL orders.");
                            ui.label("• Locally running Ollama is used for the AI layer.");
                            ui.add_space(8.0);
                            ui::components::mono(ui, "Select paper mode unless you fully understand live-order risk.", C::AMBER, 10.0);
                        }
                        1 => {
                            ui.horizontal(|ui| {
                                if ui::components::cyber_btn(ui, "FOREX", st.engine == "forex", C::CYAN).clicked() {
                                    st.engine = "forex".into();
                                }
                                if ui::components::cyber_btn(ui, "GOLD", st.engine == "gold", C::AMBER).clicked() {
                                    st.engine = "gold".into();
                                }
                            });
                            ui.add_space(8.0);
                            ui.checkbox(&mut st.paper, "PAPER TRADING (Simulated demo funds)");
                        }
                        2 => {
                            ui.label("Select FOREX or GOLD in the engine mode above. Both markets use OANDA.");
                            ui.label("Forex symbols: EUR_USD, GBP_USD, USD_JPY");
                            ui.label("Gold symbol: XAU_USD");
                        }
                        3 => {
                            ui.label("OANDA API Configuration (Optional):");
                            ui.horizontal(|ui| {
                                ui.label("API TOKEN:");
                                ui.text_edit_singleline(&mut st.oanda_token);
                            });
                            ui.horizontal(|ui| {
                                ui.label("ACCOUNT ID:");
                                ui.text_edit_singleline(&mut st.oanda_account);
                            });
                            ui.checkbox(&mut st.practice, "PRACTICE (demo account)");
                        }
                        4 => {
                            ui.horizontal(|ui| {
                                ui.label("RISK PER TRADE %:");
                                ui.text_edit_singleline(&mut st.risk_pct);
                            });
                            ui.horizontal(|ui| {
                                ui.label("MAX OPEN TRADES:");
                                ui.text_edit_singleline(&mut st.max_open);
                            });
                            ui.horizontal(|ui| {
                                ui.label("MAX DAILY LOSS %:");
                                ui.text_edit_singleline(&mut st.max_daily);
                            });
                        }
                        _ => {
                            ui.label(format!("ENGINE: {}", st.engine.to_uppercase()));
                            ui.label(format!("MODE: {}", if st.paper { "PAPER" } else { "LIVE" }));
                            ui.label(format!("RISK/TRADE: {}%", st.risk_pct));
                            ui.label(format!("MAX OPEN: {}", st.max_open));
                        }
                    }

                    ui.add_space(16.0);
                    ui::components::hline(ui);
                    ui.add_space(8.0);

                    // Nav Buttons
                    ui.horizontal(|ui| {
                        if st.step > 0 {
                            if ui::components::cyber_btn(ui, "← BACK", false, C::TEXT_DIM).clicked() {
                                back_step = true;
                            }
                        }

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if st.step == ONBOARD_STEPS - 1 {
                                if ui::components::cyber_btn(ui, "FINISH & START ENGINE", true, C::GREEN).clicked() {
                                    finish = true;
                                }
                            } else {
                                if ui::components::cyber_btn(ui, "CONTINUE →", true, C::CYAN).clicked() {
                                    next_step = true;
                                }
                            }
                        });
                    });
                });
            });
        });

        if next_step {
            if let Phase::Setup(o) = &mut self.phase {
                o.step = (o.step + 1).min(ONBOARD_STEPS - 1);
            }
        }
        if back_step {
            if let Phase::Setup(o) = &mut self.phase {
                o.step = o.step.saturating_sub(1);
            }
        }
        if finish {
            self.finish_setup();
        }
    }
}

fn write_env(_st: &OnboardState, c: &trading_core::Config) {
    let mut buf = String::from("// JRX AI 1.0 - Trading AI Terminal configuration (written by setup wizard)\n");
    buf.push_str(&format!("OANDA_TOKEN={}\n", c.oanda_token));
    buf.push_str(&format!("OANDA_ACCOUNT_ID={}\n", c.oanda_account_id));
    buf.push_str(&format!("OANDA_PRACTICE={}\n", if c.oanda_practice { "true" } else { "false" }));
    buf.push_str(&format!("FX_BROKER={}\n", c.fx_broker));
    buf.push_str(&format!("DERIV_TOKEN={}\n", c.deriv_token));
    buf.push_str(&format!("DERIV_APP_ID={}\n", c.deriv_app_id));
    buf.push_str(&format!("DERIV_ACCOUNT_ID={}\n", c.deriv_account_id));
    buf.push_str(&format!("TRADING_MODE={}\n", c.trading_mode));
    buf.push_str(&format!("FOREX_ENABLED={}\n", if c.forex_enabled { "true" } else { "false" }));
    buf.push_str(&format!("GOLD_ENABLED={}\n", if c.gold_enabled { "true" } else { "false" }));
    buf.push_str(&format!("DEFAULT_ENGINE={}\n", c.default_engine));
    buf.push_str(&format!("RISK_PER_TRADE_PCT={:.2}\n", c.risk_per_trade_pct));
    buf.push_str(&format!("MAX_OPEN_POSITIONS={}\n", c.max_open_positions));
    buf.push_str(&format!("MAX_DAILY_LOSS={:.2}\n", c.max_daily_loss));
    buf.push_str(&format!("MAX_CONSECUTIVE_LOSSES={}\n", c.max_consecutive_losses));

    let path: PathBuf = std::env::current_dir().unwrap_or_default().join(".env");
    match std::fs::write(&path, buf) {
        Ok(_) => log::info!("setup wizard wrote {}", path.display()),
        Err(e) => log::error!("failed writing {}: {e}", path.display()),
    }
}