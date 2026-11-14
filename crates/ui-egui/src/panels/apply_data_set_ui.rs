//! Image › Apply Data Set… (#14495): a small dialog with the document's data sets in a list and
//! Apply / Cancel beside it, as in the reference application. It only applies a set
//! (`image.applyDataSet`); the variables and data sets are edited in Image › Variables.
//!
//! The selected row is the dialog's `index` field, so the control channel picks a set with
//! `ui.dialog.set` and applies it with `ui.dialog.confirm`.
//!
//! Preview (#14496, the `preview` field) shows the selected set in the document: the canvas and
//! the Layers panel show the document as Apply would leave it, but no history state is made and
//! a clean document stays clean. Unticking it, Cancel, Escape and the close box put the document
//! back as it was. Apply puts it back first and then runs the command, so it leaves the same
//! document and the one history state Apply without Preview does. The box keeps its state for
//! the next open after Apply, and is off again after Cancel.

use std::sync::Arc;

use photocraft_doc::{DocId, Document};
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::state::DialogKind;
use crate::theme::Tokens;

const MARK: &str = "__applyDataSet";
const LIST: egui::Vec2 = egui::vec2(300.0, 240.0);
const ROW: f32 = 20.0;

/// The set Preview shows and the document as it was before.
pub(crate) struct Preview {
    doc: DocId,
    index: usize,
    original: Arc<Document>,
    /// The revision showing the preview left. Anything that moved it since changed the previewed
    /// document itself, so putting `original` back would undo that change: the preview is dropped.
    revision: u64,
    was_clean: bool,
}

/// Preview's state for the session (not saved, as the checkbox is per session).
#[derive(Default)]
pub(crate) struct PreviewState {
    shown: Option<Preview>,
    /// The preview that could not be built (document, revision, set), so it isn't retried each frame.
    failed: Option<(DocId, u64, usize)>,
    /// The checkbox on the next open: what Apply left, off after Cancel.
    remembered: bool,
}

/// This dialog owns a field set when it carries its marker.
pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

/// Open the dialog with the first set selected, as on every open in the reference application.
pub fn open(app: &mut PhotocraftApp) -> u64 {
    let mut f = Map::new();
    f.insert(MARK.into(), json!(true));
    f.insert("__command".into(), json!("image.applyDataSet"));
    f.insert("__label".into(), json!("Apply Data Set"));
    f.insert("index".into(), json!(0));
    f.insert("preview".into(), json!(app.data_set_preview.remembered));
    app.ui.open_dialog(DialogKind::Command, f)
}

/// The document's data set names, in document order.
fn names(app: &PhotocraftApp) -> Vec<String> {
    app.session.active().map(|st| st.doc.variables.data_sets.iter().map(|s| s.name.clone()).collect()).unwrap_or_default()
}

/// The list on the left, Apply and Cancel on the right. Returns `Some(true)` when a set is to be
/// applied (Apply, Enter, a double-click on a row) and `Some(false)` for Cancel.
pub fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>, interactive: bool) -> Option<bool> {
    let t = Tokens::get(ui.ctx());
    let sets = names(app);
    let selected = f.get("index").and_then(Value::as_u64);
    let mut outcome = None;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.label(tl!("Data Set:"));
            let (frame, _) = ui.allocate_exact_size(LIST, egui::Sense::hover());
            ui.painter().rect_filled(frame, t.radius_sm, t.field);
            ui.painter().rect_stroke(frame, t.radius_sm, egui::Stroke::new(1.0, t.field_border), egui::StrokeKind::Inside);
            let mut list = ui.new_child(egui::UiBuilder::new().max_rect(frame.shrink(1.0)));
            egui::ScrollArea::vertical().id_salt("apply-data-set-list").auto_shrink([false, false]).show(&mut list, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for (i, name) in sets.iter().enumerate() {
                    let on = selected == Some(i as u64);
                    let (rect, r) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW), egui::Sense::click());
                    r.widget_info(|| egui::WidgetInfo::selected(egui::Role::ListBoxOption, true, on, name));
                    if on {
                        ui.painter().rect_filled(rect, 0.0, t.row_selected);
                    } else if r.hovered() {
                        ui.painter().rect_filled(rect, 0.0, t.hover);
                    }
                    let text = ui.painter().layout_no_wrap(name.clone(), egui::FontId::proportional(12.0), t.text);
                    let text_rect = rect.shrink2(egui::vec2(6.0, 0.0));
                    ui.painter().with_clip_rect(text_rect).galley(egui::pos2(text_rect.left(), rect.center().y - text.size().y / 2.0), text, t.text);
                    // Any click selects only this row (Shift and Ctrl too); right-click shows no menu.
                    if r.clicked() || r.secondary_clicked() || r.double_clicked() {
                        f.insert("index".into(), json!(i));
                    }
                    if r.double_clicked() {
                        outcome = Some(true);
                    }
                }
            });
        });
        ui.add_space(12.0);
        ui.vertical(|ui| {
            ui.add_space(ui.text_style_height(&egui::TextStyle::Body) + ui.spacing().item_spacing.y);
            if crate::widgets::primary_button(ui, tl!("Apply"), 84.0).clicked() {
                outcome = Some(true);
            }
            ui.add_space(4.0);
            if crate::widgets::secondary_button(ui, tl!("Cancel"), 84.0).clicked() {
                outcome = Some(false);
            }
            ui.add_space(8.0);
            let mut preview = f.get("preview").and_then(Value::as_bool).unwrap_or(false);
            if crate::widgets::checkbox(ui, &mut preview, tl!("Preview")).on_hover_text(tl!("Preview the selected data set")).changed() {
                f.insert("preview".into(), json!(preview));
            }
        });
    });
    if outcome.is_none() && interactive && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        outcome = Some(true);
    }
    // Alt+P toggles Preview, its mnemonic in the reference application.
    if interactive && ui.input_mut(|i| i.consume_key(egui::Modifiers::ALT, egui::Key::P)) {
        let on = f.get("preview").and_then(Value::as_bool).unwrap_or(false);
        f.insert("preview".into(), json!(!on));
    }
    if outcome.is_none() {
        let wanted = f.get("preview").and_then(Value::as_bool).unwrap_or(false).then(|| f.get("index").and_then(Value::as_u64)).flatten();
        sync(app, wanted.and_then(|i| usize::try_from(i).ok()));
    }
    outcome
}

/// Show set `wanted` in the active document, or nothing. A set that can't be applied (a missing
/// or unreadable replacement file, an index past the list) shows nothing rather than part of it.
fn sync(app: &mut PhotocraftApp, wanted: Option<usize>) {
    let current = app.session.active().map(|st| (st.doc.id, st.revision));
    if let Some(p) = &app.data_set_preview.shown
        && current != Some((p.doc, p.revision))
    {
        app.data_set_preview.shown = None;
    }
    if app.data_set_preview.shown.as_ref().map(|p| p.index) == wanted {
        return;
    }
    revert(app);
    let Some(index) = wanted else { return };
    let Some(st) = app.session.active() else { return };
    let key = (st.doc.id, st.revision, index);
    if app.data_set_preview.failed == Some(key) {
        return;
    }
    let Ok(previewed) = photocraft_engine::data_set_preview(&app.session, index) else {
        app.data_set_preview.failed = Some(key);
        return;
    };
    let Some(st) = app.session.active_mut() else { return };
    let was_clean = !st.is_dirty();
    let original = std::mem::replace(&mut st.doc, Arc::new(previewed));
    shown(st, was_clean);
    app.data_set_preview.shown = Some(Preview { doc: original.id, index, original, revision: st.revision, was_clean });
}

/// A new revision for a document swapped in by Preview: everything is redrawn, a clean document
/// stays clean.
fn shown(st: &mut photocraft_engine::DocState, was_clean: bool) {
    st.revision += 1;
    if was_clean {
        st.saved_revision = st.revision;
    }
    st.last_damage = None;
}

/// Put the document back as it was before Preview showed a set.
fn revert(app: &mut PhotocraftApp) {
    let Some(p) = app.data_set_preview.shown.take() else { return };
    if let Some(st) = app.session.active_mut()
        && st.doc.id == p.doc
        && st.revision == p.revision
    {
        st.doc = p.original;
        shown(st, p.was_clean);
    }
}

/// Cancel, Escape and the close box: the preview goes, and the next open starts with it off.
pub fn cancel(app: &mut PhotocraftApp, f: &Map<String, Value>) {
    if owns(f) {
        revert(app);
        app.data_set_preview.remembered = false;
    }
}

/// Apply, Enter or a double-click: apply the selected set once. The variables and data sets
/// themselves are never written here.
pub fn confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    revert(app);
    app.data_set_preview.remembered = f.get("preview").and_then(Value::as_bool).unwrap_or(false);
    let index = f.get("index").and_then(Value::as_u64).ok_or_else(|| tl!("Select a data set to apply.").to_string())?;
    app.run("image.applyDataSet", json!({"index": index}))
}
