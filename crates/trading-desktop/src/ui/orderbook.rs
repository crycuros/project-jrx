//! Order book depth: asks (red) up top, spread banner in center, bids (green) below.

use egui::{pos2, vec2, FontFamily, FontId, Rect, Rounding, Ui};
use trading_core::OrderBook;

use super::components::{hline, mono, mono_bold, panel};
use crate::theme::C;

pub fn view(ui: &mut Ui, book: Option<&OrderBook>) {
    panel(
        ui,
        "ORDER BOOK",
        |_| {},
        |ui| match book {
            Some(b) if b.live && !b.asks.is_empty() && !b.bids.is_empty() => {
                let digits = b.asks[0].price.abs().max(b.bids[0].price.abs());
                let d = price_digits(digits);
                let asks: Vec<(f64, f64)> = b.asks.iter().take(6).map(|l| (l.price, l.qty)).collect();
                let bids: Vec<(f64, f64)> = b.bids.iter().take(6).map(|l| (l.price, l.qty)).collect();

                ui.vertical(|ui| {
                    // ── Asks (Red) ───────────────────────────────────────────
                    depth_rows(ui, &asks, true, d);

                    hline(ui);

                    // ── Spread Banner ────────────────────────────────────────
                    ui.horizontal(|ui| {
                        mono(ui, "SPREAD", C::TEXT_DIM, 9.0);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            mono_bold(ui, format!("{:>.*}", d as usize, b.spread), C::AMBER, 9.5);
                        });
                    });

                    hline(ui);

                    // ── Bids (Green) ─────────────────────────────────────────
                    depth_rows(ui, &bids, false, d);
                });
            }
            Some(b) => {
                let msg = if b.symbol.contains('_') || b.symbol.to_uppercase().starts_with("XAU") {
                    "NO PUBLIC L2 DEPTH\nFX / GOLD BROKER FEED"
                } else if !b.live {
                    "BOOK OFFLINE"
                } else {
                    "NO DEPTH DATA"
                };
                centered_hint(ui, msg);
            }
            None => centered_hint(ui, "NO LIVE BOOK"),
        },
    );
}

fn centered_hint(ui: &mut Ui, msg: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() / 3.0);
        mono(ui, msg, C::TEXT_DIM, 9.0);
    });
}

fn price_digits(p: f64) -> i32 {
    if p < 0.01 {
        8
    } else if p < 1.0 {
        6
    } else if p < 100.0 {
        4
    } else {
        2
    }
}

fn depth_rows(ui: &mut Ui, rows: &[(f64, f64)], is_ask: bool, digits: i32) {
    let color = if is_ask { C::RED } else { C::GREEN };
    let max_qty = rows.iter().map(|(_, q)| *q).fold(1.0f64, f64::max);

    for (price, vol) in rows {
        let frac = (*vol / max_qty).clamp(0.0, 1.0) as f32;
        let row_h = 16.0;
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), egui::Sense::hover());

        if ui.is_rect_visible(rect) {
            // Depth fill bar
            let fill_w = rect.width() * frac;
            let bar_rect = if is_ask {
                Rect::from_min_size(rect.min, vec2(fill_w, row_h))
            } else {
                Rect::from_min_max(pos2(rect.max.x - fill_w, rect.min.y), rect.max)
            };
            ui.painter().rect_filled(
                bar_rect,
                Rounding::same(1.0),
                C::with_alpha(color, 25),
            );

            // Price text (left)
            let p_str = format!("{price:>10.*}", digits as usize);
            ui.painter().text(
                pos2(rect.min.x + 4.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                p_str,
                FontId::new(9.0, FontFamily::Monospace),
                C::TEXT,
            );

            // Size text (right)
            let v_str = format!("{:>6.2}", vol);
            ui.painter().text(
                pos2(rect.max.x - 4.0, rect.center().y),
                egui::Align2::RIGHT_CENTER,
                v_str,
                FontId::new(9.0, FontFamily::Monospace),
                color,
            );
        }
    }
}