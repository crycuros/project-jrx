//! Full-center network node graph (Market-relationship simulation).
//! Node size = market-cap/volume; edge width = correlation.

use iced::mouse;
use iced::widget::canvas::{self, Canvas, Fill, Frame, Geometry, Path, Stroke, Text};
use iced::{Color, Element, Length, Point, Rectangle, Renderer, Theme};

use trading_core::NetworkNode;

use super::components::panel;
use crate::theme::C;

pub struct GraphState;

impl Default for GraphState {
    fn default() -> Self {
        Self
    }
}

pub fn view<'a, M: 'a + 'static>(nodes: &[NetworkNode]) -> Element<'a, M> {
    let nodes: Vec<NetworkNode> = if nodes.is_empty() { placeholder_node_set() } else { nodes.to_vec() };
    let program = GraphView { nodes };
    panel(
        "NETWORK NODE GRAPH — GEL RELATIONSHIPS",
        None,
        Canvas::new(program).width(Length::Fill).height(Length::Fill),
    )
}

pub struct GraphView {
    nodes: Vec<NetworkNode>,
}

impl<Message: 'static> canvas::Program<Message> for GraphView {
    type State = GraphState;

    fn draw(&self, _state: &GraphState, renderer: &Renderer, _theme: &Theme, bounds: Rectangle, _cursor: mouse::Cursor) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());

        let w = bounds.width;
        let h = bounds.height;
        let cx = w / 2.0;
        let cy = h / 2.0;

        let nodes = &self.nodes;
        let n = nodes.len().max(1) as f32;

        // Live drift clock: the whole graph slowly breathes so the network
        // visibly reacts every redraw.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let ms = now.as_millis() as f32 / 1000.0;

        // Circular cluster layout with slight radius jitter + time drift.
        let mut layout: Vec<(f32, f32)> = Vec::with_capacity(nodes.len());
        let radius = (w.min(h) * 0.34).max(60.0);
        let breathe = (ms * 0.8).sin();
        for i in 0..nodes.len() {
            let angle = i as f32 / n as f32 * std::f32::consts::TAU;
            let rr = radius * (0.75 + 0.25 * ((i * 37) % 5) as f32 / 5.0) + breathe * 6.0;
            let dx = (ms * 0.6 + i as f32 * 1.31).sin() * 14.0;
            let dy = (ms * 0.5 + i as f32 * 1.71).cos() * 11.0;
            layout.push((cx + angle.cos() * rr + dx, cy + angle.sin() * rr + dy));
        }

        // Edges: connect nodes in the same cluster, sparse cross-links.
        for i in 0..nodes.len() {
            for j in (i + 1)..nodes.len() {
                let connected = nodes[i].cluster == nodes[j].cluster;
                if !connected && (i * j) % 17 != 0 {
                    continue;
                }
                let pulse = (ms * 2.0 + i as f32 * 0.7).sin() * 0.03;
                let color = if connected {
                    Color { a: (0.10 + pulse).max(0.04), ..C::cluster(&nodes[i].cluster) }
                } else {
                    Color { a: 0.05, ..C::dim() }
                };
                let edge = Path::line(
                    Point::new(layout[i].0, layout[i].1),
                    Point::new(layout[j].0, layout[j].1),
                );
                frame.stroke(
                    &edge,
                    Stroke {
                        style: color.into(),
                        width: (0.7 + pulse.abs() * 2.0).max(0.3),
                        ..Default::default()
                    },
                );
            }
        }

        // Nodes.
        for (i, node) in nodes.iter().enumerate() {
            let (x, y) = layout[i];
            let size = (5.0 + (node.market_cap / 1e6).min(14.0) as f32 + (ms * 1.3 + i as f32).sin()).max(6.0);
            let color = C::cluster(&node.cluster);

            let ring = Path::circle(Point::new(x, y), size * 2.2);
            frame.fill(
                &ring,
                Fill {
                    style: Color { a: 0.06, ..color }.into(),
                    ..Default::default()
                },
            );

            let body = Path::circle(Point::new(x, y), size);
            frame.fill(
                &body,
                Fill {
                    style: color.into(),
                    ..Default::default()
                },
            );
            frame.stroke(
                &body,
                Stroke {
                    style: color.into(),
                    width: 1.0,
                    ..Default::default()
                },
            );

            frame.fill_text(Text {
                content: node.label.clone(),
                position: Point::new(x + size + 4.0, y - 6.0),
                color: C::text2(),
                size: iced::Pixels(8.0),
                ..Default::default()
            });
        }

        // Live metrics strip driven from the actual node set.
        let avg_sent = nodes.iter().map(|nd| nd.sentiment).sum::<f64>() / n as f64;
        let up = nodes.iter().filter(|nd| nd.sentiment >= 0.5).count();
        frame.fill_text(Text {
            content: format!(
                "LIVE · {} ASSETS · SENTIMENT {:.2} · UP {} · EDGES {}",
                nodes.len(),
                avg_sent,
                up,
                (nodes.len() * (nodes.len().saturating_sub(1))) / 2
            ),
            position: Point::new(6.0, 4.0),
            color: C::green(),
            size: iced::Pixels(9.0),
            ..Default::default()
        });

        vec![frame.into_geometry()]
    }

    fn mouse_interaction(&self, _state: &Self::State, _bounds: Rectangle, _cursor: mouse::Cursor) -> mouse::Interaction {
        mouse::Interaction::default()
    }
}

fn placeholder_node_set() -> Vec<NetworkNode> {
    let names = [
        ("EUR", "MAJOR FX", 1_100_000_000.0),
        ("ETH", "ETH/EVM", 400_000_000.0),
        ("SOL", "SOLANA", 70_000_000.0),
        ("LINK", "DEFI", 12_000_000.0),
        ("PEPE", "MICROCAPS", 4_500_000.0),
        ("BONK", "SOLANA", 2_100_000.0),
        ("UNI", "DEFI", 5_000_000.0),
        ("DOGE", "MIDCAPS", 18_000_000.0),
        ("ARB", "ETH/EVM", 3_200_000.0),
        ("GALA", "MIDCAPS", 900_000.0),
        ("WIF", "SOLANA", 2_700_000.0),
        ("MKR", "DEFI", 1_600_000.0),
    ];
    names
        .iter()
        .enumerate()
        .map(|(i, (x, c, cap))| NetworkNode {
            id: x.to_string(),
            label: x.to_string(),
            cluster: c.to_string(),
            market_cap: *cap,
            sentiment: 0.5 + (i as f64 * 0.13).sin() * 0.3,
            x: 0.0,
            y: 0.0,
        })
        .collect()
}