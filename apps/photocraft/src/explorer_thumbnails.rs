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
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};

use photocraft_engine::prefs::ExplorerThumbnails;
use serde_json::Value;

/// The preference value this process last saw: read at launch, then updated by every save
/// (0 = launch has not run). A save that leaves it unchanged is not a request to change anything.
static SEEN: AtomicU8 = AtomicU8::new(0);
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
/// process-local value must not restore an outdated registration. Falls back to the last value this
/// process saw when the file cannot be read.
fn current_mode() -> Option<ExplorerThumbnails> {
    PREFS_FILE.get().and_then(|path| read_mode(path)).or_else(|| mode_from(SEEN.load(Ordering::SeqCst)))
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
/// The preference is read here, before any save can happen, so that [`after_save`] can tell a
/// changed setting from an unrelated save.
pub fn startup(prefs_file: Option<&Path>) {
    if !applies() {
        return;
    }
    if let Some(path) = prefs_file {
        let _ = PREFS_FILE.set(path.to_path_buf());
    }
    let text = prefs_file.and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    SEEN.store(code(wanted(&text)), Ordering::SeqCst);
    in_background(|| {
        let _guard = REGISTRATION_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        photocraft_shell::with_registration_lock(|| {
            // The persisted value, read under the lock: another instance may have changed the
            // setting since this process read it at launch.
            let Some(mode) = current_mode() else {
                return;
            };
            let result = if mode == ExplorerThumbnails::Off {
                photocraft_shell::enable_explorer_thumbnails(false, true)
            } else {
                // Typically fails because the DLL is not beside the executable (a development build).
                photocraft_shell::ensure_explorer_thumbnails(mode == ExplorerThumbnails::WithLogo)
            };
            if let Err(e) = result {
                log::debug!("Explorer thumbnails: {e}");
            }
        });
    });
}

/// After the preferences were written: apply a change of the setting, and only a change. The
/// user's explicit choice takes `.psd` over from other programs, and Off removes only PhotoCraft's
/// own registration. Any other save returns at once, without the registry or the lock, so it cannot
/// overwrite another program's handler or stall the window.
///
/// A change that failed is tried again with the next save. The worker applies the value persisted
/// when it holds the lock, so a stale value cannot override another instance's newer choice.
pub fn after_save(text: &str) {
    if !applies() || !note(&SEEN, wanted(text)) {
        return;
    }
    in_background(|| {
        let _guard = REGISTRATION_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        photocraft_shell::with_registration_lock(|| {
            // The persisted value, read under the lock: it is this save's value unless another
            // instance has saved a different one since, which is then the one in force.
            let Some(mode) = current_mode() else {
                return;
            };
            if let Err(e) = photocraft_shell::enable_explorer_thumbnails(mode != ExplorerThumbnails::Off, mode == ExplorerThumbnails::WithLogo) {
                log::debug!("Explorer thumbnails: {e}");
                // Forget that this value was seen, so the next save tries again (unless the
                // setting has moved on, which that save handles).
                let _ = SEEN.compare_exchange(code(mode), 0, Ordering::SeqCst, Ordering::SeqCst);
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_with_logo_unless_the_preference_says_otherwise() {
        assert_eq!(wanted(""), ExplorerThumbnails::WithLogo);
        assert_eq!(wanted("not json"), ExplorerThumbnails::WithLogo);
        assert_eq!(wanted(r#"{"fileHandling": {"autosave": false}}"#), ExplorerThumbnails::WithLogo);
        assert_eq!(wanted(r#"{"fileHandling": {"explorerThumbnails": "off"}}"#), ExplorerThumbnails::Off);
        assert_eq!(wanted(r#"{"fileHandling": {"explorerThumbnails": "noLogo"}}"#), ExplorerThumbnails::NoLogo);
        assert_eq!(wanted(r#"{"fileHandling": {"explorerThumbnails": "withLogo"}}"#), ExplorerThumbnails::WithLogo);
        // The earlier boolean form and unknown values fall back to the default.
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
        std::fs::write(&path, r#"{"fileHandling": {"autosave": false}}"#).unwrap();
        assert_eq!(read_mode(&path), Some(ExplorerThumbnails::WithLogo));
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn only_a_changed_preference_counts_as_a_choice() {
        let seen = AtomicU8::new(code(ExplorerThumbnails::WithLogo));
        // Unrelated saves repeat the value in force: never a request to touch the registry.
        assert!(!note(&seen, ExplorerThumbnails::WithLogo));
        assert!(!note(&seen, ExplorerThumbnails::WithLogo));
        // A real change is, once.
        assert!(note(&seen, ExplorerThumbnails::Off));
        assert!(!note(&seen, ExplorerThumbnails::Off));
        assert!(note(&seen, ExplorerThumbnails::NoLogo));
        // After a failed apply the value is forgotten, so the next save retries it.
        seen.store(0, Ordering::SeqCst);
        assert!(note(&seen, ExplorerThumbnails::NoLogo));
    }
}
