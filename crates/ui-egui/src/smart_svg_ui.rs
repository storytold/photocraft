//! Explicit desktop SVG editor selection and bounded, undoable Smart Object save-back.

use base64::Engine as _;
use std::sync::Arc;
use std::time::Duration;

use craft::ui::contract::Bind;
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::env::Label;
use craft::ui::runtime::requests::PathKind;
use photocraft_doc::{DocId, LayerContent, LayerId, SmartSource};
use serde_json::{Value, json};

use crate::dialogs::params::ParamFamily;
use crate::{ExternalSvgEditor, ExternalSvgSession, PhotocraftApp};

const MAX_SESSIONS: usize = 8;
const MAX_SVG_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Editor {
    VectorCraft,
    Illustrator,
    Program,
    Internal,
}

struct Target {
    doc: DocId,
    layer: LayerId,
    contents_id: Value,
    fingerprint: Value,
    svg: Arc<Vec<u8>>,
    extension: String,
    size: Option<(f64, f64)>,
}

struct Pending {
    target: Target,
    editor: Editor,
    program: Option<String>,
}

struct Editing {
    target: Target,
    session: Box<dyn ExternalSvgSession>,
    saved: Option<crate::ExternalVectorSave>,
}

#[derive(Default)]
pub(crate) struct State {
    pending: Option<Pending>,
    editing: Vec<Editing>,
    internal_once: bool,
    /// The editor-choice dialog opened for `pending` (see [`host`]).
    dialog: Option<DialogId>,
}

impl State {
    /// True while the editor-choice dialog awaits OK or Cancel.
    pub(crate) fn modal_open(&self) -> bool {
        self.pending.is_some()
    }
}

/// Translate dialog and status text in the `smartSvg` context.
fn tr(source: &str) -> &str {
    crate::i18n::tr_ctx(crate::i18n::current(), "smartSvg", source)
}

/// Surface a non-fatal editing or sync failure in the status bar; the document is unchanged.
fn report(app: &mut PhotocraftApp, error: String) {
    app.ui.status = error;
    app.ui.status_error = true;
}

/// Automation retains the engine's internal Edit Contents path; it never launches a program.
pub(crate) fn intercept(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if id != "layer.smartObjects.editContents" {
        return None;
    }
    if std::mem::take(&mut app.smart_svg.internal_once) || app.automation_input || app.services.open_external_svg.is_none() {
        return None;
    }
    if !(params.is_null() || params.as_object().is_some_and(|p| p.keys().all(|key| key == "layer"))) {
        return None;
    }
    let st = app.session.active()?;
    let layer = match params.get("layer") {
        Some(v) => LayerId(v.as_u64()?),
        None => st.active_layer?,
    };
    let LayerContent::Smart(smart) = &st.doc.layer(layer)?.content else { return None };
    let bytes = match &smart.source {
        SmartSource::Embedded { bytes, .. } => bytes.as_slice(),
        SmartSource::Linked { path } => photocraft_io::linked::embedded_payload(&st.doc.metadata, path)?.1,
    };
    if !(photocraft_io::svg::is_svg(bytes) || bytes.starts_with(b"%PDF-") || bytes.starts_with(b"%!PS-Adobe-") || bytes.starts_with(&[0xc5, 0xd0, 0xd3, 0xc6]))
    {
        return None;
    }
    let doc = st.doc.id;
    // Contents already open in PhotoCraft's own editor: the engine switches to that document.
    if app.session.smart_links.iter().any(|link| link.parent == doc && link.contents_id == smart.contents_id) {
        return None;
    }
    let result: Result<Value, String> = (|| {
        if app.smart_svg.pending.is_some() {
            return Ok(json!({"deferred": true}));
        }
        if !safe_to_apply(app) {
            return Err("Finish the current edit or dialog before editing this SVG Smart Object".into());
        }
        let exported = app.run("layer.smartObjects.exportSvg", json!({"layer": layer.0}))?;
        let extension = exported.get("extension").and_then(Value::as_str).unwrap_or("svg").to_string();
        let svg = if let Some(svg) = exported.get("svg").and_then(Value::as_str) {
            svg.as_bytes().to_vec()
        } else {
            let encoded = exported.get("sourceBase64").and_then(Value::as_str).ok_or("The Smart Object did not export a vector")?;
            if encoded.len() > MAX_SVG_BYTES.div_ceil(3) * 4 {
                return Err("The vector exceeds the 16 MiB editor limit".into());
            }
            base64::engine::general_purpose::STANDARD.decode(encoded).map_err(|_| "Invalid vector source encoding")?
        };
        let size: Option<(f64, f64)> =
            serde_json::from_value(exported.get("sourceSize").cloned().unwrap_or(Value::Null)).map_err(|_| "Invalid vector source size")?;
        if svg.len() > MAX_SVG_BYTES {
            return Err("The SVG exceeds the 16 MiB editor limit".into());
        }
        let fingerprint = exported.get("fingerprint").filter(|v| !v.is_null()).cloned().ok_or("The SVG fingerprint is missing")?;
        let contents_id = exported.get("contentsId").filter(|v| !v.is_null()).cloned().ok_or("The Smart Object identity is missing")?;
        app.smart_svg.pending = Some(Pending {
            target: Target { doc, layer, contents_id, fingerprint, svg: Arc::new(svg), extension, size },
            editor: Editor::VectorCraft,
            program: None,
        });
        Ok(json!({"deferred": true}))
    })();
    if let Err(error) = &result {
        report(app, error.clone());
    }
    Some(result)
}

/// Saves apply only while the target's document is active, never by switching documents.
fn same_target(app: &PhotocraftApp, target: &Target) -> bool {
    app.session.active().is_some_and(|st| st.doc.id == target.doc)
}

/// The update command must be enabled and no layer sharing these contents (every placed
/// instance changes together) may be fully or pixel locked.
fn writable_target(app: &PhotocraftApp, target: &Target) -> bool {
    let Some(st) = app.session.active().filter(|st| st.doc.id == target.doc) else { return false };
    if !app.session.is_enabled_with("layer.smartObjects.updateSvg", &json!({"document":target.doc.0,"layer":target.layer.0})) {
        return false;
    }
    st.doc
        .walk()
        .into_iter()
        .filter(|(_, _, layer)| matches!(&layer.content, LayerContent::Smart(smart) if json!(smart.contents_id.0) == target.contents_id))
        .all(|(_, _, layer)| {
            let locks = st.doc.effective_locks(layer.id);
            !locks.all && !locks.pixels
        })
}

/// Act on the dialog's choice after re-exporting to confirm the fingerprint and contents id
/// still match: run the internal editor, or start (or replace) one external session per
/// shared contents, at most [`MAX_SESSIONS`]. Errors leave the document unchanged.
fn confirm(app: &mut PhotocraftApp) -> Result<Value, String> {
    let Some(pending) = app.smart_svg.pending.take() else { return Ok(json!({"cancelled": true})) };
    if !safe_to_apply(app) {
        return Err("Finish the current edit or dialog before editing this SVG Smart Object".into());
    }
    if !same_target(app, &pending.target) {
        return Err("SVG editing cancelled: the active document changed".into());
    }
    let exported = app.run("layer.smartObjects.exportSvg", json!({"layer": pending.target.layer.0}))?;
    if exported.get("fingerprint") != Some(&pending.target.fingerprint) || exported.get("contentsId") != Some(&pending.target.contents_id) {
        return Err("SVG editing cancelled: the Smart Object contents changed".into());
    }
    if pending.editor == Editor::Internal {
        // Re-enter through the normal shell dispatch, retaining its authorization and bookkeeping.
        app.smart_svg.internal_once = true;
        let result = app.run("layer.smartObjects.editContents", json!({"layer": pending.target.layer.0}));
        app.smart_svg.internal_once = false;
        if result.is_ok() {
            app.smart_svg.editing.retain(|edit| edit.target.doc != pending.target.doc || edit.target.contents_id != pending.target.contents_id);
        }
        return result;
    }
    let existing = app.smart_svg.editing.iter().position(|edit| edit.target.doc == pending.target.doc && edit.target.contents_id == pending.target.contents_id);
    if existing.is_none() && app.smart_svg.editing.len() >= MAX_SESSIONS {
        return Err("Close a document or stop an SVG editor session before starting another (limit: 8)".into());
    }
    let editor = match pending.editor {
        Editor::VectorCraft => ExternalSvgEditor::VectorCraft,
        Editor::Illustrator => ExternalSvgEditor::Illustrator,
        Editor::Program => ExternalSvgEditor::Program(pending.program.ok_or("Choose an editor executable first")?),
        Editor::Internal => return Err("Invalid SVG editor choice".into()),
    };
    let open = app.services.open_external_svg.as_mut().ok_or("External SVG editing is unavailable")?;
    let session = open(
        editor,
        crate::ExternalVectorSource { bytes: pending.target.svg.clone(), extension: pending.target.extension.clone(), size: pending.target.size },
    )?;
    if let Some(index) = existing {
        app.smart_svg.editing.remove(index);
    }
    let mut target = pending.target;
    target.svg = Arc::default();
    let target_is_native = target.extension != "svg";
    app.smart_svg.editing.push(Editing { target, session, saved: None });
    app.ui.status = tr(if target_is_native {
        "Vector editor opened. Save the original-format file to update this Smart Object."
    } else {
        "SVG editor opened. Save the SVG to update this Smart Object."
    })
    .into();
    app.ui.status_error = false;
    Ok(json!({"externalEditor": true}))
}

/// The key of the editor-choice dialog.
const KEY: &str = "layer.smartObjects.editSvg";

/// Each choice with its id in the dialog's `editor` field and its label.
const CHOICES: [(Editor, &str, &str); 4] = [
    (Editor::VectorCraft, "vectorCraft", "VectorCraft"),
    (Editor::Illustrator, "illustrator", "Adobe Illustrator"),
    (Editor::Program, "program", "Other application"),
    (Editor::Internal, "internal", "PhotoCraft (internal editor)"),
];

fn editor_id(editor: Editor) -> &'static str {
    CHOICES.iter().find(|(choice, ..)| *choice == editor).map_or("vectorCraft", |&(_, id, _)| id)
}

fn editor_from(id: Option<&str>) -> Editor {
    CHOICES.iter().find(|(_, choice, _)| Some(*choice) == id).map_or(Editor::VectorCraft, |(editor, ..)| *editor)
}

/// The dialog's rows. The internal editor is offered only for SVG, and Illustrator only where
/// the desktop shell discovers installations (Windows and macOS).
fn params(svg: bool) -> Vec<Param> {
    let options = CHOICES
        .iter()
        .filter(|(editor, ..)| match editor {
            Editor::Illustrator => cfg!(any(windows, target_os = "macos")),
            Editor::Internal => svg,
            _ => true,
        })
        .map(|&(_, id, label)| (id.to_owned(), Label::from(tr(label))))
        .collect();
    let choose = tr(if svg { "Choose the application used to edit this SVG:" } else { "Choose the application used to edit this vector:" });
    let mut rows = vec![Param::read_only_text("choose", Label::Empty, Label::from(choose)).full_width()];
    if !svg {
        let note = "AI/PDF/EPS files keep their original format. Unsupported features may change when saved. Original contents remain available through Undo.";
        rows.push(Param::read_only_text("nativeNote", Label::Empty, Label::from(tr(note))).full_width());
    }
    rows.push(Param::new("editor", Label::from(tr("Editor")), ParamKind::radio(options, Some("vectorCraft".to_owned()))));
    let extensions: &[&str] = if cfg!(windows) { &["exe"] } else { &[] };
    rows.push(
        Param::new("program", Label::from(tr("Executable")), ParamKind::text("")).browse(PathKind::open_file(extensions)).visible_when("editor", "program"),
    );
    let saved = tr(if svg {
        "Save to the opened SVG file. Each saved change becomes one undo step in PhotoCraft."
    } else {
        "Save to the opened original-format file. Each saved change becomes one undo step in PhotoCraft."
    });
    rows.push(Param::read_only_text("savedNote", Label::Empty, Label::from(saved)).full_width());
    rows.push(
        Param::read_only_text("waitNote", Label::Empty, Label::from(tr("Updates wait while another document or an editing dialog is active."))).full_width(),
    );
    rows
}

/// The editor-choice dialog over `pending`'s choice. "Stop syncing" is offered while an editor
/// session for the same contents runs (`running`).
fn dialog(ctx: &egui::Context, pending: &Pending, running: bool) -> HostedParamDialog {
    let svg = pending.target.extension == "svg";
    let fields = DialogFields::from_pairs([("editor", editor_id(pending.editor).to_owned()), ("program", pending.program.clone().unwrap_or_default())]);
    HostedParamDialog::new(ctx, KEY, fields, move |fields| {
        let title = tr(if svg { "Edit SVG Smart Object" } else { "Edit Vector Smart Object" });
        let dialog = ParamDialog::new(KEY, Label::from(title), params(svg)).values(Bind::bound(fields));
        if running { dialog.extra_buttons(vec![(STOP_SYNC.to_owned(), Label::from(tr("Stop syncing this Smart Object")))]) } else { dialog }
    })
}

/// The id of the "Stop syncing" footer button.
const STOP_SYNC: &str = "stopSync";

/// Whether an editor session runs for the contents `pending` would edit.
fn session_running(app: &PhotocraftApp, pending: &Pending) -> bool {
    app.smart_svg.editing.iter().any(|edit| edit.target.doc == pending.target.doc && edit.target.contents_id == pending.target.contents_id)
}

/// Once per pass: opens the editor-choice dialog while a choice is pending, and closes it when the
/// choice was cleared elsewhere (the document went away, a session ended).
pub(crate) fn host(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let open = app.smart_svg.dialog.filter(|id| app.framework.dialogs.stack().ids().any(|shown| shown == *id));
    match (app.smart_svg.pending.as_ref(), open) {
        (Some(pending), None) => {
            let dialog = dialog(ctx, pending, session_running(app, pending));
            app.smart_svg.dialog = app.framework.dialogs.open_unless_open(dialog).ok();
        }
        (None, Some(id)) => {
            app.framework.dialogs.cancel_by_id(ctx, id);
            app.smart_svg.dialog = None;
        }
        (None, None) => app.smart_svg.dialog = None,
        (Some(_), Some(_)) => {}
    }
}

/// The `PARAM_FAMILIES` row of the editor-choice dialog.
pub(crate) const fn family() -> ParamFamily {
    ParamFamily { check: Some(check), footer: Some(footer), cancel: Some(cancel), ..ParamFamily::new(owns, run) }
}

fn owns(key: &str) -> bool {
    key == KEY
}

/// Refuses OK for "Other application" until an executable is chosen.
fn check(_app: &mut PhotocraftApp, result: &ParamResult) -> Result<(), Label> {
    let program = result.fields.text("program").unwrap_or_default();
    if editor_from(result.fields.text("editor")) == Editor::Program && program.is_empty() {
        return Err(Label::from(tr("Choose an editor executable first")));
    }
    Ok(())
}

/// OK: the dialog's choice becomes the pending one and is confirmed.
fn run(app: &mut PhotocraftApp, _ctx: &egui::Context, result: ParamResult) {
    app.smart_svg.dialog = None;
    let Some(pending) = app.smart_svg.pending.as_mut() else { return };
    pending.editor = editor_from(result.fields.text("editor"));
    pending.program = result.fields.text("program").filter(|program| !program.is_empty()).map(str::to_owned);
    if let Err(error) = confirm(app) {
        report(app, error);
    }
}

/// Cancel, Escape, the close box, or a parent closing over the dialog: no choice, nothing launched.
fn cancel(app: &mut PhotocraftApp, _ctx: &egui::Context, _id: DialogId) {
    app.smart_svg.pending = None;
    app.smart_svg.dialog = None;
}

/// The footer buttons: "Stop syncing" ends the running sessions for the same contents and drops the choice.
fn footer(app: &mut PhotocraftApp, ctx: &egui::Context, id: DialogId, action: &str) {
    if action != STOP_SYNC {
        return;
    }
    if let Some(pending) = app.smart_svg.pending.take() {
        app.smart_svg.editing.retain(|edit| edit.target.doc != pending.target.doc || edit.target.contents_id != pending.target.contents_id);
    }
    app.smart_svg.dialog = None;
    app.framework.dialogs.cancel_by_id(ctx, id);
}

/// True only when no save, dialog, mask/channel target, transform, preview or drag is in
/// progress, so an external save cannot land in the middle of another edit.
fn safe_to_apply(app: &PhotocraftApp) -> bool {
    !app.automation_input
        && !app.saving()
        && !app.save_options_open()
        && !app.file_dialog_open()
        && !app.ui.mask_target
        && !app.layer_mask_targeted()
        && !app.ui.vector_mask_target
        && !app.session.tools.mask_targeted
        && app.session.active().is_some_and(|st| {
            st.doc.quick_mask.is_none()
                && st.channel_view.layer_mask.is_none()
                && st.channel_view.target == photocraft_engine::channel_cmds::ChannelTarget::Composite
                && app.session.job_on(st.doc.id).is_none()
        })
        && !app.dialog_open()
        && app.discard.is_none()
        && !app.smart_svg.modal_open()
        && app.ui.transform.is_none()
        && app.ui.cam_transform.is_none()
        && app.ui.text_edit.is_none()
        && app.ui.type_transform.is_none()
        && app.ui.symmetry_transform.is_none()
        && !app.ui.shape_stroke_open
        && !crate::direct_select::editing(app)
        && !crate::shape_controls::editing(app)
        && !app.distort.active()
        && crate::distort_ui::modal_workspace(app).is_none()
        && app.color_range.is_none()
        && app.text_style_preview.is_none()
        && !crate::crop_ui::pending(app)
        && !crate::gradient_ui::editing(app)
        && app.move_preview.is_none()
        && app.patch_preview.is_none()
        && app.filter_preview.is_none()
        && app.adjust_preview.is_none()
        && app.live_adjust.is_none()
        && app.transform_preview.is_none()
        && app.shape_fill_preview.is_none()
        && app.solid_fill_preview.is_none()
        && app.live_stroke.is_none()
        && !crate::canvas::tools::in_gesture(app)
}

/// Poll platform snapshots without changing the active document or selection.
pub(crate) fn poll(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if app
        .smart_svg
        .pending
        .as_ref()
        .is_some_and(|pending| !app.session.documents().iter().any(|st| st.doc.id == pending.target.doc && st.doc.layer(pending.target.layer).is_some()))
    {
        app.smart_svg.pending = None;
    }
    if app.smart_svg.editing.is_empty() {
        return;
    }
    let safe = safe_to_apply(app) && ctx.memory(|memory| memory.top_modal_layer().is_none()) && !ctx.input(|input| input.pointer.any_down());
    let mut editing = std::mem::take(&mut app.smart_svg.editing);
    for mut edit in editing.drain(..) {
        let exists = app
            .session
            .documents()
            .iter()
            .find(|st| st.doc.id == edit.target.doc)
            .and_then(|st| st.doc.layer(edit.target.layer))
            .is_some_and(|layer| matches!(&layer.content, LayerContent::Smart(smart) if json!(smart.contents_id.0) == edit.target.contents_id));
        if !exists {
            continue;
        }
        if let Some(result) = edit.session.poll_vector() {
            match result {
                Ok(saved) if saved.bytes.len() <= MAX_SVG_BYTES && saved.preview_svg.as_ref().is_none_or(|svg| svg.len() <= MAX_SVG_BYTES) => {
                    edit.saved = Some(saved)
                }
                Ok(_) => report(app, "The SVG exceeds the 16 MiB editor limit; save a smaller SVG to retry".into()),
                Err(error) => report(app, error),
            }
        }
        // Release admission capacity after cancellation/startup failure. Retain a final
        // pending save until its target is safe to update.
        if edit.saved.is_none() && edit.session.finished() {
            continue;
        }
        let internal_edit = app.session.smart_links.iter().any(|link| link.parent == edit.target.doc && json!(link.contents_id.0) == edit.target.contents_id);
        if safe
            && !internal_edit
            && same_target(app, &edit.target)
            && edit.saved.is_some()
            && writable_target(app, &edit.target)
            && let Some(bytes) = edit.saved.take()
        {
            let native = edit.target.extension != "svg";
            let preview = if native { bytes.preview_svg } else { Some(bytes.bytes.clone()) };
            let svg = match preview.and_then(|svg| String::from_utf8(svg).ok()) {
                Some(svg) => svg,
                None => {
                    report(app, "The saved vector has no valid display preview; save again to retry".into());
                    app.smart_svg.editing.push(edit);
                    continue;
                }
            };
            let mut params = json!({"document": edit.target.doc.0, "layer": edit.target.layer.0, "fingerprint": edit.target.fingerprint, "contentsId": edit.target.contents_id});
            let command = if native {
                params["previewSvg"] = json!(svg);
                params["sourceBase64"] = json!(base64::engine::general_purpose::STANDARD.encode(&bytes.bytes));
                params["extension"] = json!(edit.target.extension);
                "layer.smartObjects.updateVectorSource"
            } else {
                params["svg"] = json!(svg);
                "layer.smartObjects.updateSvg"
            };
            match app.run(command, params) {
                Ok(result) => {
                    if let Some(fingerprint) = result.get("fingerprint").filter(|v| !v.is_null()) {
                        edit.target.fingerprint = fingerprint.clone();
                        crate::notices::io_result_warnings(app, &result, "SVG");
                    } else {
                        report(app, "The SVG update did not return its fingerprint; syncing stopped".into());
                        continue;
                    }
                }
                Err(error) => {
                    let stale = error.contains("contents changed") || error.contains("conflicting contents");
                    let kind = if native { "Vector" } else { "SVG" };
                    report(
                        app,
                        if stale {
                            format!("{kind} syncing stopped: {error}")
                        } else {
                            format!("{kind} update failed; change the working file and save to retry: {error}")
                        },
                    );
                    if stale {
                        continue;
                    }
                }
            }
        }
        app.smart_svg.editing.push(edit);
    }
    if !app.smart_svg.editing.is_empty() {
        ctx.request_repaint_after(Duration::from_millis(500));
    }
}

#[cfg(test)]
mod tests;
