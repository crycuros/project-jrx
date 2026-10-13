//! Modern Monochrome White-Dark Institutional Theme for egui.
//!
//! Palette philosophy:
//!   • Pure matte carbon/obsidian surfaces (#09090B → #1E1E22) with zero blue tint
//!   • High-contrast Stark White (#FAFAFA) primary typography & active accents
//!   • Neutral Zinc (#A1A1AA / #71717A) secondary & metadata hierarchy
//!   • Restrained semantic trading colors (Institutional Emerald #22C55E, Rose #F43F5E, Gold #F59E0B)

use egui::{epaint::Shadow, Color32, Frame, Margin, Rounding, Stroke};

pub struct C;

impl C {
    // ── Pure Carbon / Zinc Dark Surfaces (Zero Blue Tint) ───────────────────
    pub const BG: Color32 = Color32::from_rgb(9, 9, 11);               // #09090B (App Canvas)
    pub const BG_PANEL: Color32 = Color32::from_rgb(15, 15, 18);       // #0F0F12 (Primary Panel)
    pub const BG_CARD: Color32 = Color32::from_rgb(22, 22, 26);        // #16161A (Grouped Card)
    pub const BG_ELEVATED: Color32 = Color32::from_rgb(28, 28, 33);    // #1C1C21 (Elevated / Tooltip)
    pub const BG_CARD_HOVER: Color32 = Color32::from_rgb(34, 34, 40);  // #222228 (Interactive Hover)

    // ── Typography Hierarchy (Stark White + Neutral Zinc) ───────────────────
    pub const TEXT: Color32 = Color32::from_rgb(250, 250, 250);        // #FAFAFA (Stark White Primary)
    pub const TEXT_MUTED: Color32 = Color32::from_rgb(161, 161, 170);  // #A1A1AA (Zinc 400 Secondary)
    pub const TEXT_DIM: Color32 = Color32::from_rgb(113, 113, 122);    // #71717A (Zinc 500 Metadata)

    // ── Semantic Trading Colors (Restrained Institutional Palette) ──────────
    pub const GREEN: Color32 = Color32::from_rgb(34, 197, 94);         // #22C55E (Emerald Bull)
    pub const RED: Color32 = Color32::from_rgb(244, 63, 94);           // #F43F5E (Rose Bear)
    pub const AMBER: Color32 = Color32::from_rgb(245, 158, 11);        // #F59E0B (Gold / Caution)

    // ── Monochrome White & Silver Accents (No Blue / Neon Cyan / Purple) ────
    pub const WHITE: Color32 = Color32::from_rgb(250, 250, 250);       // #FAFAFA (Primary White Accent)
    pub const CYAN: Color32 = Color32::from_rgb(244, 244, 245);        // #F4F4F5 (Mapped to Stark White)
    pub const BLUE: Color32 = Color32::from_rgb(212, 212, 216);        // #D4D4D8 (Mapped to Crisp Silver)
    pub const PURPLE: Color32 = Color32::from_rgb(161, 161, 170);      // #A1A1AA (Mapped to Neutral Zinc)
    pub const ORANGE: Color32 = Color32::from_rgb(245, 158, 11);       // #F59E0B (Warm Gold)

    // ── Crisp Neutral Hairline Borders ──────────────────────────────────────
    pub const BORDER: Color32 = Color32::from_rgb(36, 36, 41);         // #242429 (Subtle Hairline)
    pub const BORDER_LIGHT: Color32 = Color32::from_rgb(63, 63, 70);   // #3F3F46 (Active / Hover Border)
    #[allow(dead_code)]
    pub const BORDER_CYAN: Color32 = Color32::from_rgba_premultiplied(250, 250, 250, 60);

    // Dynamic alpha helper
    pub fn with_alpha(c: Color32, a: u8) -> Color32 {
        Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
    }

    /// Apply monochrome White-Dark visuals to egui context so native sliders,
    /// inputs, scrollbars, and selection highlights have zero blue tint.
    pub fn apply_visuals(ctx: &egui::Context) {
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = Self::BG;
        visuals.window_fill = Self::BG_PANEL;
        visuals.extreme_bg_color = Self::BG;
        visuals.faint_bg_color = Self::BG_CARD;
        visuals.code_bg_color = Self::BG_CARD;

        visuals.selection.bg_fill = Color32::from_rgb(244, 244, 245);
        visuals.selection.stroke = Stroke::new(1.0_f32, Self::BG);

        visuals.widgets.noninteractive.bg_fill = Self::BG_CARD;
        visuals.widgets.noninteractive.weak_bg_fill = Self::BG_CARD;
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, Self::BORDER);
        visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, Self::TEXT_MUTED);
        visuals.widgets.noninteractive.rounding = Rounding::same(4.0);

        visuals.widgets.inactive.bg_fill = Self::BG_ELEVATED;
        visuals.widgets.inactive.weak_bg_fill = Self::BG_CARD;
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, Self::BORDER);
        visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, Self::TEXT);
        visuals.widgets.inactive.rounding = Rounding::same(4.0);

        visuals.widgets.hovered.bg_fill = Self::BG_CARD_HOVER;
        visuals.widgets.hovered.weak_bg_fill = Self::BG_CARD_HOVER;
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, Self::BORDER_LIGHT);
        visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, Self::WHITE);
        visuals.widgets.hovered.rounding = Rounding::same(4.0);

        visuals.widgets.active.bg_fill = Self::WHITE;
        visuals.widgets.active.weak_bg_fill = Self::WHITE;
        visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, Self::WHITE);
        visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, Self::BG);
        visuals.widgets.active.rounding = Rounding::same(4.0);

        ctx.set_visuals(visuals);
    }

    // Modern Topbar / Header Frame
    pub fn header_frame() -> Frame {
        Frame {
            inner_margin: Margin::symmetric(16.0, 10.0),
            outer_margin: Margin::ZERO,
            rounding: Rounding::same(6.0),
            shadow: Shadow::NONE,
            fill: Self::BG_PANEL,
            stroke: Stroke::new(1.0_f32, Self::BORDER),
        }
    }

    // Modern Panel Frame
    pub fn panel_frame() -> Frame {
        Frame {
            inner_margin: Margin::same(12.0),
            outer_margin: Margin::ZERO,
            rounding: Rounding::same(6.0),
            shadow: Shadow::NONE,
            fill: Self::BG_PANEL,
            stroke: Stroke::new(1.0_f32, Self::BORDER),
        }
    }

    // Modern Elevated Card Frame
    pub fn card_frame() -> Frame {
        Frame {
            inner_margin: Margin::same(10.0),
            outer_margin: Margin::ZERO,
            rounding: Rounding::same(4.0),
            shadow: Shadow::NONE,
            fill: Self::BG_CARD,
            stroke: Stroke::new(1.0_f32, Self::BORDER),
        }
    }

    // Modal Frame
    #[allow(dead_code)]
    pub fn modal_frame() -> Frame {
        Frame {
            inner_margin: Margin::same(20.0),
            outer_margin: Margin::ZERO,
            rounding: Rounding::same(8.0),
            shadow: Shadow {
                offset: egui::vec2(0.0, 4.0),
                blur: 16.0,
                spread: 0.0,
                color: Color32::from_black_alpha(120),
            },
            fill: Self::BG_ELEVATED,
            stroke: Stroke::new(1.0_f32, Self::BORDER_LIGHT),
        }
    }
}