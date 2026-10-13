//! Left sidebar panel: Portfolio KPIs, Risk Gauges, Open Positions, and SL/Factor Weights.

use egui::{FontFamily, FontId, Ui};
use trading_core::{OrderSide, Status, Trade};

use super::components::{cyber_btn, hline, mono, mono_bold, panel};
use crate::theme::C;

pub fn view(
    ui: &mut Ui,
    status: &Status,
    positions: &[Trade],
    sl_weight: &mut f32,
    factor_weight: &mut f32,
    _contract_mode: &mut trading_core::ContractMode,
    _rise_fall_stake: &mut f64,
    _tick_duration: &mut u32,
    max_open: i32,
    max_daily_loss: f64,
    _risk_trade_pct: f64,
) {
    panel(
        ui,
        "PORTFOLIO & RISK",
        |_| {},
        |ui| {
            egui::ScrollArea::vertical()
                .id_salt("sidebar_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.vertical(|ui| {
                        // ── 1. Top KPI Summary Card ───────────────────────────
                        C::card_frame().show(ui, |ui| {
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    mono(ui, "NET PROFIT", C::TEXT_DIM, 9.0);
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        mono_bold(ui, format!("WIN {:.1}%", status.win_rate), C::TEXT_MUTED, 9.0);
                                    });
                                });

                                let pnl_col = if status.total_pnl >= 0.0 { C::GREEN } else { C::RED };
                                ui.label(
                                    egui::RichText::new(format!("${:>+8.2}", status.total_pnl))
                                        .font(FontId::new(20.0, FontFamily::Monospace))
                                        .color(pnl_col)
                                        .strong(),
                                );

                                ui.horizontal(|ui| {
                                    mono(ui, format!("24H P&L: ${:>+6.2}", status.daily_pnl), C::TEXT_MUTED, 9.0);
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        mono(ui, format!("BAL: ${:.2}", status.balance), C::TEXT_MUTED, 9.0);
                                    });
                                });
                            });
                        });

                        ui.add_space(8.0);

                        // ── 2. Execution & Auto-Compounding Status ────────────
                        C::card_frame().show(ui, |ui| {
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    mono_bold(ui, "EXECUTION ENGINE", C::TEXT, 9.0);
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        mono_bold(ui, "AUTO-COMPOUND", C::WHITE, 8.5);
                                    });
                                });
                                ui.add_space(4.0);
                                ui.label(
                                    egui::RichText::new("Dynamic equity sizing · 5.0R Shallow Reclaim\nSmart Profit Lock (+0.60R @ +2.0R)")
                                        .font(FontId::new(8.5, FontFamily::Monospace))
                                        .color(C::TEXT_MUTED),
                                );
                            });
                        });

                        ui.add_space(8.0);

                        // ── 3. Open Positions List ────────────────────────────
                        C::card_frame().show(ui, |ui| {
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    mono_bold(ui, format!("ACTIVE POSITIONS ({})", positions.len()), C::TEXT_MUTED, 9.0);
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        mono(ui, format!("MAX {}", max_open), C::TEXT_DIM, 8.5);
                                    });
                                });
                                ui.add_space(4.0);

                                if positions.is_empty() {
                                    ui.label(egui::RichText::new("NO ACTIVE TRADES").font(FontId::new(9.0, FontFamily::Monospace)).color(C::TEXT_DIM));
                                } else {
                                    for t in positions {
                                        let is_buy = t.side == OrderSide::Buy;
                                        let dir_col = if is_buy { C::GREEN } else { C::RED };
                                        let pnl = t.unrealized_pnl.unwrap_or(0.0);
                                        let pnl_col = if pnl >= 0.0 { C::GREEN } else { C::RED };
                                        let mode_lbl = "MULT";

                                        ui.horizontal(|ui| {
                                            mono_bold(ui, &t.symbol, C::TEXT, 9.0);
                                            mono(ui, format!("[{:?} {}]", t.side, mode_lbl), dir_col, 8.5);
                                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                mono_bold(ui, format!("${:>+6.2}", pnl), pnl_col, 9.0);
                                            });
                                        });
                                    }
                                }
                            });
                        });

                        ui.add_space(4.0);

                        // ── 4. Real-time Interactive SL/Factor Weights ────────
                        C::card_frame().show(ui, |ui| {
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    mono_bold(ui, "SL / FACTOR WEIGHTS", C::TEXT_MUTED, 9.0);
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        mono(ui, "LIVE TUNING", C::CYAN, 8.5);
                                    });
                                });
                                ui.add_space(4.0);

                                // Stop Loss Slider & Presets
                                ui.horizontal(|ui| {
                                    mono(ui, "STOP LOSS:", C::RED, 9.0);
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        mono_bold(ui, format!("{:.1}%", *sl_weight), C::RED, 9.5);
                                    });
                                });
                                ui.add(
                                    egui::Slider::new(sl_weight, 0.5..=10.0)
                                        .step_by(0.1)
                                        .show_value(false)
                                );
                                // Quick SL Presets
                                ui.horizontal(|ui| {
                                    for &p in &[1.0_f32, 2.0_f32, 3.0_f32, 5.0_f32] {
                                        let active = (*sl_weight - p).abs() < 0.05;
                                        if cyber_btn(ui, &format!("{:.0}%", p), active, C::RED).clicked() {
                                            *sl_weight = p;
                                        }
                                    }
                                });

                                ui.add_space(4.0);

                                // Factor Sizing Slider & Presets
                                ui.horizontal(|ui| {
                                    mono(ui, "FACTOR WT:", C::BLUE, 9.0);
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        mono_bold(ui, format!("{:.0}%", *factor_weight), C::BLUE, 9.5);
                                    });
                                });
                                ui.add(
                                    egui::Slider::new(factor_weight, 5.0..=100.0)
                                        .step_by(1.0)
                                        .show_value(false)
                                );
                                // Quick Factor Presets
                                ui.horizontal(|ui| {
                                    for &f in &[15.0_f32, 30.0_f32, 50.0_f32, 100.0_f32] {
                                        let active = (*factor_weight - f).abs() < 1.0;
                                        if cyber_btn(ui, &format!("{:.0}%", f), active, C::BLUE).clicked() {
                                            *factor_weight = f;
                                        }
                                    }
                                });

                                ui.add_space(4.0);
                                hline(ui);
                                ui.add_space(4.0);

                                let mut param_row = |lbl: &str, val: &str| {
                                    ui.horizontal(|ui| {
                                        mono(ui, lbl, C::TEXT_DIM, 9.0);
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            mono_bold(ui, val, C::TEXT, 9.0);
                                        });
                                    });
                                };

                                param_row("RISK/TRADE", &format!("{:.1}%", *sl_weight));
                                param_row("MAX OPEN", &max_open.to_string());
                                param_row("MAX DAILY LOSS", &format!("{:.1}%", max_daily_loss));
                                param_row("LOSS COOLDOWN", &status.consecutive_losses.to_string());
                            });
                        });
                    });
                });
        },
    );
}

#[allow(dead_code)]
fn pnl_pct(t: &Trade) -> f64 {
    if t.entry_price <= 0.0 {
        return 0.0;
    }
    let cur = t.current_price.unwrap_or(t.entry_price);
    let raw = (cur - t.entry_price) / t.entry_price * 100.0;
    match t.side {
        OrderSide::Buy => raw,
        OrderSide::Sell => -raw,
    }
}