//! BMP Options: the bits per pixel File › Save As and Save a Copy ask for before writing an RGB
//! BMP, as Photoshop's BMP Options does (#8374). Photoshop starts at 24 Bit; its 16-bit and
//! Advanced Modes depths aren't offered. 24 bits composite the transparency over white; 32 keep
//! it as the alpha (Photoshop's A8 R8 G8 B8, see `photocraft_io::BmpBits`).
//!
//! The save is parked in [`Prompt`] while the modal is up (egui is immediate-mode) and written
//! once answered. Cancel drops the save. Scripts skip the prompt by passing `bmpBits`.

use std::sync::Arc;

use photocraft_doc::Document;
use photocraft_io::BmpBits;

use crate::{ExportSettings, PhotocraftApp};

/// A save waiting on the BMP Options answer.
pub struct Prompt {
    /// The document that requested the save, even if tabs change while this is open.
    doc: photocraft_doc::DocId,
    /// Where the document goes.
    pub path: String,
    /// The bits per pixel chosen, starting at 24 as in Photoshop.
    pub bits: BmpBits,
    /// Save a Copy: the document keeps its path and unsaved changes.
    pub copy: bool,
    /// The step the answered save is recorded as: `file.saveAs`, `file.saveACopy`, or
    /// `file.save` when File › Save confirmed the document's own BMP.
    pub record: &'static str,
    /// Save a Copy: the document as it was when asked, written as is even if it changes while
    /// this is open.
    snapshot: Option<Arc<Document>>,
}

/// `true` when `path` names a BMP (`.bmp`, `.dib`).
pub fn is_bmp_path(path: &str) -> bool {
    photocraft_engine::file_cmds::extension(path).and_then(|e| photocraft_codecs::from_extension(&e)) == Some(photocraft_codecs::Format::Bmp)
}

/// Whether saving the active document to `path` asks for BMP Options: an RGB BMP. Grayscale and
/// Indexed BMPs are 8-bit, with nothing to choose.
pub fn wants_prompt(app: &PhotocraftApp, path: &str) -> bool {
    use photocraft_color::ColorMode;
    is_bmp_path(path) && app.session.active().is_some_and(|st| !matches!(st.doc.mode, ColorMode::Grayscale | ColorMode::Indexed | ColorMode::Bitmap))
}

/// The BMP bits a command or control call asks for (`"bmpBits": 24 | 32`), which skip the prompt.
pub fn bits_param(params: &serde_json::Value, cmd: &str) -> Result<Option<BmpBits>, String> {
    photocraft_engine::file_cmds::bmp_bits_param(params, cmd).map_err(|e| e.to_string())
}

/// Parks the save behind the prompt, starting at 24 bits. A prompt already waiting is never
/// replaced (a shortcut can still reach Save a Copy). `snapshot` is Save a Copy's document as it
/// was when asked.
pub fn park(app: &mut PhotocraftApp, path: String, copy: bool, record: &'static str, snapshot: Option<Arc<Document>>) -> Result<(), String> {
    if app.save_options_open() {
        return Err(crate::SAVE_OPTIONS_OPEN.into());
    }
    let doc = app.active_doc_id()?;
    app.bmp_options = Some(Prompt { doc, path, bits: BmpBits::Bits24, copy, record, snapshot });
    Ok(())
}

pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(p) = &mut app.bmp_options else { return };
    let (mut ok, mut cancel) = (false, false);
    let mut bits = p.bits;
    let modal = egui::Modal::new(egui::Id::new("bmp-options")).show(ctx, |ui| {
        ui.set_max_width(340.0);
        ui.label(egui::RichText::new(tl!("BMP Options")).font(crate::theme::semibold(15.0)));
        ui.add_space(4.0);
        crate::widgets::hairline(ui);
        ui.add_space(8.0);
        crate::widgets::section_label(ui, tl!("Depth"));
        ui.radio_value(&mut bits, BmpBits::Bits24, tl!("24 Bit"));
        ui.radio_value(&mut bits, BmpBits::Bits32, tl!("32 Bit"));
        ui.add_space(12.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            ok = crate::widgets::primary_button(ui, tl!("OK"), 84.0).clicked();
            cancel = crate::widgets::secondary_button(ui, tl!("Cancel"), 84.0).clicked();
        });
    });
    cancel |= modal.should_close();
    p.bits = bits;
    // A focused button takes Enter as its click without consuming the key: Enter on Cancel cancels.
    if ok || (!cancel && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))) {
        let Some(p) = app.bmp_options.take() else { return };
        // A plain Save writes whichever file the action is played on: no path.
        let mut step = if p.record == "file.save" { serde_json::json!({}) } else { serde_json::json!({"path": p.path}) };
        step["bmpBits"] = p.bits.bits().into();
        if let Some(doc) = p.snapshot {
            // Save a Copy writes the document as it was when asked.
            let choices = crate::FormatChoices { bmp_bits: Some(p.bits), ..Default::default() };
            match crate::view_cmds::write_copy(app, &doc, &p.path, choices) {
                Ok(_) => app.session.journal.push((p.record.into(), step)),
                Err(e) => {
                    app.ui.status = crate::i18n::fmt(tl!("Save failed: {error}"), &[("error", &e)]);
                    app.ui.status_error = true;
                }
            }
            return;
        }
        // The document may have changed while the prompt was open (a layer added to a flat one):
        // whether this BMP is only a copy is decided now, as it would be without the prompt.
        let copy = app.with_document(p.doc, |app| app.saves_copy(&p.path, None)).unwrap_or(p.copy);
        let settings = ExportSettings { bmp_bits: Some(p.bits), ..app.save_settings() };
        app.write_answered_save(ctx, p.doc, p.path, &settings, copy, (p.record, step));
    } else if cancel {
        app.bmp_options = None;
        app.ui.status = tl!("Save cancelled").into();
        app.ui.status_error = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bmp_names() {
        assert!(is_bmp_path("a.bmp"));
        assert!(is_bmp_path("dir/sub/a.BMP"));
        assert!(is_bmp_path("C:\\x\\y.dib"));
        assert!(!is_bmp_path("a.bmp.psd"));
        assert!(!is_bmp_path(".bmp"));
        assert!(!is_bmp_path("a.png"));
    }

    #[test]
    fn bits_params() {
        use serde_json::json;
        assert_eq!(bits_param(&json!({}), "file.saveAs"), Ok(None));
        assert_eq!(bits_param(&json!({"bmpBits": null}), "file.saveAs"), Ok(None));
        assert_eq!(bits_param(&json!({"bmpBits": 24}), "file.saveAs"), Ok(Some(BmpBits::Bits24)));
        assert_eq!(bits_param(&json!({"bmpBits": 32}), "file.saveAs"), Ok(Some(BmpBits::Bits32)));
        for bad in [json!(16), json!("32"), json!(32.5), json!(false)] {
            assert!(bits_param(&json!({"bmpBits": bad}), "file.saveAs").is_err(), "{bad}");
        }
    }
}
