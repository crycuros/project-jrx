//! Bottom-right: recent trades matrix (direction, price, win prob, P&L).

use iced::widget::{column, container, row, scrollable, text};
use iced::{Element, Length};

use trading_core::{OrderSide, Trade};

use super::components::{mono_color, panel};
use crate::theme::{C, MONO, MONO_BOLD};

pub fn view<'a, M: 'a>(trades: &'a [Trade]) -> Element<'a, M> {
    let header = row![
        cell_text("DIR", Length::Fixed(52.0)),
        cell_text("SYMBOL", Length::Fixed(90.0)),
        cell_text("PRICE", Length::Fixed(76.0)),
        cell_text("WIN%", Length::Fill),
        cell_text("P&L", Length::Fill),
    ]
    .spacing(4);

    let body = if trades.is_empty() {
        column![mono_color("NO TRADE EXECUTIONS YET", C::dim())]
    } else {
        column![].extend(trades.iter().take(8).map(trade_row))
    };

    panel(
        "RECENT TRADES",
        None,
        column![header, iced::widget::horizontal_rule(1), scrollable(body)].spacing(4),
    )
}

fn cell_text<'a, M: 'a>(s: impl Into<String>, w: Length) -> Element<'a, M> {
    container(mono_color(s, C::dim())).width(w).into()
}

fn trade_row<'a, M: 'a>(t: &'a Trade) -> Element<'a, M> {
    let long = t.side == OrderSide::Buy;
    let dir_color = if long { C::green() } else { C::red() };
    let dir_label = if long { "LONG" } else { "SHORT" };

    // WIN% shows the actual outcome for closed trades (won=100/lost=0);
    // for open/synced trades it falls back to the AI confidence estimate.
    let (win_pct, win_col) = match t.pnl {
        Some(p) if p > 0.0 => (100.0, C::green()),
        Some(_) => (0.0, C::red()),
        None => {
            let conf_pct = t.confidence * 100.0;
            let conf_col = if conf_pct >= 70.0 { C::green() } else if conf_pct >= 50.0 { C::amber() } else { C::red() };
            (conf_pct, conf_col)
        }
    };

    let pnl_text = match t.pnl {
        Some(p) => format!("{p:+.2}"),
        None => match t.unrealized_pnl {
            Some(u) => format!("{u:+.2}"),
            None => "—".to_string(),
        },
    };
    let pnl_color = if pnl_text.starts_with('+') { C::green() } else { C::red() };

    container(
        row![
            container(text(dir_label).font(MONO_BOLD).size(9).color(dir_color)).width(Length::Fixed(52.0)),
            container(text(&t.symbol).font(MONO).size(10).color(C::text())).width(Length::Fixed(90.0)),
            container(text(format!("{:.1}", t.entry_price)).font(MONO).size(10).color(C::text())).width(Length::Fixed(76.0)),
            container(text(format!("{:.0}%", win_pct)).font(MONO).size(10).color(win_col)).width(Length::Fill),
            container(text(pnl_text).font(MONO_BOLD).size(10).color(pnl_color)).width(Length::Fill),
        ]
        .spacing(4),
    )
    .style(row_style)
    .into()
}

fn row_style(theme: &iced::Theme) -> iced::widget::container::Style {
    let _ = theme;
    iced::widget::container::Style {
        background: Some(iced::Background::Color(C::bg_card())),
        border: iced::Border {
            color: C::border(),
            width: 1.0,
            radius: iced::border::Radius::from(2.0),
        },
        ..Default::default()
    }
}