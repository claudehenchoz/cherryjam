//! Look & feel.

use egui::{Color32, CornerRadius, Stroke, Visuals};

pub const ACCENT: Color32 = Color32::from_rgb(0xD7, 0x26, 0x3D);
pub const ACCENT_DIM: Color32 = Color32::from_rgb(0x8E, 0x1A, 0x2A);

pub fn apply(ctx: &egui::Context) {
    let mut v = Visuals::dark();
    v.panel_fill = Color32::from_rgb(0x1B, 0x1A, 0x1F);
    v.window_fill = Color32::from_rgb(0x22, 0x21, 0x27);
    v.extreme_bg_color = Color32::from_rgb(0x12, 0x11, 0x15);
    v.faint_bg_color = Color32::from_rgb(0x26, 0x24, 0x2B);
    v.selection.bg_fill = ACCENT_DIM;
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.hyperlink_color = ACCENT;
    let r = CornerRadius::same(6);
    v.widgets.noninteractive.corner_radius = r;
    v.widgets.inactive.corner_radius = r;
    v.widgets.hovered.corner_radius = r;
    v.widgets.active.corner_radius = r;
    v.widgets.open.corner_radius = r;
    v.widgets.active.bg_fill = ACCENT;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT);
    v.slider_trailing_fill = true;
    ctx.set_visuals(v);
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(10.0, 4.0);
        s.spacing.slider_width = 160.0;
    });
}
