//! DDS Options: the compression and mip chain File › Save As and Save a Copy ask for before
//! writing a DirectDraw Surface texture (#1327). Photoshop has no DDS writer of its own (a plug-in
//! adds one), so this follows the texture tools' usual choices: uncompressed, or BC1/BC2/BC3
//! (DXT1/DXT3/DXT5), and whether to generate mipmaps (see `photocraft_io::DdsOptions`).
//!
//! The save is parked (`format_prompts::ParkedSave`) while the hosted dialog is up and written
//! once answered, recorded as `file.saveAs` or `file.saveACopy`; `file.save` when File › Save
//! confirmed the document's own texture (#3531), which plays back on the file the action is
//! played on. Cancel drops the save. Scripts skip the prompt by passing `ddsCompression` or
//! `ddsMipmaps`.

use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{
    DdsCompression as UiDdsCompression, DdsOptions as UiDdsOptions, ExportOptionsKind, FormatOptions, HostedFormatOptions,
};
use photocraft_io::{DdsCompression, DdsOptions};

use crate::format_prompts::{self, ParkedFormat, ParkedSave};
use crate::{ExportSettings, PhotocraftApp};

/// `true` when `path` names a DDS texture.
pub fn wants_prompt(path: &str) -> bool {
    photocraft_engine::file_cmds::extension(path).and_then(|e| photocraft_formats::from_extension(&e)) == Some(photocraft_formats::Format::Dds)
}

/// The DDS options a command or control call asks for (`"ddsCompression"`, `"ddsMipmaps"`),
/// which skip the prompt.
pub fn options_param(params: &serde_json::Value, cmd: &str) -> Result<Option<DdsOptions>, String> {
    photocraft_engine::file_cmds::dds_param(params, cmd).map_err(|e| e.to_string())
}

/// The params a save with `options` is recorded with.
pub fn record(options: DdsOptions, step: &mut serde_json::Value) {
    step["ddsCompression"] = options.compression.name().into();
    step["ddsMipmaps"] = options.mipmaps.into();
}

/// Parks the save behind the prompt. A prompt already waiting is never replaced.
pub fn park(app: &mut PhotocraftApp, path: String, copy: bool, record: &'static str) -> Result<(), String> {
    park_dialog(app, path, copy, record).map(drop)
}

/// [`park`], returning the id of the dialog it opened: it starts uncompressed without mipmaps.
pub fn park_dialog(app: &mut PhotocraftApp, path: String, copy: bool, record: &'static str) -> Result<DialogId, String> {
    let doc = app.active_doc_id()?;
    let dialog = HostedFormatOptions::new(app.ctx(), ExportOptionsKind::Dds).initial(FormatOptions::Dds(UiDdsOptions::default()));
    format_prompts::park(app, ParkedSave { doc, path, copy, record, format: ParkedFormat::Dds }, dialog)
}

/// The codec's compression for the framework's.
fn codec_compression(c: UiDdsCompression) -> DdsCompression {
    match c {
        UiDdsCompression::Bc1 => DdsCompression::Bc1,
        UiDdsCompression::Bc2 => DdsCompression::Bc2,
        UiDdsCompression::Bc3 => DdsCompression::Bc3,
        _ => DdsCompression::None,
    }
}

/// OK: writes the parked save with the dialog's options, and journals it.
pub(crate) fn finish(app: &mut PhotocraftApp, ctx: &egui::Context, save: ParkedSave, options: UiDdsOptions) {
    let options = DdsOptions { compression: codec_compression(options.compression), mipmaps: options.mipmaps };
    let settings = ExportSettings { dds: options, ..save.settings(app) };
    let mut step = save.step();
    record(options, &mut step);
    let ParkedSave { doc, path, copy, record, .. } = save;
    app.write_answered_save(ctx, doc, path, &settings, copy, (record, step));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_params() {
        use serde_json::json;
        assert_eq!(options_param(&json!({}), "file.saveAs"), Ok(None));
        assert_eq!(options_param(&json!({"ddsCompression": "bc3"}), "file.saveAs"), Ok(Some(DdsOptions { compression: DdsCompression::Bc3, mipmaps: false })));
        assert_eq!(options_param(&json!({"ddsMipmaps": true}), "file.saveAs"), Ok(Some(DdsOptions { compression: DdsCompression::None, mipmaps: true })));
        for bad in [json!({"ddsCompression": "bc7"}), json!({"ddsCompression": 3}), json!({"ddsMipmaps": "yes"})] {
            assert!(options_param(&bad, "file.saveAs").is_err(), "{bad}");
        }
        let mut step = json!({"path": "a.dds"});
        record(DdsOptions { compression: DdsCompression::Bc1, mipmaps: true }, &mut step);
        assert_eq!(step, json!({"path": "a.dds", "ddsCompression": "bc1", "ddsMipmaps": true}));
        assert_eq!(options_param(&step, "file.saveAs"), Ok(Some(DdsOptions { compression: DdsCompression::Bc1, mipmaps: true })));
    }
}
