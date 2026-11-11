//! Windows only: keeps File Explorer's PSD/PSB/PhotoCraft thumbnails in step with Preferences › File Handling ›
//! Explorer Thumbnails (`fileHandling.explorerThumbnails`: Off, On with logo, On no logo; on with
//! logo by default).
//!
//! The thumbnail provider itself is `photocraft_shell.dll` (the `photocraft-shell` crate), shipped
//! next to `photocraft.exe`. Registering it only touches the current user's registry.
//!
//! Only a *change* of the preference is an explicit choice. Every other preferences save leaves the
//! registry alone, so it can neither take `.psd` over from another program nor wait for the
//! registration lock; the registry work for a real change runs on its own thread.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};

use photocraft_engine::prefs::ExplorerThumbnails;
use serde_json::Value;

/// The preference value this process last saw: read at launch, then updated by every save
/// (0 = launch has not run / baseline uninitialized). A save that leaves it unchanged is not a request to change anything.
static SEEN: AtomicU8 = AtomicU8::new(0);

/// Tracks whether an automatic registration retry is pending. Keeps automatic retries isolated
/// from explicit user selections to prevent overwriting third-party PSD handlers.
static RETRY_PENDING: AtomicBool = AtomicBool::new(false);

/// Serializes this process's registration threads (startup and preference changes).
static REGISTRATION_LOCK: Mutex<()> = Mutex::new(());

/// The preferences file, so a worker can re-read what is persisted now (set by [`startup`]).
static PREFS_FILE: OnceLock<PathBuf> = OnceLock::new();

/// A stable number for each mode, used to remember what was last seen.
fn code(mode: ExplorerThumbnails) -> u8 {
    match mode {
        ExplorerThumbnails::Off => 1,
        ExplorerThumbnails::WithLogo => 2,
        ExplorerThumbnails::NoLogo => 3,
    }
}

/// The mode a [`code`] stands for; `None` for 0 (not seen yet) or an unknown number.
#[cfg(test)]
fn mode_from(code: u8) -> Option<ExplorerThumbnails> {
    [ExplorerThumbnails::Off, ExplorerThumbnails::WithLogo, ExplorerThumbnails::NoLogo].into_iter().find(|&mode| self::code(mode) == code)
}

/// Records `mode` as the preference now in force; true when it differs from the one seen before.
fn note(seen: &AtomicU8, mode: ExplorerThumbnails) -> bool {
    seen.swap(code(mode), Ordering::SeqCst) != code(mode)
}

/// The preference in the preferences text; the default when the file or the value is missing.
fn wanted(text: &str) -> ExplorerThumbnails {
    let v: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    v.get("fileHandling").and_then(|f| f.get("explorerThumbnails")).and_then(Value::as_str).and_then(ExplorerThumbnails::parse).unwrap_or_default()
}

/// The preference persisted in `path`; `None` when the file cannot be read.
fn read_mode(path: &Path) -> Option<ExplorerThumbnails> {
    std::fs::read_to_string(path).ok().map(|text| wanted(&text))
}

/// The value a worker should apply. Called while holding the cross-process registration lock: another
/// instance may have saved a different setting since this process last saw one, and a stale
/// process-local value must not restore an outdated registration. Returns `None` if the preferences file
/// cannot be read to prevent applying a stale local fallback.
fn current_mode() -> Option<ExplorerThumbnails> {
    PREFS_FILE.get().and_then(|path| read_mode(path))
}

/// Safely re-arms a failed request for subsequent save attempts without overwriting a newer pending choice.
fn rearm_failed_request_if_current(failed_mode: ExplorerThumbnails) {
    if let Some(current) = current_mode()
        && current == failed_mode
    {
        let _ = SEEN.compare_exchange(code(failed_mode), 0, Ordering::SeqCst, Ordering::SeqCst);
        RETRY_PENDING.store(true, Ordering::SeqCst);
    }
}

/// Only the normal per-user install writes the registry: a portable copy leaves nothing behind, a
/// `PHOTOCRAFT_CONFIG_DIR` run (tests, agents) must not touch the real user's settings, and a
/// 32-bit build's class registration is invisible to the 64-bit Explorer.
fn applies() -> bool {
    cfg!(target_pointer_width = "64") && crate::app_dirs::current().mode == crate::app_dirs::Mode::Platform
}

/// Runs `work` on its own thread: writing the registry, waiting for another process's registration
/// lock and telling the shell must never delay the window or a preferences save.
fn in_background(work: impl FnOnce() + Send + 'static) {
    if let Err(e) = std::thread::Builder::new().name("explorer-thumbnails".into()).spawn(work) {
        log::debug!("Explorer thumbnails: {e}");
    }
}

/// At launch: unless the preference is Off, register the thumbnails, except where another program
/// (Photoshop) already provides them. Turning them off is only ever done from the preferences.
///
/// If startup read fails, baseline SEEN remains 0 (uninitialized) until UI preference load.
pub fn startup(prefs_file: Option<&Path>) {
    if !applies() {
        return;
    }
    if let Some(path) = prefs_file {
        let _ = PREFS_FILE.set(path.to_path_buf());
    }

    let mode_read = prefs_file.and_then(read_mode);
    if let Some(mode) = mode_read {
        SEEN.store(code(mode), Ordering::SeqCst);
    } else {
        SEEN.store(0, Ordering::SeqCst);
    }

    in_background(|| {
        let _guard = REGISTRATION_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        let lock_result = photocraft_shell::with_registration_lock(|| {
            let Some(mode) = current_mode() else {
                return Ok(());
            };
            if mode == ExplorerThumbnails::Off {
                photocraft_shell::enable_explorer_thumbnails(false, true)
            } else {
                photocraft_shell::ensure_explorer_thumbnails(mode == ExplorerThumbnails::WithLogo)
            }
        });

        match lock_result {
            Ok(()) => {}
            Err(e) => {
                log::debug!("Explorer thumbnails startup/lock retry failed: {e}");
                if let Some(mode) = current_mode() {
                    rearm_failed_request_if_current(mode);
                }
            }
        }
    });
}

/// Called during UI preferences initialization to set the comparison baseline if startup read failed.
#[allow(dead_code)]
pub fn init_baseline_from_ui(ui_mode: Option<ExplorerThumbnails>) {
    let mode = ui_mode.unwrap_or_default();
    let _ = SEEN.compare_exchange(0, code(mode), Ordering::SeqCst, Ordering::SeqCst);
}

/// After the preferences were written: apply a change of the setting, and only a change.
pub fn after_save(text: &str) {
    let new_mode = wanted(text);
    let is_changed = note(&SEEN, new_mode);
    let is_retry = RETRY_PENDING.swap(false, Ordering::SeqCst);

    if !applies() || (!is_changed && !is_retry) {
        return;
    }

    in_background(move || {
        let _guard = REGISTRATION_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        let lock_result = photocraft_shell::with_registration_lock(|| {
            let Some(mode) = current_mode() else {
                return Ok(());
            };

            if is_changed {
                photocraft_shell::enable_explorer_thumbnails(mode != ExplorerThumbnails::Off, mode == ExplorerThumbnails::WithLogo)
            } else if mode == ExplorerThumbnails::Off {
                photocraft_shell::enable_explorer_thumbnails(false, true)
            } else {
                photocraft_shell::ensure_explorer_thumbnails(mode == ExplorerThumbnails::WithLogo)
            }
        });

        match lock_result {
            Ok(()) => {}
            Err(e) => {
                log::debug!("Explorer thumbnails registration failed: {e}");
                if let Some(mode) = current_mode() {
                    rearm_failed_request_if_current(mode);
                } else {
                    RETRY_PENDING.store(true, Ordering::SeqCst);
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn on_with_logo_unless_the_preference_says_otherwise() {
        assert_eq!(wanted(""), ExplorerThumbnails::WithLogo);
        assert_eq!(wanted("not json"), ExplorerThumbnails::WithLogo);
        assert_eq!(wanted(r#"{"fileHandling": {"autosave": false}}"#), ExplorerThumbnails::WithLogo);
        assert_eq!(wanted(r#"{"fileHandling": {"explorerThumbnails": "off"}}"#), ExplorerThumbnails::Off);
        assert_eq!(wanted(r#"{"fileHandling": {"explorerThumbnails": "noLogo"}}"#), ExplorerThumbnails::NoLogo);
        assert_eq!(wanted(r#"{"fileHandling": {"explorerThumbnails": "withLogo"}}"#), ExplorerThumbnails::WithLogo);
        assert_eq!(wanted(r#"{"fileHandling": {"explorerThumbnails": false}}"#), ExplorerThumbnails::WithLogo);
        assert_eq!(wanted(r#"{"fileHandling": {"explorerThumbnails": "sometimes"}}"#), ExplorerThumbnails::WithLogo);
    }

    #[test]
    fn codes_round_trip_and_zero_means_not_seen() {
        for mode in [ExplorerThumbnails::Off, ExplorerThumbnails::WithLogo, ExplorerThumbnails::NoLogo] {
            assert_eq!(mode_from(code(mode)), Some(mode));
        }
        assert_eq!(mode_from(0), None);
        assert_eq!(mode_from(200), None);
    }

    #[test]
    fn the_persisted_preference_is_read_from_the_file() {
        let path = std::env::temp_dir().join(format!("photocraft-explorer-thumbnails-{}.json", std::process::id()));
        assert_eq!(read_mode(&path), None, "a missing file is not a preference");
        std::fs::write(&path, r#"{"fileHandling": {"explorerThumbnails": "off"}}"#).unwrap();
        assert_eq!(read_mode(&path), Some(ExplorerThumbnails::Off));
        std::fs::write(&path, r#"{"fileHandling": {"explorerThumbnails": "withLogo"}}"#).unwrap();
        assert_eq!(read_mode(&path), Some(ExplorerThumbnails::WithLogo));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn unreadable_file_prevents_stale_mode_fallback() {
        let path = std::env::temp_dir().join(format!("photocraft-explorer-thumbnails-missing-{}.json", std::process::id()));
        let _ = PREFS_FILE.set(path.clone());
        SEEN.store(code(ExplorerThumbnails::Off), Ordering::SeqCst);

        assert_eq!(current_mode(), None);
    }

    /// Helper for testable worker logic accepting path and callback.
    fn execute_test_worker<F>(path: &Path, on_apply: F) -> Option<ExplorerThumbnails>
    where
        F: FnOnce(Option<ExplorerThumbnails>),
    {
        let _guard = REGISTRATION_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        let mode = read_mode(path);
        on_apply(mode);
        mode
    }

    #[test]
    fn concurrent_workers_re_read_persisted_file_order() {
        let path = std::env::temp_dir().join(format!("photocraft-explorer-thumbnails-race-{}.json", std::process::id()));
        std::fs::write(&path, r#"{"fileHandling": {"explorerThumbnails": "withLogo"}}"#).unwrap();

        let applied_mode = Arc::new(Mutex::new(None));
        let worker_finished = Arc::new(AtomicBool::new(false));

        let path_clone = path.clone();
        let applied_mode_clone = Arc::clone(&applied_mode);
        let worker_finished_clone = Arc::clone(&worker_finished);

        let handle = std::thread::spawn(move || {
            // Update preferences file mid-flight before worker evaluates read_mode() under lock
            std::fs::write(&path_clone, r#"{"fileHandling": {"explorerThumbnails": "off"}}"#).unwrap();

            execute_test_worker(&path_clone, |mode| {
                *applied_mode_clone.lock().unwrap() = mode;
            });
            worker_finished_clone.store(true, Ordering::SeqCst);
        });

        handle.join().unwrap();

        assert!(worker_finished.load(Ordering::SeqCst));
        assert_eq!(*applied_mode.lock().unwrap(), Some(ExplorerThumbnails::Off));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn only_a_changed_preference_counts_as_a_choice() {
        let seen = AtomicU8::new(code(ExplorerThumbnails::WithLogo));
        assert!(!note(&seen, ExplorerThumbnails::WithLogo));
        assert!(!note(&seen, ExplorerThumbnails::WithLogo));
        assert!(note(&seen, ExplorerThumbnails::Off));
        assert!(!note(&seen, ExplorerThumbnails::Off));
        assert!(note(&seen, ExplorerThumbnails::NoLogo));
        seen.store(0, Ordering::SeqCst);
        assert!(note(&seen, ExplorerThumbnails::NoLogo));
    }
}
