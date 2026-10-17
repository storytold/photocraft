//! Last-resort crash guard (AGENTS.md, Never crash).
//!
//! Code must return errors, never panic. This is the safety net for a panic that escapes anyway:
//! the hook logs it, and [`guard`] turns a panic during file import/export into an error the UI
//! shows, so the open documents stay in place. Commands get the same treatment in
//! `Session::execute`.

use std::ffi::OsString;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};
use std::thread::ThreadId;

/// Armed while the window and GPU are starting: the thread that starts them and the arguments
/// for the safe relaunch.
static STARTUP: Mutex<Option<(ThreadId, Vec<OsString>)>> = Mutex::new(None);
static RELAUNCHED: AtomicBool = AtomicBool::new(false);

/// Log every panic (with its location) before the default hook prints it. A panic on the start-up
/// thread while [`arm_startup_relaunch`] is armed also starts the safe relaunch (see there).
pub fn install_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("PhotoCraft internal error: {info}");
        relaunch_after_startup_panic(&info.to_string());
        default(info);
    }));
}

/// The command that restarts this program in CPU compatibility mode.
fn safe_relaunch_command(exe: &std::path::Path, args: &[OsString], reason: &str) -> Command {
    let mut cmd = Command::new(exe);
    cmd.args(args).arg("--safe-gpu").env_remove("WGPU_BACKEND").env("PHOTOCRAFT_GPU_STARTUP_FAILURE", reason);
    cmd
}

/// Panics while the window opens (a driver or the surface setup) happen inside winit's launch
/// callback, which cannot unwind: the process aborts and no error ever reaches `main`
/// (#5999). So the hook itself starts the relaunch, once, from the start-up thread only.
fn relaunch_after_startup_panic(reason: &str) {
    let armed = STARTUP.try_lock().ok().and_then(|g| g.as_ref().filter(|(t, _)| *t == std::thread::current().id()).map(|(_, a)| a.clone()));
    let Some(args) = armed else { return };
    if RELAUNCHED.swap(true, Ordering::SeqCst) {
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    if let Err(e) = safe_relaunch_command(&exe, &args, &format!("PhotoCraft stopped while the window opened: {reason}")).spawn() {
        log::error!("could not start CPU compatibility mode: {e}");
    }
}

/// From now until [`disarm_startup_relaunch`], a panic on this thread relaunches with `--safe-gpu`.
pub fn arm_startup_relaunch(args: Vec<OsString>) {
    *STARTUP.lock().unwrap_or_else(PoisonError::into_inner) = Some((std::thread::current().id(), args));
}

/// The app has started (or start-up returned an error): panics are ordinary again.
pub fn disarm_startup_relaunch() {
    *STARTUP.lock().unwrap_or_else(PoisonError::into_inner) = None;
}

/// Run `f`; a panic inside it becomes an `Err` naming `what`.
pub fn guard<T>(what: &str, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| Err(format!("{what} failed with an internal error (logged); your open documents are unchanged")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_relaunch_asks_for_the_cpu_renderer_and_drops_wgpu_backend() {
        let cmd = safe_relaunch_command(std::path::Path::new("/x/PhotoCraft"), &["a.psd".into()], "boom");
        let args: Vec<_> = cmd.get_args().collect();
        assert_eq!(args, ["a.psd", "--safe-gpu"]);
        let env: Vec<_> = cmd.get_envs().map(|(k, v)| (k.to_string_lossy().into_owned(), v.map(|v| v.to_string_lossy().into_owned()))).collect();
        assert!(env.contains(&("WGPU_BACKEND".into(), None)));
        assert!(env.contains(&("PHOTOCRAFT_GPU_STARTUP_FAILURE".into(), Some("boom".into()))));
    }

    #[test]
    fn a_panic_on_another_thread_or_when_disarmed_does_not_relaunch() {
        disarm_startup_relaunch();
        relaunch_after_startup_panic("x");
        arm_startup_relaunch(vec![]);
        std::thread::spawn(|| relaunch_after_startup_panic("x")).join().ok();
        disarm_startup_relaunch();
        assert!(!RELAUNCHED.load(Ordering::SeqCst));
    }

    #[test]
    fn guard_turns_a_panic_into_an_error() {
        let r: Result<(), String> = guard("Open", || panic!("boom"));
        assert!(r.unwrap_err().contains("Open failed"));
        assert_eq!(guard("Open", || Ok(3)), Ok(3));
        assert_eq!(guard::<()>("Open", || Err("bad file".into())), Err("bad file".into()));
    }
}
