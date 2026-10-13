//! Top status header: terminal branding + global metrics + live beacon + clock.

use egui::{FontFamily, FontId, Ui};
use trading_core::{Status, SystemHealth};

use super::components::{badge, live_beacon};
use crate::theme::C;

pub fn view<F>(ui: &mut Ui, status: &Status, health: &SystemHealth, mut on_kill_switch: F)
where
    F: FnMut(bool),
{
    C::header_frame().show(ui, |ui| {
        ui.horizontal(|ui| {
            // ── Left: Branding & Subtitle ─────────────────────────────────────
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new("JRX AI 1.0 — AI TRADING TERMINAL")
                        .font(FontId::new(12.0, FontFamily::Monospace))
                        .color(C::TEXT)
                        .strong(),
                );
                let mode_label = if status.kill_switch {
                    "HALTED".to_string()
                } else if status.running {
                    format!("{} · {}", status.mode.to_uppercase(), status.active_engine.to_uppercase())
                } else {
                    "IDLE".to_string()
                };
                ui.label(
                    egui::RichText::new(format!("DIRECTX 11/12 GPU ACCELERATED · {} · OANDA FOREX + GOLD", mode_label))
                        .font(FontId::new(8.0, FontFamily::Monospace))
                        .color(C::TEXT_DIM),
                );
            });

            ui.add_space(16.0);

            // ── Center: Status Chips ──────────────────────────────────────────
            ui.horizontal(|ui| {
                let fx_label = if health.fx_source.is_empty() || health.fx_source == "oanda" {
                    "OANDA".to_string()
                } else {
                    health.fx_source.to_uppercase()
                };

                badge(
                    ui,
                    &format!("FX: {} {}", fx_label, health.oanda_status),
                    C::CYAN,
                    C::BG_CARD,
                );

                badge(
                    ui,
                    &format!("AI: OLLAMA {}", health.ollama_status),
                    if health.ollama_status == "OK" { C::GREEN } else { C::AMBER },
                    C::BG_CARD,
                );

                badge(
                    ui,
                    &format!("WIN RATE: {:.1}%", status.win_rate),
                    if status.win_rate >= 60.0 { C::GREEN } else { C::TEXT_MUTED },
                    C::BG_CARD,
                );

                let is_open = health.market_status == "OPEN";
                badge(
                    ui,
                    if is_open { "MARKET: OPEN" } else { "MARKET: CLOSED" },
                    if is_open { C::GREEN } else { C::RED },
                    C::BG_CARD,
                );

                let engine_color = match status.active_engine.as_str() {
                    "forex" => C::PURPLE,
                    _ => C::CYAN,
                };
                badge(
                    ui,
                    &format!("ENGINE: {}", status.active_engine.to_uppercase()),
                    engine_color,
                    C::BG_CARD,
                );

                // Animated Live Radar Beacon
                let is_live = status.running && !status.kill_switch;
                let status_text = if status.kill_switch {
                    "HALTED"
                } else if status.running {
                    "LIVE ACTIVE"
                } else {
                    "ENGINE IDLE"
                };
                live_beacon(ui, is_live, status_text);

                ui.add_space(8.0);

                // Emergency stop. The beacon above is read-only — this is the
                // actual control that trips the engine's kill switch.
                let halt_text = if status.kill_switch { "▶  RESUME" } else { "■  HALT" };
                let halt_color = if status.kill_switch { C::GREEN } else { C::RED };
                let resp = ui.add(
                    egui::Button::new(
                        egui::RichText::new(halt_text)
                            .font(FontId::new(9.0, FontFamily::Monospace))
                            .color(halt_color)
                            .strong(),
                    )
                    .fill(C::BG_CARD)
                    .stroke(egui::Stroke::new(1.0_f32, halt_color)),
                );
                if resp.clicked() {
                    let engaged = !status.kill_switch;
                    if engaged {
                        log::warn!("KILL SWITCH engaged by user — halting new entries");
                    } else {
                        log::info!("KILL SWITCH released by user — resuming");
                    }
                    on_kill_switch(engaged);
                }
            });

            // ── Right: UTC Digital Clock ──────────────────────────────────────
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.vertical(|ui| {
                    ui.label(
                        egui::RichText::new(now_clock())
                            .font(FontId::new(12.0, FontFamily::Monospace))
                            .color(C::TEXT)
                            .strong(),
                    );
                    ui.label(
                        egui::RichText::new("UTC TIME")
                            .font(FontId::new(8.0, FontFamily::Monospace))
                            .color(C::TEXT_DIM),
                    );
                });
            });
        });
    });
}

fn now_clock() -> String {
    use chrono::Utc;
    Utc::now().format("%H:%M:%S").to_string()
}