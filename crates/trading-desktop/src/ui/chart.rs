//! Center-top: candlestick price chart (+ order book on the right).
//! Interactive viewport: wheel = zoom, left-drag = pan through history.

use iced::event;
use iced::mouse::{self, ScrollDelta};
use iced::widget::canvas::{self, Canvas, Fill, Frame, Geometry, Path, Stroke, Text};
use iced::{Element, Length, Point, Rectangle, Renderer, Theme};

use trading_core::Candle;

use super::components::panel;
use crate::theme::C;

#[derive(Default)]
pub struct ChartState {
    /// Number of candles scrolled back from the right (latest) edge.
    first: usize,
    /// Number of candles visible in the window.
    shown: usize,
    /// Active drag: (cursor x at press, `first` at press).
    drag: Option<(f32, f32)>,
}

impl ChartState {
    fn clamp(&mut self, total: usize) {
        self.first = self.first.min(total.saturating_sub(1));
        self.shown = self.shown.clamp(10, total.max(10));
        let max_first = total.saturating_sub(self.shown);
        self.first = self.first.min(max_first);
    }
}

pub fn view<'a, M: 'a + 'static>(candles: &[Candle], symbol: &str, tf: &str) -> Element<'a, M> {
    let mut candles = candles.to_vec();
    let real = candles.len() >= 2;
    if !real {
        candles = placeholder_candles();
    }
    let program = ChartView {
        candles,
        symbol: symbol.to_string(),
        real,
    };
    let right = Some(super::components::mono_color(
        format!("{symbol} · {tf} · scroll=zoom · drag=pan"),
        crate::theme::C::cyan(),
    ));
    panel(
        "PRICE / LIQUIDITY",
        right,
        Canvas::new(program).width(Length::Fill).height(Length::Fill),
    )
}

pub struct ChartView {
    candles: Vec<Candle>,
    symbol: String,
    real: bool,
}

impl<Message: 'static> canvas::Program<Message> for ChartView {
    type State = ChartState;

    fn update(
        &self,
        state: &mut ChartState,
        event: canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (event::Status, Option<Message>) {
        let total = self.candles.len().max(1);
        match event {
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let lines = match delta {
                    ScrollDelta::Lines { y, .. } => y,
                    ScrollDelta::Pixels { y, .. } => y / 50.0,
                };
                if lines > 0.0 {
                    state.shown = state.shown.saturating_sub((state.shown / 8).max(1)).max(10);
                } else {
                    state.shown += (state.shown / 8).max(1);
                }
                state.clamp(total);
                (event::Status::Captured, None)
            }
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let mouse::Cursor::Available(p) = cursor {
                    state.drag = Some((p.x, state.first as f32));
                } else {
                    state.drag = None;
                }
                (event::Status::Captured, None)
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.drag = None;
                (event::Status::Captured, None)
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { position }) => {
                if let Some((anchor_x, anchor_first)) = state.drag {
                    let span = (bounds.width - 20.0).max(1.0);
                    let per_px = total as f32 / span as f32;
                    let nf = anchor_first + (anchor_x - position.x) * per_px;
                    let first = nf.max(0.0) as usize;
                    if first != state.first {
                        state.first = first;
                        state.clamp(total);
                        return (event::Status::Captured, None);
                    }
                }
                (event::Status::Ignored, None)
            }
            _ => (event::Status::Ignored, None),
        }
    }

    fn draw(
        &self,
        state: &ChartState,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());

        let candles = &self.candles;
        let total = candles.len().max(1);
        let mut st = ChartState {
            first: state.first,
            shown: state.shown,
            drag: state.drag,
        };
        // Show the full long history by default; the user can scroll/zoom to inspect.
        if st.shown == 0 {
            st.shown = candles.len().max(10);
        }
        // Show a chunk of the full history by default (zoomable with wheel).
        if st.shown == 0 {
            st.shown = candles.len().min(60).max(10);
        }
        st.clamp(total);
        let vis = &candles[st.first..(st.first + st.shown).min(total)];

        let len = vis.len().max(1) as f32;
        let pad_x = 10.0;
        let top_pad = 22.0;
        let bottom_pad = 18.0;
        let plot_w = (bounds.width - pad_x * 2.0).max(1.0);
        let plot_h = (bounds.height - top_pad - bottom_pad).max(1.0);

        let mut hi = f32::MIN;
        let mut lo = f32::MAX;
        for c in vis {
            hi = hi.max(c.high as f32);
            lo = lo.min(c.low as f32);
        }
        // Guarantee a visible spread so a flat/closed market still shows
        // variance instead of collapsing to a single flat line.
        if lo >= hi {
            let mid = if hi.is_finite() { hi } else { 1.0 };
            let spread = (mid * 0.002).max(1e-3);
            lo = mid - spread;
            hi = mid + spread;
        } else {
            let pad = (hi - lo) * 0.15;
            hi += pad;
            lo -= pad;
        }
        let range = (hi - lo).max(1e-9);

        let x_of = |i: usize| pad_x + (i as f32 + 0.5) / len * plot_w;
        let y_of = |v: f32| top_pad + (hi - v) / range * plot_h;

        for f in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let price = hi - range * f;
            frame.fill_text(Text {
                content: format!("{:.4}", price),
                position: Point::new(bounds.width - 60.0, y_of(price) - 7.0),
                color: C::dim(),
                size: iced::Pixels(9.0),
                ..Default::default()
            });
        }

        let tfmt = |ts: i64| {
            chrono::DateTime::<chrono::Utc>::from_timestamp(ts, 0)
                .map(|dt| dt.format("%H:%M").to_string())
                .unwrap_or_default()
        };
        for i in [0, (vis.len().max(1) - 1) / 2, vis.len() - 1] {
            frame.fill_text(Text {
                content: tfmt(vis[i].time),
                position: Point::new(x_of(i) - 20.0, bounds.height - 10.0),
                color: C::dim(),
                size: iced::Pixels(9.0),
                ..Default::default()
            });
        }

        let body_w = (plot_w / len * 0.7).clamp(3.0, 30.0);
        for (i, c) in vis.iter().enumerate() {
            let o = c.open as f32;
            let cl = c.close as f32;
            let up = cl >= o;
            let color = if up { C::green() } else { C::red() };
            let x = x_of(i);

            let wick = Path::line(Point::new(x, y_of(c.high as f32)), Point::new(x, y_of(c.low as f32)));
            frame.stroke(
                &wick,
                Stroke {
                    style: color.into(),
                    width: 1.0,
                    ..Default::default()
                },
            );

            let body_top = y_of(o.max(cl));
            let body_h = (y_of(o.min(cl)) - body_top).max(2.5);
            let rect = Path::rectangle(
                Point::new(x - body_w / 2.0, body_top),
                iced::Size::new(body_w, body_h),
            );
            frame.fill(
                &rect,
                Fill {
                    style: color.into(),
                    ..Default::default()
                },
            );
        }

        let last = vis.last().unwrap();
        let last_color = if last.close >= last.open { C::green() } else { C::red() };
        let last_y = y_of(last.close as f32);
        let line = Path::line(Point::new(pad_x, last_y), Point::new(bounds.width - pad_x, last_y));
        frame.stroke(
            &line,
            Stroke {
                style: last_color.into(),
                width: 1.0,
                ..Default::default()
            },
        );

        frame.fill_text(Text {
            content: format!(
                "{}  HI {:.4}  LO {:.4}  {:+.2}%",
                self.symbol,
                hi,
                lo,
                (last.close as f32 - vis[0].open as f32) / (vis[0].open as f32) * 100.0
            ),
            position: Point::new(4.0, 4.0),
            color: C::text(),
            size: iced::Pixels(10.0),
            ..Default::default()
        });

        if !self.real {
            frame.fill_text(Text {
                content: "▪ WAITING FOR LIVE DATA — INITIALIZING FEED…".to_string(),
                position: Point::new(4.0, bounds.height - 30.0),
                color: C::amber(),
                size: iced::Pixels(10.0),
                ..Default::default()
            });
        }

        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        _state: &Self::State,
        _bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        let _ = &self.candles;
        mouse::Interaction::Crosshair
    }
}

fn placeholder_candles() -> Vec<Candle> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut candles = Vec::new();
    let mut price = 83.0_f64;
    for i in 0..120 {
        let ts = now - (120 - i as u64) * 300;
        let drift = (i as f64 * 0.15).sin() * 0.4;
        let open = price;
        let close = (price + drift).max(1.0);
        let high = open.max(close) + 0.2;
        let low = open.min(close) - 0.2;
        candles.push(Candle { time: ts as i64, open, high, low, close, volume: 100.0 + (i as f64 * 7.3).sin() * 40.0 });
        price = close;
    }
    candles
}