//! Paragraph Text Size: Alt+drag with the Type tool sizes the new paragraph box in a modal dialog
//! instead of committing the drag (#12044). Width and Height open in document pixels with the size
//! that was dragged, decimals kept (a ~100 px drag prefills 99.5), and the box keeps its top-left
//! where the drag started. OK runs the same `type.create` a plain drag does, so the placeholder
//! text lands selected exactly as it does there; Cancel closes the dialog and creates nothing.
//!
//! Only what the ticket measured is here. No field rule was measured — no unit menu, no minimum,
//! no maximum — so the number goes to `type.create` as typed and the engine refuses a box it
//! cannot make. A drag upwards or to the left of its start keeps today's rule (the box is the
//! rectangle between the two points), and Cancel beyond closing the dialog is not specified.
//!
//! The dialog is an ordinary one in the UI state, so `ui.inspect`, `ui.dialog.confirm` and
//! `ui.dialog.cancel` drive it, and its two fields live in the dialog's field map.

use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::state::DialogKind;

const MARK: &str = "__paragraphTextSize";
const COMMAND: &str = "type.create";
/// The top-left the box was dragged from, in document pixels: OK puts the box there, whatever its
/// size becomes in the dialog.
const AT: &str = "__at";
/// The document the drag was made in (its id): OK creates the text only while it is still the active one.
const DOCUMENT: &str = "__document";

/// Identify the Paragraph Text Size dialog fields.
pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

/// Open the dialog on the box an Alt+drag drew: top-left `(x, y)`, size `w` × `h`, in document
/// pixels and exactly as dragged (the fields keep the decimals).
pub fn open(app: &mut PhotocraftApp, at: [f64; 2], w: f64, h: f64) -> u64 {
    let mut f = Map::new();
    f.insert(MARK.into(), json!(true));
    f.insert("__command".into(), json!(COMMAND));
    f.insert("__label".into(), json!("Paragraph Text Size"));
    f.insert(AT.into(), json!([at[0], at[1]]));
    if let Some(st) = app.session.active() {
        f.insert(DOCUMENT.into(), json!(st.doc.id.0));
    }
    f.insert("width".into(), json!(w));
    f.insert("height".into(), json!(h));
    app.ui.open_dialog(DialogKind::Command, f)
}

/// `f[key]` as a document-pixel length.
fn px(f: &Map<String, Value>, key: &str) -> Option<f64> {
    f.get(key).and_then(Value::as_f64).filter(|v| v.is_finite())
}

/// The two rows, Width and Height, in document pixels.
pub fn body(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    egui::Grid::new("paragraph-text-size-dialog").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
        for (key, label) in [("width", tl!("Width")), ("height", tl!("Height"))] {
            ui.label(label);
            let mut px = px_or(f, key);
            // One decimal shows a dragged half pixel (99.5) as such; no range is measured, so the
            // field takes the number and `type.create` refuses a box it cannot make.
            let field = crate::widgets::value_field_prec(ui, &mut px, -1.0e9..=1.0e9, "px", 110.0, 1).accessible_name(label);
            if field.changed() {
                f.insert(key.into(), json!(f64::from(px)));
            }
            ui.end_row();
        }
    });
}

/// The field's current length, as the widget sees it (a length the dialog never held shows 0).
fn px_or(f: &Map<String, Value>, key: &str) -> f32 {
    px(f, key).map_or(0.0, |v| v as f32)
}

/// OK: create the paragraph text the dialog sized, the same layer a plain drag of the same box
/// would have made.
pub fn confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    // The box was dragged in one document: never put the text into another one that became active since.
    if f.get(DOCUMENT).and_then(Value::as_u64) != app.session.active().map(|st| st.doc.id.0) {
        return Err("the document the box was dragged in is no longer the active one".into());
    }
    let at = f.get(AT).and_then(Value::as_array);
    let (Some(x), Some(y)) = (at.as_ref().and_then(|a| a.first()).and_then(Value::as_f64), at.as_ref().and_then(|a| a.get(1)).and_then(Value::as_f64)) else {
        return Err("the Paragraph Text Size dialog has no position".into());
    };
    let (Some(w), Some(h)) = (px(f, "width"), px(f, "height")) else {
        return Err("the Paragraph Text Size dialog has no size".into());
    };
    let mut p = crate::type_tool::create_params(app);
    p["box"] = json!([x, y, w, h]);
    crate::type_tool::create_layer(app, p, None).map(|layer| json!({"layer": layer}))
}

#[cfg(test)]
mod tests;
