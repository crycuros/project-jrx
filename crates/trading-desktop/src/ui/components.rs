//! Shared UI helpers: modern terminal panels, gauges, badges, buttons.

use egui::{vec2, Color32, FontFamily, FontId, Pos2, Rect, Response, Rounding, Stroke, Ui};

use crate::theme::C;

/// Modern Panel Frame wrapper with header and content.
pub fn panel<R>(
    ui: &mut Ui,
    title: &str,
    right_fn: impl FnOnce(&mut Ui),
    content_fn: impl FnOnce(&mut Ui) -> R,
) -> R {
    C::panel_frame().show(ui, |ui| {
        ui.vertical(|ui| {
            // Header
            ui.horizontal(|ui| {
                ui.heading(
                    egui::RichText::new(title.to_uppercase())
                        .font(FontId::new(10.5, FontFamily::Monospace))
                        .color(C::TEXT)
                        .strong(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    right_fn(ui);
                });
            });

            ui.add_space(4.0);
            hline(ui);
            ui.add_space(6.0);

            // Content
            content_fn(ui)
        })
        .inner
    })
    .inner
}

/// Subtle horizontal separator line.
pub fn hline(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().line_segment(
        [rect.left_top(), rect.right_top()],
        Stroke::new(1.0_f32, C::BORDER),
    );
}

/// Monospace text label.
pub fn mono(ui: &mut Ui, s: impl Into<String>, color: Color32, size: f32) {
    ui.label(
        egui::RichText::new(s.into())
            .font(FontId::new(size, FontFamily::Monospace))
            .color(color),
    );
}

/// Strong monospace text label.
pub fn mono_bold(ui: &mut Ui, s: impl Into<String>, color: Color32, size: f32) {
    ui.label(
        egui::RichText::new(s.into())
            .font(FontId::new(size, FontFamily::Monospace))
            .color(color)
            .strong(),
    );
}

/// Sleek Chip Badge.
pub fn badge(ui: &mut Ui, text: &str, fg: Color32, bg: Color32) -> Response {
    let padding = vec2(7.0, 3.0);
    let font = FontId::new(9.0, FontFamily::Monospace);
    let galley = ui.painter().layout_no_wrap(text.to_uppercase(), font, fg);
    let size = galley.size() + padding * 2.0;

    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter().rect(
            rect,
            Rounding::same(3.0),
            bg,
            Stroke::new(1.0_f32, C::with_alpha(fg, 45)),
        );
        let text_pos = Pos2::new(
            rect.min.x + padding.x,
            rect.min.y + padding.y,
        );
        ui.painter().galley(text_pos, galley, fg);
    }
    response
}

/// Custom progress / risk gauge bar.
#[allow(dead_code)]
pub fn gauge(ui: &mut Ui, label: &str, pct: f32, color: Color32) {
    let pct_clamped = (pct / 100.0).clamp(0.0, 1.0);
    ui.horizontal(|ui| {
        ui.add_sized(
            [54.0, 14.0],
            egui::Label::new(
                egui::RichText::new(label.to_uppercase())
                    .font(FontId::new(9.0, FontFamily::Monospace))
                    .color(C::TEXT_DIM),
            ),
        );

        let height = 6.0;
        let width = (ui.available_width() - 48.0).max(20.0);
        let (rect, _) = ui.allocate_exact_size(vec2(width, height), egui::Sense::hover());

        if ui.is_rect_visible(rect) {
            // Track background
            ui.painter().rect_filled(rect, Rounding::same(2.0), C::BG_CARD);

            // Fill bar
            if pct_clamped > 0.001 {
                let fill_w = rect.width() * pct_clamped;
                let fill_rect = Rect::from_min_size(rect.min, vec2(fill_w, height));
                ui.painter().rect_filled(fill_rect, Rounding::same(2.0), color);
            }
        }

        ui.add_sized(
            [42.0, 14.0],
            egui::Label::new(
                egui::RichText::new(format!("{:>4.0}%", pct))
                    .font(FontId::new(9.0, FontFamily::Monospace))
                    .color(C::TEXT_MUTED),
            ),
        );
    });
}

/// Modern White-Dark tab/action button:
/// Active neutral/primary buttons render with a crisp Stark White (#FAFAFA) pill and dark #09090B text.
pub fn cyber_btn(ui: &mut Ui, label: &str, active: bool, color: Color32) -> Response {
    let padding = vec2(10.0, 5.0);
    let font = FontId::new(9.0, FontFamily::Monospace);
    let is_monochrome_accent = color == C::CYAN || color == C::WHITE || color == C::BLUE || color == C::TEXT;
    let text_color = if active {
        if is_monochrome_accent { C::BG } else { C::WHITE }
    } else {
        C::TEXT_MUTED
    };
    let galley = ui.painter().layout_no_wrap(label.to_uppercase(), font, text_color);
    let size = galley.size() + padding * 2.0;

    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let (bg, stroke) = if active {
            if is_monochrome_accent {
                (C::WHITE, Stroke::new(1.0_f32, C::WHITE))
            } else {
                (C::with_alpha(color, 45), Stroke::new(1.0_f32, color))
            }
        } else if response.hovered() {
            (
                C::BG_CARD_HOVER,
                Stroke::new(1.0_f32, C::BORDER_LIGHT),
            )
        } else {
            (
                C::BG_CARD,
                Stroke::new(1.0_f32, C::BORDER),
            )
        };

        ui.painter().rect(rect, Rounding::same(4.0), bg, stroke);
        let text_pos = Pos2::new(rect.min.x + padding.x, rect.min.y + padding.y);
        ui.painter().galley(text_pos, galley, text_color);
    }
    response
}

/// Animated live radar beacon dot.
pub fn live_beacon(ui: &mut Ui, is_live: bool, label: &str) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(10.0, 10.0), egui::Sense::hover());
        if ui.is_rect_visible(rect) {
            let center = rect.center();
            let color = if is_live { C::GREEN } else { C::AMBER };

            if is_live {
                let time = ui.input(|i| i.time);
                let pulse = (time * 2.5).sin().abs() as f32;
                // Outer radar ring
                ui.painter().circle_stroke(
                    center,
                    3.5 + pulse * 2.5,
                    Stroke::new(1.0_f32, C::with_alpha(color, (180.0 * (1.0 - pulse)) as u8)),
                );
            }

            // Center solid dot
            ui.painter().circle_filled(center, 3.0, color);
        }

        ui.label(
            egui::RichText::new(label)
                .font(FontId::new(9.0, FontFamily::Monospace))
                .color(if is_live { C::GREEN } else { C::AMBER })
                .strong(),
        );
    });
}