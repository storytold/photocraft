//! The save waiting on a format's options: JPEG, TIFF, Targa and DDS Options are one hosted
//! dialog (`HostedFormatOptions`) and one parked save, so the park, the cancel and the write are
//! written once here and each format keeps its start facts and its answer.
//!
//! A save [`park`]s behind the dialog and returns. `dialogs::run` draws the dialog and [`take`]
//! (a `dialogs::TAKERS` entry) takes its answer: OK writes the save through the format's `finish`
//! and journals it, Cancel drops it.

use craft::ui::dialog::{DialogId, DialogResult};
use craft::ui::patterns::form_dialogs::{FormatOptions, HostedFormatOptions};
use photocraft_doc::DocId;
use serde_json::{Value, json};

use super::{dds_options_ui, jpeg_options_ui, targa_options_ui, tiff_options_ui};
use crate::{ExportSettings, PhotocraftApp};

/// A save waiting on a format's options.
pub struct ParkedSave {
    /// The document that requested the save, even if tabs change while the dialog is open.
    pub doc: DocId,
    /// Where the document goes.
    pub path: String,
    /// Save a Copy: the document keeps its path and unsaved changes.
    pub copy: bool,
    /// The step the answered save is recorded as, so an action being recorded keeps it:
    /// `file.saveAs` for Save As (#2032), `file.saveACopy` for Save a Copy (#4373), and
    /// `file.save` (with no path) for File › Save writing the document's own file (#3531).
    pub record: &'static str,
    /// The format asked about, with what only its answer needs.
    pub format: ParkedFormat,
}

/// The format a [`ParkedSave`] asks about.
pub enum ParkedFormat {
    /// JPEG Options.
    Jpeg(jpeg_options_ui::Parked),
    /// TIFF Options.
    Tiff(tiff_options_ui::Parked),
    /// Targa Options.
    Targa,
    /// DDS Options.
    Dds,
}

impl ParkedSave {
    /// The save settings of the waiting document (its path resolves linked smart objects), even
    /// if another tab is active now. Closed meanwhile, the active tab's, and the write reports
    /// the closed document.
    pub fn settings(&self, app: &mut PhotocraftApp) -> ExportSettings {
        app.with_document(self.doc, |app| Ok(app.save_settings())).unwrap_or_else(|_| app.save_settings())
    }

    /// The params the answered save is recorded with before the format adds its own: the path,
    /// none for a plain Save, which writes whichever file the action is played on (#3531).
    pub fn step(&self) -> Value {
        if self.record == "file.save" { json!({}) } else { json!({"path": self.path}) }
    }
}

/// Parks `save` behind `dialog` and opens it; the id of the dialog. Refuses while another save
/// waits.
pub fn park(app: &mut PhotocraftApp, save: ParkedSave, dialog: HostedFormatOptions) -> Result<DialogId, String> {
    if app.parked_save.is_some() {
        return Err(crate::SAVE_OPTIONS_OPEN.into());
    }
    let id = app.framework.dialogs.open(dialog);
    app.parked_save = Some(save);
    Ok(id)
}

/// Takes the dialog's answer for the parked save (a `dialogs::TAKERS` entry): answers a pending
/// JPEG size request, then writes the save on OK or drops it on Cancel. Everything it drains is
/// taken once, so a second pass in the same frame finds nothing and changes nothing.
pub fn take(app: &mut PhotocraftApp, ctx: &egui::Context) {
    jpeg_options_ui::poll_estimate(app, ctx);
    let Some(result) = app.framework.dialogs.take::<HostedFormatOptions>() else { return };
    let Some(save) = app.parked_save.take() else { return };
    // A mismatched or unknown answer drops the save as a cancel does.
    match result {
        DialogResult::Ok(FormatOptions::Jpeg(options)) if matches!(save.format, ParkedFormat::Jpeg(_)) => jpeg_options_ui::finish(app, ctx, save, options),
        DialogResult::Ok(FormatOptions::Tiff(options)) if matches!(save.format, ParkedFormat::Tiff(_)) => tiff_options_ui::finish(app, ctx, save, options),
        DialogResult::Ok(FormatOptions::Targa(options)) if matches!(save.format, ParkedFormat::Targa) => targa_options_ui::finish(app, ctx, save, options),
        DialogResult::Ok(FormatOptions::Dds(options)) if matches!(save.format, ParkedFormat::Dds) => dds_options_ui::finish(app, ctx, save, options),
        _ => canceled(app),
    }
}

/// The save was canceled: nothing is written.
fn canceled(app: &mut PhotocraftApp) {
    app.ui.status = tl!("Save canceled").into();
    app.ui.status_error = false;
}

/// The `formatOptions` dialog of the control API: parks a save of `params.path` as Save As
/// does (no copy, recorded as `file.saveAs`) behind the options of `params.format` (`jpeg`,
/// `tiff`, `targa` or `dds`). The request opens the prompt for the format it names, whatever the
/// path's extension.
pub fn open(app: &mut PhotocraftApp, _ctx: &egui::Context, params: &Value) -> Result<DialogId, String> {
    let format = params.get("format").and_then(Value::as_str).ok_or("formatOptions: `format` must be a string")?;
    let path = params.get("path").and_then(Value::as_str).ok_or("formatOptions: `path` must be a string")?.to_owned();
    let record = "file.saveAs";
    match format {
        "jpeg" => jpeg_options_ui::park_dialog(app, path, false, record),
        "tiff" => tiff_options_ui::park_dialog(app, path, false, record),
        "targa" => targa_options_ui::park_dialog(app, path, false, record),
        "dds" => dds_options_ui::park_dialog(app, path, false, record),
        other => Err(format!("unknown format options `{other}`")),
    }
}

/// OK on the open dialog, then [`take`], as the button and the next pass do.
#[cfg(test)]
pub fn confirm(app: &mut PhotocraftApp, ctx: &egui::Context) {
    app.framework.dialogs.confirm::<HostedFormatOptions>(ctx);
    take(app, ctx);
}
