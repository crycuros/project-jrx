//! Shared UI helpers: terminal panels, mono labels, gauges.

use iced::widget::{column, container, progress_bar, row, text, Container};
use iced::{Element, Length};

use crate::theme::{C, MONO, MONO_BOLD};

pub const PAD: u16 = 8;
pub const SPACE: u16 = 6;

/// Dark bordered panel container.
pub fn panel<'a, M: 'a>(title: impl Into<String>, right: Option<Element<'a, M>>, body: impl Into<Element<'a, M>>) -> Element<'a, M> {
    let header = row![
        container(title_elem(title)).width(Length::Fill),
        right.unwrap_or_else(|| text("").into()),
    ]
    .spacing(SPACE);

    container(
        column![header, iced::widget::horizontal_rule(1), body.into()].spacing(4),
    )
    .padding(PAD)
    .width(Length::Fill)
    .height(Length::Fill)
    .style(panel_style)
    .into()
}

pub fn panel_style(theme: &iced::Theme) -> iced::widget::container::Style {
    let _ = theme;
    iced::widget::container::Style {
        background: Some(C::bg_panel().into()),
        border: iced::Border {
            color: C::border(),
            width: 1.0,
            radius: iced::border::Radius::from(2.0),
        },
        ..Default::default()
    }
}

/// Micro uppercase terminal label.
pub fn micro<'a, M: 'a>(s: impl Into<String>) -> Element<'a, M> {
    text(s.into().to_uppercase()).font(MONO).size(9).color(C::dim()).into()
}

/// Mono body label.
pub fn mono<'a, M: 'a>(s: impl Into<String>) -> Element<'a, M> {
    text(s.into()).font(MONO).size(11).color(C::text()).into()
}

pub fn mono_color<'a, M: 'a>(s: impl Into<String>, c: iced::Color) -> Element<'a, M> {
    text(s.into()).font(MONO).size(10).color(c).into()
}

/// Mono label that fills available width.
pub fn mono_fill<'a, M: 'a>(s: impl Into<String>, c: iced::Color) -> Element<'a, M> {
    container(mono_color(s, c)).width(Length::Fill).into()
}

pub fn title_elem<'a, M: 'a>(s: impl Into<String>) -> Element<'a, M> {
    text(s.into().to_uppercase()).font(MONO_BOLD).size(10).color(C::text2()).into()
}

/// Confidence/accuracy progress bar.
pub fn gauge<'a, M: 'a>(label: impl Into<String>, pct: f32) -> Element<'a, M> {
    row![
        container(micro(label)).width(Length::Fixed(56.0)),
        progress_bar(0.0..=100.0, pct).width(Length::Fill).height(6),
        text(format!("{:>4.0}%", pct)).font(MONO).size(9).color(C::text2()),
    ]
    .spacing(6)
    .align_y(iced::Alignment::Center)
    .into()
}

pub fn hline<'a, M: 'a>() -> Element<'a, M> {
    iced::widget::horizontal_rule(1).into()
}

// Re-export type alias used elsewhere.
#[allow(dead_code)]
pub type Boxed<'a, M> = Container<'a, M>;

#[allow(dead_code)]
pub fn container_w<'a, M: 'a>(e: Element<'a, M>, w: Length) -> Element<'a, M> {
    container(e).width(w).into()
}