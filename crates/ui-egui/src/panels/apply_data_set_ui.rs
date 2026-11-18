//! Image › Apply Data Set… (#14495): a small dialog with the document's data sets in a list and
//! Apply / Cancel beside it, as in the reference application, hosted on the framework's dialog
//! host. It only applies a set (`image.applyDataSet`); the variables and data sets are edited in
//! Image › Variables.
//!
//! The selected row is the dialog's `index` field, so the control channel picks a set with
//! `ui.dialog.set` and applies it with `ui.dialog.confirm`.

use craft::behavior::{DisplayOrder, ItemSource, Selection};
use craft::ui::contract::{Bind, CollectionEvent, SelectMode, Show as _, ShowCollection as _};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::Text;
use craft::ui::list::{List, ListRow};
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::runtime::env::Label;
use craft::ui::runtime::overlay::CloseReason;
use serde_json::{Value, json};
use std::borrow::Cow;

use crate::PhotocraftApp;

fn label(text: &str) -> Label {
    Label::from(text.to_string())
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

/// The Apply Data Set dialog: the document's set names, one of them selected.
pub struct ApplyDataSetDialog {
    names: Vec<String>,
    selected: Selection<usize>,
}

impl ApplyDataSetDialog {
    /// The selected row, when it is one of the sets.
    fn index(&self) -> Option<usize> {
        self.selected.items.first().copied().filter(|i| *i < self.names.len())
    }
}

impl StackDialog for ApplyDataSetDialog {
    /// The row to apply; `None` when the document has no data sets.
    type Result = Option<usize>;

    fn id(&self) -> DialogId {
        dialog_id("photocraft.applyDataSet")
    }

    fn show(&mut self, ui: &mut egui::Ui, _stack: &DialogStack) -> Option<DialogResult<Option<usize>>> {
        let ApplyDataSetDialog { names, selected } = self;
        let shown = FormDialog::new(label(tl!("Apply Data Set")), Bind::bound(selected))
            .ok_label(label(tl!("Apply")))
            .button_place(craft::ui::dialog::ButtonPlace::Side)
            // Holding Alt keeps Cancel: the dialog has nothing to reset.
            .alt_resets(false)
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
                true
            });
        match (shown.accepted, shown.closed) {
            (Some(picked), _) => Some(DialogResult::Ok(picked.items.first().copied().filter(|i| *i < names.len()))),
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }

    fn confirm(&mut self, _ctx: &egui::Context) -> Option<Option<usize>> {
        Some(self.index())
    }

    fn json_fields(&self) -> Option<Value> {
        Some(json!({"__command": "image.applyDataSet", "index": self.index(), "dataSets": self.names}))
    }

    fn set_json_field(&mut self, ctx: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        if field != "index" {
            return Err(format!("the Apply Data Set dialog has no field `{field}`"));
        }
        // Checked against the document's sets here: an index past `usize` (wasm32) must not wrap
        // around to a valid row.
        let index = value
            .as_u64()
            .and_then(|i| usize::try_from(i).ok())
            .filter(|i| *i < self.names.len())
            .ok_or_else(|| format!("field `index` takes a row of the {} data sets, got {value}", self.names.len()))?;
        self.selected = Selection::single(index);
        ctx.request_repaint();
        Ok(())
    }
}

/// The document's data set names, in document order.
fn names(app: &PhotocraftApp) -> Vec<String> {
    app.session.active().map(|st| st.doc.variables.data_sets.iter().map(|s| s.name.clone()).collect()).unwrap_or_default()
}

/// Opens the dialog with the first set selected, as on every open in the reference application.
pub fn open(app: &mut PhotocraftApp) -> DialogId {
    let names = names(app);
    let selected = if names.is_empty() { Selection::default() } else { Selection::single(0) };
    app.framework.dialogs.open(ApplyDataSetDialog { names, selected })
}

/// The dialog's result pump (a `TAKERS` row): Apply, Enter or a double-click applies the
/// selected set once. The variables and data sets themselves are never written here.
pub(crate) fn take(app: &mut PhotocraftApp, _ctx: &egui::Context) {
    let Some(DialogResult::Ok(index)) = app.framework.dialogs.take::<ApplyDataSetDialog>() else { return };
    let outcome = match index {
        Some(index) => app.run("image.applyDataSet", json!({"index": index})).map(|_| ()),
        None => Err(tl!("Select a data set to apply.").to_string()),
    };
    if let Err(error) = outcome {
        app.ui.status = error;
        app.ui.status_error = true;
    }
}
