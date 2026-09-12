//! Left sidebar panel: Portfolio & Risk Controls.
//! SL/Factor weights box sits at the bottom (per planUi correction).

use iced::widget::{column, container, row, scrollable, text};
use iced::{Element, Length};

use trading_core::{OrderSide, Status, Trade};

use super::components::{gauge, mono, panel, panel_style, title_elem};
use crate::theme::{C, MONO, MONO_BOLD};

pub fn view<'a, M: 'a>(
    status: &Status,
    positions: &[Trade],
    sl_amount: f32,
    factor_amount: f32,
    max_open: i32,
    max_daily_loss: f64,
    risk_trade_pct: f64,
) -> Element<'a, M> {
    let pnl_color = if status.total_pnl >= 0.0 { C::green() } else { C::red() };
    let roi_color = if status.daily_pnl >= 0.0 { C::green() } else { C::red() };

    let kpis = column![
        row![
            text("WIN RATE").font(MONO).size(9).color(C::dim()),
            mono_color(format!("{:.1}%", status.win_rate), C::text2()),
        ]
        .spacing(6),
        text(format!("${:>+8.0}", status.total_pnl))
            .font(MONO_BOLD)
            .size(26)
            .color(pnl_color),
        text("NET PROFIT").font(MONO).size(8).color(C::dim()),
        row![
            text(format!("24H ROI ${:+.2}", status.daily_pnl))
                .font(MONO)
                .size(10)
                .color(roi_color),
            text(format!("BAL ${:.0}", status.balance))
                .font(MONO)
                .size(10)
                .color(C::text2()),
        ]
        .spacing(10),
    ]
    .spacing(2);

    // Real gauges derived from live status.
    let s_score = status.win_rate.clamp(0.0, 100.0) as f32;
    let o_lp = if max_open > 0 {
        (status.open_positions as f32 / max_open as f32 * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    };
    let div = if status.balance > 0.0 {
        (status.daily_pnl.abs() / status.balance * 500.0).clamp(0.0, 100.0) as f32
    } else {
        0.0
    };
    let o_ac = if status.ai_available { 100.0 } else { 8.0 };

    let gauges = column![
        gauge("S-SCORE", s_score),
        gauge("O.L.P", o_lp),
        gauge("DIV", div),
        gauge("O.A.C", o_ac),
    ]
    .spacing(6);

    let mut pos_body = column![title_elem("OPEN POSITIONS")].spacing(4);
    if positions.is_empty() {
        pos_body = pos_body.push(mono_color("— no open trades —", C::dim()));
    } else {
        for p in positions.iter().take(4) {
            let pct = pnl_pct(p);
            let signed = format!("{:+.2}%", pct);
            let c = if pct >= 0.0 { C::green() } else { C::red() };
            pos_body = pos_body.push(
                row![
                    mono(format!("{:<12}", p.symbol)),
                    text(signed).font(MONO).size(10).color(c),
                ]
                .spacing(4),
            );
        }
    }

    let weights = container(
        column![
            title_elem("SL/FACTOR WEIGHTS"),
            weight_row("STOP LOSS", sl_amount, C::red()),
            weight_row("FACTOR", factor_amount, C::blue()),
            row![
                mono_color("RISK/TRADE", C::dim()),
                mono_color(format!("{:.1}%", risk_trade_pct), C::text()),
            ]
            .spacing(4),
            row![
                mono_color("MAX OPEN", C::dim()),
                mono_color(max_open.to_string(), C::text()),
            ]
            .spacing(4),
            row![
                mono_color("MAX +LOSS", C::dim()),
                mono_color(format!("{:.2}%", max_daily_loss), C::text()),
            ]
            .spacing(4),
            row![
                mono_color("COOL SEQ", C::dim()),
                mono_color(status.consecutive_losses.to_string(), C::text()),
            ]
            .spacing(4),
        ]
        .spacing(5),
    )
    .padding(8)
    .width(Length::Fill)
    .style(panel_style);

    panel(
        "PORTFOLIO & RISK",
        None,
        scrollable(
            column![kpis, super::components::hline(), gauges, super::components::hline(), pos_body, weights]
                .spacing(10)
                .padding(2),
        )
        .height(Length::Fill)
        .width(Length::Fill),
    )
}

fn weight_row<'a, M: 'a>(label: impl Into<String>, pct: f32, color: iced::Color) -> Element<'a, M> {
    row![
        text(label.into()).font(MONO).size(9).color(C::dim()).width(Length::Fixed(70.0)),
        iced::widget::progress_bar(0.0..=100.0, pct)
            .width(Length::Fill)
            .height(6),
        text(format!("{:.0}%", pct)).font(MONO).size(9).color(color),
    ]
    .spacing(6)
    .align_y(iced::Alignment::Center)
    .into()
}

fn pnl_pct(t: &Trade) -> f64 {
    if t.entry_price <= 0.0 {
        return 0.0;
    }
    let cur = t.current_price.unwrap_or(t.entry_price);
    let raw = (cur - t.entry_price) / t.entry_price * 100.0;
    match t.side {
        OrderSide::Buy => raw,
        OrderSide::Sell => -raw,
    }
}

fn mono_color<'a, M: 'a>(s: impl Into<String>, c: iced::Color) -> Element<'a, M> {
    text(s.into()).font(MONO).size(10).color(c).into()
}