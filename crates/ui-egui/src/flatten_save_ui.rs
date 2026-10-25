//! The flatten confirmation File › Save shows before writing a layered document back to the
//! flat file it was opened from (JPEG, PNG, WebP, or a TIFF with Preferences › File Handling ›
//! Save Layers in TIFF off), as Photoshop asks before flattening.
//!
//! The save is parked in [`Prompt`] while the modal is up (egui is immediate-mode) and written
//! once answered. **Flatten and Save** replaces the file with the flattened image; the layers stay
//! in the window until it closes. **Save As…** goes to the save dialog (a layered format is
//! suggested). Cancel drops the save. Scripts answer in advance with `file.save {flatten: true}`.

use crate::PhotocraftApp;

/// A save waiting on the flatten answer.
pub struct Prompt {
    /// The document that requested the save, even if tabs change while this is open.
    doc: photocraft_doc::DocId,
    /// The file it was opened from, which the save writes back to.
    pub path: String,
}

/// The format name shown in the prompt: the file's extension in capitals (`JPEG` for `.jpg`).
pub fn format_name(path: &str) -> String {
    match photocraft_engine::file_cmds::extension(path).as_deref() {
        Some("jpg" | "jpe" | "jpeg") => "JPEG".into(),
        Some("tif" | "tiff") => "TIFF".into(),
        Some(ext) => ext.to_ascii_uppercase(),
        None => String::new(),
    }
}

/// Parks the save behind the prompt.
pub fn park(app: &mut PhotocraftApp, path: String) -> Result<(), String> {
    let doc = app.active_doc_id()?;
    app.flatten_save = Some(Prompt { doc, path });
    Ok(())
}

/// The prompt's state for `ui.inspect` (`flattenSave`): `null` while closed.
pub fn inspect(app: &PhotocraftApp) -> serde_json::Value {
    match &app.flatten_save {
        Some(p) => serde_json::json!({"path": p.path, "format": format_name(&p.path), "document": p.doc.0}),
        None => serde_json::Value::Null,
    }
}

/// What the user picked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Flatten,
    SaveAs,
    Cancel,
}

/// Applies an answer to the parked save (the modal's buttons, and `ui.dialog.confirm` /
/// `ui.dialog.cancel` over the control channel).
pub fn answer(app: &mut PhotocraftApp, ctx: &egui::Context, answer: Answer) {
    let Some(p) = app.flatten_save.take() else { return };
    let written = match answer {
        Answer::Cancel => {
            app.ui.status = tl!("Save cancelled").into();
            app.ui.status_error = false;
            return;
        }
        // Another app changed the file since it was opened or saved: ask first (#6596).
        Answer::Flatten => app.with_document(p.doc, |app| {
            crate::disk_change_prompt::intercept(app, p.doc, &p.path, false, crate::FormatChoices::default())
                .unwrap_or_else(|| app.save_back(p.path.clone(), crate::FormatChoices::default()))
        }),
        Answer::SaveAs => app.with_document(p.doc, |app| app.save_as(None)),
    };
    match written {
        Err(e) if e == crate::file_dialog::CANCELLED => {}
        Err(e) => {
            app.ui.status = crate::i18n::fmt(tl!("Save failed: {error}"), &[("error", &e)]);
            app.ui.status_error = true;
        }
        // A finished write may release the close prompt; a background save does that when it
        // ends (`jobs_ui`), JPEG Options once answered, and Save As once its dialog is answered.
        Ok(r) if answer == Answer::Flatten && r.get("pending").is_none() && app.jpeg_options.is_none() => crate::discard_ui::saved_document(app, ctx, p.doc),
        Ok(_) => {}
    }
}

pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(p) = &app.flatten_save else { return };
    let name = crate::file_open::display_name(&p.path);
    let format = format_name(&p.path);
    let (mut flatten, mut save_as, mut cancel) = (false, false, false);
    let modal = egui::Modal::new(egui::Id::unique("flatten-save")).show(ctx, |ui| {
        ui.set_max_width(440.0);
        ui.label(egui::RichText::new(tl!("Save Flattened?")).font(crate::theme::semibold(15.0)));
        ui.add_space(4.0);
        crate::widgets::hairline(ui);
        ui.add_space(8.0);
        let message = tl!(
            "“{name}” is a {format} file, which can't keep layers. Saving flattens the image in the file; the layers stay in this window until you close it."
        );
        ui.label(crate::i18n::fmt(message, &[("name", name.as_str()), ("format", format.as_str())]));
        ui.add_space(6.0);
        ui.label(egui::RichText::new(tl!("Use Save As to keep the layers in a PSD or TIFF.")).weak());
        ui.add_space(12.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            flatten = crate::widgets::primary_button(ui, tl!("Flatten and Save"), 128.0).clicked();
            save_as = crate::widgets::secondary_button(ui, tl!("Save As…"), 96.0).clicked();
            cancel = crate::widgets::secondary_button(ui, tl!("Cancel"), 84.0).clicked();
        });
    });
    cancel |= modal.should_close();
    flatten |= ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    let picked = if flatten {
        Answer::Flatten
    } else if save_as {
        Answer::SaveAs
    } else if cancel {
        Answer::Cancel
    } else {
        return;
    };
    answer(app, ctx, picked);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_names() {
        assert_eq!(format_name("a.jpg"), "JPEG");
        assert_eq!(format_name("dir/a.JPEG"), "JPEG");
        assert_eq!(format_name("a.tif"), "TIFF");
        assert_eq!(format_name("a.png"), "PNG");
        assert_eq!(format_name("a.webp"), "WEBP");
        assert_eq!(format_name("noext"), "");
    }
}
