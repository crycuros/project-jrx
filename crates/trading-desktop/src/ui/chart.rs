//! Center-top: DX-accelerated interactive candlestick price chart.
//! Interactive viewport: mouse wheel = zoom, left-drag = pan through history.
//! Real-time crosshair HUD, volume histogram, and glowing price beacon.

use egui::{
    pos2, vec2, Align2, Color32, FontFamily, FontId, Rect, Rounding, Stroke, Ui,
};
use trading_core::{Candle, OrderSide, Trade};

use crate::theme::C;

#[derive(Default, Clone)]
pub struct ChartState {
    /// Number of candles scrolled back from the right (latest) edge.
    pub first: usize,
    /// Number of candles visible in the window.
    pub shown: usize,
    /// Active drag anchor: (cursor x at press, `first` at press).
    pub drag: Option<(f32, f32)>,
}

impl ChartState {
    pub fn clamp(&mut self, total: usize) {
        if self.shown == 0 {
            self.shown = total.clamp(20, 50);
        }
        self.shown = self.shown.clamp(10, total.max(10));
        let max_first = total.saturating_sub(self.shown);
        self.first = self.first.min(max_first);
    }
}

pub fn view(
    ui: &mut Ui,
    state: &mut ChartState,
    candles: &[Candle],
    symbol: &str,
    tf: &str,
    positions: &[Trade],
) {
    let candles = if candles.len() >= 2 {
        candles.to_vec()
    } else {
        placeholder_candles()
    };
    let total = candles.len().max(1);
    state.clamp(total);

    let (response, painter) = ui.allocate_painter(
        ui.available_size(),
        egui::Sense::click_and_drag(),
    );
    let bounds = response.rect;

    if !ui.is_rect_visible(bounds) || bounds.width() < 10.0 || bounds.height() < 10.0 {
        return;
    }

    // ── Interaction: Mouse Wheel Zoom ──────────────────────────────────────────
    if response.hovered() {
        let scroll_delta = ui.input(|i| i.raw_scroll_delta.y);
        if scroll_delta.abs() > 0.0 {
            if scroll_delta > 0.0 {
                state.shown = state.shown.saturating_sub((state.shown / 8).max(1)).max(10);
            } else {
                state.shown = (state.shown + (state.shown / 8).max(1)).min(total);
            }
            state.clamp(total);
        }
    }

    // ── Interaction: Mouse Drag Pan ───────────────────────────────────────────
    if response.drag_started() {
        if let Some(pos) = response.interact_pointer_pos() {
            state.drag = Some((pos.x, state.first as f32));
        }
    }
    if response.dragged() {
        if let (Some((anchor_x, anchor_first)), Some(pos)) =
            (state.drag, response.interact_pointer_pos())
        {
            let span = (bounds.width() - 50.0).max(1.0);
            let per_px = state.shown as f32 / span;
            let nf = anchor_first + (pos.x - anchor_x) * per_px;
            state.first = (nf.max(0.0) as usize).min(total.saturating_sub(state.shown));
            state.clamp(total);
        }
    }
    if response.drag_stopped() {
        state.drag = None;
    }

    // ── Visible candles slice ──────────────────────────────────────────────────
    // state.first is the offset from the latest right edge (0 = live real-time candle).
    let end_idx = total.saturating_sub(state.first);
    let start_idx = end_idx.saturating_sub(state.shown);
    let vis = &candles[start_idx..end_idx];
    if vis.is_empty() {
        return;
    }

    let len = vis.len() as f32;
    let pad_left = 12.0;
    let pad_right = 65.0; // Space for right price axis labels
    let top_pad = 28.0;   // Space for top HUD
    let bottom_pad = 24.0;// Space for bottom time axis
    let vol_h = 42.0;     // Bottom volume section height

    let plot_w = (bounds.width() - pad_left - pad_right).max(1.0);
    let plot_h = (bounds.height() - top_pad - bottom_pad - vol_h).max(1.0);

    // Calculate High/Low range
    let mut hi = f32::MIN;
    let mut lo = f32::MAX;
    let mut max_vol = 1.0f32;

    for c in vis {
        hi = hi.max(c.high as f32);
        lo = lo.min(c.low as f32);
        max_vol = max_vol.max(c.volume as f32);
    }

    // Ensure chart bounding box always includes open position Entry, SL, and TP
    for pos in positions.iter().filter(|p| p.symbol == symbol) {
        if pos.entry_price > 0.0 {
            hi = hi.max(pos.entry_price as f32);
            lo = lo.min(pos.entry_price as f32);
        }
        if pos.take_profit > 0.0 {
            hi = hi.max(pos.take_profit as f32);
            lo = lo.min(pos.take_profit as f32);
        }
        if pos.stop_loss > 0.0 {
            hi = hi.max(pos.stop_loss as f32);
            lo = lo.min(pos.stop_loss as f32);
        }
    }

    if lo >= hi {
        let mid = if hi.is_finite() { hi } else { 1.0 };
        let spread = (mid * 0.002).max(1e-3);
        lo = mid - spread;
        hi = mid + spread;
    } else {
        let pad = (hi - lo) * 0.08;
        hi += pad;
        lo -= pad;
    }
    let range = (hi - lo).max(1e-9);

    let x_of = |i: usize| bounds.min.x + pad_left + (i as f32 + 0.5) / len * plot_w;
    let y_of = |v: f32| bounds.min.y + top_pad + (hi - v) / range * plot_h;
    let vol_y_of = |v: f32| bounds.max.y - bottom_pad - (v / max_vol) * vol_h;

    // ── Background Grid Lines ─────────────────────────────────────────────────
    for f in [0.0, 0.25, 0.5, 0.75, 1.0] {
        let price = hi - range * f;
        let y = y_of(price);

        painter.line_segment(
            [pos2(bounds.min.x + pad_left, y), pos2(bounds.max.x - pad_right, y)],
            Stroke::new(1.0_f32, C::with_alpha(C::BORDER, 120)),
        );

        // Price Axis Label on right
        painter.text(
            pos2(bounds.max.x - pad_right + 6.0, y),
            Align2::LEFT_CENTER,
            format!("{:.4}", price),
            FontId::new(9.0, FontFamily::Monospace),
            C::TEXT_DIM,
        );
    }

    // Vertical time grid lines
    let time_steps = [0, vis.len() / 2, vis.len().saturating_sub(1)];
    for &i in &time_steps {
        if i < vis.len() {
            let x = x_of(i);
            painter.line_segment(
                [pos2(x, bounds.min.y + top_pad), pos2(x, bounds.max.y - bottom_pad)],
                Stroke::new(1.0_f32, C::with_alpha(C::BORDER, 80)),
            );

            let t = vis[i].time;
            let epoch_sec = if t > 10_000_000_000 { t / 1000 } else { t };
            let dt_str = chrono::DateTime::<chrono::Utc>::from_timestamp(epoch_sec, 0)
                .map(|dt| dt.format("%H:%M:%S").to_string())
                .unwrap_or_default();

            painter.text(
                pos2(x, bounds.max.y - bottom_pad + 6.0),
                Align2::CENTER_TOP,
                dt_str,
                FontId::new(9.0, FontFamily::Monospace),
                C::TEXT_DIM,
            );
        }
    }

    // ── Volume Histogram ──────────────────────────────────────────────────────
    let body_w = (plot_w / len * 0.75).clamp(2.0, 24.0);
    for (i, c) in vis.iter().enumerate() {
        let x = x_of(i);
        let up = c.close >= c.open;
        let vol_col = if up {
            C::with_alpha(C::GREEN, 50)
        } else {
            C::with_alpha(C::RED, 50)
        };
        let vy = vol_y_of(c.volume as f32);
        let base_y = bounds.max.y - bottom_pad;
        let v_rect = Rect::from_min_max(
            pos2(x - body_w / 2.0, vy),
            pos2(x + body_w / 2.0, base_y),
        );
        painter.rect_filled(v_rect, Rounding::ZERO, vol_col);
    }

    // ── Candlesticks Rendering ────────────────────────────────────────────────
    for (i, c) in vis.iter().enumerate() {
        let o = c.open as f32;
        let cl = c.close as f32;
        let h_val = c.high as f32;
        let l_val = c.low as f32;
        let up = cl >= o;
        let color = if up { C::GREEN } else { C::RED };
        let x = x_of(i);

        // Wick
        let wick_top = y_of(h_val);
        let wick_bot = y_of(l_val);
        painter.line_segment(
            [pos2(x, wick_top), pos2(x, wick_bot)],
            Stroke::new(1.0_f32, color),
        );

        // Candle Body
        let body_top = y_of(o.max(cl));
        let body_bot = y_of(o.min(cl));
        let body_h = (body_bot - body_top).max(2.0);
        let body_rect = Rect::from_min_size(
            pos2(x - body_w / 2.0, body_top),
            vec2(body_w, body_h),
        );

        painter.rect_filled(body_rect, Rounding::same(1.0), color);
    }

    // ── Latest Price Beacon Line ──────────────────────────────────────────────
    if let Some(last) = vis.last() {
        let last_cl = last.close as f32;
        let last_y = y_of(last_cl);
        let up = last.close >= last.open;
        let last_col = if up { C::GREEN } else { C::RED };

        // Horizontal dashed/solid price line
        painter.line_segment(
            [pos2(bounds.min.x + pad_left, last_y), pos2(bounds.max.x - pad_right, last_y)],
            Stroke::new(1.0_f32, C::with_alpha(last_col, 160)),
        );

        // Right axis price badge
        let badge_rect = Rect::from_min_size(
            pos2(bounds.max.x - pad_right + 2.0, last_y - 7.0),
            vec2(58.0, 14.0),
        );
        painter.rect_filled(badge_rect, Rounding::same(2.0), last_col);
        painter.text(
            badge_rect.center(),
            Align2::CENTER_CENTER,
            format!("{:.4}", last_cl),
            FontId::new(9.0, FontFamily::Monospace),
            Color32::BLACK,
        );

        // Glowing pulsating dot on current price
        let pulse = (ui.input(|i| i.time) * 3.0).sin().abs() as f32;
        let dot_pos = pos2(x_of(vis.len() - 1), last_y);
        painter.circle_stroke(
            dot_pos,
            4.0 + pulse * 3.0,
            Stroke::new(1.0_f32, C::with_alpha(last_col, (200.0 * (1.0 - pulse)) as u8)),
        );
        painter.circle_filled(dot_pos, 3.0, last_col);
    }

    // ── Open Positions: Entry, Stop Loss & Take Profit Overlays ──────────────
    for pos in positions.iter().filter(|p| p.symbol == symbol) {
        let entry = pos.entry_price as f32;
        let sl = pos.stop_loss as f32;
        let tp = pos.take_profit as f32;
        let is_buy = pos.side == OrderSide::Buy;

        // 1. Entry Line & Badge
        let ey = y_of(entry);
        if entry >= lo && entry <= hi {
            painter.line_segment(
                [pos2(bounds.min.x + pad_left, ey), pos2(bounds.max.x - pad_right, ey)],
                Stroke::new(1.2_f32, C::with_alpha(C::CYAN, 200)),
            );
            let entry_badge = Rect::from_min_size(
                pos2(bounds.min.x + pad_left + 6.0, ey - 8.0),
                vec2(104.0, 16.0),
            );
            painter.rect_filled(entry_badge, Rounding::same(2.0), C::with_alpha(C::BG_ELEVATED, 240));
            painter.rect_stroke(entry_badge, Rounding::same(2.0), Stroke::new(1.0_f32, C::CYAN));
            painter.text(
                entry_badge.center(),
                Align2::CENTER_CENTER,
                format!("ENTRY {:?} {:.4}", pos.side, entry),
                FontId::new(8.5, FontFamily::Monospace),
                if is_buy { C::GREEN } else { C::RED },
            );
        }

        // 2. Take Profit Line & Badge (Green)
        if tp > 0.0 && tp >= lo && tp <= hi {
            let tpy = y_of(tp);
            painter.line_segment(
                [pos2(bounds.min.x + pad_left, tpy), pos2(bounds.max.x - pad_right, tpy)],
                Stroke::new(1.2_f32, C::with_alpha(C::GREEN, 220)),
            );
            let tp_badge = Rect::from_min_size(
                pos2(bounds.max.x - pad_right - 88.0, tpy - 8.0),
                vec2(84.0, 16.0),
            );
            painter.rect_filled(tp_badge, Rounding::same(2.0), C::with_alpha(C::GREEN, 50));
            painter.rect_stroke(tp_badge, Rounding::same(2.0), Stroke::new(1.0_f32, C::GREEN));
            painter.text(
                tp_badge.center(),
                Align2::CENTER_CENTER,
                format!("TP: {:.4}", tp),
                FontId::new(9.0, FontFamily::Monospace),
                C::GREEN,
            );

            // Shaded Green Profit Target Zone
            if entry >= lo && entry <= hi {
                let top_y = ey.min(tpy);
                let bot_y = ey.max(tpy);
                let corridor = Rect::from_min_max(
                    pos2(bounds.min.x + pad_left, top_y),
                    pos2(bounds.max.x - pad_right, bot_y),
                );
                painter.rect_filled(corridor, Rounding::ZERO, C::with_alpha(C::GREEN, 14));
            }
        }

        // 3. Stop Loss Line & Badge (Red)
        if sl > 0.0 && sl >= lo && sl <= hi {
            let sly = y_of(sl);
            painter.line_segment(
                [pos2(bounds.min.x + pad_left, sly), pos2(bounds.max.x - pad_right, sly)],
                Stroke::new(1.2_f32, C::with_alpha(C::RED, 220)),
            );
            let sl_badge = Rect::from_min_size(
                pos2(bounds.max.x - pad_right - 88.0, sly - 8.0),
                vec2(84.0, 16.0),
            );
            painter.rect_filled(sl_badge, Rounding::same(2.0), C::with_alpha(C::RED, 50));
            painter.rect_stroke(sl_badge, Rounding::same(2.0), Stroke::new(1.0_f32, C::RED));
            painter.text(
                sl_badge.center(),
                Align2::CENTER_CENTER,
                format!("SL: {:.4}", sl),
                FontId::new(9.0, FontFamily::Monospace),
                C::RED,
            );

            // Shaded Red Stop Loss Risk Zone
            if entry >= lo && entry <= hi {
                let top_y = ey.min(sly);
                let bot_y = ey.max(sly);
                let corridor = Rect::from_min_max(
                    pos2(bounds.min.x + pad_left, top_y),
                    pos2(bounds.max.x - pad_right, bot_y),
                );
                painter.rect_filled(corridor, Rounding::ZERO, C::with_alpha(C::RED, 14));
            }
        }
    }

    // ── Interactive Crosshair & Floating HUD ──────────────────────────────────
    let mut hovered_candle: Option<&Candle> = None;

    if let Some(cpos) = response.hover_pos() {
        if cpos.x >= bounds.min.x + pad_left && cpos.x <= bounds.max.x - pad_right
            && cpos.y >= bounds.min.y + top_pad && cpos.y <= bounds.max.y - bottom_pad
        {
            // Vertical crosshair guide
            painter.line_segment(
                [pos2(cpos.x, bounds.min.y + top_pad), pos2(cpos.x, bounds.max.y - bottom_pad)],
                Stroke::new(1.0_f32, C::with_alpha(C::CYAN, 80)),
            );

            // Horizontal crosshair guide
            painter.line_segment(
                [pos2(bounds.min.x + pad_left, cpos.y), pos2(bounds.max.x - pad_right, cpos.y)],
                Stroke::new(1.0_f32, C::with_alpha(C::CYAN, 80)),
            );

            // Cursor Price Pill on right axis (Stark White with Carbon Black text)
            let cursor_price = hi - ((cpos.y - (bounds.min.y + top_pad)) / plot_h) * range;
            let c_badge = Rect::from_min_size(
                pos2(bounds.max.x - pad_right + 2.0, cpos.y - 7.0),
                vec2(58.0, 14.0),
            );
            painter.rect(
                c_badge,
                Rounding::same(3.0),
                C::WHITE,
                Stroke::new(1.0_f32, C::WHITE),
            );
            painter.text(
                c_badge.center(),
                Align2::CENTER_CENTER,
                format!("{:.4}", cursor_price),
                FontId::new(9.0, FontFamily::Monospace),
                C::BG,
            );

            // Identify candle under cursor
            let rel_x = cpos.x - (bounds.min.x + pad_left);
            let c_idx = ((rel_x / plot_w) * len).clamp(0.0, len - 1.0) as usize;
            if let Some(c) = vis.get(c_idx) {
                hovered_candle = Some(c);
            }
        }
    }

    // ── Top Header / HUD ──────────────────────────────────────────────────────
    let display_candle = hovered_candle.or_else(|| vis.last());
    if let Some(c) = display_candle {
        let chg = (c.close - c.open) / c.open * 100.0;
        let chg_col = if chg >= 0.0 { C::GREEN } else { C::RED };
        let hud_text = format!(
            "{} · {}  O: {:.4}  H: {:.4}  L: {:.4}  C: {:.4}  VOL: {:.0}  {:+.2}%",
            symbol, tf, c.open, c.high, c.low, c.close, c.volume, chg
        );

        painter.text(
            pos2(bounds.min.x + pad_left, bounds.min.y + 10.0),
            Align2::LEFT_CENTER,
            hud_text,
            FontId::new(10.0, FontFamily::Monospace),
            chg_col,
        );
    }
}

fn placeholder_candles() -> Vec<Candle> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut candles = Vec::new();
    let mut price = 1.0850_f64;
    for i in 0..120 {
        let ts = now - (120 - i as u64) * 300;
        let drift = (i as f64 * 0.12).sin() * 0.0015;
        let open = price;
        let close = (price + drift).max(0.1);
        let high = open.max(close) + 0.0008;
        let low = open.min(close) - 0.0008;
        candles.push(Candle {
            time: ts as i64,
            open,
            high,
            low,
            close,
            volume: 250.0 + (i as f64 * 5.3).sin().abs() * 300.0,
        });
        price = close;
    }
    candles
}