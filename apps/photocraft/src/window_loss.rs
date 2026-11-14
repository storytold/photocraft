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
//! started ([`RELAUNCHED_AT`]) and doesn't restart again within [`MIN_SECS_BETWEEN`] seconds, and
//! how many restarts led to it ([`RESTARTS`]), at most [`MAX_RESTARTS`] in a row (a clock that
//! jumps forward can't defeat that); it closes like before instead. Launches with options that apply to one process only (`--safe-gpu`,
//! a present mode, an automation root), a control port or `--new-instance` don't restart at all:
//! the new process would not have those.

use std::ffi::OsString;
use std::sync::{Arc, Mutex, PoisonError};

use photocraft_ui_egui::WindowRestart;

use crate::launch_args::PerProcess;

/// The restarted process's environment: when (Unix seconds) its predecessor ended.
pub const RELAUNCHED_AT: &str = "PHOTOCRAFT_GPU_LOSS_RELAUNCHED_AT";
/// The restarted process's environment: how many restarts in a row led to it. A wall clock that
/// jumps forward can't be trusted to space the restarts, so the count caps them too.
pub const RESTARTS: &str = "PHOTOCRAFT_GPU_LOSS_RESTARTS";
/// The restarted process's environment: why the device was lost (one line), for its notice.
pub const REASON: &str = "PHOTOCRAFT_GPU_LOSS_REASON";
/// The restarted process's environment: how many open files without unsaved changes it was not
/// told to reopen (too many for a command line), for its notice.
pub const LEFT_OUT: &str = "PHOTOCRAFT_GPU_LOSS_LEFT_OUT";
/// A restarted process that loses its device within this many seconds closes instead of
/// restarting again.
pub const MIN_SECS_BETWEEN: u64 = 600;
/// A chain of restarts ends after this many; a launch by the user starts a new chain.
pub const MAX_RESTARTS: u32 = 5;

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

/// Whether a process may restart now. `previous` is [`RELAUNCHED_AT`] and `restarts` is
/// [`RESTARTS`] as this process found them (`None` for a launch by the user), `now` the Unix time.
/// An unreadable value counts as a restart that just happened: no restart is better than a loop.
pub fn may_relaunch(previous: Option<&str>, restarts: Option<&str>, now: u64) -> bool {
    let spaced = match previous.map(|v| v.trim().parse::<u64>()) {
        None => true,
        Some(Ok(at)) => now.saturating_sub(at) >= MIN_SECS_BETWEEN,
        Some(Err(_)) => false,
    };
    let few = match restarts.map(|v| v.trim().parse::<u32>()) {
        None => true,
        Some(Ok(n)) => n < MAX_RESTARTS,
        Some(Err(_)) => false,
    };
    spaced && few
}

/// Whether a launch starts the way its restarted process would: no per-process option (from the
/// command line or from the environment, see `single_instance::env_launch_options`), no control
/// port and no separate instance (`--new-instance` or `PHOTOCRAFT_NEW_INSTANCE`).
pub fn plain_launch(launch_options: &[PerProcess], env_options: &[PerProcess], control: bool, new_instance_flag: bool, new_instance_env: Option<&str>) -> bool {
    launch_options.is_empty() && env_options.is_empty() && !control && !crate::single_instance::new_instance_requested(new_instance_flag, new_instance_env)
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
    let restarts = std::env::var(RESTARTS).ok();
    if !may_relaunch(std::env::var(RELAUNCHED_AT).ok().as_deref(), restarts.as_deref(), now) {
        return Err("it was already restarted after a lost graphics device a short time ago, or too many times in a row".into());
    }
    let restarts = restarts.and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(0).saturating_add(1);
    let exe = std::env::current_exe().map_err(|e| format!("can't find the program: {e}"))?;
    std::process::Command::new(exe)
        .args(arguments(&request.reopen))
        .env(RELAUNCHED_AT, now.to_string())
        .env(RESTARTS, restarts.to_string())
        .env(REASON, env_reason(&request.reason))
        .env(LEFT_OUT, request.left_out.to_string())
        .spawn()
        .map(drop)
        .map_err(|e| format!("can't start it: {e}"))
}

/// What the restarted process's notice says: title, lines and the `{name}` values in them.
pub type Notice = (&'static str, Vec<String>, Vec<(String, String)>);

/// The notice the restarted process shows, if it was started after a loss. `left_out` is
/// [`LEFT_OUT`] as found: a second line names the files that were not reopened.
pub fn notice(reason: Option<&str>, left_out: Option<&str>) -> Option<Notice> {
    let reason = reason.map(str::trim).filter(|r| !r.is_empty())?;
    let mut lines = vec!["The graphics driver stopped working ({reason}), so PhotoCraft restarted to get a new graphics device.".to_string()];
    let mut args = vec![("reason".to_string(), reason.to_string())];
    let left_out = left_out.and_then(|n| n.trim().parse::<usize>().ok()).filter(|n| *n > 0);
    if let Some(n) = left_out {
        lines.push("{count} open file(s) without unsaved changes were not reopened. Open them again from File > Open Recent.".into());
        args.push(("count".into(), n.to_string()));
    }
    Some(("PhotoCraft restarted", lines, args))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_user_launch_may_restart_and_a_quick_second_loss_may_not() {
        assert!(may_relaunch(None, None, 1_000));
        // Restarted 5 s ago: the new process closes instead of looping.
        assert!(!may_relaunch(Some("995"), None, 1_000));
        assert!(!may_relaunch(Some("1000"), None, 1_000));
        assert!(!may_relaunch(Some(&(1_000 - MIN_SECS_BETWEEN + 1).to_string()), None, 1_000));
        // Long enough ago.
        assert!(may_relaunch(Some(&(1_000 - MIN_SECS_BETWEEN).to_string()), None, 1_000));
        assert!(may_relaunch(Some("0"), None, 1_000_000));
        // A clock that went backwards, junk and empty values: no restart.
        assert!(!may_relaunch(Some("2000"), None, 1_000));
        assert!(!may_relaunch(Some("soon"), None, 1_000));
        assert!(!may_relaunch(Some(""), None, 1_000));
        assert!(!may_relaunch(Some("-5"), None, 1_000));
        assert!(may_relaunch(Some(" 0 "), None, 1_000_000), "whitespace is trimmed");
    }

    /// A clock that jumped forward makes every restart look long ago; the count still stops a loop.
    #[test]
    fn the_number_of_restarts_in_a_row_is_capped() {
        let long_ago = Some("0");
        let now = 10_000_000;
        assert!(may_relaunch(long_ago, Some("0"), now));
        assert!(may_relaunch(long_ago, Some(&(MAX_RESTARTS - 1).to_string()), now));
        assert!(!may_relaunch(long_ago, Some(&MAX_RESTARTS.to_string()), now));
        assert!(!may_relaunch(long_ago, Some(&u32::MAX.to_string()), now));
        assert!(!may_relaunch(long_ago, Some("many"), now), "junk counts as too many");
        assert!(!may_relaunch(long_ago, Some("-1"), now));
        assert!(!may_relaunch(Some("now"), Some("0"), now), "the time still has to be spaced");
    }

    /// An option set through the environment (`PHOTOCRAFT_PRESENT_MODE`, say) or
    /// `PHOTOCRAFT_NEW_INSTANCE` is a per-process launch like its flag: it doesn't restart.
    #[test]
    fn only_a_plain_launch_restarts() {
        assert!(plain_launch(&[], &[], false, false, None));
        assert!(plain_launch(&[], &[], false, false, Some("")), "an empty variable is unset");
        assert!(plain_launch(&[], &[], false, false, Some("0")));
        assert!(!plain_launch(&[PerProcess::SafeGpu], &[], false, false, None));
        assert!(!plain_launch(&[], &[PerProcess::PresentMode], false, false, None), "PHOTOCRAFT_PRESENT_MODE");
        assert!(!plain_launch(&[], &[], true, false, None));
        assert!(!plain_launch(&[], &[], false, true, None));
        assert!(!plain_launch(&[], &[], false, false, Some("1")), "PHOTOCRAFT_NEW_INSTANCE");
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
        assert_eq!(notice(None, None), None);
        assert_eq!(notice(Some("  "), Some("3")), None);
        let (title, lines, args) = notice(Some(" device is lost "), None).unwrap();
        assert_eq!(title, "PhotoCraft restarted");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("{reason}"));
        assert_eq!(args, vec![("reason".to_string(), "device is lost".to_string())]);
        // Nothing here knows whether recovery loaded anything, so the notice doesn't say so.
        let text = lines[0].to_lowercase();
        assert!(!text.contains("recover") && !text.contains("restored"), "{text}");
    }

    /// Files that didn't fit on the command line are reported, not dropped without a word.
    #[test]
    fn the_notice_names_the_files_left_out() {
        for none in [None, Some("0"), Some("junk"), Some("")] {
            assert_eq!(notice(Some("lost"), none).unwrap().1.len(), 1, "{none:?}");
        }
        let (_, lines, args) = notice(Some("lost"), Some(" 7 ")).unwrap();
        assert_eq!(lines.len(), 2);
        assert!(lines[1].contains("{count}"));
        assert_eq!(args[1], ("count".to_string(), "7".to_string()));
    }

    #[test]
    fn the_service_hands_the_request_to_main_once() {
        let pending = Pending::default();
        assert_eq!(pending.take(), None);
        let mut service = pending.service();
        service(WindowRestart { reason: "lost".into(), reopen: vec!["a.psd".into()], left_out: 0 });
        assert_eq!(pending.take(), Some(WindowRestart { reason: "lost".into(), reopen: vec!["a.psd".into()], left_out: 0 }));
        assert_eq!(pending.take(), None, "taken once");
    }
}
