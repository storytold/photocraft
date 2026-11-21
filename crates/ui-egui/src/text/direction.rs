//! Right-to-left shell layout (Arabic, Hebrew, Persian, Urdu).
//!
//! The direction follows the UI language ([`crate::i18n::Lang::dir`]). In a right-to-left
//! language the window's chrome mirrors, as in right-to-left macOS and Windows apps: the menu
//! bar starts at the right, the toolbox docks on the right, the panel dock and its rail on the
//! left, and the title and status bars read from the right. Panel interiors, dialogs, document
//! tabs and the canvas keep their left-to-right layout for now (`docs/localization-parity.md`).
//!
//! [`current`] and [`is_rtl`] feed the framework's `RootEnv::direction`; the framework's menu
//! bar, bars and title bar mirror by that direction.
//!
//! `PHOTOCRAFT_UI_DIRECTION=rtl` (or `ltr`) forces a direction whatever the language, so the
//! mirrored layout can be checked before a right-to-left catalog is installed. It is read once
//! per process; the web build has no environment and always follows the language.

use std::cell::Cell;
use std::sync::OnceLock;

pub use crate::i18n::Dir;

thread_local! {
    // Like the drawing language, per thread: test harnesses run side by side.
    static FORCED: Cell<Option<Dir>> = const { Cell::new(None) };
}

/// The direction the UI is drawn in on this thread.
pub fn current() -> Dir {
    FORCED.get().or_else(env_override).unwrap_or_else(|| crate::i18n::current().dir())
}

/// Is the UI drawn right-to-left?
pub fn is_rtl() -> bool {
    current() == Dir::Rtl
}

/// Draw in `dir` whatever the language until the guard drops on this thread (tests).
#[must_use]
pub fn direction_scope(dir: Dir) -> impl Drop {
    struct Restore {
        previous: Option<Dir>,
        _thread: std::marker::PhantomData<std::rc::Rc<()>>,
    }
    impl Drop for Restore {
        fn drop(&mut self) {
            FORCED.set(self.previous);
        }
    }
    let restore = Restore { previous: FORCED.get(), _thread: std::marker::PhantomData };
    FORCED.set(Some(dir));
    restore
}

fn env_override() -> Option<Dir> {
    // Unit tests follow the language or a `direction_scope`, whatever the developer's shell sets
    // (the value is cached per process, so a test can't clear it).
    if cfg!(test) {
        return None;
    }
    static ENV: OnceLock<Option<Dir>> = OnceLock::new();
    *ENV.get_or_init(|| parse_dir(&std::env::var("PHOTOCRAFT_UI_DIRECTION").ok()?))
}

/// `rtl` / `ltr`, any case; anything else (including empty) follows the language.
fn parse_dir(value: &str) -> Option<Dir> {
    match value.trim().to_ascii_lowercase().as_str() {
        "rtl" => Some(Dir::Rtl),
        "ltr" => Some(Dir::Ltr),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
