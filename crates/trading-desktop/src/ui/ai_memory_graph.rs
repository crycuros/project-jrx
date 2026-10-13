//! AI Neural Memory Topology — 3D Volumetric Space Simulation.
//!
//! Features:
//!   • Full 3D perspective projection with depth sorting (Z-buffer / Painter's algorithm)
//!   • Interactive 3D camera controls: Left-drag = 3D Orbit Rotate, Wheel = 3D Zoom
//!   • Volumetric 3D glowing starfield with stellar parallax
//!   • Multi-tier tilted celestial orbital planes with depth gradient
//!   • Pulsating central AI Singularity with coronal plasma glow shells & solar flares
//!   • 3D Synaptic energy rays with live streaming photon data packets
//!   • Directional shaded 3D node spheres with billboarded depth-scaled labels
//!   • Interactive 3D hit testing and holographic tooltip cards

use egui::{
    pos2, vec2, Align2, Color32, FontFamily, FontId, Pos2, Rect, Rounding, Stroke, Ui,
};
use trading_core::{AiMemoryNode, MemoryOrbit, NodeVerdict, Status};

use crate::theme::C;

#[derive(Clone)]
pub struct NeuralMemoryState {
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
    pub auto_orbit: bool,
    pub hovered: Option<(u8, usize)>,
    pub selected: Option<(u8, usize)>,
}

impl Default for NeuralMemoryState {
    fn default() -> Self {
        Self {
            yaw: 0.45,
            pitch: 0.55,
            zoom: 1.0,
            auto_orbit: true,
            hovered: None,
            selected: None,
        }
    }
}

#[derive(Clone, Copy)]
struct Vec3 {
    x: f32,
    y: f32,
    z: f32,
}

impl Vec3 {
    fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }
}

struct Projected3D {
    screen: Pos2,
    depth: f32,
    scale: f32,
}

fn project_3d(
    p: Vec3,
    yaw: f32,
    pitch: f32,
    zoom: f32,
    center: Pos2,
    world_scale: f32,
) -> Projected3D {
    // 1. Yaw rotation (Y axis)
    let (sin_y, cos_y) = yaw.sin_cos();
    let x1 = p.x * cos_y - p.z * sin_y;
    let z1 = p.x * sin_y + p.z * cos_y;
    let y1 = p.y;

    // 2. Pitch rotation (X axis)
    let (sin_p, cos_p) = pitch.sin_cos();
    let y2 = y1 * cos_p - z1 * sin_p;
    let z2 = y1 * sin_p + z1 * cos_p;
    let x2 = x1;

    // 3. Perspective Camera
    let cam_dist = 550.0 * zoom;
    let z_depth = (z2 + cam_dist).max(20.0);
    let fov = 420.0;
    let scale = (fov / z_depth) * (world_scale / 120.0);

    let sx = center.x + x2 * scale;
    let sy = center.y + y2 * scale;

    Projected3D {
        screen: pos2(sx, sy),
        depth: z_depth,
        scale,
    }
}

enum DrawItem<'a> {
    Star {
        screen: Pos2,
        depth: f32,
        radius: f32,
        color: Color32,
        flare: bool,
    },
    RingSegment {
        p1: Pos2,
        p2: Pos2,
        depth: f32,
        color: Color32,
        glow_color: Color32,
        width: f32,
    },
    SynapseRay {
        p1: Pos2,
        p2: Pos2,
        depth: f32,
        color: Color32,
        glow_color: Color32,
        width: f32,
        photon: Option<Pos2>,
        photon_color: Color32,
    },
    CoreHalo {
        screen: Pos2,
        depth: f32,
        radius: f32,
        color: Color32,
    },
    CoreBody {
        screen: Pos2,
        depth: f32,
        radius: f32,
        heartbeat: f32,
    },
    NodeSphere {
        orbit_type: u8,
        idx: usize,
        screen: Pos2,
        depth: f32,
        radius: f32,
        node: &'a AiMemoryNode,
        is_focus: bool,
    },
}

impl<'a> DrawItem<'a> {
    fn depth(&self) -> f32 {
        match self {
            DrawItem::Star { depth, .. } => *depth,
            DrawItem::RingSegment { depth, .. } => *depth,
            DrawItem::SynapseRay { depth, .. } => *depth,
            DrawItem::CoreHalo { depth, .. } => *depth,
            DrawItem::CoreBody { depth, .. } => *depth,
            DrawItem::NodeSphere { depth, .. } => *depth,
        }
    }
}

pub fn view(
    ui: &mut Ui,
    state: &mut NeuralMemoryState,
    nodes: &[AiMemoryNode],
    _status: &Status,
) {
    let nodes = if nodes.is_empty() {
        placeholder_nodes()
    } else {
        nodes.to_vec()
    };

    let (response, painter) = ui.allocate_painter(
        ui.available_size(),
        egui::Sense::click_and_drag(),
    );
    let bounds = response.rect;

    if !ui.is_rect_visible(bounds) || bounds.width() < 10.0 || bounds.height() < 10.0 {
        return;
    }

    let time = ui.input(|i| i.time) as f32;
    let dt = ui.input(|i| i.stable_dt).min(0.05) as f32;

    // ── 1. Interactive 3D Camera Controls (Fluid, Zero-Snapping) ──────────────
    if response.dragged() {
        let delta = response.drag_delta();
        state.yaw += delta.x * 0.006;
        state.pitch = (state.pitch + delta.y * 0.006).clamp(-1.45, 1.45);
    } else if state.auto_orbit {
        // Continuous celestial rotation without any snap jumps
        state.yaw += dt * 0.12;
    }

    if response.hovered() {
        let scroll = ui.input(|i| i.raw_scroll_delta.y);
        if scroll.abs() > 0.01 {
            state.zoom = (state.zoom - scroll * 0.0015).clamp(0.40, 2.8);
        }
    }

    let current_yaw = state.yaw;
    let current_pitch = state.pitch;
    let current_zoom = state.zoom;

    let center = bounds.center();
    let world_scale = (bounds.width().min(bounds.height()) * 0.46).max(50.0);

    // ── 2. Monochrome Obsidian & Subtle White Radial Vignette ────────────────
    let nebula_r = world_scale * 1.8;
    for (step_r, col) in [
        (nebula_r * 1.00, Color32::from_rgba_unmultiplied(255, 255, 255, 3)),
        (nebula_r * 0.72, Color32::from_rgba_unmultiplied(255, 255, 255, 5)),
        (nebula_r * 0.45, Color32::from_rgba_unmultiplied(255, 255, 255, 8)),
        (nebula_r * 0.24, Color32::from_rgba_unmultiplied(255, 255, 255, 14)),
    ] {
        painter.circle_filled(center, step_r, col);
    }

    let mut draw_queue: Vec<DrawItem> = Vec::with_capacity(400);

    // ── 3. Volumetric 3D Monochrome Starfield ────────────────────────────────
    for i in 0u32..120 {
        let u = (i as f32 * 137.508).fract();
        let v = (i as f32 * 53.371).fract();
        let theta = u * std::f32::consts::TAU;
        let phi = (v * 2.0 - 1.0).asin();
        let dist = 210.0 + (i as f32 * 19.17).fract() * 150.0;

        let star_3d = Vec3::new(
            dist * phi.cos() * theta.cos(),
            dist * phi.sin(),
            dist * phi.cos() * theta.sin(),
        );

        let p = project_3d(star_3d, current_yaw * 0.4, current_pitch * 0.4, current_zoom, center, world_scale);
        if bounds.contains(p.screen) {
            let twinkle = ((time * 2.0 + i as f32 * 1.7).sin() * 0.5 + 0.5) * 0.4 + 0.6;
            let alpha = ((45.0 + v * 120.0) * p.scale.clamp(0.5, 1.4) * twinkle).clamp(15.0, 210.0) as u8;
            let r = (0.7 + v * 0.9) * p.scale.clamp(0.6, 1.4);
            let has_flare = i % 15 == 0;
            let col = Color32::from_rgba_unmultiplied(244, 244, 245, alpha);

            draw_queue.push(DrawItem::Star {
                screen: p.screen,
                depth: p.depth,
                radius: r,
                color: col,
                flare: has_flare,
            });
        }
    }

    // ── 4. 3D Tilted Monochrome Orbital Rings ────────────────────────────────
    let ring_configs = [
        (48.0_f32, 0.05_f32, 0.0_f32, C::with_alpha(C::WHITE, 140), C::with_alpha(C::WHITE, 22)),
        (88.0_f32, 0.35_f32, 0.2_f32, C::with_alpha(C::TEXT_MUTED, 110), C::with_alpha(C::TEXT_MUTED, 18)),
        (132.0_f32, -0.45_f32, -0.3_f32, C::with_alpha(C::TEXT_DIM, 95), C::with_alpha(C::TEXT_DIM, 14)),
    ];

    for &(radius, tilt_x, tilt_z, ring_col, glow_col) in &ring_configs {
        const SEGMENTS: usize = 40;
        let mut prev_pt: Option<Projected3D> = None;
        let mut first_pt: Option<Projected3D> = None;

        for s in 0..=SEGMENTS {
            let angle = (s as f32 / SEGMENTS as f32) * std::f32::consts::TAU;
            let raw_x = radius * angle.cos();
            let raw_z = radius * angle.sin();
            let raw_y = raw_x * tilt_x + raw_z * tilt_z;

            let pt = project_3d(
                Vec3::new(raw_x, raw_y, raw_z),
                current_yaw,
                current_pitch,
                current_zoom,
                center,
                world_scale,
            );

            if let Some(prev) = prev_pt {
                let mid_depth = (prev.depth + pt.depth) * 0.5;
                draw_queue.push(DrawItem::RingSegment {
                    p1: prev.screen,
                    p2: pt.screen,
                    depth: mid_depth,
                    color: ring_col,
                    glow_color: glow_col,
                    width: (1.0 * pt.scale).clamp(0.6, 1.8),
                });
            } else {
                first_pt = Some(Projected3D { screen: pt.screen, depth: pt.depth, scale: pt.scale });
            }
            prev_pt = Some(pt);
        }

        if let (Some(prev), Some(first)) = (prev_pt, first_pt) {
            let mid_depth = (prev.depth + first.depth) * 0.5;
            draw_queue.push(DrawItem::RingSegment {
                p1: prev.screen,
                p2: first.screen,
                depth: mid_depth,
                color: ring_col,
                glow_color: glow_col,
                width: 1.0,
            });
        }
    }

    // ── 5. Central Monochrome Stark-White AI Core ────────────────────────────
    let core_pt = project_3d(Vec3::new(0.0, 0.0, 0.0), current_yaw, current_pitch, current_zoom, center, world_scale);
    let heartbeat = (time * 2.4).sin() * 0.5 + 0.5;

    let halo_layers = [
        (95.0_f32, Color32::from_rgba_unmultiplied(250, 250, 250, 8)),
        (68.0_f32, Color32::from_rgba_unmultiplied(250, 250, 250, 16)),
        (46.0_f32, Color32::from_rgba_unmultiplied(250, 250, 250, 28)),
        (30.0_f32, Color32::from_rgba_unmultiplied(250, 250, 250, 55)),
        (20.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 110)),
    ];

    for (hr, col) in halo_layers {
        let r = (hr + heartbeat * 3.0) * core_pt.scale * 0.85;
        draw_queue.push(DrawItem::CoreHalo {
            screen: core_pt.screen,
            depth: core_pt.depth + 1.0,
            radius: r,
            color: col,
        });
    }

    draw_queue.push(DrawItem::CoreBody {
        screen: core_pt.screen,
        depth: core_pt.depth,
        radius: (15.0 + heartbeat * 1.5) * core_pt.scale * 0.85,
        heartbeat,
    });

    // ── 6. 3D Orbital Node Coordinates & Synaptic Laser Rays ──────────────────
    let signals: Vec<&AiMemoryNode> = nodes.iter().filter(|n| n.orbit == MemoryOrbit::Signal).collect();
    let regimes: Vec<&AiMemoryNode> = nodes.iter().filter(|n| n.orbit == MemoryOrbit::Regime).collect();
    let assets: Vec<&AiMemoryNode> = nodes.iter().filter(|n| n.orbit == MemoryOrbit::Asset).collect();

    let active_focus = state.selected.or(state.hovered);

    // Compute 3D node positions
    let compute_3d_nodes = |items: &[&AiMemoryNode], radius: f32, tilt_x: f32, tilt_z: f32, speed: f32, orbit_offset: f32| -> Vec<(Vec3, Projected3D)> {
        let count = items.len().max(1);
        (0..items.len()).map(|i| {
            let angle = (i as f32 / count as f32) * std::f32::consts::TAU + time * speed + orbit_offset;
            let rx = radius * angle.cos();
            let rz = radius * angle.sin();
            let ry = rx * tilt_x + rz * tilt_z + (time * 1.8 + i as f32).sin() * 4.0;
            let v3 = Vec3::new(rx, ry, rz);
            let proj = project_3d(v3, current_yaw, current_pitch, current_zoom, center, world_scale);
            (v3, proj)
        }).collect()
    };

    let sig_3d = compute_3d_nodes(&signals, 48.0, 0.05, 0.0, 0.16, 0.2);
    let reg_3d = compute_3d_nodes(&regimes, 88.0, 0.35, 0.2, -0.08, 1.1);
    let ast_3d = compute_3d_nodes(&assets, 132.0, -0.45, -0.3, 0.04, 2.3);

    // Synaptic Laser Beams from Core to Signals
    for (i, node) in signals.iter().enumerate() {
        let (_, p) = &sig_3d[i];
        let col = verdict_color(node.verdict);
        let photon_t = (time * 2.2 + i as f32 * 0.35).fract();
        let photon_pos = pos2(
            core_pt.screen.x + (p.screen.x - core_pt.screen.x) * photon_t,
            core_pt.screen.y + (p.screen.y - core_pt.screen.y) * photon_t,
        );

        let mid_depth = (core_pt.depth + p.depth) * 0.5;
        let alpha = (90.0 + node.confidence as f32 * 140.0).min(240.0) as u8;

        draw_queue.push(DrawItem::SynapseRay {
            p1: core_pt.screen,
            p2: p.screen,
            depth: mid_depth,
            color: C::with_alpha(col, alpha),
            glow_color: C::with_alpha(col, 50),
            width: (1.6 * p.scale).clamp(0.8, 3.0),
            photon: Some(photon_pos),
            photon_color: col,
        });
    }

    // Secondary Synaptic Arcs between Regimes and Assets
    for (ri, _rnode) in regimes.iter().enumerate() {
        for (ai, anode) in assets.iter().enumerate() {
            if (ri * 2 + ai) % 3 != 0 {
                continue;
            }
            let (_, rp) = &reg_3d[ri];
            let (_, ap) = &ast_3d[ai];
            let m_col = market_color(anode.market);
            let mid_depth = (rp.depth + ap.depth) * 0.5;

            draw_queue.push(DrawItem::SynapseRay {
                p1: rp.screen,
                p2: ap.screen,
                depth: mid_depth,
                color: C::with_alpha(m_col, 90),
                glow_color: C::with_alpha(m_col, 30),
                width: 0.9 * ap.scale,
                photon: None,
                photon_color: m_col,
            });
        }
    }

    // Add 3D Node Spheres to queue
    for (i, node) in signals.iter().enumerate() {
        let (_, p) = &sig_3d[i];
        let is_focus = active_focus == Some((0, i));
        draw_queue.push(DrawItem::NodeSphere {
            orbit_type: 0,
            idx: i,
            screen: p.screen,
            depth: p.depth,
            radius: (7.2 * p.scale).clamp(3.5, 22.0),
            node,
            is_focus,
        });
    }

    for (i, node) in regimes.iter().enumerate() {
        let (_, p) = &reg_3d[i];
        let is_focus = active_focus == Some((1, i));
        draw_queue.push(DrawItem::NodeSphere {
            orbit_type: 1,
            idx: i,
            screen: p.screen,
            depth: p.depth,
            radius: (8.2 * p.scale).clamp(4.0, 24.0),
            node,
            is_focus,
        });
    }

    for (i, node) in assets.iter().enumerate() {
        let (_, p) = &ast_3d[i];
        let is_focus = active_focus == Some((2, i));
        draw_queue.push(DrawItem::NodeSphere {
            orbit_type: 2,
            idx: i,
            screen: p.screen,
            depth: p.depth,
            radius: (6.5 * p.scale).clamp(3.2, 19.0),
            node,
            is_focus,
        });
    }

    // ── 7. Depth Sorting (Painter's Algorithm: Furthest to Nearest) ────────────
    draw_queue.sort_by(|a, b| b.depth().partial_cmp(&a.depth()).unwrap_or(std::cmp::Ordering::Equal));

    // ── 8. Render All 3D Geometry with Multi-Pass Glowing Bloom ───────────────
    let mut hit_test_nodes: Vec<(u8, usize, Pos2, f32, &AiMemoryNode)> = Vec::new();

    for item in draw_queue {
        match item {
            DrawItem::Star { screen, radius, color, flare, .. } => {
                // Star glow halo
                painter.circle_filled(screen, radius * 2.5, C::with_alpha(color, 45));
                painter.circle_filled(screen, radius, color);
                if flare {
                    let flare_len = radius * 4.5;
                    let fl_col = C::with_alpha(color, 120);
                    painter.line_segment([pos2(screen.x - flare_len, screen.y), pos2(screen.x + flare_len, screen.y)], Stroke::new(1.0_f32, fl_col));
                    painter.line_segment([pos2(screen.x, screen.y - flare_len), pos2(screen.x, screen.y + flare_len)], Stroke::new(1.0_f32, fl_col));
                }
            }
            DrawItem::RingSegment { p1, p2, color, glow_color, width, .. } => {
                // Pass 1: Outer wide glow stroke
                painter.line_segment([p1, p2], Stroke::new(width * 3.5, glow_color));
                // Pass 2: Crisp neon core
                painter.line_segment([p1, p2], Stroke::new(width, color));
            }
            DrawItem::SynapseRay { p1, p2, color, glow_color, width, photon, photon_color, .. } => {
                // Pass 1: Luminous laser beam halo
                painter.line_segment([p1, p2], Stroke::new(width * 3.8, glow_color));
                // Pass 2: Sharp core laser line
                painter.line_segment([p1, p2], Stroke::new(width, color));

                // Streaming High-Energy Photons
                if let Some(pos) = photon {
                    // Outer photon halo
                    painter.circle_filled(pos, width * 4.0, C::with_alpha(photon_color, 70));
                    // Mid energetic shell
                    painter.circle_filled(pos, width * 2.0, photon_color);
                    // Incandescent core spark
                    painter.circle_filled(pos, width * 0.9, Color32::WHITE);
                }
            }
            DrawItem::CoreHalo { screen, radius, color, .. } => {
                painter.circle_filled(screen, radius, color);
            }
            DrawItem::CoreBody { screen, radius, heartbeat, .. } => {
                // Monochrome Architectural Singularity Core
                painter.circle_stroke(
                    screen,
                    radius * (1.28 + heartbeat * 0.08),
                    Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(244, 244, 245, 70)),
                );
                painter.circle_filled(screen, radius * 1.02, Color32::from_rgba_unmultiplied(36, 36, 41, 255));
                painter.circle_filled(screen, radius * 0.82, Color32::from_rgba_unmultiplied(161, 161, 170, 255));
                painter.circle_filled(screen, radius * 0.56, Color32::from_rgba_unmultiplied(244, 244, 245, 255));
                painter.circle_filled(screen, radius * 0.30, Color32::WHITE);
                painter.circle_stroke(
                    screen,
                    radius * 1.02,
                    Stroke::new(1.6_f32, Color32::from_rgba_unmultiplied(250, 250, 250, 220)),
                );
            }
            DrawItem::NodeSphere { orbit_type, idx, screen, radius, node, is_focus, .. } => {
                let col = verdict_color(node.verdict);
                let m_col = market_color(node.market);

                // Multi-stage Atmosphere Glow Aura
                painter.circle_filled(
                    screen,
                    radius * 3.6,
                    C::with_alpha(col, if is_focus { 90 } else { 40 }),
                );
                painter.circle_filled(
                    screen,
                    radius * 2.0,
                    C::with_alpha(col, if is_focus { 140 } else { 75 }),
                );

                // Market Orbit Ring with Neon Stroke
                painter.circle_stroke(
                    screen,
                    radius + 3.0,
                    Stroke::new(
                        if node.active { 2.2_f32 } else { 1.2_f32 },
                        if node.active { m_col } else { C::with_alpha(m_col, 100) },
                    ),
                );

                // 3D Sphere Body with Shading
                painter.circle_filled(screen, radius, col);
                // Specular Light Spot
                let spec_pos = pos2(screen.x - radius * 0.30, screen.y - radius * 0.30);
                painter.circle_filled(spec_pos, radius * 0.40, Color32::from_rgba_unmultiplied(255, 255, 255, 210));

                // Focus Halo when Selected / Hovered
                if is_focus {
                    painter.circle_stroke(
                        screen,
                        radius * 3.0,
                        Stroke::new(2.2_f32, C::CYAN),
                    );
                }

                // Billboard 3D Holographic Text Label
                let label_pos = pos2(screen.x + radius + 5.0, screen.y - 4.0);
                let label_font = FontId::new((9.0 * (radius / 7.0).clamp(0.75, 1.35)).clamp(8.0, 12.0), FontFamily::Monospace);
                
                // Dark pill backdrop behind text
                let text_rect = Rect::from_min_size(
                    pos2(label_pos.x - 2.0, label_pos.y - 1.0),
                    vec2(node.label.len() as f32 * 6.8 + 8.0, 14.0),
                );
                painter.rect_filled(text_rect, Rounding::same(2.0), C::with_alpha(C::BG_CARD, 210));
                painter.rect_stroke(text_rect, Rounding::same(2.0), Stroke::new(1.0_f32, C::with_alpha(col, if is_focus { 220 } else { 110 })));

                painter.text(
                    pos2(text_rect.min.x + 4.0, text_rect.center().y),
                    Align2::LEFT_CENTER,
                    &node.label,
                    label_font,
                    if is_focus { Color32::WHITE } else { C::TEXT },
                );

                hit_test_nodes.push((orbit_type, idx, screen, radius * 2.0, node));
            }
        }
    }

    // ── 9. Interactive 3D Hit-Testing (Hover & Click) ─────────────────────────
    let mut current_hover = None;
    if let Some(cpos) = response.hover_pos() {
        for &(orbit_type, idx, pos, hit_r, _) in hit_test_nodes.iter().rev() {
            let dx = cpos.x - pos.x;
            let dy = cpos.y - pos.y;
            if dx * dx + dy * dy <= (hit_r + 8.0) * (hit_r + 8.0) {
                current_hover = Some((orbit_type, idx));
                break;
            }
        }
    }
    state.hovered = current_hover;

    if response.clicked() {
        state.selected = state.hovered;
    }

    // ── 10. Holographic Tooltip Card on Hover / Selection ─────────────────────
    if let Some((orbit_type, idx)) = active_focus {
        let hovered_info = hit_test_nodes.iter().find(|(ot, i, _, _, _)| *ot == orbit_type && *i == idx);
        if let Some(&(_, _, screen, _, node)) = hovered_info {
            let col = verdict_color(node.verdict);
            let tip_w = 220.0;
            let tip_h = 76.0;

            let mut bx = screen.x + 20.0;
            if bx + tip_w > bounds.max.x - 8.0 {
                bx = screen.x - tip_w - 20.0;
            }
            let mut by = screen.y - 12.0;
            if by + tip_h > bounds.max.y - 8.0 {
                by = bounds.max.y - tip_h - 8.0;
            }

            let card_rect = Rect::from_min_size(pos2(bx, by), vec2(tip_w, tip_h));
            painter.rect(
                card_rect,
                Rounding::same(4.0),
                C::with_alpha(C::BG_ELEVATED, 250),
                Stroke::new(1.4_f32, col),
            );

            // Left accent bar
            painter.rect_filled(
                Rect::from_min_size(card_rect.min, vec2(3.5, tip_h)),
                Rounding::same(2.0),
                col,
            );

            // Header line
            painter.text(
                pos2(bx + 10.0, by + 8.0),
                Align2::LEFT_TOP,
                format!("3D NEURAL CORE :: {}", node.label),
                FontId::new(9.5, FontFamily::Monospace),
                C::TEXT,
            );

            // Verdict & Confidence
            painter.text(
                pos2(bx + 10.0, by + 28.0),
                Align2::LEFT_TOP,
                format!("VERDICT: {:?} · CONF: {:.0}%", node.verdict, node.confidence * 100.0),
                FontId::new(9.0, FontFamily::Monospace),
                col,
            );

            // Detail line
            painter.text(
                pos2(bx + 10.0, by + 48.0),
                Align2::LEFT_TOP,
                &node.detail,
                FontId::new(8.5, FontFamily::Monospace),
                C::TEXT_MUTED,
            );
        }
    }

    // ── 11. Interactive HUD Floating Controls in Top-Right ────────────────────
    let ctrl_rect = Rect::from_min_size(
        pos2(bounds.max.x - 180.0, bounds.min.y + 8.0),
        vec2(172.0, 22.0),
    );
    painter.rect_filled(ctrl_rect, Rounding::same(3.0), C::with_alpha(C::BG_CARD, 230));
    painter.rect_stroke(ctrl_rect, Rounding::same(3.0), Stroke::new(1.0_f32, C::BORDER));

    let orbit_text = if state.auto_orbit { "● ORBIT: ON" } else { "○ ORBIT: OFF" };
    painter.text(
        pos2(ctrl_rect.min.x + 8.0, ctrl_rect.center().y),
        Align2::LEFT_CENTER,
        orbit_text,
        FontId::new(8.5, FontFamily::Monospace),
        if state.auto_orbit { C::CYAN } else { C::AMBER },
    );

    painter.text(
        pos2(ctrl_rect.max.x - 8.0, ctrl_rect.center().y),
        Align2::RIGHT_CENTER,
        "DRAG/ZOOM",
        FontId::new(8.0, FontFamily::Monospace),
        C::TEXT_DIM,
    );

    // Toggle orbit if clicking top-right button
    if response.clicked() {
        if let Some(mpos) = response.interact_pointer_pos() {
            if ctrl_rect.contains(mpos) {
                state.auto_orbit = !state.auto_orbit;
            }
        }
    }
}

fn verdict_color(v: NodeVerdict) -> Color32 {
    match v {
        NodeVerdict::Go => C::GREEN,
        NodeVerdict::Watch => C::AMBER,
        NodeVerdict::NoGo => C::RED,
        NodeVerdict::Neutral => C::BLUE,
    }
}

fn market_color(m: trading_core::Market) -> Color32 {
    match m {
        trading_core::Market::Crypto => C::ORANGE,
        trading_core::Market::Forex => C::BLUE,
        trading_core::Market::Gold => C::AMBER,
        trading_core::Market::Synthetic => C::CYAN,
    }
}

fn placeholder_nodes() -> Vec<AiMemoryNode> {
    use trading_core::{Market, MemoryOrbit, NodeVerdict};
    vec![
        AiMemoryNode {
            id: "EMA_CROSS".into(),
            label: "EMA_CROSS".into(),
            orbit: MemoryOrbit::Signal,
            verdict: NodeVerdict::Go,
            confidence: 0.84,
            market: Market::Forex,
            active: true,
            detail: "Bullish EMA 9/21 cross confirmed on 5m".into(),
        },
        AiMemoryNode {
            id: "RSI_OB".into(),
            label: "RSI_OB".into(),
            orbit: MemoryOrbit::Signal,
            verdict: NodeVerdict::Watch,
            confidence: 0.58,
            market: Market::Forex,
            active: false,
            detail: "RSI 62 neutral-bullish buffer".into(),
        },
        AiMemoryNode {
            id: "BB_BREAK".into(),
            label: "BB_BREAK".into(),
            orbit: MemoryOrbit::Signal,
            verdict: NodeVerdict::Go,
            confidence: 0.76,
            market: Market::Forex,
            active: true,
            detail: "Upper Bollinger expansion trigger".into(),
        },
        AiMemoryNode {
            id: "BULL".into(),
            label: "BULL".into(),
            orbit: MemoryOrbit::Regime,
            verdict: NodeVerdict::Go,
            confidence: 0.72,
            market: Market::Forex,
            active: true,
            detail: "Trend regime Bullish 4H momentum".into(),
        },
        AiMemoryNode {
            id: "EUR_USD".into(),
            label: "EUR_USD".into(),
            orbit: MemoryOrbit::Asset,
            verdict: NodeVerdict::Go,
            confidence: 0.82,
            market: Market::Forex,
            active: true,
            detail: "Primary active FX pair".into(),
        },
        AiMemoryNode {
            id: "XAU_USD".into(),
            label: "XAU_USD".into(),
            orbit: MemoryOrbit::Asset,
            verdict: NodeVerdict::Neutral,
            confidence: 0.65,
            market: Market::Gold,
            active: false,
            detail: "Gold market consolidation".into(),
        },
    ]
}
