//! Bottom-right floating Cyber Toast Notifications for live executions and strategy alerts.

use std::time::{Duration, Instant};
use egui::{pos2, vec2, Align2, FontFamily, FontId, Rect, Rounding, Stroke};
use trading_core::{OrderSide, Trade};

use crate::theme::C;

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct Toast {
    pub id: u64,
    pub title: String,
    pub symbol: String,
    pub side: OrderSide,
    pub entry_price: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub amount: f64,
    pub strategy: String,
    pub confidence: f64,
    pub created_at: Instant,
    pub duration: Duration,
}

#[derive(Default)]
pub struct ToastManager {
    pub toasts: Vec<Toast>,
    next_id: u64,
    seen_position_keys: std::collections::HashSet<String>,
}

impl ToastManager {
    pub fn push_trade(&mut self, trade: &Trade) {
        self.next_id += 1;
        let strat = if !trade.reason.is_empty() {
            trade.reason.clone()
        } else if !trade.regime.is_empty() {
            format!("Regime: {}", trade.regime)
        } else {
            "AI Neural Signal Engine".to_string()
        };

        self.toasts.push(Toast {
            id: self.next_id,
            title: format!("NEW POSITION OPENED · {:?} {}", trade.side, trade.symbol),
            symbol: trade.symbol.clone(),
            side: trade.side,
            entry_price: trade.entry_price,
            stop_loss: trade.stop_loss,
            take_profit: trade.take_profit,
            amount: trade.amount,
            strategy: strat,
            confidence: trade.confidence,
            created_at: Instant::now(),
            duration: Duration::from_secs(8),
        });
    }

    /// Automatically checks snapshot positions and triggers toasts on newly opened positions.
    pub fn sync_positions(&mut self, positions: &[Trade]) {
        for pos in positions {
            let key = format!(
                "{}:{}:{:?}:{:.5}:{:.5}",
                pos.symbol, pos.entry_time, pos.side, pos.entry_price, pos.stop_loss
            );
            if !self.seen_position_keys.contains(&key) {
                self.seen_position_keys.insert(key);
                self.push_trade(pos);
            }
        }
    }

    /// Renders active toasts floating in the bottom-right viewport.
    pub fn render(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        // Retain only unexpired toasts
        self.toasts.retain(|t| now.duration_since(t.created_at) < t.duration);

        if self.toasts.is_empty() {
            return;
        }

        let screen_rect = ctx.screen_rect();
        let toast_w = 340.0;
        let toast_h = 96.0;
        let pad = 16.0;

        let mut bottom_y = screen_rect.max.y - pad;

        let mut remove_id = None;

        for toast in self.toasts.iter().rev().take(3) {
            let elapsed = now.duration_since(toast.created_at).as_secs_f32();
            let total_dur = toast.duration.as_secs_f32();
            let progress = (1.0 - elapsed / total_dur).clamp(0.0, 1.0);

            let toast_rect = Rect::from_min_size(
                pos2(screen_rect.max.x - toast_w - pad, bottom_y - toast_h),
                vec2(toast_w, toast_h),
            );
            bottom_y -= toast_h + 8.0;

            let side_col = if toast.side == OrderSide::Buy { C::GREEN } else { C::RED };

            egui::Area::new(egui::Id::new("toast_area").with(toast.id))
                .fixed_pos(toast_rect.min)
                .order(egui::Order::Tooltip)
                .show(ctx, |ui| {
                    // Dismiss button allocation
                    let close_btn_rect = Rect::from_min_size(
                        pos2(toast_rect.max.x - 22.0, toast_rect.min.y + 6.0),
                        vec2(16.0, 16.0),
                    );
                    let close_resp = ui.allocate_rect(close_btn_rect, egui::Sense::click());
                    if close_resp.clicked() {
                        remove_id = Some(toast.id);
                    }

                    let painter = ui.painter();

                    // Background with cyber glow border
                    painter.rect(
                        toast_rect,
                        Rounding::same(6.0),
                        C::with_alpha(C::BG_ELEVATED, 245),
                        Stroke::new(1.2_f32, side_col),
                    );

                    // Left accent bar
                    painter.rect_filled(
                        Rect::from_min_size(toast_rect.min, vec2(4.0, toast_h)),
                        Rounding::same(2.0),
                        side_col,
                    );

                    // Title header
                    let title_text = format!("⚡ {}", toast.title);
                    painter.text(
                        pos2(toast_rect.min.x + 12.0, toast_rect.min.y + 8.0),
                        Align2::LEFT_TOP,
                        title_text,
                        FontId::new(10.5, FontFamily::Monospace),
                        side_col,
                    );

                    painter.text(
                        close_btn_rect.center(),
                        Align2::CENTER_CENTER,
                        "✕",
                        FontId::new(11.0, FontFamily::Monospace),
                        if close_resp.hovered() { C::TEXT } else { C::TEXT_DIM },
                    );

                    // Details line 1: Entry, SL, TP
                    let price_line = format!(
                        "ENTRY: {:.4}   SL: {:.4}   TP: {:.4}",
                        toast.entry_price, toast.stop_loss, toast.take_profit
                    );
                    painter.text(
                        pos2(toast_rect.min.x + 12.0, toast_rect.min.y + 28.0),
                        Align2::LEFT_TOP,
                        price_line,
                        FontId::new(9.5, FontFamily::Monospace),
                        C::TEXT,
                    );

                    // Details line 2: Strategy info
                    let strat_line = format!("STRATEGY: {}", toast.strategy);
                    painter.text(
                        pos2(toast_rect.min.x + 12.0, toast_rect.min.y + 48.0),
                        Align2::LEFT_TOP,
                        strat_line,
                        FontId::new(9.0, FontFamily::Monospace),
                        C::CYAN,
                    );

                    // Details line 3: Confidence & Position Size
                    let meta_line = format!(
                        "CONFIDENCE: {:.0}%   SIZE: {:.0}",
                        toast.confidence * 100.0,
                        toast.amount
                    );
                    painter.text(
                        pos2(toast_rect.min.x + 12.0, toast_rect.min.y + 66.0),
                        Align2::LEFT_TOP,
                        meta_line,
                        FontId::new(9.0, FontFamily::Monospace),
                        C::TEXT_MUTED,
                    );

                    // Lifetime animated progress bar on bottom
                    let bar_h = 2.5;
                    let bar_w = (toast_w - 8.0) * progress;
                    let bar_rect = Rect::from_min_size(
                        pos2(toast_rect.min.x + 4.0, toast_rect.max.y - bar_h - 1.0),
                        vec2(bar_w, bar_h),
                    );
                    painter.rect_filled(bar_rect, Rounding::ZERO, side_col);
                });
        }

        if let Some(rid) = remove_id {
            self.toasts.retain(|t| t.id != rid);
        }
    }
}
