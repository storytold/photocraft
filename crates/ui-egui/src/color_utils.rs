//! Small colour operations shared by egui UI components.

use egui::Color32;

/// Colours for a surface and the text rendered on it.
pub(crate) struct SurfaceColors {
    pub background: Color32,
    pub foreground: Color32,
}

/// Derive a subtly contrasted surface and readable neutral foreground from a theme surface.
/// Contrast ratios are policy inputs; the sRGB/luminance calculations stay encapsulated here.
pub(crate) fn contrasting_surface_colors(base: Color32, dark: bool, surface_contrast: f32, text_contrast: f32) -> SurfaceColors {
    let base_luminance = relative_luminance(base);
    let surface_luminance = ((base_luminance + 0.05) / surface_contrast - 0.05).max(0.0);
    let background = with_luminance(base, surface_luminance);
    let background_luminance = relative_luminance(background);
    let foreground_luminance = if dark {
        (text_contrast * (background_luminance + 0.05) - 0.05).clamp(0.0, 1.0)
    } else {
        ((background_luminance + 0.05) / text_contrast - 0.05).clamp(0.0, 1.0)
    };

    SurfaceColors { background, foreground: gray_with_luminance(foreground_luminance) }
}

fn relative_luminance(color: Color32) -> f32 {
    let linear = |channel: u8| egui::ecolor::linear_from_gamma(f32::from(channel) / 255.0);
    0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
}

/// Scale a colour's linear RGB channels to the requested luminance while preserving hue and alpha.
fn with_luminance(color: Color32, luminance: f32) -> Color32 {
    let source_luminance = relative_luminance(color);
    let target = luminance.clamp(0.0, 1.0);
    if source_luminance <= f32::EPSILON {
        let channel = (egui::ecolor::gamma_from_linear(target) * 255.0).round().clamp(0.0, 255.0) as u8;
        return Color32::from_rgba_unmultiplied(channel, channel, channel, color.a());
    }

    let scale = target / source_luminance;
    let channel = |value: u8| {
        (egui::ecolor::gamma_from_linear((egui::ecolor::linear_from_gamma(f32::from(value) / 255.0) * scale).clamp(0.0, 1.0)) * 255.0).round().clamp(0.0, 255.0)
            as u8
    };
    Color32::from_rgba_unmultiplied(channel(color.r()), channel(color.g()), channel(color.b()), color.a())
}

/// Return a neutral grey with the requested relative luminance.
fn gray_with_luminance(luminance: f32) -> Color32 {
    let channel = (egui::ecolor::gamma_from_linear(luminance.clamp(0.0, 1.0)) * 255.0).round().clamp(0.0, 255.0) as u8;
    Color32::from_gray(channel)
}

/// Scale a colour's alpha by an opacity fraction, preserving its RGB channels.
pub(crate) fn with_opacity(color: Color32, opacity: f32) -> Color32 {
    let opacity = if opacity.is_finite() { opacity.clamp(0.0, 1.0) } else { 1.0 };
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), (f32::from(color.a()) * opacity).round() as u8)
}
