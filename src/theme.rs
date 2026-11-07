//! cheapedit's look. Dark only — there is no light theme, by design.

use eframe::egui::{self, Color32, CornerRadius, Frame, Margin, RichText, Stroke};

pub const PANEL: Color32 = Color32::from_rgb(0x1d, 0x1c, 0x25);
pub const CARD: Color32 = Color32::from_rgb(0x24, 0x23, 0x2e);
pub const CODE_BG: Color32 = Color32::from_rgb(0x12, 0x11, 0x17);
pub const BORDER: Color32 = Color32::from_rgb(0x34, 0x32, 0x42);
pub const TEXT: Color32 = Color32::from_rgb(0xe8, 0xe6, 0xf0);
pub const MUTED: Color32 = Color32::from_rgb(0x9a, 0x97, 0xab);
pub const ACCENT: Color32 = Color32::from_rgb(0x7f, 0x77, 0xdd);
pub const ACCENT_DIM: Color32 = Color32::from_rgb(0x3c, 0x34, 0x89);
pub const CORAL: Color32 = Color32::from_rgb(0xf0, 0x99, 0x7b);
pub const OK: Color32 = Color32::from_rgb(0x6f, 0xcf, 0x97);
pub const WARN: Color32 = Color32::from_rgb(0xef, 0xb0, 0x4c);
pub const DANGER: Color32 = Color32::from_rgb(0xf0, 0x6e, 0x6e);

pub fn apply(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = PANEL;
    v.window_fill = PANEL;
    v.extreme_bg_color = CODE_BG;
    v.faint_bg_color = CARD;
    v.override_text_color = Some(TEXT);
    v.hyperlink_color = ACCENT;
    v.selection.bg_fill = ACCENT_DIM;
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.inactive.corner_radius = CornerRadius::same(5);
    v.widgets.hovered.corner_radius = CornerRadius::same(5);
    v.widgets.active.corner_radius = CornerRadius::same(5);
    v.window_corner_radius = CornerRadius::same(10);
    ctx.set_visuals_of(egui::Theme::Dark, v);
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(10.0, 4.0);
        s.spacing.interact_size.y = 26.0;
        // egui's default "small" text (~9px) is too small to read comfortably.
        if let Some(f) = s.text_styles.get_mut(&egui::TextStyle::Small) {
            f.size = 12.0;
        }
    });
}

pub fn card() -> Frame {
    Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::same(10))
}

pub fn code_frame() -> Frame {
    Frame::new()
        .fill(CODE_BG)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::same(8))
}

pub fn primary_button(label: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(label.to_string()).color(Color32::WHITE).size(15.0))
        .fill(ACCENT_DIM)
        .stroke(Stroke::new(1.0, ACCENT))
}

/// Section heading on the right-hand tools panel.
pub fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(title).strong().size(14.5));
}

pub fn hint(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.label(RichText::new(text.into()).small().color(MUTED));
}

pub fn warn(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.label(RichText::new(text.into()).small().color(WARN));
}
