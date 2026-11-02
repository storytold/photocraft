use super::*;
use photocraft_gen::Error;
use std::time::{Duration, Instant};

#[test]
fn missing_runtime_is_distinct_from_an_expired_account_and_preserves_document() {
    struct Missing;
    impl AccountService for Missing {
        fn check(&self, _: &dyn Fn() -> bool) -> std::result::Result<AccountInfo, Error> {
            Err(Error::RuntimeUnavailable("Install Codex first".into()))
        }
        fn login(&self, _: &mut dyn FnMut(String) -> std::result::Result<(), Error>, cancel: &dyn Fn() -> bool) -> std::result::Result<AccountInfo, Error> {
            self.check(cancel)
        }
    }
    let mut s = Session::new();
    s.execute("file.new", json!({"width":16,"height":16})).unwrap();
    s.ai_account.set_service(Arc::new(Missing)).unwrap();
    s.execute(COMMAND, json!({"action":"refresh"})).unwrap();
    wait(&mut s, State::RuntimeUnavailable);
    let status = s.execute(COMMAND, json!({"action":"status"})).unwrap();
    assert_eq!(status["state"], "runtimeUnavailable");
    assert_eq!(status["message"], "Install Codex first");
    assert!(s.execute(COMMAND, json!({"action":"loginUrl"})).unwrap()["url"].is_null());
    assert_eq!(s.active().unwrap().doc.layers.len(), 1);
}

struct Mock {
    info: AccountInfo,
    finish_login: Arc<AtomicBool>,
    fail_check: bool,
}

impl AccountService for Mock {
    fn check(&self, _: &dyn Fn() -> bool) -> std::result::Result<AccountInfo, Error> {
        if self.fail_check { Err(Error::Provider("Sign-in expired".into())) } else { Ok(self.info.clone()) }
    }
    fn login(&self, url: &mut dyn FnMut(String) -> std::result::Result<(), Error>, cancelled: &dyn Fn() -> bool) -> std::result::Result<AccountInfo, Error> {
        url("https://auth.openai.com/authorize?state=test".into())?;
        while !self.finish_login.load(Ordering::Acquire) && !cancelled() {
            std::thread::sleep(Duration::from_millis(1));
        }
        // Deliberately complete successfully even after cancellation to test stale-result refusal.
        Ok(AccountInfo::ChatGpt { plan: Some("plus".into()) })
    }
}

fn wait(s: &mut Session, state: State) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        assert!(s.execute(COMMAND, json!({"action":"status"})).is_ok());
        if s.ai_account.snapshot.state == state {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(s.ai_account.snapshot.state, state);
}

#[test]
fn validates_params_without_starting_any_authentication() {
    let mut s = Session::new();
    for params in [Value::Null, json!({}), json!({"action":false}), json!({"action":"logout"}), json!({"action":"login","apiKey":"secret"})] {
        assert!(s.execute(COMMAND, params).is_err());
        assert!(!s.ai_account.pending());
    }
    let state = s.execute(COMMAND, json!({"action":"status"})).unwrap();
    assert_eq!(state["state"], "unknown");
    assert!(!s.ai_account.configured());
    assert!(s.journal.is_empty());
}

#[test]
fn checks_signed_out_supported_and_unsupported_accounts_without_a_document() {
    for (info, expected) in [
        (AccountInfo::SignedOut, State::SignedOut),
        (AccountInfo::Unsupported, State::Unsupported),
        (AccountInfo::ChatGpt { plan: Some("pro".into()) }, State::Connected),
    ] {
        let mut s = Session::new();
        s.ai_account.set_service(Arc::new(Mock { info, finish_login: Arc::new(AtomicBool::new(false)), fail_check: false })).unwrap();
        s.execute(COMMAND, json!({"action":"refresh"})).unwrap();
        wait(&mut s, expected);
        assert!(s.active().is_none());
        assert!(s.journal.is_empty());
    }
}

#[test]
fn login_exposes_url_only_on_explicit_request_and_completion_preserves_document() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":8,"height":8})).unwrap();
    let before = s.active().unwrap().doc.clone();
    let release = Arc::new(AtomicBool::new(false));
    s.ai_account.set_service(Arc::new(Mock { info: AccountInfo::SignedOut, finish_login: release.clone(), fail_check: false })).unwrap();
    s.execute(COMMAND, json!({"action":"login"})).unwrap();
    wait(&mut s, State::WaitingForBrowser);
    let status = s.execute(COMMAND, json!({"action":"status"})).unwrap();
    assert!(!status.to_string().contains("authorize"));
    assert!(s.execute(COMMAND, json!({"action":"loginUrl"})).unwrap()["url"].as_str().unwrap().starts_with("https://auth.openai.com/"));
    assert!(s.execute(COMMAND, json!({"action":"login"})).is_err(), "one worker at a time");
    release.store(true, Ordering::Release);
    wait(&mut s, State::Connected);
    assert!(s.execute(COMMAND, json!({"action":"loginUrl"})).unwrap()["url"].is_null());
    assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
}

#[test]
fn cancellation_discards_late_success_and_expired_refresh_is_actionable() {
    let mut s = Session::new();
    s.ai_account.set_service(Arc::new(Mock { info: AccountInfo::SignedOut, finish_login: Arc::new(AtomicBool::new(false)), fail_check: false })).unwrap();
    s.execute(COMMAND, json!({"action":"login"})).unwrap();
    wait(&mut s, State::WaitingForBrowser);
    s.execute(COMMAND, json!({"action":"cancel"})).unwrap();
    wait(&mut s, State::Cancelled);
    assert!(!s.ai_account.pending());
    s.ai_account.set_service(Arc::new(Mock { info: AccountInfo::SignedOut, finish_login: Arc::new(AtomicBool::new(false)), fail_check: true })).unwrap();
    s.execute(COMMAND, json!({"action":"refresh"})).unwrap();
    wait(&mut s, State::Failed);
    assert_eq!(s.ai_account.snapshot.message.as_deref(), Some("Sign-in expired"));
}

#[test]
fn dropping_session_waits_for_private_account_worker_cleanup() {
    struct Cooperative {
        started: Arc<AtomicBool>,
        finished: Arc<AtomicBool>,
    }
    impl AccountService for Cooperative {
        fn check(&self, cancel: &dyn Fn() -> bool) -> std::result::Result<AccountInfo, Error> {
            self.started.store(true, Ordering::Release);
            while !cancel() {
                std::thread::sleep(Duration::from_millis(1));
            }
            self.finished.store(true, Ordering::Release);
            Err(Error::Cancelled)
        }
        fn login(&self, _: &mut dyn FnMut(String) -> std::result::Result<(), Error>, cancel: &dyn Fn() -> bool) -> std::result::Result<AccountInfo, Error> {
            self.check(cancel)
        }
    }
    let started = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(AtomicBool::new(false));
    let mut s = Session::new();
    s.ai_account.set_service(Arc::new(Cooperative { started: started.clone(), finished: finished.clone() })).unwrap();
    s.execute(COMMAND, json!({"action":"refresh"})).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !started.load(Ordering::Acquire) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(started.load(Ordering::Acquire));
    drop(s);
    assert!(finished.load(Ordering::Acquire), "a normal app exit must not leave its private auth server behind");
}
