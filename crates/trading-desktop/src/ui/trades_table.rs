//! Bottom-right: Recent Trades Matrix (direction, symbol, price, win prob, P&L).

use egui::{FontFamily, FontId, Ui};
use trading_core::{OrderSide, Trade};

use super::components::{badge, hline, mono, panel};
use crate::theme::C;

pub fn view(ui: &mut Ui, trades: &[Trade]) {
    panel(
        ui,
        "RECENT TRADES",
        |ui| {
            mono(ui, format!("{} EXECUTIONS", trades.len()), C::TEXT_DIM, 9.0);
        },
        |ui| {
            ui.vertical(|ui| {
                // Table Header
                ui.horizontal(|ui| {
                    ui.add_sized([45.0, 14.0], egui::Label::new(header_text("DIR")));
                    ui.add_sized([75.0, 14.0], egui::Label::new(header_text("SYMBOL")));
                    ui.add_sized([65.0, 14.0], egui::Label::new(header_text("PRICE")));
                    ui.add_sized([50.0, 14.0], egui::Label::new(header_text("WIN%")));
                    ui.add_sized([70.0, 14.0], egui::Label::new(header_text("P&L USD")));
                });

                hline(ui);

                egui::ScrollArea::vertical()
                    .id_salt("trades_table_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if trades.is_empty() {
                            ui.add_space(8.0);
                            mono(ui, "NO TRADE EXECUTIONS RECORDED YET", C::TEXT_DIM, 9.0);
                        } else {
                            for t in trades.iter().take(8) {
                                trade_row_view(ui, t);
                            }
                        }
                    });
            });
        },
    );
}

fn header_text(s: &str) -> egui::WidgetText {
    egui::RichText::new(s)
        .font(FontId::new(8.5, FontFamily::Monospace))
        .color(C::TEXT_DIM)
        .into()
}

fn trade_row_view(ui: &mut Ui, t: &Trade) {
    let long = t.side == OrderSide::Buy;
    let dir_color = if long { C::GREEN } else { C::RED };
    let dir_label = if long { "BUY" } else { "SELL" };

    let (win_pct, win_col) = match t.pnl {
        Some(p) if p > 0.0 => (100.0, C::GREEN),
        Some(_) => (0.0, C::RED),
        None => {
            let conf_pct = t.confidence * 100.0;
            let conf_col = if conf_pct >= 70.0 {
                C::GREEN
            } else if conf_pct >= 50.0 {
                C::AMBER
            } else {
                C::RED
            };
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
    let pnl_color = if pnl_text.starts_with('+') { C::GREEN } else if pnl_text.starts_with('-') { C::RED } else { C::TEXT_DIM };

    C::card_frame().show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.add_sized([40.0, 16.0], |ui: &mut Ui| {
                badge(ui, dir_label, dir_color, C::BG_ELEVATED)
            });
            ui.add_sized([70.0, 16.0], |ui: &mut Ui| {
                ui.label(egui::RichText::new(&t.symbol).font(FontId::new(9.5, FontFamily::Monospace)).color(C::TEXT))
            });
            ui.add_sized([60.0, 16.0], |ui: &mut Ui| {
                ui.label(egui::RichText::new(format!("{:.4}", t.entry_price)).font(FontId::new(9.5, FontFamily::Monospace)).color(C::TEXT_MUTED))
            });
            ui.add_sized([45.0, 16.0], |ui: &mut Ui| {
                ui.label(egui::RichText::new(format!("{:.0}%", win_pct)).font(FontId::new(9.5, FontFamily::Monospace)).color(win_col))
            });
            ui.add_sized([65.0, 16.0], |ui: &mut Ui| {
                ui.label(egui::RichText::new(pnl_text).font(FontId::new(9.5, FontFamily::Monospace)).color(pnl_color).strong())
            });
        });
    });
}