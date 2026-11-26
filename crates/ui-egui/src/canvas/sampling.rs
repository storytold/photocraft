//! Colour sampling at a document point: the Eyedropper's pick, and the composite colour the dialog
//! eyedroppers read.

use craft::color_ui::ui::{SampleSize, SampleSource};
use serde_json::json;

use super::PhotocraftApp;
use crate::state::Tool;

/// Native ink sample and its document-profile RGB stand-in, sharing the RGB sampler's box and source.
pub(crate) fn native_color(app: &PhotocraftApp, at: [f64; 2], size: SampleSize, source: SampleSource) -> Option<([f32; 3], [f32; 4])> {
    use photocraft_engine::sample_cmds::{SampleLayers, sample_cmyk_at};
    let st = app.session.active()?;
    let inks = sample_cmyk_at(&st.doc, st.active_layer, at[0], at[1], size.pixels(), Some(f64::from(app.current_zoom())), SampleLayers::parse(source.id())?)?;
    let space = photocraft_compose::cmyk_space(&st.doc);
    let rgb = photocraft_color::convert::with_cmyk_space(space.as_ref(), || photocraft_color::convert::cmyk_to_rgb(inks));
    Some((rgb.map(|v| v.clamp(0.0, 1.0)), inks))
}

/// Pixel picks retain inks; mask, alpha and Quick Mask picks remain grayscale.
pub(crate) fn native_pick(app: &PhotocraftApp, at: [f64; 2], size: SampleSize, source: SampleSource) -> Option<([f32; 3], [f32; 4])> {
    if eyedropper_mask_source(app, source).is_some() || super::paint_target(app) != json!("pixels") {
        return None;
    }
    native_color(app, at, size, source)
}

/// The colour at document point `at` averaged over `size`, from the layers `source` names
/// (`document.sampleColor`). `None` off the image or over transparency.
pub fn sample_color(app: &mut PhotocraftApp, at: [f64; 2], size: SampleSize, source: SampleSource) -> Option<[f32; 3]> {
    let [x, y] = at;
    if !(x.is_finite() && y.is_finite()) {
        return None;
    }
    // The box depends on the view zoom at 100 % and below, as in Photoshop (#11751).
    let zoom = f64::from(app.current_zoom());
    let v = app.run("document.sampleColor", json!({"x": x, "y": y, "size": size.pixels(), "sampleLayer": source.id(), "zoom": zoom})).ok()?;
    match serde_json::from_value::<Vec<f32>>(v).ok()?[..] {
        [r, g, b, a] if a > 0.0 => Some([r, g, b]),
        _ => None,
    }
}

/// What the Eyedropper picks at document point `at`: the targeted raster mask's value when the mask
/// source applies, else the colour (grey unless the paint target is pixels).
pub fn pick(app: &mut PhotocraftApp, at: [f64; 2], size: SampleSize, source: SampleSource) -> Option<[f32; 3]> {
    if let Some((surface, canvas)) = eyedropper_mask_source(app, source) {
        return sample_layer_mask(surface, canvas, at, size.pixels());
    }
    sample_color(app, at, size, source)
        .map(|rgb| if super::paint_target(app) == json!("pixels") { rgb } else { [photocraft_color::convert::rgb_to_gray(rgb); 3] })
}

/// What the Eyedropper tool (and a painting tool's ⌥-click) picks at document point `at`: its options
/// bar's Sample Size and Sample (#1649).
pub fn eyedropper_color(app: &mut PhotocraftApp, at: [f64; 2]) -> Option<[f32; 3]> {
    let options = app.ui.tool_options.eyedropper;
    sample_color(app, at, options.size, options.source)
}

/// The active document's composite colour at document point (x, y), averaged over the Eyedropper's
/// Sample Size as Photoshop's dialog eyedroppers (Curves, Color Picker) do. `None` off the image or
/// over transparency.
pub fn composite_color(app: &mut PhotocraftApp, x: f64, y: f64) -> Option<[f32; 3]> {
    let size = app.ui.tool_options.eyedropper.size;
    sample_color(app, [x, y], size, SampleSource::All)
}

/// Only the Eyedropper's Current Layer and Current & Below modes pick the targeted raster mask.
fn eyedropper_mask_source(app: &PhotocraftApp, source: SampleSource) -> Option<(&photocraft_raster::Surface, photocraft_geom::Rect)> {
    use photocraft_engine::{channel_cmds::ChannelTarget, mask_view_cmds};

    if app.ui.tool != Tool::Eyedropper || app.ui.vector_mask_target || !matches!(source, SampleSource::Current | SampleSource::CurrentAndBelow) {
        return None;
    }
    let st = app.session.active()?;
    // Painting can implicitly edit an adjustment's mask even with its content thumbnail selected.
    let targeted = app.ui.mask_target || st.channel_view.target == ChannelTarget::LayerMask || mask_view_cmds::current(st).is_some_and(|v| !v.filter);
    if !targeted || st.doc.quick_mask.is_some() || !app.layer_mask_targeted() {
        return None;
    }
    let mask = st.doc.layer(st.active_layer?)?.mask.as_ref()?;
    Some((&mask.surface, st.doc.bounds()))
}

/// Read the stored mask channel, including its sparse default, rather than its composited coverage.
fn sample_layer_mask(surface: &photocraft_raster::Surface, canvas: photocraft_geom::Rect, at: [f64; 2], size: u32) -> Option<[f32; 3]> {
    let rect = photocraft_engine::cmd::sample_cmds::sample_rect(at[0], at[1], size, canvas)?;
    let mut sum = 0.0;
    for y in rect.y0..rect.y1 {
        for x in rect.x0..rect.x1 {
            sum += f64::from(surface.sample_channel(x, y, 0));
        }
    }
    let gray = (sum / (f64::from(rect.width()) * f64::from(rect.height()))) as f32;
    gray.is_finite().then_some([gray; 3])
}

#[cfg(test)]
#[path = "../eyedropper_mask_sample_tests.rs"]
mod eyedropper_mask_sample_tests;
