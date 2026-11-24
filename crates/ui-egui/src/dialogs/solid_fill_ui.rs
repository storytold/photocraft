//! The Solid Color Fill thumbnail opens the shared Color Picker. Preview a shallow document
//! snapshot; Cancel changes nothing and OK commits exactly one engine command.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use craft::color_ui::ui::{ColorPickerDialog, PickedColor};
use craft::theme::Srgba;
use craft::ui::dialog::DialogId;
use photocraft_color::Color;
use photocraft_doc::{DocId, Document, Fill, LayerContent, LayerId};
use photocraft_engine::solid_fill_cmds as cmds;
use serde_json::{Value, json};

use crate::color_picker_ui::{PickTarget, hex, inks_model, inks_of, rgb_hex, same_inks};
use crate::{PhotocraftApp, color_picker_ui};

pub(crate) struct Preview {
    document: DocId,
    revision: u64,
    dialog: DialogId,
    /// Which open of the picker the preview belongs to (`DialogHost::open_serial`).
    open: u64,
    layer: LayerId,
    color: Color,
    shown: Arc<Document>,
    key: u64,
}

pub(crate) fn open(app: &mut PhotocraftApp) -> Option<DialogId> {
    let st = app.session.active()?;
    let layer = st.active_layer?;
    let LayerContent::Fill(Fill::Solid(color)) = st.doc.layer(layer)?.content else { return None };
    let document = st.doc.id;
    // A CMYK fill reopens showing its own inks, not ones recomputed from its sRGB stand-in (#8568).
    let kept = (color.mode == photocraft_color::ColorMode::Cmyk).then(|| inks_model(color.c));
    let start = Srgba::from_rgb_array(color.to_rgb());
    Some(color_picker_ui::open_for(app, "Color Picker (Solid Color)", start, kept, PickTarget::SolidFill { document, layer }))
}

/// Creating a Solid Color fill is transactional: the document is unchanged until OK.
/// The picker is seeded with the foreground colour, matching the engine's default.
pub(crate) fn open_new(app: &mut PhotocraftApp) -> Option<DialogId> {
    let document = app.session.active()?.doc.id;
    let foreground = app.session.tools.foreground;
    let rgb = [foreground[0], foreground[1], foreground[2]];
    // A foreground picked as CMYK opens showing those inks, as its own picker does, so OK keeps
    // them even when the foreground is finer than the picker's hex (#8568).
    let kept = app.session.tools.picked_cmyk(rgb).map(inks_model);
    Some(color_picker_ui::open_for(app, "Color Picker (Solid Color)", Srgba::from_rgb_array(rgb), kept, PickTarget::NewSolidFill { document }))
}

pub(crate) fn confirm_new(app: &mut PhotocraftApp, document: DocId, picked: &PickedColor) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document open")?;
    if st.doc.id != document {
        return Err("the target document is no longer active".into());
    }
    // CMYK typed into the picker stays those inks in a CMYK document (#8568).
    app.run("layer.newFillLayer.solidColor", json!({"color": rgb_hex(picked.color), "cmyk": inks_of(picked)}))
}

/// The colour the fill `layer` shows: the open picker's pick while one edits it.
pub(crate) fn thumbnail_color(app: &PhotocraftApp, document: DocId, layer: LayerId, original: Color) -> Color {
    let found = color_picker_ui::find(app, |t| matches!(t, PickTarget::SolidFill { document: d, layer: l } if *d == document && *l == layer));
    let Some((id, _)) = found else { return original };
    let Some(mode) = app.session.documents().iter().find(|st| st.doc.id == document).map(|st| st.doc.mode) else { return original };
    let Some(current) = app.framework.dialogs.get_by_id::<ColorPickerDialog>(id).and_then(|d| d.current()) else { return original };
    picked_color(&current, original, mode).unwrap_or(original)
}

/// The colour the picker would commit to a fill of `original` in a `mode` document, built as the
/// engine builds it: the picked hex, and in a CMYK document the inks it was picked as (#8568).
/// `None` while the picker still shows the fill as it is ([`is_edit`]).
fn picked_color(picked: &PickedColor, original: Color, mode: photocraft_color::ColorMode) -> Option<Color> {
    if !is_edit(picked, original, mode) {
        return None;
    }
    let color = cmds::color_param(&json!(rgb_hex(picked.color)), original, mode).ok()?;
    Some(inks_of(picked).filter(|_| mode == photocraft_color::ColorMode::Cmyk).map_or(color, |c| Color {
        mode: photocraft_color::ColorMode::Cmyk,
        c,
        alpha: color.alpha,
    }))
}

/// Whether the picker differs from the fill it edits, inks included: different inks can share a
/// display colour (CMYK 0/0/0/100 and 50/50/50/100 both read black), so the hex alone doesn't
/// tell, and an RGB or Lab fill given inks in a CMYK document is an edit too.
fn is_edit(picked: &PickedColor, original: Color, mode: photocraft_color::ColorMode) -> bool {
    let cmyk = photocraft_color::ColorMode::Cmyk;
    let inks_differ = mode == cmyk && inks_of(picked).is_some_and(|k| original.mode != cmyk || !same_inks(original.c, k));
    rgb_hex(picked.color) != hex(original.to_rgb()) || inks_differ
}

/// No-op OK preserves imported high-precision / non-RGB colour values rather than quantising
/// them to the picker's display hex. A stale document must never receive the edit.
pub(crate) fn confirm(app: &mut PhotocraftApp, document: DocId, layer: LayerId, picked: &PickedColor) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document open")?;
    if st.doc.id != document {
        return Err("the target document is no longer active".into());
    }
    let Some(LayerContent::Fill(Fill::Solid(original))) = st.doc.layer(layer).map(|l| &l.content) else {
        return Err("the target Solid Color Fill layer no longer exists".into());
    };
    if !is_edit(picked, *original, st.doc.mode) {
        return Ok(json!({"layer": layer.0, "unchanged": true}));
    }
    let mut params = json!({"layer": layer.0, "document": document.0, "color": rgb_hex(picked.color)});
    if let Some(inks) = inks_of(picked) {
        params["cmyk"] = json!(inks);
    }
    app.run(cmds::SET, params)
}

pub(crate) fn display_doc(app: &mut PhotocraftApp, idx: usize) -> Option<(Arc<Document>, u64)> {
    let Some((dialog, PickTarget::SolidFill { document, layer })) = color_picker_ui::find(app, |t| matches!(t, PickTarget::SolidFill { .. })) else {
        app.solid_fill_preview = None;
        return None;
    };
    let st = app.session.documents().get(idx)?;
    if document != st.doc.id {
        return None;
    }
    let LayerContent::Fill(Fill::Solid(original)) = st.doc.layer(layer)?.content else { return None };
    let current = app.framework.dialogs.get_by_id::<ColorPickerDialog>(dialog)?.current()?;
    let open = app.framework.dialogs.open_serial(dialog)?;
    let color = picked_color(&current, original, st.doc.mode)?;
    if let Some(p) = &app.solid_fill_preview
        && p.document == st.doc.id
        && p.revision == st.revision
        && p.dialog == dialog
        && p.open == open
        && p.layer == layer
        && p.color == color
    {
        return Some((p.shown.clone(), p.key));
    }
    let mut doc = (*st.doc).clone();
    doc.layer_mut(layer)?.content = LayerContent::Fill(Fill::Solid(color));
    // Include the dialog identity and the open it belongs to: cancelling and reopening at the same document revision
    // must not reuse an old preview's pixels in the canvas caches.
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    (st.doc.id.0, st.revision, dialog, open, layer.0).hash(&mut hash);
    color.mode.hash(&mut hash);
    for component in color.c.into_iter().chain([color.alpha]) {
        component.to_bits().hash(&mut hash);
    }
    let key = (1 << 36) | (hash.finish() & ((1 << 36) - 1));
    let shown = Arc::new(photocraft_engine::mode_cmds::display_document(&doc).unwrap_or(doc));
    app.solid_fill_preview = Some(Preview { document: st.doc.id, revision: st.revision, dialog, open, layer, color, shown: shown.clone(), key });
    Some((shown, key))
}

#[cfg(test)]
mod tests;
