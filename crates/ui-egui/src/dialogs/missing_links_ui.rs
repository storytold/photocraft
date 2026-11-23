//! Cannot Locate Linked Assets (#14122): a document opened with linked smart objects whose files
//! are not there lists those files, as Photoshop does on open. Each row is a Relink… button, a
//! chain and the file's absolute path; the footer says how to relink to a Library graphic; OK and
//! Cancel close it.
//!
//! - OK opens the document as it was read: the links are kept and the layers show their last
//!   pixels. Nothing is added to the History.
//! - Relink… asks for a file and relinks every layer linked to the missing file to it with Layer ›
//!   Smart Objects › Relink to File (`layer.smartObjects.relinkToFile`, one History state per
//!   smart object), then the row leaves the list.
//! - Cancel is not measured in the reference yet: it does what OK does.
//!
//! Only an interactive open asks ([`crate::jobs_ui::after_open`]), and it waits at most half a
//! second for the files to be looked at, on a worker thread. An automation open never shows
//! the dialog and lists the files in its `warnings` instead
//! ([`photocraft_engine::link_watch::missing_file_warnings`]).
//!
//! The dialog is hosted on the framework's dialog host, one per opened document, so agents see it
//! in `ui.inspect` (its `document`, `name` and `files`) and close it with `ui.dialog.confirm` /
//! `ui.dialog.cancel`.

use std::collections::HashSet;

use craft::behavior::IconId;
use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Show as _};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::{Icon, Text};
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::primitives::IconSpec;
use craft::ui::runtime::env::Label;
use photocraft_doc::{DocId, LayerContent, LayerId, SmartSource};
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// The dialog's key; each open document's dialog is its own instance of it.
const KEY: &str = "photocraft.missingLinks";

/// How long an open waits for its linked files to be looked at. Local files answer at once; a
/// share that doesn't answer in time doesn't hold the interface up, and the document opens without
/// the dialog (the link watcher keeps looking, and the Layers panel's Missing filter lists them).
const LOOK_WAIT: std::time::Duration = std::time::Duration::from_millis(500);

fn label(text: &str) -> Label {
    Label::from(text.to_string())
}

/// One missing file the dialog lists: its absolute path and the stored link paths that name it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MissingRow {
    /// The absolute path the links resolve to.
    pub(crate) file: String,
    /// The paths the layers keep for it (sorted).
    pub(crate) paths: Vec<String>,
}

/// Cannot Locate Linked Assets for one opened document.
pub struct MissingLinksDialog {
    document: DocId,
    name: String,
    files: Vec<MissingRow>,
    /// The row whose Relink… was pressed this pass, for [`take`] to ask for a file.
    relink: Option<String>,
}

#[cfg(test)]
impl MissingLinksDialog {
    /// The missing files the dialog still lists.
    pub(crate) fn files(&self) -> &[MissingRow] {
        &self.files
    }

    /// The document the dialog is about.
    pub(crate) fn document(&self) -> DocId {
        self.document
    }
}

/// The dialog id of document `document`'s dialog.
fn id_of(document: DocId) -> DialogId {
    dialog_id(format!("{KEY}.{}", document.0))
}

impl StackDialog for MissingLinksDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        id_of(self.document)
    }

    fn instance(&self) -> Option<DialogId> {
        Some(id_of(self.document))
    }

    fn show(&mut self, ui: &mut egui::Ui, _stack: &DialogStack) -> Option<DialogResult<()>> {
        let MissingLinksDialog { files, relink, .. } = self;
        let shown = FormDialog::new(label("PhotoCraft"), Bind::bound(files)).alt_resets(false).show(ui, |ui, _form, files| {
            Text::new(label(tl!("Cannot locate linked assets:"))).show(ui);
            ui.add_space(6.0);
            egui::ScrollArea::vertical().max_height(220.0).auto_shrink([false, true]).show(ui, |ui| {
                for row in files.iter() {
                    ui.horizontal(|ui| {
                        // The picker answers on a later pass; `take` starts it.
                        if Button::new(label(tl!("Relink…"))).show(ui).clicked() {
                            *relink = Some(row.file.clone());
                        }
                        Icon::new(IconSpec::svg(IconId::new("link"))).show(ui);
                        Text::new(label(&row.file)).show(ui);
                    });
                }
            });
            ui.add_space(8.0);
            Text::new(label(tl!(
                "To relink to a Library graphic, close this dialog, select the layer and choose Layer › Smart Objects › Relink to Library Graphic."
            )))
            .show(ui);
            true
        });
        match (shown.accepted, shown.closed) {
            (Some(_), _) => Some(DialogResult::Ok(())),
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }

    fn confirm(&mut self, _ctx: &egui::Context) -> Option<()> {
        Some(())
    }

    fn json_fields(&self) -> Option<Value> {
        let files: Vec<Value> = self.files.iter().map(|row| json!({"file": row.file, "paths": row.paths})).collect();
        Some(json!({"document": self.document.0, "name": self.name, "files": files}))
    }
}

/// After document `document` opened interactively from `path`: shows the dialog when any of its
/// linked files is not there. A document whose links are all there, or that has none, opens without
/// it. Relative links are taken from `path` (else the document's own file).
pub(crate) fn ask(app: &mut PhotocraftApp, document: DocId, path: Option<&str>) {
    let Some(st) = app.session.documents().iter().find(|st| st.doc.id == document) else { return };
    let opened = path.or(st.path.as_deref()).map(std::path::Path::new);
    let Some(missing) = photocraft_engine::link_watch::missing_files_within(&st.doc, opened, LOOK_WAIT) else {
        log::warn!("{}: its linked files did not answer in time; opened without Cannot Locate Linked Assets", st.doc.name);
        return;
    };
    let files: Vec<MissingRow> = missing.into_iter().map(|(file, paths)| MissingRow { file: file.to_string_lossy().into_owned(), paths }).collect();
    if files.is_empty() {
        return;
    }
    let name = st.doc.name.clone();
    app.framework.dialogs.open(MissingLinksDialog { document, name, files, relink: None });
}

/// The open Cannot Locate Linked Assets dialogs.
pub(crate) fn open_ids(app: &PhotocraftApp) -> Vec<DialogId> {
    let host = &app.framework.dialogs;
    host.open_dialogs().into_iter().map(|open| open.id).filter(|id| host.get_by_id::<MissingLinksDialog>(*id).is_some()).collect()
}

/// The result pump (a `TAKERS` row): a pressed Relink… asks for a file. OK and Cancel leave the
/// document as it was read, so their answers only need taking.
pub(crate) fn take(app: &mut PhotocraftApp, _ctx: &egui::Context) {
    while app.framework.dialogs.take_keyed::<MissingLinksDialog>().is_some() {}
    for id in open_ids(app) {
        let Some(file) = app.framework.dialogs.get_mut_by_id::<MissingLinksDialog>(id).and_then(|d| d.relink.take()) else { continue };
        // One that can't open the picker now says why.
        if let Err(e) = relink(app, id, &file)
            && e != crate::file_dialog::CANCELLED
        {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
}

/// Relink…: asks for a file, then relinks the layers linked to `file` (an absolute path the dialog
/// lists) to it ([`relink_to`]).
pub(crate) fn relink(app: &mut PhotocraftApp, dialog: DialogId, file: &str) -> Result<Value, String> {
    let file = file.to_string();
    // A link needs a file on disk: the desktop picker answers its full path, the web picker only a
    // name and the bytes, which can't be linked to (as Layer › Smart Objects › Relink to File).
    app.pick_file_bytes(move |app, path, _| relink_to(app, dialog, &file, &path))
}

/// Relinks every layer of the dialog's document that is linked to `file` to `path` with Relink to
/// File, and takes `file` off the dialog's list. A smart object's other instances follow it, so
/// each set of instances is relinked once. Nothing leaves the list when a relink fails.
pub(crate) fn relink_to(app: &mut PhotocraftApp, dialog: DialogId, file: &str, path: &str) -> Result<Value, String> {
    if !std::path::Path::new(path).is_absolute() {
        return Err(format!("{path}: Relink to File needs a file on disk"));
    }
    let shown = app.framework.dialogs.get_by_id::<MissingLinksDialog>(dialog).ok_or("the dialog was closed")?;
    let document = shown.document;
    let stored: HashSet<String> = shown.files.iter().find(|row| row.file == file).map(|row| row.paths.iter().cloned().collect()).unwrap_or_default();
    if stored.is_empty() {
        return Err(format!("{file} is not listed"));
    }
    let relinked = app.with_document(document, |app| {
        let st = app.session.active().ok_or("no document")?;
        let layers: Vec<LayerId> = st
            .doc
            .walk()
            .into_iter()
            .filter(|(_, _, l)| matches!(&l.content, LayerContent::Smart(sm) if matches!(&sm.source, SmartSource::Linked { path } if stored.contains(path))))
            .map(|(_, _, l)| l.id)
            .collect();
        let mut done: HashSet<u64> = HashSet::new();
        for layer in layers {
            if done.contains(&layer.0) {
                continue;
            }
            let r = app.run("layer.smartObjects.relinkToFile", json!({"layer": layer.0, "path": path}))?;
            done.insert(layer.0);
            done.extend(r.get("instances").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_u64));
        }
        let mut done: Vec<u64> = done.into_iter().collect();
        done.sort_unstable();
        Ok(done)
    })?;
    app.sync_views();
    if let Some(shown) = app.framework.dialogs.get_mut_by_id::<MissingLinksDialog>(dialog) {
        shown.files.retain(|row| row.file != file);
    }
    app.ctx().request_repaint();
    Ok(json!({"file": file, "path": path, "layers": relinked}))
}

#[cfg(test)]
#[path = "missing_links_tests.rs"]
mod tests;
