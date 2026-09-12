//! Order book depth (asks red up top, bids green below).
//! Built from live broker book data only — never synthesised.

use iced::widget::{column, container, row, text};
use iced::{Color, Element};

use trading_core::OrderBook;

use super::components::panel;
use crate::theme::{C, MONO, MONO_BOLD};

pub fn view<'a, M: 'a>(book: Option<&OrderBook>) -> Element<'a, M> {
    let body: Element<'a, M> = match book {
        Some(b) if b.live && !b.asks.is_empty() && !b.bids.is_empty() => {
            let digits = b.asks[0].price.abs().max(b.bids[0].price.abs());
            let d = price_digits(digits);
            let asks: Vec<(f64, f64)> = b.asks.iter().map(|l| (l.price, l.qty)).collect();
            let bids: Vec<(f64, f64)> = b.bids.iter().map(|l| (l.price, l.qty)).collect();
            column![
                depth_column("ASK", &asks, true, d),
                iced::widget::horizontal_rule(1),
                row![
                    text("SPREAD").font(MONO).size(8).color(C::dim()),
                    text(format!("{:>.*}", d as usize, b.spread))
                        .font(MONO_BOLD)
                        .size(9)
                        .color(C::amber()),
                ]
                .spacing(4),
                iced::widget::horizontal_rule(1),
                depth_column("BID", &bids, false, d),
            ]
            .spacing(2)
            .into()
        }
        Some(b) => {
            let msg = if b.symbol.contains('_') || b.symbol.to_uppercase().starts_with("XAU") {
                "NO PUBLIC BOOK —\nFX / GOLD FEED"
            } else if !b.live {
                "BOOK OFFLINE"
            } else {
                "NO DEPTH"
            };
            centered_hint(msg)
        }
        None => centered_hint("NO LIVE BOOK"),
    };

    panel("ORDER BOOK", None, body)
}

fn centered_hint<'a, M: 'a>(msg: &'a str) -> Element<'a, M> {
    text(msg)
        .font(MONO)
        .size(9)
        .color(C::dim())
        .width(iced::Length::Fill)
        .height(iced::Length::Fill)
        .align_x(iced::alignment::Horizontal::Center)
        .align_y(iced::alignment::Vertical::Center)
        .into()
}

    /// Price decimals scaled to the instrument magnitude.
fn price_digits(p: f64) -> i32 {
    if p < 0.01 {
        8
    } else if p < 1.0 {
        6
    } else if p < 100.0 {
        4
    } else {
        2
    }
}

fn depth_column<'a, 'b, M: 'a>(label: impl Into<String>, rows: &'b [(f64, f64)], is_ask: bool, digits: i32) -> Element<'a, M> {
    let color = if is_ask { C::red() } else { C::green() };
    let mut col = column![
        text(label.into()).font(MONO).size(8).color(color).align_x(iced::alignment::Horizontal::Center),
    ];
    for (price, vol) in rows {
        let frac = (*vol).clamp(0.0, 100.0) / 100.0;
        col = col.push(
            container(
                row![
                    text(format!("{price:>12.*}", digits as usize)).font(MONO).size(9).color(C::text()),
                    text(format!("{:>4.2}", vol)).font(MONO).size(9).color(color),
                ]
                .spacing(4),
            )
            .style(bar_style(frac as f32, color)),
        );
    }
    col.spacing(1).into()
}

fn bar_style(frac: f32, color: Color) -> impl Fn(&iced::Theme) -> iced::widget::container::Style {
    move |_theme: &iced::Theme| -> iced::widget::container::Style {
        iced::widget::container::Style {
            background: Some(iced::Background::Color(Color { a: 0.12 * frac.min(1.0), ..color })),
            border: iced::Border {
                color: color,
                width: 1.0,
                radius: iced::border::Radius::from(1.0),
            },
            ..Default::default()
        }
    }
}