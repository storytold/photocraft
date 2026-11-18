//! Gradient tool, live mode (Photoshop 2023+ "Gradient" vs "Classic gradient"): a drag makes a
//! Gradient Fill layer (`gradient.fill.create`), and with a gradient fill layer selected the canvas
//! shows its widget (`live_overlay`): start and end handles, colour stops, midpoints and the style's
//! shape. A drag elsewhere redraws the selected gradient (`tool`).
//!
//! Every gesture previews live through the regular canvas path (a shallow copy of the document
//! with the edited fill, so the GPU canvas just re-runs its gradient pass) and commits one command
//! on release: one history step. The Properties panel and the Gradient Editor panel (`properties_panel`,
//! `editor`) preview through the same path; the options bar's cells are `bar`. The one conversion between
//! the document's gradient and the framework's is `stops`.

use std::sync::Arc;

use egui::Color32;
use photocraft_compose::gradient_fill as gf;
use photocraft_doc::{Document, Fill, Layer, LayerContent, LayerId};
use photocraft_engine::gradient_fill_cmds as cmds;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::state::Tool;

mod bar;
pub(crate) mod editor;
pub(crate) mod live_overlay;
mod properties_panel;
pub(crate) mod stops;
pub(crate) mod tool;

pub(crate) use bar::bar;

#[cfg(test)]
mod editor_tests;
#[cfg(test)]
mod live_overlay_tests;
#[cfg(test)]
mod tests;

/// Id of the temporary layer previewing a new gradient while it is drawn.
pub const PREVIEW_LAYER: LayerId = LayerId(u64::MAX - 78);
/// Tag of this module's canvas preview keys.
const TAG: u64 = 1 << 58;

/// Live-gradient interaction state (not saved): the value of a gesture in progress, cleared on its commit, set or cancel.
#[derive(Default)]
pub struct LiveGradient {
    /// Pending edit from the Properties panel's or the Gradient Editor panel's gradient rows: the layer and the
    /// `gradient.fill.set` params (previewed like a canvas drag).
    panel: Option<(LayerId, Value)>,
    /// Pending edit from the Gradient Editor panel's stop bar over the tool gradient.
    preset: Option<craft::color_ui::ui::gradient::GradientStops>,
    /// Cached preview document by key.
    preview: Option<(u64, Arc<Document>)>,
}

/// Whether an uncommitted gradient gesture must finish before applying a saved vector.
pub(crate) fn editing(app: &PhotocraftApp) -> bool {
    app.canvas_tools.gradient.from.is_some() || app.gradient.panel.is_some() || app.gradient.preset.is_some()
}

/// Whether the Gradient tool is in its live (Gradient Fill layer) mode.
/// Painting into a layer mask, an alpha channel or the Quick Mask stays classic (pixels), except
/// that a selected gradient fill layer is always edited live (selecting it targets its mask), even
/// with Classic gradient chosen ([`fill_layer_active`]).
pub(crate) fn live_mode(app: &PhotocraftApp) -> bool {
    if app.ui.tool != Tool::Gradient || (app.ui.tool_options.gradient_classic && !fill_layer_active(app)) {
        return false;
    }
    match crate::canvas::paint_target(app).as_str() {
        Some("pixels") => true,
        Some("mask") => active_gradient(app).is_some(),
        _ => false,
    }
}

/// Whether the active layer is a shown, unlocked gradient fill layer the tool paints into (its
/// pixels or its mask). The Gradient tool then edits it live whatever the stored Gradient / Classic
/// gradient choice, and the options bar greys that choice out (#12008). Hidden and locked fill
/// layers (or ones in a hidden or locked group), and the Quick Mask or an alpha channel as the
/// target, were not measured and keep the choice.
pub(crate) fn fill_layer_active(app: &PhotocraftApp) -> bool {
    let Some((l, ..)) = active_gradient(app) else { return false };
    matches!(crate::canvas::paint_target(app).as_str(), Some("pixels" | "mask"))
        && app.session.active().is_some_and(|st| st.doc.path_of(l.id).is_some_and(|p| st.doc.shown_at(&p) && !st.doc.locks_at(&p).any()))
}

/// The active layer when it is a gradient fill layer: (layer, its frame, the canvas).
pub(crate) fn active_gradient(app: &PhotocraftApp) -> Option<(Layer, Rect32, photocraft_geom::Rect)> {
    let st = app.session.active()?;
    let l = st.doc.layer(st.active_layer?)?;
    if !matches!(l.content, LayerContent::Fill(Fill::Gradient { .. })) || !l.visible {
        return None;
    }
    let canvas = st.doc.bounds();
    Some((l.clone(), photocraft_compose::fill_frame(l, canvas), canvas))
}

type Rect32 = photocraft_geom::Rect;

pub(crate) fn fill_of(l: &Layer) -> Option<&Fill> {
    match &l.content {
        LayerContent::Fill(f @ Fill::Gradient { .. }) => Some(f),
        _ => None,
    }
}

/// The fill an in-progress edit shows (`None` when it can't apply, e.g. a stale index).
fn edited_fill(app: &PhotocraftApp, layer: &Layer, canvas: Rect32, cmd: &str, p: &Value) -> Option<Fill> {
    let f = fill_of(layer)?;
    let (fg, bg) = (app.session.tools.foreground, app.session.tools.background);
    let edited = match cmd {
        cmds::SET => cmds::apply_set(layer, f, canvas, p, fg, bg).ok(),
        _ => None,
    }?;
    // As the command commits it: colours in the document's mode.
    Some(match app.session.active() {
        Some(st) => edited.in_mode(st.doc.mode),
        None => edited,
    })
}

/// Options-bar params of a new live gradient. Its fill layer is Normal at 100 %: the options bar's
/// blend Mode and Opacity belong to the Classic gradient (#12007).
fn create_params(app: &PhotocraftApp, from: [f32; 2], to: [f32; 2]) -> Value {
    let o = &app.ui.tool_options;
    json!({"from": from, "to": to, "style": o.gradient_style, "reverse": o.gradient_reverse, "dither": o.gradient_dither})
}

fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// Opens the Color Picker on colour stop `i`; OK runs `gradient.fill.stop` (one history step).
pub(crate) fn edit_stop_color(app: &mut PhotocraftApp, layer: LayerId, i: usize) {
    let Some(st) = app.session.active() else { return };
    let Some(Fill::Gradient { stops, .. }) = st.doc.layer(layer).and_then(fill_of) else { return };
    let mut sorted = stops.clone();
    sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
    let Some((_, c)) = sorted.get(i) else { return };
    crate::color_picker_ui::open_for_command(
        app,
        "Color Picker (Stop Color)",
        c.to_rgb(),
        cmds::STOP,
        json!({"layer": layer.0, "action": "color", "index": i}),
    );
}

fn preview_key(revision: u64, what: &str) -> u64 {
    let h = what.bytes().fold(revision.wrapping_mul(0x9e37_79b9_7f4a_7c15), |h, b| h.wrapping_mul(31).wrapping_add(u64::from(b)));
    (h & (TAG - 1)) | TAG
}

/// The cached preview document of `key`, if it is the one kept.
fn cached_preview(app: &PhotocraftApp, key: u64) -> Option<(Arc<Document>, u64)> {
    match &app.gradient.preview {
        Some((k, d)) if *k == key => Some((d.clone(), key)),
        _ => None,
    }
}

/// Keeps `shown` as the preview of `key`.
fn keep_preview(app: &mut PhotocraftApp, key: u64, shown: Document) -> (Arc<Document>, u64) {
    let shown = Arc::new(shown);
    app.gradient.preview = Some((key, shown.clone()));
    (shown, key)
}

/// The document to show while a gradient is drawn or edited (`canvas::display_doc`). `live` is the fill the canvas body's
/// gradient widget proposes this pass (`live_overlay::show`'s answer, `None` when none does); it comes first, then the
/// drag drawing a gradient, then an edit from the Properties or Gradient Editor panel. Nothing is kept for the next pass but
/// the cached document of the last key.
pub fn display_doc(app: &mut PhotocraftApp, idx: usize, live: Option<&(LayerId, Fill)>) -> Option<(Arc<Document>, u64)> {
    if app.session.active_index() != Some(idx) {
        return None;
    }
    let st = app.session.active()?;
    let (doc, revision) = (st.doc.clone(), st.revision);
    if let Some((layer, fill)) = live
        && doc.layer(*layer).is_some()
    {
        let key = preview_key(revision, &format!("{}{fill:?}", layer.0));
        if let Some(cached) = cached_preview(app, key) {
            return Some(cached);
        }
        let mut shown = (*doc).clone();
        shown.layer_mut(*layer)?.content = LayerContent::Fill(fill.clone());
        return Some(keep_preview(app, key, shown));
    }
    let draw = &app.canvas_tools.gradient;
    let edit: Option<(LayerId, &'static str, Value)> = match draw.from {
        Some(from) if live_mode(app) && dist(from, draw.to) >= 1.0 => match draw.redraw {
            Some(id) => Some((id, cmds::SET, json!({"from": from, "to": draw.to}))),
            None => Some((PREVIEW_LAYER, cmds::CREATE, create_params(app, from, draw.to))),
        },
        _ => app.gradient.panel.clone().map(|(layer, p)| (layer, cmds::SET, p)),
    };
    let Some((layer, cmd, p)) = edit else {
        app.gradient.preview = None;
        return None;
    };
    let key = preview_key(revision, &format!("{}{cmd}{p}", layer.0));
    if let Some(cached) = cached_preview(app, key) {
        return Some(cached);
    }
    let mut shown = (*doc).clone();
    if cmd == cmds::CREATE {
        let mut l = cmds::new_layer(&app.session, &doc, &p).ok()?;
        l.id = PREVIEW_LAYER;
        // Where the committed layer will go, so the preview doesn't jump on release.
        let active = app.session.active().and_then(|s| s.active_layer);
        photocraft_engine::layer_place::insert_new(&mut shown, active, l, photocraft_engine::layer_place::Place::AutoEditable);
    } else {
        let l = doc.layer(layer)?;
        let f = edited_fill(app, l, doc.bounds(), cmd, &p)?;
        shown.layer_mut(layer)?.content = LayerContent::Fill(f);
    }
    Some(keep_preview(app, key, shown))
}

/// Pixels of a gradient fill's Layers-panel thumbnail, `side` pixels square: the fill as it lays out (style, angle,
/// scale, offset, reverse, stops, midpoints and opacity) in a square, as Photoshop shows it.
/// `None` for other fills.
pub fn thumbnail_image(f: &Fill, side: u32) -> Option<egui::ColorImage> {
    let Fill::Gradient { .. } = f else { return None };
    let n = side as i32;
    let square = photocraft_geom::Rect::new(0, 0, n, n);
    // No dither: it is invisible at this size and would only add noise.
    let mut plain = f.clone();
    if let Fill::Gradient { dither, .. } = &mut plain {
        *dither = false;
    }
    let px = gf::render(&plain, square, square);
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let rgba: Vec<Color32> = px.iter().map(|c| Color32::from_rgba_unmultiplied(q(c[0]), q(c[1]), q(c[2]), q(c[3]))).collect();
    Some(egui::ColorImage::new([side as usize; 2], rgba))
}

/// The selected layer when it is a gradient fill layer. Unlike [`active_gradient`] it does not
/// require the layer to be visible: the editor edits it either way.
fn selected_gradient(app: &PhotocraftApp) -> Option<Layer> {
    let st = app.session.active()?;
    let l = st.doc.layer(st.active_layer?)?;
    fill_of(l)?;
    Some(l.clone())
}

/// Live mode: style, reverse and dither changes in the options bar also edit the selected gradient
/// fill layer (one history step each), as in Photoshop. Its blend mode and opacity are the Layers
/// panel's: the live bar has no Mode or Opacity (#12007).
pub fn options_changed(app: &mut PhotocraftApp, before: &crate::state::ToolOptions) {
    let o = app.ui.tool_options.clone();
    // A gradient fill layer is edited live whatever the stored choice (#12008).
    if app.ui.tool != Tool::Gradient || (o.gradient_classic && !fill_layer_active(app)) {
        return;
    }
    let Some((l, ..)) = active_gradient(app) else { return };
    if o.gradient_style == before.gradient_style && o.gradient_reverse == before.gradient_reverse && o.gradient_dither == before.gradient_dither {
        return;
    }
    let mut p = json!({"layer": l.id.0});
    if o.gradient_style != before.gradient_style {
        p["style"] = json!(o.gradient_style);
    }
    if o.gradient_reverse != before.gradient_reverse {
        p["reverse"] = json!(o.gradient_reverse);
    }
    if o.gradient_dither != before.gradient_dither {
        p["dither"] = json!(o.gradient_dither);
    }
    let _ = app.run(cmds::SET, p);
}

/// Properties panel for a live gradient. Every change uses the existing gradient commands.
pub fn properties(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: &Layer) {
    properties_panel::show(app, ui, layer);
}
