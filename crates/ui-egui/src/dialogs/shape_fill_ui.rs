//! The shared Color Picker on a shape's paint, like Photoshop (#2812): a solid-colour shape
//! layer's thumbnail (or Layer › Layer Content Options…) opens it on the fill, and the Fill and
//! Stroke popovers' Color Picker button ([`crate::paint_picker`]) on the fill or the stroke of the
//! selected shape and/or the next shape's colours. The canvas previews a shallow document
//! snapshot; Cancel changes nothing and OK commits exactly one `shape.edit`.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use craft::color_ui::ui::{ColorPickerDialog, PickedColor};
use craft::theme::Srgba;
use craft::ui::dialog::DialogId;
use photocraft_color::Color;
use photocraft_doc::{DocId, Document, Fill, LayerContent, LayerId};
use serde_json::{Value, json};

use crate::color_picker_ui::{PickTarget, hex, rgb_hex};
use crate::{PhotocraftApp, color_picker_ui};

const COMMAND: &str = "shape.edit";

pub(crate) struct Preview {
    document: DocId,
    revision: u64,
    dialog: DialogId,
    layer: LayerId,
    color: Color,
    shown: Arc<Document>,
    key: u64,
}

/// Which of a shape's paints the picker edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Paint {
    Fill,
    Stroke,
}

impl Paint {
    fn key(self) -> &'static str {
        match self {
            Paint::Fill => "fill",
            Paint::Stroke => "stroke",
        }
    }
}

/// What a shape-paint picker edits.
#[derive(Clone, Debug)]
pub(crate) struct ShapeTarget {
    /// The paint.
    pub(crate) which: Paint,
    /// The shape layer; `None` for the next shape's colours only.
    pub(crate) layer: Option<LayerId>,
    /// The document of `layer`.
    pub(crate) document: Option<DocId>,
    /// Also set the next shape's colour (the options bar's popover).
    pub(crate) tool: bool,
    /// Opened from a layer thumbnail: edits a solid fill only.
    pub(crate) thumbnail: bool,
    /// The alpha a pick keeps.
    pub(crate) alpha: f32,
    /// The paint was solid when the picker opened: OK on its colour changes nothing. A paint that is not
    /// solid yet changes on OK even with the colour it opened on.
    pub(crate) solid: bool,
}

/// The paint `which` of shape layer `layer`: `Some(None)` when the shape has none (No Color),
/// `None` when the layer is gone or not a shape.
fn paint_of(doc: &Document, layer: LayerId, which: Paint) -> Option<Option<Fill>> {
    match &doc.layer(layer)?.content {
        LayerContent::Shape(sh) => Some(match which {
            Paint::Fill => sh.fill.clone(),
            Paint::Stroke => sh.stroke.as_ref().filter(|s| s.width > 0.0).map(|s| s.paint.clone()),
        }),
        _ => None,
    }
}

/// The solid fill colour of shape layer `layer`, if it has one (not a gradient, pattern or none).
fn solid_fill(doc: &Document, layer: LayerId) -> Option<Color> {
    match paint_of(doc, layer, Paint::Fill)? {
        Some(Fill::Solid(c)) => Some(c),
        _ => None,
    }
}

fn clamp_alpha(alpha: f32) -> f32 {
    if alpha.is_finite() { alpha.clamp(0.0, 1.0) } else { 1.0 }
}

/// Opens the picker for the active layer when it is a shape filled with a solid colour.
pub(crate) fn open(app: &mut PhotocraftApp) -> Option<DialogId> {
    let st = app.session.active()?;
    let layer = st.active_layer?;
    let color = solid_fill(&st.doc, layer)?;
    let target =
        ShapeTarget { which: Paint::Fill, layer: Some(layer), document: Some(st.doc.id), tool: false, thumbnail: true, alpha: color.alpha, solid: true };
    let start = Srgba::from_rgb_array(color.to_rgb());
    Some(color_picker_ui::open_for(app, "Color Picker (Solid Color)", start, None, PickTarget::Shape(target)))
}

/// Opens the picker from a Fill or Stroke popover on `which` paint of shape `layer` (the
/// selected shape), and of the next shape's colours too when `tool` (the options bar). It opens
/// on the paint's solid colour, else on `fallback`, whose alpha a pick keeps. OK makes the paint
/// that colour, also from No Color or a gradient.
pub(crate) fn open_paint(app: &mut PhotocraftApp, which: Paint, layer: Option<LayerId>, tool: bool, fallback: [f32; 4]) -> Option<DialogId> {
    if layer.is_none() && !tool {
        return None;
    }
    let mut start = Color::rgba(fallback[0], fallback[1], fallback[2], fallback[3]);
    let mut document = None;
    let mut solid = false;
    if let Some(layer) = layer {
        let st = app.session.active()?;
        if let Some(Fill::Solid(c)) = paint_of(&st.doc, layer, which)? {
            start = c;
            solid = true;
        }
        document = Some(st.doc.id);
    }
    let target = ShapeTarget { which, layer, document, tool, thumbnail: false, alpha: clamp_alpha(start.alpha), solid: solid || layer.is_none() };
    let label = match which {
        Paint::Fill => "Color Picker (Fill Color)",
        Paint::Stroke => "Color Picker (Stroke Color)",
    };
    Some(color_picker_ui::open_for(app, label, Srgba::from_rgb_array(start.to_rgb()), None, PickTarget::Shape(target)))
}

/// The picker's colour as `shape.edit` stores it: RGB with the original paint's alpha.
fn picked(t: &ShapeTarget, p: &PickedColor) -> Color {
    let [r, g, b] = p.color.to_rgb_array();
    Color::rgba(r, g, b, clamp_alpha(t.alpha))
}

/// The target layer, unless the dialog has none or is stale (another document is active, or the
/// layer is gone or no longer a shape). `Ok(None)`: only the next shape's colours, which need no
/// document (`idx` is the active one's).
fn target(app: &PhotocraftApp, t: &ShapeTarget, idx: Option<usize>) -> Result<Option<LayerId>, String> {
    let Some(layer) = t.layer else {
        return if t.tool { Ok(None) } else { Err("missing shape layer".into()) };
    };
    let st = idx.and_then(|i| app.session.documents().get(i)).ok_or("no document open")?;
    if t.document != Some(st.doc.id) {
        return Err("the target document is no longer active".into());
    }
    let paint = paint_of(&st.doc, layer, t.which).ok_or("the target layer is no longer a shape")?;
    // The thumbnail's picker edits a solid fill only, as it opened on one.
    if t.thumbnail && !matches!(paint, Some(Fill::Solid(_))) {
        return Err("the target shape layer no longer has a solid colour fill".into());
    }
    Ok(Some(layer))
}

/// OK: one `shape.edit` with the new paint, then the next shape's colour when the picker came from
/// the options bar. A refused edit (a locked layer, a job running on the document) changes neither.
pub(crate) fn confirm(app: &mut PhotocraftApp, t: &ShapeTarget, picker: &PickedColor) -> Result<Value, String> {
    let layer = target(app, t, app.session.active_index())?;
    let color = picked(t, picker);
    let rgba = [color.c[0], color.c[1], color.c[2], color.alpha];
    // A locked shape refuses the edit: check first, so the next shape's colour isn't changed either.
    if let Some(layer) = layer
        && app.session.active().is_some_and(|st| {
            let locks = st.doc.effective_locks(layer);
            locks.all || locks.pixels
        })
    {
        return Err("shape layer is locked".into());
    }
    let result = match layer {
        None => json!({"color": rgba}),
        Some(layer) => edit_paint(app, t, layer, rgba)?,
    };
    // Only once the shape took the colour (or there is none): a refused edit changes nothing.
    if t.tool {
        crate::shape_options::set_tool_paint(app, t.which == Paint::Stroke, crate::shape_paint_ui::PaintChange::Color(rgba))?;
    }
    Ok(result)
}

/// The `shape.edit` that gives `layer`'s paint the colour `rgba`; an unchanged solid colour keeps the
/// stored one as it is.
fn edit_paint(app: &mut PhotocraftApp, t: &ShapeTarget, layer: LayerId, rgba: [f32; 4]) -> Result<Value, String> {
    let which = t.which;
    let rgb_now = hex([rgba[0], rgba[1], rgba[2]]);
    let same =
        app.session.active().and_then(|st| paint_of(&st.doc, layer, which)).is_some_and(|p| matches!(p, Some(Fill::Solid(c)) if hex(c.to_rgb()) == rgb_now));
    if t.solid && same {
        return Ok(json!({"layer": layer.0, "unchanged": true}));
    }
    let paint = match which {
        Paint::Fill => json!(rgba),
        Paint::Stroke => {
            let width = app
                .session
                .active()
                .and_then(|st| st.doc.layer(layer))
                .and_then(|l| match &l.content {
                    LayerContent::Shape(sh) => sh.stroke.as_ref().map(|s| s.width),
                    _ => None,
                })
                .unwrap_or(0.0);
            json!({"color": rgba, "width": crate::vector_ui::enabled_stroke_width(width)})
        }
    };
    let mut p = json!({"layer": layer.0});
    p[which.key()] = paint;
    app.run(COMMAND, p)
}

/// The canvas preview while the picker is open: the document with the shape re-rendered in the
/// picked colour, cached per dialog, revision and colour.
pub(crate) fn display_doc(app: &mut PhotocraftApp, idx: usize) -> Option<(Arc<Document>, u64)> {
    let Some((dialog, PickTarget::Shape(t))) = color_picker_ui::find(app, |t| matches!(t, PickTarget::Shape(_))) else {
        app.shape_fill_preview = None;
        return None;
    };
    let (current, opening) = {
        let d = app.framework.dialogs.get_by_id::<ColorPickerDialog>(dialog)?;
        (d.current()?, d.opening())
    };
    // OK on the opening colour changes a paint that isn't solid yet (No Color, a gradient), so
    // that one previews too.
    if t.solid && opening.is_some_and(|o| rgb_hex(o.color) == rgb_hex(current.color)) {
        return None;
    }
    let layer = target(app, &t, Some(idx)).ok()??;
    let color = picked(&t, &current);
    let which = t.which;
    let st = app.session.documents().get(idx)?;
    if let Some(p) = &app.shape_fill_preview
        && p.document == st.doc.id
        && p.revision == st.revision
        && p.dialog == dialog
        && p.layer == layer
        && p.color == color
    {
        return Some((p.shown.clone(), p.key));
    }
    let mut doc = (*st.doc).clone();
    if let LayerContent::Shape(sh) = &mut doc.layer_mut(layer)?.content {
        match which {
            Paint::Fill => sh.fill = Some(Fill::Solid(color)),
            // A stroke that is off (none, or 0 px) previews as OK makes it: on, at its enabled
            // width, with `shape.edit`'s default style when the shape has none.
            Paint::Stroke => {
                let stroke = sh.stroke.get_or_insert_with(Default::default);
                stroke.width = crate::vector_ui::enabled_stroke_width(stroke.width);
                stroke.paint = Fill::Solid(color);
            }
        }
        photocraft_engine::vector_cmds::refresh_shape(&st.doc, sh);
    }
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    (st.doc.id.0, st.revision, dialog, layer.0, which.key()).hash(&mut hash);
    for component in color.c.into_iter().chain([color.alpha]) {
        component.to_bits().hash(&mut hash);
    }
    let key = (1 << 46) | (hash.finish() & ((1 << 40) - 1));
    let shown = Arc::new(photocraft_engine::mode_cmds::display_document(&doc).unwrap_or(doc));
    app.shape_fill_preview = Some(Preview { document: st.doc.id, revision: st.revision, dialog, layer, color, shown: shown.clone(), key });
    Some((shown, key))
}

#[cfg(test)]
mod tests;
