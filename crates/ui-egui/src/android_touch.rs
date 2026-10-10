//! Android UI ergonomics for PhotoCraft's existing egui interface.
//!
//! eframe's egui-winit backend *already* converts the primary Android touch to
//! pointer events, while preserving native events for multi-touch. Injecting
//! a second PointerButton here makes a single tap paint twice. Leave event
//! conversion to eframe and change only visual/touch target sizing.
#![cfg(target_os = "android")]

use egui::{Context, Theme, vec2};

/// Install touch-friendly sizes after the PhotoCraft theme is installed.
///
/// The OS/winit pipeline reports positions in egui points, so this intentionally
/// does not multiply input coordinates by Android's physical pixel density.
pub fn configure_touch_ui(ctx: &Context) {
    ctx.set_zoom_factor(1.0);
    for theme in [Theme::Light, Theme::Dark] {
        ctx.style_mut_of(theme, |style| {
            style.spacing.interact_size.y = style.spacing.interact_size.y.max(44.0);
            style.spacing.interact_size.x = style.spacing.interact_size.x.max(44.0);
            style.spacing.button_padding = vec2(12.0, 9.0);
            // egui's default 3 pt window-edge grab radius is intended for a mouse.
            // Keep these Android-only so desktop hit testing is unchanged.
            style.interaction.interact_radius = style.interaction.interact_radius.max(8.0);
            style.interaction.resize_grab_radius_side = style.interaction.resize_grab_radius_side.max(18.0);
            style.interaction.resize_grab_radius_corner = style.interaction.resize_grab_radius_corner.max(24.0);
        });
    }
}
