//! The live exporter: a background thread owns one D-Bus session connection, serves the
//! dbusmenu interface on it, publishes model changes as signals (only what changed), and
//! keeps the registrar pointed at our X11 windows. The UI thread only pushes models and
//! drains clicks — non-blocking on both sides.

use crate::dbus::{MENU_OBJECT_PATH, MenuBarIface, MenuState, wire_props};
use crate::layout::{FlatMenu, ROOT_ID, diff};
use crate::registrar::Registrar;
use crate::x11;
use crate::{Error, MenuEvent, MenuModel, MenuWake};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use zbus::blocking::Connection;
use zvariant::OwnedValue;

/// How often the worker wakes to re-scan X11 windows and retry the registrar. Cheap: one
/// `QueryTree` round trip and, while no registrar exists, one failing D-Bus call.
const SCAN_INTERVAL: Duration = Duration::from_secs(1);

/// Bounds every outbound call the worker's connection makes — the registrar's
/// `GetMenuForWindow` / `RegisterWindow` / `UnregisterWindow` and the internal bus calls —
/// so a registrar that is alive on the bus but wedged (answering nothing) cannot stall the
/// scan for zbus's 25 s default, sequentially per window, while publishes queue and
/// activations and `hosted()` go stale. A deadline miss surfaces as `Err`, which the scan
/// already handles (retry the registration / forget the sticky service choice).
const REGISTRAR_CALL_TIMEOUT: Duration = Duration::from_secs(2);

enum Message {
    Model(MenuModel),
    Shutdown,
}

/// The server-side menu interface handle for one app.
pub struct AppMenu {
    tx: Sender<Message>,
    state: Arc<Mutex<MenuState>>,
    /// Set by the worker once the connection is up: our unique bus name.
    bus: Arc<Mutex<Option<String>>>,
    /// Set true while a registrar holds a registration for one of our windows — a global menu
    /// host is *serving* the menus, and the app can then hide its in-window menu bar.
    hosted: Arc<AtomicBool>,
}

impl AppMenu {
    /// Starts the exporter on a background thread. Fails when the session bus is
    /// unavailable (the app then uses only its in-window menu bar). `wake` is called on
    /// the exporter's threads whenever a click is recorded or the hosted flag flips, so a
    /// reactive app renders the frame in which `try_event`/`hosted` are read at once
    /// instead of at its next input event.
    pub fn start(app_name: &str, wake: MenuWake) -> Result<AppMenu, Error> {
        let state = Arc::new(Mutex::new(MenuState::default()));
        {
            // The D-Bus interface threads read it from the shared state when a click lands.
            let mut locked = state.lock().unwrap_or_else(PoisonError::into_inner);
            locked.wake = Some(wake.clone());
        }
        let bus = Arc::new(Mutex::new(None));
        let hosted = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let worker_state = state.clone();
        let worker_bus = bus.clone();
        let worker_hosted = hosted.clone();
        let name = app_name.to_string();
        std::thread::Builder::new()
            .name("photocraft-appmenu".into())
            .spawn(move || worker(&name, rx, worker_state, worker_bus, worker_hosted, wake))
            .map_err(|e| Error::Platform(format!("cannot start the appmenu thread: {e}")))?;
        Ok(AppMenu { tx, state, bus, hosted })
    }

    /// The unique bus name the menu is served on, once the connection is up. A
    /// Linux-backend detail (D-Bus connection address of the export), mostly for
    /// diagnostics and tests; importers find us through the registrar, not by name.
    pub fn bus_name(&self) -> Option<String> {
        self.bus.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Publishes a new model: the worker diffs it against the last one and emits only what
    /// changed, coalescing bursts of models into one signal batch. Non-blocking.
    pub fn replace(&self, model: &MenuModel) {
        let _ = self.tx.send(Message::Model(model.clone()));
    }

    /// Takes the oldest pending menu click, if any (the UI calls this from its update loop).
    pub fn try_event(&self) -> Option<MenuEvent> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let (id, label, action) = state.activations.pop_front()?;
        Some(MenuEvent::Activated { label, action, id })
    }

    /// Whether a menu host is currently serving the menus on this app's behalf, i.e. the app
    /// should hide its in-window menu bar (letting the host's bar own the menus). What a host
    /// is depends on the backend — here, the Linux one, a D-Bus menu registrar holding a
    /// registration for one of the app's windows; a future macOS backend (the platform menu
    /// bar) would always answer true, the no-op stub always false. Portable contract, so the
    /// shell calls it on every platform the same way; the flag follows the registrations every
    /// scan, so it flips back at the same second a host (or its service) disappears.
    pub fn hosted(&self) -> bool {
        self.hosted.load(Ordering::Relaxed)
    }

    /// Unregisters the windows and closes the connection. Called on drop; safe to call
    /// again.
    pub fn shutdown(&self) {
        let _ = self.tx.send(Message::Shutdown);
    }
}

impl Drop for AppMenu {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn worker(app_name: &str, rx: Receiver<Message>, state: Arc<Mutex<MenuState>>, bus: Arc<Mutex<Option<String>>>, hosted: Arc<AtomicBool>, wake: MenuWake) {
    log::debug!("AppMenu: worker starting ({app_name})");
    let Ok(conn) = connection(&state, &bus) else {
        log::debug!("AppMenu: no D-Bus session bus; global menu stays off");
        return;
    };
    let mut registrar = Registrar::new(conn.unique_name().map(|n| n.to_string()).as_deref().unwrap_or_default());
    let mut registered: HashMap<u32, ()> = HashMap::new();
    loop {
        // A burst of models collapses into one publish: only the newest matters.
        let mut model: Option<MenuModel> = None;
        let stop = match rx.recv_timeout(SCAN_INTERVAL) {
            Ok(Message::Model(m)) => {
                model = Some(m);
                while let Ok(Message::Model(m)) = rx.try_recv() {
                    model = Some(m);
                }
                false
            }
            Ok(Message::Shutdown) | Err(RecvTimeoutError::Disconnected) => true,
            Err(RecvTimeoutError::Timeout) => false,
        };
        if let Some(model) = &model {
            publish(&state, &conn, model);
        }
        if stop {
            break;
        }
        let mut windows = X11Windows;
        let mut view = RegistrarView { conn: &conn, registrar: &mut registrar };
        sync_registrations(&mut windows, &mut view, &mut registered);
        let now = !registered.is_empty();
        if hosted.swap(now, Ordering::Relaxed) != now {
            // The hosted flag drives the shell's title-bar decision: wake the app, or a
            // host appearing/vanishing is only rendered at the next input event.
            wake();
        }
    }
    hosted.store(false, Ordering::Relaxed);
    for &window in registered.keys() {
        registrar.unregister(&conn, window);
    }
    let _ = conn.close();
    log::debug!("AppMenu: worker finished");
}

/// Connects to the session bus and serves the dbusmenu interface at `MENU_OBJECT_PATH`.
fn connection(state: &Arc<Mutex<MenuState>>, bus: &Arc<Mutex<Option<String>>>) -> Result<Connection, Error> {
    let iface = MenuBarIface { state: state.clone() };
    let conn = zbus::blocking::connection::Builder::session()
        .and_then(|b| b.method_timeout(REGISTRAR_CALL_TIMEOUT).serve_at(MENU_OBJECT_PATH, iface))
        .and_then(|b| b.build())
        .map_err(|e| Error::Platform(format!("D-Bus session: {e}")))?;
    let mut slot = bus.lock().unwrap_or_else(PoisonError::into_inner);
    *slot = conn.unique_name().map(ToString::to_string);
    drop(slot);
    log::debug!("AppMenu: serving {MENU_OBJECT_PATH} on {:?}", conn.unique_name());
    Ok(conn)
}

/// Rebuilds the flat menu, diffs it against the exported one and emits only what changed.
fn publish(state: &Arc<Mutex<MenuState>>, conn: &Connection, model: &MenuModel) {
    let (structure, updated, revision) = {
        let mut locked = state.lock().unwrap_or_else(PoisonError::into_inner);
        // Ids stay stable across publishes (a click racing this rebuild still resolves the
        // item it was made on); a fresh numbering would reassign every item's id.
        let menu = FlatMenu::build_with_ids(model, &mut locked.ids);
        let structure = locked.menu.as_ref().is_none_or(|old| diff(old, &menu).structure);
        locked.revision = locked.revision.wrapping_add(1);
        // Property-only updates only matter when the shape didn't change: importers
        // re-fetch the whole layout on a structure change anyway.
        let updated = if structure { Vec::new() } else { locked.menu.as_ref().map_or_else(Vec::new, |old| diff(old, &menu).updated) };
        locked.menu = Some(menu);
        (structure, updated, locked.revision)
    };
    // The interface handlers lock the same mutex, so signals go out unlocked.
    if structure {
        log::debug!("AppMenu: layout change → revision {revision}");
        let _ = conn.emit_signal(Option::<String>::None, MENU_OBJECT_PATH, "com.canonical.dbusmenu", "LayoutUpdated", &(revision, ROOT_ID as i32));
    } else if !updated.is_empty() {
        let updated: Vec<(i32, HashMap<String, OwnedValue>)> = updated.into_iter().map(|(id, props)| (id as i32, wire_props(&props, &[]))).collect();
        let _ = conn.emit_signal(
            Option::<String>::None,
            MENU_OBJECT_PATH,
            "com.canonical.dbusmenu",
            "ItemsPropertiesUpdated",
            &(updated, Vec::<(i32, Vec<String>)>::new()),
        );
    }
}

/// The registration loop seam: where windows come from and whether the registrar still knows
/// them. Two small traits so the sync logic is testable with fakes — a real registrar can
/// restart, reset or drop entries at any time, and X11 doesn't exist on every box.
pub(crate) trait MenuWindows {
    /// This process's top-level X windows (empty on any error; retried next scan).
    fn windows(&mut self) -> Vec<u32>;
}

pub(crate) trait MenuRegistrar {
    /// Re-reads the registrar's entry for the window: `Ok(true)` = present and pointed at
    /// this app's menu; `Ok(false)` = missing (or someone else's) — the window must be
    /// (re-)registered; `Err` = the registrar is unreachable right now.
    fn is_menu_ours(&mut self, window: u32) -> Result<bool, Error>;
    fn register(&mut self, window: u32) -> Result<(), Error>;
    fn unregister(&mut self, window: u32);
    /// Forgets a sticky service choice the next register tries both names again.
    fn reset(&mut self);
}

/// Keeps the registrar in step with the app's windows: registers newly appeared ones,
/// unregisters closed ones, and — every scan — re-verifies the ones believed registered,
/// because registrars forget (observed live: the menu bar went silent mid-session while the
/// exporter's connection was healthy). Failures are never fatal: the next scan retries.
pub(crate) fn sync_registrations(windows: &mut dyn MenuWindows, registrar: &mut dyn MenuRegistrar, registered: &mut HashMap<u32, ()>) {
    let current = windows.windows();
    // Closed windows first, so a failing register below can't skip their cleanup.
    let gone: Vec<u32> = registered.keys().copied().filter(|w| !current.contains(w)).collect();
    for &window in &gone {
        registrar.unregister(window);
        let _ = registered.remove(&window);
    }
    for &window in &current {
        match registrar.is_menu_ours(window) {
            Ok(true) => {}
            // Missing, stale or the registrar is unreachable: retry the registration;
            // when that fails, drop all bookings so a returning registrar gets every
            // window again (re-registering is harmless per the spec).
            Ok(false) | Err(_) => {
                if registrar.register(window).is_err() {
                    registrar.reset();
                    registered.clear();
                    return;
                }
            }
        }
        let _ = registered.insert(window, ());
    }
}

/// The real seam adapters: windows by X11 scan, registrar over the live D-Bus connection.
pub(crate) struct X11Windows;

impl MenuWindows for X11Windows {
    fn windows(&mut self) -> Vec<u32> {
        x11::top_level_windows(std::process::id())
    }
}

pub(crate) struct RegistrarView<'a> {
    conn: &'a Connection,
    registrar: &'a mut Registrar,
}

impl MenuRegistrar for RegistrarView<'_> {
    fn is_menu_ours(&mut self, window: u32) -> Result<bool, Error> {
        self.registrar.is_menu_ours(self.conn, window)
    }
    fn register(&mut self, window: u32) -> Result<(), Error> {
        self.registrar.register(self.conn, window)
    }
    fn unregister(&mut self, window: u32) {
        self.registrar.unregister(self.conn, window)
    }
    fn reset(&mut self) {
        self.registrar.reset()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixed window list; real scanning is environment work.
    struct FakeWindows(Vec<u32>);

    impl MenuWindows for FakeWindows {
        fn windows(&mut self) -> Vec<u32> {
            self.0.clone()
        }
    }

    /// A registrar fake with a memory: entries, call counters and failure switches, so every
    /// behaviour the live loop must survive is reproducible without a desktop.
    struct FakeRegistrar {
        entries: HashMap<u32, bool>,
        registers: u32,
        unregisters: u32,
        queries: u32,
        fail_register: bool,
        fail_query: bool,
    }

    impl FakeRegistrar {
        fn new() -> FakeRegistrar {
            FakeRegistrar { entries: HashMap::new(), registers: 0, unregisters: 0, queries: 0, fail_register: false, fail_query: false }
        }
        fn forget(&mut self, window: u32) {
            let _ = self.entries.remove(&window);
        }
    }

    impl MenuRegistrar for FakeRegistrar {
        fn is_menu_ours(&mut self, window: u32) -> Result<bool, Error> {
            self.queries += 1;
            if self.fail_query {
                return Err(Error::Platform("registrar unresponsive".into()));
            }
            Ok(self.entries.get(&window).copied().unwrap_or(false))
        }
        fn register(&mut self, window: u32) -> Result<(), Error> {
            self.registers += 1;
            if self.fail_register {
                return Err(Error::Platform("no registrar".into()));
            }
            let _ = self.entries.insert(window, true);
            Ok(())
        }
        fn unregister(&mut self, window: u32) {
            self.unregisters += 1;
            let _ = self.entries.remove(&window);
        }
        fn reset(&mut self) {}
    }

    fn sync(windows: &[u32], registrar: &mut FakeRegistrar, registered: &mut HashMap<u32, ()>) {
        let mut fake_windows = FakeWindows(windows.to_vec());
        sync_registrations(&mut fake_windows, registrar, registered);
    }

    #[test]
    fn new_windows_register_once_and_verifications_stay_cheap() {
        let mut registrar = FakeRegistrar::new();
        let mut registered = HashMap::new();
        sync(&[0x11], &mut registrar, &mut registered);
        assert_eq!(registrar.registers, 1);
        assert_eq!(registrar.queries, 1); // one check even for the first booking
        sync(&[0x11], &mut registrar, &mut registered);
        sync(&[0x11], &mut registrar, &mut registered);
        // A registrar that keeps remembering is never re-registered.
        assert_eq!(registrar.registers, 1);
        assert_eq!(registrar.queries, 3);
        assert_eq!(registrar.unregisters, 0);
    }

    #[test]
    fn a_registrar_that_forgets_gets_the_window_again() {
        // The live failure: menus visible at launch, then the bar goes silent although the
        // exporter is healthy — because the registrar dropped its entry and the exporter
        // trusted a once-successful registration. The loop must notice and re-register.
        let mut registrar = FakeRegistrar::new();
        let mut registered = HashMap::new();
        sync(&[0x22], &mut registrar, &mut registered);
        registrar.forget(0x22);
        sync(&[0x22], &mut registrar, &mut registered);
        assert_eq!(registrar.registers, 2, "the dropped entry must be re-registered");
        assert_eq!(registrar.unregisters, 0);
    }

    #[test]
    fn closed_windows_are_unregistered_and_new_ones_are_taken() {
        let mut registrar = FakeRegistrar::new();
        let mut registered: HashMap<u32, ()> = HashMap::new();
        sync(&[0x31, 0x32], &mut registrar, &mut registered);
        assert_eq!(registrar.registers, 2);
        sync(&[0x31], &mut registrar, &mut registered);
        assert_eq!(registrar.unregisters, 1);
        sync(&[0x31, 0x33], &mut registrar, &mut registered);
        // The closed one is NOT re-registered: only the new window.
        assert_eq!(registrar.registers, 3);
        assert_eq!(registrar.unregisters, 1);
    }

    #[test]
    fn no_registrar_never_bookings_and_recovers_when_it_returns() {
        let mut registrar = FakeRegistrar { fail_register: true, ..FakeRegistrar::new() };
        let mut registered = HashMap::new();
        sync(&[0x44], &mut registrar, &mut registered);
        assert_eq!(registrar.registers, 1);
        assert!(registered.is_empty(), "a failed register leaves no booking");
        sync(&[0x44], &mut registrar, &mut registered);
        assert!(registered.is_empty(), "and keeps retrying without bookings");
        registrar.fail_register = false;
        sync(&[0x44], &mut registrar, &mut registered);
        assert_eq!(registrar.registers, 3, "the returning registrar gets the window");
        assert_eq!(registered.len(), 1);
    }

    #[test]
    fn an_unresponsive_query_still_reaches_the_register_fallback() {
        let mut registrar = FakeRegistrar { fail_query: true, ..FakeRegistrar::new() };
        let mut registered = HashMap::new();
        sync(&[0x55], &mut registrar, &mut registered);
        // The query errored, but the register underneath still happens; nothing is orphaned.
        assert_eq!(registrar.registers, 1);
        assert_eq!(registered.len(), 1);
    }

    #[test]
    fn a_failed_register_drops_all_bookings_for_a_later_registrar() {
        let mut registrar = FakeRegistrar::new();
        let mut registered = HashMap::new();
        sync(&[0x66], &mut registrar, &mut registered);
        assert_eq!(registered.len(), 1);
        registrar.fail_register = true;
        registrar.forget(0x66);
        sync(&[0x66], &mut registrar, &mut registered);
        // Register failed (fine, the registrar is being flaky): the stale booking must not
        // survive, or a returning registrar would never see the window again.
        assert!(registered.is_empty());
        registrar.fail_register = false;
    }

    /// The worker's own connection must carry the bounded method timeout: that constant is
    /// what makes a wedged registrar cost seconds instead of zbus's 25 s default per call
    /// (see Registrar's wedged test for the wait itself).
    #[test]
    fn the_worker_connection_bounds_its_calls() {
        let state = Arc::new(Mutex::new(MenuState::default()));
        let bus = Arc::new(Mutex::new(None));
        let conn = match connection(&state, &bus) {
            Ok(conn) => conn,
            Err(e) => {
                eprintln!("skipping (no usable session bus): {e}");
                return;
            }
        };
        assert_eq!(conn.method_timeout(), Some(REGISTRAR_CALL_TIMEOUT));
    }
}
