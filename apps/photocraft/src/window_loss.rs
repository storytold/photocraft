//! Starting PhotoCraft again after the window's graphics device was lost (#10464).
//!
//! When the device the window draws with is lost (a driver reset, or a driver fault wgpu reports
//! as "Unexpected error variant"), egui-wgpu can't draw another frame and eframe can't build a new
//! device in the same process. The shell (`gpu_status::check_window`) saves every unsaved
//! document for crash recovery and closes; here the process that ended starts a fresh one, which
//! gets a new window and a new device, brings the documents back from recovery, reopens the files
//! that had no unsaved changes, and says what happened in a notice.
//!
//! A device that is lost again right away must not loop: the new process is told when it was
//! started ([`RELAUNCHED_AT`]) and doesn't restart again within [`MIN_SECS_BETWEEN`] seconds; it
//! closes like before instead. Launches with options that apply to one process only (`--safe-gpu`,
//! a present mode, an automation root), a control port or `--new-instance` don't restart at all:
//! the new process would not have those.

use std::ffi::OsString;
use std::sync::{Arc, Mutex, PoisonError};

use photocraft_ui_egui::WindowRestart;

/// The restarted process's environment: when (Unix seconds) its predecessor ended.
pub const RELAUNCHED_AT: &str = "PHOTOCRAFT_GPU_LOSS_RELAUNCHED_AT";
/// The restarted process's environment: why the device was lost (one line), for its notice.
pub const REASON: &str = "PHOTOCRAFT_GPU_LOSS_REASON";
/// A restarted process that loses its device within this many seconds closes instead of
/// restarting again.
pub const MIN_SECS_BETWEEN: u64 = 600;

/// The request the shell made while the app ran, read once the event loop has ended.
#[derive(Clone, Default)]
pub struct Pending(Arc<Mutex<Option<WindowRestart>>>);

impl Pending {
    /// The service handed to the shell.
    pub fn service(&self) -> photocraft_ui_egui::RestartFn {
        let slot = self.0.clone();
        Box::new(move |request| *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(request))
    }

    /// The request, if the shell made one.
    pub fn take(&self) -> Option<WindowRestart> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).take()
    }
}

/// Whether a process may restart now. `previous` is [`RELAUNCHED_AT`] as this process found it
/// (`None` for a launch by the user), `now` the Unix time. An unreadable value counts as a
/// restart that just happened: no restart is better than a loop.
pub fn may_relaunch(previous: Option<&str>, now: u64) -> bool {
    match previous.map(|v| v.trim().parse::<u64>()) {
        None => true,
        Some(Ok(at)) => now.saturating_sub(at) >= MIN_SECS_BETWEEN,
        Some(Err(_)) => false,
    }
}

/// The command line of the restarted process: only the files to reopen (`--` first, so a name that
/// starts with `-` stays a file). The flags of the first launch aren't carried over; see the
/// module docs for the launches that don't restart.
pub fn arguments(reopen: &[String]) -> Vec<OsString> {
    if reopen.is_empty() {
        return Vec::new();
    }
    std::iter::once("--".into()).chain(reopen.iter().map(OsString::from)).collect()
}

/// One line of `reason` for an environment variable: no control characters, never empty.
pub fn env_reason(reason: &str) -> String {
    let line: String = reason.lines().next().unwrap_or("").chars().filter(|c| !c.is_control()).take(300).collect();
    if line.trim().is_empty() { "the graphics device was lost".into() } else { line }
}

/// Start PhotoCraft again for `request`. Call after the event loop ended and the single-instance
/// lock was released. `Err` says why nothing started; the app has closed either way.
pub fn relaunch(request: &WindowRestart) -> Result<(), String> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    if !may_relaunch(std::env::var(RELAUNCHED_AT).ok().as_deref(), now) {
        return Err("it was already restarted after a lost graphics device a short time ago".into());
    }
    let exe = std::env::current_exe().map_err(|e| format!("can't find the program: {e}"))?;
    std::process::Command::new(exe)
        .args(arguments(&request.reopen))
        .env(RELAUNCHED_AT, now.to_string())
        .env(REASON, env_reason(&request.reason))
        .spawn()
        .map(drop)
        .map_err(|e| format!("can't start it: {e}"))
}

/// The notice the restarted process shows (title, line, reason), if it was started after a loss.
pub fn notice(reason: Option<&str>) -> Option<(&'static str, &'static str, String)> {
    let reason = reason.map(str::trim).filter(|r| !r.is_empty())?;
    Some((
        "PhotoCraft restarted",
        "The graphics driver stopped working ({reason}), so PhotoCraft restarted to get a new graphics device. Unsaved changes were restored from crash recovery.",
        reason.to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_user_launch_may_restart_and_a_quick_second_loss_may_not() {
        assert!(may_relaunch(None, 1_000));
        // Restarted 5 s ago: the new process closes instead of looping.
        assert!(!may_relaunch(Some("995"), 1_000));
        assert!(!may_relaunch(Some("1000"), 1_000));
        assert!(!may_relaunch(Some(&(1_000 - MIN_SECS_BETWEEN + 1).to_string()), 1_000));
        // Long enough ago.
        assert!(may_relaunch(Some(&(1_000 - MIN_SECS_BETWEEN).to_string()), 1_000));
        assert!(may_relaunch(Some("0"), 1_000_000));
        // A clock that went backwards, junk and empty values: no restart.
        assert!(!may_relaunch(Some("2000"), 1_000));
        assert!(!may_relaunch(Some("soon"), 1_000));
        assert!(!may_relaunch(Some(""), 1_000));
        assert!(!may_relaunch(Some("-5"), 1_000));
        assert!(may_relaunch(Some(" 0 "), 1_000_000), "whitespace is trimmed");
    }

    #[test]
    fn the_new_process_gets_only_the_files_to_reopen() {
        assert!(arguments(&[]).is_empty());
        let args = arguments(&["C:/a.psd".into(), "-odd.psd".into()]);
        assert_eq!(args, vec![OsString::from("--"), "C:/a.psd".into(), "-odd.psd".into()]);
    }

    #[test]
    fn the_reason_fits_an_environment_variable() {
        assert_eq!(env_reason("Unexpected error variant (driver implementation is at fault)"), "Unexpected error variant (driver implementation is at fault)");
        assert_eq!(env_reason("first\nsecond"), "first");
        assert_eq!(env_reason("a\u{0}b\tc"), "abc");
        assert_eq!(env_reason(""), "the graphics device was lost");
        assert_eq!(env_reason("\n\n"), "the graphics device was lost");
        assert_eq!(env_reason(&"x".repeat(1_000)).chars().count(), 300);
        assert_eq!(env_reason("設備遺失").chars().count(), 4);
    }

    #[test]
    fn the_notice_only_shows_after_a_restart() {
        assert_eq!(notice(None), None);
        assert_eq!(notice(Some("  ")), None);
        let (title, line, reason) = notice(Some(" device is lost ")).unwrap();
        assert_eq!(title, "PhotoCraft restarted");
        assert!(line.contains("{reason}"));
        assert_eq!(reason, "device is lost");
    }

    #[test]
    fn the_service_hands_the_request_to_main_once() {
        let pending = Pending::default();
        assert_eq!(pending.take(), None);
        let mut service = pending.service();
        service(WindowRestart { reason: "lost".into(), reopen: vec!["a.psd".into()] });
        assert_eq!(pending.take(), Some(WindowRestart { reason: "lost".into(), reopen: vec!["a.psd".into()] }));
        assert_eq!(pending.take(), None, "taken once");
    }
}
