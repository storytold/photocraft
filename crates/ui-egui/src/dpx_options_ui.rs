//! DPX Options: the bits per sample File › Save As and Save a Copy ask for before writing a DPX
//! frame (#1049). Photoshop has no DPX writer, so this offers the depths film tools write: 8, 10
//! (the usual film-scan depth), 12 and 16 (see `photocraft_io::DpxBits`). 10 and 12 bits keep
//! fewer bits than a 16-bit document holds, which the save reports as a warning.
//!
//! The save is parked in [`Prompt`] while the modal is up (egui is immediate-mode) and written
//! once answered. Cancel drops the save. Scripts skip the prompt by passing `dpxBits`.

use photocraft_io::DpxBits;

use crate::{ExportSettings, PhotocraftApp};

/// The depths offered, in the order shown.
const ALL: [DpxBits; 4] = [DpxBits::Bits8, DpxBits::Bits10, DpxBits::Bits12, DpxBits::Bits16];

/// A save waiting on the DPX Options answer.
pub struct Prompt {
    /// The document that requested the save, even if tabs change while this is open.
    doc: photocraft_doc::DocId,
    /// Where the frame goes.
    pub path: String,
    /// The bits per sample chosen, starting at [`default_bits`].
    pub bits: DpxBits,
    /// Save a Copy: the document keeps its path and unsaved changes.
    pub copy: bool,
    /// The step the answered save is recorded as (`file.saveAs` or `file.saveACopy`), so an
    /// action being recorded keeps it; `file.save` when File › Save confirmed the document's own
    /// frame (#3531), which plays back on the file the action is played on.
    pub record: &'static str,
}

/// `true` when `path` names a DPX frame.
pub fn wants_prompt(path: &str) -> bool {
    photocraft_engine::file_cmds::extension(path).and_then(|e| photocraft_codecs::from_extension(&e)) == Some(photocraft_codecs::Format::Dpx)
}

/// The DPX bits a command or control call asks for (`"dpxBits": 8 | 10 | 12 | 16`), which skip
/// the prompt.
pub fn bits_param(params: &serde_json::Value, cmd: &str) -> Result<Option<DpxBits>, String> {
    photocraft_engine::file_cmds::dpx_bits_param(params, cmd).map_err(|e| e.to_string())
}

/// What a save writes without a choice: 8 bits for an 8-bit document, else 16.
pub fn default_bits(depth: photocraft_color::SampleType) -> DpxBits {
    if depth == photocraft_color::SampleType::U8 { DpxBits::Bits8 } else { DpxBits::Bits16 }
}

/// Parks the save behind the prompt, starting at the document's own depth. A prompt already
/// waiting is never replaced.
pub fn park(app: &mut PhotocraftApp, path: String, copy: bool, record: &'static str) -> Result<(), String> {
    if app.save_options_open() {
        return Err(crate::SAVE_OPTIONS_OPEN.into());
    }
    let doc = app.active_doc_id()?;
    let bits = app.session.active().map_or(DpxBits::Bits16, |st| default_bits(st.doc.depth));
    app.dpx_options = Some(Prompt { doc, path, bits, copy, record });
    Ok(())
}

fn label(bits: DpxBits) -> &'static str {
    match bits {
        DpxBits::Bits8 => tl!("8 Bits/Channel"),
        DpxBits::Bits10 => tl!("10 Bits/Channel"),
        DpxBits::Bits12 => tl!("12 Bits/Channel"),
        DpxBits::Bits16 => tl!("16 Bits/Channel"),
    }
}

pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(p) = &mut app.dpx_options else { return };
    let (mut ok, mut cancel) = (false, false);
    let mut bits = p.bits;
    let modal = egui::Modal::new(egui::Id::new("dpx-options")).show(ctx, |ui| {
        ui.set_max_width(340.0);
        ui.label(egui::RichText::new(tl!("DPX Options")).font(crate::theme::semibold(15.0)));
        ui.add_space(4.0);
        crate::widgets::hairline(ui);
        ui.add_space(8.0);
        crate::widgets::section_label(ui, tl!("Bit depth"));
        for b in ALL {
            ui.radio_value(&mut bits, b, label(b));
        }
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
        let Some(p) = app.dpx_options.take() else { return };
        let settings = ExportSettings { dpx_bits: Some(p.bits), ..app.save_settings() };
        // A plain Save writes whichever file the action is played on (#3531): no path.
        let mut step = if p.record == "file.save" { serde_json::json!({}) } else { serde_json::json!({"path": p.path}) };
        step["dpxBits"] = p.bits.bits().into();
        app.write_answered_save(ctx, p.doc, p.path, &settings, p.copy, (p.record, step));
    } else if cancel {
        app.dpx_options = None;
        app.ui.status = tl!("Save cancelled").into();
        app.ui.status_error = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::SampleType;

    #[test]
    fn dpx_names() {
        assert!(wants_prompt("a.dpx"));
        assert!(wants_prompt("dir/sub/a.DPX"));
        assert!(!wants_prompt("a.dpx.psd"));
        assert!(!wants_prompt(".dpx"));
        assert!(!wants_prompt("a.cin"));
    }

    #[test]
    fn bits_params_and_defaults() {
        use serde_json::json;
        assert_eq!(bits_param(&json!({}), "file.saveAs"), Ok(None));
        assert_eq!(bits_param(&json!({"dpxBits": null}), "file.saveAs"), Ok(None));
        for b in ALL {
            assert_eq!(bits_param(&json!({"dpxBits": b.bits()}), "file.saveAs"), Ok(Some(b)));
        }
        for bad in [json!(24), json!("10"), json!(10.5), json!(-8), json!(true)] {
            assert!(bits_param(&json!({"dpxBits": bad}), "file.saveAs").is_err(), "{bad}");
        }
        assert_eq!(default_bits(SampleType::U8), DpxBits::Bits8);
        assert_eq!(default_bits(SampleType::U16), DpxBits::Bits16);
        assert_eq!(default_bits(SampleType::F32), DpxBits::Bits16);
    }

    /// The answer reaches the DPX writer: the header's bits per sample (byte 803) is the one chosen.
    #[test]
    fn the_answer_is_the_depth_written() {
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", serde_json::json!({"width": 4, "height": 2})).unwrap();
        let doc = &session.active().unwrap().doc;
        for (bits, want) in [(None, 8), (Some(DpxBits::Bits10), 10), (Some(DpxBits::Bits12), 12), (Some(DpxBits::Bits16), 16)] {
            let opts = ExportSettings { dpx_bits: bits, ..ExportSettings::default() }.io_options();
            let bytes = photocraft_io::export(doc, "a.dpx", &opts).unwrap().bytes;
            assert_eq!((&bytes[..4], bytes[803]), (&b"SDPX"[..], want), "{bits:?}");
        }
    }
}
