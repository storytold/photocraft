//! The window system PhotoCraft installs in the runtime: the platform's own ([`SystemWindows`]),
//! with the connected monitors listed (#13599).
//!
//! The runtime records the monitor each OS window is on and fits a saved window to the monitors
//! connected when it opens again: a native dock window left on a monitor that is still there
//! opens where it was left, one left on a monitor that is gone opens on the main window's monitor,
//! wholly on it. `SystemWindows` lists no monitors, and without them the runtime keeps every saved
//! position as it is, also one on a monitor that is no longer connected, where the window opens
//! out of reach. winit lists them; the app reads them from its main window ([`refresh`]) into the
//! list this window system reports.

use std::sync::{Arc, Mutex, PoisonError};

use craft::behavior::monitor::{Monitor, MonitorKey};
use craft::window::system::{SystemWindows, WindowSystem};

/// [`SystemWindows`] with the monitors the app last read.
#[derive(Debug, Default)]
pub(crate) struct DesktopWindows {
    monitors: Mutex<Vec<Monitor>>,
}

impl DesktopWindows {
    /// Replaces the listed monitors.
    pub(crate) fn set_monitors(&self, monitors: Vec<Monitor>) {
        *self.monitors.lock().unwrap_or_else(PoisonError::into_inner) = monitors;
    }
}

impl WindowSystem for DesktopWindows {
    fn desktop_pointer(&self) -> Option<[f32; 2]> {
        SystemWindows.desktop_pointer()
    }

    fn primary_held(&self) -> Option<bool> {
        SystemWindows.primary_held()
    }

    fn command_held(&self) -> Option<bool> {
        SystemWindows.command_held()
    }

    fn warp_pointer(&self, to: [f32; 2]) -> bool {
        SystemWindows.warp_pointer(to)
    }

    fn monitors(&self) -> Vec<Monitor> {
        self.monitors.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

/// The context's key to the window system installed on it.
fn key() -> egui::Id {
    egui::Id::unique("photocraft-desktop-windows")
}

/// The window system for the runtime installed on `ctx`, made on the first call; [`of`] finds it
/// again. A second install on the same context (which the runtime refuses) gets the same one.
pub(crate) fn for_context(ctx: &egui::Context) -> Arc<DesktopWindows> {
    ctx.data_mut(|data| data.get_temp_mut_or_insert_with(key(), || Arc::new(DesktopWindows::default())).clone())
}

/// The window system installed on `ctx`, once the runtime is.
pub(crate) fn of(ctx: &egui::Context) -> Option<Arc<DesktopWindows>> {
    ctx.data(|data| data.get_temp::<Arc<DesktopWindows>>(key()))
}

/// A monitor as winit reports it (`MonitorHandle`): its name, desktop position and size in
/// physical pixels, and scale factor; the runtime's monitors are in the same pixels. `None` for
/// one with no area or no usable scale.
pub(crate) fn monitor(name: Option<String>, position: [i32; 2], size: [u32; 2], scale: f64) -> Option<Monitor> {
    let scale = scale as f32;
    let usable = size.iter().all(|v| *v > 0) && scale.is_finite() && scale > 0.0;
    usable.then(|| Monitor::new(MonitorKey::new(name.unwrap_or_default(), size), position, size, scale))
}

/// How often the monitors are read again, in seconds. Monitors come and go rarely; a saved window
/// is fitted to them when it opens.
#[cfg(not(target_arch = "wasm32"))]
const REREAD: f64 = 1.0;

/// Reads the connected monitors from the app's main window into the installed window system: in
/// the first frame, before the dock opens a saved floating window, then every [`REREAD`] seconds.
/// Without a winit window (the test harness) the list stays as it is.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn refresh(ctx: &egui::Context, frame: &eframe::Frame) {
    let (Some(window), Some(windows)) = (frame.winit_window(), of(ctx)) else { return };
    let read = egui::Id::unique("photocraft-desktop-windows-read");
    let now = ctx.input(|input| input.time);
    if ctx.data(|data| data.get_temp::<f64>(read)).is_some_and(|at| (at..at + REREAD).contains(&now)) {
        return;
    }
    ctx.data_mut(|data| data.insert_temp(read, now));
    let monitors = window
        .available_monitors()
        .filter_map(|handle| {
            let (position, size) = (handle.position(), handle.size());
            monitor(handle.name(), [position.x, position.y], [size.width, size.height], handle.scale_factor())
        })
        .collect();
    windows.set_monitors(monitors);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_monitor_keeps_winits_pixels_name_and_scale() {
        // A 150% monitor left of and above the primary: a negative origin, in pixels, not points.
        let left = monitor(Some(r"\\.\DISPLAY2".into()), [-3840, -300], [3840, 2160], 1.5).unwrap();
        assert_eq!((left.pos_px, left.size_px, left.scale), ([-3840, -300], [3840, 2160], 1.5));
        assert_eq!(left.key, MonitorKey::new(r"\\.\DISPLAY2", [3840, 2160]));
    }

    #[test]
    fn a_monitor_without_an_area_or_a_scale_is_left_out() {
        assert!(monitor(None, [0, 0], [0, 1080], 1.0).is_none());
        assert!(monitor(None, [0, 0], [1920, 1080], 0.0).is_none());
        assert!(monitor(None, [0, 0], [1920, 1080], f64::NAN).is_none());
        assert_eq!(monitor(None, [1920, 0], [1280, 1024], 1.0).map(|m| m.key.name), Some(String::new()));
    }

    #[test]
    fn the_installed_system_lists_the_monitors_set_on_it() {
        let ctx = egui::Context::default();
        let installed: Arc<dyn WindowSystem> = for_context(&ctx);
        assert!(installed.monitors().is_empty());
        let main = monitor(Some("main".into()), [0, 0], [1920, 1080], 1.0).unwrap();
        of(&ctx).unwrap().set_monitors(vec![main.clone()]);
        assert_eq!(installed.monitors(), [main]);
    }
}
