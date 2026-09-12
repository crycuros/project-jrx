//! Right sidebar: News Scanner + Neural Feed + Trade Execution Box.

use iced::widget::{column, container, row, scrollable, text};
use iced::{Element, Length};

use trading_core::{NewsItem, OrderSide, Trade};

use super::components::{mono_color, mono_fill, panel, title_elem};
use crate::theme::{C, MONO, MONO_BOLD};

pub fn view<'a, M: 'a>(
    news: &'a [NewsItem],
    neural: &[String],
    trade_action: &'a str,
    trade_symbol: &'a str,
    trade_msg_opt: Option<&'a str>,
    position: Option<Trade>,
) -> Element<'a, M> {
    let news_body = if news.is_empty() {
        column![mono_color("NO NEWS FEED — AWAITING DATA", C::dim())]
    } else {
        column![].extend(news.iter().take(6).map(|n| news_row(n)))
    };

    let neural_body = if neural.is_empty() {
        column![mono_color("NEURAL FEED IDLE", C::dim())]
    } else {
        column![].extend(neural.iter().take(5).map(|s| {
            row![mono_color(">", C::purple()), mono_fill(s, C::text())].spacing(4).into()
        }))
    };

    let trade_box = trade_box_inner(trade_action, trade_symbol, trade_msg_opt, position);

    panel(
        "FEEDS & EXECUTION",
        None,
        column![
            title_elem("NEWS SCANNER"),
            scrollable(column![news_body].spacing(2)).height(Length::Fixed(150.0)),
            super::components::hline(),
            title_elem("NEURAL FEED"),
            scrollable(column![neural_body].spacing(3)).height(Length::Fixed(120.0)),
            super::components::hline(),
            trade_box,
        ]
        .spacing(6),
    )
}

fn news_row<'a, M: 'a>(n: &'a NewsItem) -> Element<'a, M> {
    let impact_color = match n.impact.as_str() {
        "HIGH" => C::red(),
        "MED" => C::amber(),
        _ => C::green(),
    };
    container(
        column![
            row![
                text(&n.time).font(MONO).size(8).color(C::dim()),
                text(&n.source).font(MONO).size(8).color(C::blue()),
                text(&n.impact).font(MONO).size(8).color(impact_color),
            ]
            .spacing(6),
            text(&n.title).font(MONO).size(9).color(C::text()),
        ]
        .spacing(2),
    )
    .padding(4)
    .style(container_row_style)
    .into()
}

fn trade_box_inner<'a, M: 'a>(action: &'a str, symbol: &'a str, msg: Option<&'a str>, position: Option<Trade>) -> Element<'a, M> {
    let has_pos = position.is_some();
    let (disp_sym, disp_action, entry, cur, pnl, sl, tp, verdict, is_buy) = match position {
        Some(p) => {
            let side_label = match p.side {
                OrderSide::Buy => "LONG",
                OrderSide::Sell => "SHORT",
            };
            let is_buy = p.side == OrderSide::Buy;
            let verdict_label = if p.ai_verdict.is_empty() { "AWAITING AI" } else { p.ai_verdict.as_str() };
            (p.symbol, side_label.to_string(), p.entry_price, p.current_price.unwrap_or(p.entry_price), p.unrealized_pnl.unwrap_or(0.0), p.stop_loss, p.take_profit, verdict_label.to_string(), is_buy)
        }
        None => (symbol.to_string(), if action.to_lowercase() == "buy" { "LONG" } else { "SHORT" }.to_string(), 0.0, 0.0, 0.0, 0.0, 0.0, "NO OPEN POSITION".to_string(), action.to_lowercase() == "buy"),
    };

    let action_color = if is_buy { C::green() } else { C::red() };
    let pnl_color = if !has_pos {
        C::dim()
    } else if pnl.abs() < f64::EPSILON {
        C::amber()
    } else if pnl > 0.0 {
        C::green()
    } else {
        C::red()
    };

    let sig_price = if has_pos { format!("{:.5}", entry) } else { "—".to_string() };
    let cur_price = if has_pos { format!("{:.5}", cur) } else { "—".to_string() };
    let pnl_txt = if has_pos { format!("{}{:.2}", if pnl > 0.0 { "+" } else { "" }, pnl) } else { "—".to_string() };
    let sl_txt = if has_pos { format!("{:.5}", sl) } else { "—".to_string() };
    let tp_txt = if has_pos { format!("{:.5}", tp) } else { "—".to_string() };

    column![
        title_elem("TRADE BOX"),
        row![
            text("QUICK ORDER").font(MONO).size(8).color(C::dim()),
            text(disp_sym.clone()).font(MONO_BOLD).size(11).color(action_color),
            text(disp_action.clone()).font(MONO).size(9).color(action_color),
        ]
        .spacing(6),
        row![mono_color("SYMBOL", C::dim()), mono_color(disp_sym.clone(), C::text())].spacing(4),
        row![mono_color("SIG PRICE", C::dim()), mono_color(sig_price, C::text())].spacing(4),
        row![mono_color("CURRENT", C::dim()), mono_color(cur_price, C::blue())].spacing(4),
        row![mono_color("P/L USD", C::dim()), mono_color(pnl_txt, pnl_color)].spacing(4),
        row![mono_color("SL", C::dim()), mono_color(sl_txt, C::red())].spacing(4),
        row![mono_color("TP", C::dim()), mono_color(tp_txt, C::green())].spacing(4),
        row![mono_color("AI VERDICT", C::dim()), mono_color(verdict.clone(), C::amber())].spacing(4),
        if let Some(m) = msg {
            text(m).font(MONO).size(9).color(C::green())
        } else {
            text("ENGINE OFFLINE").font(MONO).size(9).color(C::dim())
        },
    ]
    .spacing(4)
    .into()
}

fn container_row_style(theme: &iced::Theme) -> iced::widget::container::Style {
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