//! Windows only: keeps File Explorer's PSD/PSB/PhotoCraft thumbnails in step with Preferences › File Handling ›
//! Explorer Thumbnails (`fileHandling.explorerThumbnails`: Off, On with logo, On no logo; on with
//! logo by default).
//!
//! The thumbnail provider itself is `photocraft_shell.dll` (the `photocraft-shell` crate), shipped
//! next to `photocraft.exe`. Registering it only touches the current user's registry.

use std::path::Path;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, PoisonError};

use photocraft_engine::prefs::ExplorerThumbnails;
use serde_json::Value;

/// What the registry was last set to by this process (0 = not yet).
static APPLIED: AtomicU8 = AtomicU8::new(0);
/// Serializes asynchronous startup registration with preference saves.
static REGISTRATION_LOCK: Mutex<()> = Mutex::new(());

/// A stable number for each mode, used to remember what was last applied.
fn code(mode: ExplorerThumbnails) -> u8 {
    match mode {
        ExplorerThumbnails::Off => 1,
        ExplorerThumbnails::WithLogo => 2,
        ExplorerThumbnails::NoLogo => 3,
    }
}

/// The preference in the preferences text; the default when the file or the value is missing.
fn wanted(text: &str) -> ExplorerThumbnails {
    let v: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    v.get("fileHandling").and_then(|f| f.get("explorerThumbnails")).and_then(Value::as_str).and_then(ExplorerThumbnails::parse).unwrap_or_default()
}

/// Only the normal per-user install writes the registry: a portable copy leaves nothing behind, a
/// `PHOTOCRAFT_CONFIG_DIR` run (tests, agents) must not touch the real user's settings, and a
/// 32-bit build's class registration is invisible to the 64-bit Explorer.
fn applies() -> bool {
    cfg!(target_pointer_width = "64") && crate::app_dirs::current().mode == crate::app_dirs::Mode::Platform
}

/// At launch: unless the preference is Off, register the thumbnails, except where another program
/// (Photoshop) already provides them. Turning them off is only ever done from the preferences.
/// Runs on its own thread: writing the registry and telling the shell must not delay the window.
pub fn startup(prefs_file: Option<&Path>) {
    if !applies() {
        return;
    }
    let prefs_file = prefs_file.map(Path::to_path_buf);
    let work = move || {
        let _guard = REGISTRATION_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        let mode = wanted(&prefs_file.and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default());
        if mode == ExplorerThumbnails::Off {
            match photocraft_shell::enable_explorer_thumbnails(false, true) {
                Ok(()) => APPLIED.store(code(mode), Ordering::Relaxed),
                Err(e) => log::debug!("Explorer thumbnails: {e}"),
            }
        } else if let Err(e) = photocraft_shell::ensure_explorer_thumbnails(mode == ExplorerThumbnails::WithLogo) {
            // Typically the DLL is not beside the executable (a development build).
            log::debug!("Explorer thumbnails: {e}");
        } else {
            APPLIED.store(code(mode), Ordering::Relaxed);
        }
    };
    if let Err(e) = std::thread::Builder::new().name("explorer-thumbnails".into()).spawn(work) {
        log::debug!("Explorer thumbnails: {e}");
    }
}

/// After the preferences were written: apply a change of the setting. The user's explicit choice
/// takes `.psd` over from other programs, and Off removes only PhotoCraft's own registration. A
/// change that failed is tried again with the next save.
pub fn after_save(text: &str) {
    if !applies() {
        return;
    }
    let _guard = REGISTRATION_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let mode = wanted(text);
    if APPLIED.load(Ordering::Relaxed) == code(mode) {
        return;
    }
    match photocraft_shell::enable_explorer_thumbnails(mode != ExplorerThumbnails::Off, mode == ExplorerThumbnails::WithLogo) {
        Ok(()) => APPLIED.store(code(mode), Ordering::Relaxed),
        Err(e) => log::debug!("Explorer thumbnails: {e}"),
    }
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
}
