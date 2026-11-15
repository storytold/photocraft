//! Where Camera Raw's own window opens: the size, place and maximised state it was last left in,
//! remembered across opens and restarts in the preferences (`dialogs["filter.cameraRaw.window"]`,
//! beside the scope view's `filter.cameraRaw.scope`), as Adobe Camera Raw keeps its window.
//!
//! The window's geometry is read from its viewport every frame; a change is saved once it has
//! kept still for [`SETTLE_S`] (a resize by the window's edges moves it every frame with no button
//! down), or when the dialog closes. Opening, the saved geometry is fitted to the screen the main
//! window is on, so it never comes up off-screen or larger than the display (a monitor unplugged
//! since, a smaller display); the first open keeps the default size.

use egui::{Pos2, Rect, Vec2, pos2, vec2};
use serde::{Deserialize, Serialize};

/// The preferences key (`Preferences::dialogs`).
pub(super) const KEY: &str = "filter.cameraRaw.window";
/// The window's size the first time it opens, in points.
pub(super) const DEFAULT_SIZE: Vec2 = vec2(1360.0, 900.0);
/// The least the window can be resized to.
pub(super) const MIN_SIZE: Vec2 = vec2(900.0, 600.0);
/// A geometry change is saved once the window has kept it this long.
const SETTLE_S: f64 = 0.35;
/// A saved side beyond this is not a real window's (a corrupt preferences file).
const MAX_SIDE: f32 = 32_768.0;
/// How much of the title bar must be on the screen for a saved place to be kept.
const GRAB: Vec2 = vec2(120.0, 32.0);

/// The window's geometry: its content size, its outer top-left on the desktop (unknown on
/// Wayland), and whether it is maximised (then `size` and `pos` are the place it goes back to).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Placement {
    pub size: [f32; 2],
    #[serde(default)]
    pub pos: Option<[f32; 2]>,
    #[serde(default)]
    pub maximized: bool,
}

impl Placement {
    /// The saved placement, or `None` when there is none or it doesn't read as a window's.
    pub(super) fn read(saved: Option<&serde_json::Value>) -> Option<Self> {
        let p: Self = serde_json::from_value(saved?.clone()).ok()?;
        let side = |v: f32| v.is_finite() && (1.0..=MAX_SIDE).contains(&v);
        let coord = |v: f32| v.is_finite() && v.abs() <= MAX_SIDE * 4.0;
        (p.size.iter().all(|v| side(*v)) && p.pos.is_none_or(|[x, y]| coord(x) && coord(y))).then_some(p)
    }

    fn size(self) -> Vec2 {
        vec2(self.size[0], self.size[1])
    }

    /// The geometry `info` (the window's viewport) shows, keeping `last`'s normal size and place
    /// while the window is maximised. `None` until the window's size is known.
    pub(super) fn observe(info: &egui::ViewportInfo, last: Option<Self>) -> Option<Self> {
        if info.minimized == Some(true) || info.fullscreen == Some(true) {
            return last;
        }
        let maximized = info.maximized == Some(true);
        if maximized {
            // the size to go back to is the one before it was maximised
            return Some(match last {
                Some(l) => Self { maximized: true, ..l },
                None => Self { size: info.inner_rect?.size().into(), pos: info.outer_rect.map(|r| r.min.into()), maximized: true },
            });
        }
        let inner = info.inner_rect?;
        if !inner.is_finite() || inner.width() < 1.0 || inner.height() < 1.0 {
            return last;
        }
        Some(Self { size: inner.size().into(), pos: info.outer_rect.filter(|r| r.is_finite()).map(|r| r.min.into()), maximized: false })
    }
}

/// The screen the main window is on, less a taskbar-sized strip at its bottom: from `monitor`
/// (the main window's monitor size) and `main` (its outer rect on the desktop). egui knows the
/// monitor's size but not where it is, so monitors are taken to tile the desktop by their size
/// (as [`crate::work_area`] does). `None` when the monitor isn't known.
pub(super) fn screen(monitor: Option<Vec2>, main: Option<Rect>) -> Option<Rect> {
    let m = monitor.filter(|m| m.is_finite() && m.x >= 1.0 && m.y >= 1.0)?;
    let c = main.filter(|r| r.is_finite()).map_or(Pos2::ZERO, |r| r.center());
    let origin = pos2((c.x / m.x).floor() * m.x, (c.y / m.y).floor() * m.y);
    let usable = vec2(m.x, (m.y - crate::work_area::TASKBAR).max(m.y * 0.5));
    Some(Rect::from_min_size(origin, usable))
}

/// Where the window opens: `saved` fitted to `screen`. The size is at most the screen's (and
/// at least [`MIN_SIZE`] where the screen has room); a place whose title bar isn't on the screen is
/// dropped for the system's, and one that runs off the screen is moved back onto it.
pub(super) fn fit(saved: Option<Placement>, screen: Option<Rect>) -> Placement {
    let mut p = saved.unwrap_or(Placement { size: DEFAULT_SIZE.into(), pos: None, maximized: false });
    let Some(screen) = screen.filter(|s| s.is_positive()) else { return p };
    let size = p.size().max(MIN_SIZE.min(screen.size())).min(screen.size());
    p.size = size.into();
    p.pos = p.pos.and_then(|[x, y]| {
        let bar = Rect::from_min_size(pos2(x, y), vec2(size.x.min(GRAB.x * 2.0), GRAB.y));
        let shown = bar.intersect(screen);
        if !shown.is_positive() || shown.width() < GRAB.x.min(bar.width()) || shown.height() < GRAB.y * 0.5 {
            return None;
        }
        let x = x.min(screen.max.x - size.x).max(screen.min.x);
        let y = y.min(screen.max.y - size.y).max(screen.min.y);
        Some([x, y])
    });
    p
}

/// The builder for a window opening at `p`.
pub(super) fn apply(builder: egui::ViewportBuilder, p: Placement) -> egui::ViewportBuilder {
    let mut b = builder.with_inner_size(p.size).with_min_inner_size(MIN_SIZE.min(p.size()));
    if let Some(pos) = p.pos {
        b = b.with_position(pos);
    }
    if p.maximized {
        b = b.with_maximized(true);
    }
    b
}

/// What the dialog's window is tracked by between frames (kept in the context's memory, so it
/// outlives the dialog for the save at its close).
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Tracker {
    /// The placement the window was opened at (its builder must not change while it is open:
    /// egui would resize it to the builder's size).
    pub opened: Option<Placement>,
    /// The main window's frame the dialog's window was last drawn in.
    pub frame: u64,
    /// The geometry last seen, and when it was first seen.
    pub seen: Option<(Placement, f64)>,
}

impl Tracker {
    fn id() -> egui::Id {
        egui::Id::unique("camera-raw-window-place")
    }

    pub(super) fn load(ctx: &egui::Context) -> Self {
        ctx.data(|d| d.get_temp::<Self>(Self::id())).unwrap_or_default()
    }

    pub(super) fn store(self, ctx: &egui::Context) {
        ctx.data_mut(|d| d.insert_temp(Self::id(), self));
    }

    /// Takes in this frame's geometry `now` at time `t`; returns a placement to save (it kept
    /// still long enough and differs from `saved`). Asks `ctx` to look again when it may settle.
    pub(super) fn observe(&mut self, now: Option<Placement>, t: f64, saved: Option<Placement>) -> Option<Placement> {
        let now = now?;
        match self.seen {
            Some((p, _)) if p == now => {}
            _ => self.seen = Some((now, t)),
        }
        let (p, since) = self.seen?;
        (t - since >= SETTLE_S && Some(p) != saved).then_some(p)
    }

    /// The placement still to save once the window has closed.
    pub(super) fn unsaved(&self, saved: Option<Placement>) -> Option<Placement> {
        self.seen.map(|(p, _)| p).filter(|p| Some(*p) != saved)
    }

    pub(super) const SETTLE: std::time::Duration = std::time::Duration::from_millis(400);
}

/// The saved placement.
pub(super) fn saved(prefs: &photocraft_engine::prefs::Preferences) -> Option<Placement> {
    Placement::read(prefs.dialogs.get(KEY))
}

/// Saves `p`.
pub(super) fn save(app: &mut crate::PhotocraftApp, p: Placement) {
    if let Ok(v) = serde_json::to_value(p) {
        app.session.prefs.edit(|prefs| prefs.dialogs.insert(KEY.into(), v));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect { min: pos2(0.0, 0.0), max: pos2(1512.0, 934.0) };

    fn place(w: f32, h: f32, pos: Option<[f32; 2]>) -> Placement {
        Placement { size: [w, h], pos, maximized: false }
    }

    #[test]
    fn the_first_open_keeps_the_default_size() {
        let p = fit(None, Some(Rect::from_min_size(Pos2::ZERO, vec2(2560.0, 1392.0))));
        assert_eq!((p.size, p.pos, p.maximized), (DEFAULT_SIZE.into(), None, false));
        assert_eq!(fit(None, None).size, <[f32; 2]>::from(DEFAULT_SIZE));
    }

    #[test]
    fn a_saved_window_comes_back_where_it_was() {
        let p = place(1480.0, 900.0, Some([10.0, 25.0]));
        assert_eq!(fit(Some(p), Some(SCREEN)), p);
        let m = Placement { maximized: true, ..p };
        assert_eq!(fit(Some(m), Some(SCREEN)), m);
    }

    #[test]
    fn it_never_opens_larger_than_the_screen_or_off_it() {
        // saved on a big display, opened on a laptop's
        let p = fit(Some(place(2400.0, 1300.0, Some([300.0, 200.0]))), Some(SCREEN));
        assert_eq!(p.size, [1512.0, 934.0]);
        assert_eq!(p.pos, Some([0.0, 0.0]));
        // its right part runs off: moved back
        let p = fit(Some(place(1000.0, 700.0, Some([1200.0, 400.0]))), Some(SCREEN));
        assert_eq!(p.pos, Some([512.0, 234.0]));
        // on a monitor that is gone (title bar nowhere on this screen): the system places it
        let p = fit(Some(place(1000.0, 700.0, Some([3000.0, 100.0]))), Some(SCREEN));
        assert_eq!((p.size, p.pos), ([1000.0, 700.0], None));
        let p = fit(Some(place(1000.0, 700.0, Some([100.0, -900.0]))), Some(SCREEN));
        assert_eq!(p.pos, None);
    }

    #[test]
    fn a_tiny_saved_size_grows_to_the_minimum() {
        assert_eq!(fit(Some(place(200.0, 100.0, None)), Some(SCREEN)).size, <[f32; 2]>::from(MIN_SIZE));
        // unless the screen is smaller still
        let small = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 500.0));
        assert_eq!(fit(Some(place(200.0, 100.0, None)), Some(small)).size, [800.0, 500.0]);
    }

    #[test]
    fn a_corrupt_saved_placement_is_ignored() {
        for v in [
            serde_json::json!({"size": [0.0, 900.0]}),
            serde_json::json!({"size": [1e9, 900.0]}),
            serde_json::json!({"size": [1000.0, 900.0], "pos": [1e12, 0.0]}),
            serde_json::json!({"size": "big"}),
            serde_json::json!(42),
        ] {
            assert_eq!(Placement::read(Some(&v)), None, "{v}");
        }
        let ok = serde_json::json!({"size": [1000.0, 800.0]});
        assert_eq!(Placement::read(Some(&ok)), Some(place(1000.0, 800.0, None)));
    }

    #[test]
    fn the_screen_is_the_main_windows_monitor_less_the_taskbar() {
        let m = Some(vec2(1512.0, 982.0));
        assert_eq!(screen(m, Some(Rect::from_min_size(pos2(100.0, 50.0), vec2(800.0, 600.0)))), Some(Rect::from_min_size(Pos2::ZERO, vec2(1512.0, 934.0))));
        // the main window on a second monitor to the right
        let s = screen(m, Some(Rect::from_min_size(pos2(1700.0, 50.0), vec2(800.0, 600.0)))).unwrap();
        assert_eq!(s.min, pos2(1512.0, 0.0));
        assert_eq!(screen(None, None), None);
    }

    #[test]
    fn a_maximised_window_keeps_the_size_it_goes_back_to() {
        let normal = place(1200.0, 800.0, Some([40.0, 60.0]));
        let mut info = egui::ViewportInfo {
            maximized: Some(true),
            inner_rect: Some(Rect::from_min_size(pos2(0.0, 25.0), vec2(1512.0, 900.0))),
            outer_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1512.0, 925.0))),
            ..Default::default()
        };
        assert_eq!(Placement::observe(&info, Some(normal)), Some(Placement { maximized: true, ..normal }));
        info.maximized = Some(false);
        info.inner_rect = Some(Rect::from_min_size(pos2(40.0, 88.0), vec2(1300.0, 850.0)));
        info.outer_rect = Some(Rect::from_min_size(pos2(40.0, 60.0), vec2(1300.0, 878.0)));
        assert_eq!(Placement::observe(&info, Some(normal)), Some(place(1300.0, 850.0, Some([40.0, 60.0]))));
        info.minimized = Some(true);
        assert_eq!(Placement::observe(&info, Some(normal)), Some(normal), "a minimised window keeps its place");
    }

    #[test]
    fn a_resize_is_saved_once_it_settles() {
        let mut t = Tracker::default();
        let (a, b) = (place(1000.0, 700.0, None), place(1100.0, 760.0, None));
        assert_eq!(t.observe(Some(a), 0.0, None), None, "just seen");
        assert_eq!(t.observe(Some(b), 0.1, None), None, "still moving");
        assert_eq!(t.observe(Some(b), 0.3, None), None);
        assert_eq!(t.observe(Some(b), 0.5, None), Some(b), "kept still");
        assert_eq!(t.observe(Some(b), 0.9, Some(b)), None, "already saved");
        assert_eq!(t.unsaved(Some(b)), None);
        assert_eq!(t.observe(Some(a), 1.0, Some(b)), None);
        assert_eq!(t.unsaved(Some(b)), Some(a), "closed before it settled: saved at the close");
    }
}
