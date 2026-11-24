//! Targa Options: the bits per pixel File › Save As and Save a Copy ask for before writing an RGB
//! Targa, as Photoshop's Targa Options does (its Resolution; 16 bits/pixel isn't offered). 32 bits
//! write the document's alpha channel as the Targa's alpha, so a game texture keeps the mask its
//! alpha holds (see `photocraft_io::TgaBits`).
//!
//! The save is parked (`format_prompts::ParkedSave`) while the hosted dialog is up and written
//! once answered, recorded as `file.saveAs` (#2032, even when a flat Targa is written as a copy),
//! `file.saveACopy` (#4373), or `file.save` when File › Save confirmed the document's own Targa
//! (#3531), which plays back on the file the action is played on. Cancel drops the save.
//! Scripts skip the prompt by passing `tgaBits`.

use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{ExportOptionsKind, FormatOptions, HostedFormatOptions, TargaOptions};
use photocraft_io::TgaBits;

use crate::format_prompts::{self, ParkedFormat, ParkedSave};
use crate::{ExportSettings, PhotocraftApp};

/// `true` when `path` names a Targa (`.tga`, and the rarer `.icb`, `.vda`, `.vst`).
pub fn is_tga_path(path: &str) -> bool {
    photocraft_engine::file_cmds::extension(path).and_then(|e| photocraft_formats::from_extension(&e)) == Some(photocraft_formats::Format::Tga)
}

/// Whether saving the active document to `path` asks for Targa Options: an RGB Targa. Grayscale
/// Targas are 8-bit, with nothing to choose.
pub fn wants_prompt(app: &PhotocraftApp, path: &str) -> bool {
    is_tga_path(path) && app.session.active().is_some_and(|st| st.doc.mode != photocraft_color::ColorMode::Grayscale)
}

/// The Targa bits a command or control call asks for (`"tgaBits": 24 | 32`), which skip the prompt.
pub fn bits_param(params: &serde_json::Value, cmd: &str) -> Result<Option<TgaBits>, String> {
    photocraft_engine::file_cmds::tga_bits_param(params, cmd).map_err(|e| e.to_string())
}

/// Parks the save behind the prompt, starting at 32 bits when the document has an alpha channel.
/// A prompt already waiting is never replaced (a shortcut can still reach Save a Copy).
pub fn park(app: &mut PhotocraftApp, path: String, copy: bool, record: &'static str) -> Result<(), String> {
    park_dialog(app, path, copy, record).map(drop)
}

/// [`park`], returning the id of the dialog it opened.
pub fn park_dialog(app: &mut PhotocraftApp, path: String, copy: bool, record: &'static str) -> Result<DialogId, String> {
    let doc = app.active_doc_id()?;
    let bits = app.session.active().map_or(TgaBits::Bits24, |st| photocraft_io::tga_default_bits(&st.doc));
    let dialog = HostedFormatOptions::new(app.ctx(), ExportOptionsKind::Targa).initial(FormatOptions::Targa(TargaOptions::new(bits.bits())));
    format_prompts::park(app, ParkedSave { doc, path, copy, record, format: ParkedFormat::Targa }, dialog)
}

/// OK: writes the parked save at the dialog's bits per pixel, and journals it.
pub(crate) fn finish(app: &mut PhotocraftApp, ctx: &egui::Context, save: ParkedSave, options: TargaOptions) {
    let bits = if options.bits == 32 { TgaBits::Bits32 } else { TgaBits::Bits24 };
    let settings = ExportSettings { tga_bits: Some(bits), ..save.settings(app) };
    let mut step = save.step();
    step["tgaBits"] = bits.bits().into();
    let ParkedSave { doc, path, copy, record, .. } = save;
    app.write_answered_save(ctx, doc, path, &settings, copy, (record, step));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_params() {
        use serde_json::json;
        assert_eq!(bits_param(&json!({}), "file.saveAs"), Ok(None));
        assert_eq!(bits_param(&json!({"tgaBits": null}), "file.saveAs"), Ok(None));
        assert_eq!(bits_param(&json!({"tgaBits": 24}), "file.saveAs"), Ok(Some(TgaBits::Bits24)));
        assert_eq!(bits_param(&json!({"tgaBits": 32}), "file.saveAs"), Ok(Some(TgaBits::Bits32)));
        for bad in [json!(16), json!("32"), json!(32.5), json!(false)] {
            assert!(bits_param(&json!({"tgaBits": bad}), "file.saveAs").is_err(), "{bad}");
        }
    }
}
