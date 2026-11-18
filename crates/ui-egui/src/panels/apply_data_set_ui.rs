//! Image › Apply Data Set… (#14495): a small dialog with the document's data sets in a list and
//! Apply / Cancel beside it, as in the reference application, hosted on the framework's dialog
//! host. It only applies a set (`image.applyDataSet`); the variables and data sets are edited in
//! Image › Variables.
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

use std::borrow::Cow;
use std::sync::Arc;

use craft::behavior::{DisplayOrder, ItemSource, Selection};
use craft::ui::contract::{Bind, CollectionEvent, SelectMode, Show as _, ShowCollection as _};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::Text;
use craft::ui::list::{List, ListRow};
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::runtime::env::Label;
use craft::ui::runtime::overlay::CloseReason;
use photocraft_doc::{DocId, Document};
use serde_json::{Value, json};

use crate::PhotocraftApp;

fn label(text: &str) -> Label {
    Label::from(text.to_string())
}

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

/// The data set names as the list's display order, keyed by position (names may repeat).
struct Names<'a>(&'a [String]);

impl DisplayOrder<usize> for Names<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.0.len()).then_some(*key)
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        self.0.get(at).map(|n| Cow::Borrowed(n.as_str()))
    }
}

impl ItemSource<usize> for Names<'_> {}

/// What Apply answers with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ApplyAnswer {
    /// The row to apply; `None` when the document has no data sets.
    pub index: Option<usize>,
    /// The Preview box, kept for the next open.
    pub preview: bool,
}

/// The Apply Data Set dialog: the document's set names, one of them selected, and Preview.
pub struct ApplyDataSetDialog {
    names: Vec<String>,
    selected: Selection<usize>,
    preview: bool,
}

impl ApplyDataSetDialog {
    /// The selected row, when it is one of the sets.
    fn index(&self) -> Option<usize> {
        self.selected.items.first().copied().filter(|i| *i < self.names.len())
    }

    /// The set Preview should show now.
    fn previewed(&self) -> Option<usize> {
        self.preview.then(|| self.index()).flatten()
    }
}

impl StackDialog for ApplyDataSetDialog {
    type Result = ApplyAnswer;

    fn id(&self) -> DialogId {
        dialog_id("photocraft.applyDataSet")
    }

    fn show(&mut self, ui: &mut egui::Ui, _stack: &DialogStack) -> Option<DialogResult<ApplyAnswer>> {
        let ApplyDataSetDialog { names, selected, preview } = self;
        let mut toggle = false;
        let shown = FormDialog::new(label(tl!("Apply Data Set")), Bind::bound(selected))
            .ok_label(label(tl!("Apply")))
            .button_place(craft::ui::dialog::ButtonPlace::Side)
            // Holding Alt keeps Cancel: the dialog has nothing to reset.
            .alt_resets(false)
            .preview(Some(Bind::bound(&mut *preview)))
            .show(ui, |ui, form, selected| {
                Text::new(label(tl!("Data Set:"))).show(ui);
                let list = List::<usize>::new(label(tl!("Data Set:")))
                    .select(SelectMode::Single)
                    .max_rows(12)
                    .selection(Bind::bound(&mut *selected))
                    .show_items(ui, &Names(names), &mut |key, _| ListRow::new(label(names.get(*key).map_or("", String::as_str))));
                // A double-click (or Enter on the list) applies the row it is on.
                for event in list.events {
                    if let CollectionEvent::Activate(key) = event {
                        *selected = Selection::single(key);
                        form.request_close_as(CloseReason::Activated);
                    }
                }
                // Alt+P toggles Preview, its mnemonic in the reference application.
                toggle = ui.input_mut(|i| i.consume_key(egui::Modifiers::ALT, egui::Key::P));
                true
            });
        if toggle {
            *preview = !*preview;
        }
        let answer = |items: &[usize]| ApplyAnswer { index: items.first().copied().filter(|i| *i < names.len()), preview: *preview };
        match (shown.accepted, shown.closed) {
            (Some(picked), _) => Some(DialogResult::Ok(answer(&picked.items))),
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }

    fn confirm(&mut self, _ctx: &egui::Context) -> Option<ApplyAnswer> {
        Some(ApplyAnswer { index: self.index(), preview: self.preview })
    }

    fn json_fields(&self) -> Option<Value> {
        Some(json!({"__command": "image.applyDataSet", "index": self.index(), "preview": self.preview, "dataSets": self.names}))
    }

    fn set_json_field(&mut self, ctx: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        match field {
            "index" => {
                // Checked against the document's sets here: an index past `usize` (wasm32) must
                // not wrap around to a valid row.
                let index = value
                    .as_u64()
                    .and_then(|i| usize::try_from(i).ok())
                    .filter(|i| *i < self.names.len())
                    .ok_or_else(|| format!("field `index` takes a row of the {} data sets, got {value}", self.names.len()))?;
                self.selected = Selection::single(index);
            }
            "preview" => self.preview = value.as_bool().ok_or_else(|| format!("field `preview` takes true or false, got {value}"))?,
            _ => return Err(format!("the Apply Data Set dialog has no field `{field}`")),
        }
        ctx.request_repaint();
        Ok(())
    }
}

/// The document's data set names, in document order.
fn names(app: &PhotocraftApp) -> Vec<String> {
    app.session.active().map(|st| st.doc.variables.data_sets.iter().map(|s| s.name.clone()).collect()).unwrap_or_default()
}

/// Opens the dialog with the first set selected, as on every open in the reference application,
/// and Preview as the last Apply left it.
pub fn open(app: &mut PhotocraftApp) -> DialogId {
    let names = names(app);
    let selected = if names.is_empty() { Selection::default() } else { Selection::single(0) };
    let preview = app.data_set_preview.remembered;
    app.framework.dialogs.open(ApplyDataSetDialog { names, selected, preview })
}

/// Show set `wanted` in the active document, or nothing. A set that can't be applied (a missing
/// or unreadable replacement file, an index past the list) shows nothing rather than part of it.
fn sync(app: &mut PhotocraftApp, wanted: Option<usize>) {
    let active = app.session.active().map(|st| st.doc.id);
    if app.data_set_preview.shown.as_ref().is_some_and(|p| Some(p.doc) == active && Some(p.index) == wanted) {
        return;
    }
    // Also when another document became active: the previewed one gets its own document back.
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

/// Put the document back as it was before Preview showed a set, whether it is active or not. A
/// document changed since keeps that change (and with it the preview).
fn revert(app: &mut PhotocraftApp) {
    let Some(p) = app.data_set_preview.shown.take() else { return };
    if let Some(st) = app.session.document_mut(p.doc)
        && st.revision == p.revision
    {
        st.doc = p.original;
        shown(st, p.was_clean);
    }
}

/// Before any command runs: the document goes back to itself, so the command neither works on
/// nor saves the preview. The dialog, still open with Preview on, shows the set again.
pub(crate) fn end_preview(app: &mut PhotocraftApp) {
    revert(app);
}

/// Whether Preview is showing a set (autosave waits for it to end).
pub(crate) fn previewing(app: &PhotocraftApp) -> bool {
    app.data_set_preview.shown.is_some()
}

/// The dialog's result pump (a `TAKERS` row), after every host pass. While the dialog is open it
/// keeps Preview in step with the box and the selected row. Apply, Enter or a double-click puts
/// the document back and applies the selected set once; Cancel, Escape and the close box put it
/// back and turn Preview off for the next open. The variables and data sets are never written here.
pub(crate) fn take(app: &mut PhotocraftApp, _ctx: &egui::Context) {
    match app.framework.dialogs.take::<ApplyDataSetDialog>() {
        Some(DialogResult::Ok(answer)) => {
            revert(app);
            app.data_set_preview.remembered = answer.preview;
            let outcome = match answer.index {
                Some(index) => app.run("image.applyDataSet", json!({"index": index})).map(|_| ()),
                None => Err(tl!("Select a data set to apply.").to_string()),
            };
            if let Err(error) = outcome {
                app.ui.status = error;
                app.ui.status_error = true;
            }
        }
        // Cancel, Escape, the close box, and any other ending the host may add: nothing applies.
        Some(_) => {
            revert(app);
            app.data_set_preview.remembered = false;
        }
        None => match app.framework.dialogs.get::<ApplyDataSetDialog>().map(ApplyDataSetDialog::previewed) {
            Some(wanted) => sync(app, wanted),
            None => revert(app),
        },
    }
}
