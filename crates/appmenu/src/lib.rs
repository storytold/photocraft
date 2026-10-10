//! # photocraft-appmenu
//!
//! Publishes an application menu bar over the Canonical AppMenu global-menu protocol
//! ("dbusmenu"): the app serves the `com.canonical.dbusmenu` D-Bus interface and registers
//! its X11 window with the menu registrar (`com.canonical.AppMenu.Registrar`, and its
//! Ayatana fork `org.ayatana.AppMenu.Registrar` when present), so global-menu environments
//! (GNOME with an app-menu shell extension, Unity-like shells, Gershwin Menu) can host the
//! menu natively. Menu clicks come back as [`MenuEvent::Activated`] carrying the item's
//! `action` token from the model.
//!
//! The menu model is plain, toolkit-independent data ([`MenuModel`], [`MenuEntry`]) — no UI
//! crate is involved, so any application can drive it from its own menu tree.
//!

//! The public surface is deliberately **backend-neutral**: portable, toolkit-independent data
//! (`MenuModel`/`MenuEntry`/`MenuCommand`, `Shortcut`), one backend handle ([`AppMenu`])
//! and the click
//! events — with platform backends slotting in behind that seam, and no portable code able
//! to reach a backend's internals. Today: the Linux AppMenu/dbusmenu backend (this crate's
//! live side) and the no-op stub everywhere else; a future macOS backend over AppKit/`muda`
//! implements the same handle (see the README's *Backends* section) without touching the
//! model or any caller.
//!
//! ```no_run
//! use photocraft_appmenu::{MenuEntry, MenuEvent, MenuModel, AppMenu};
//!
//! let model = MenuModel::top(vec![MenuEntry::command("File", "file.new", true)
//!     .submenu(vec![MenuEntry::command("New…", "file.new", true)])]);
//! // Linux/BSD with D-Bus: export and register our X11 top-level windows.
//! // Other platforms: Ok with a no-op handle (activation events never fire).
//! let wake = std::sync::Arc::new(|| {}); // egui: `ctx.request_repaint()`
//! let menu = AppMenu::start("photocraft", wake).ok();
//! if let Some(menu) = &menu {
//!     menu.replace(&model);
//!     while let Some(ev) = menu.try_event() {
//!         if let MenuEvent::Activated { label, action, id } = ev {
//!             // dispatch `action` …
//!             println!("menu click: {label} ({action})");
//!         }
//!     }
//! }
//! ```
//!
//! Never panics (never-crash standard): every entry point returns a `Result` or a
//! no-op handle; malformed input from the bus is rejected; the worker thread retires
//! itself on unrecoverable D-Bus errors instead of crashing the app.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(not(all(unix, not(target_os = "macos"), not(target_arch = "wasm32"))))]
pub use stub::AppMenu;
#[cfg(not(all(unix, not(target_os = "macos"), not(target_arch = "wasm32"))))]
mod stub;

#[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
pub use live::AppMenu;
#[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
mod dbus;
#[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
mod layout;
/// The X11 window scan, exposed (hidden from docs) for diagnostics: which windows the
/// export registrar actually learns about.
#[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
#[doc(hidden)]
pub use x11::top_level_windows;
#[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
mod live;
#[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
mod registrar;
#[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
mod x11;

mod model;

/// Everything the exporter can fail with. Failures are always "feature absent" (no D-Bus
/// session or no registrar): the app keeps its in-window menu bar; activations never fail.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The D-Bus session bus, its services or the X server could not be used.
    #[error("{0}")]
    Platform(String),
}

/// The shell-side waker handed to [`AppMenu::start`]. Clicks land and the hosted flag
/// flips on the exporter's own threads, and a reactive app renders no frames while idle:
/// without the waker a global-menu click would only show at the app's next real input
/// event. The waker makes the next frame happen (egui's `Context::request_repaint` is the
/// natural choice), so `try_event`/`hosted` are read at once.
pub type MenuWake = std::sync::Arc<dyn Fn() + Send + Sync>;

pub use model::{MenuCommand, MenuEntry, MenuModel, Shortcut, ShortcutMod};

/// A click the global menu host sent back.
#[derive(Clone, Debug)]
pub enum MenuEvent {
    /// A menu item was activated (`label` is the item's label, `action` its action token,
    /// `id` the backend's item id — the dbusmenu item id on the Linux backend).
    Activated { label: String, action: String, id: u32 },
}
