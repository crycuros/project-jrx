//! Right sidebar: News Scanner + Neural Feed Stream + Quick Execution Box.

use egui::Ui;
use trading_core::{NewsItem, OrderSide, Trade};

use super::components::{badge, hline, mono, mono_bold, panel};
use crate::theme::C;

pub fn view(
    ui: &mut Ui,
    news: &[NewsItem],
    neural: &[String],
    trade_action: &str,
    trade_symbol: &str,
    trade_msg_opt: Option<&str>,
    position: Option<Trade>,
    contract_mode: trading_core::ContractMode,
    stake: f64,
    duration: u32,
) -> Option<trading_api::ManualOrderRequest> {
    let mut manual_order = None;
    panel(
        ui,
        "FEEDS & EXECUTION",
        |_| {},
        |ui| {
            egui::ScrollArea::vertical()
                .id_salt("news_panel_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.vertical(|ui| {
                        // ── 1. News Scanner ───────────────────────────────────
                        C::card_frame().show(ui, |ui| {
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    mono_bold(ui, "NEWS SCANNER", C::TEXT_MUTED, 9.0);
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        mono(ui, format!("{} items", news.len()), C::TEXT_DIM, 9.0);
                                    });
                                });
                                ui.add_space(2.0);

                                if news.is_empty() {
                                    mono(ui, "NO NEWS FEED — AWAITING DATA", C::TEXT_DIM, 9.0);
                                } else {
                                    for n in news.iter().take(5) {
                                        news_item_view(ui, n);
                                    }
                                }
                            });
                        });

                        ui.add_space(4.0);

                        // ── 2. Neural Feed Stream ─────────────────────────────
                        C::card_frame().show(ui, |ui| {
                            ui.vertical(|ui| {
                                mono_bold(ui, "NEURAL FEED STREAM", C::PURPLE, 9.0);
                                ui.add_space(2.0);

                                if neural.is_empty() {
                                    mono(ui, "NEURAL FEED IDLE", C::TEXT_DIM, 9.0);
                                } else {
                                    for s in neural.iter().take(5) {
                                        ui.horizontal(|ui| {
                                            mono_bold(ui, ">", C::PURPLE, 9.0);
                                            mono(ui, s, C::TEXT, 8.5);
                                        });
                                    }
                                }
                            });
                        });

                        ui.add_space(4.0);

                        // ── 3. Trade Execution Box ────────────────────────────
                        manual_order = trade_box_view(ui, trade_action, trade_symbol, trade_msg_opt, position, contract_mode, stake, duration);
                    });
                });
        },
    );
    manual_order
}

fn news_item_view(ui: &mut Ui, n: &NewsItem) {
    let impact_col = match n.impact.as_str() {
        "HIGH" => C::RED,
        "MED" => C::AMBER,
        _ => C::GREEN,
    };

    ui.horizontal(|ui| {
        mono(ui, &n.time, C::TEXT_DIM, 8.0);
        mono(ui, &n.source, C::BLUE, 8.0);
        badge(ui, &n.impact, impact_col, C::BG_ELEVATED);
    });
    mono(ui, &n.title, C::TEXT, 8.5);
    ui.add_space(2.0);
}

fn trade_box_view(
    ui: &mut Ui,
    action: &str,
    symbol: &str,
    msg: Option<&str>,
    position: Option<Trade>,
    contract_mode: trading_core::ContractMode,
    stake: f64,
    duration: u32,
) -> Option<trading_api::ManualOrderRequest> {
    let mut clicked_order = None;

    C::card_frame().show(ui, |ui| {
        ui.vertical(|ui| {
            let has_pos = position.is_some();
            let (disp_sym, disp_action, entry, cur, pnl, sl, tp, verdict, is_buy) = match position {
                Some(p) => {
                    let side_label = match p.side {
                        OrderSide::Buy => "LONG",
                        OrderSide::Sell => "SHORT",
                    };
                    let is_buy = p.side == OrderSide::Buy;
                    let verdict_label = if p.ai_verdict.is_empty() { "AWAITING AI" } else { p.ai_verdict.as_str() };
                    (p.symbol, side_label.to_string(), p.entry_price, p.current_price.unwrap_or(p.entry_price), p.unrealized_pnl.unwrap_or(0.0), p.stop_loss, p.take_profit, verdict_label.to_string(), is_buy)
                }
                None => (
                    symbol.to_string(),
                    if action.to_lowercase() == "buy" { "LONG" } else { "SHORT" }.to_string(),
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    "NO OPEN POSITION".to_string(),
                    action.to_lowercase() == "buy",
                ),
            };

            let action_color = if is_buy { C::GREEN } else { C::RED };
            let pnl_color = if !has_pos {
                C::TEXT_DIM
            } else if pnl.abs() < f64::EPSILON {
                C::AMBER
            } else if pnl > 0.0 {
                C::GREEN
            } else {
                C::RED
            };

            ui.horizontal(|ui| {
                mono_bold(ui, "QUICK EXECUTION", C::TEXT_MUTED, 9.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    badge(ui, &disp_action, action_color, C::BG_ELEVATED);
                    mono_bold(ui, &disp_sym, C::TEXT, 10.0);
                });
            });

            hline(ui);

            let mut row_fn = |lbl: &str, val: &str, col: egui::Color32| {
                ui.horizontal(|ui| {
                    mono(ui, lbl, C::TEXT_DIM, 9.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        mono_bold(ui, val, col, 9.0);
                    });
                });
            };

            let sig_price = if has_pos { format!("{:.5}", entry) } else { "—".to_string() };
            let cur_price = if has_pos { format!("{:.5}", cur) } else { "—".to_string() };
            let pnl_txt = if has_pos { format!("{}{:.2} USD", if pnl > 0.0 { "+" } else { "" }, pnl) } else { "—".to_string() };
            let sl_txt = if has_pos { format!("{:.5}", sl) } else { "—".to_string() };
            let tp_txt = if has_pos { format!("{:.5}", tp) } else { "—".to_string() };

            row_fn("ENTRY PRICE", &sig_price, C::TEXT);
            row_fn("CURRENT", &cur_price, C::CYAN);
            row_fn("P/L USD", &pnl_txt, pnl_color);
            row_fn("STOP LOSS", &sl_txt, C::RED);
            row_fn("TAKE PROFIT", &tp_txt, C::GREEN);
            row_fn("AI VERDICT", &verdict, C::AMBER);

            if let Some(m) = msg {
                mono(ui, m, C::GREEN, 8.5);
            } else {
                mono(ui, "ENGINE ACTIVE", C::TEXT_DIM, 8.5);
            }

            ui.add_space(4.0);
            hline(ui);
            ui.add_space(4.0);

            // 1-Click Instant Execution — multiplier only (Rise/Fall removed).
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    let call_text = format!("▲ BUY MULT ${:.0}", stake);
                    let btn_call = egui::Button::new(egui::RichText::new(call_text).color(egui::Color32::BLACK).strong().size(11.0))
                        .fill(C::GREEN)
                        .min_size(egui::vec2((ui.available_width() - 4.0) / 2.0, 24.0));
                    if ui.add(btn_call).clicked() {
                        clicked_order = Some(trading_api::ManualOrderRequest {
                            symbol: symbol.to_string(),
                            side: OrderSide::Buy,
                            contract_mode,
                            stake,
                            duration_ticks: duration,
                        });
                    }

                    let put_text = format!("▼ SELL MULT ${:.0}", stake);
                    let btn_put = egui::Button::new(egui::RichText::new(put_text).color(egui::Color32::WHITE).strong().size(11.0))
                        .fill(C::RED)
                        .min_size(egui::vec2(ui.available_width(), 24.0));
                    if ui.add(btn_put).clicked() {
                        clicked_order = Some(trading_api::ManualOrderRequest {
                            symbol: symbol.to_string(),
                            side: OrderSide::Sell,
                            contract_mode,
                            stake,
                            duration_ticks: duration,
                        });
                    }
                });
            });
        });
    });

    clicked_order
}