//! Bottom-left: Equity Curve (gradient area chart renderer with hover inspection).

use egui::{
    pos2, vec2, Align2, FontFamily, FontId, Rect, Rounding, Stroke, Ui,
};
use super::components::panel;
use crate::theme::C;

pub fn view(ui: &mut Ui, points: &[(f64, f64)], daily_pnl: f64) {
    let pnl_col = if daily_pnl >= 0.0 { C::GREEN } else { C::RED };
    panel(
        ui,
        "EQUITY CURVE",
        |ui| {
            ui.label(
                egui::RichText::new(format!("24H ROI: ${:+.2}", daily_pnl))
                    .font(FontId::new(9.5, FontFamily::Monospace))
                    .color(pnl_col)
                    .strong(),
            );
        },
        |ui| {
            let pts = if points.len() >= 2 {
                points.to_vec()
            } else {
                placeholder_series()
            };

            let (response, painter) = ui.allocate_painter(
                ui.available_size(),
                egui::Sense::hover(),
            );
            let bounds = response.rect;

            if !ui.is_rect_visible(bounds) || bounds.width() < 10.0 || bounds.height() < 10.0 {
                return;
            }

            let pad_x = 8.0;
            let pad_y = 12.0;
            let w = (bounds.width() - pad_x * 2.0).max(1.0);
            let h = (bounds.height() - pad_y * 2.0).max(1.0);

            let min_y = pts.iter().map(|(_, y)| *y).fold(f64::MAX, f64::min);
            let max_y = pts.iter().map(|(_, y)| *y).fold(f64::MIN, f64::max);
            let range_y = (max_y - min_y).max(1e-9);

            let (_, first) = pts[0];
            let (_, last) = *pts.last().unwrap();
            let up = last >= first;
            let line_color = if up { C::GREEN } else { C::RED };

            let n = pts.len().saturating_sub(1).max(1) as f32;
            let xy_of = |i: usize, _x: f64, y: f64| {
                pos2(
                    bounds.min.x + pad_x + (i as f32) / n * w,
                    bounds.min.y + pad_y + ((max_y - y) / range_y) as f32 * h,
                )
            };

            // ── Area Polygon Fill ─────────────────────────────────────────────────
            let mut mesh_points = Vec::with_capacity(pts.len() + 2);
            for (i, (x, y)) in pts.iter().enumerate() {
                mesh_points.push(xy_of(i, *x, *y));
            }

            // Draw area vertical strips or outline
            for i in 0..pts.len().saturating_sub(1) {
                let p1 = mesh_points[i];
                let p2 = mesh_points[i + 1];
                let base_y = bounds.max.y - pad_y;

                let quad = vec![
                    p1,
                    p2,
                    pos2(p2.x, base_y),
                    pos2(p1.x, base_y),
                ];
                painter.add(egui::Shape::convex_polygon(quad, C::with_alpha(line_color, 25), Stroke::NONE));
            }

            // ── Line Stroke ───────────────────────────────────────────────────────
            for i in 0..mesh_points.len().saturating_sub(1) {
                painter.line_segment(
                    [mesh_points[i], mesh_points[i + 1]],
                    Stroke::new(1.8_f32, line_color),
                );
            }

            // ── Glowing Live Endpoint Beacon ──────────────────────────────────────
            let time = ui.input(|i| i.time) as f32;
            if let Some(&last_pt) = mesh_points.last() {
                painter.circle_filled(last_pt, 3.5_f32, line_color);
                let pulse = (time * 3.0_f32).sin() * 0.5_f32 + 0.5_f32;
                painter.circle_stroke(
                    last_pt,
                    5.0_f32 + pulse * 4.0_f32,
                    Stroke::new(1.0_f32, C::with_alpha(line_color, (180.0_f32 * (1.0_f32 - pulse)) as u8)),
                );
            }

            // Latest Balance Metric Tag
            painter.text(
                pos2(bounds.max.x - pad_x - 4.0, bounds.min.y + 4.0),
                Align2::RIGHT_TOP,
                format!("${:.2}", last),
                FontId::new(10.0, FontFamily::Monospace),
                line_color,
            );

            // ── Hover Inspection Guide ────────────────────────────────────────────
            if let Some(cpos) = response.hover_pos() {
                if cpos.x >= bounds.min.x + pad_x && cpos.x <= bounds.max.x - pad_x {
                    let rel_x = (cpos.x - (bounds.min.x + pad_x)) / w;
                    let idx = (rel_x * n).clamp(0.0, n) as usize;
                    if let Some(&(px, py)) = pts.get(idx) {
                        let pt_pos = xy_of(idx, px, py);

                        painter.line_segment(
                            [pos2(pt_pos.x, bounds.min.y + pad_y), pos2(pt_pos.x, bounds.max.y - pad_y)],
                            Stroke::new(1.0_f32, C::with_alpha(C::CYAN, 90)),
                        );

                        painter.circle_filled(pt_pos, 4.0, C::CYAN);

                        let tip_rect = Rect::from_min_size(
                            pos2((pt_pos.x - 45.0).clamp(bounds.min.x, bounds.max.x - 90.0), pt_pos.y - 20.0),
                            vec2(90.0, 16.0),
                        );
                        painter.rect(tip_rect, Rounding::same(2.0), C::BG_ELEVATED, Stroke::new(1.0_f32, C::CYAN));
                        painter.text(
                            tip_rect.center(),
                            Align2::CENTER_CENTER,
                            format!("${:.2}", py),
                            FontId::new(9.0, FontFamily::Monospace),
                            C::CYAN,
                        );
                    }
                }
            }
        },
    );
}

fn placeholder_series() -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    let mut v = 10_000.0;
    for i in 0..80 {
        v += (i as f64 * 0.21).sin() * 14.0 + 4.0;
        out.push((i as f64, v));
    }
    out
}