//! The connected monitors, for the dock's native floating windows (#13599).
//!
//! A floating panel's OS window stores the monitor it is on, and a restored workspace puts it
//! back there, or on the main window's monitor when that one is gone. Without a monitor list
//! craft-ui knows only the size of the main window's monitor, so every restored floating window
//! landed on it, whichever monitor it was left on.

use craft_ui::docking::Monitor;

/// A monitor as winit reports it: its desktop position and size in physical pixels, and the
/// scale of a window on it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Display {
    pub name: Option<String>,
    pub position: [i32; 2],
    pub size: [u32; 2],
    /// Physical pixels per logical pixel (winit's `scale_factor`).
    pub scale: f64,
}

/// `displays` in the points the dock's windows report: a window on a monitor reports its rect
/// in physical pixels over that window's pixels per point (egui-winit's `inner_rect_in_points`),
/// which is the UI zoom times the monitor's scale. Each monitor is therefore divided by its own
/// scale, so on mixed-DPI desktops a window and the monitor it is on agree. Displays with no
/// area or a nonsensical scale are left out.
pub(crate) fn dock_monitors(displays: impl IntoIterator<Item = Display>, zoom: f32) -> Vec<Monitor> {
    displays
        .into_iter()
        .filter_map(|display| {
            let pixels_per_point = zoom * display.scale as f32;
            let [x, y] = display.position.map(|v| v as f32);
            let [width, height] = display.size.map(|v| v as f32);
            let valid = pixels_per_point.is_finite() && pixels_per_point > 0.0 && width > 0.0 && height > 0.0;
            valid.then(|| Monitor {
                name: display.name,
                rect: [x / pixels_per_point, y / pixels_per_point, width / pixels_per_point, height / pixels_per_point],
                scale: pixels_per_point,
            })
        })
        .collect()
}

/// How often the monitors are read again, in seconds: monitors come and go rarely, and a window
/// left on one that is gone keeps its bar reachable meanwhile.
#[cfg(not(target_arch = "wasm32"))]
const REREAD: f64 = 1.0;

/// Read the connected monitors for the dock (`app.ui.docking_monitors`): on the first frame,
/// before any floating window is restored, then every [`REREAD`] seconds and whenever the UI zoom
/// changes. Without a winit window (tests) the list is left as it is.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn refresh(app: &mut crate::PhotocraftApp, ctx: &egui::Context, frame: &eframe::Frame) {
    let Some(window) = frame.winit_window() else { return };
    let key = egui::Id::new("photocraft-dock-monitors-read");
    let (now, zoom) = (ctx.input(|i| i.time), ctx.zoom_factor());
    let fresh = |(time, read_zoom): (f64, f32)| read_zoom == zoom && (time..time + REREAD).contains(&now);
    if ctx.data(|d| d.get_temp::<(f64, f32)>(key)).is_some_and(fresh) {
        return;
    }
    ctx.data_mut(|d| d.insert_temp(key, (now, zoom)));
    let displays = window.available_monitors().map(|monitor| {
        let (position, size) = (monitor.position(), monitor.size());
        Display { name: monitor.name(), position: [position.x, position.y], size: [size.width, size.height], scale: monitor.scale_factor() }
    });
    app.ui.docking_monitors = dock_monitors(displays, zoom);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A display at `position` with `size` in physical pixels.
    fn display(name: &str, position: [i32; 2], size: [u32; 2], scale: f64) -> Display {
        Display { name: Some(name.into()), position, size, scale }
    }

    #[test]
    fn monitors_are_in_the_points_of_a_window_on_them() {
        // A 150% monitor left of a 100% primary, and a 200% one above it: negative origins.
        let displays = [
            display("primary", [0, 0], [1920, 1080], 1.0),
            display("left", [-3840, 300], [3840, 2160], 1.5),
            display("above", [0, -2880], [3840, 2880], 2.0),
        ];
        let monitors = dock_monitors(displays, 1.0);
        let rects: Vec<_> = monitors.iter().map(|m| (m.name.as_deref().unwrap(), m.rect, m.scale)).collect();
        assert_eq!(
            rects,
            [
                ("primary", [0.0, 0.0, 1920.0, 1080.0], 1.0),
                ("left", [-2560.0, 200.0, 2560.0, 1440.0], 1.5),
                ("above", [0.0, -1440.0, 1920.0, 1440.0], 2.0),
            ]
        );
        // A window at (-3000, 600) px on the left monitor reports (-2000, 400) pt, inside it.
        let left = egui::Rect::from_min_size(egui::pos2(-2560.0, 200.0), egui::vec2(2560.0, 1440.0));
        assert!(left.contains(egui::pos2(-3000.0, 600.0) / 1.5));
    }

    #[test]
    fn the_ui_zoom_scales_every_monitor() {
        // Interface scaling at 125% on a 100% and a 200% monitor: windows draw at 1.25 and 2.5.
        let monitors = dock_monitors([display("a", [0, 0], [2560, 1440], 1.0), display("b", [2560, 0], [5120, 2880], 2.0)], 1.25);
        assert_eq!(monitors[0].rect, [0.0, 0.0, 2048.0, 1152.0]);
        assert_eq!(monitors[0].scale, 1.25);
        assert_eq!(monitors[1].rect, [1024.0, 0.0, 2048.0, 1152.0]);
        assert_eq!(monitors[1].scale, 2.5);
    }

    #[test]
    fn displays_without_an_area_or_a_scale_are_left_out() {
        let displays = [
            display("empty", [0, 0], [0, 1080], 1.0),
            display("unscaled", [0, 0], [1920, 1080], 0.0),
            display("nan", [0, 0], [1920, 1080], f64::NAN),
            Display { name: None, position: [1920, 0], size: [1280, 1024], scale: 1.0 },
        ];
        assert_eq!(dock_monitors(displays, 1.0), [Monitor { name: None, rect: [1920.0, 0.0, 1280.0, 1024.0], scale: 1.0 }]);
    }
}
