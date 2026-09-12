//! Bottom-left: equity curve (gradient area chart renderer).

use iced::mouse;
use iced::widget::canvas::{self, Canvas, Fill, Frame, Geometry, Path, Stroke, Text};
use iced::{Element, Length, Point, Rectangle, Renderer, Theme};

use super::components::{mono_color, panel};
use crate::theme::C;

pub struct EquityState;

impl Default for EquityState {
    fn default() -> Self {
        Self
    }
}

pub fn view<'a, M: 'a + 'static>(points: &[(f64, f64)], daily_pnl: f64) -> Element<'a, M> {
    let program = EquityProgram {
        points: points.to_vec(),
        daily_pnl,
    };
    let right = Some(mono_color(format!("DAILY {daily_pnl:+.2}"), C::amber()));
    panel(
        "EQUITY CURVE",
        right,
        Canvas::new(program).width(Length::Fill).height(Length::Fill),
    )
}

pub struct EquityProgram {
    points: Vec<(f64, f64)>,
    daily_pnl: f64,
}

impl<Message: 'static> canvas::Program<Message> for EquityProgram {
    type State = EquityState;

    fn draw(&self, _state: &EquityState, renderer: &Renderer, _theme: &Theme, bounds: Rectangle, _cursor: mouse::Cursor) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());

        let pts = if self.points.len() >= 2 {
            self.points.clone()
        } else {
            placeholder_series()
        };

        let pad_x = 6.0;
        let pad_y = 14.0;
        let w = (bounds.width - pad_x * 2.0).max(1.0);
        let h = (bounds.height - pad_y * 2.0).max(1.0);

        let min_y = pts.iter().map(|(_, y)| *y).fold(f64::MAX, f64::min);
        let max_y = pts.iter().map(|(_, y)| *y).fold(f64::MIN, f64::max);
        let range_y = (max_y - min_y).max(1e-9);

        let (_, first) = pts[0];
        let (_, last) = *pts.last().unwrap();
        let up = last >= first;
        let line_color = if up { C::green() } else { C::red() };

        let n = pts.len().saturating_sub(1).max(1) as f32;
        let xy_of = |i: usize, _x: f64, y: f64| Point::new(
            pad_x + (i as f32) / n * w,
            pad_y + ((max_y - y) / range_y) as f32 * h,
        );

        // Area path (fill then stroke).
        let area = Path::new(|b| {
            for (i, (x, y)) in pts.iter().enumerate() {
                if i == 0 {
                    b.move_to(xy_of(i, *x, *y));
                } else {
                    b.line_to(xy_of(i, *x, *y));
                }
            }
            b.line_to(Point::new(pad_x + w, pad_y + h));
            b.line_to(Point::new(pad_x, pad_y + h));
            b.close()
        });
        frame.fill(
            &area,
            Fill {
                style: ColorAlpha(line_color, 0.14).into(),
                ..Default::default()
            },
        );

        let line = Path::new(|b| {
            for (i, (x, y)) in pts.iter().enumerate() {
                if i == 0 {
                    b.move_to(xy_of(i, *x, *y));
                } else {
                    b.line_to(xy_of(i, *x, *y));
                }
            }
        });
        frame.stroke(
            &line,
            Stroke {
                style: line_color.into(),
                width: 1.6,
                ..Default::default()
            },
        );

        frame.fill_text(Text {
            content: format!("${:.0} · Δ {delta:+.2}", last, delta = self.daily_pnl),
            position: Point::new(pad_x + w - 90.0, pad_y),
            color: line_color,
            size: iced::Pixels(10.0),
            ..Default::default()
        });

        vec![frame.into_geometry()]
    }

    fn mouse_interaction(&self, _state: &Self::State, _bounds: Rectangle, _cursor: mouse::Cursor) -> mouse::Interaction {
        mouse::Interaction::default()
    }
}

/// Helper to carry an alpha-modified color into a canvas Style.
pub struct ColorAlpha(pub iced::Color, pub f32);

impl From<ColorAlpha> for iced::widget::canvas::Style {
    fn from(v: ColorAlpha) -> Self {
        iced::Color {
            a: v.0.a * v.1,
            ..v.0
        }
        .into()
    }
}

fn placeholder_series() -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    let mut v = 10_000.0;
    for i in 0..80 {
        v += (i as f64 * 0.21).sin() * 12.0 + 3.0;
        out.push((i as f64, v));
    }
    out
}