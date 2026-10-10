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
    if app.update_rx.is_some() {
        return;
    }
    app.ui.status = tl!("Checking for updates…").to_string();
    app.ui.status_error = false;

    #[cfg(not(target_arch = "wasm32"))]
    {
        let (tx, rx) = std::sync::mpsc::channel();
        let current_ver = photocraft_engine::build_info::VERSION.to_string();
        if let Err(error) = std::thread::Builder::new().name("update-check".into()).spawn(move || {
            let _ = tx.send((true, fetch(&current_ver)));
        }) {
            crate::notices::error(app, format!("Could not start update check: {error}"));
            return;
        }
        app.update_rx = Some(rx);
    }

    #[cfg(target_arch = "wasm32")]
    {
        crate::notices::post(app, tl!("Check for Updates"), vec![tl!("Web builds are always running the latest version.").to_string()], false, None);
    }
}

/// Periodic background check executed on startup if enabled and rate limit interval has passed.
pub fn check_periodic(app: &mut PhotocraftApp) {
    if !app.session.prefs().general.check_for_updates || app.update_rx.is_some() || cfg!(target_arch = "wasm32") {
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
        if let Err(error) = std::thread::Builder::new().name("update-check".into()).spawn(move || {
            let _ = tx.send((false, fetch(&current_ver)));
        }) {
            log::warn!("Could not start update check: {error}");
            return;
        }
        app.update_rx = Some(rx);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn fetch(current: &str) -> UpdateCheckOutcome {
    match std::panic::catch_unwind(|| photocraft_engine::update::check_for_update(current, 10)) {
        Ok(Ok(Some(release))) => UpdateCheckOutcome::Available(release),
        Ok(Ok(None)) => UpdateCheckOutcome::UpToDate,
        Ok(Err(error)) => UpdateCheckOutcome::Failed(error),
        Err(_) => UpdateCheckOutcome::Failed("Update check failed unexpectedly".into()),
    }
}

/// Polls for completed update checks and updates UI state accordingly.
pub fn poll(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(rx) = &app.update_rx else { return };
    if !app.ui.dialogs.is_empty() {
        return;
    }
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
                    #[cfg(not(target_arch = "wasm32"))]
                    let appimage = std::env::var_os("APPIMAGE").is_some();
                    #[cfg(target_arch = "wasm32")]
                    let appimage = false;
                    if let Some(url) = release.download_url(std::env::consts::OS, std::env::consts::ARCH, appimage) {
                        fields.insert("downloadUrl".into(), json!(url));
                    }
                    fields.insert("tagName".into(), json!(release.tag_name));
                    // A build with a native updater (macOS with a signed Sparkle feed) installs
                    // in-app; the rest follow the release's download links.
                    fields.insert("nativeUpdater".into(), json!(app.services.install_update.is_some()));
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
        Err(std::sync::mpsc::TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(100)),
        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
            app.update_rx = None;
            log::warn!("Update-check worker disconnected");
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
        if fields.get("nativeUpdater").and_then(Value::as_bool).unwrap_or(false) {
            // macOS with a signed Sparkle feed: the update installs itself, so no download to fetch.
            ui.label(tl!(
                "PhotoCraft downloads the update in the background and installs it when you quit, so your session is not interrupted. Save your work first."
            ));
        } else if fields.get("downloadUrl").and_then(Value::as_str).is_some() {
            ui.label(tl!("Download the update and save your work before installing the new version."));
        } else {
            ui.label(tl!("For package-managed installations, update PhotoCraft through your package manager. Other downloads are on the release page."));
        }
        if photocraft_engine::update::trusted_release_url(&html_url) {
            ui.horizontal(|ui| {
                if ui.link(egui::RichText::new(tl!("Open Release Page on GitHub")).color(t.accent)).clicked() {
                    crate::links::open(app, ui.ctx(), &html_url);
                }
            });
        }
    });
}

/// The dialog's default action. A build with a native updater (macOS with a signed Sparkle feed)
/// hands the installation to it, which fetches the signed update and replaces the app on quit;
/// every other build opens the release's download in the browser.
pub fn install(app: &mut PhotocraftApp, fields: &Map<String, Value>) -> Result<Value, String> {
    if fields.get("nativeUpdater").and_then(Value::as_bool).unwrap_or(false) {
        let updater = app.services.install_update.as_mut().ok_or("The in-app updater is no longer available")?;
        updater()?;
        app.ui.status = tl!("PhotoCraft will install the update when you quit.").to_string();
        return Ok(json!({"updater": "native"}));
    }
    open_download(app, fields)
}

pub fn open_download(app: &PhotocraftApp, fields: &Map<String, Value>) -> Result<Value, String> {
    let tag = fields.get("tagName").and_then(Value::as_str).unwrap_or("");
    let url = if let Some(url) = fields.get("downloadUrl").and_then(Value::as_str) {
        if !photocraft_engine::update::trusted_download_url(url, tag) {
            return Err("Untrusted update download URL".into());
        }
        url
    } else {
        let url = fields.get("htmlUrl").and_then(Value::as_str).ok_or("Missing release URL")?;
        if !photocraft_engine::update::trusted_release_url(url) {
            return Err("Untrusted update release URL".into());
        }
        url
    };
    let open = app.services.open_url.as_ref().ok_or("Opening update downloads is unavailable on this platform")?;
    open(url)?;
    Ok(json!({"url": url}))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOWNLOAD: &str = "https://github.com/storytold/photocraft/releases/download/v99.0.0/photocraft-99.0.0-macos-universal.dmg";
    static CHECKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    fn available() -> photocraft_engine::update::ReleaseInfo {
        photocraft_engine::update::parse_release(&json!({
            "tag_name":"v99.0.0", "name":"PhotoCraft 99.0.0", "body":"Improved editing and export.",
            "html_url":"https://github.com/storytold/photocraft/releases/tag/v99.0.0"
        }))
        .unwrap()
    }

    #[test]
    fn general_preferences_expose_update_controls_and_preserve_opt_out() {
        use egui_kittest::{Harness, kittest::Queryable};
        let old: photocraft_engine::prefs::Preferences = serde_json::from_value(json!({"general":{}})).unwrap();
        assert!(old.general.check_for_updates);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let id = crate::prefs_ui::open_preferences(&mut app, "general");
        let mut h = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            app
        });
        h.run_steps(4);
        h.get_by_label("Automatically Check for Updates at Startup").click();
        h.run_steps(2);
        let _ = h.get_by_label("Check for Updates…");
        crate::dialogs::confirm(h.state_mut(), id).unwrap();
        assert!(!h.state().session.prefs().general.check_for_updates);
        let saved = h.state().session.prefs().to_json();
        let restored: photocraft_engine::prefs::Preferences = serde_json::from_value(saved).unwrap();
        assert!(!restored.general.check_for_updates);
    }

    #[test]
    fn background_prompt_waits_for_existing_dialogs() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        let id = app.ui.open_dialog(DialogKind::About, Map::new());
        let (tx, rx) = std::sync::mpsc::channel();
        app.update_rx = Some(rx);
        tx.send((false, UpdateCheckOutcome::Available(available()))).unwrap();
        poll(&mut app, &ctx);
        assert_eq!(app.ui.dialogs.len(), 1);
        assert!(app.update_rx.is_some());
        app.ui.close_dialog(id);
        poll(&mut app, &ctx);
        assert_eq!(app.ui.dialogs[0].kind, DialogKind::Update);
        assert_eq!(app.ui.dialogs[0].fields["version"], "99.0.0");
        assert!(app.update_rx.is_none());
    }

    #[test]
    fn opt_out_and_in_flight_checks_do_not_start_network_work() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.prefs.edit(|p| p.general.check_for_updates = false);
        check_periodic(&mut app);
        assert!(app.update_rx.is_none());
        let (tx, rx) = std::sync::mpsc::channel();
        app.update_rx = Some(rx);
        check_manual(&mut app);
        tx.send((true, UpdateCheckOutcome::UpToDate)).unwrap();
        assert!(app.update_rx.as_ref().unwrap().try_recv().is_ok());
    }

    /// macOS with a signed Sparkle feed installs in-app instead of opening a download in the browser.
    #[test]
    fn the_native_updater_is_preferred_and_the_prompt_says_so() {
        let mut app = PhotocraftApp::new(
            photocraft_engine::Session::new(),
            crate::Services {
                install_update: Some(Box::new(|| {
                    CHECKED.store(true, std::sync::atomic::Ordering::Relaxed);
                    Ok(())
                })),
                open_url: Some(Box::new(|_| panic!("the browser must not open when Sparkle installs"))),
                ..Default::default()
            },
        );
        let fields = Map::from_iter([("nativeUpdater".into(), json!(true)), ("downloadUrl".into(), json!(DOWNLOAD))]);
        let id = app.ui.open_dialog(DialogKind::Update, fields.clone());
        assert_eq!(crate::dialogs::confirm(&mut app, id).unwrap(), json!({"updater": "native"}));
        assert!(CHECKED.load(std::sync::atomic::Ordering::Relaxed));
        assert_eq!(app.ui.status, "PhotoCraft will install the update when you quit.");
    }

    /// Without an updater the same dialog still opens the verified download, and a stale
    /// `nativeUpdater` flag (the updater went away) is an error, not a silent browser hand-off.
    #[test]
    fn without_a_native_updater_the_download_still_opens() {
        let mut app = PhotocraftApp::new(
            photocraft_engine::Session::new(),
            crate::Services {
                open_url: Some(Box::new(|url| {
                    assert_eq!(url, DOWNLOAD);
                    Ok(())
                })),
                ..Default::default()
            },
        );
        let fields = Map::from_iter([("tagName".into(), json!("v99.0.0")), ("downloadUrl".into(), json!(DOWNLOAD))]);
        assert_eq!(install(&mut app, &fields).unwrap(), json!({"url": DOWNLOAD}));
        let mut stale = fields.clone();
        stale.insert("nativeUpdater".into(), json!(true));
        assert_eq!(install(&mut app, &stale).unwrap_err(), "The in-app updater is no longer available");
    }

    #[test]
    fn downloads_validate_destinations_and_report_browser_failures() {
        let mut app = PhotocraftApp::new(
            photocraft_engine::Session::new(),
            crate::Services {
                open_url: Some(Box::new(|url| {
                    assert_eq!(url, "https://github.com/storytold/photocraft/releases/download/v99.0.0/photocraft-99.0.0-windows-x64.msi");
                    Err("Browser could not open".into())
                })),
                ..Default::default()
            },
        );
        let mut fields = Map::from_iter([
            ("tagName".into(), json!("v99.0.0")),
            ("downloadUrl".into(), json!("https://github.com/storytold/photocraft/releases/download/v99.0.0/photocraft-99.0.0-windows-x64.msi")),
        ]);
        let id = app.ui.open_dialog(DialogKind::Update, fields.clone());
        assert_eq!(crate::dialogs::confirm(&mut app, id).unwrap_err(), "Browser could not open");
        fields.insert("downloadUrl".into(), json!("file:///tmp/installer"));
        assert!(open_download(&app, &fields).is_err());
        fields.remove("downloadUrl");
        fields.insert("htmlUrl".into(), json!("https://evil.example/"));
        assert!(open_download(&app, &fields).is_err());
    }
}
