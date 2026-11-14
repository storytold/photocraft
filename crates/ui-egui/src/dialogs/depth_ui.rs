//! Estimate Depth: the Surface popover's button. With the depth model installed it runs
//! `image.estimateDepth`; otherwise a consent dialog offers the one-time download (or loading the
//! two model files from disk), shows its progress, and runs the estimate when the model is ready.
//! Every action is a command (`depth.*`, `image.estimateDepth`); this file only sequences them.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::file_dialog::{FileDialogAnswer, FileDialogRequest};

/// Which step the dialog shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DepthPhase {
    /// Asking before the download.
    #[default]
    Consent,
    /// The download runs; `done` and `total` are bytes.
    Downloading,
    /// Choosing the two model files from disk.
    Files,
}

/// The dialog's state, kept in the UI state so `ui.inspect` shows it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DepthDialog {
    pub phase: DepthPhase,
    pub done: u64,
    pub total: u64,
    /// The last error, shown in the dialog.
    pub error: Option<String>,
    /// The DINOv2 backbone file chosen in [`DepthPhase::Files`].
    pub dinov2: String,
    /// The depth head file chosen in [`DepthPhase::Files`].
    pub head: String,
}

/// What a `depth.modelStatus` reply asks of the dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Next {
    /// Nothing: keep showing the dialog.
    Wait,
    /// The model is installed: close the dialog and estimate.
    Estimate,
}

/// Fold one `depth.modelStatus` reply into the dialog: progress while downloading, the error if
/// the download failed (back to the consent step), and [`Next::Estimate`] once installed.
pub fn apply_status(d: &mut DepthDialog, status: &Value) -> Next {
    if status.get("installed").and_then(Value::as_bool) == Some(true) {
        return Next::Estimate;
    }
    if let Some(p) = status.get("downloading").filter(|v| !v.is_null()) {
        d.phase = DepthPhase::Downloading;
        d.done = p.get("done").and_then(Value::as_u64).unwrap_or(0);
        d.total = p.get("total").and_then(Value::as_u64).unwrap_or(0);
    } else if d.phase == DepthPhase::Downloading {
        // The download ended without installing: it failed or was cancelled.
        d.phase = DepthPhase::Consent;
        d.error = status.get("error").and_then(Value::as_str).map(str::to_owned);
    }
    Next::Wait
}

fn status(app: &mut PhotocraftApp) -> Result<Value, String> {
    app.session.execute("depth.modelStatus", json!({})).map_err(|e| e.to_string())
}

/// Run `image.estimateDepth` into the brush's depth channel, as a background job when jobs are on.
pub fn estimate(app: &mut PhotocraftApp) {
    let surface = &app.session.tools.brush.surface;
    let (channel, quality) = (surface.depth_channel.clone(), if surface.depth_high_quality { "high" } else { "fast" });
    if let Err(e) = app.run("image.estimateDepth", json!({"channel": channel, "quality": quality})) {
        match app.ui.depth_dialog.as_mut() {
            Some(d) => d.error = Some(e),
            None => {
                app.ui.status = e;
                app.ui.status_error = true;
            }
        }
    }
}

/// The Estimate Depth… button: estimate at once when the model is installed, else ask first.
pub fn start(app: &mut PhotocraftApp) {
    match status(app) {
        Ok(s) if s.get("installed").and_then(Value::as_bool) == Some(true) => estimate(app),
        Ok(s) if s.get("supported").and_then(Value::as_bool) == Some(false) => {
            app.ui.status = tl!("Not available in this build").into();
            app.ui.status_error = true;
        }
        Ok(_) => app.ui.depth_dialog = Some(DepthDialog::default()),
        Err(e) => {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
}

/// Start the download (the consent was given).
pub fn download(app: &mut PhotocraftApp) {
    let Some(d) = app.ui.depth_dialog.as_mut() else { return };
    d.error = None;
    match app.run("depth.downloadModel", json!({})) {
        Ok(_) => {
            if let Some(d) = app.ui.depth_dialog.as_mut() {
                d.phase = DepthPhase::Downloading;
                d.done = 0;
                d.total = 0;
            }
        }
        Err(e) => {
            if let Some(d) = app.ui.depth_dialog.as_mut() {
                d.error = Some(e);
            }
        }
    }
}

/// Cancel: stop a running download and close.
pub fn cancel(app: &mut PhotocraftApp) {
    if app.ui.depth_dialog.as_ref().is_some_and(|d| d.phase == DepthPhase::Downloading) {
        let _ = app.run("depth.cancelDownload", json!({}));
    }
    app.ui.depth_dialog = None;
}

/// Load the two chosen files (`depth.loadModelFiles`), then estimate.
pub fn load_files(app: &mut PhotocraftApp) {
    let Some(d) = app.ui.depth_dialog.as_ref() else { return };
    let params = json!({"dinov2": d.dinov2, "head": d.head});
    match app.run("depth.loadModelFiles", params) {
        Ok(_) => {
            app.ui.depth_dialog = None;
            estimate(app);
        }
        Err(e) => {
            if let Some(d) = app.ui.depth_dialog.as_mut() {
                d.error = Some(e);
            }
        }
    }
}

/// Ask for one model file's path; `set` stores it in the dialog.
fn pick(app: &mut PhotocraftApp, set: fn(&mut DepthDialog, String)) {
    let request = FileDialogRequest::Open { multiple: false, initial_dir: None, extensions: Some(vec!["safetensors".into()]) };
    let asked = app.ask_file(request, move |app, answer| {
        if let FileDialogAnswer::Paths(paths) = answer
            && let Some(path) = paths.into_iter().next()
            && let Some(d) = app.ui.depth_dialog.as_mut()
        {
            set(d, path);
        }
        Ok(Value::Null)
    });
    if let (Err(e), Some(d)) = (asked, app.ui.depth_dialog.as_mut()) {
        d.error = Some(e);
    }
}

/// Draw the dialog and step it: poll the download, close and estimate when the model is ready.
pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(phase) = app.ui.depth_dialog.as_ref().map(|d| d.phase) else { return };
    if phase == DepthPhase::Downloading {
        // Like the update check: poll while it runs.
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
        if let Ok(s) = status(app) {
            let next = app.ui.depth_dialog.as_mut().map(|d| apply_status(d, &s));
            if next == Some(Next::Estimate) {
                app.ui.depth_dialog = None;
                estimate(app);
                return;
            }
        }
    }
    let Some(d) = app.ui.depth_dialog.clone() else { return };
    #[derive(PartialEq)]
    enum Act {
        None,
        Download,
        Files,
        Pick(bool),
        Load,
        Cancel,
    }
    let mut act = Act::None;
    egui::Window::new(tl!("Estimate Depth")).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO).show(ctx, |ui| {
        ui.set_max_width(380.0);
        match d.phase {
            DepthPhase::Consent => {
                ui.label(tl!("Estimate Depth needs a depth model (about 95 MB, Apache-2.0) downloaded once from huggingface.co. Download it now?"));
            }
            DepthPhase::Downloading => {
                ui.label(tl!("Downloading the depth model…"));
                let fraction = if d.total > 0 { (d.done as f32 / d.total as f32).clamp(0.0, 1.0) } else { 0.0 };
                ui.add(egui::ProgressBar::new(fraction).show_percentage());
            }
            DepthPhase::Files => {
                ui.label(tl!("Choose the two model files (.safetensors): the DINOv2 backbone and the depth head."));
                for (is_head, label, path) in [(false, tl!("DINOv2 backbone…"), &d.dinov2), (true, tl!("Depth head…"), &d.head)] {
                    ui.horizontal(|ui| {
                        if ui.button(label).clicked() {
                            act = Act::Pick(is_head);
                        }
                        ui.label(if path.is_empty() { "-" } else { path.as_str() });
                    });
                }
            }
        }
        if let Some(e) = &d.error {
            ui.colored_label(crate::theme::Tokens::get(ui.ctx()).danger, e);
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            match d.phase {
                DepthPhase::Consent => {
                    if ui.button(tl!("Download")).clicked() {
                        act = Act::Download;
                    }
                    if ui.button(tl!("Load Files…")).clicked() {
                        act = Act::Files;
                    }
                }
                DepthPhase::Files => {
                    let ready = !d.dinov2.is_empty() && !d.head.is_empty();
                    if ui.add_enabled(ready, egui::Button::new(tl!("Load"))).clicked() {
                        act = Act::Load;
                    }
                }
                DepthPhase::Downloading => {}
            }
            if ui.button(tl!("Cancel")).clicked() {
                act = Act::Cancel;
            }
        });
    });
    match act {
        Act::None => {}
        Act::Download => download(app),
        Act::Files => {
            if let Some(d) = app.ui.depth_dialog.as_mut() {
                d.phase = DepthPhase::Files;
                d.error = None;
            }
        }
        Act::Pick(false) => pick(app, |d, p| d.dinov2 = p),
        Act::Pick(true) => pick(app, |d, p| d.head = p),
        Act::Load => load_files(app),
        Act::Cancel => cancel(app),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_then_installed_estimates() {
        let mut d = DepthDialog { phase: DepthPhase::Downloading, ..Default::default() };
        assert_eq!(apply_status(&mut d, &json!({"installed": false, "downloading": {"done": 10, "total": 40}})), Next::Wait);
        assert_eq!((d.done, d.total, d.phase), (10, 40, DepthPhase::Downloading));
        assert_eq!(apply_status(&mut d, &json!({"installed": true, "downloading": null})), Next::Estimate);
    }

    #[test]
    fn a_failed_download_returns_to_consent_with_the_error() {
        let mut d = DepthDialog { phase: DepthPhase::Downloading, ..Default::default() };
        assert_eq!(apply_status(&mut d, &json!({"installed": false, "downloading": null, "error": "offline"})), Next::Wait);
        assert_eq!((d.phase, d.error.as_deref()), (DepthPhase::Consent, Some("offline")));
    }

    #[test]
    fn consent_shows_no_progress_or_error_state() {
        let mut d = DepthDialog::default();
        assert_eq!(apply_status(&mut d, &json!({"installed": false, "downloading": null})), Next::Wait);
        assert_eq!(d, DepthDialog::default());
    }

    #[test]
    fn the_state_round_trips_for_ui_inspect() {
        let d = DepthDialog { phase: DepthPhase::Files, dinov2: "a".into(), ..Default::default() };
        let back: DepthDialog = serde_json::from_value(serde_json::to_value(&d).unwrap()).unwrap();
        assert_eq!(back, d);
    }
}
