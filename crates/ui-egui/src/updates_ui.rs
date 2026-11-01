//! One background check at startup and a non-modal window for release information.

use photocraft_engine::updates_cmds::Status;
use serde_json::Value;

use crate::PhotocraftApp;

/// A self-update that left this copy unchanged, told to the user instead of offering the install
/// again: the same install would fail the same way and the prompt would come back after every
/// restart (#10477). The text is an English template, translated and filled in when drawn.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) struct Problem {
    template: &'static str,
    args: Vec<(&'static str, String)>,
    /// Where to get the release by hand.
    pub(crate) url: String,
}

impl Problem {
    const HELPER_FAILED: &'static str = "The last update could not be installed: {reason}";
    const NOT_APPLIED: &'static str =
        "PhotoCraft {version} was installed a moment ago, but the copy that is running still reports {running}, so the installer did not replace it.";

    /// The update helper recorded `error` before it started the old copy again.
    pub(crate) fn helper_failed(error: &str) -> Self {
        let error = error.trim();
        let reason = if error.is_empty() { "the update helper stopped without saying why" } else { error };
        Self { template: Self::HELPER_FAILED, args: vec![("reason", reason.to_owned())], url: photocraft_engine::updates_cmds::RELEASES.to_owned() }
    }

    /// `version` was installed a moment ago, yet this copy (`running`) is still older. The page is
    /// built from our own releases address, never taken from what the network answered.
    pub(crate) fn not_applied(version: &str, running: &str) -> Self {
        let url = format!("{}/tag/v{version}", photocraft_engine::updates_cmds::RELEASES);
        Self { template: Self::NOT_APPLIED, args: vec![("version", version.to_owned()), ("running", running.to_owned())], url }
    }

    fn fill(&self, template: &str) -> String {
        let args: Vec<(&str, &str)> = self.args.iter().map(|(k, v)| (*k, v.as_str())).collect();
        crate::i18n::fmt(template, &args)
    }

    /// The message in English, as `ui.inspect` reports it.
    pub(crate) fn message(&self) -> String {
        self.fill(self.template)
    }

    /// The message in the current language.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    fn text(&self) -> String {
        self.fill(crate::i18n::t(self.template))
    }
}

pub(crate) struct Runtime {
    startup_checked: bool,
    notified_version: Option<String>,
    /// Why the update did not happen, until the user closes the window that says so.
    pub(crate) problem: Option<Problem>,
    /// The startup prompt and the self-update behind it (desktop).
    #[cfg(not(target_arch = "wasm32"))]
    updater: craft_update::updater::Updater,
}

impl Default for Runtime {
    fn default() -> Self {
        Self {
            startup_checked: false,
            notified_version: None,
            problem: None,
            #[cfg(not(target_arch = "wasm32"))]
            updater: craft_update::updater::Updater::new(&crate::UPDATE_APP, photocraft_engine::build_info::VERSION),
        }
    }
}

/// The release the engine found, as `craft-update` knows it.
#[cfg(not(target_arch = "wasm32"))]
fn release_of(found: &photocraft_engine::updates_cmds::Release) -> craft_update::Release {
    craft_update::Release {
        tag: format!("v{}", found.version),
        version: craft_update::parse_version(&found.version),
        name: format!("PhotoCraft {}", found.version),
        html_url: found.url.clone(),
        ..Default::default()
    }
}

pub(crate) fn check(app: &mut PhotocraftApp, _ctx: &egui::Context, params: Value) -> Result<Value, String> {
    let before = app.session.updates.status.clone();
    let result = app.run("help.checkForUpdates", params);
    // A check that fails to start still shows its failure and Check Again; the command error
    // stands. Only this call's failure: a Failed left from an earlier check doesn't open the
    // window for a call refused for another reason (bad params).
    let failed_to_start = match (&result, &app.session.updates.status) {
        (Err(error), status @ Status::Failed { message }) => *status != before || (!message.is_empty() && error.contains(message.as_str())),
        _ => false,
    };
    if result.is_ok() || failed_to_start {
        app.ui.updates_open = true;
        app.updates_rt.startup_checked = true;
        // A release the user dismissed is offered again by a manual check.
        app.updates_rt.notified_version = None;
    }
    result
}

pub(crate) fn tick(app: &mut PhotocraftApp, ctx: &egui::Context) {
    // `updates.status` also polls (automation may consume the result before this frame).
    app.session.updates.poll();
    if let Status::Available { release } = &app.session.updates.status
        && app.updates_rt.notified_version.as_ref() != Some(&release.version)
    {
        app.updates_rt.notified_version = Some(release.version.clone());
        // This copy installed that very release a moment ago and still is not it: the install
        // does not take effect here, so offering it again would only loop (update, restart, the
        // same prompt). Say so and point at the release page.
        let installed_already = app.services.update_installed.is_some_and(|installed| installed(&format!("v{}", release.version)));
        if installed_already {
            app.updates_rt.problem = Some(Problem::not_applied(&release.version, photocraft_engine::build_info::VERSION));
        } else {
            app.ui.updates_open = true;
            #[cfg(not(target_arch = "wasm32"))]
            app.updates_rt.updater.offer(release_of(release));
        }
    }
    if app.session.updates.check.is_none() {
        return;
    }
    if !app.updates_rt.startup_checked {
        app.updates_rt.startup_checked = true;
        if app.session.prefs().general.automatically_check_updates && matches!(app.session.updates.status, Status::Idle) {
            let _ = app.run("help.checkForUpdates", serde_json::json!({}));
        }
    }
    if matches!(app.session.updates.status, Status::Checking) {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn show(_app: &mut PhotocraftApp, _ctx: &egui::Context) {}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let mut download: Option<String> = None;
    {
        let was_open = app.updates_rt.updater.prompt_open();
        app.updates_rt.updater.tick(ctx);
        // The restart helper is scheduled before the app closes, so no update starts over unsaved work.
        let clean = !app.has_unsaved_work();
        match app.updates_rt.updater.show_if(ctx, clean) {
            craft_update::updater::Action::None => {}
            craft_update::updater::Action::OpenUrl(url) => download = Some(url),
            // The guarded close path asks about unsaved work and waits for saves; the restart helper
            // relaunches once the process exits.
            craft_update::updater::Action::Close => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        }
        if was_open && !app.updates_rt.updater.prompt_open() {
            app.ui.updates_open = false;
        }
    }
    if let Some(url) = download {
        crate::links::open(app, ctx, &url);
    }
    show_problem(app, ctx);
    if app.updates_rt.updater.prompt_open() {
        return;
    }
    if app.ui.updates_open {
        let dialog = craft_update::CheckResultDialog::new(&crate::UPDATE_APP, photocraft_engine::build_info::VERSION);
        match dialog.show(ctx, &check_status(&app.session.updates.status)) {
            craft_update::CheckResultResponse::None => {}
            craft_update::CheckResultResponse::Close => app.ui.updates_open = false,
            craft_update::CheckResultResponse::CheckAgain => {
                let _ = check(app, ctx, serde_json::json!({}));
            }
        }
    }
}

/// The window that says why PhotoCraft was not updated, with the release page to update by hand.
/// It stays until closed, above the startup prompt when that is open too.
#[cfg(not(target_arch = "wasm32"))]
fn show_problem(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(problem) = app.updates_rt.problem.clone() else { return };
    let (mut open_page, mut close) = (false, false);
    egui::Window::new(tl!("Update Problem"))
        .id(egui::Id::unique("photocraft_update_problem"))
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .collapsible(false)
        .resizable(false)
        .default_width(420.0)
        .show(ctx, |ui| {
            let t = crate::theme::Tokens::get(ctx);
            ui.colored_label(t.warning, tl!("PhotoCraft was not updated."));
            ui.add_space(4.0);
            ui.label(problem.text());
            ui.add_space(4.0);
            ui.label(tl!("Download PhotoCraft from the release page and run its installer. Your documents and settings are not affected."));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                open_page = crate::widgets::primary_button(ui, tl!("Open Release Page"), 160.0).clicked();
                close = ui.button(tl!("Close")).clicked();
            });
        });
    if open_page {
        crate::links::open(app, ctx, &problem.url);
    }
    if open_page || close {
        app.updates_rt.problem = None;
    }
}

/// The engine's check state as the result dialog shows it. An available release goes to the
/// startup prompt, so the dialog draws nothing for it.
#[cfg(not(target_arch = "wasm32"))]
fn check_status(status: &Status) -> craft_update::updater::Status {
    use craft_update::updater::Status as Shown;
    match status {
        Status::Checking => Shown::Checking,
        Status::UpToDate => Shown::UpToDate,
        Status::Failed { message } => Shown::Failed(message.clone()),
        Status::Idle | Status::Available { .. } => Shown::Idle,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Services, menus};
    use photocraft_engine::{Session, updates_cmds::Release};
    use std::sync::{Arc, Mutex, mpsc};

    type Sender = Arc<Mutex<Option<mpsc::Sender<photocraft_engine::updates_cmds::CheckResult>>>>;

    fn app() -> (PhotocraftApp, Arc<Mutex<u32>>, Sender) {
        let calls = Arc::new(Mutex::new(0));
        let count = calls.clone();
        let sender = Arc::new(Mutex::new(None));
        let tx = sender.clone();
        let app = PhotocraftApp::new(
            Session::new(),
            Services {
                check_updates: Some(Box::new(move || {
                    *count.lock().unwrap() += 1;
                    let (sender, receiver) = mpsc::channel();
                    *tx.lock().unwrap() = Some(sender);
                    Ok(receiver)
                })),
                ..Default::default()
            },
        );
        (app, calls, sender)
    }

    fn at(app: &mut PhotocraftApp, ctx: &egui::Context, time: f64) {
        let mut output = ctx.run_ui(egui::RawInput { time: Some(time), ..Default::default() }, |ui| tick(app, ui.ctx()));
        output.textures_delta.clear();
    }

    #[test]
    fn automatic_check_runs_once_per_launch_even_after_days_or_preference_changes() {
        let (mut app, calls, sender) = app();
        let ctx = egui::Context::default();
        at(&mut app, &ctx, 0.0);
        at(&mut app, &ctx, 1.0);
        at(&mut app, &ctx, 2.0);
        assert_eq!(*calls.lock().unwrap(), 1);
        sender.lock().unwrap().as_ref().unwrap().send(Ok(None)).unwrap();
        at(&mut app, &ctx, 3.0);
        assert!(!app.ui.updates_open, "automatic up-to-date checks stay quiet");
        at(&mut app, &ctx, 86400.0);
        app.session.edit_prefs(|p| p.general.automatically_check_updates = false);
        at(&mut app, &ctx, 86401.0);
        app.session.edit_prefs(|p| p.general.automatically_check_updates = true);
        at(&mut app, &ctx, 172800.0);
        assert_eq!(*calls.lock().unwrap(), 1, "no periodic checks or preference-triggered rechecks");
        let (mut restarted, restarted_calls, _) = self::app();
        at(&mut restarted, &ctx, 0.0);
        assert_eq!(*restarted_calls.lock().unwrap(), 1, "the next launch checks again");
    }

    #[test]
    fn startup_opt_out_and_failed_checks_do_not_retry_automatically() {
        let (mut disabled, calls, _) = app();
        let ctx = egui::Context::default();
        disabled.session.edit_prefs(|p| p.general.automatically_check_updates = false);
        at(&mut disabled, &ctx, 0.0);
        disabled.session.edit_prefs(|p| p.general.automatically_check_updates = true);
        at(&mut disabled, &ctx, 86400.0);
        assert_eq!(*calls.lock().unwrap(), 0, "opt-out is evaluated at startup");

        let (mut failed, calls, sender) = app();
        at(&mut failed, &ctx, 0.0);
        sender.lock().unwrap().as_ref().unwrap().send(Err("offline".into())).unwrap();
        at(&mut failed, &ctx, 1.0);
        at(&mut failed, &ctx, 172800.0);
        assert_eq!(*calls.lock().unwrap(), 1);
        assert!(!failed.ui.updates_open, "an automatic failure stays quiet");
    }

    #[test]
    fn manual_check_works_when_automatic_checks_are_off_and_opens_status() {
        let (mut app, calls, sender) = app();
        let ctx = egui::Context::default();
        app.session.edit_prefs(|p| p.general.automatically_check_updates = false);
        assert!(menus::is_enabled(&app, "help.checkForUpdates"));
        menus::invoke(&mut app, &ctx, "help.checkForUpdates", serde_json::json!({})).unwrap();
        assert!(app.ui.updates_open);
        assert_eq!(*calls.lock().unwrap(), 1);
        sender.lock().unwrap().as_ref().unwrap().send(Err("offline".into())).unwrap();
        at(&mut app, &ctx, 1.0);
        assert!(matches!(app.session.updates.status, Status::Failed { .. }));
        let mut web = PhotocraftApp::new(Session::new(), Services::default());
        assert!(!menus::is_enabled(&web, "help.checkForUpdates"));
        at(&mut web, &ctx, 2.0);
        assert_eq!(web.session.updates.status, Status::Idle);
    }

    #[test]
    fn available_release_notifies_once_even_when_automation_polls_first() {
        let (mut app, _, sender) = app();
        let ctx = egui::Context::default();
        at(&mut app, &ctx, 0.0);
        sender
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .send(Ok(Some(Release { version: "99.0.0".into(), url: "https://github.com/storytold/photocraft/releases/tag/v99.0.0".into() })))
            .unwrap();
        app.run("updates.status", serde_json::json!({})).unwrap();
        at(&mut app, &ctx, 1.0);
        assert!(app.ui.updates_open);
        app.ui.updates_open = false;
        at(&mut app, &ctx, 2.0);
        assert!(!app.ui.updates_open);
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn startup_update_prompt_appears_and_can_be_dismissed() {
        let (mut app, _, sender) = app();
        let ctx = egui::Context::default();
        at(&mut app, &ctx, 0.0);
        sender
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .send(Ok(Some(Release { version: "99.0.0".into(), url: "https://github.com/storytold/photocraft/releases/tag/v99.0.0".into() })))
            .unwrap();
        at(&mut app, &ctx, 1.0);
        assert!(app.updates_rt.updater.prompt_open(), "startup prompt initialized on available release");

        // Rendering show() renders the prompt modal
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            show(&mut app, ui.ctx());
        });
        output.textures_delta.clear();
        assert!(app.updates_rt.updater.prompt_open());
    }

    #[test]
    fn a_manual_check_that_fails_to_start_opens_the_status_window() {
        let mut app =
            PhotocraftApp::new(Session::new(), Services { check_updates: Some(Box::new(|| Err("Could not start worker".into()))), ..Default::default() });
        let ctx = egui::Context::default();
        let error = menus::invoke(&mut app, &ctx, "help.checkForUpdates", serde_json::json!({})).unwrap_err();
        assert!(error.contains("Could not start worker"), "{error}");
        assert!(matches!(app.session.updates.status, Status::Failed { .. }));
        assert!(app.ui.updates_open, "the failure and Check Again are shown");
        // Bad params are a caller error, not a failed check: nothing opens, also while the earlier
        // failure is still the status.
        app.ui.updates_open = false;
        assert!(menus::invoke(&mut app, &ctx, "help.checkForUpdates", serde_json::json!({"x": 1})).is_err());
        assert!(matches!(app.session.updates.status, Status::Failed { .. }), "the earlier failure stays");
        assert!(!app.ui.updates_open, "bad params after a failed check");
        app.session.updates.status = Status::Idle;
        assert!(menus::invoke(&mut app, &ctx, "help.checkForUpdates", serde_json::json!({"x": 1})).is_err());
        assert!(!app.ui.updates_open);
        // The same start failure again opens it again.
        app.session.updates.status = Status::Failed { message: "Could not start worker".into() };
        assert!(menus::invoke(&mut app, &ctx, "help.checkForUpdates", serde_json::json!({})).is_err());
        assert!(app.ui.updates_open, "a repeated start failure");
    }

    #[test]
    fn update_window_is_drivable_and_bad_state_changes_are_atomic() {
        let (mut app, _, _) = app();
        let ctx = egui::Context::default();
        let call = |app: &mut PhotocraftApp, method: &str, params: Value| {
            let (req, _) = crate::control::ControlRequest::new(method, params);
            match crate::control::handle(app, &ctx, &req) {
                crate::control::Outcome::Done(value) => value,
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

    fn release(version: &str) -> Release {
        Release { version: version.into(), url: format!("https://github.com/storytold/photocraft/releases/tag/v{version}") }
    }

    /// An app whose automatic check has found `version`, with `installed` saying which tag this
    /// copy installed itself a moment ago.
    fn app_offered(version: &str, installed: Option<fn(&str) -> bool>) -> PhotocraftApp {
        let found = release(version);
        let mut app = PhotocraftApp::new(
            Session::new(),
            Services {
                check_updates: Some(Box::new(move || {
                    let (sender, receiver) = mpsc::channel();
                    sender.send(Ok(Some(found.clone()))).unwrap();
                    Ok(receiver)
                })),
                update_installed: installed,
                ..Default::default()
            },
        );
        let ctx = egui::Context::default();
        at(&mut app, &ctx, 0.0);
        at(&mut app, &ctx, 1.0);
        app
    }

    #[test]
    fn a_failure_the_update_helper_recorded_is_told_at_startup() {
        let app = PhotocraftApp::new(
            Session::new(),
            Services { update_failure: Some("the Windows Installer stopped with error 1603\n".into()), ..Default::default() },
        );
        let problem = app.updates_rt.problem.as_ref().expect("the failure is shown");
        assert_eq!(problem.message(), "The last update could not be installed: the Windows Installer stopped with error 1603");
        assert_eq!(problem.url, photocraft_engine::updates_cmds::RELEASES, "the page to update by hand");
        assert_eq!(app.services.update_failure, None, "told once");
        let none = PhotocraftApp::new(Session::new(), Services::default());
        assert!(none.updates_rt.problem.is_none(), "an ordinary start says nothing");
        let blank = Problem::helper_failed("  ");
        assert!(blank.message().contains("stopped without saying why"), "{}", blank.message());
    }

    #[test]
    fn a_release_that_was_just_installed_is_explained_instead_of_offered_again() {
        let mut app = app_offered("99.0.0", Some(|tag| tag == "v99.0.0"));
        let problem = app.updates_rt.problem.as_ref().expect("the loop is named");
        let message = problem.message();
        assert!(message.contains("99.0.0") && message.contains(photocraft_engine::build_info::VERSION), "{message}");
        assert_eq!(problem.url, "https://github.com/storytold/photocraft/releases/tag/v99.0.0");
        assert!(!app.ui.updates_open);
        #[cfg(not(target_arch = "wasm32"))]
        assert!(!app.updates_rt.updater.prompt_open(), "no install that would fail the same way");
        let (req, _) = crate::control::ControlRequest::new("ui.inspect", serde_json::json!({}));
        let crate::control::Outcome::Done(value) = crate::control::handle(&mut app, &egui::Context::default(), &req) else {
            panic!("expected an immediate reply")
        };
        assert_eq!(value["result"]["updates"]["problem"], message);
    }

    #[test]
    fn a_release_that_was_not_installed_here_is_offered_as_before() {
        for installed in [None, Some((|_: &str| false) as fn(&str) -> bool), Some(|tag: &str| tag == "v98.0.0")] {
            let app = app_offered("99.0.0", installed);
            assert!(app.updates_rt.problem.is_none());
            assert!(app.ui.updates_open);
            #[cfg(not(target_arch = "wasm32"))]
            assert!(app.updates_rt.updater.prompt_open());
        }
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn the_problem_window_draws_above_the_prompt_and_stays_until_closed() {
        let mut app = app_offered("99.0.0", Some(|_| false));
        app.updates_rt.problem = Some(Problem::helper_failed("disk full"));
        let ctx = egui::Context::default();
        for time in [2.0, 3.0] {
            let mut output = ctx.run_ui(egui::RawInput { time: Some(time), ..Default::default() }, |ui| show(&mut app, ui.ctx()));
            output.textures_delta.clear();
        }
        assert!(app.updates_rt.problem.is_some(), "nothing closes it but the user");
        assert!(app.updates_rt.updater.prompt_open());
    }
}
