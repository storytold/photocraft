//! Automatic submissions wait for the active background job to finish.

use super::{PhotocraftApp, show_result, start_submit};

pub(super) fn enqueue(app: &mut PhotocraftApp, path: &str) {
    // Bound memory even when automation produces saves faster than the network.
    if app.deviantart_pending.len() >= 128 {
        app.ui.status = tl!("A DeviantArt submit is already running.").to_string();
        app.ui.status_error = true;
        show_result(app, app.ui.status.clone(), None);
        return;
    }
    app.deviantart_pending.push_back(path.to_string());
}

pub(super) fn start_next(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if app.deviantart_job.is_some() {
        return;
    }
    if !app.deviantart.ready() {
        return;
    }
    while let Some(path) = app.deviantart_pending.pop_front() {
        match start_submit(app, path, false) {
            Ok(()) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
                break;
            }
            Err(error) => {
                app.ui.status = error;
                app.ui.status_error = true;
                show_result(app, app.ui.status.clone(), None);
            }
        }
    }
}
