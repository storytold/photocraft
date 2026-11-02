//! The menu opens a real command form without starting a provider request.
use photocraft_ui_egui::{PhotocraftApp, Services};
use serde_json::json;

#[test]
fn ai_edit_opens_prompt_form() {
    let mut session = photocraft_engine::Session::new();
    session.execute("file.new", json!({"width":16,"height":16})).unwrap();
    let mut app = PhotocraftApp::new(session, Services::default());
    let items = photocraft_ui_egui::menus::menu_items(&app);
    let fill = items.iter().position(|item| item.id == "edit.contentAwareFill").unwrap();
    let ai = &items[fill + 1];
    assert_eq!(ai.id, "edit.ai.codex");
    assert_eq!(ai.path, ["Edit"]);
    assert!(ai.enabled);
    let ctx = egui::Context::default();
    let result = photocraft_ui_egui::menus::invoke(&mut app, &ctx, "edit.ai.codex", json!({})).unwrap();
    assert!(result["dialog"].is_u64());
    assert_eq!(app.session.active().unwrap().doc.layers.len(), 1);
}

#[test]
fn ai_edit_requires_an_open_document() {
    let app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    assert!(!photocraft_ui_egui::menus::is_enabled(&app, "edit.ai.codex"));
}

#[cfg(not(target_arch = "wasm32"))]
mod account_flow {
    use super::*;
    use egui_kittest::{
        Harness,
        kittest::{NodeT, Queryable},
    };
    use photocraft_engine::cmd::ai_account_cmds::{AccountError, AccountInfo, AccountService, State};
    use std::sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    use std::time::{Duration, Instant};

    struct Fake {
        info: Mutex<Option<AccountInfo>>,
        finish: AtomicBool,
        opened: Mutex<Vec<String>>,
        fail_browser: AtomicBool,
        runtime_missing: AtomicBool,
        hold_cancel: AtomicBool,
        checks: AtomicUsize,
    }
    impl AccountService for Fake {
        fn check(&self, _: &dyn Fn() -> bool) -> Result<AccountInfo, AccountError> {
            self.checks.fetch_add(1, Ordering::AcqRel);
            if self.runtime_missing.load(Ordering::Acquire) {
                return Err(AccountError::RuntimeUnavailable("Synthetic missing runtime".into()));
            }
            self.info.lock().unwrap_or_else(PoisonError::into_inner).clone().ok_or_else(|| AccountError::Provider("Expired session".into()))
        }
        fn login(&self, url: &mut dyn FnMut(String) -> Result<(), AccountError>, cancel: &dyn Fn() -> bool) -> Result<AccountInfo, AccountError> {
            url("https://auth.openai.com/authorize?state=test".into())?;
            while !cancel() && !self.finish.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(1));
            }
            // Model a bounded runtime that needs time to stop after cancellation.
            let deadline = Instant::now() + Duration::from_secs(3);
            while cancel() && self.hold_cancel.load(Ordering::Acquire) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(1));
            }
            if cancel() { Err(AccountError::Cancelled) } else { Ok(AccountInfo::ChatGpt { plan: Some("plus".into()) }) }
        }
    }

    fn harness(info: AccountInfo) -> (Harness<'static, PhotocraftApp>, Arc<Fake>) {
        let fake = Arc::new(Fake {
            info: Mutex::new(Some(info)),
            finish: AtomicBool::new(false),
            opened: Mutex::new(Vec::new()),
            fail_browser: AtomicBool::new(false),
            runtime_missing: AtomicBool::new(false),
            hold_cancel: AtomicBool::new(false),
            checks: AtomicUsize::new(0),
        });
        let service = fake.clone();
        let mut h = Harness::builder().with_size(egui::vec2(1280.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let mut session = photocraft_engine::Session::new();
            session.edit_prefs(|p| p.interface.language = "en".into());
            assert!(session.execute("file.new", json!({"width":16,"height":16})).is_ok());
            // Even a broken confirmation path must never reach a real provider in this harness.
            session.authorize = Some(|command, _| {
                if command == "edit.ai.codex" {
                    Err(photocraft_engine::EngineError::Other("Offline UI test: image requests are forbidden".into()))
                } else {
                    Ok(())
                }
            });
            let browser = service.clone();
            assert!(session.ai_account.set_service(service).is_ok());
            let ctx = cc.egui_ctx.clone();
            let services = Services {
                open_url: Some(Box::new(move |url| {
                    browser.opened.lock().unwrap_or_else(PoisonError::into_inner).push(url.to_owned());
                    ctx.data_mut(|data| data.insert_temp(egui::Id::unique("native-auth-browser-called"), true));
                    if browser.fail_browser.load(Ordering::Acquire) { Err("Synthetic browser failure".into()) } else { Ok(()) }
                })),
                ..Default::default()
            };
            PhotocraftApp::new(session, services)
        });
        h.run_steps(3);
        let ctx = h.ctx.clone();
        assert!(photocraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "edit.ai.codex", json!({})).is_ok());
        (h, fake)
    }

    fn wait(h: &mut Harness<'_, PhotocraftApp>, expected: State) -> bool {
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut browser = false;
        while Instant::now() < deadline {
            h.run_steps(1);
            browser |= h.ctx.data(|data| data.get_temp::<bool>(egui::Id::unique("native-auth-browser-called"))).unwrap_or(false);
            if h.state().ui.ai_account.status.state == expected {
                // Egui measures a newly opened modal before exposing its widgets.
                h.run_steps(2);
                return browser;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(h.state().ui.ai_account.status.state, expected);
        browser
    }

    #[test]
    fn signed_out_login_opens_browser_only_after_click_and_cancel_preserves_canvas() {
        let (mut h, fake) = harness(AccountInfo::SignedOut);
        assert!(!wait(&mut h, State::SignedOut), "checking must not open a browser");
        assert!(h.get_by_label("OK").accesskit_node().is_disabled());
        h.get_by_label("Sign in with ChatGPT").click();
        assert!(wait(&mut h, State::WaitingForBrowser));
        assert_eq!(fake.opened.lock().unwrap_or_else(PoisonError::into_inner).len(), 1, "use the native browser service exactly once");
        assert!(!h.state().ui.status.contains("authorize"), "auth URL must not enter status history");
        let serialized = serde_json::to_string(&h.state().ui.ai_account).unwrap();
        assert!(!serialized.contains("authorize") && !serialized.contains("state=test"));
        h.get_by_label("Cancel sign-in").click();
        wait(&mut h, State::Cancelled);
        assert_eq!(h.state().session.active().unwrap().doc.layers.len(), 1);
        assert!(h.get_by_label("OK").accesskit_node().is_disabled());
    }

    #[test]
    fn completion_enables_edit_only_with_prompt_and_expired_refresh_disables_it() {
        let (mut h, fake) = harness(AccountInfo::SignedOut);
        wait(&mut h, State::SignedOut);
        h.get_by_label("Sign in with ChatGPT").click();
        assert!(wait(&mut h, State::WaitingForBrowser));
        fake.finish.store(true, Ordering::Release);
        wait(&mut h, State::Connected);
        assert!(h.get_by_label("OK").accesskit_node().is_disabled(), "empty prompt");
        h.state_mut().ui.dialogs[0].fields.insert("prompt".into(), json!("Make the circle yellow"));
        h.run_steps(2);
        assert!(!h.get_by_label("OK").accesskit_node().is_disabled());
        *fake.info.lock().unwrap_or_else(PoisonError::into_inner) = None;
        h.get_by_label("Check connection").click();
        wait(&mut h, State::Failed);
        assert!(h.get_by_label("OK").accesskit_node().is_disabled());
        assert!(h.query_by_label("Sign in with ChatGPT").is_some());
        assert_eq!(h.state().session.active().unwrap().doc.layers.len(), 1);
    }

    #[test]
    fn closing_the_form_cancels_the_pending_browser_login() {
        let (mut h, _) = harness(AccountInfo::SignedOut);
        wait(&mut h, State::SignedOut);
        h.get_by_label("Sign in with ChatGPT").click();
        wait(&mut h, State::WaitingForBrowser);
        h.get_by_label("Cancel").click();
        wait(&mut h, State::Cancelled);
        assert!(h.state().ui.dialogs.is_empty());
        assert!(!h.state().session.ai_account.pending());
    }

    #[test]
    fn existing_subscription_needs_no_new_authorization_and_api_login_is_refused() {
        for info in [AccountInfo::ChatGpt { plan: Some("pro".into()) }, AccountInfo::Unsupported] {
            let expected = if matches!(info, AccountInfo::Unsupported) { State::Unsupported } else { State::Connected };
            let (mut h, _) = harness(info);
            assert!(!wait(&mut h, expected.clone()));
            assert_eq!(h.query_by_label("Sign in with ChatGPT").is_some(), expected == State::Unsupported);
        }
    }

    #[test]
    fn missing_runtime_guides_install_and_recheck_without_requesting_login() {
        let (mut h, fake) = harness(AccountInfo::ChatGpt { plan: None });
        wait(&mut h, State::Connected);
        fake.runtime_missing.store(true, Ordering::Release);
        h.get_by_label("Check connection").click();
        assert!(!wait(&mut h, State::RuntimeUnavailable));
        assert!(h.query_by_label("Codex was not found. Install and open the Codex desktop app, then check the connection again.").is_some());
        assert!(h.query_by_label("Sign in with ChatGPT").is_none());
        assert!(h.get_by_label("OK").accesskit_node().is_disabled());
        fake.runtime_missing.store(false, Ordering::Release);
        h.get_by_label("Check connection").click();
        assert!(!wait(&mut h, State::Connected));
        assert!(fake.opened.lock().unwrap_or_else(PoisonError::into_inner).is_empty());
        assert_eq!(h.state().session.active().unwrap().doc.layers.len(), 1);
    }

    #[test]
    fn native_browser_failure_is_visible_and_user_can_copy_link_or_retry() {
        let (mut h, fake) = harness(AccountInfo::SignedOut);
        wait(&mut h, State::SignedOut);
        fake.fail_browser.store(true, Ordering::Release);
        h.get_by_label("Sign in with ChatGPT").click();
        assert!(wait(&mut h, State::WaitingForBrowser));
        assert!(h.query_by_label("Could not open your browser. Try again or copy the sign-in link into your browser.").is_some());
        h.get_by_label("Copy sign-in link").click();
        h.run_steps(1);
        assert!(
            h.output()
                .platform_output
                .commands
                .iter()
                .any(|command| matches!(command, egui::OutputCommand::CopyText(text) if text.starts_with("https://auth.openai.com/")))
        );
        fake.fail_browser.store(false, Ordering::Release);
        h.get_by_label("Open sign-in page").click();
        h.run_steps(2);
        assert!(h.query_by_label("Could not open your browser. Try again or copy the sign-in link into your browser.").is_none());
        assert_eq!(fake.opened.lock().unwrap_or_else(PoisonError::into_inner).len(), 2);
    }

    #[test]
    fn missing_native_browser_service_reports_failure_without_emitting_an_open_url() {
        let (mut h, _) = harness(AccountInfo::SignedOut);
        wait(&mut h, State::SignedOut);
        h.state_mut().services.open_url = None;
        h.get_by_label("Sign in with ChatGPT").click();
        wait(&mut h, State::WaitingForBrowser);
        assert!(h.query_by_label("Could not open your browser. Try again or copy the sign-in link into your browser.").is_some());
        assert!(!h.output().platform_output.commands.iter().any(|cmd| matches!(cmd, egui::OutputCommand::OpenUrl(_))));
        h.get_by_label("Copy sign-in link").click();
        h.run_steps(1);
        assert!(
            h.output()
                .platform_output
                .commands
                .iter()
                .any(|cmd| matches!(cmd, egui::OutputCommand::CopyText(text) if text.starts_with("https://auth.openai.com/")))
        );
    }

    #[test]
    fn reopening_during_cancel_rechecks_once_when_worker_finishes() {
        let (mut h, fake) = harness(AccountInfo::SignedOut);
        wait(&mut h, State::SignedOut);
        fake.hold_cancel.store(true, Ordering::Release);
        h.get_by_label("Sign in with ChatGPT").click();
        wait(&mut h, State::WaitingForBrowser);
        h.get_by_label("Cancel").click();
        h.run_steps(1);
        assert!(h.state().session.ai_account.pending());
        *fake.info.lock().unwrap_or_else(PoisonError::into_inner) = Some(AccountInfo::ChatGpt { plan: None });
        let ctx = h.ctx.clone();
        photocraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "edit.ai.codex", json!({})).unwrap();
        h.run_steps(2);
        assert_eq!(fake.checks.load(Ordering::Acquire), 1);
        fake.hold_cancel.store(false, Ordering::Release);
        wait(&mut h, State::Connected);
        h.run_steps(4);
        assert_eq!(fake.checks.load(Ordering::Acquire), 2);
        assert_eq!(fake.opened.lock().unwrap_or_else(PoisonError::into_inner).len(), 1);
    }

    #[test]
    fn closing_reopened_dialog_clears_deferred_refresh() {
        let (mut h, fake) = harness(AccountInfo::SignedOut);
        wait(&mut h, State::SignedOut);
        fake.hold_cancel.store(true, Ordering::Release);
        h.get_by_label("Sign in with ChatGPT").click();
        wait(&mut h, State::WaitingForBrowser);
        let id = h.state().ui.dialogs[0].id;
        photocraft_ui_egui::dialogs::cancel(h.state_mut(), id).unwrap();
        let ctx = h.ctx.clone();
        photocraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "edit.ai.codex", json!({})).unwrap();
        let id = h.state().ui.dialogs[0].id;
        photocraft_ui_egui::dialogs::cancel(h.state_mut(), id).unwrap();
        fake.hold_cancel.store(false, Ordering::Release);
        wait(&mut h, State::Cancelled);
        assert_eq!(fake.checks.load(Ordering::Acquire), 1);
        assert!(h.state().ui.dialogs.is_empty());
    }

    #[test]
    fn control_confirm_during_login_keeps_dialog_and_pending_worker() {
        let (mut h, _) = harness(AccountInfo::SignedOut);
        wait(&mut h, State::SignedOut);
        h.get_by_label("Sign in with ChatGPT").click();
        wait(&mut h, State::WaitingForBrowser);
        let id = h.state().ui.dialogs[0].id;
        h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("prompt".into(), json!("Make the circle yellow"));
        assert!(photocraft_ui_egui::dialogs::confirm(h.state_mut(), id).is_err());
        assert!(h.state().ui.dialogs.iter().any(|dialog| dialog.id == id));
        assert!(h.state().session.ai_account.pending());
        assert_eq!(h.state().session.active().unwrap().doc.layers.len(), 1);
    }

    #[test]
    fn enter_in_prompt_inserts_newline_without_confirming() {
        let (mut h, _) = harness(AccountInfo::ChatGpt { plan: None });
        wait(&mut h, State::Connected);
        h.get_by_role(egui::accesskit::Role::MultilineTextInput).click();
        h.event(egui::Event::Text("Make the circle yellow".into()));
        h.run_steps(2);
        assert!(!h.get_by_label("OK").accesskit_node().is_disabled());
        h.key_press(egui::Key::Enter);
        h.run_steps(2);
        assert_eq!(h.state().ui.dialogs.len(), 1, "Enter must not submit the canvas");
        assert!(h.state().ui.dialogs[0].fields["prompt"].as_str().unwrap().contains('\n'));
        assert_eq!(h.state().session.active().unwrap().doc.layers.len(), 1);
    }

    #[test]
    fn control_confirm_rejects_stale_connected_ui_while_refresh_is_pending() {
        let (mut h, _) = harness(AccountInfo::ChatGpt { plan: None });
        wait(&mut h, State::Connected);
        let id = h.state().ui.dialogs[0].id;
        h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("prompt".into(), json!("Make the circle yellow"));
        h.state_mut().run("ai.codex.account", json!({"action":"refresh"})).unwrap();
        assert_eq!(h.state().ui.ai_account.status.state, State::Connected);
        assert!(photocraft_ui_egui::dialogs::confirm(h.state_mut(), id).is_err());
        assert!(h.state().ui.dialogs.iter().any(|dialog| dialog.id == id));
        assert_eq!(h.state().session.active().unwrap().doc.layers.len(), 1);
    }

    #[test]
    fn ready_confirmation_still_closes_dialog_and_reaches_the_command_gate() {
        let (mut h, _) = harness(AccountInfo::ChatGpt { plan: None });
        wait(&mut h, State::Connected);
        let id = h.state().ui.dialogs[0].id;
        h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("prompt".into(), json!("Make the circle yellow"));
        let error = photocraft_ui_egui::dialogs::confirm(h.state_mut(), id).unwrap_err();
        assert!(error.contains("Offline UI test: image requests are forbidden"));
        assert!(!h.state().ui.dialogs.iter().any(|dialog| dialog.id == id));
        assert_eq!(h.state().session.active().unwrap().doc.layers.len(), 1);
    }
}
