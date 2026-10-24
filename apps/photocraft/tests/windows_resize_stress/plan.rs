//! The deterministic core of the resize stress test: where the cursor travels, what a soak run does for a
//! given seed, and how a run is judged. Plain data and arithmetic, so it is unit-tested on every platform;
//! `win.rs` and `main.rs` only drive Windows with it.
//!
//! Every size and position here is in physical pixels: the test process is DPI aware, so the display's size,
//! window rectangles and cursor positions agree at any display scale.

/// Where the test parks the window before a drag (its top-left, px): clear of the screen's corner, so the
/// left and top borders can be grabbed too.
pub const PARK: (i32, i32) = (80, 40);
/// Space kept free between a parked window and the screen's right and bottom edges.
pub const MARGIN: i32 = 60;
/// px the cursor moves after the press, so that egui starts the drag and asks the OS to resize.
pub const NUDGE_PX: i32 = 8;
/// How close to the edge of the display the cursor may be sent. The OS pins the pointer at the edge, and an
/// injected move lands a pixel off, so a drag that went further would look like the mouse leaving its path.
pub const EDGE_GUARD_PX: i32 = 3;
/// The least a sweep may travel and still stress anything. A display with less room than this fails the
/// test with a message, rather than shrinking the sweep to nothing and passing.
pub const MIN_TRAVEL_PX: i32 = 60;
/// How far inside a window's right or bottom edge the grab point is: on the window, not the desktop behind it.
const GRAB_INSET_PX: i32 = 4;
/// How far inside a window's left edge the grab point is.
const LEFT_GRAB_INSET_PX: i32 = 3;
/// The shortest outer height a window is given: the app's minimum inner height (480 logical px), at 100 %.
const MIN_WINDOW_HEIGHT: i32 = 480;
/// The window size the scenarios that do not size it themselves start from, before it is fitted to the display.
pub const DEFAULT_SIZE: (i32, i32) = (1100, 700);
/// Seconds between `ui.inspect` probes while the window is being resized.
pub const PROBE_INTERVAL_S: f64 = 0.25;

/// SplitMix64: a small seeded generator, so a soak run is reproducible from its seed without a new dependency.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A value in `lo..=hi` (`lo` when the range is empty).
    pub fn range(&mut self, lo: i32, hi: i32) -> i32 {
        let span = u64::try_from(i64::from(hi) - i64::from(lo) + 1).unwrap_or(1).max(1);
        lo.saturating_add(i32::try_from(self.next_u64() % span).unwrap_or(0))
    }

    /// A value in `lo..hi`.
    pub fn float(&mut self, lo: f64, hi: f64) -> f64 {
        let unit = (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64;
        lo + (hi - lo) * unit
    }

    /// An item chosen uniformly, or `None` for an empty slice.
    pub fn pick<T: Copy>(&mut self, items: &[T]) -> Option<T> {
        let n = u64::try_from(items.len()).ok().filter(|n| *n > 0)?;
        items.get(usize::try_from(self.next_u64() % n).ok()?).copied()
    }

    /// An item with probability proportional to its weight.
    pub fn weighted<T: Copy>(&mut self, items: &[(T, u32)]) -> Option<T> {
        let total: u64 = items.iter().map(|(_, w)| u64::from(*w)).sum();
        if total == 0 {
            return None;
        }
        let mut at = self.next_u64() % total;
        for (item, weight) in items {
            let weight = u64::from(*weight);
            if at < weight {
                return Some(*item);
            }
            at -= weight;
        }
        None
    }
}

/// A triangle wave in `0.0..=1.0` with period 1: 0 at phase 0, 1 at phase 0.5.
pub fn triangle(phase: f64) -> f64 {
    1.0 - (2.0 * phase.rem_euclid(1.0) - 1.0).abs()
}

/// How far the grabbed border travels, and how fast.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sweep {
    /// px the border travels back and forth. Negative drags the other way (a left border).
    pub amplitude: i32,
    /// Seconds for one there-and-back.
    pub period_s: f64,
    /// px added to every offset: negative starts the sweep inside the starting size, so it crosses the
    /// window's minimum size.
    pub shift: i32,
}

impl Sweep {
    /// The cursor's offset from the grab point `t` seconds into the sweep.
    pub fn offset(&self, t: f64) -> i32 {
        let travel = f64::from(self.amplitude) * triangle(t / self.period_s.max(f64::EPSILON));
        NUDGE_PX.saturating_add(self.shift).saturating_add(travel.round() as i32)
    }
}

/// Which cursor coordinates a drag moves: x for a left or right border, both for a corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Axes {
    pub x: bool,
    pub y: bool,
}

/// The point `offset` px along a drag that started at `grab`.
pub fn point_at(grab: (i32, i32), axes: Axes, offset: i32) -> (i32, i32) {
    (grab.0.saturating_add(if axes.x { offset } else { 0 }), grab.1.saturating_add(if axes.y { offset } else { 0 }))
}

/// Whether the cursor is off the line a drag moves it along, which only something else moving the mouse
/// can cause. The test stops then instead of fighting for the pointer. Compares the coordinate that
/// should not move, so it does not depend on how far a fast sweep has got.
pub fn off_path(cursor: (i32, i32), grab: (i32, i32), axes: Axes) -> bool {
    let (dx, dy) = (cursor.0 - grab.0, cursor.1 - grab.1);
    match (axes.x, axes.y) {
        (true, false) => dy.abs() > 6,
        (false, true) => dx.abs() > 6,
        (true, true) => (dx - dy).abs() > 8,
        (false, false) => false,
    }
}

/// What a drag's cursor made of the OS's pointer handling; see [`check_cursor`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorCheck {
    /// On the line the drag moves along.
    OnPath,
    /// Off that line at the edge of the display, where the OS pins the pointer: the sweep ran out of screen.
    AtScreenEdge,
    /// Off that line in the open, which only something else moving the mouse can cause.
    Moved,
}

/// Classify the cursor during a drag on a `screen`-sized display. A pointer the OS stopped at the display's
/// edge looks just like a moved mouse to [`off_path`] (on a corner drag one coordinate stops, the other goes
/// on), so the two are told apart: only a pointer off its line away from the edges is blamed on someone.
pub fn check_cursor(cursor: (i32, i32), grab: (i32, i32), axes: Axes, screen: (i32, i32)) -> CursorCheck {
    if !off_path(cursor, grab, axes) {
        return CursorCheck::OnPath;
    }
    let at_edge = |v: i32, size: i32| v <= 1 || v >= size - 2;
    if at_edge(cursor.0, screen.0) || at_edge(cursor.1, screen.1) { CursorCheck::AtScreenEdge } else { CursorCheck::Moved }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    RightEdge,
    LeftEdge,
    Corner,
    /// Many short press-drag-release cycles, as a person re-grabbing the border.
    ShortGrabs,
    MaximizeRestore,
}

/// Where to press to grab the border or corner `kind` names (the bottom-right corner) of a window whose outer
/// rectangle is `rect` (left, top, right, bottom), and which cursor coordinates the drag then moves. The point
/// is a few px inside the window, where its resize zone is.
pub fn grab_point(kind: Kind, rect: (i32, i32, i32, i32)) -> ((i32, i32), Axes) {
    let (left, top, right, bottom) = rect;
    let mid = (top + bottom) / 2;
    match kind {
        Kind::LeftEdge => ((left + LEFT_GRAB_INSET_PX, mid), Axes { x: true, y: false }),
        Kind::Corner => ((right - GRAB_INSET_PX, bottom - GRAB_INSET_PX), Axes { x: true, y: true }),
        _ => ((right - GRAB_INSET_PX, mid), Axes { x: true, y: false }),
    }
}

/// px the cursor can still move from `grab` in `direction` (+1 right and down, -1 left and up) along `axes`
/// before it is [`EDGE_GUARD_PX`] from the edge of a `screen`-sized display. The tighter axis counts, and the
/// result is negative when `grab` is already past the guard.
fn room(grab: (i32, i32), axes: Axes, screen: (i32, i32), direction: i32) -> i32 {
    let on_axis = |moves: bool, at: i32, size: i32| moves.then(|| if direction > 0 { size - 1 - EDGE_GUARD_PX - at } else { at - EDGE_GUARD_PX });
    [on_axis(axes.x, grab.0, screen.0), on_axis(axes.y, grab.1, screen.1)].into_iter().flatten().min().unwrap_or(i32::MAX)
}

/// A sweep cut down to the display by [`fit_sweep`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fitted {
    pub sweep: Sweep,
    /// Whether the sweep had to be cut down from the one asked for (the report says so).
    pub reduced: bool,
}

/// Fit `asked` to a `screen`-sized display for a drag that starts at `grab`: the whole drag (the nudge, the sweep
/// and the return to the start) stays [`EDGE_GUARD_PX`] inside the display on every axis it moves, so the OS
/// never stops the pointer. `grab` is the real one, from the window's real rectangle, so a window the OS made
/// bigger than planned (the app's minimum size at a display scale above 100 %) is accounted for.
///
/// A sweep that does not fit is cut down, not dropped: its amplitude shrinks to the room there is and its
/// period shrinks with it, so the border moves at the same speed. When the room is under [`MIN_TRAVEL_PX`]
/// there is no meaningful sweep to make, and the error says so.
pub fn fit_sweep(asked: Sweep, grab: (i32, i32), axes: Axes, screen: (i32, i32)) -> Result<Fitted, String> {
    let (up, down) = (room(grab, axes, screen, 1), room(grab, axes, screen, -1));
    let leftward = asked.amplitude < 0;
    // The grab point itself has to be inside the guard (`down >= 0`), then the nudge and the return to the start
    // (NUDGE_PX away on the sweep's side) have to fit.
    if up < NUDGE_PX || down < 0 || (leftward && down < NUDGE_PX) {
        return Err(format!(
            "the cursor has no room to start this drag from {grab:?} on a {}x{} display: the window is too close to its edge",
            screen.0, screen.1
        ));
    }
    let mut sweep = asked;
    // The sweep starts `NUDGE_PX + shift` from the grab point, which has to be on the display too.
    sweep.shift = sweep.shift.max(-down - NUDGE_PX).min(up - NUDGE_PX);
    let base = NUDGE_PX + sweep.shift;
    sweep.amplitude = if leftward { asked.amplitude.max(-(down + base)) } else { asked.amplitude.min(up - base) };
    if sweep.amplitude.abs() < MIN_TRAVEL_PX {
        return Err(format!(
            "a {}x{} display leaves {} px to resize from {grab:?}, and the test needs at least {MIN_TRAVEL_PX} px to mean anything: use a larger display",
            screen.0,
            screen.1,
            sweep.amplitude.abs()
        ));
    }
    if sweep.amplitude != asked.amplitude {
        // The same speed over a shorter way: the same px per second.
        sweep.period_s = (asked.period_s * f64::from(sweep.amplitude.abs()) / f64::from(asked.amplitude.abs())).max(0.03);
    }
    Ok(Fitted { sweep, reduced: sweep != asked })
}

/// Whether the value of `PHOTOCRAFT_RESIZE_STRESS` (`None`: not set) allows the tests that take the real mouse to
/// run. Only the exact value `1` does: `0`, `false`, `no`, an empty value and anything else leave them off, because
/// a value that reads as "off" must never switch on something that takes over the mouse.
pub fn opted_in(value: Option<&std::ffi::OsStr>) -> bool {
    value.is_some_and(|v| v == "1")
}

/// The `WxH` size in `text` (as `PHOTOCRAFT_STRESS_SCREEN` gives it), if it is one a window can be placed in.
pub fn parse_screen(text: &str) -> Option<(i32, i32)> {
    let (w, h) = text.trim().split_once(['x', 'X'])?;
    let (w, h) = (w.trim().parse::<i32>().ok()?, h.trim().parse::<i32>().ok()?);
    (w >= 640 && h >= 480).then_some((w, h))
}

/// One step of a soak run.
#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    pub kind: Kind,
    /// The window's outer size and position before the segment.
    pub size: (i32, i32),
    pub park: (i32, i32),
    pub sweep: Sweep,
    pub seconds: f64,
    /// Seconds between synthetic mouse moves (0.001 is a 1 kHz mouse).
    pub step_s: f64,
}

const START_SIZES: [(i32, i32); 4] = [(1000, 650), (1200, 720), (900, 600), (1400, 800)];
const PERIODS: [f64; 6] = [0.06, 0.1, 0.15, 0.25, 0.5, 1.0];

/// A window size that fits a `screen`-sized primary display with room to grow to the right.
fn fit(size: (i32, i32), screen: (i32, i32)) -> (i32, i32) {
    ((size.0).min(screen.0 - PARK.0 - 300).max(600), (size.1).min(screen.1 - PARK.1 - MARGIN).max(400))
}

/// The size a window starts from when the scenario does not choose one, fitted to a `screen`-sized display.
pub fn default_size(screen: (i32, i32)) -> (i32, i32) {
    fit(DEFAULT_SIZE, screen)
}

/// Where a window of `size` is parked so that its left border can travel left: against the display's right side.
fn left_park(size: (i32, i32), screen: (i32, i32)) -> (i32, i32) {
    ((screen.0 - MARGIN - size.0).max(PARK.0), PARK.1)
}

/// The drags the fixed (non-soak) scenarios make.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fixed {
    RightBorder,
    LeftBorder,
    Corner,
    BelowMinimum,
}

/// Where a fixed scenario parks the window, and the drag it makes there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DragPlan {
    pub kind: Kind,
    pub park: (i32, i32),
    pub size: (i32, i32),
    /// The sweep asked for. [`fit_sweep`] cuts it down to what the real window and display allow.
    pub sweep: Sweep,
}

/// What `which` does on a `screen`-sized display. The window is fitted to the display, and the corner's window is
/// kept short enough to leave room below it for the whole diagonal sweep (a border sweep only needs room to the
/// side), so a display of 1280 x 800 or more runs the sweeps as asked and a 1366 x 768 one loses only a few px.
pub fn fixed_plan(which: Fixed, screen: (i32, i32)) -> DragPlan {
    match which {
        Fixed::RightBorder => {
            DragPlan { kind: Kind::RightEdge, park: PARK, size: default_size(screen), sweep: Sweep { amplitude: 400, period_s: 0.3, shift: 0 } }
        }
        Fixed::BelowMinimum => {
            DragPlan { kind: Kind::RightEdge, park: PARK, size: fit((900, 700), screen), sweep: Sweep { amplitude: 800, period_s: 0.2, shift: -250 } }
        }
        Fixed::LeftBorder => {
            let size = default_size(screen);
            DragPlan { kind: Kind::LeftEdge, park: left_park(size, screen), size, sweep: Sweep { amplitude: -400, period_s: 0.4, shift: 0 } }
        }
        Fixed::Corner => {
            let sweep = Sweep { amplitude: 250, period_s: 0.4, shift: 0 };
            let below = sweep.amplitude + NUDGE_PX + GRAB_INSET_PX + EDGE_GUARD_PX + 1;
            let height = (screen.1 - PARK.1 - below).max(MIN_WINDOW_HEIGHT).min(DEFAULT_SIZE.1);
            DragPlan { kind: Kind::Corner, park: PARK, size: (default_size(screen).0, height), sweep }
        }
    }
}

/// A seeded plan of segments adding up to at least `seconds`, for a primary display of `screen` px. The
/// same seed and screen give the same plan.
pub fn soak_plan(seed: u64, seconds: f64, screen: (i32, i32)) -> Vec<Segment> {
    let mut rng = Rng::new(seed);
    let mut plan = Vec::new();
    let mut planned = 0.0;
    while planned < seconds {
        let kind = rng
            .weighted(&[(Kind::RightEdge, 5), (Kind::LeftEdge, 2), (Kind::Corner, 3), (Kind::ShortGrabs, 2), (Kind::MaximizeRestore, 1)])
            .unwrap_or(Kind::RightEdge);
        let mut size = fit(rng.pick(&START_SIZES).unwrap_or((1000, 650)), screen);
        if kind == Kind::Corner {
            // A corner also travels down: on a short display the window gives up height so that at least 100 px remain.
            size.1 = size.1.min(screen.1 - PARK.1 - MARGIN - 100).max(MIN_WINDOW_HEIGHT);
        }
        // A left border needs room to travel left, so that window sits against the screen's right side.
        let park = if kind == Kind::LeftEdge { left_park(size, screen) } else { PARK };
        let room_right = (screen.0 - park.0 - size.0 - MARGIN).max(0);
        let room_below = (screen.1 - park.1 - size.1 - MARGIN).max(0);
        let room_left = (park.0 - MARGIN / 2).max(0);
        let period_s = rng.pick(&PERIODS).unwrap_or(0.25);
        let (amplitude, shift, step_s) = match kind {
            Kind::RightEdge => {
                (rng.range(250, 900).min(room_right).max(60), rng.pick(&[0, -150, -350]).unwrap_or(0), rng.pick(&[0.001, 0.004, 0.008]).unwrap_or(0.004))
            }
            Kind::LeftEdge => (-rng.range(150, 400).min(room_left).max(60), 0, rng.pick(&[0.002, 0.004]).unwrap_or(0.004)),
            Kind::Corner => (rng.range(150, 300).min(room_right).min(room_below).max(MIN_TRAVEL_PX), rng.pick(&[0, -100, -250]).unwrap_or(0), 0.004),
            Kind::ShortGrabs => (rng.range(60, 300).min(room_right).max(40), 0, 0.004),
            Kind::MaximizeRestore => (0, 0, 0.004),
        };
        let seconds_here = rng.float(2.5, 7.0);
        // A segment also spends about a second and a half parking the window and settling.
        planned += seconds_here + 1.5;
        plan.push(Segment { kind, size, park, sweep: Sweep { amplitude, period_s, shift }, seconds: seconds_here, step_s });
    }
    plan
}

/// `count` outer window sizes for the API-driven storm: seeded, within what `screen` holds, every tenth
/// one an extreme, so the sequence jumps between the smallest and the largest window.
pub fn storm_sizes(seed: u64, count: usize, screen: (i32, i32)) -> Vec<(i32, i32)> {
    let mut rng = Rng::new(seed);
    let (min_w, min_h) = (700, 460);
    let max_w = (screen.0 - PARK.0 - 20).max(min_w + 1);
    let max_h = (screen.1 - PARK.1 - MARGIN).max(min_h + 1);
    (0..count)
        .map(|i| match i % 10 {
            0 if i % 20 == 0 => (min_w, min_h),
            0 => (max_w, max_h),
            _ => (rng.range(min_w, max_w), rng.range(min_h, max_h)),
        })
        .collect()
}

/// One answer to `ui.inspect` while the window was being resized.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Probe {
    /// When the answer arrived, in seconds since probing began.
    pub at_s: f64,
    pub latency_ms: f64,
    /// The app's frame counter, for the report.
    pub frame: u64,
}

/// What a run saw, for [`judge`].
#[derive(Clone, Debug, Default)]
pub struct Observations {
    /// How the app ended, if it did before the run did.
    pub died: Option<String>,
    /// Steps that failed on their own: a drag that did not start, a segment that ended at the wrong size.
    pub step_failures: Vec<String>,
    pub probes: Vec<Probe>,
    pub probe_failures: Vec<String>,
    /// How long probing ran, in seconds.
    pub probe_span_s: f64,
    /// After the run settled: the window's client size, the size of the frame the app rendered
    /// (`ui.screenshot`), and the size it laid its UI out at, all in physical px.
    pub client_px: (i32, i32),
    pub surface_px: Option<(u32, u32)>,
    pub laid_out_px: Option<(f64, f64)>,
    /// Why a command sent after the run (new document, fill) failed.
    pub edit_error: Option<String>,
    /// The app's private memory in MB before the run and after it; `None` when either could not be read, which
    /// [`judge`] reports as a failure (an unchecked limit is not a pass).
    pub memory_mb: Option<(f64, f64)>,
}

/// What counts as a failure. The defaults leave room for a slow debug build or a busy machine, so
/// they flag a hang and a leak, not a hiccup.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limits {
    /// The longest the UI thread may go without answering a probe.
    pub max_stall_ms: f64,
    pub memory_growth_mb: f64,
    pub size_tolerance_px: f64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { max_stall_ms: 3000.0, memory_growth_mb: 200.0, size_tolerance_px: 2.0 }
    }
}

/// The longest stretch, in ms, with no answered probe.
pub fn max_gap_ms(probes: &[Probe], span_s: f64) -> f64 {
    let mut longest = 0.0f64;
    let mut last = 0.0;
    for probe in probes {
        longest = longest.max(probe.at_s - last);
        last = probe.at_s;
    }
    longest.max(span_s - last) * 1000.0
}

/// The problems in a run; empty when it passed.
pub fn judge(obs: &Observations, limits: &Limits) -> Vec<String> {
    let mut problems = Vec::new();
    if let Some(how) = &obs.died {
        problems.push(format!("the app exited during the run ({how})"));
    }
    problems.extend(obs.step_failures.iter().cloned());
    problems.extend(obs.probe_failures.iter().map(|f| format!("a probe during the resize got no answer: {f}")));
    let gap = max_gap_ms(&obs.probes, obs.probe_span_s);
    if gap > limits.max_stall_ms {
        problems.push(format!("the UI thread answered nothing for {gap:.0} ms (limit {:.0} ms)", limits.max_stall_ms));
    }
    let (client_w, client_h) = (f64::from(obs.client_px.0), f64::from(obs.client_px.1));
    match obs.surface_px {
        None => problems.push("no frame was rendered after the resize (ui.screenshot failed)".into()),
        Some((w, h)) => {
            if (f64::from(w) - client_w).abs() > limits.size_tolerance_px || (f64::from(h) - client_h).abs() > limits.size_tolerance_px {
                problems.push(format!(
                    "the render surface is {w}x{h} px but the window is {}x{} px: it was not resized to match (a stale swap chain)",
                    obs.client_px.0, obs.client_px.1
                ));
            }
        }
    }
    if let Some((w, h)) = obs.laid_out_px
        && ((w - client_w).abs() > limits.size_tolerance_px || (h - client_h).abs() > limits.size_tolerance_px)
    {
        problems.push(format!("the UI is laid out at {w:.0}x{h:.0} px but the window is {}x{} px", obs.client_px.0, obs.client_px.1));
    }
    if let Some(error) = &obs.edit_error {
        problems.push(format!("the app stopped accepting commands after the resize: {error}"));
    }
    match obs.memory_mb {
        Some((before, after)) if after - before > limits.memory_growth_mb => {
            problems.push(format!("private memory grew from {before:.0} MB to {after:.0} MB (limit +{:.0} MB)", limits.memory_growth_mb));
        }
        Some(_) => {}
        None => problems.push(format!("private memory could not be measured, so the +{:.0} MB growth limit was not checked", limits.memory_growth_mb)),
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: (i32, i32) = (1920, 1080);

    /// A run in which nothing went wrong, for a test to spoil one field of.
    fn healthy() -> Observations {
        Observations {
            probes: (1u32..=20).map(|i| Probe { at_s: f64::from(i) * 0.25, latency_ms: 4.0, frame: u64::from(i) * 15 }).collect(),
            probe_span_s: 5.1,
            client_px: (1107, 700),
            surface_px: Some((1107, 700)),
            laid_out_px: Some((1107.0, 700.0)),
            memory_mb: Some((380.0, 383.0)),
            ..Observations::default()
        }
    }

    #[test]
    fn the_generator_repeats_from_a_seed_and_stays_in_range() {
        let draw = |seed| {
            let mut rng = Rng::new(seed);
            (0..64).map(|_| rng.range(-5, 5)).collect::<Vec<_>>()
        };
        assert_eq!(draw(7252), draw(7252));
        assert_ne!(draw(7252), draw(7253));
        let mut rng = Rng::new(1);
        for _ in 0..10_000 {
            assert!((-5..=5).contains(&rng.range(-5, 5)));
            assert!((2.5..7.0).contains(&rng.float(2.5, 7.0)));
        }
        assert_eq!(rng.range(3, 3), 3);
        assert_eq!(rng.range(9, 2), 9, "an empty range gives its low end");
        assert_eq!(rng.pick::<u8>(&[]), None);
        assert_eq!(rng.weighted::<u8>(&[]), None);
        assert_eq!(rng.weighted(&[('a', 0), ('b', 3)]), Some('b'));
    }

    #[test]
    fn the_triangle_wave_rises_to_one_and_returns() {
        assert_eq!(triangle(0.0), 0.0);
        assert_eq!(triangle(0.5), 1.0);
        assert_eq!(triangle(1.0), 0.0);
        assert_eq!(triangle(2.25), triangle(0.25), "periodic");
        assert_eq!(triangle(-0.25), triangle(0.75), "and defined before zero");
        assert!((0..1000).all(|i| (0.0..=1.0).contains(&triangle(f64::from(i) * 0.037))));
    }

    #[test]
    fn a_sweep_travels_its_amplitude_and_back_each_period() {
        let sweep = Sweep { amplitude: 400, period_s: 0.4, shift: 0 };
        assert_eq!(sweep.offset(0.0), NUDGE_PX);
        assert_eq!(sweep.offset(0.2), NUDGE_PX + 400);
        assert_eq!(sweep.offset(0.4), NUDGE_PX);
        let left = Sweep { amplitude: -300, period_s: 1.0, shift: 0 };
        assert_eq!(left.offset(0.5), NUDGE_PX - 300, "a left border is dragged left");
        let inward = Sweep { amplitude: 800, period_s: 0.12, shift: -250 };
        assert_eq!(inward.offset(0.0), NUDGE_PX - 250, "a negative shift starts inside the starting size");
        assert_eq!(Sweep { amplitude: 10, period_s: 0.0, shift: 0 }.offset(1.0).signum(), 1, "a zero period does not divide by zero");
    }

    #[test]
    fn a_drag_stops_when_something_else_moves_the_mouse() {
        let x_only = Axes { x: true, y: false };
        let corner = Axes { x: true, y: true };
        let grab = (1176, 390);
        // Anywhere along the line of travel, however far, is ours.
        assert!(!off_path((1176, 390), grab, x_only));
        assert!(!off_path((1576, 393), grab, x_only), "a pixel or two of rounding is not a user");
        assert!(!off_path((300, 390), grab, x_only), "a left sweep");
        assert!(off_path((1300, 420), grab, x_only), "someone moved the mouse sideways");
        assert!(!off_path((1276, 490), grab, corner), "a corner moves both coordinates together");
        assert!(off_path((1276, 390), grab, corner));
        assert!(!off_path((5, 5000), grab, Axes { x: false, y: false }), "a drag that moves nothing cannot be off its path");
        assert_eq!(point_at(grab, x_only, 50), (1226, 390));
        assert_eq!(point_at(grab, corner, -20), (1156, 370));
        assert_eq!(point_at(grab, Axes { x: false, y: true }, 7), (1176, 397));
    }

    #[test]
    fn a_soak_plan_is_reproducible_and_covers_every_kind() {
        let plan = soak_plan(7252, 300.0, SCREEN);
        assert_eq!(plan, soak_plan(7252, 300.0, SCREEN));
        assert_ne!(plan, soak_plan(7253, 300.0, SCREEN));
        let kinds: Vec<Kind> = plan.iter().map(|s| s.kind).collect();
        for kind in [Kind::RightEdge, Kind::LeftEdge, Kind::Corner, Kind::ShortGrabs, Kind::MaximizeRestore] {
            assert!(kinds.contains(&kind), "{kind:?} missing from {} segments", plan.len());
        }
        let spent: f64 = plan.iter().map(|s| s.seconds + 1.5).sum();
        assert!(spent >= 300.0, "the plan covers the requested time");
        assert!(spent < 300.0 + 8.5 + 1.5, "and stops at the first segment past it: {spent}");
    }

    #[test]
    fn soak_segments_keep_the_cursor_on_the_screen() {
        for screen in [SCREEN, (1366, 768), (2560, 1440), (3840, 2160)] {
            for seed in 0..40 {
                for s in soak_plan(seed, 120.0, screen) {
                    let right = s.park.0 + s.size.0;
                    let bottom = s.park.1 + s.size.1;
                    assert!(s.size.0 >= 600 && s.size.1 >= 400, "{s:?}");
                    assert!(right <= screen.0 && bottom <= screen.1, "the parked window fits {screen:?}: {s:?}");
                    let travel = (0..=200).map(|i| s.sweep.offset(f64::from(i) * s.sweep.period_s / 100.0)).collect::<Vec<_>>();
                    let (lo, hi) = (travel.iter().min().copied().unwrap_or(0), travel.iter().max().copied().unwrap_or(0));
                    match s.kind {
                        Kind::LeftEdge => assert!(s.park.0 + lo >= 0, "the left border stays on screen: {s:?} {screen:?}"),
                        Kind::MaximizeRestore => {}
                        _ => assert!(right + hi <= screen.0, "the right border stays on screen: {s:?} {screen:?}"),
                    }
                    if s.kind == Kind::Corner {
                        assert!(bottom + hi <= screen.1, "the bottom border stays on screen: {s:?} {screen:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn storm_sizes_repeat_stay_in_bounds_and_jump_between_extremes() {
        let sizes = storm_sizes(7252, 400, SCREEN);
        assert_eq!(sizes, storm_sizes(7252, 400, SCREEN));
        assert_eq!(sizes.len(), 400);
        assert!(sizes.iter().all(|&(w, h)| (700..=SCREEN.0 - PARK.0 - 20).contains(&w) && (460..=SCREEN.1 - PARK.1 - MARGIN).contains(&h)));
        assert_eq!(sizes.first(), Some(&(700, 460)));
        assert_eq!(sizes.get(10), Some(&(SCREEN.0 - PARK.0 - 20, SCREEN.1 - PARK.1 - MARGIN)));
        assert!(storm_sizes(1, 0, SCREEN).is_empty());
    }

    #[test]
    fn a_healthy_run_has_no_problems() {
        assert_eq!(judge(&healthy(), &Limits::default()), Vec::<String>::new());
    }

    #[test]
    fn a_dead_app_a_missed_drag_and_an_unanswered_probe_are_each_reported() {
        let obs = Observations {
            died: Some("exit code: 0xc0000005".into()),
            step_failures: vec!["segment 2: the OS never entered its modal sizing loop".into()],
            probe_failures: vec!["ui.inspect: timed out".into()],
            ..healthy()
        };
        let problems = judge(&obs, &Limits::default());
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert!(problems[0].contains("0xc0000005"));
        assert!(problems[1].contains("modal sizing loop"));
        assert!(problems[2].contains("timed out"));
    }

    #[test]
    fn a_frozen_ui_thread_is_a_gap_in_the_answers() {
        let mut obs = healthy();
        assert!(max_gap_ms(&obs.probes, obs.probe_span_s) < 400.0);
        // Answers at 0.25, 0.5 and 0.75 s, then nothing until 4.75 s.
        obs.probes.retain(|p| p.at_s < 1.0 || p.at_s > 4.5);
        let gap = max_gap_ms(&obs.probes, obs.probe_span_s);
        assert!((3900.0..4100.0).contains(&gap), "{gap}");
        assert!(judge(&obs, &Limits::default()).iter().any(|p| p.contains("answered nothing")));
        assert!(judge(&obs, &Limits { max_stall_ms: 5000.0, ..Limits::default() }).is_empty(), "the limit is configurable");
        // No answer at all, for the whole run, is a stall too (and a run that never probed is not).
        assert_eq!(max_gap_ms(&[], 4.0), 4000.0);
        assert_eq!(max_gap_ms(&[], 0.0), 0.0);
        // A stall before the first answer and after the last one counts as well.
        assert_eq!(max_gap_ms(&[Probe { at_s: 2.0, latency_ms: 1.0, frame: 1 }], 2.5), 2000.0);
        assert_eq!(max_gap_ms(&[Probe { at_s: 0.5, latency_ms: 1.0, frame: 1 }], 4.5), 4000.0);
    }

    /// The displays the review of the first version named, where the corner sweep ran off the screen.
    const SMALL_DISPLAYS: [(i32, i32); 3] = [(1366, 768), (1600, 900), (1280, 800)];
    /// Displays this test is likely to meet, from the smallest laptops to 4K.
    const DISPLAYS: [(i32, i32); 10] =
        [(1280, 720), (1280, 800), (1366, 768), (1440, 900), (1600, 900), (1680, 1050), (1920, 1080), (2560, 1440), (3440, 1440), (3840, 2160)];
    const FIXED: [Fixed; 4] = [Fixed::RightBorder, Fixed::LeftBorder, Fixed::Corner, Fixed::BelowMinimum];

    /// Where the OS leaves a pointer sent to `p`: never past the edge of the display.
    fn pinned(p: (i32, i32), screen: (i32, i32)) -> (i32, i32) {
        (p.0.clamp(0, screen.0 - 1), p.1.clamp(0, screen.1 - 1))
    }

    /// Every cursor position a drag visits: the nudge after the press, the sweep (over two periods) and the return.
    fn visited(grab: (i32, i32), axes: Axes, sweep: Sweep) -> Vec<(i32, i32)> {
        let direction = if sweep.amplitude < 0 { -1 } else { 1 };
        let nudge = (1..=NUDGE_PX).map(|i| point_at(grab, axes, direction * i));
        let swept = (0..=400).map(|i| point_at(grab, axes, sweep.offset(f64::from(i) * sweep.period_s / 200.0)));
        nudge.chain(swept).chain([point_at(grab, axes, NUDGE_PX)]).collect()
    }

    /// The grab point and axes of a fixed plan's drag, from the rectangle it parks the window at.
    fn grab_of(plan: &DragPlan) -> ((i32, i32), Axes) {
        grab_point(plan.kind, (plan.park.0, plan.park.1, plan.park.0 + plan.size.0, plan.park.1 + plan.size.1))
    }

    /// Whether `p` is on the display, with the guard, on every axis the drag moves.
    fn on_screen(p: (i32, i32), axes: Axes, screen: (i32, i32)) -> bool {
        let inside = |v: i32, size: i32| (EDGE_GUARD_PX..=size - 1 - EDGE_GUARD_PX).contains(&v);
        (!axes.x || inside(p.0, screen.0)) && (!axes.y || inside(p.1, screen.1))
    }

    /// px per second the border travels at.
    fn speed(s: Sweep) -> f64 {
        2.0 * f64::from(s.amplitude.abs()) / s.period_s
    }

    #[test]
    fn the_corner_sweep_as_first_written_ran_off_these_displays() {
        // The first version: a 1100 x 700 window at (80, 40), a corner grab 4 px inside it and 250 px of sweep on
        // both axes took the cursor to (1434, 994): past the bottom of a 768, 800 or 900 px high display.
        let (park, size, sweep) = (PARK, (1100, 700), Sweep { amplitude: 250, period_s: 0.4, shift: 0 });
        let (grab, axes) = grab_point(Kind::Corner, (park.0, park.1, park.0 + size.0, park.1 + size.1));
        let far = point_at(grab, axes, sweep.offset(sweep.period_s / 2.0));
        assert_eq!(far, (1434, 994));
        for screen in SMALL_DISPLAYS {
            assert!(far.1 > screen.1 - 1, "{screen:?}: the target is below the display");
            let pointer = pinned(far, screen);
            // The OS stops y at the bottom edge while x goes on, so the pointer is off its diagonal ...
            assert!(off_path(pointer, grab, axes), "{screen:?}: {pointer:?}");
            // ... which is the display's doing, and must not be put down to a person.
            assert_eq!(check_cursor(pointer, grab, axes, screen), CursorCheck::AtScreenEdge, "{screen:?}");
        }
        assert!(far.0 < 1919 && far.1 < 1079, "a 1920 x 1080 display holds it");
    }

    #[test]
    fn a_pointer_the_display_stopped_is_never_blamed_on_the_user() {
        let corner = Axes { x: true, y: true };
        let x_only = Axes { x: true, y: false };
        let (grab, screen) = ((1176, 736), (1600, 900));
        // Every point of a sweep that runs off the display, pinned by it, is on the path or at the edge: never "moved".
        for i in 0..=100 {
            let wanted = point_at(grab, corner, NUDGE_PX + i * 3);
            assert_ne!(check_cursor(pinned(wanted, screen), grab, corner, screen), CursorCheck::Moved, "offset {i}");
        }
        // Away from the edges, a pointer off its line is still someone else's doing ...
        assert_eq!(check_cursor((1300, 420), grab, x_only, screen), CursorCheck::Moved);
        assert_eq!(check_cursor((1290, 600), grab, corner, screen), CursorCheck::Moved);
        // ... and one on its line is fine, as is an x-only drag whose pointer stops at the right edge (only y matters to it).
        assert_eq!(check_cursor((1276, 836), grab, corner, screen), CursorCheck::OnPath);
        assert_eq!(check_cursor((1599, 736), grab, x_only, screen), CursorCheck::OnPath);
    }

    #[test]
    fn every_fixed_scenario_fits_every_common_display() {
        for screen in DISPLAYS {
            for which in FIXED {
                let plan = fixed_plan(which, screen);
                let label = format!("{which:?} on {screen:?}");
                assert!(plan.park.0 + plan.size.0 <= screen.0 && plan.park.1 + plan.size.1 <= screen.1, "{label}: the window does not fit: {plan:?}");
                let (grab, axes) = grab_of(&plan);
                let fitted = fit_sweep(plan.sweep, grab, axes, screen).unwrap_or_else(|e| panic!("{label}: {e}"));
                for p in visited(grab, axes, fitted.sweep) {
                    assert!(on_screen(p, axes, screen), "{label}: {p:?} is off the display; sweep {:?}", fitted.sweep);
                }
                // Cut down to fit, not gutted, and no slower.
                assert!(fitted.sweep.amplitude.abs() * 2 >= plan.sweep.amplitude.abs(), "{label}: {:?} from {:?}", fitted.sweep, plan.sweep);
                assert!((speed(fitted.sweep) - speed(plan.sweep)).abs() < 1e-6, "{label}: the border slowed down");
                assert_eq!(fitted.reduced, fitted.sweep != plan.sweep, "{label}");
            }
        }
    }

    #[test]
    fn the_small_displays_run_the_corner_sweep_as_asked_or_nearly() {
        let corner = |screen| {
            let plan = fixed_plan(Fixed::Corner, screen);
            let (grab, axes) = grab_of(&plan);
            fit_sweep(plan.sweep, grab, axes, screen).unwrap()
        };
        // 1280 x 800 and 1600 x 900 hold the whole sweep: the window is made just short enough.
        for screen in [(1280, 800), (1600, 900)] {
            let fitted = corner(screen);
            assert!(!fitted.reduced, "{screen:?}: {fitted:?}");
            assert_eq!(fitted.sweep.amplitude, 250);
        }
        // 1366 x 768 gives up 10 px of it: the window is already at the app's minimum height there.
        assert_eq!(fixed_plan(Fixed::Corner, (1366, 768)).size.1, 480);
        let fitted = corner((1366, 768));
        assert!(fitted.reduced);
        assert_eq!(fitted.sweep.amplitude, 240);
        assert!((speed(fitted.sweep) - 2.0 * 250.0 / 0.4).abs() < 1e-6, "at the same speed");
    }

    #[test]
    fn a_display_with_no_room_is_reported_not_skipped() {
        let check = |which: Fixed, screen: (i32, i32)| {
            let plan = fixed_plan(which, screen); // planning never panics, however small the display
            let (grab, axes) = grab_of(&plan);
            fit_sweep(plan.sweep, grab, axes, screen)
        };
        // 640 x 480: these windows start past the display's edge, so there is nowhere to press.
        for which in [Fixed::RightBorder, Fixed::BelowMinimum, Fixed::Corner] {
            let error = check(which, (640, 480)).expect_err("the window starts off the display");
            assert!(error.contains("640x480") && error.contains("no room to start"), "{which:?}: {error}");
        }
        // The left border still has 80 px to travel there: a short sweep, reported as cut down, not an error.
        let left = check(Fixed::LeftBorder, (640, 480)).unwrap();
        assert!(left.reduced && left.sweep.amplitude == -88, "{left:?}");
        // 700 x 600: room to start, but 12 px of travel are not a stress test.
        let error = check(Fixed::RightBorder, (700, 600)).expect_err("12 px of travel");
        assert!(error.contains("700x600") && error.contains(&format!("at least {MIN_TRAVEL_PX}")), "{error}");
    }

    #[test]
    fn fitting_cuts_a_sweep_to_the_room_and_keeps_its_speed() {
        let screen = (1366, 768);
        let x_only = Axes { x: true, y: false };
        let asked = Sweep { amplitude: 400, period_s: 0.3, shift: 0 };
        // 300 px are left to the right of the grab point once the guard is kept: 292 after the 8 px nudge.
        let fitted = fit_sweep(asked, (1062, 384), x_only, screen).unwrap();
        assert!(fitted.reduced);
        assert_eq!(fitted.sweep.amplitude, 292);
        assert!((speed(fitted.sweep) - speed(asked)).abs() < 1e-6);
        // A sweep that fits comes back as asked.
        let roomy = fit_sweep(asked, (1062, 384), x_only, (1920, 1080)).unwrap();
        assert_eq!((roomy.sweep, roomy.reduced), (asked, false));
        // A left border goes the other way.
        let left = fit_sweep(Sweep { amplitude: -400, period_s: 0.4, shift: 0 }, (323, 384), x_only, screen).unwrap();
        assert_eq!(left.sweep.amplitude, -328);
        // A sweep that starts inside the window stays on the display too: 97 px of room, minus the nudge.
        let inward = fit_sweep(Sweep { amplitude: 800, period_s: 0.2, shift: -250 }, (100, 384), x_only, screen).unwrap();
        assert_eq!(inward.sweep.shift, -105);
        // A corner is held by whichever axis has the least room.
        let both = Axes { x: true, y: true };
        let diagonal = fit_sweep(Sweep { amplitude: 250, period_s: 0.4, shift: 0 }, (1062, 564), both, screen).unwrap();
        assert_eq!(diagonal.sweep.amplitude, 200 - NUDGE_PX);
    }

    #[test]
    fn fitting_refuses_a_start_with_no_room_and_a_sweep_too_short_to_mean_anything() {
        let x_only = Axes { x: true, y: false };
        let sweep = Sweep { amplitude: 400, period_s: 0.3, shift: 0 };
        let screen = (1366, 768);
        // 5 px from the right edge: not even the nudge fits. Likewise a left border 5 px from the left edge.
        let error = fit_sweep(sweep, (1361, 100), x_only, screen).unwrap_err();
        assert!(error.contains("no room to start"), "{error}");
        let error = fit_sweep(Sweep { amplitude: -400, ..sweep }, (8, 100), x_only, screen).unwrap_err();
        assert!(error.contains("no room to start"), "{error}");
        // A grab point already inside the edge guard is refused whichever way the sweep goes (the nudge would cross it).
        let error = fit_sweep(sweep, (1, 100), x_only, screen).unwrap_err();
        assert!(error.contains("no room to start"), "{error}");
        // 30 px of travel are not a stress test, and 60 are.
        let error = fit_sweep(sweep, (1366 - 1 - EDGE_GUARD_PX - NUDGE_PX - 30, 100), x_only, screen).unwrap_err();
        assert!(error.contains("1366x768") && error.contains(&format!("at least {MIN_TRAVEL_PX}")), "{error}");
        let fitted = fit_sweep(sweep, (1366 - 1 - EDGE_GUARD_PX - NUDGE_PX - MIN_TRAVEL_PX, 100), x_only, screen).unwrap();
        assert_eq!(fitted.sweep.amplitude, MIN_TRAVEL_PX);
    }

    #[test]
    fn a_fitted_sweep_never_leaves_the_display() {
        let mut rng = Rng::new(7252);
        let mut fitted_count = 0;
        for _ in 0..20_000 {
            let screen = rng.pick(&DISPLAYS).unwrap_or((1920, 1080));
            let axes = if rng.range(0, 1) == 0 { Axes { x: true, y: false } } else { Axes { x: true, y: true } };
            let grab = (rng.range(0, screen.0 - 1), rng.range(0, screen.1 - 1));
            let asked = Sweep { amplitude: rng.range(-900, 900), period_s: rng.float(0.05, 1.0), shift: rng.range(-400, 100) };
            if let Ok(fitted) = fit_sweep(asked, grab, axes, screen) {
                fitted_count += 1;
                for p in visited(grab, axes, fitted.sweep) {
                    assert!(on_screen(p, axes, screen), "{p:?} is off a {screen:?} display: grab {grab:?}, asked {asked:?}, fitted {:?}", fitted.sweep);
                }
                assert!(fitted.sweep.amplitude.abs() >= MIN_TRAVEL_PX && fitted.sweep.amplitude.abs() <= asked.amplitude.abs());
                assert_eq!(fitted.sweep.amplitude.signum(), asked.amplitude.signum(), "a sweep never changes direction");
            }
        }
        assert!(fitted_count > 2_000, "the property was exercised: {fitted_count}");
    }

    #[test]
    fn fitting_leaves_every_soak_segment_as_planned() {
        for screen in DISPLAYS {
            for seed in 0..40 {
                for seg in soak_plan(seed, 120.0, screen) {
                    let kind = if seg.kind == Kind::ShortGrabs { Kind::RightEdge } else { seg.kind };
                    if kind == Kind::MaximizeRestore {
                        continue;
                    }
                    let (grab, axes) = grab_point(kind, (seg.park.0, seg.park.1, seg.park.0 + seg.size.0, seg.park.1 + seg.size.1));
                    let fitted = fit_sweep(seg.sweep, grab, axes, screen).unwrap_or_else(|e| panic!("{screen:?} seed {seed}: {seg:?}: {e}"));
                    assert!(!fitted.reduced, "{screen:?} seed {seed}: the plan already fits, yet {seg:?} was cut to {:?}", fitted.sweep);
                }
            }
        }
    }

    #[test]
    fn only_the_exact_value_1_allows_the_tests_that_take_the_mouse() {
        use std::ffi::OsStr;
        assert!(opted_in(Some(OsStr::new("1"))));
        // Anything else is off, however much it reads like "on" or like "off".
        for value in ["0", "false", "False", "FALSE", "true", "TRUE", "yes", "on", "y", "2", "01", "11", "1 ", " 1", " ", ""] {
            assert!(!opted_in(Some(OsStr::new(value))), "{value:?} must not allow them");
        }
        assert!(!opted_in(None), "unset must not allow them");
    }

    #[test]
    fn a_display_size_is_wxh() {
        assert_eq!(parse_screen("1366x768"), Some((1366, 768)));
        assert_eq!(parse_screen(" 1280 X 800 "), Some((1280, 800)));
        for bad in ["", "banana", "1366", "1366x", "x768", "0x0", "-1366x768", "639x480", "1366x479", "1366x768x2", "13.5x7"] {
            assert_eq!(parse_screen(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_surface_left_at_the_old_size_is_a_stale_swap_chain() {
        // The reporter's screenshot in #7252: content drawn at the old width, black where the window grew.
        let obs = Observations { client_px: (1260, 895), surface_px: Some((1110, 895)), laid_out_px: Some((1110.0, 895.0)), ..healthy() };
        let problems = judge(&obs, &Limits::default());
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems[0].contains("1110x895") && problems[0].contains("1260x895") && problems[0].contains("stale"));
        assert!(problems[1].contains("laid out"));
        // Within the tolerance is not a mismatch (client sizes round).
        let near = Observations { client_px: (1108, 701), surface_px: Some((1107, 700)), laid_out_px: Some((1107.5, 700.4)), ..healthy() };
        assert!(judge(&near, &Limits::default()).is_empty());
        // No frame at all is reported, not skipped.
        let none = Observations { surface_px: None, ..healthy() };
        assert!(judge(&none, &Limits::default()).iter().any(|p| p.contains("no frame was rendered")));
    }

    #[test]
    fn a_run_whose_memory_could_not_be_measured_fails_instead_of_passing() {
        // `win::private_mb` can fail (the process is gone, or the OS will not say), and a limit that was not
        // checked is not a pass.
        let unmeasured = Observations { memory_mb: None, ..healthy() };
        let problems = judge(&unmeasured, &Limits::default());
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("could not be measured") && problems[0].contains("+200 MB"), "{problems:?}");
        // Whatever the limit, and next to other problems, it is still reported.
        let limits = Limits { memory_growth_mb: 50.0, ..Limits::default() };
        assert!(judge(&unmeasured, &limits).iter().any(|p| p.contains("could not be measured") && p.contains("+50 MB")));
        let and_dead = Observations { memory_mb: None, died: Some("exit code: 1".into()), ..healthy() };
        assert_eq!(judge(&and_dead, &Limits::default()).len(), 2);
    }

    #[test]
    fn memory_growth_is_judged_against_the_configured_limit() {
        let judged = |growth: f64, limit: f64| {
            let obs = Observations { memory_mb: Some((400.0, 400.0 + growth)), ..healthy() };
            judge(&obs, &Limits { memory_growth_mb: limit, ..Limits::default() })
        };
        // Normal growth, none and shrinking are fine, and growth exactly at the limit is not past it.
        for growth in [3.0, 0.0, -80.0, 200.0] {
            assert!(judged(growth, 200.0).is_empty(), "{growth}");
        }
        // 1 MB past the limit fails, naming both readings and the limit.
        let over = judged(201.0, 200.0);
        assert_eq!(over.len(), 1, "{over:?}");
        assert!(over[0].contains("grew from 400 MB to 601 MB") && over[0].contains("limit +200 MB"), "{over:?}");
        // The limit is whatever was configured.
        assert!(judged(60.0, 50.0).iter().any(|p| p.contains("limit +50 MB")));
        assert!(judged(40.0, 50.0).is_empty());
        assert!(judged(500.0, 1000.0).is_empty());
    }

    #[test]
    fn commands_that_stop_working_and_memory_that_keeps_growing_are_reported() {
        let obs = Observations { edit_error: Some("engine.execute: timed out".into()), memory_mb: Some((380.0, 700.0)), ..healthy() };
        let problems = judge(&obs, &Limits::default());
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems[0].contains("stopped accepting commands"));
        assert!(problems[1].contains("380 MB to 700 MB"));
        // Shrinking, or growing by less than the limit, is fine.
        assert!(judge(&Observations { memory_mb: Some((380.0, 120.0)), ..healthy() }, &Limits::default()).is_empty());
        assert!(judge(&Observations { memory_mb: Some((380.0, 579.0)), ..healthy() }, &Limits::default()).is_empty());
    }
}
