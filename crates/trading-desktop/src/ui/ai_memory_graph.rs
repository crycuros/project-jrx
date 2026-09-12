//! AI Memory Graph — the AI's brain as a living star system.
//!
//! Three orbital tiers rendered on a canvas:
//!   • Core  — the AI engine itself (glowing cyan/white star, pulses with heartbeat)
//!   • Inner — live Strategy Signal nodes  (GO=green, WATCH=amber, NO-GO=red)
//!   • Mid   — Regime Memory nodes (BULL/BEAR/RANGING/BREAKOUT)
//!   • Outer — Asset Universe nodes (colored by market: forex=blue, gold=yellow)
//!
//! Background: procedural star field that slowly drifts.
//! Edges: confidence-weighted arcs from core to signal nodes, regime to asset nodes.

use iced::mouse;
use iced::widget::canvas::{self, Canvas, Fill, Frame, Geometry, Path, Stroke, Text};
use iced::{Color, Element, Length, Point, Rectangle, Renderer, Theme};

use trading_core::{AiMemoryNode, MemoryOrbit, NodeVerdict, Status};

use super::components::panel;
use crate::theme::C;

// *one italic private thought* — this graph doesn't lie; it shows exactly what the machine is thinking.

pub struct GalaxyState {
    hovered: Option<(u8, usize)>,
}
impl Default for GalaxyState {
    fn default() -> Self {
        Self { hovered: None }
    }
}

pub fn view<'a, M: 'a + 'static>(nodes: &[AiMemoryNode], status: &Status) -> Element<'a, M> {
    let nodes = if nodes.is_empty() { placeholder_nodes() } else { nodes.to_vec() };
    let running = status.running;
    let regime  = status.engine.clone();
    let ai_ok   = status.ai_available;
    let program = GalaxyView { nodes, running, regime, ai_ok };

    let right_label = super::components::mono_color(
        format!("AI {} · MEM {} NODES", if ai_ok { "ONLINE" } else { "OFFLINE" }, program.nodes.len()),
        if ai_ok { C::cyan() } else { C::dim() },
    );
    panel(
        "AI MEMORY GRAPH",
        Some(right_label),
        Canvas::new(program).width(Length::Fill).height(Length::Fill),
    )
}

pub struct GalaxyView {
    nodes:   Vec<AiMemoryNode>,
    running: bool,
    #[allow(dead_code)]
    regime:  String,
    ai_ok:   bool,
}

// ── orbit radii (fraction of min(w,h)/2) ──────────────────────────────────────
const R_INNER: f32 = 0.24;
const R_MID:   f32 = 0.42;
const R_OUTER: f32 = 0.60;

/// Node position on a ring (reused by draw + hover hit-testing so the cursor
/// always matches what is rendered).
fn orbit_pos(count: usize, r_frac: f32, phase: f32, speed: f32, cx: f32, cy: f32, rbase: f32, t: f32) -> Vec<(f32, f32)> {
    let n = count.max(1) as f32;
    let r = rbase * r_frac;
    (0..count)
        .map(|i| {
            let angle = (i as f32 / n) * std::f32::consts::TAU + phase + t * speed;
            let wobble_r = r + (t * 0.7 + i as f32 * 0.9).sin() * rbase * 0.015;
            (cx + angle.cos() * wobble_r, cy + angle.sin() * wobble_r)
        })
        .collect()
}

/// (orbit, index) of the node under the cursor, else None.
fn hit_test(
    signals: usize,
    regimes: usize,
    assets: usize,
    signal_pos: &[(f32, f32)],
    regime_pos: &[(f32, f32)],
    asset_pos: &[(f32, f32)],
    cpos: Point,
) -> Option<(u8, usize)> {
    let hit = |pos: &[(f32, f32)], i: usize| {
        let (x, y) = pos[i];
        let dx = cpos.x - x;
        let dy = cpos.y - y;
        dx * dx + dy * dy <= 90.0 * 90.0
    };
    for i in 0..signals {
        if hit(signal_pos, i) {
            return Some((0, i));
        }
    }
    for i in 0..regimes {
        if hit(regime_pos, i) {
            return Some((1, i));
        }
    }
    for i in 0..assets {
        if hit(asset_pos, i) {
            return Some((2, i));
        }
    }
    None
}

fn verdict_color(v: NodeVerdict) -> Color {
    match v {
        NodeVerdict::Go      => Color { r: 0.18, g: 0.95, b: 0.45, a: 1.0 },
        NodeVerdict::Watch   => Color { r: 1.0,  g: 0.72, b: 0.0,  a: 1.0 },
        NodeVerdict::NoGo    => Color { r: 0.95, g: 0.18, b: 0.18, a: 1.0 },
        NodeVerdict::Neutral => Color { r: 0.4,  g: 0.6,  b: 0.9,  a: 1.0 },
    }
}

fn market_color(m: trading_core::Market) -> Color {
    match m {
        trading_core::Market::Forex => Color { r: 0.2,  g: 0.55, b: 1.0,  a: 1.0 },
        trading_core::Market::Gold   => Color { r: 1.0,  g: 0.80, b: 0.1,  a: 1.0 },
    }
}

fn alpha(c: Color, a: f32) -> Color { Color { a: a * c.a, ..c } }

impl<Message: 'static> canvas::Program<Message> for GalaxyView {
    type State = GalaxyState;

    fn draw(
        &self,
        _state: &GalaxyState,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());

        let w  = bounds.width;
        let h  = bounds.height;
        let cx = w / 2.0;
        let cy = h / 2.0;
        let rbase = (w.min(h) * 0.5).max(40.0);

        // ── time clock ────────────────────────────────────────────────────────
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let t = now.as_millis() as f32 / 1000.0;  // seconds float

        // ═══════════════════════════════════════════════════════════════════════
        // 1. STAR FIELD  (180 procedural stars, slow parallax drift)
        // ═══════════════════════════════════════════════════════════════════════
        for i in 0u32..180 {
            let seed = (i as f32 * 137.508).fract() * 6.283;
            let seed2 = (i as f32 * 53.371).fract();
            let sx = (seed.cos() * 0.5 + 0.5) * w + (t * 0.06 + seed).sin() * 1.8;
            let sy = (seed2 * h) + (t * 0.04 + seed2 * 6.28).cos() * 1.5;
            let brightness = 0.08 + seed2 * 0.20;
            let size = 0.6 + seed2 * 1.0;
            let star = Path::circle(Point::new(sx % w, sy % h), size);
            frame.fill(&star, Fill {
                style: Color { r: brightness * 0.85, g: brightness * 0.9, b: brightness, a: brightness }.into(),
                ..Default::default()
            });
        }

        // ═══════════════════════════════════════════════════════════════════════
        // 2. ORBIT RINGS
        // ═══════════════════════════════════════════════════════════════════════
        for (r_frac, ring_col) in [
            (R_INNER, Color { r: 0.1, g: 0.7,  b: 0.9,  a: 0.10 }),
            (R_MID,   Color { r: 0.4, g: 0.4,  b: 1.0,  a: 0.08 }),
            (R_OUTER, Color { r: 0.6, g: 0.2,  b: 1.0,  a: 0.07 }),
        ] {
            let r = rbase * r_frac;
            let segs = 96usize;
            for s in 0..segs {
                if s % 3 == 0 { continue; }
                let a0 = (s as f32 / segs as f32) * std::f32::consts::TAU;
                let a1 = ((s + 1) as f32 / segs as f32) * std::f32::consts::TAU;
                let arc = Path::line(
                    Point::new(cx + a0.cos() * r, cy + a0.sin() * r),
                    Point::new(cx + a1.cos() * r, cy + a1.sin() * r),
                );
                frame.stroke(&arc, Stroke { style: ring_col.into(), width: 1.0, ..Default::default() });
            }
        }

        // ═══════════════════════════════════════════════════════════════════════
        // 3. CORE AI STAR
        // ═══════════════════════════════════════════════════════════════════════
        let heartbeat = (t * 1.8).sin() * 0.5 + 0.5;
        let core_col = if self.ai_ok {
            Color { r: 0.1, g: 0.95, b: 1.0, a: 1.0 }
        } else {
            Color { r: 0.5, g: 0.5, b: 0.5, a: 0.6 }
        };

        for (halo_r, halo_a) in [
            (rbase * 0.15, 0.04f32),
            (rbase * 0.09, 0.07),
            (rbase * 0.055, 0.12),
        ] {
            let halo_size = halo_r + heartbeat * halo_r * 0.18;
            let halo = Path::circle(Point::new(cx, cy), halo_size);
            frame.fill(&halo, Fill { style: alpha(core_col, halo_a + heartbeat * 0.03).into(), ..Default::default() });
        }

        let core_r = 7.0 + heartbeat * 2.5;
        let core = Path::circle(Point::new(cx, cy), core_r);
        frame.fill(&core, Fill { style: core_col.into(), ..Default::default() });

        for spike_angle in [0.0f32, std::f32::consts::FRAC_PI_2, std::f32::consts::PI, std::f32::consts::FRAC_PI_2 * 3.0] {
            let spike_len = rbase * 0.06 * (0.7 + heartbeat * 0.3);
            let spike_path = Path::line(
                Point::new(cx + spike_angle.cos() * core_r, cy + spike_angle.sin() * core_r),
                Point::new(cx + spike_angle.cos() * spike_len, cy + spike_angle.sin() * spike_len),
            );
            frame.stroke(&spike_path, Stroke {
                style: alpha(core_col, 0.6 + heartbeat * 0.3).into(),
                width: 1.5 + heartbeat,
                ..Default::default()
            });
        }

        // ═══════════════════════════════════════════════════════════════════════
        // 4. PARTITION NODES
        // ═══════════════════════════════════════════════════════════════════════
        let signals: Vec<&AiMemoryNode> = self.nodes.iter().filter(|n| n.orbit == MemoryOrbit::Signal).collect();
        let regimes: Vec<&AiMemoryNode> = self.nodes.iter().filter(|n| n.orbit == MemoryOrbit::Regime).collect();
        let assets:  Vec<&AiMemoryNode> = self.nodes.iter().filter(|n| n.orbit == MemoryOrbit::Asset).collect();

        let signal_pos: Vec<(f32, f32)> = orbit_pos(signals.len(), R_INNER, 0.3, 0.18, cx, cy, rbase, t);
        let regime_pos: Vec<(f32, f32)> = orbit_pos(regimes.len(), R_MID,   1.1, 0.07, cx, cy, rbase, t);
        let asset_pos:  Vec<(f32, f32)> = orbit_pos(assets.len(),  R_OUTER, 0.0, 0.035, cx, cy, rbase, t);

        // ═══════════════════════════════════════════════════════════════════════
        // 5. EDGES
        // ═══════════════════════════════════════════════════════════════════════
        for (i, node) in signals.iter().enumerate() {
            let (nx, ny) = signal_pos[i];
            let edge_alpha = (0.08 + node.confidence as f32 * 0.22)
                * (0.7 + (t * 2.5 + i as f32 * 0.8).sin() * 0.3);
            let ec = alpha(verdict_color(node.verdict), edge_alpha);
            let edge = Path::line(Point::new(cx, cy), Point::new(nx, ny));
            frame.stroke(&edge, Stroke {
                style: ec.into(),
                width: 0.5 + node.confidence as f32 * 1.5,
                ..Default::default()
            });
        }

        for (ri, _rnode) in regimes.iter().enumerate() {
            for (ai, anode) in assets.iter().enumerate() {
                if (ri * 3 + ai) % 5 != 0 { continue; }
                let (rx, ry) = regime_pos[ri];
                let (ax, ay) = asset_pos[ai];
                let pulse = (t * 1.2 + ri as f32 * 0.4 + ai as f32 * 0.3).sin() * 0.03;
                let edge = Path::line(Point::new(rx, ry), Point::new(ax, ay));
                frame.stroke(&edge, Stroke {
                    style: alpha(market_color(anode.market), 0.06 + pulse.abs()).into(),
                    width: 0.4,
                    ..Default::default()
                });
            }
        }

        // ═══════════════════════════════════════════════════════════════════════
        // 6. DRAW NODES
        // ═══════════════════════════════════════════════════════════════════════
        let draw_node = |frame: &mut Frame, nx: f32, ny: f32, node: &AiMemoryNode, base_r: f32| {
            let col = verdict_color(node.verdict);
            let market_col = market_color(node.market);
            let pulse = (t * 1.6 + nx * 0.01).sin() * 0.5 + 0.5;
            let r = base_r * (0.85 + node.confidence as f32 * 0.30 + pulse * 0.10);

            let glow = Path::circle(Point::new(nx, ny), r * 2.5);
            frame.fill(&glow, Fill { style: alpha(col, 0.07 + pulse * 0.04).into(), ..Default::default() });

            let ring = Path::circle(Point::new(nx, ny), r + 2.5);
            frame.stroke(&ring, Stroke {
                style: alpha(market_col, 0.35 + if node.active { pulse * 0.5 } else { 0.0 }).into(),
                width: 1.0 + if node.active { 1.5 } else { 0.0 },
                ..Default::default()
            });

            let body = Path::circle(Point::new(nx, ny), r);
            frame.fill(&body, Fill { style: alpha(col, 0.80).into(), ..Default::default() });

            if node.active {
                let active_ring = Path::circle(Point::new(nx, ny), r * 2.8);
                frame.stroke(&active_ring, Stroke {
                    style: alpha(col, 0.25 + pulse * 0.25).into(),
                    width: 1.5,
                    ..Default::default()
                });
            }

            frame.fill_text(Text {
                content: node.label.clone(),
                position: Point::new(nx + r + 3.5, ny - 5.5),
                color: Color { a: 0.85, ..C::text2() },
                size: iced::Pixels(8.0),
                ..Default::default()
            });

            if node.confidence > 0.01 {
                frame.fill_text(Text {
                    content: format!("{:.0}%", node.confidence * 100.0),
                    position: Point::new(nx - 6.0, ny + r + 3.0),
                    color: alpha(col, 0.75),
                    size: iced::Pixels(7.0),
                    ..Default::default()
                });
            }
        };

        for (i, node) in signals.iter().enumerate() {
            let (nx, ny) = signal_pos[i];
            draw_node(&mut frame, nx, ny, node, 5.5);
        }
        for (i, node) in regimes.iter().enumerate() {
            let (nx, ny) = regime_pos[i];
            draw_node(&mut frame, nx, ny, node, 6.5);
        }
        for (i, node) in assets.iter().enumerate() {
            let (nx, ny) = asset_pos[i];
            draw_node(&mut frame, nx, ny, node, 5.0);
        }

        // ── hovered node: highlight + tooltip with its memory/pattern detail ──
        if let Some(cpos) = cursor.position_in(bounds) {
            let cpos = Point::new(cpos.x, cpos.y);
            let hovered = hit_test(
                signals.len(), regimes.len(), assets.len(),
                &signal_pos, &regime_pos, &asset_pos,
                cpos,
            );
            let hovered_node: Option<(&AiMemoryNode, (f32, f32), f32)> = match hovered {
                Some((0, i)) => signals.get(i).map(|n| (*n, signal_pos[i], 5.5)),
                Some((1, i)) => regimes.get(i).map(|n| (*n, regime_pos[i], 6.5)),
                Some((2, i)) => assets.get(i).map(|n| (*n, asset_pos[i], 5.0)),
                _ => None,
            };
            if let Some((node, (nx, ny), base_r)) = hovered_node {
                let col = verdict_color(node.verdict);
                // highlight ring
                let hl = Path::circle(Point::new(nx, ny), base_r * 2.2);
                frame.stroke(&hl, Stroke {
                    style: alpha(col, 0.55).into(),
                    width: 2.0,
                    ..Default::default()
                });

                // tooltip box
                let tip_lines: Vec<String> = node.detail.lines()
                    .filter(|l| !l.trim().is_empty())
                    .map(|l| l.trim().to_string())
                    .collect();
                let tip_h = 16.0 + tip_lines.len() as f32 * 12.0;
                let tip_w = tip_lines.iter().map(|l| (l.len() as f32 * 5.8).min(300.0)).fold(150.0f32, f32::max) + 20.0;

                let mut bx = nx + base_r + 16.0;
                if bx + tip_w > w - 6.0 {
                    bx = nx - base_r - 16.0 - tip_w;
                }
                if bx < 6.0 {
                    bx = 6.0;
                }
                let mut by = ny - 12.0;
                if by + tip_h > h - 6.0 {
                    by = h - 6.0 - tip_h;
                }
                if by < 6.0 {
                    by = 6.0;
                }

                let box_r = Path::rectangle(Point::new(bx, by), iced::Size::new(tip_w, tip_h));
                frame.fill(&box_r, Fill { style: Color { r: 0.03, g: 0.05, b: 0.1, a: 0.92 }.into(), ..Default::default() });
                frame.stroke(&box_r, Stroke { style: alpha(col, 0.5).into(), width: 1.0, ..Default::default() });

                frame.fill_text(Text {
                    content: format!("{} · {:.0}%", node.label, node.confidence * 100.0),
                    position: Point::new(bx + 8.0, by + 8.0),
                    color: col,
                    size: iced::Pixels(10.0),
                    ..Default::default()
                });
                for (li, line) in tip_lines.iter().enumerate() {
                    frame.fill_text(Text {
                        content: line.clone(),
                        position: Point::new(bx + 10.0, by + 24.0 + li as f32 * 12.0),
                        color: Color { a: 0.9, ..C::text2() },
                        size: iced::Pixels(8.5),
                        ..Default::default()
                    });
                }
            }
        }

        // ═══════════════════════════════════════════════════════════════════════
        // 7. CORE LABEL + STATUS STRIP
        // ═══════════════════════════════════════════════════════════════════════
        frame.fill_text(Text {
            content: "AI CORE".to_string(),
            position: Point::new(cx - 14.0, cy + core_r + 5.0),
            color: alpha(core_col, 0.85),
            size: iced::Pixels(8.0),
            ..Default::default()
        });

        let status_str = format!(
            "SIGNAL {} · REGIME {} · ASSETS {} · ENGINE {}",
            signals.len(), regimes.len(), assets.len(),
            if self.running { "LIVE" } else { "IDLE" },
        );
        frame.fill_text(Text {
            content: status_str,
            position: Point::new(6.0, 4.0),
            color: if self.ai_ok { C::cyan() } else { C::dim() },
            size: iced::Pixels(9.0),
            ..Default::default()
        });

        vec![frame.into_geometry()]
    }

    fn mouse_interaction(&self, _state: &Self::State, bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        let Some(cpos) = cursor.position_in(bounds) else {
            return mouse::Interaction::default();
        };
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let t = t.as_millis() as f32 / 1000.0;
        let cx = bounds.width / 2.0;
        let cy = bounds.height / 2.0;
        let rbase = (bounds.width.min(bounds.height) * 0.5).max(40.0);

        let signals: Vec<&AiMemoryNode> = self.nodes.iter().filter(|n| n.orbit == MemoryOrbit::Signal).collect();
        let regimes: Vec<&AiMemoryNode> = self.nodes.iter().filter(|n| n.orbit == MemoryOrbit::Regime).collect();
        let assets:  Vec<&AiMemoryNode> = self.nodes.iter().filter(|n| n.orbit == MemoryOrbit::Asset).collect();

        let signal_pos = orbit_pos(signals.len(), R_INNER, 0.3, 0.18, cx, cy, rbase, t);
        let regime_pos = orbit_pos(regimes.len(), R_MID,   1.1, 0.07, cx, cy, rbase, t);
        let asset_pos  = orbit_pos(assets.len(),  R_OUTER, 0.0, 0.035, cx, cy, rbase, t);

        if hit_test(signals.len(), regimes.len(), assets.len(), &signal_pos, &regime_pos, &asset_pos, cpos).is_some() {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }
}

// ── Placeholder nodes for pre-launch state ────────────────────────────────────
fn placeholder_nodes() -> Vec<AiMemoryNode> {
    use trading_core::{Market, MemoryOrbit, NodeVerdict};
    let mut nodes = Vec::new();

    for (label, verdict, conf) in [
        ("EMA_CROSS", NodeVerdict::Go,    0.82),
        ("RSI_OB",    NodeVerdict::Watch, 0.55),
        ("BB_BREAK",  NodeVerdict::Go,    0.74),
        ("MACD_DIV",  NodeVerdict::NoGo,  0.31),
    ] {
        nodes.push(AiMemoryNode {
            id: label.to_string(), label: label.to_string(),
            orbit: MemoryOrbit::Signal, verdict, confidence: conf,
                market: Market::Forex, active: verdict == NodeVerdict::Go,
                detail: String::new(),
        });
    }

    for (label, verdict, conf) in [
        ("BULL",     NodeVerdict::Go,    0.68),
        ("BEAR",     NodeVerdict::NoGo,  0.45),
        ("RANGING",  NodeVerdict::Watch, 0.58),
        ("BREAKOUT", NodeVerdict::Go,    0.77),
    ] {
        nodes.push(AiMemoryNode {
            id: label.to_string(), label: label.to_string(),
            orbit: MemoryOrbit::Regime, verdict, confidence: conf,
                market: Market::Forex, active: label == "BULL",
                detail: String::new(),
        });
    }

    for (label, market, conf) in [
            ("EUR_USD", Market::Forex, 0.85),
            ("GBP_USD", Market::Forex, 0.71),
            ("USD_JPY", Market::Forex, 0.63),
            ("XAU_USD", Market::Gold, 0.80),
        ("XAU_USD",  Market::Gold,   0.59),
        ("EUR_USD",  Market::Forex,  0.52),
        ("GBP_USD",  Market::Forex,  0.48),
        ("USD_JPY",  Market::Forex,  0.39),
    ] {
        nodes.push(AiMemoryNode {
            id: label.to_string(), label: label.to_string(),
            orbit: MemoryOrbit::Asset, verdict: NodeVerdict::Neutral,
                confidence: conf, market, active: label == "EUR_USD",
                detail: String::new(),
        });
    }
    nodes
}
