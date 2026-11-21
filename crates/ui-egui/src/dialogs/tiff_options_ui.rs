//! TIFF Options: what File › Save As and Save a Copy ask before writing a TIFF, as Photoshop's
//! TIFF Options dialog does (#8827): Image Compression (None, LZW, ZIP), Save Image Pyramid,
//! Save Transparency (offered when the document has no Background layer) and, for a document a
//! TIFF stores with its layers, "Discard Layers and Save a Copy". Photoshop's JPEG compression,
//! Pixel Order and Byte Order aren't offered: the TIFF encoder writes interleaved samples in the
//! machine's byte order without JPEG.
//!
//! With "Ask Before Saving Layered TIFF Files" on (Preferences › File Handling), a layered save
//! also notes that layers increase the file size; "Don't show again" clears the preference.
//!
//! The save is parked (`format_prompts::ParkedSave`) while the hosted dialog is up and written
//! once answered, journalled as the `record` command passed to [`park`] with its path,
//! `tiffLayers` (layered documents) and [`TiffChoice::record`]'s params. Cancel drops the save.
//! Scripts skip the prompt by passing any of `tiffCompression`, `tiffTransparency`,
//! `tiffPyramid` or `tiffLayers`. The answer is remembered for the next TIFF save of the session.

use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{ExportOptionsKind, FormatOptions, HostedFormatOptions, TiffCompression as UiTiffCompression, TiffOptions};
use photocraft_codecs::TiffCompression;
use serde_json::Value;

use crate::format_prompts::{self, ParkedFormat, ParkedSave};
use crate::{ExportSettings, PhotocraftApp};

/// A TIFF Options answer, from the prompt or from a command's params.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TiffChoice {
    pub compression: TiffCompression,
    /// Save Transparency; `None` for the default (see [`ExportSettings::tiff_transparency`]).
    pub transparency: Option<bool>,
    /// Save Image Pyramid.
    pub pyramid: bool,
    /// `Some(false)`: "Discard Layers and Save a Copy". `None` keeps what the save would do.
    pub layers: Option<bool>,
}

impl TiffChoice {
    /// `settings` writing this choice.
    pub fn apply(self, settings: ExportSettings) -> ExportSettings {
        ExportSettings {
            tiff_compression: self.compression,
            tiff_transparency: self.transparency,
            tiff_pyramid: self.pyramid,
            tiff_layers: self.layers.unwrap_or(settings.tiff_layers),
            ..settings
        }
    }

    /// Adds `tiffCompression`, and `tiffTransparency` and `tiffPyramid` when set, to a recorded
    /// save step, so replaying it doesn't ask again. `tiffLayers` is recorded by the save itself.
    pub fn record(self, step: &mut Value) {
        step["tiffCompression"] = compression_name(self.compression).into();
        if let Some(t) = self.transparency {
            step["tiffTransparency"] = t.into();
        }
        if self.pyramid {
            step["tiffPyramid"] = true.into();
        }
    }
}

/// The `tiffCompression` name of `c` (PackBits, which the dialog doesn't offer, as none).
pub fn compression_name(c: TiffCompression) -> &'static str {
    match c {
        TiffCompression::Lzw => "lzw",
        TiffCompression::Deflate => "zip",
        TiffCompression::None | TiffCompression::PackBits => "none",
    }
}

/// The TIFF Options a command or control call gives (`tiffCompression`, `tiffTransparency`,
/// `tiffPyramid`, `tiffLayers`), which skip the prompt. None given: `None`. A bad value is an
/// error, as in the engine's saves.
pub fn choice_param(params: &Value, cmd: &str) -> Result<Option<TiffChoice>, String> {
    use photocraft_engine::file_cmds::{tiff_bool_param, tiff_compression_param};
    let compression = tiff_compression_param(params, cmd).map_err(|e| e.to_string())?;
    let transparency = tiff_bool_param(params, "tiffTransparency", cmd).map_err(|e| e.to_string())?;
    let pyramid = tiff_bool_param(params, "tiffPyramid", cmd).map_err(|e| e.to_string())?;
    let layers = match params.get("tiffLayers") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(b)) => Some(*b),
        Some(v) => return Err(format!("{cmd}: `tiffLayers` must be true or false, got {v}")),
    };
    let any = compression.is_some() || transparency.is_some() || pyramid.is_some() || layers.is_some();
    Ok(any.then(|| TiffChoice { compression: compression.unwrap_or_default(), transparency, pyramid: pyramid.unwrap_or(false), layers }))
}

/// What the last TIFF Options answer of the session chose, where the next prompt starts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Remembered {
    pub compression: TiffCompression,
    pub pyramid: bool,
}

/// What a parked TIFF save knows of its document: which rows the dialog offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Parked {
    /// The TIFF would carry the document's layers: "Discard Layers and Save a Copy" is offered.
    layered: bool,
    /// The document has no Background layer, so its composite can be transparent: Save
    /// Transparency is offered.
    has_transparency: bool,
    /// The layered-file note ("Ask Before Saving Layered TIFF Files") shows.
    layer_note: bool,
}

/// The framework's compression for the codec's (PackBits, which the dialog doesn't offer, as none).
fn ui_compression(c: TiffCompression) -> UiTiffCompression {
    match c {
        TiffCompression::Lzw => UiTiffCompression::Lzw,
        TiffCompression::Deflate => UiTiffCompression::Zip,
        TiffCompression::None | TiffCompression::PackBits => UiTiffCompression::None,
    }
}

/// The codec's compression for the framework's.
fn codec_compression(c: UiTiffCompression) -> TiffCompression {
    match c {
        UiTiffCompression::Lzw => TiffCompression::Lzw,
        UiTiffCompression::Zip => TiffCompression::Deflate,
        _ => TiffCompression::None,
    }
}

/// `true` when `path` is a TIFF name (`.tif` / `.tiff`).
pub fn is_tiff_path(path: &str) -> bool {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    name.rsplit_once('.').is_some_and(|(stem, ext)| !stem.is_empty() && (ext.eq_ignore_ascii_case("tif") || ext.eq_ignore_ascii_case("tiff")))
}

/// Whether saving the active document to `path` asks for TIFF Options: any TIFF.
pub fn wants_prompt(app: &PhotocraftApp, path: &str) -> bool {
    is_tiff_path(path) && app.session.active().is_some()
}

/// Parks the save of the active document to `path` behind the prompt. A prompt already waiting
/// is never replaced.
pub fn park(app: &mut PhotocraftApp, path: String, copy: bool, record: &'static str) -> Result<(), String> {
    park_dialog(app, path, copy, record).map(drop)
}

/// [`park`], returning the id of the dialog it opened.
pub fn park_dialog(app: &mut PhotocraftApp, path: String, copy: bool, record: &'static str) -> Result<DialogId, String> {
    let doc = app.active_doc_id()?;
    let st = app.session.active().ok_or("no document")?;
    let layered = photocraft_engine::file_cmds::tiff_would_write_layers(&st.doc);
    let has_transparency = st.doc.background().is_none();
    // Starts where an unset Save Transparency writes: kept unless there are alpha or spot channels.
    let transparency = st.doc.channels.is_empty() && st.doc.quick_mask.is_none();
    let layer_note = layered && app.session.prefs().file_handling.ask_before_saving_layered_tiff;
    let Remembered { compression, pyramid } = app.tiff_remembered;
    let mut options = TiffOptions::default();
    options.compression = ui_compression(compression);
    options.pyramid = pyramid;
    options.transparency = transparency;
    let dialog =
        HostedFormatOptions::new(app.ctx(), ExportOptionsKind::Tiff).initial(FormatOptions::Tiff(options)).tiff_document(layered, has_transparency, layer_note);
    let parked = Parked { layered, has_transparency, layer_note };
    format_prompts::park(app, ParkedSave { doc, path, copy, record, format: ParkedFormat::Tiff(parked) }, dialog)
}

/// OK: writes the parked save with the dialog's answer, and journals it.
pub(crate) fn finish(app: &mut PhotocraftApp, ctx: &egui::Context, save: ParkedSave, options: TiffOptions) {
    let ParkedFormat::Tiff(Parked { layered, has_transparency, layer_note }) = save.format else { return };
    if layer_note && options.dont_show_again && !options.discard_layers {
        app.session.edit_prefs(|prefs| prefs.file_handling.ask_before_saving_layered_tiff = false);
    }
    let compression = codec_compression(options.compression);
    app.tiff_remembered = Remembered { compression, pyramid: options.pyramid };
    let choice = TiffChoice {
        compression,
        transparency: has_transparency.then_some(options.transparency),
        pyramid: options.pyramid,
        layers: layered.then_some(!options.discard_layers),
    };
    let mut step = save.step();
    if let Some(layers) = choice.layers {
        step["tiffLayers"] = layers.into();
    }
    choice.record(&mut step);
    let settings = choice.apply(save.settings(app));
    let ParkedSave { doc, path, copy, record, .. } = save;
    app.write_answered_save(ctx, doc, path, &settings, copy || options.discard_layers, (record, step));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn choice_params() {
        assert_eq!(choice_param(&json!({}), "file.saveAs"), Ok(None));
        assert_eq!(choice_param(&json!({"tiffCompression": null, "tiffPyramid": null}), "file.saveAs"), Ok(None));
        assert_eq!(
            choice_param(&json!({"tiffCompression": "lzw"}), "file.saveAs"),
            Ok(Some(TiffChoice { compression: TiffCompression::Lzw, ..Default::default() }))
        );
        assert_eq!(
            choice_param(&json!({"tiffTransparency": false, "tiffPyramid": true, "tiffLayers": false}), "file.saveAs"),
            Ok(Some(TiffChoice { compression: TiffCompression::None, transparency: Some(false), pyramid: true, layers: Some(false) }))
        );
        for bad in [
            json!({"tiffCompression": "jpeg"}),
            json!({"tiffCompression": 1}),
            json!({"tiffTransparency": "yes"}),
            json!({"tiffTransparency": 1}),
            json!({"tiffPyramid": [true]}),
            json!({"tiffLayers": "false"}),
        ] {
            assert!(choice_param(&bad, "file.saveAs").is_err(), "{bad}");
        }
    }

    #[test]
    fn apply_sets_the_export_settings() {
        let base = ExportSettings::default();
        let s = TiffChoice { compression: TiffCompression::Deflate, transparency: Some(false), pyramid: true, layers: Some(false) }.apply(base.clone());
        assert_eq!((s.tiff_compression, s.tiff_transparency, s.tiff_pyramid, s.tiff_layers), (TiffCompression::Deflate, Some(false), true, false));
        let io = s.io_options();
        assert_eq!(
            (io.encode.tiff_compression, io.tiff_transparency, io.encode.tiff_pyramid, io.tiff_layers),
            (TiffCompression::Deflate, Some(false), true, false)
        );
        // No layers answer keeps what the save would do.
        assert!(TiffChoice::default().apply(base).tiff_layers);
    }
}
