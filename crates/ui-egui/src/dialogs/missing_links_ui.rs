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
//! The dialog is an ordinary one in the UI state, so agents see it in `ui.inspect` and close it
//! with `ui.dialog.confirm` / `ui.dialog.cancel`.

use std::collections::HashSet;

use photocraft_doc::{DocId, LayerContent, LayerId, SmartSource};
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::state::DialogKind;

const MARK: &str = "__missingLinks";

/// Whether `fields` are this dialog's.
pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

/// How long an open waits for its linked files to be looked at. Local files answer at once; a
/// share that doesn't answer in time doesn't hold the interface up, and the document opens without
/// the dialog (the link watcher keeps looking, and the Layers panel's Missing filter lists them).
const LOOK_WAIT: std::time::Duration = std::time::Duration::from_millis(500);

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
    let files: Vec<Value> = missing.into_iter().map(|(file, paths)| json!({"file": file.to_string_lossy(), "paths": paths})).collect();
    if files.is_empty() {
        return;
    }
    let fields = json!({MARK: true, "__label": "PhotoCraft", "document": document.0, "name": st.doc.name, "files": files});
    app.ui.open_dialog(DialogKind::Command, fields.as_object().cloned().unwrap_or_default());
}

/// The missing files the dialog still lists, by absolute path.
fn files(fields: &Map<String, Value>) -> Vec<String> {
    let rows = fields.get("files").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    rows.iter().filter_map(|row| row.get("file").and_then(Value::as_str).map(str::to_string)).collect()
}

/// The dialog's contents: the heading, a row per missing file and the footer hint.
pub(crate) fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, fields: &Map<String, Value>, dialog: u64) {
    ui.label(tl!("Cannot locate linked assets:"));
    ui.add_space(6.0);
    let tint = ui.visuals().text_color();
    egui::ScrollArea::vertical().max_height(220.0).auto_shrink([false, true]).show(ui, |ui| {
        for file in files(fields) {
            ui.horizontal(|ui| {
                // The picker answers on a later frame; one that can't open now says why.
                if ui.button(tl!("Relink…")).clicked()
                    && let Err(e) = relink(app, dialog, &file)
                    && e != crate::file_dialog::CANCELLED
                {
                    app.ui.status = e;
                    app.ui.status_error = true;
                }
                let (chain, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                crate::icons::paint(ui, chain, "link", 14.0, tint);
                ui.add(egui::Label::new(file.as_str()).wrap());
            });
        }
    });
    ui.add_space(8.0);
    ui.add(
        egui::Label::new(
            egui::RichText::new(tl!(
                "To relink to a Library graphic, close this dialog, select the layer and choose Layer › Smart Objects › Relink to Library Graphic."
            ))
            .small(),
        )
        .wrap(),
    );
}

/// Relink…: asks for a file, then relinks the layers linked to `file` (an absolute path the dialog
/// lists) to it ([`relink_to`]).
pub(crate) fn relink(app: &mut PhotocraftApp, dialog: u64, file: &str) -> Result<Value, String> {
    let file = file.to_string();
    // A link needs a file on disk: the desktop picker answers its full path, the web picker only a
    // name and the bytes, which can't be linked to (as Layer › Smart Objects › Relink to File).
    app.pick_file_bytes(move |app, path, _| relink_to(app, dialog, &file, &path))
}

/// Relinks every layer of the dialog's document that is linked to `file` to `path` with Relink to
/// File, and takes `file` off the dialog's list. A smart object's other instances follow it, so
/// each set of instances is relinked once. Nothing leaves the list when a relink fails.
pub(crate) fn relink_to(app: &mut PhotocraftApp, dialog: u64, file: &str, path: &str) -> Result<Value, String> {
    if !std::path::Path::new(path).is_absolute() {
        return Err(format!("{path}: Relink to File needs a file on disk"));
    }
    let fields = app.ui.dialogs.iter().find(|d| d.id == dialog && owns(&d.fields)).map(|d| d.fields.clone()).ok_or("the dialog was closed")?;
    let document = fields.get("document").and_then(Value::as_u64).map(DocId).ok_or("the dialog names no document")?;
    let row = fields.get("files").and_then(Value::as_array).and_then(|rows| rows.iter().find(|r| r.get("file").and_then(Value::as_str) == Some(file)));
    let stored: HashSet<String> =
        row.and_then(|r| r.get("paths")).and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
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
    if let Some(rows) = app.ui.dialog_mut(dialog).and_then(|d| d.fields.get_mut("files")).and_then(Value::as_array_mut) {
        rows.retain(|r| r.get("file").and_then(Value::as_str) != Some(file));
    }
    Ok(json!({"file": file, "path": path, "layers": relinked}))
}

#[cfg(test)]
#[path = "missing_links_tests.rs"]
mod tests;
