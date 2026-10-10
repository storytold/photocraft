//! UI integration for application update checking (manual and periodic background checks).

use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::state::DialogKind;
use crate::theme::Tokens;

/// Outcome of an update check.
#[derive(Debug, Clone)]
pub enum UpdateCheckOutcome {
    Available(photocraft_engine::update::ReleaseInfo),
    UpToDate,
    Failed(String),
}

/// Manual update check initiated by the user (Help › Check for Updates).
pub fn check_manual(app: &mut PhotocraftApp) {
    app.ui.status = tl!("Checking for updates…").to_string();
    app.ui.status_error = false;

    #[cfg(not(target_arch = "wasm32"))]
    {
        let (tx, rx) = std::sync::mpsc::channel();
        let current_ver = photocraft_engine::build_info::VERSION.to_string();
        std::thread::spawn(move || {
            let outcome = match photocraft_engine::update::check_for_update(&current_ver, 10) {
                Ok(Some(release)) => UpdateCheckOutcome::Available(release),
                Ok(None) => UpdateCheckOutcome::UpToDate,
                Err(err) => UpdateCheckOutcome::Failed(err),
            };
            let _ = tx.send((true, outcome));
        });
        app.update_rx = Some(rx);
    }

    #[cfg(target_arch = "wasm32")]
    {
        crate::notices::post(app, tl!("Check for Updates"), vec![tl!("Web builds are always running the latest version.").to_string()], false, None);
    }
}

/// Periodic background check executed on startup if enabled and rate limit interval has passed.
pub fn check_periodic(app: &mut PhotocraftApp) {
    if !app.session.prefs().general.check_for_updates {
        return;
    }

    let now_secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);

    let last_check = app.session.prefs().dialogs.get("help.checkForUpdates.lastCheck").and_then(Value::as_u64);

    if !photocraft_engine::update::should_check_periodically(last_check, now_secs) {
        return;
    }

    // Update the timestamp to prevent spammed retries
    app.session.prefs.edit(|p| {
        p.dialogs.insert("help.checkForUpdates.lastCheck".into(), json!(now_secs));
    });

    #[cfg(not(target_arch = "wasm32"))]
    {
        let (tx, rx) = std::sync::mpsc::channel();
        let current_ver = photocraft_engine::build_info::VERSION.to_string();
        std::thread::spawn(move || {
            let outcome = match photocraft_engine::update::check_for_update(&current_ver, 10) {
                Ok(Some(release)) => UpdateCheckOutcome::Available(release),
                Ok(None) => UpdateCheckOutcome::UpToDate,
                Err(err) => UpdateCheckOutcome::Failed(err),
            };
            let _ = tx.send((false, outcome));
        });
        app.update_rx = Some(rx);
    }
}

/// Polls for completed update checks and updates UI state accordingly.
pub fn poll(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(rx) = &app.update_rx else { return };
    match rx.try_recv() {
        Ok((is_manual, outcome)) => {
            app.update_rx = None;
            let now_secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            app.session.prefs.edit(|p| {
                p.dialogs.insert("help.checkForUpdates.lastCheck".into(), json!(now_secs));
            });

            match outcome {
                UpdateCheckOutcome::Available(release) => {
                    let mut fields = Map::new();
                    fields.insert("version".into(), json!(release.version));
                    fields.insert("currentVersion".into(), json!(photocraft_engine::build_info::VERSION));
                    fields.insert("body".into(), json!(release.body));
                    fields.insert("htmlUrl".into(), json!(release.html_url));
                    fields.insert("name".into(), json!(release.name));
                    app.ui.open_dialog(DialogKind::Update, fields);
                    app.ui.status = format!("Update available: PhotoCraft {}", release.version);
                    ctx.request_repaint();
                }
                UpdateCheckOutcome::UpToDate => {
                    if is_manual {
                        crate::notices::post(
                            app,
                            tl!("You're up to date"),
                            vec![format!("PhotoCraft {} is currently the newest version.", photocraft_engine::build_info::VERSION)],
                            false,
                            None,
                        );
                        app.ui.status = format!("PhotoCraft {} is up to date", photocraft_engine::build_info::VERSION);
                        ctx.request_repaint();
                    }
                }
                UpdateCheckOutcome::Failed(err) => {
                    if is_manual {
                        crate::notices::error(app, format!("Could not check for updates: {err}"));
                        ctx.request_repaint();
                    }
                }
            }
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => {}
        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
            app.update_rx = None;
        }
    }
}

/// Renders the update dialog body.
pub fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, fields: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    let version = fields.get("version").and_then(Value::as_str).unwrap_or("");
    let current = fields.get("currentVersion").and_then(Value::as_str).unwrap_or(photocraft_engine::build_info::VERSION);
    let name = fields.get("name").and_then(Value::as_str).unwrap_or("");
    let body = fields.get("body").and_then(Value::as_str).unwrap_or("");
    let html_url = fields.get("htmlUrl").and_then(Value::as_str).unwrap_or("").to_string();

    let title_text = if !name.is_empty() { name.to_string() } else { format!("PhotoCraft {version}") };

    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            ui.heading(&title_text);
        });
        ui.label(egui::RichText::new(format!("Current version: {current}")).color(t.text_dim));
        ui.add_space(8.0);

        ui.label(egui::RichText::new(tl!("What's new in this release:")).strong());
        ui.add_space(4.0);

        egui::Frame::NONE.fill(t.field).stroke(egui::Stroke::new(1.0, t.field_border)).corner_radius(t.radius).inner_margin(egui::Margin::same(8)).show(
            ui,
            |ui| {
                ui.set_min_width(460.0);
                egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                    ui.label(egui::RichText::new(body).color(t.text));
                });
            },
        );

        ui.add_space(8.0);
        if !html_url.is_empty() {
            ui.horizontal(|ui| {
                if ui.link(egui::RichText::new(tl!("Open Release Page on GitHub")).color(t.accent)).clicked() {
                    crate::links::open(app, ui.ctx(), &html_url);
                }
            });
        }
    });
}
