//! Terminal theme: dark pitch-black palette from planUi.txt.
//! Uses Colors only (AppTheme dark-only for v1).

use iced::Color;
use iced::font::{Family, Font, Weight};

pub const MONO: Font = Font {
    family: Family::Name("JetBrains Mono"),
    weight: Weight::Normal,
    stretch: iced::font::Stretch::Normal,
    style: iced::font::Style::Normal,
};

pub const MONO_BOLD: Font = Font {
    family: Family::Name("JetBrains Mono"),
    weight: Weight::Bold,
    stretch: iced::font::Stretch::Normal,
    style: iced::font::Style::Normal,
};

/// Forked from planUi.txt.
pub struct C;

impl C {
    // Backgrounds
pub fn bg() -> Color { Color::from_rgb(0.051, 0.055, 0.071) }        // #0D0E12
    pub fn bg_panel() -> Color { Color::from_rgb(0.071, 0.075, 0.090) }  // #12131A
    #[allow(dead_code)]
    pub fn bg_elevated() -> Color { Color::from_rgb(0.110, 0.114, 0.130) } // #1C1D21
    #[allow(dead_code)]
    pub fn bg_card() -> Color { Color::from_rgb(0.082, 0.086, 0.102) }   // #151619

    // Text
    pub fn text() -> Color { Color::from_rgb(0.878, 0.878, 0.878) }      // #E0E0E0
    pub fn text2() -> Color { Color::from_rgb(0.550, 0.550, 0.550) }     // #8C8C8C
    pub fn dim() -> Color { Color::from_rgb(0.400, 0.400, 0.400) }       // #666666

    // Trading colors
    pub fn green() -> Color { Color::from_rgb(0.0, 1.0, 0.529) }         // #00FF87
    pub fn red() -> Color { Color::from_rgb(1.0, 0.231, 0.188) }         // #FF3B30
    pub fn amber() -> Color { Color::from_rgb(1.0, 0.757, 0.027) }       // #FFC107

    // Accents
    pub fn blue() -> Color { Color::from_rgb(0.290, 0.620, 1.0) }
    pub fn purple() -> Color { Color::from_rgb(0.659, 0.333, 0.969) }
    pub fn orange() -> Color { Color::from_rgb(1.0, 0.600, 0.0) }
    pub fn yellow() -> Color { Color::from_rgb(1.0, 0.757, 0.027) }
    pub fn cyan() -> Color { Color::from_rgb(0.0, 0.878, 0.878) }

    // Borders
    pub fn border() -> Color { Color::from_rgb(0.118, 0.118, 0.141) }    // #1E1E24
    #[allow(dead_code)]
    pub fn border_light() -> Color { Color::from_rgb(0.180, 0.180, 0.210) } // #2E2E36

    // Network clusters
    pub fn cluster(s: &str) -> Color {
        match s {
            "MICROCAPS" => Self::red(),
            "MIDCAPS" => Self::orange(),
            "MEGACAPS" => Self::yellow(),
            "SOLANA" => Self::blue(),
            "DEFI" => Self::green(),
            "ETH/EVM" => Self::purple(),
            _ => Self::cyan(),
        }
    }
}

/// Root window background.
pub fn bg_root(theme: &iced::Theme) -> iced::widget::container::Style {
    let _ = theme;
    iced::widget::container::Style {
        background: Some(C::bg().into()),
        ..Default::default()
    }
}