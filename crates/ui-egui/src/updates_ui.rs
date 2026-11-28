//! Help ▸ Check for Updates and the check at startup, both driven by craft-update's `Updater`.

use serde_json::Value;

use crate::PhotocraftApp;

/// The update checker's state (desktop); the web build has none.
pub(crate) struct Runtime {
    /// The one checker: the startup prompt, the manual check, the installation and the last update's outcome.
    #[cfg(not(target_arch = "wasm32"))]
    updater: Option<craft::update::updater::Updater>,
    /// The check-result dialog on the dialog host, while one is open.
    #[cfg(not(target_arch = "wasm32"))]
    result_dialog: Option<craft::ui::dialog::DialogId>,
    /// The last update's outcome opened the result window; it opens once, so closing it with `ui.set` stays closed.
    #[cfg(not(target_arch = "wasm32"))]
    outcome_announced: bool,
    /// The signed installer's last handoff error, shown on the startup prompt until the next try.
    #[cfg(not(target_arch = "wasm32"))]
    signed_error: Option<String>,
}

#[cfg(not(target_arch = "wasm32"))]
fn stable_updater<T>(identity: crate::os_window::LaunchIdentity, make: impl FnOnce() -> T) -> Option<T> {
    crate::os_window::stable_services_for(identity).then(make)
}

impl Default for Runtime {
    fn default() -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            updater: stable_updater(crate::os_window::identity(), || {
                craft::update::updater::Updater::new(&crate::UPDATE_APP, photocraft_engine::build_info::VERSION)
            }),
            #[cfg(not(target_arch = "wasm32"))]
            result_dialog: None,
            #[cfg(not(target_arch = "wasm32"))]
            outcome_announced: false,
            #[cfg(not(target_arch = "wasm32"))]
            signed_error: None,
        }
    }
}

/// The startup prompt's id, the same key craft-update uses for [`craft::update::stack_dialogs::UpdatePromptDialog`].
#[cfg(not(target_arch = "wasm32"))]
fn prompt_id() -> craft::ui::dialog::DialogId {
    craft::ui::dialog::dialog_id("updateDialog.prompt")
}

/// Hands Update and Restart to the signed host installer when the bundle has one. Discovery and
/// Download stay with the updater, so Sparkle does not open a second prompt or check at launch.
/// A failed handoff leaves the prompt open and reports the error.
#[cfg(not(target_arch = "wasm32"))]
fn install_signed_request(app: &mut PhotocraftApp) {
    if app.services.install_update.is_none() {
        return;
    }
    let id = prompt_id();
    let requested = app
        .framework
        .dialogs
        .get_mut_by_id::<craft::update::stack_dialogs::UpdatePromptDialog>(id)
        .is_some_and(craft::update::stack_dialogs::UpdatePromptDialog::take_update_request);
    if !requested {
        return;
    }
    match hand_signed_install(app) {
        Ok(()) => {
            app.updates_rt.signed_error = None;
            if let Some(dialog) = app.framework.dialogs.get_mut_by_id::<craft::update::stack_dialogs::UpdatePromptDialog>(id) {
                dialog.withdraw();
            }
        }
        Err(error) => app.updates_rt.signed_error = Some(error),
    }
}

/// Runs the signed installer hook. Success closes the update window; failure leaves it open.
#[cfg(not(target_arch = "wasm32"))]
fn hand_signed_install(app: &mut PhotocraftApp) -> Result<(), String> {
    let Some(install) = app.services.install_update.as_mut() else { return Ok(()) };
    match install() {
        Ok(()) => {
            app.ui.updates_open = false;
            Ok(())
        }
        Err(error) => {
            app.ui.status = error.clone();
            app.ui.status_error = true;
            Err(error)
        }
    }
}

/// Help ▸ Check for Updates: starts a check whose outcome the result window shows.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn check(app: &mut PhotocraftApp, _ctx: &egui::Context, _params: Value) -> Result<Value, String> {
    let updater = app.updates_rt.updater.as_mut().ok_or("Nightly updates are managed by ArtCraft Launcher")?;
    updater.check_now();
    app.ui.updates_open = true;
    Ok(serde_json::json!({"status": "checking"}))
}

/// Update checks need a desktop install.
#[cfg(target_arch = "wasm32")]
pub(crate) fn check(_app: &mut PhotocraftApp, _ctx: &egui::Context, _params: Value) -> Result<Value, String> {
    Err("Update checks are not available here".into())
}

/// Once per pass: the check at start when the preference is on (the `Updater` runs it once per
/// launch), else only the read of the last update's outcome (also once, inside the `Updater`).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn tick(app: &mut PhotocraftApp) {
    use craft::update::updater::Status;
    let Some(updater) = app.updates_rt.updater.as_mut() else { return };
    if app.session.prefs().general.automatically_check_updates {
        updater.check_at_start();
    } else {
        updater.read_last_update();
    }
    if !app.updates_rt.outcome_announced && matches!(updater.status(), Status::Updated(_) | Status::InstallFailed(_)) {
        app.updates_rt.outcome_announced = true;
        app.ui.updates_open = true;
    }
}

/// The web build has no checker.
#[cfg(target_arch = "wasm32")]
pub(crate) fn tick(_app: &mut PhotocraftApp) {}

/// The web build has no checker.
#[cfg(target_arch = "wasm32")]
pub(crate) fn show(_app: &mut PhotocraftApp, _ctx: &egui::Context) {}

/// Drives the updater's own prompt and the check-result dialog on the framework dialog host. Runs
/// before `DialogHost::run`.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    use craft::update::stack_dialogs::{CheckChoice, UpdateCheckDialog};
    use craft::update::updater::{Action, Status};

    // An update starts, and a ready one asks the app to close, only while no work is unsaved.
    let clean = !app.has_unsaved_work();
    install_signed_request(app);
    let action = match app.updates_rt.updater.as_mut() {
        Some(updater) => {
            updater.tick(ctx);
            updater.run(&mut app.framework.dialogs, clean)
        }
        None => return,
    };
    match action {
        Action::None => {}
        Action::OpenUrl(url) => {
            let _ = crate::links::open(app, ctx, &url);
        }
        // The guarded close path asks about unsaved work and waits for saves; the restart helper
        // relaunches once the process exits.
        Action::Close => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        _ => {}
    }
    // The updater writes the prompt's status line itself; put the signed installer's error back.
    if let Some(error) = app.updates_rt.signed_error.clone()
        && let Some(dialog) = app.framework.dialogs.get_mut_by_id::<craft::update::stack_dialogs::UpdatePromptDialog>(prompt_id())
    {
        dialog.set_status(Some(error));
    } else if !app.updates_rt.updater.as_ref().is_some_and(|updater| updater.prompt_open()) {
        app.updates_rt.signed_error = None;
    }

    // A cancelled result (the dialog withdrawn below) answers nothing.
    if let Some(choice) = app.framework.dialogs.take::<UpdateCheckDialog>().and_then(|result| result.ok()) {
        app.updates_rt.result_dialog = None;
        let Some(updater) = app.updates_rt.updater.as_mut() else { return };
        match choice {
            CheckChoice::CheckNow | CheckChoice::CheckAgain => updater.check_now(),
            CheckChoice::RestartToUpdate => updater.restart_to_update(),
            CheckChoice::Close => {
                updater.dismiss_status();
                app.ui.updates_open = false;
            }
            _ => {}
        }
    }

    // An available release shows in the updater's prompt instead of the result window.
    let Some(updater) = app.updates_rt.updater.as_ref() else { return };
    let wanted = app.ui.updates_open && *updater.status() != Status::Idle && !updater.prompt_open();
    let status = updater.status().clone();
    let dialogs = &mut app.framework.dialogs;
    if wanted {
        if let Ok(id) = dialogs.open_unless_open(UpdateCheckDialog::new(photocraft_engine::build_info::VERSION)) {
            app.updates_rt.result_dialog = Some(id);
        }
        if let Some(dialog) = app.updates_rt.result_dialog.and_then(|id| dialogs.get_mut_by_id::<UpdateCheckDialog>(id)) {
            dialog.set_status(status);
        }
    } else if let Some(id) = app.updates_rt.result_dialog.take() {
        dialogs.cancel_by_id(ctx, id);
    }
}

/// The updater's installation progress or error line, for `ui.inspect`.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn status_message(app: &PhotocraftApp) -> Option<String> {
    app.updates_rt.updater.as_ref()?.status_message()
}

/// The web build has no updater, so no message.
#[cfg(target_arch = "wasm32")]
pub(crate) fn status_message(_app: &PhotocraftApp) -> Option<String> {
    None
}

/// Offers `version` as an available release, as a check at start would, for the snapshot example.
#[cfg(not(target_arch = "wasm32"))]
pub fn preview_release(app: &mut PhotocraftApp, version: &str, url: &str) {
    let Some(updater) = app.updates_rt.updater.as_mut() else { return };
    updater.offer(craft::update::Release {
        tag: format!("v{version}"),
        version: craft::update::parse_version(version),
        name: format!("PhotoCraft {version}"),
        html_url: url.into(),
        ..Default::default()
    });
    app.ui.updates_open = true;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Services, menus};
    use photocraft_engine::Session;
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::Duration;

    fn app() -> Box<PhotocraftApp> {
        PhotocraftApp::new(Session::new(), Services::default())
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn manual_check_works_when_automatic_checks_are_off_and_opens_status() {
        let (started, checks) = mpsc::channel();
        let started = Mutex::new(started);
        let source: craft::update::updater::CheckSource = Arc::new(move || {
            let _ = started.lock().unwrap().send(());
            Err("offline".into())
        });
        let mut app = app();
        app.updates_rt.updater = Some(craft::update::updater::Updater::new(&crate::UPDATE_APP, photocraft_engine::build_info::VERSION).with_source(source));
        app.session.edit_prefs(|p| p.general.automatically_check_updates = false);

        tick(&mut app);
        assert!(checks.recv_timeout(Duration::from_millis(100)).is_err(), "no automatic check while the preference is off");
        assert_eq!(*app.updates_rt.updater.as_ref().unwrap().status(), craft::update::updater::Status::Idle);

        let ctx = app.ctx().clone();
        menus::invoke(&mut app, &ctx, "help.checkForUpdates", serde_json::json!({})).unwrap();
        assert!(app.ui.updates_open);
        assert_ne!(*app.updates_rt.updater.as_ref().unwrap().status(), craft::update::updater::Status::Idle);
        assert!(checks.recv_timeout(Duration::from_secs(5)).is_ok(), "the manual check ran");
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn signed_install_uses_the_native_hook_and_does_not_open_a_browser() {
        let installs = Arc::new(Mutex::new(0u32));
        let counted = installs.clone();
        let opened = Arc::new(Mutex::new(Vec::new()));
        let urls = opened.clone();
        let mut app = app();
        app.services.install_update = Some(Box::new(move || {
            *counted.lock().unwrap() += 1;
            Ok(())
        }));
        app.services.open_url = Some(Box::new(move |url| {
            urls.lock().unwrap().push(url.to_string());
            Ok(())
        }));
        app.ui.updates_open = true;
        assert!(hand_signed_install(&mut app).is_ok());
        assert_eq!(*installs.lock().unwrap(), 1);
        assert!(opened.lock().unwrap().is_empty());
        assert!(!app.ui.updates_open);

        app.ui.updates_open = true;
        app.services.install_update = Some(Box::new(|| Err("Sparkle could not start".into())));
        let error = hand_signed_install(&mut app).unwrap_err();
        assert_eq!(error, "Sparkle could not start");
        assert!(app.ui.updates_open, "a failed handoff leaves the window open");
        assert!(app.ui.status_error);
        assert_eq!(app.ui.status, "Sparkle could not start");
        assert!(opened.lock().unwrap().is_empty());
    }

    #[test]
    fn update_window_is_drivable_and_bad_state_changes_are_atomic() {
        let mut app = app();
        let ctx = app.ctx().clone();
        let call = |app: &mut PhotocraftApp, method: &str, params: Value| {
            let (req, _) = crate::control::ControlRequest::new(method, params);
            match crate::control::handle(app, &ctx, &req) {
                crate::control::Reply::Done(value) => value,
                _ => panic!("expected an immediate reply"),
            }
        };
        assert_eq!(call(&mut app, "ui.set", serde_json::json!({"updatesOpen": true}))["ok"], true);
        let inspected = call(&mut app, "ui.inspect", serde_json::json!({}));
        assert_eq!(inspected["result"]["updatesOpen"], true);
        assert_eq!(inspected["result"]["updates"]["supported"], true);
        assert_eq!(call(&mut app, "ui.set", serde_json::json!({"updatesOpen": false, "maskTarget": "bad"}))["ok"], false);
        assert!(app.ui.updates_open);
        assert_eq!(call(&mut app, "ui.set", serde_json::json!({"updatesOpen": false}))["ok"], true);
        assert!(!app.ui.updates_open);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod profile_tests {
    use super::*;
    use crate::os_window::LaunchIdentity;
    #[test]
    fn a_nightly_never_even_constructs_the_stable_updater() {
        let calls = std::cell::Cell::new(0);
        let nightly = stable_updater(LaunchIdentity::NightlyV1, || {
            calls.set(calls.get() + 1);
            42
        });
        assert_eq!(nightly, None);
        assert_eq!(calls.get(), 0, "Updater::new reads stable installed-version state");
        assert_eq!(
            stable_updater(LaunchIdentity::Stable, || {
                calls.set(calls.get() + 1);
                42
            }),
            Some(42)
        );
        assert_eq!(calls.get(), 1);
    }
}
