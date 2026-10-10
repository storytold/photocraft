//! JPEG Options: the prompt File › Save shows before writing over a flat JPEG, as Photoshop does
//! (#2223). Same pattern as `tiff_options_ui`: the save is parked in [`Prompt`] while the modal
//! is up (egui is immediate-mode) and written once answered. The quality starts at
//! Preferences › Export's JPEG quality (the scale Export As uses, 1–100).

use crate::{ExportSettings, PhotocraftApp};

/// A save waiting on the JPEG Options answer.
pub struct Prompt {
    /// The document that requested the save, even if tabs change while this is open.
    doc: photocraft_doc::DocId,
    /// Where the document goes (the JPEG it was opened from).
    pub path: String,
    /// Quality 1–100.
    pub quality: u8,
}

/// The parked save, if one is waiting.
pub fn pending(app: &PhotocraftApp) -> Option<&Prompt> {
    app.jpeg_options.as_ref()
}

/// Parks a save to `path` behind the prompt.
pub fn park(app: &mut PhotocraftApp, path: String) -> Result<(), String> {
    let doc = app.active_doc_id()?;
    let quality = app.session.prefs().export.jpeg_quality.clamp(1, 100) as u8;
    app.jpeg_options = Some(Prompt { doc, path, quality });
    Ok(())
}

pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(p) = &mut app.jpeg_options else { return };
    let (mut ok, mut cancel) = (false, false);
    let mut q = f32::from(p.quality);
    let modal = egui::Modal::new(egui::Id::new("jpeg-options")).show(ctx, |ui| {
        ui.set_max_width(420.0);
        ui.label(egui::RichText::new(tl!("JPEG Options")).font(crate::theme::semibold(15.0)));
        ui.add_space(4.0);
        crate::widgets::hairline(ui);
        ui.add_space(8.0);
        crate::widgets::slider_row(ui, tl!("Quality"), &mut q, 1.0..=100.0, "%", None);
        ui.add_space(12.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            ok = crate::widgets::primary_button(ui, tl!("OK"), 84.0).clicked();
            cancel = crate::widgets::secondary_button(ui, tl!("Cancel"), 84.0).clicked();
        });
    });
    p.quality = q.round().clamp(1.0, 100.0) as u8;
    cancel |= modal.should_close();
    if ok || ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
        finish(app, ctx, true);
    } else if cancel {
        finish(app, ctx, false);
    }
}

/// Answers the prompt: OK writes the parked save at its quality, Cancel drops it.
pub fn finish(app: &mut PhotocraftApp, ctx: &egui::Context, ok: bool) {
    let Some(p) = app.jpeg_options.take() else { return };
    if !ok {
        app.ui.status = tl!("Save cancelled").into();
        app.ui.status_error = false;
        return;
    }
    let settings = ExportSettings { jpeg_quality: Some(p.quality), ..Default::default() };
    match app.with_document(p.doc, |app| app.write_document(p.path, &settings, false)) {
        Err(e) => {
            app.ui.status = crate::i18n::fmt(tl!("Save failed: {error}"), &[("error", &e)]);
            app.ui.status_error = true;
        }
        Ok(Some(_)) => crate::discard_ui::saved_document(app, ctx, p.doc),
        // A background save releases the close prompt when it finishes (`jobs_ui`).
        Ok(None) => {}
    }
}
