//! Image › Adjustments › Equalize with a selection: the dialog that asks what to equalize, as in
//! Photoshop. Equalize Selected Area Only (the default) evenly distributes the selected pixels;
//! Equalize Entire Image Based On Selected Area maps the layer within the canvas by the selected
//! pixels' histogram. OK runs `image.adjustments.equalize` with `selectedAreaOnly`. Without a selection,
//! Equalize runs at once.
//!
//! The dialog is a hosted parameter dialog whose OK is held: [`check`] refuses it while the
//! document, layer or selection it was opened for is no longer the active one.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::env::Label;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::dialogs::params;

pub const COMMAND: &str = "image.adjustments.equalize";
const KEY: &str = "selectedAreaOnly";
const SELECTED: &str = "selected";
const ENTIRE: &str = "entire";

/// Whether `key` is the Equalize dialog's.
pub fn owns(key: &str) -> bool {
    key == COMMAND
}

/// Whether Equalize asks first: the active document has a selection and Equalize can run.
pub fn asks(app: &PhotocraftApp) -> bool {
    app.session.active().is_some_and(|st| st.doc.selection.is_some()) && app.session.is_enabled(COMMAND)
}

fn build() -> impl for<'b> Fn(&'b mut DialogFields) -> ParamDialog<'b> + Send + 'static {
    |_fields: &mut DialogFields| {
        let options = vec![
            (SELECTED.to_string(), tl!("Equalize Selected Area Only").into()),
            (ENTIRE.to_string(), tl!("Equalize Entire Image Based On Selected Area").into()),
        ];
        let rows = vec![Param::new(KEY, tl!("Equalize"), ParamKind::radio(options, Some(SELECTED.to_string())))];
        ParamDialog::new(COMMAND, tl!("Equalize"), rows)
    }
}

/// Opens the dialog, capturing the document, layer and selection it asks about.
pub fn open(app: &mut PhotocraftApp) -> DialogId {
    let st = app.session.active();
    let captured = json!({
        "document": st.map(|s| json!(s.doc.id)).unwrap_or(Value::Null),
        "layer": st.and_then(|s| s.active_layer).map(|l| json!(l.0)).unwrap_or(Value::Null),
        "selection": st.and_then(selection_fingerprint).map(Value::String).unwrap_or(Value::Null),
    });
    let fields = DialogFields::from_pairs([(KEY, FieldValue::Text(SELECTED.to_string()))]);

    params::open_param(app, COMMAND, fields, captured, build())
}

/// A hash of the selection's stored tiles and its default value, so OK can tell whether the
/// selection changed after the dialog opened. Tiles that share one allocation (uniform tiles,
/// copy-on-write clones) are hashed once.
fn selection_fingerprint(st: &photocraft_engine::DocState) -> Option<String> {
    let sel = st.doc.selection.as_ref()?;
    let mut h = DefaultHasher::new();
    // A point far outside every tile reads the default value.
    sel.sample_channel(i32::MIN / 2, i32::MIN / 2, 0).to_bits().hash(&mut h);
    let mut seen: HashMap<usize, u64> = HashMap::new();
    for (c, t) in sel.tiles() {
        let tile = *seen.entry(Arc::as_ptr(t) as usize).or_insert_with(|| {
            let mut th = DefaultHasher::new();
            t.bytes().hash(&mut th);
            th.finish()
        });
        (c.tx, c.ty, tile).hash(&mut h);
    }
    Some(format!("{:016x}", h.finish()))
}

/// The held OK's check: OK runs only while the document, layer and selection the dialog was opened
/// for are still the active ones; otherwise the dialog stays open with the reason.
pub fn check(app: &mut PhotocraftApp, result: &ParamResult) -> Result<(), Label> {
    let captured = params::captured(app, &result.key);
    let st = app.session.active().ok_or_else(|| Label::from("no document"))?;
    if captured.get("document") != Some(&json!(st.doc.id)) {
        return Err("the Equalize dialog belongs to another document".into());
    }
    if st.doc.selection.is_none() {
        return Err("the selection was cleared while the Equalize dialog was open".into());
    }
    if captured.get("layer") != Some(&st.active_layer.map(|l| json!(l.0)).unwrap_or(Value::Null)) {
        return Err("another layer was selected while the Equalize dialog was open".into());
    }
    if captured.get("selection") != Some(&selection_fingerprint(st).map(Value::String).unwrap_or(Value::Null)) {
        return Err("the selection changed while the Equalize dialog was open".into());
    }
    Ok(())
}

/// OK: equalizes with the chosen scope.
pub fn run(app: &mut PhotocraftApp, _ctx: &egui::Context, result: ParamResult) {
    let only = result.fields.text(KEY) != Some(ENTIRE);
    if let Err(e) = app.run(COMMAND, json!({ KEY: only })) {
        crate::dialogs::alerts::report_error(app, e);
    }
}

#[cfg(test)]
mod tests;
