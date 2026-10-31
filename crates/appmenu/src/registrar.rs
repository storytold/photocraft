//! A client for the AppMenu registrar: `com.canonical.AppMenu.Registrar`, with the Ayatana
//! fork (`org.ayatana.AppMenu.Registrar`) as fallback on its own bus name. Both live at the
//! same object path and serve the same `RegisterWindow`/`UnregisterWindow` methods.
//!
//! Environments without a global-menu registrar will always fail these calls; the worker
//! retries lazily, so plugging in a menu shell later picks the menu up without a restart.

use crate::Error;
use crate::dbus::MENU_OBJECT_PATH;
use zbus::blocking::Connection;

/// Bus names in preference order: canonical first (the name almost every shell matches).
pub const SERVICES: [&str; 2] = ["com.canonical.AppMenu.Registrar", "org.ayatana.AppMenu.Registrar"];
pub const REGISTRAR_PATH: &str = "/com/canonical/AppMenu/Registrar";
pub const REGISTRAR_INTERFACE: &str = "com.canonical.AppMenu.Registrar";

/// Which registrar service is being used (sticky: the first one that answers) plus the app's
/// own unique bus name, so a registration can be verified, not assumed: registrars restart,
/// reset and (as seen live) drop entries — a once-successful `RegisterWindow` proves nothing
/// about now. `is_menu_ours` re-checks; the worker re-registers whenever the answer is "no".
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Registrar {
    working: Option<&'static str>,
    /// This exporter's unique bus name ("" when unknown: nothing then counts as ours).
    app: String,
}

impl Registrar {
    /// Creates a verifier for the exporter running at `app_service` (its unique bus name).
    pub fn new(app_service: &str) -> Registrar {
        Registrar { working: None, app: app_service.to_string() }
    }

    /// Whether some registrar has answered before.
    pub fn is_connected(&self) -> bool {
        self.working.is_some()
    }

    /// Whether the registrar's entry for `window_id` exists and still points at this app's
    /// Whether the registrar's entry for `window_id` exists and still points at this app's
    /// menu. `Ok(false)` covers both "no entry" and "someone else's entry" — both mean
    /// re-register. `Err` means the registrar is unreachable or unresponsive right now.
    /// Unresponsive is *bounded*, not the zbus 25 s default: the worker builds its
    /// connection with a 2 s method timeout, so a wedged registrar costs at most one
    /// bounded wait per call and the scan (and with it publishes, activations and
    /// `hosted()`) keeps turning over.
    pub fn is_menu_ours(&self, conn: &Connection, window_id: u32) -> Result<bool, Error> {
        if !self.is_connected() {
            return Ok(false);
        }
        let Some(service) = self.working else { return Ok(false) };
        let proxy = zbus::blocking::Proxy::new(conn, service, REGISTRAR_PATH, REGISTRAR_INTERFACE).map_err(|e| Error::Platform(e.to_string()))?;
        let (bus, path): (String, zvariant::OwnedObjectPath) = proxy.call("GetMenuForWindow", &(window_id,)).map_err(|e| Error::Platform(e.to_string()))?;
        // Registrars answer `("", "/")` for windows they know nothing about.
        Ok(!bus.is_empty() && bus == self.app && path.as_str() == MENU_OBJECT_PATH)
    }

    /// Forgets the sticky service choice: the next `register` tries both names again. Called
    /// after a failed call, so a temporarily gone registrar picks its (possibly new) place
    /// up when it returns.
    pub fn reset(&mut self) {
        self.working = None;
    }

    /// Registers one window with the first registrar that accepts it. Errors mean "no
    /// (usable) registrar" — the caller retries later; nothing is ever fatal here.
    pub fn register(&mut self, conn: &Connection, window_id: u32) -> Result<(), Error> {
        let mut last = String::new();
        for &service in SERVICES.iter() {
            // Skip a service we've already seen failing when another one works.
            if self.working.is_some_and(|w| w != service) {
                continue;
            }
            match call(conn, service, "RegisterWindow", &(window_id, MENU_OBJECT_PATH)) {
                Ok(()) => {
                    self.working = Some(service);
                    return Ok(());
                }
                Err(e) => {
                    log::debug!("AppMenu: {service}: Window {window_id} register failed: {e}");
                    last = e.to_string();
                    self.working = None;
                }
            }
        }
        Err(Error::Platform(last))
    }

    /// Best effort unregistration: only used on shutdown and window removal, so errors are
    /// logged, not propagated.
    pub fn unregister(&mut self, conn: &Connection, window_id: u32) {
        let Some(service) = self.working else { return };
        if let Err(e) = call::<(u32,)>(conn, service, "UnregisterWindow", &(window_id,)) {
            log::debug!("AppMenu: {service}: Window {window_id} unregister failed: {e}");
            self.working = None;
        }
    }
}

/// Calls a registrar method; used with RegisterWindow/UnregisterWindow above.
fn call<B>(conn: &Connection, service: &str, method: &'static str, body: &B) -> Result<(), Error>
where
    B: serde::ser::Serialize + zvariant::DynamicType,
{
    let proxy = zbus::blocking::Proxy::new(conn, service, REGISTRAR_PATH, REGISTRAR_INTERFACE).map_err(|e| Error::Platform(e.to_string()))?;
    proxy.call(method, body).map_err(|e| Error::Platform(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    use zbus::blocking::connection::Builder;

    /// A registrar that takes the call and never replies — the wedged-host case the
    /// connection deadline exists for.
    #[derive(Default)]
    struct Wedged;

    #[zbus::interface(name = "com.canonical.AppMenu.Registrar")]
    impl Wedged {
        async fn get_menu_for_window(&self, _id: u32) -> (String, zbus::zvariant::OwnedObjectPath) {
            let () = futures_lite::future::pending().await;
            // Unreachable while the future above pends; the type needs a value.
            (String::new(), zbus::zvariant::OwnedObjectPath::default())
        }
    }

    /// The regression: `is_menu_ours` against a wedged registrar must surface `Err` at the
    /// connection's method timeout, not zbus's 25 s default (which stalled a whole scan per
    /// window — publishes queued, activations and `hosted()` went stale). A 400 ms bound
    /// rides through the same code path the 2 s constant takes.
    #[allow(clippy::expect_used)] // test-only unwraps of infallible-by-construction setup are loud
    #[test]
    fn a_wedged_registrar_cannot_stall_verification() {
        let server = match Builder::session().and_then(|b| b.serve_at(REGISTRAR_PATH, Wedged)).and_then(|b| b.build()) {
            Ok(conn) => conn,
            Err(e) => {
                eprintln!("skipping (no usable session bus): {e}");
                return;
            }
        };
        let Some(service) = server.unique_name() else {
            eprintln!("skipping (server never got a bus name)");
            return;
        };
        let bounded = match Builder::session().and_then(|b| b.method_timeout(std::time::Duration::from_millis(400)).build()) {
            Ok(conn) => conn,
            Err(e) => {
                eprintln!("skipping (no usable session bus): {e}");
                return;
            }
        };
        assert_eq!(bounded.method_timeout(), Some(std::time::Duration::from_millis(400)),);

        let mut registrar = Registrar::new("craft-appmenu-test");
        // Pin the sticky service at the wedged fake directly (its unique name works as a
        // destination), so the test exercises only the bounded wait.
        registrar.working = Some(Box::leak(service.to_string().into_boxed_str()));
        let start = Instant::now();
        let result = registrar.is_menu_ours(&bounded, 42);
        let elapsed = start.elapsed();

        assert!(result.is_err(), "wedged registrar must surface as unreachable: {result:?}");
        assert!(elapsed < Duration::from_secs(10), "took {elapsed:?}: the connection deadline is not applied (the unbounded default is 25 s)");
    }
}
