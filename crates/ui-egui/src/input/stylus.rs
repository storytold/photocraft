//! Stylus input: real pen pressure (and tilt / barrel rotation where the platform reports them)
//! for the painting tools. A mouse always paints at pressure 1 with no tilt.
//!
//! Sources, by platform (eframe 0.36.2, egui-winit 0.36.2, winit 0.30.13):
//!
//! - **Windows**: winit turns `WM_POINTER` pen and touch input into `WindowEvent::Touch` with a
//!   normalised force (`POINTER_PEN_INFO::pressure / 1024`); egui-winit forwards it as
//!   `egui::Event::Touch { force }` alongside the emulated pointer. The runtime's pen source reads
//!   it and gives each pointer move its [`PointerSample`]. winit drops the pen's tilt and
//!   rotation, so those stay 0, and reports pressure 0 (a tap lighter than the pen's first
//!   pressure level) as no force, which the pen source reads as pressure 0 once it knows a pen is
//!   in use. The desktop app's pointer message hook
//!   (`photocraft-tablet`) reports which end of the pen is in use through `Services::pen_end`.
//! - **Web**: eframe forwards touch force but not pen pointer events, so the web runner listens
//!   to `pointerdown`/`pointermove` itself and writes `pressure`, `tiltX`, `tiltY`, `twist` and
//!   the eraser button of `pointerType == "pen"` events into the [`StylusFeed`].
//! - **macOS**: winit 0.30 drops `NSEvent` tablet data, so the desktop app installs an AppKit
//!   local event monitor (the `photocraft-tablet` crate) that writes pressure, tilt, rotation and
//!   the eraser end into the [`StylusFeed`] before winit handles each event.
//! - **Linux X11**: the desktop app reads XInput2 raw valuator events on its own X connection
//!   (`photocraft-tablet`, x11rb) and writes them into the [`StylusFeed`].
//! - **Linux Wayland**: compositors give a pen only to clients that bind `zwp_tablet_v2`, which
//!   winit doesn't (it would have to share winit's connection), so with a pen attached the
//!   desktop app opens its window through Xwayland, which reports tablet valuators to the X11
//!   reader. `PHOTOCRAFT_NATIVE_WAYLAND=1` keeps native Wayland, where the pen has no input.
//!
//! Preferences › Tools › Use Tablet Pressure off makes a pen paint like a mouse. Flipping the pen
//! to its eraser end selects the Eraser tool and flipping back restores the previous tool, as in
//! Photoshop.
//!
//! Automation simulates a pen with `ui.pointer` events carrying `pressure`, `tiltX`, `tiltY`
//! and `rotation`.

use craft::behavior::PointerSample;
use craft::ui::runtime::pen::PenSource;

/// One stylus reading. Tilt is in degrees (-90..90, W3C Pointer Events convention), rotation in
/// degrees 0..360.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PenSample {
    pub pressure: f32,
    pub tilt_x: f32,
    pub tilt_y: f32,
    pub rotation: f32,
    /// The pen's eraser end is in use.
    pub eraser: bool,
}

impl Default for PenSample {
    fn default() -> Self {
        Self { pressure: 1.0, tilt_x: 0.0, tilt_y: 0.0, rotation: 0.0, eraser: false }
    }
}

impl PenSample {
    /// Clamp to the documented ranges (and replace non-finite values).
    pub fn sanitized(self) -> Self {
        let f = |v: f32, lo: f32, hi: f32, d: f32| if v.is_finite() { v.clamp(lo, hi) } else { d };
        Self {
            pressure: f(self.pressure, 0.0, 1.0, 1.0),
            tilt_x: f(self.tilt_x, -90.0, 90.0, 0.0),
            tilt_y: f(self.tilt_y, -90.0, 90.0, 0.0),
            // `rem_euclid` rounds tiny negative angles up to exactly 360.
            rotation: if self.rotation.is_finite() { self.rotation.rem_euclid(360.0) % 360.0 } else { 0.0 },
            eraser: self.eraser,
        }
    }

    /// The reading as the runtime's pointer feed carries it (radians).
    fn pointer(self) -> PointerSample {
        let mut p = PointerSample::new(self.pressure);
        p.tilt = [self.tilt_x.to_radians(), self.tilt_y.to_radians()];
        p.rotation = self.rotation.to_radians();
        p.eraser = self.eraser;
        p
    }

    /// The runtime's reading `p` in degrees, in the documented ranges.
    fn of_pointer(p: PointerSample) -> Self {
        let p = p.sanitized();
        PenSample { pressure: p.pressure, tilt_x: p.tilt[0].to_degrees(), tilt_y: p.tilt[1].to_degrees(), rotation: p.rotation.to_degrees(), eraser: p.eraser }
            .sanitized()
    }
}

/// The pen readings a platform backend writes: the runtime's pen source (`InstallSpec::pen`), which gives each pointer
/// move the canvas takes its own reading. Cloning shares it.
#[derive(Clone, Default)]
pub struct StylusFeed(PenSource);

impl std::fmt::Debug for StylusFeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("StylusFeed").field(&self.get()).finish()
    }
}

impl StylusFeed {
    /// The feed installed with the runtime on `ctx` (`app::framework::install`); a feed of its own, which reaches no
    /// pointer, on a context without one.
    pub(crate) fn of(ctx: &egui::Context) -> Self {
        ctx.data(|d| d.get_temp::<StylusFeed>(feed_id())).unwrap_or_default()
    }

    /// Keeps `self` as the feed of `ctx`, the one the runtime was installed with.
    pub(crate) fn keep_on(&self, ctx: &egui::Context) {
        ctx.data_mut(|d| d.insert_temp(feed_id(), self.clone()));
    }

    /// The runtime's pen source behind the feed.
    pub(crate) fn source(&self) -> PenSource {
        self.0.clone()
    }

    /// Queues one reading in order; `None` = the pen left the tablet (a mouse).
    pub fn set(&self, s: Option<PenSample>) {
        self.0.set(s.map(|s| s.sanitized().pointer()));
    }

    /// The current reading (`None` = no pen down).
    pub fn get(&self) -> Option<PenSample> {
        self.0.current().map(PenSample::of_pointer)
    }
}

fn feed_id() -> egui::Id {
    egui::Id::unique("photocraft::stylus::feed")
}

/// Per-app stylus state.
#[derive(Clone, Debug)]
pub struct Stylus {
    /// Pen samples pushed by the platform (desktop tablet monitor, web runner) or by automation.
    pub feed: StylusFeed,
    /// Preferences › Tools › Use Tablet Pressure: off, a pen paints like a mouse.
    pub use_pressure: bool,
    /// Preferences › Tools › Pressure Curve (`None`: linear), with the points it was built from.
    pressure_curve: Option<photocraft_engine::prefs::PressureCurve>,
    curve_points: Vec<[f32; 2]>,
    /// The pen end last seen (`Some(true)` = eraser), for the eraser tool switch.
    end: Option<bool>,
    /// The tool to restore when the pen tip comes back after the eraser end switched tools.
    pub(crate) tool_before_eraser: Option<crate::state::Tool>,
    /// The pen reading of the last pointer the tool host delivered to the paint tool
    /// ([`Self::record_pointer`]); `None` for a pointer with no sample.
    pose: Option<PenSample>,
    /// The time of that pointer, in milliseconds.
    time_ms: f64,
    /// The highest pen pressure (before the curve) since the drag began, for a tap's one dab.
    peak: f32,
    /// The pen's touch contacts in the raw input, so a held pen is no press-and-hold right click.
    pub(crate) pen_contacts: PenContacts,
    /// Tilt X, tilt Y, rotation of each point of the current drag (parallel to its points).
    pub(crate) stroke: Vec<[f32; 3]>,
    /// Time in milliseconds of each point of the current drag (parallel to its points), from the
    /// pointers' times. Speed spacing, airbrush build-up and smoothing catch-up use it.
    pub(crate) times: Vec<f64>,
}

impl Default for Stylus {
    fn default() -> Self {
        Self::over(StylusFeed::default())
    }
}

impl Stylus {
    /// The stylus state of an app whose platform writes the pen into `feed`.
    pub(crate) fn over(feed: StylusFeed) -> Self {
        Self {
            feed,
            use_pressure: true,
            pressure_curve: None,
            curve_points: Vec::new(),
            end: None,
            tool_before_eraser: None,
            pose: None,
            time_ms: 0.0,
            peak: 0.0,
            pen_contacts: PenContacts::default(),
            stroke: Vec::new(),
            times: Vec::new(),
        }
    }
}

impl Stylus {
    /// Records the pointer the paint tool is handling: its time in milliseconds and its pen
    /// reading (the runtime's pen source gives each move its own, interpolated). `None` is a
    /// pointer with no reading. The pose is state of one pointer, so a second pass of a frame
    /// that records the same pointer again changes nothing.
    pub fn record_pointer(&mut self, time_ms: f64, sample: Option<PointerSample>) {
        self.time_ms = time_ms;
        self.pose = sample.map(PenSample::of_pointer);
        if let Some(p) = self.pose {
            self.peak = self.peak.max(p.pressure);
        }
    }

    /// The current pen sample, `None` for a mouse (and for any pen while Use Tablet Pressure is
    /// off).
    pub fn sample(&self) -> Option<PenSample> {
        if !self.use_pressure {
            return None;
        }
        let s = self.pose?;
        // The pen's pressure through Preferences › Tools › Pressure Curve, before any brush sees it.
        Some(match &self.pressure_curve {
            Some(c) => PenSample { pressure: c.eval(s.pressure), ..s },
            None => s,
        })
    }

    /// Preferences › Tools › Pressure Curve (rebuilt only when the points change).
    pub fn set_pressure_curve(&mut self, points: &[[f32; 2]]) {
        if self.curve_points != points {
            self.curve_points = points.to_vec();
            let c = photocraft_engine::prefs::PressureCurve::new(points);
            self.pressure_curve = (!c.is_linear()).then_some(c);
        }
    }

    /// Did the pen just flip to its eraser end (`Some(true)`) or back to its tip (`Some(false)`)?
    /// Each flip is reported once; a mouse in between changes nothing. `platform_end` is the end
    /// a platform reports outside the feed (Windows), read when the feed has no pen.
    pub(crate) fn take_end_flip(&mut self, platform_end: Option<bool>) -> Option<bool> {
        let eraser = self.feed.get().map(|s| s.eraser).or(platform_end)?;
        let flipped = self.end.map_or(eraser, |e| e != eraser);
        self.end = Some(eraser);
        flipped.then_some(eraser)
    }

    /// Switch to the Eraser when the pen's eraser end comes in, and back to the previous tool when
    /// the tip does (Photoshop). Not during a drag. Returns whether the tool changed.
    pub fn sync_eraser_tool(app: &mut crate::PhotocraftApp) -> bool {
        use crate::state::Tool;
        // PLAN: `in_gesture(app)` is cw.P1's (canvas-widgets-w14, `canvas/tools/mod.rs`); the module path is theirs to make visible.
        if crate::canvas::tools::in_gesture(app) {
            return false;
        }
        let platform_end = app.services.pen_end.as_ref().and_then(|f| f());
        match app.stylus.take_end_flip(platform_end) {
            Some(true) if app.ui.tool != Tool::Eraser => {
                app.stylus.tool_before_eraser = Some(app.ui.tool);
                app.ui.tool = Tool::Eraser;
                // A tool switch like any other: Add New Artboard's placing mode ends (#11386).
                crate::tool_memory::ended_by_tool(app);
                true
            }
            Some(false) => match app.stylus.tool_before_eraser.take() {
                Some(t) if app.ui.tool == Tool::Eraser => {
                    app.ui.tool = t;
                    true
                }
                _ => false,
            },
            _ => false,
        }
    }

    /// Pressure for the next tool event (1 for a mouse).
    pub fn pressure(&self) -> f32 {
        self.sample().map_or(1.0, |s| s.pressure)
    }

    /// Pressure for a tap, a drag released where it was pressed: the highest the pen pressed
    /// since the drag began (through the pressure curve), as the first sample of a contact is
    /// often far lighter than the tap. 1 for a mouse.
    pub(crate) fn tap_pressure(&self) -> f32 {
        let Some(s) = self.sample() else { return 1.0 };
        let peak = match &self.pressure_curve {
            Some(c) => c.eval(self.peak),
            None => self.peak,
        };
        if self.peak > 0.0 { peak.max(s.pressure) } else { s.pressure }
    }

    /// Start recording a drag's tilt/rotation, from the press's pointer ([`Self::record_pointer`]
    /// is called for it first). The tap peak restarts at that pointer's pressure, so a new stroke
    /// never starts from an earlier stroke's.
    pub(crate) fn begin_stroke(&mut self) {
        self.peak = self.pose.map_or(0.0, |p| p.pressure);
        self.stroke.clear();
        self.times.clear();
        self.record_point();
    }

    /// Record the tilt/rotation and time of a point just added to the drag.
    pub(crate) fn record_point(&mut self) {
        let s = self.sample().unwrap_or_default();
        self.stroke.push([s.tilt_x, s.tilt_y, s.rotation]);
        self.times.push(self.time_ms);
    }

    /// Record a point the held pen repeats at `time_ms` (the pose stays the last pointer's).
    pub(crate) fn repeat_point(&mut self, time_ms: f64) {
        self.time_ms = time_ms;
        self.record_point();
    }

    /// Time of the drag's last recorded point.
    pub(crate) fn last_point_ms(&self) -> Option<f64> {
        self.times.last().copied()
    }

    /// Time of drag point `i` (the last known time past the end).
    pub(crate) fn point_ms(&self, i: usize) -> f64 {
        self.times.get(i).or(self.times.last()).copied().unwrap_or(0.0)
    }

    /// Stroke points for `paint.stroke`: `[x, y, pressure]`, extended with
    /// `tiltX, tiltY, rotation` when the drag carried any tilt or rotation, and with `timeMs`
    /// (from the drag's first point) when its points carry times.
    pub fn stroke_points(&self, points: &[[f64; 3]]) -> Vec<Vec<f64>> {
        let has_pose = self.stroke.iter().any(|t| t.iter().any(|v| *v != 0.0));
        let t0 = self.point_ms(0);
        let has_time = self.times.iter().any(|t| *t != t0);
        points
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let mut v = vec![p[0], p[1], p[2]];
                if has_pose || has_time {
                    let t = self.stroke.get(i).or(self.stroke.last()).copied().unwrap_or_default();
                    v.extend(t.map(f64::from));
                }
                if has_time {
                    v.push(self.point_ms(i) - t0);
                }
                v
            })
            .collect()
    }
}

/// The touch contacts a pen makes, read from the raw input. egui takes every touch contact held
/// still past `max_click_duration` (0.8 s) for a touch screen's press-and-hold: a secondary click
/// (a context menu) that also drops the drag. On Windows a pen arrives as touch events, so a pen
/// held still opened a context menu. A pen held still is just a pen, as in Photoshop.
#[derive(Clone, Debug, Default)]
pub(crate) struct PenContacts {
    /// Touch ids that moved without touching: only a pen hovers.
    hovering: Vec<egui::TouchId>,
    /// The contacts down, and whether each is a pen.
    down: Vec<(egui::TouchId, bool)>,
}

/// Most touch ids [`PenContacts`] remembers (the oldest go first).
const CONTACTS: usize = 16;

impl PenContacts {
    /// Follow a frame's touch events. True when a pen contact was down at any point of the frame.
    pub(crate) fn observe(&mut self, events: &[egui::Event]) -> bool {
        let pressed = |f: &Option<f32>| f.is_some_and(|f| f > 0.0);
        let mut pen = self.down.iter().any(|(_, p)| *p);
        for e in events {
            let egui::Event::Touch { id, phase, force, .. } = e else { continue };
            match phase {
                egui::TouchPhase::Start => {
                    let is_pen = self.hovering.contains(id) || pressed(force);
                    self.down.retain(|(c, _)| c != id);
                    if self.down.len() >= CONTACTS {
                        self.down.remove(0);
                    }
                    self.down.push((*id, is_pen));
                    pen |= is_pen;
                }
                egui::TouchPhase::Move => match self.down.iter_mut().find(|(c, _)| c == id) {
                    Some(c) if pressed(force) => {
                        c.1 = true;
                        pen = true;
                    }
                    Some(_) => {}
                    None if !self.hovering.contains(id) => {
                        if self.hovering.len() >= CONTACTS {
                            self.hovering.remove(0);
                        }
                        self.hovering.push(*id);
                    }
                    None => {}
                },
                egui::TouchPhase::End | egui::TouchPhase::Cancel => self.down.retain(|(c, _)| c != id),
            }
        }
        pen
    }
}

/// Turn egui's touch press-and-hold off while a pen contact is down (and through the frame it
/// lifts in, so the release is the click a quick tap would be), back on otherwise for fingers.
pub(crate) fn pen_hold_is_no_right_click(ctx: &egui::Context, pen_down: bool) {
    let want = if pen_down { f64::INFINITY } else { egui::InputOptions::default().max_click_duration };
    if ctx.options(|o| o.input_options.max_click_duration) != want {
        ctx.options_mut(|o| o.input_options.max_click_duration = want);
    }
}

#[cfg(test)]
mod tests;
