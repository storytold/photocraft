//! LightCraft's develop-view look for the RAW Develop dialog of a raw the shared engine develops:
//! its panel palette and metrics ([`Lc`]), icons ([`icons`]) and widgets ([`widgets`]), ported
//! from LightCraft's `theme.rs`, `icons.rs` and `widgets.rs` (storytold/lightcraft, MIT OR
//! Apache-2.0). Everything here is scoped to the dialog: [`style`] restyles one `Ui` and its
//! children, never the context, so the rest of PhotoCraft (and the classic Camera Raw panel) keeps
//! its own look.

pub(crate) mod icons;
pub(crate) mod widgets;

use egui::{Color32, CornerRadius, FontId, Stroke};

/// LightCraft's design tokens, the ones the develop view uses: Charcoal in a dark PhotoCraft
/// theme, Silver in a light one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Lc {
    /// Panels, the icon rail and the footer.
    pub chrome: Color32,
    /// Behind the photo.
    pub canvas: Color32,
    pub divider: Color32,
    /// Flyout rows and mask tiles.
    pub inset: Color32,
    pub field: Color32,
    pub field_border: Color32,
    pub button: Color32,
    pub button_border: Color32,
    pub hover: Color32,
    pub pressed: Color32,
    pub tool_active: Color32,
    pub text: Color32,
    pub text_label: Color32,
    pub text_dim: Color32,
    pub text_disabled: Color32,
    pub icon: Color32,
    pub track: Color32,
    pub thumb: Color32,
    pub thumb_hover: Color32,
    pub accent: Color32,
    /// The icon of the active tool on the rail.
    pub pick: Color32,
    /// The selected mask's overlay.
    pub mask_overlay: Color32,
}

/// Width of the settings panel (LightCraft's 270 pt, with room for PhotoCraft's histogram).
pub(crate) const PANEL_W: f32 = 300.0;
/// Width of the icon rail at the far right.
pub(crate) const RAIL_W: f32 = 48.0;
/// Height of a slider row.
pub(crate) const SLIDER_ROW_H: f32 = 45.0;

const fn rgb(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

impl Lc {
    pub(crate) const CHARCOAL: Lc = Lc {
        chrome: rgb(0x2d2d2d),
        canvas: rgb(0x1c1c1c),
        divider: rgb(0x1c1c1c),
        inset: rgb(0x232323),
        field: rgb(0x222222),
        field_border: rgb(0x3c3c3c),
        button: rgb(0x242424),
        button_border: rgb(0x4c4c4c),
        hover: rgb(0x3a3a3a),
        pressed: rgb(0x3f3f3f),
        tool_active: rgb(0x3f3f3f),
        text: rgb(0xe2e2e2),
        text_label: rgb(0xbcbcbc),
        text_dim: rgb(0x8e8e8e),
        text_disabled: rgb(0x5c5c5c),
        icon: rgb(0x9a9a9a),
        track: rgb(0x5a5a5a),
        thumb: rgb(0xa0a0a0),
        thumb_hover: rgb(0xe0e0e0),
        accent: rgb(0x0165dd),
        pick: rgb(0xf0f0f0),
        mask_overlay: Color32::from_rgba_premultiplied(0x62, 0x0e, 0x15, 110),
    };

    pub(crate) const SILVER: Lc = Lc {
        chrome: rgb(0xe6e6e6),
        canvas: rgb(0xcfcfcf),
        divider: rgb(0xc4c4c4),
        inset: rgb(0xdcdcdc),
        field: rgb(0xf6f6f6),
        field_border: rgb(0xb8b8b8),
        button: rgb(0xf2f2f2),
        button_border: rgb(0xb0b0b0),
        hover: rgb(0xd2d2d2),
        pressed: rgb(0xc4c4c4),
        tool_active: rgb(0xcacaca),
        text: rgb(0x1c1c1c),
        text_label: rgb(0x383838),
        text_dim: rgb(0x6a6a6a),
        text_disabled: rgb(0xa2a2a2),
        icon: rgb(0x555555),
        track: rgb(0xa8a8a8),
        thumb: rgb(0x5e5e5e),
        thumb_hover: rgb(0x262626),
        accent: rgb(0x0a62d0),
        pick: rgb(0x2a2a2a),
        mask_overlay: Color32::from_rgba_premultiplied(0x62, 0x0e, 0x15, 110),
    };

    /// The tokens for `ctx`'s theme (PhotoCraft's dark themes get Charcoal, the light ones Silver).
    pub(crate) fn get(ctx: &egui::Context) -> Lc {
        if ctx.global_style().visuals.dark_mode { Lc::CHARCOAL } else { Lc::SILVER }
    }

    pub(crate) fn font(&self, size: f32) -> FontId {
        FontId::proportional(size)
    }

    pub(crate) fn semibold(&self, size: f32) -> FontId {
        crate::theme::semibold(size)
    }
}

/// Restyles `ui` (and what is drawn in it) with the develop view's palette: egui's own widgets
/// there (text fields, menus, check boxes, PhotoCraft's buttons) take LightCraft's colours. Only
/// `ui`'s style changes.
pub(crate) fn style(ui: &mut egui::Ui) {
    let t = Lc::get(ui.ctx());
    let s = ui.style_mut();
    let v = &mut s.visuals;
    v.panel_fill = t.chrome;
    v.window_fill = t.chrome;
    v.extreme_bg_color = t.field;
    v.faint_bg_color = t.inset;
    v.override_text_color = None;
    v.selection.bg_fill = t.accent;
    v.selection.stroke = Stroke::new(1.0, t.text);
    let radius = CornerRadius::same(4);
    for (w, fill, stroke, text) in [
        (&mut v.widgets.noninteractive, t.chrome, t.divider, t.text_label),
        (&mut v.widgets.inactive, t.button, t.button_border, t.text),
        (&mut v.widgets.hovered, t.hover, t.button_border, t.text),
        (&mut v.widgets.active, t.pressed, t.button_border, t.text),
        (&mut v.widgets.open, t.pressed, t.button_border, t.text),
    ] {
        w.bg_fill = fill;
        w.weak_bg_fill = fill;
        w.bg_stroke = Stroke::new(1.0, stroke);
        w.fg_stroke = Stroke::new(1.0, text);
        w.corner_radius = radius;
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, t.divider);
}
