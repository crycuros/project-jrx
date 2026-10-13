//! Pipeline Console — DAG graph with animated data flow packets in immediate mode.
//!
//! Stages: RAW → NORMALIZED → FEATURES → WEIGHTS → PNL
//! Interactive: hover a table to highlight its edges, click to pin it and
//! inspect upstream/downstream step breakdown.

use egui::{
    pos2, vec2, Align2, Color32, FontFamily, FontId, Pos2, Rect, Rounding, Stroke, Ui,
};
use std::collections::HashSet;

use crate::theme::C;

const SLAB_W: f32 = 148.0;
const SLAB_H: f32 = 68.0;
const COL_GAP: f32 = 72.0;
const ROW_GAP: f32 = 16.0;
const RAIL_H: f32 = 34.0;
const ARC_CTRL: f32 = 0.38;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Raw,
    Normalized,
    Features,
    Weights,
    Pnl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableState {
    Empty,
    Live,
    Ok,
}

pub struct TableNode {
    pub name: String,
    pub rows: String,
    pub stage: Stage,
    pub state: TableState,
    pub col: usize,
    pub row: usize,
}

pub struct Edge {
    pub from_col: usize,
    pub from_row: usize,
    pub to_col: usize,
    pub to_row: usize,
    pub label: String,
    pub has_data: bool,
}

#[derive(Default, Clone)]
pub struct PipelineState {
    pub hovered: Option<usize>,
    pub selected: Option<usize>,
}

fn stage_color(s: Stage) -> Color32 {
    match s {
        Stage::Raw => C::GREEN,
        Stage::Normalized => C::CYAN,
        Stage::Features => C::BLUE,
        Stage::Weights => C::AMBER,
        Stage::Pnl => C::PURPLE,
    }
}

fn build_graph(
    snapshot: &trading_core::UiSnapshot,
) -> (Vec<TableNode>, Vec<Edge>, Vec<(usize, &'static str, Color32)>) {
    let s = &snapshot.status;
    let candle_count: usize = snapshot.candles.values().map(|v| v.len()).sum();
    let symbols: usize = snapshot.candles.len();
    let news_count = snapshot.news.len();
    let trade_count = snapshot.recent_trades.len();
    let position_count = s.open_positions as usize;
    let engine_running = s.running;

    let prices_ok = candle_count > 0;
    let news_ok = news_count > 0;
    let weights_ok = trade_count > 0 || position_count > 0;
    let pnl_ok = trade_count > 0;

    let mut nodes = Vec::new();

    // ── Col 0: RAW ──────────────────────────────────────────────────────────
    nodes.push(TableNode {
        name: "feed_prices".into(),
        rows: format!("LIVE · {candle_count} bars"),
        stage: Stage::Raw,
        state: if engine_running { TableState::Live } else { TableState::Empty },
        col: 0,
        row: 0,
    });
    nodes.push(TableNode {
        name: "feed_news".into(),
        rows: format!("{news_count} articles"),
        stage: Stage::Raw,
        state: if news_ok { TableState::Live } else { TableState::Empty },
        col: 0,
        row: 1,
    });

    // ── Col 1: NORMALIZED ───────────────────────────────────────────────────
    nodes.push(TableNode {
        name: "ohlcv_series".into(),
        rows: format!("{symbols} symbols · {candle_count} bars"),
        stage: Stage::Normalized,
        state: if prices_ok { TableState::Ok } else { TableState::Empty },
        col: 1,
        row: 0,
    });

    // ── Col 2: FEATURES ─────────────────────────────────────────────────────
    nodes.push(TableNode {
        name: "momentum_idx".into(),
        rows: "EMA cross · RSI · MACD".into(),
        stage: Stage::Features,
        state: if prices_ok { TableState::Ok } else { TableState::Empty },
        col: 2,
        row: 0,
    });
    nodes.push(TableNode {
        name: "volatility_atr".into(),
        rows: "ATR · Bollinger Bands".into(),
        stage: Stage::Features,
        state: if prices_ok { TableState::Ok } else { TableState::Empty },
        col: 2,
        row: 1,
    });
    nodes.push(TableNode {
        name: "news_sentiment".into(),
        rows: format!("{news_count} vectorized"),
        stage: Stage::Features,
        state: if news_ok { TableState::Ok } else { TableState::Empty },
        col: 2,
        row: 2,
    });

    // ── Col 3: WEIGHTS ──────────────────────────────────────────────────────
    nodes.push(TableNode {
        name: "risk_allocator".into(),
        rows: "portfolio stake · top 4".into(),
        stage: Stage::Weights,
        state: if weights_ok { TableState::Live } else { TableState::Ok },
        col: 3,
        row: 0,
    });
    nodes.push(TableNode {
        name: "alpha_momentum".into(),
        rows: "trend weight".into(),
        stage: Stage::Weights,
        state: if prices_ok { TableState::Ok } else { TableState::Empty },
        col: 3,
        row: 1,
    });
    nodes.push(TableNode {
        name: "alpha_reversal".into(),
        rows: "mean-revert weight".into(),
        stage: Stage::Weights,
        state: if prices_ok { TableState::Ok } else { TableState::Empty },
        col: 3,
        row: 2,
    });
    nodes.push(TableNode {
        name: "alpha_low_vol".into(),
        rows: "min-variance weight".into(),
        stage: Stage::Weights,
        state: if prices_ok { TableState::Ok } else { TableState::Empty },
        col: 3,
        row: 3,
    });

    // ── Col 4: PNL ──────────────────────────────────────────────────────────
    nodes.push(TableNode {
        name: "net_pnl_tracker".into(),
        rows: format!("${:+.2} total", s.total_pnl),
        stage: Stage::Pnl,
        state: if pnl_ok { TableState::Ok } else { TableState::Empty },
        col: 4,
        row: 0,
    });
    nodes.push(TableNode {
        name: "win_rate_metric".into(),
        rows: format!("{:.1}% win rate", s.win_rate),
        stage: Stage::Pnl,
        state: if pnl_ok { TableState::Ok } else { TableState::Empty },
        col: 4,
        row: 1,
    });
    nodes.push(TableNode {
        name: "trade_logger".into(),
        rows: format!("{trade_count} trades"),
        stage: Stage::Pnl,
        state: if pnl_ok { TableState::Ok } else { TableState::Empty },
        col: 4,
        row: 2,
    });
    nodes.push(TableNode {
        name: "balance_monitor".into(),
        rows: format!("${:.2} balance", s.balance),
        stage: Stage::Pnl,
        state: if pnl_ok { TableState::Ok } else { TableState::Empty },
        col: 4,
        row: 3,
    });

    // ── Edges ───────────────────────────────────────────────────────────────
    let edges = vec![
        Edge { from_col: 0, from_row: 0, to_col: 1, to_row: 0, label: "normalize".into(), has_data: prices_ok },
        Edge { from_col: 1, from_row: 0, to_col: 2, to_row: 0, label: "calc_mom".into(), has_data: prices_ok },
        Edge { from_col: 1, from_row: 0, to_col: 2, to_row: 1, label: "calc_atr".into(), has_data: prices_ok },
        Edge { from_col: 0, from_row: 1, to_col: 2, to_row: 2, label: "nlp_tone".into(), has_data: news_ok },
        Edge { from_col: 2, from_row: 0, to_col: 3, to_row: 0, label: "blend".into(), has_data: prices_ok && news_ok },
        Edge { from_col: 2, from_row: 1, to_col: 3, to_row: 0, label: "blend".into(), has_data: prices_ok && news_ok },
        Edge { from_col: 2, from_row: 2, to_col: 3, to_row: 0, label: "blend".into(), has_data: prices_ok && news_ok },
        Edge { from_col: 1, from_row: 0, to_col: 3, to_row: 1, label: "alpha_mom".into(), has_data: prices_ok },
        Edge { from_col: 1, from_row: 0, to_col: 3, to_row: 2, label: "alpha_rev".into(), has_data: prices_ok },
        Edge { from_col: 1, from_row: 0, to_col: 3, to_row: 3, label: "alpha_vol".into(), has_data: prices_ok },
        Edge { from_col: 3, from_row: 0, to_col: 4, to_row: 0, label: "execute".into(), has_data: weights_ok },
        Edge { from_col: 3, from_row: 1, to_col: 4, to_row: 1, label: "execute".into(), has_data: prices_ok },
        Edge { from_col: 3, from_row: 2, to_col: 4, to_row: 2, label: "execute".into(), has_data: prices_ok },
        Edge { from_col: 3, from_row: 3, to_col: 4, to_row: 3, label: "execute".into(), has_data: prices_ok },
    ];

    let bands = vec![
        (0usize, "RAW", C::GREEN),
        (1, "NORMALIZED", C::CYAN),
        (2, "FEATURES", C::BLUE),
        (3, "WEIGHTS", C::AMBER),
        (4, "PNL", C::PURPLE),
    ];

    (nodes, edges, bands)
}

fn node_index(nodes: &[TableNode], col: usize, row: usize) -> Option<usize> {
    nodes.iter().position(|n| n.col == col && n.row == row)
}

struct Layout {
    ox: f32,
    oy: f32,
    sc: f32,
}

impl Layout {
    fn new(bounds: Rect, nodes: &[TableNode]) -> Self {
        let total_w = 5.0 * SLAB_W + 4.0 * COL_GAP;
        let max_rows = nodes.iter().map(|n| n.row).max().unwrap_or(3) + 1;
        let total_h = max_rows as f32 * SLAB_H + (max_rows as f32 - 1.0) * ROW_GAP;

        let sx = ((bounds.width() - 30.0) / total_w).min(1.0);
        let sy = ((bounds.height() - RAIL_H - 24.0) / total_h).min(1.0);
        let sc = sx.min(sy).max(0.4);

        Self {
            ox: bounds.min.x + (bounds.width() - total_w * sc) / 2.0,
            oy: bounds.min.y + RAIL_H + (bounds.height() - RAIL_H - total_h * sc) / 2.0,
            sc,
        }
    }

    fn node_x(&self, col: usize) -> f32 {
        self.ox + col as f32 * (SLAB_W + COL_GAP) * self.sc
    }

    fn node_y(&self, row: usize) -> f32 {
        self.oy + row as f32 * (SLAB_H + ROW_GAP) * self.sc
    }

    fn node_rect(&self, col: usize, row: usize) -> Rect {
        Rect::from_min_size(
            pos2(self.node_x(col), self.node_y(row)),
            vec2(SLAB_W * self.sc, SLAB_H * self.sc),
        )
    }
}

fn bezier_point(
    p0: Pos2,
    cp0: Pos2,
    cp1: Pos2,
    p1: Pos2,
    t: f32,
) -> Pos2 {
    let mt = 1.0 - t;
    let x = mt * mt * mt * p0.x + 3.0 * mt * mt * t * cp0.x + 3.0 * mt * t * t * cp1.x + t * t * t * p1.x;
    let y = mt * mt * mt * p0.y + 3.0 * mt * mt * t * cp0.y + 3.0 * mt * t * t * cp1.y + t * t * t * p1.y;
    pos2(x, y)
}

pub fn view(
    ui: &mut Ui,
    state: &mut PipelineState,
    snapshot: &trading_core::UiSnapshot,
) {
    let (nodes, edges, bands) = build_graph(snapshot);

    let (response, painter) = ui.allocate_painter(
        ui.available_size(),
        egui::Sense::click_and_drag(),
    );
    let bounds = response.rect;

    if !ui.is_rect_visible(bounds) || bounds.width() < 10.0 || bounds.height() < 10.0 {
        return;
    }

    let layout = Layout::new(bounds, &nodes);
    let sc = layout.sc;
    let time = ui.input(|i| i.time) as f32;

    // ── Hit Testing ───────────────────────────────────────────────────────────
    let mut hovered_node = None;
    if let Some(cpos) = response.hover_pos() {
        for (i, n) in nodes.iter().enumerate() {
            if layout.node_rect(n.col, n.row).contains(cpos) {
                hovered_node = Some(i);
                break;
            }
        }
    }
    state.hovered = hovered_node;

    if response.clicked() {
        state.selected = state.hovered;
    }

    let focus = state.selected.or(state.hovered);
    let focus_set = focus.map(|f| {
        let mut set = HashSet::new();
        set.insert(f);
        for e in &edges {
            if let (Some(a), Some(b)) = (
                node_index(&nodes, e.from_col, e.from_row),
                node_index(&nodes, e.to_col, e.to_row),
            ) {
                if a == f { set.insert(b); }
                if b == f { set.insert(a); }
            }
        }
        set
    });

    // ── 1. Column Stage Bands ─────────────────────────────────────────────────
    for (ci, label, col) in &bands {
        let x0 = layout.node_x(*ci) - COL_GAP * sc * 0.5;
        let x1 = x0 + (SLAB_W + COL_GAP) * sc;
        let pulse = (time * 1.5 + *ci as f32 * 0.8).sin() * 0.5 + 0.5;

        // Band subtle fill
        painter.rect_filled(
            Rect::from_min_max(pos2(x0, bounds.min.y), pos2(x1, bounds.max.y)),
            Rounding::ZERO,
            C::with_alpha(*col, 6),
        );

        // Header label
        let cx = (x0 + x1) / 2.0;
        painter.text(
            pos2(cx, bounds.min.y + 10.0),
            Align2::CENTER_TOP,
            *label,
            FontId::new((10.0 * sc).max(8.0), FontFamily::Monospace),
            C::with_alpha(*col, (180.0 + pulse * 60.0) as u8),
        );
    }

    // ── 2. Edges & Flowing Data Packets ───────────────────────────────────────
    for e in &edges {
        let fa = node_index(&nodes, e.from_col, e.from_row);
        let ta = node_index(&nodes, e.to_col, e.to_row);

        let related = match (fa, ta, focus, &focus_set) {
            (Some(a), Some(b), Some(fi), Some(set)) => a == fi || b == fi || (set.contains(&a) && set.contains(&b)),
            _ => false,
        };

        let x0 = layout.node_x(e.from_col) + SLAB_W * sc;
        let y0 = layout.node_y(e.from_row) + SLAB_H * sc * 0.5;
        let x1 = layout.node_x(e.to_col);
        let y1 = layout.node_y(e.to_row) + SLAB_H * sc * 0.5;

        let dx = x1 - x0;
        let p0 = pos2(x0, y0);
        let cp0 = pos2(x0 + dx * ARC_CTRL, y0);
        let cp1 = pos2(x1 - dx * ARC_CTRL, y1);
        let p1 = pos2(x1, y1);

        let live = if focus.is_some() { related } else { e.has_data };
        let edge_col = if live { C::with_alpha(C::GREEN, 140) } else { C::with_alpha(C::BORDER, 100) };
        let stroke_w = if live { 1.5 * sc } else { 0.8 * sc };

        // Draw cubic bezier curve in segments
        const STEPS: usize = 20;
        let mut prev = p0;
        for s in 1..=STEPS {
            let t = s as f32 / STEPS as f32;
            let pt = bezier_point(p0, cp0, cp1, p1, t);
            painter.line_segment([prev, pt], Stroke::new(stroke_w, edge_col));
            prev = pt;
        }

        // Animated traveling packets
        if live {
            let speed = 0.45 + (x0 * 0.01 + y0 * 0.007).fract() * 0.15;
            for p in 0..2 {
                let tt = ((time * speed + p as f32 / 2.0) % 1.0).abs();
                let pt = bezier_point(p0, cp0, cp1, p1, tt);
                let fade = 1.0 - (tt - 0.5).abs() * 2.0;

                painter.circle_filled(
                    pt,
                    3.5 * sc,
                    C::with_alpha(C::GREEN, (180.0 * fade) as u8),
                );
            }
        }
    }

    // ── 3. Table Slabs / Nodes ────────────────────────────────────────────────
    for (i, n) in nodes.iter().enumerate() {
        let sc_col = stage_color(n.stage);
        let rect = layout.node_rect(n.col, n.row);
        let is_focus = Some(i) == focus;
        let pulse = (time * 2.0 + i as f32 * 0.7).sin() * 0.5 + 0.5;

        // Hover / Selection Glow
        if is_focus {
            painter.rect_filled(
                rect.expand(4.0 * sc),
                Rounding::same(6.0 * sc),
                C::with_alpha(sc_col, 35),
            );
        }

        // Slab body
        painter.rect(
            rect,
            Rounding::same(4.0 * sc),
            C::BG_CARD,
            Stroke::new(
                if is_focus { 1.8_f32 } else { 1.0_f32 },
                if is_focus { C::CYAN } else if n.state == TableState::Live { C::GREEN } else { C::BORDER },
            ),
        );

        // Top color bar
        let bar_h = 2.5 * sc;
        painter.rect_filled(
            Rect::from_min_size(rect.min, vec2(rect.width(), bar_h)),
            Rounding::same(2.0 * sc),
            sc_col,
        );

        // Node Title
        painter.text(
            pos2(rect.min.x + 8.0 * sc, rect.min.y + 8.0 * sc),
            Align2::LEFT_TOP,
            &n.name,
            FontId::new((10.0 * sc).max(8.0), FontFamily::Monospace),
            if is_focus { C::TEXT } else { C::TEXT_MUTED },
        );

        // Node Row/Metric detail
        painter.text(
            pos2(rect.min.x + 8.0 * sc, rect.min.y + 24.0 * sc),
            Align2::LEFT_TOP,
            &n.rows,
            FontId::new((8.5 * sc).max(7.0), FontFamily::Monospace),
            C::TEXT_DIM,
        );

        // Status badge at bottom
        let status_str = match n.state {
            TableState::Empty => "IDLE",
            TableState::Live => "LIVE",
            TableState::Ok => "SYNCED",
        };
        let status_col = match n.state {
            TableState::Empty => C::TEXT_DIM,
            TableState::Live => C::GREEN,
            TableState::Ok => C::CYAN,
        };

        painter.text(
            pos2(rect.max.x - 6.0 * sc, rect.max.y - 6.0 * sc),
            Align2::RIGHT_BOTTOM,
            status_str,
            FontId::new((7.5 * sc).max(6.5), FontFamily::Monospace),
            status_col,
        );

        // Live progress pulse line
        if n.state == TableState::Live {
            let p_bar_w = rect.width() - 16.0 * sc;
            let sweep = pulse * p_bar_w;
            painter.rect_filled(
                Rect::from_min_size(pos2(rect.min.x + 8.0 * sc, rect.max.y - 5.0 * sc), vec2(sweep, 2.0 * sc)),
                Rounding::same(1.0),
                C::GREEN,
            );
        }
    }

    // ── 4. Bottom Detail Inspector Ribbon ─────────────────────────────────────
    let status_y = bounds.max.y - 12.0;
    if let Some(idx) = focus {
        if let Some(node) = nodes.get(idx) {
            let mut out_names: Vec<&str> = Vec::new();
            let mut in_count = 0usize;
            for e in &edges {
                if (e.from_col, e.from_row) == (node.col, node.row) {
                    if !out_names.contains(&e.label.as_str()) {
                        out_names.push(&e.label);
                    }
                }
                if (e.to_col, e.to_row) == (node.col, node.row) {
                    in_count += 1;
                }
            }

            let out_str = out_names.join(", ");
            let out_summary = if out_names.is_empty() { "TERMINAL" } else { out_str.as_str() };

            let detail = format!(
                "PIPELINE NODE :: {} · {} · IN {} · OUT {} [ {} ]",
                node.name.to_uppercase(),
                node.rows,
                in_count,
                out_names.len(),
                out_summary
            );

            painter.text(
                pos2(bounds.min.x + 8.0, status_y),
                Align2::LEFT_BOTTOM,
                detail,
                FontId::new(9.0, FontFamily::Monospace),
                stage_color(node.stage),
            );
        }
    } else {
        let s = &snapshot.status;
        painter.text(
            pos2(bounds.min.x + 8.0, status_y),
            Align2::LEFT_BOTTOM,
            format!(
                "PIPELINE DAG :: ACTIVE {} · BALANCE ${:.2} · P/L ${:+.2} · WIN RATE {:.1}%",
                s.active_engine.to_uppercase(),
                s.balance,
                s.daily_pnl,
                s.win_rate
            ),
            FontId::new(9.0, FontFamily::Monospace),
            C::CYAN,
        );
    }
}