//! Trading AI desktop — iced UI + in-process engine.
//!
//! First run shows a setup wizard (engine starts only after finish).

mod config;
mod engine;
mod theme;
mod ui;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use iced::widget::{button, checkbox, column, container, row, text, text_input};
use iced::{time, Element, Length, Subscription, Task, Theme};

use trading_core::UiSnapshot;

use crate::config::AppConfig;
use crate::engine::Engine;
use crate::theme::{C, MONO, MONO_BOLD};
use crate::ui::components;

fn main() -> iced::Result {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    log::info!("Trading AI Rust terminal starting…");

    let icon = iced::window::icon::from_file_data(include_bytes!("../icons/app.ico"), None)
        .ok();

    iced::application("JRX AI 1.0 — Trading AI Terminal", App::update, App::view)
        .subscription(App::subscription)
        .theme(App::theme)
        .window(iced::window::Settings {
            size: iced::Size::new(1440.0, 900.0),
            icon,
            ..Default::default()
        })
        .font(include_bytes!("../fonts/JetBrainsMono-Regular.ttf"))
        .font(include_bytes!("../fonts/JetBrainsMono-Bold.ttf"))
        .run_with(App::new)
}

const ONBOARD_STEPS: usize = 6;
const STEP_TITLES: [&str; ONBOARD_STEPS] = [
    "WELCOME",
    "MODE & ENGINE",
    "FOREX DATA — OANDA",
    "GOLD DATA — OANDA",
    "RISK LIMITS",
    "REVIEW & START",
];

#[derive(Debug, Clone, Copy)]
enum ObField {
    OaToken,
    OaAccount,
    RiskPct,
    MaxOpen,
    MaxDaily,
    MaxLoss,
}

#[derive(Clone)]
struct OnboardState {
    step: usize,
    engine: String,
    paper: bool,
    practice: bool,
    oanda_token: String,
    oanda_account: String,
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
            engine: c.default_engine.clone(),
            paper: c.trading_mode != "live",
            practice: c.oanda_practice,
            oanda_token: String::new(),
            oanda_account: String::new(),
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
    // Keeps the engine's tokio runtime alive after onboarding.
    #[allow(dead_code)]
    rt: Option<tokio::runtime::Runtime>,
    phase: Phase,
    snapshot: UiSnapshot,
    equity: Vec<(f64, f64)>,
    sl_weight: f32,
    factor_weight: f32,
    trade_action: String,
    trade_symbol: String,
    chart_symbol: String,
    data_ready: bool,
}

impl App {
    fn new() -> (Self, Task<Message>) {
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
            trade_action: "buy".to_string(),
            trade_symbol: "EUR_USD".to_string(),
            chart_symbol: "EUR_USD".to_string(),
            data_ready: false,
        };
        // If a previous setup wrote .env, skip the wizard and start trading directly.
        if std::path::Path::new(".env").exists() {
            app.finish_setup();
        }

        (app, Task::none())
    }

    fn theme(&self) -> Theme {
        Theme::Dark
    }

    fn snapshot(&mut self) {
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
        // Flip once we have real candles for the selected chart symbol.
        if self
            .snapshot
            .candles
            .get(&self.chart_symbol)
            .map(|c| c.len() >= 2)
            .unwrap_or(false)
        {
            self.data_ready = true;
        }
        // SL/Factor weights derived from live positions + risk config.
        self.sl_weight = Self::real_sl_weight(&self.snapshot.positions, self.cfg.config.risk_per_trade_pct);
        self.factor_weight = Self::real_factor_weight(&self.snapshot.positions, self.snapshot.status.balance);
    }

    /// Average stop-loss distance as % of entry across open positions.
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

    /// % of current balance committed as stake across open positions.
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
        // During a hotboot (.env already exists) keep the .env engine flags
        // (e.g. FOREX + GOLD both on) instead of forcing a single market.
        if !std::path::Path::new(".env").exists() {
            c.forex_enabled = st.engine != "gold";
            c.gold_enabled = st.engine == "gold";
        }
        c.trading_mode = if st.paper { "paper".to_string() } else { "live".to_string() };
        c.oanda_token = st.oanda_token.trim().to_string();
        c.oanda_account_id = st.oanda_account.trim().to_string();
        c.oanda_practice = st.practice && st.paper;
        // During a hotboot (.env already exists) keep the .env broker selection
        // (e.g. "deriv" for real demo orders) instead of forcing yahoo/oanda.
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

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Tick => {
                if matches!(self.phase, Phase::Live) {
                    self.snapshot();
                }
                Task::none()
            }
            Message::SetTradeAction(a) => {
                self.trade_action = a;
                Task::none()
            }
            Message::SetTradeSymbol(s) => {
                self.trade_symbol = s;
                Task::none()
            }
            Message::SetChartSymbol(s) => {
                self.chart_symbol = s;
                Task::none()
            }
            Message::OnboardNext => {
                if let Phase::Setup(o) = &mut self.phase {
                    o.step = (o.step + 1).min(ONBOARD_STEPS - 1);
                }
                Task::none()
            }
            Message::OnboardBack => {
                if let Phase::Setup(o) = &mut self.phase {
                    o.step = o.step.saturating_sub(1);
                }
                Task::none()
            }
            Message::OnboardSet(f, v) => {
                if let Phase::Setup(o) = &mut self.phase {
                    match f {
                        ObField::OaToken => o.oanda_token = v,
                        ObField::OaAccount => o.oanda_account = v,
                        ObField::RiskPct => o.risk_pct = v,
                        ObField::MaxOpen => o.max_open = v,
                        ObField::MaxDaily => o.max_daily = v,
                        ObField::MaxLoss => o.max_loss = v,
                    }
                }
                Task::none()
            }
            Message::OnboardEngine(e) => {
                if let Phase::Setup(o) = &mut self.phase {
                    o.engine = e;
                }
                Task::none()
            }
            Message::OnboardPaper(b) => {
                if let Phase::Setup(o) = &mut self.phase {
                    o.paper = b;
                }
                Task::none()
            }
            Message::OnboardPractice(b) => {
                if let Phase::Setup(o) = &mut self.phase {
                    o.practice = b;
                }
                Task::none()
            }
            Message::OnboardFinish => {
                self.finish_setup();
                Task::none()
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        match &self.phase {
            Phase::Setup(o) => self.setup_view(o),
            Phase::Live => {
                if !self.data_ready {
                    self.loading_view()
                } else {
                    self.terminal_view()
                }
            }
        }
    }

    /// Full-screen boot screen shown until the first live feed arrives.
    fn loading_view(&self) -> Element<'_, Message> {
        let status = &self.snapshot.status;
        let health = &self.snapshot.system;
        let spinner = ((std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            / 120)
            % 4) as usize;
        let dots = ".".repeat(spinner);

        let lines = column![
            text("J R X   A I   T E R M I N A L")
                .font(MONO_BOLD)
                .size(28)
                .color(C::cyan()),
            text("INITIALIZING LIVE FEED")
                .font(MONO_BOLD)
                .size(16)
                .color(C::amber()),
            text(format!("connecting to {} broker{dots}", status.active_engine.to_uppercase()))
                .font(MONO)
                .size(12)
                .color(C::text2()),
            components::hline(),
            text(format!("BROKER     : {}", status.active_engine.to_uppercase()))
                .font(MONO)
                .size(11)
                .color(C::text()),
            text(format!("MODE       : {}", status.mode.to_uppercase()))
                .font(MONO)
                .size(11)
                .color(C::text()),
            text(format!("BALANCE    : ${:.2}", status.balance))
                .font(MONO)
                .size(11)
                .color(C::text()),
            text(format!("FX FEED    : {}", if health.oanda_status == "OK" { "OK" } else { health.oanda_status.as_str() }))
                .font(MONO)
                .size(11)
                .color(C::text2()),
            text(format!("AI ENGINE  : {}", if status.ai_available { "ONLINE" } else { "OFFLINE" }))
                .font(MONO)
                .size(11)
                .color(C::text2()),
        ]
        .spacing(6)
        .align_x(iced::Alignment::Center);

        container(
            container(lines)
                .padding(36)
                .style(components::panel_style)
                .width(Length::Fixed(520.0)),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(iced::Length::Fill)
        .center_y(iced::Length::Fill)
        .style(theme::bg_root)
        .into()
    }

    fn terminal_view(&self) -> Element<'_, Message> {
        let status = &self.snapshot.status;
        let health = &self.snapshot.system;
        let depth = self.snapshot.order_books.get(&self.chart_symbol);
        let chart_assets: Vec<String> = self
            .cfg
            .config
            .forex_assets
            .iter()
            .chain(self.cfg.config.gold_assets.iter())
            .cloned()
            .collect();

        let latest_pos = self.snapshot.positions.last().cloned();
        let selector_row: iced::widget::Row<'_, Message> = iced::widget::Row::with_children(
            std::iter::once(
                text("CHART").font(theme::MONO).size(8).color(theme::C::dim()).into(),
            )
            .chain(chart_assets.iter().map(|s| {
                let active = s == &self.chart_symbol;
                container(
                    button(text(s.clone()).font(theme::MONO).size(9).color(if active { theme::C::text() } else { theme::C::dim() }))
                        .padding([3, 8])
                        .style(move |_t, _st| {
                            iced::widget::button::Style {
                                background: Some(iced::Background::Color(if active { theme::C::blue() } else { theme::C::bg_card() })),
                                text_color: if active { theme::C::text() } else { theme::C::dim() },
                                border: iced::Border { color: theme::C::border(), width: 1.0, radius: iced::border::Radius::from(2.0) },
                                ..Default::default()
                            }
                        })
                        .on_press(Message::SetChartSymbol(s.clone())),
                )
                .into()
            }))
            .chain(std::iter::once(iced::widget::Space::with_width(Length::Fill).into()))
            .collect::<Vec<iced::Element<'_, Message>>>(),
        )
        .spacing(4);

        let content = column![
            // TOP STATUS HEADER
            ui::topbar::view(status, health),

            // MAIN AREA: sidebar | center | right
            row![
                // LEFT SIDEBAR (Portfolio & Risk + SL/Factor bottom)
                container(ui::sidebar::view(
                    status,
                    &self.snapshot.positions,
                    self.sl_weight,
                    self.factor_weight,
                    self.snapshot.system.trade_limit,
                    self.cfg.config.max_daily_loss,
                    self.cfg.config.risk_per_trade_pct,
                ))
                    .width(Length::Fixed(280.0))
                    .height(Length::Fill),

                // CENTER
                container(
                    column![
                        selector_row,
                        // Price chart + order book
                        row![
                            container(ui::chart::view(&self.current_candles(), &self.chart_symbol, "5m"))
                                .width(Length::Fill),
                            container(ui::orderbook::view(depth)).width(Length::Fixed(190.0)),
                        ]
                        .spacing(2)
                        .height(Length::Fixed(420.0)),
                        // Galaxy AI Memory Graph spans full center below chart
                        container(ui::ai_memory_graph::view(&self.ai_memory_nodes(), status)).height(Length::Fill),
                    ]
                    .spacing(2),
                )
                .width(Length::Fill)
                .height(Length::Fill),

                // RIGHT SIDEBAR (News + Neural + Trade box)
                container(
                    ui::news_panel::view(
                        &self.snapshot.news,
                        &self.neural_feed(),
                        &self.trade_action,
                        &self.trade_symbol,
                        Some("AWAITING ORDER"),
                        latest_pos,
                    ),
                )
                .width(Length::Fixed(300.0))
                .height(Length::Fill),
            ]
            .spacing(2)
            .height(Length::Fill),

            // BOTTOM (Equity curve + Recent trades)
            row![
                container(ui::equity_curve::view(&self.equity, status.daily_pnl)).width(Length::Fill),
                container(ui::trades_table::view(&self.snapshot.recent_trades)).width(Length::Fill),
            ]
            .spacing(2)
            .height(Length::Fixed(200.0)),
        ]
        .spacing(2)
        .padding(4);

        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(theme::bg_root)
            .into()
    }

    fn current_candles(&self) -> Vec<trading_core::Candle> {
        let symbol = &self.chart_symbol;
        self.snapshot
            .candles
            .get(symbol)
            .filter(|c| c.len() >= 2)
            .cloned()
            .unwrap_or_default()
    }

    fn ai_memory_nodes(&self) -> Vec<trading_core::AiMemoryNode> {
        use trading_core::{AiMemoryNode, Market, MemoryOrbit, NodeVerdict};

        let mut nodes: Vec<AiMemoryNode> = Vec::new();
        let status = &self.snapshot.status;

        // ── Inner ring: recent trade signals ─────────────────────────────────
        for t in self.snapshot.recent_trades.iter().take(6) {
            let verdict = match t.ai_verdict.as_str() {
                "GO"    => NodeVerdict::Go,
                "WATCH" => NodeVerdict::Watch,
                "NO-GO" => NodeVerdict::NoGo,
                _       => NodeVerdict::Neutral,
            };
            nodes.push(AiMemoryNode {
                id: format!("sig_{}", t.symbol),
                label: format!("{} {}", t.symbol.split('/').next().unwrap_or(&t.symbol), t.side.as_str().to_uppercase()),
                orbit:  MemoryOrbit::Signal,
                verdict,
                confidence: t.confidence,
                market: t.market,
                active: t.confidence > 0.65,
                detail: format!(
                    "{} {} · {}", t.side.as_str().to_uppercase(), t.symbol, t.ai_verdict
                ),
            });
        }

        // ── Mid ring: regime memory nodes ─────────────────────────────────────
        let regime = status.active_engine.as_str();
        for (label, verdict, conf, active) in [
            ("BULL",     NodeVerdict::Go,      0.70f64, regime == "hybrid" || regime == "crypto"),
            ("BEAR",     NodeVerdict::NoGo,    0.40,    false),
            ("RANGING",  NodeVerdict::Watch,   0.55,    false),
            ("BREAKOUT", NodeVerdict::Go,      0.75,    regime == "forex"),
            ("FOREX",    NodeVerdict::Neutral, 0.50,    status.forex_enabled),
            ("GOLD",     NodeVerdict::Neutral, 0.60,    status.gold_enabled),
        ] {
            nodes.push(AiMemoryNode {
                id: format!("regime_{label}"),
                label: label.to_string(),
                orbit:  MemoryOrbit::Regime,
                verdict,
                confidence: conf,
                market: Market::Forex,
                active,
                detail: format!("regime {} {}", label, if active { "(active)" } else { "" }),
            });
        }

        // ── Outer ring: full asset universe ───────────────────────────────────
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

            // Real momentum leader from the live candle set.
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

            // Real spread from the live book of the active asset.
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

    fn subscription(&self) -> Subscription<Message> {
        time::every(Duration::from_millis(1000)).map(|_| Message::Tick)
    }

    // ---- Setup wizard ----

    fn setup_view<'a>(&'a self, ob: &'a OnboardState) -> Element<'a, Message> {
        let header = column![
            text("JRX AI 1.0 TRADING TERMINAL").font(MONO_BOLD).size(22).color(C::text2()),
            components::micro(format!("STEP {} / {} — {}", ob.step + 1, ONBOARD_STEPS, STEP_TITLES[ob.step])),
        ]
        .spacing(2);

        let body: Element<'_, Message> = match ob.step {
            0 => Self::step_welcome(),
            1 => Self::step_mode(ob),
            2 => Self::step_market(ob),
            3 => Self::step_oanda(ob),
            4 => Self::step_risk(ob),
            _ => Self::step_review(ob),
        };

        let back_msg = if ob.step > 0 { Some(Message::OnboardBack) } else { None };
        let (next_label, next_msg) = if ob.step == ONBOARD_STEPS - 1 {
            ("FINISH & START ENGINE", Some(Message::OnboardFinish))
        } else {
            ("CONTINUE →", Some(Message::OnboardNext))
        };

        let nav = row![
            Self::btn("← BACK", back_msg, false),
            iced::widget::Space::with_width(Length::Fill),
            Self::btn(next_label, next_msg, true),
        ]
        .spacing(8);

        let card = container(
            column![header, components::hline(), body, nav].spacing(14).padding(20),
        )
        .width(Length::Fill)
        .style(components::panel_style);

        container(
            column![
                iced::widget::Space::with_height(Length::Fill),
                card.width(Length::Fixed(720.0)),
                iced::widget::Space::with_height(Length::Fill),
            ]
            .align_x(iced::Alignment::Center)
            .width(Length::Fill)
            .height(Length::Fill)
            .spacing(10),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .style(theme::bg_root)
        .into()
    }

    fn step_welcome() -> Element<'static, Message> {
        column![
            components::mono("Trading AI terminal — first-run setup."),
            components::mono("• Forex and gold prices/candles come from a free Yahoo Finance feed."),
            components::mono("• All order execution is PAPER (simulated credits, no real funds)."),
            components::mono("• OANDA is optional — only needed to place REAL orders."),
            components::mono("• Locally running Ollama is used for the AI layer."),
            iced::widget::Space::with_height(10),
            components::mono_color("Select paper mode unless you fully understand live-order risk.", C::amber()),
        ]
        .spacing(6)
        .into()
    }

    fn step_mode(ob: &OnboardState) -> Element<'_, Message> {
        let engine_row = row![
            Self::eng_btn("FOREX", "forex", &ob.engine),
            Self::eng_btn("GOLD", "gold", &ob.engine),
        ]
        .spacing(8);

        let paper_box = checkbox("PAPER TRADING — simulated credits, no real orders", ob.paper)
            .on_toggle(Message::OnboardPaper);

        let desc: Element<'static, Message> = match ob.engine.as_str() {
            "forex" => column![
                components::mono_color("FOREX — free Yahoo feed:", C::green()),
                components::mono("EUR_USD · GBP_USD · USD_JPY"),
                components::mono("Live candles & pricing, no API key needed, works now."),
                components::mono("OANDA optional (enter a token to route through your account)."),
            ]
            .spacing(2)
            .into(),
            "gold" => column![
                components::mono_color("GOLD — free Yahoo feed:", C::green()),
                components::mono("XAU_USD (GC=F gold futures)"),
                components::mono("Same setup as forex — no key needed."),
            ]
            .spacing(2)
            .into(),
            _ => column![
                components::mono_color("FOREX + GOLD — free Yahoo feed:", C::green()),
                components::mono("Choose FOREX or GOLD to keep the markets separate."),
            ]
            .spacing(2)
            .into(),
        };

        column![
            components::micro("ACTIVE ENGINE"),
            engine_row,
            iced::widget::Space::with_height(4),
            container(desc)
                .width(Length::Fill)
                .padding([10, 12])
                .style(components::panel_style),
            iced::widget::Space::with_height(2),
            components::micro("TRADING MODE"),
            paper_box,
        ]
        .spacing(6)
        .into()
    }

    fn step_market(_ob: &OnboardState) -> Element<'_, Message> {
        column![
            components::mono_color("Select FOREX or GOLD in the engine mode above. Both markets use OANDA.", C::text2()),
            components::mono_color("Forex symbols: EUR_USD, GBP_USD, USD_JPY", C::dim()),
            components::mono_color("Gold symbol: XAU_USD", C::dim()),
        ]
        .spacing(10)
        .into()
    }

    fn step_oanda(ob: &OnboardState) -> Element<'_, Message> {
        let practice_box = checkbox("PRACTICE (demo) account — Global Markets Demo", ob.practice)
            .on_toggle(Message::OnboardPractice);

        column![
            components::mono_color("OANDA account (optional):", C::green()),
            components::mono("Free Yahoo feed is on by default — your FX/gold data works with no setup."),
            components::mono("Add an OANDA token only if you want real order routing. Demo:"),
            iced::widget::Space::with_height(4),
            Self::ob_field("OANDA API TOKEN (optional)", "••••••••••••", &ob.oanda_token, ObField::OaToken),
            Self::ob_field("OANDA ACCOUNT ID", "e.g. 101-004-1234567-001", &ob.oanda_account, ObField::OaAccount),
            practice_box,
            iced::widget::Space::with_height(4),
            components::mono_color("Leave the token empty for the free Yahoo feed (paper orders).", C::amber()),
        ]
        .spacing(8)
        .into()
    }

    fn step_risk(ob: &OnboardState) -> Element<'_, Message> {
        column![
            row![
                component_width(Self::ob_field("RISK PER TRADE %", "0.5", &ob.risk_pct, ObField::RiskPct), Length::FillPortion(1)),
                component_width(Self::ob_field("MAX OPEN POSITIONS", "5", &ob.max_open, ObField::MaxOpen), Length::FillPortion(1)),
            ]
            .spacing(10),
            row![
                component_width(Self::ob_field("MAX DAILY LOSS %", "5", &ob.max_daily, ObField::MaxDaily), Length::FillPortion(1)),
                component_width(Self::ob_field("MAX CONSECUTIVE LOSSES", "3", &ob.max_loss, ObField::MaxLoss), Length::FillPortion(1)),
            ]
            .spacing(10),
        ]
        .spacing(10)
        .into()
    }

    fn step_review(ob: &OnboardState) -> Element<'_, Message> {
        let mode = if ob.paper { "PAPER (simulated)" } else { "LIVE (real orders — at your own risk)" };
        let oanda_status = {
            if ob.oanda_token.trim().is_empty() {
                "YAHOO — free feed (no key)".to_string()
            } else if ob.practice {
                format!("OANDA practice · {}", ob.oanda_account)
            } else {
                format!("OANDA LIVE · {}", ob.oanda_account)
            }
        };
        let engine = ob.engine.to_uppercase();

        let mut rows: Vec<Element<'_, Message>> = vec![
            Self::review_row("MODE", &mode),
            Self::review_row("ENGINE", &engine),
            Self::review_row("FX DATA", &oanda_status),
            Self::review_row("RISK/TRADE", &format!("{}%", ob.risk_pct)),
            Self::review_row("MAX OPEN", &ob.max_open),
            Self::review_row("MAX DAILY LOSS", &format!("{}%", ob.max_daily)),
            Self::review_row("MAX CONS. LOSSES", &ob.max_loss),
        ];
        rows.push(components::mono_color("Press FINISH & START ENGINE to enter the terminal.", C::green()).into());

        column(rows).spacing(6).into()
    }

    fn review_row(label: &str, value: &str) -> Element<'static, Message> {
        row![
            component_width(components::mono_color(format!("{label}: "), C::dim()), Length::Fixed(170.0)),
            components::mono(value),
        ]
        .into()
    }

    fn ob_field(label: &str, placeholder: &str, value: &str, f: ObField) -> Element<'static, Message> {
        let mut input = text_input(placeholder, value)
            .on_input(move |s| Message::OnboardSet(f, s))
            .width(Length::Fill)
            .padding(7);
        if matches!(f, ObField::OaToken) {
            input = input.secure(true);
        }
        let input: iced::Element<'static, Message> = input.into();
        column![components::micro(label), input].spacing(2).into()
    }

    fn eng_btn(label: &str, val: &str, sel: &str) -> Element<'static, Message> {
        let active = sel == val;
        let t = text(format!("{} {}", if active { "●" } else { "○" }, label))
            .font(MONO_BOLD)
            .size(10)
            .color(if active { C::green() } else { C::text2() });
        button(t).padding([6, 14]).on_press(Message::OnboardEngine(val.to_string())).into()
    }

    fn btn(label: &str, msg: Option<Message>, accent: bool) -> Element<'_, Message> {
        let t = text(label).font(MONO_BOLD).size(10).color(if accent { C::green() } else { C::text2() });
        let b = button(t).padding([7, 16]).width(Length::Fixed(210.0));
        match msg {
            Some(m) => b.on_press(m).into(),
            None => b.into(),
        }
    }
}

fn component_width<'a, M: 'a>(e: Element<'a, M>, w: Length) -> Element<'a, M> {
    container(e).width(w).into()
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

#[derive(Debug, Clone)]
#[allow(dead_code)]
enum Message {
    Tick,
    SetTradeAction(String),
    SetTradeSymbol(String),
    SetChartSymbol(String),
    OnboardNext,
    OnboardBack,
    OnboardSet(ObField, String),
    OnboardEngine(String),
    OnboardPaper(bool),
    OnboardPractice(bool),
    OnboardFinish,
}