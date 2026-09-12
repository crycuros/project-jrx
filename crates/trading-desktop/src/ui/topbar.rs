//! Top status header: terminal name + global metrics + clock.

use iced::widget::{column, container, row, text};
use iced::{Element, Length};

use trading_core::{Status, SystemHealth};

use super::components::panel_style;
use crate::theme::{C, MONO, MONO_BOLD};

pub fn view<'a, M: 'a>(status: &Status, health: &SystemHealth) -> Element<'a, M> {
    let engine_color = match status.active_engine.as_str() {
        "forex" => C::purple(),
        _ => C::cyan(),
    };
    let fx_label = if health.fx_source.is_empty() || health.fx_source == "oanda" {
        "OANDA".to_string()
    } else {
        health.fx_source.to_uppercase()
    };

    let (status_text, status_color) = if status.kill_switch {
        ("STATUS: HALTED", C::red())
    } else if status.running {
        ("STATUS: LIVE", C::green())
    } else {
        ("STATUS: IDLE", C::amber())
    };

    let (mkt_text, mkt_color) = match health.market_status.as_str() {
        "OPEN" => ("MARKET OPEN", C::green()),
        "CLOSED" => ("MARKET CLOSED", C::red()),
        _ => ("MARKET ?", C::amber()),
    };

    let mode_label = if status.kill_switch {
        "HALTED".to_string()
    } else if status.running {
        format!("{} · {}", status.mode.to_uppercase(), status.active_engine.to_uppercase())
    } else {
        "IDLE".to_string()
    };

    let left = column![
        text("JRX AI 1.0 — AI TRADING TERMINAL")
            .font(MONO_BOLD)
            .size(12)
            .color(C::text()),
        text(format!("RUST TERMINAL · {} · OANDA FOREX + GOLD", mode_label))
            .font(MONO)
            .size(8)
            .color(C::dim()),
    ]
    .spacing(1);

    let center = row![
        micro_color(format!("{fx_label} {}", health.oanda_status), C::text2()),
        micro_color(format!("OLLAMA {}", health.ollama_status), C::text2()),
        micro_color(format!("WIN RATE {:.1}%", status.win_rate), C::text2()),
        micro_color(status_text, status_color),
        micro_color(mkt_text, mkt_color),
        micro_color(format!("ENGINE {}", status.active_engine.to_uppercase()), engine_color),
    ]
    .spacing(14);

    let right = column![
        text(now_clock()).font(MONO).size(12).color(C::text()),
        micro_color("UTC", C::dim()),
    ]
    .spacing(1)
    .align_x(iced::Alignment::End);

    container(
        row![left, center, right]
            .spacing(24)
            .align_y(iced::Alignment::Center)
            .width(Length::Fill),
    )
    .padding(super::components::PAD)
    .width(Length::Fill)
    .style(panel_style)
    .into()
}

fn now_clock() -> String {
    use chrono::Utc;
    Utc::now().format("%H:%M:%S").to_string()
}

/// Micro uppercase label with explicit color.
pub fn micro_color<'a, M: 'a>(s: impl Into<String>, c: iced::Color) -> Element<'a, M> {
    text(s.into().to_uppercase())
        .font(MONO)
        .size(9)
        .color(c)
        .into()
}