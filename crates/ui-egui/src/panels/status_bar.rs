use std::cell::RefCell;

use craft::behavior::kurbo::Size;
use craft::theme::variant::Tone;
use craft::ui::bars::{Bar, BarKind, StatusMessage, StatusReadout};
use craft::ui::contract::{Bind, ShowContainer as _};
use craft::ui::runtime::commands::CommandRef;

use super::{PhotocraftApp, chrome_ui};

// ----------------------------------------------------------------------------- status bar

/// The status bar, in the shell's status slot: the zoom readout of the active document and the latest status
/// message, and at the far end the render-path note, the background redraw and the background jobs' progress with
/// Cancel (#210).
pub fn status_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let values = chrome_ui::status_values(app);
    let item = chrome_ui::status_item(&app.ui.chrome);
    let document = app.session.active().map(|st| Size::new(st.doc.size.width as f64, st.doc.size.height as f64));
    // The active document's main canvas as its last pass showed it; a document not yet shown has no readout.
    let mut view = crate::canvas::active_view(app);
    let message =
        (!app.ui.status.is_empty()).then(|| StatusMessage::new(app.ui.status.clone()).tone(if app.ui.status_error { Tone::Warn } else { Tone::Neutral }));
    let render_path = chrome_ui::render_path_label(app)
        .map(|(label, tip, command)| StatusMessage::new(label).tone(Tone::Warn).command(CommandRef::new(command)).tooltip(tip));
    // The passes of a frame each report their own entries; the last pass's are the ones applied below.
    let mut picked = None;
    let mut invalid_zoom = None;
    let mut view_edited = false;
    // The bar runs the trailing group before the body when it measures, so the app is borrowed by one closure at a
    // time.
    let cell = RefCell::new(&mut *app);
    Bar::new("", BarKind::Status)
        .trailing(|ui, _| {
            if let Some(shown) = &render_path {
                shown.show(ui);
            }
            let mut guard = cell.borrow_mut();
            let app = &mut **guard;
            if let Some(progress) = crate::background_redraw::progress(app) {
                progress.show(ui);
            }
            crate::jobs_ui::status_progress(app, ui);
        })
        .show(ui, |ui, _| {
            match (document, view.as_mut()) {
                (Some(size), Some(view)) => {
                    let out = StatusReadout::new(Bind::bound(view), size, item, &values).show(ui);
                    picked = out.item.or(picked);
                    invalid_zoom = out.invalid_zoom.or(invalid_zoom.take());
                    view_edited |= !out.view.is_empty();
                }
                (Some(_), None) => {}
                (None, _) => {
                    StatusMessage::new(tl!("No document")).show(ui);
                }
            }
            // A command run with no document open still reports here (#1567: File › Automate › Batch).
            if let Some(shown) = &message {
                shown.show(ui);
            }
        });
    if let Some(picked) = picked {
        chrome_ui::set_status_item(&mut app.ui.chrome, picked);
    }
    if view_edited
        && let Some(new) = &view
        && let Some(i) = app.session.active_index()
    {
        let ppp = app.canvas_ppp();
        app.ui.views[i].set_from(new, ppp);
    }
    chrome_ui::zoom_alert(app, invalid_zoom);
}
