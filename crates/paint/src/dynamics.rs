//! From input points to dabs: smoothing, brush pose, path walking (spacing, airbrush build-up) and
//! the per-dab dynamics (shape, scattering, transfer, colour, texture depth).
//!
//! Everything is incremental (feed points in any chunking, get the same dabs) and deterministic:
//! jitter comes from [`crate::rng`] keyed by the stroke seed and the step/dab index.

use photocraft_geom::Point;

use crate::brush::{BrushSettings, Control, Dynamic, Smoothing};
use crate::rng::{rand_signed, rand01, stream};
use crate::{Dab, StrokePoint};

/// Maximum pulled-string length at 100 % smoothing, in screen pixels.
pub const MAX_STRING_PX: f64 = 100.0;
/// Stroke time over which exponential smoothing moves its fraction of the way to the pointer.
pub const SMOOTHING_STEP_MS: f64 = 16.0;

/// With Spacing unchecked, one dab per this many milliseconds of stroke time, so faster strokes
/// space their dabs further apart (Photoshop: "the speed of the cursor determines the spacing").
pub const SPEED_SPACING_MS: f64 = 8.0;

/// Longest pixel-grid segment (in pixels) before falling back to distance spacing.
const MAX_GRID_STEPS: f64 = 1e7;

/// Densest speed spacing, in pixels between dabs (a slow, long stroke never floods the buffer).
const MIN_SPEED_STEP: f64 = 0.5;
/// Maximum airbrush catch-up dabs for one input segment.
const MAX_AIRBRUSH_DABS_PER_SEGMENT: usize = 512;


// ADDED
/// Dabs this thin may run freely for THIN_FREE_DABS dabs, then fade to nothing over THIN_FADE_DABS more (no hairs, no hard cut).
const THIN_FREE_DABS: u32 = 16;
const THIN_FADE_DABS: u32 = 24;


#[inline]
fn lerp_pt(a: &StrokePoint, b: &StrokePoint, f: f64) -> StrokePoint {
    let ff = f as f32;
    StrokePoint {
        x: a.x + (b.x - a.x) * f,
        y: a.y + (b.y - a.y) * f,
        pressure: a.pressure + (b.pressure - a.pressure) * ff,
        tilt_x: a.tilt_x + (b.tilt_x - a.tilt_x) * ff,
        tilt_y: a.tilt_y + (b.tilt_y - a.tilt_y) * ff,
        rotation: a.rotation + (b.rotation - a.rotation) * ff,
        wheel: a.wheel + (b.wheel - a.wheel) * ff,
        time: a.time + (b.time - a.time) * f,
    }
}

// ---------------------------------------------------------------------------
// Smoothing
// ---------------------------------------------------------------------------

/// Incremental stroke smoothing: exponential (with optional time-based catch-up) or pulled string.
#[derive(Clone)]
#[derive(Debug)]
pub struct Smoother {
    cfg: Smoothing,
    zoom: f64,
    brush_size: f32, // ----->>>> ADDED
    pos: Option<StrokePoint>,
    prev_pos: Option<StrokePoint>, // ----->>>> ADDED
    last_input: Option<StrokePoint>,
    prev_pressure: f32,  // ----->>>> ADDED
    last_dir: (f64, f64), // ----->>>> ADDED
    start_pos: Option<StrokePoint>, // Added for the startup filter
    started: bool,                  // Added for the startup filter
}

impl Smoother {
    pub fn new(cfg: &Smoothing, zoom: f32, brush_size: f32) -> Self {
        Self {
            cfg: cfg.clone(),
            zoom: f64::from(zoom.max(0.01)),
            brush_size,
            pos: None,
            prev_pos: None,
            last_input: None,
            prev_pressure: 0.0,
            last_dir: (0.0, 0.0),
            start_pos: None,
            started: false,
        }
    }

    fn active(&self) -> bool {
        self.cfg.enabled && self.cfg.amount > 0.0
    }


    ///////////////////////////////////////////////////////////////////////////////
    // FIX
    pub fn push(&mut self, mut p: StrokePoint, out: &mut Vec<StrokePoint>) {
        if p.pressure < 0.0 {
            p.pressure = 0.0;
        }

        // --- 1. THE 1.5px STARTUP FILTER ---
        // Swallow the initial burst of tablet noise, but keep the exact touch point.
        if !self.started {
            if let Some(start) = self.start_pos {
                let dx = p.x - start.x;
                let dy = p.y - start.y;
                if (dx * dx + dy * dy).sqrt() < 1.5 {
                    return;
                }
            } else {
                self.start_pos = Some(p);
                self.pos = Some(p);
                self.prev_pos = Some(p);
                self.last_input = Some(p);
                self.prev_pressure = p.pressure;
                return;
            }
            self.started = true;
            let start = self.start_pos.take().unwrap();
            out.push(start);

            let dx = p.x - start.x;
            let dy = p.y - start.y;
            let dist = (dx * dx + dy * dy).sqrt();
            if dist > 1e-4 {
                self.last_dir = (dx / dist, dy / dist);
            }

            self.pos = Some(p);
            self.prev_pos = Some(start);
            self.last_input = Some(p);
            self.prev_pressure = p.pressure;
            out.push(p);
            return;
        }

        // --- 2. THE UNIVERSAL JITTER FILTER ---
        // Applies to BOTH Smoothing ON and OFF. This is what fixes the jagged "Smoothing OFF" strokes.
        if let Some(prev) = self.last_input {
            let mdx = p.x - prev.x;
            let mdy = p.y - prev.y;
            let mdist = (mdx * mdx + mdy * mdy).sqrt();

            // Ignore sub-pixel noise
            if mdist < 0.5 {
                return;
            }

            // Ignore tiny backward movements (tablet lift/press jitter)
            let nx = mdx / mdist;
            let ny = mdy / mdist;
            let dir_mag = self.last_dir.0 * self.last_dir.0 + self.last_dir.1 * self.last_dir.1;
            if dir_mag > 1e-6 {
                let dot = nx * self.last_dir.0 + ny * self.last_dir.1;
                if dot < 0.0 && mdist < 3.0 {
                    return;
                }
            }
        }

        let prev_input = self.last_input;
        self.last_input = Some(p);

        const PRESSURE_LOWPASS: f32 = 0.35;
        let target_pressure = p.pressure * (1.0 - PRESSURE_LOWPASS) + self.prev_pressure * PRESSURE_LOWPASS;

        // --- 3. SMOOTHING OFF ---
        if !self.active() {
            let mut p2 = p;
            p2.pressure = target_pressure;

            if let Some(q) = self.pos {
                let mdx = p2.x - q.x;
                let mdy = p2.y - q.y;
                let mdist = (mdx * mdx + mdy * mdy).sqrt();
                if mdist > 1.0 {
                    let nx = mdx / mdist;
                    let ny = mdy / mdist;
                    let dir_mag = self.last_dir.0 * self.last_dir.0 + self.last_dir.1 * self.last_dir.1;
                    if dir_mag < 1e-6 {
                        self.last_dir = (nx, ny);
                    } else {
                        let dot = nx * self.last_dir.0 + ny * self.last_dir.1;
                        if dot > -0.5 {
                            let dx = self.last_dir.0 * 0.7 + nx * 0.3;
                            let dy = self.last_dir.1 * 0.7 + ny * 0.3;
                            let len = (dx * dx + dy * dy).sqrt();
                            if len > 1e-4 {
                                self.last_dir = (dx / len, dy / len);
                            }
                        }
                    }
                }
            }

            self.prev_pressure = target_pressure;
            self.prev_pos = self.pos;
            self.pos = Some(p2);
            out.push(p2);
            return;
        }

        // --- 4. SMOOTHING ON ---
        let Some(pos) = self.pos else {
            self.prev_pressure = target_pressure;
            self.prev_pos = Some(p);
            self.pos = Some(p);
            out.push(p);
            return;
        };

        let amount = f64::from(self.cfg.amount.clamp(0.0, 1.0));
        if self.cfg.pulled_string {
            let mut len = amount * MAX_STRING_PX;
            if self.cfg.adjust_for_zoom {
                len /= self.zoom;
            }
            let (dx, dy) = (p.x - pos.x, p.y - pos.y);
            let d = (dx * dx + dy * dy).sqrt();
            if d > len {
                let k = (d - len) / d;
                let np = StrokePoint { x: pos.x + dx * k, y: pos.y + dy * k, ..p };

                let mdx = np.x - pos.x;
                let mdy = np.y - pos.y;
                let mdist = (mdx * mdx + mdy * mdy).sqrt();
                if mdist > 1.0 {
                    let nx = mdx / mdist;
                    let ny = mdy / mdist;
                    let dir_mag = self.last_dir.0 * self.last_dir.0 + self.last_dir.1 * self.last_dir.1;
                    if dir_mag < 1e-6 {
                        self.last_dir = (nx, ny);
                    } else {
                        let dot = nx * self.last_dir.0 + ny * self.last_dir.1;
                        if dot > -0.5 {
                            let dx = self.last_dir.0 * 0.7 + nx * 0.3;
                            let dy = self.last_dir.1 * 0.7 + ny * 0.3;
                            let len = (dx * dx + dy * dy).sqrt();
                            if len > 1e-4 {
                                self.last_dir = (dx / len, dy / len);
                            }
                        }
                    }
                }

                self.prev_pos = self.pos;
                self.pos = Some(np);
                out.push(np);
            }
            return;
        }

        let a = 1.0 - amount.min(0.95);
        let dt = prev_input.map_or(0.0, |q| p.time - q.time);
        let still = prev_input.is_some_and(|q| q.x == p.x && q.y == p.y);
        if still && !self.cfg.catch_up {
            return;
        }

        let (iters, step) = if dt > 0.0 && dt.is_finite() {
            let steps = dt / SMOOTHING_STEP_MS;
            let iters = steps.ceil().clamp(1.0, 64.0);
            (iters as usize, 1.0 - (1.0 - a).powf(steps.min(64.0) / iters))
        } else {
            (1, a)
        };

        let mut cur = pos;
        let f = step as f32;
        for _ in 0..iters {
            cur = StrokePoint {
                x: cur.x + (p.x - cur.x) * step,
                y: cur.y + (p.y - cur.y) * step,
                pressure: cur.pressure + (target_pressure - cur.pressure) * f,
                tilt_x: cur.tilt_x + (p.tilt_x - cur.tilt_x) * f,
                tilt_y: cur.tilt_y + (p.tilt_y - cur.tilt_y) * f,
                rotation: cur.rotation + (p.rotation - cur.rotation) * f,
                wheel: cur.wheel + (p.wheel - cur.wheel) * f,
                time: p.time,
            };
            out.push(cur);
        }

        let mdx = cur.x - pos.x;
        let mdy = cur.y - pos.y;
        let mdist = (mdx * mdx + mdy * mdy).sqrt();
        if mdist > 1.0 {
            let nx = mdx / mdist;
            let ny = mdy / mdist;
            let dir_mag = self.last_dir.0 * self.last_dir.0 + self.last_dir.1 * self.last_dir.1;
            if dir_mag < 1e-6 {
                self.last_dir = (nx, ny);
            } else {
                let dot = nx * self.last_dir.0 + ny * self.last_dir.1;
                if dot > -0.5 {
                    let dx = self.last_dir.0 * 0.7 + nx * 0.3;
                    let dy = self.last_dir.1 * 0.7 + ny * 0.3;
                    let len = (dx * dx + dy * dy).sqrt();
                    if len > 1e-4 {
                        self.last_dir = (dx / len, dy / len);
                    }
                }
            }
        }

        self.prev_pos = self.pos;
        self.pos = Some(cur);
        self.prev_pressure = target_pressure;
    }
    /////////////////////////////////////////////////////////////////

    /// Is Stroke Catch-Up still pulling the brush towards a pointer that has stopped? Then the
    /// stroke needs time to keep passing (repeats of the last point) while the pointer is held.
    pub fn lagging(&self) -> bool {
        match (self.active() && self.cfg.catch_up && !self.cfg.pulled_string, self.pos, self.last_input) {
            (true, Some(pos), Some(last)) => (pos.x - last.x).abs() + (pos.y - last.y).abs() > 0.01,
            _ => false,
        }
    }

    /*/// The string between the brush and the pointer: `(brush, pointer)` document positions while
    /// smoothing holds the brush back from the last input point, `None` when they coincide (or
    /// smoothing is off). The canvas draws it, as Photoshop does for Pulled String Mode.
    pub fn string(&self) -> Option<([f64; 2], [f64; 2])> {
        let (pos, last) = (self.pos?, self.last_input?);
        (self.active() && (pos.x != last.x || pos.y != last.y)).then_some(([pos.x, pos.y], [last.x, last.y]))
    }*/

    /// Kill the flashing smoothing string completely!
    // This is only a suggestion, since it's super annoying when drawing
    // that's why I leave the original commented out, not deleted above
    pub fn string(&self) -> Option<([f64; 2], [f64; 2])> {
        None
    }


    /// End of stroke.
    pub fn finish(&mut self, out: &mut Vec<StrokePoint>) {
        let Some(cur) = self.pos else { return };

        let (ux, uy) = self.last_dir;
        let m = (ux * ux + uy * uy).sqrt();
        let (dirx, diry) = if m > 1e-4 { (ux / m, uy / m) } else { (1.0, 0.0) };

        let (dx, dy) = match self.last_input {
            Some(last) => (last.x - cur.x, last.y - cur.y),
            None => (0.0, 0.0),
        };

        let forward_lag = dx * dirx + dy * diry;

        // Guarantee a minimum 2.5px tail so the taper has room to breathe,
        // but cap it at 15px so it never creates a long hair.
        let tail_len = forward_lag.max(2.5).min(15.0);

        let target = StrokePoint {
            x: cur.x + dirx * tail_len,
            y: cur.y + diry * tail_len,
            pressure: 0.0, // Crucial for the taper
            ..cur
        };
        out.push(target);
        self.pos = Some(target);
    }
}


/// Smooth a whole point list with the brush's smoothing settings.
// UPDATED as well, naturally
pub fn smooth_points(cfg: &Smoothing, zoom: f32, pts: &[StrokePoint]) -> Vec<StrokePoint> {
    let mut s = Smoother::new(cfg, zoom, 20.0);
    let mut out = Vec::with_capacity(pts.len());
    for p in pts {
        s.push(*p, &mut out);
    }
    s.finish(&mut out);
    out
}

// ---------------------------------------------------------------------------
// Path walking
// ---------------------------------------------------------------------------

/// One spacing (or airbrush time) step along the path.
#[derive(Clone, Copy, Debug)]
pub struct StepInput {
    pub point: StrokePoint,
    /// Visual (counter-clockwise, y-up) direction of travel in radians.
    pub direction: f32,
    pub initial_direction: f32,
    /// Sequential step number (0 = first).
    pub step: u64,
}

/// Walks a polyline and emits steps every `step_len(point)` pixels, plus airbrush steps in time.
#[derive(Clone, Debug)]
pub struct PathWalker {
    last: Option<StrokePoint>,
    /// Path distance from `last` to the next step.
    next_at: f64,
    /// First point, emitted lazily once the initial direction is known.
    pending_first: Option<StrokePoint>,
    initial_dir: Option<f32>,
    dir: f32,
    step: u64,
    /// Airbrush interval in ms (None = off).
    interval: Option<f64>,
    time_acc: f64,
    /// Speed spacing (Spacing unchecked): one step per this many ms of stroke time.
    speed: Option<f64>,
    speed_acc: f64,
    /// Pixel-grid stepping for small aliased tips: the dab-centre offset inside a pixel (see
    /// [`PathWalker::pixel_grid`]).
    grid: Option<f64>,
}

impl PathWalker {
    pub fn new(build_up_interval_ms: Option<f64>) -> Self {
        Self {
            last: None,
            next_at: 0.0,
            pending_first: None,
            initial_dir: None,
            dir: 0.0,
            step: 0,
            interval: build_up_interval_ms,
            time_acc: 0.0,
            speed: None,
            speed_acc: 0.0,
            grid: None,
        }
    }

    /// Step pixel by pixel instead of by distance: one step per pixel along the segment's major
    /// axis, on the line between the pixels holding its ends, each at the pixel's dab centre
    /// (`offset` 0.5 for odd tip diameters, 0 for even ones, as `render::grid_center` snaps). A
    /// small aliased tip (the Pencil) then draws a clean 8-connected staircase: stepping by
    /// distance put two dabs in a row wherever the line crossed a pixel corner (#2139).
    pub fn pixel_grid(mut self, offset: f64) -> Self {
        self.grid = Some(offset);
        self
    }

    /// Emits the pixel steps from `a` to `p` (see [`PathWalker::pixel_grid`]). False, emitting
    /// nothing, when the span is not finite or implausibly long: distance spacing takes over.
    fn grid_steps(&mut self, a: &StrokePoint, p: &StrokePoint, offset: f64, out: &mut impl FnMut(StepInput)) -> bool {
        let cell = |v: f64| if offset > 0.0 { v.floor() } else { v.round() };
        let (x0, y0) = (cell(a.x), cell(a.y));
        let (dx, dy) = (cell(p.x) - x0, cell(p.y) - y0);
        let n = dx.abs().max(dy.abs());
        if !n.is_finite() || n > MAX_GRID_STEPS {
            return false;
        }
        let steps = n as u64;
        for i in 1..=steps {
            let i = i as f64;
            let mut q = lerp_pt(a, p, i / n);
            q.x = x0 + (dx * i / n).round() + offset;
            q.y = y0 + (dy * i / n).round() + offset;
            self.emit(q, out);
        }
        true
    }

    /// Space steps by stroke time instead of distance ([`SPEED_SPACING_MS`]): faster movement
    /// spreads them out. Segments without timestamps are spaced by distance.
    pub fn speed_spacing(mut self, interval_ms: f64) -> Self {
        self.speed = Some(interval_ms.max(0.1));
        self
    }

    fn emit(&mut self, point: StrokePoint, out: &mut impl FnMut(StepInput)) {
        let s = StepInput { point, direction: self.dir, initial_direction: self.initial_dir.unwrap_or(self.dir), step: self.step };
        self.step += 1;
        out(s);
    }

    pub fn push(&mut self, p: StrokePoint, step_len: &impl Fn(&StrokePoint) -> f64, out: &mut impl FnMut(StepInput)) {
        let Some(a) = self.last else {
            self.last = Some(p);
            self.pending_first = Some(p);
            self.next_at = step_len(&p);
            return;
        };
        let (dx, dy) = (p.x - a.x, p.y - a.y);
        let len = (dx * dx + dy * dy).sqrt();

        //////////////////////////////////////////////////////////////////
        if len > 1e-9 {
            self.dir = (-dy).atan2(dx) as f32;
            if self.initial_dir.is_none() {
                self.initial_dir = Some(self.dir);
            }
        }
        if let Some(f) = self.pending_first.take() {
            self.emit(f, out);
        }
        if len > 1e-9 {

            let dt = p.time - a.time;
            match self.speed.filter(|_| dt > 0.0 && dt.is_finite()) {
                None => {
                    if !self.grid.is_some_and(|g| self.grid_steps(&a, &p, g, out)) {
                        // First dab of this segment sits `next_at` pixels from `a` —
                        // it continues the previous segment's spacing.
                        while self.next_at <= len + 1e-9 {
                            let f = (self.next_at / len).min(1.0);
                            let q = lerp_pt(&a, &p, f);
                            self.emit(q, out);
                            self.next_at += step_len(&q).max(0.25);
                        }
                        // What is left over carries into the next segment.
                        self.next_at -= len;
                    }
                }
                /////////////////////////////////////////////////////////////////////

                Some(iv) => {
                    self.speed_acc += dt;
                    let max_n = (len / MIN_SPEED_STEP).ceil().max(1.0) as usize;
                    let mut n = 0;
                    while self.speed_acc >= iv && n < max_n {
                        self.speed_acc -= iv;
                        let f = ((dt - self.speed_acc) / dt).clamp(0.0, 1.0);
                        self.emit(lerp_pt(&a, &p, f), out);
                        n += 1;
                    }
                    if n == max_n {
                        self.speed_acc = self.speed_acc.rem_euclid(iv);
                    }
                }
            }
        }
        if let Some(iv) = self.interval {
            let dt = p.time - a.time;
            if dt > 0.0 && dt.is_finite() {
                if dt > iv * MAX_AIRBRUSH_DABS_PER_SEGMENT as f64 {
                    // A stalled or hostile timestamp must not replay an arbitrarily long backlog.
                    self.time_acc = 0.0;
                    self.emit(p, out);
                    self.last = Some(p);
                    return;
                }
                self.time_acc += dt;
                let mut n = 0;
                while self.time_acc >= iv && n < MAX_AIRBRUSH_DABS_PER_SEGMENT {
                    self.time_acc -= iv;
                    let f = ((dt - self.time_acc) / dt).clamp(0.0, 1.0);
                    self.emit(lerp_pt(&a, &p, f), out);
                    n += 1;
                }
            }
        }
        self.last = Some(p);
    }

    // UPDATED
    pub fn finish(&mut self, out: &mut impl FnMut(StepInput)) {
        if let Some(f) = self.pending_first.take() {
            self.emit(f, out);
        }
        // Always force-emit the final point so a residual spacing gap
        // never leaves the tip undrawn (the main source of “stepped” ends).
        if let Some(last) = self.last {
            let s = StepInput {
                point: last,
                direction: self.dir,
                initial_direction: self.initial_dir.unwrap_or(self.dir),
                step: self.step,
            };
            self.step += 1;
            out(s);
        }
    }
}

// ---------------------------------------------------------------------------
// Dynamics
// ---------------------------------------------------------------------------

/// Scalar control value in 0..1.
pub fn control_value(c: Control, fade_steps: u32, s: &StepInput) -> f32 {
    let p = &s.point;
    match c {
        Control::Off | Control::InitialDirection | Control::Direction => 1.0,
        Control::Fade => 1.0 - (s.step as f32 / fade_steps.max(1) as f32).min(1.0),
        Control::PenPressure => p.pressure.clamp(0.0, 1.0),
        // Upright pen = 1, flat pen = 0 (a mouse reports no tilt, so paints at full value).
        Control::PenTilt => 1.0 - (p.tilt_x.hypot(p.tilt_y) / 90.0).clamp(0.0, 1.0),
        Control::StylusWheel => p.wheel.clamp(0.0, 1.0),
        Control::Rotation => (p.rotation.rem_euclid(360.0)) / 360.0,
    }
}

/// A dynamic's control value passed through its own response curve ([`Dynamic::curve`]). Controls
/// with no 0..1 input (Off, the direction controls) read 1 and skip the curve.
pub fn dynamic_value(d: &Dynamic, s: &StepInput) -> f32 {
    let v = control_value(d.control, d.fade_steps, s);
    match d.control {
        Control::Off | Control::InitialDirection | Control::Direction => v,
        Control::Fade | Control::PenPressure | Control::PenTilt | Control::StylusWheel | Control::Rotation => d.curve.eval(v),
    }
}

/// Angle contributed by a control, in degrees (counter-clockwise).
pub fn control_angle(c: Control, fade_steps: u32, s: &StepInput) -> f32 {
    let p = &s.point;
    match c {
        Control::Off => 0.0,
        Control::Fade => 360.0 * (s.step as f32 / fade_steps.max(1) as f32).min(1.0),
        Control::PenPressure => 360.0 * p.pressure.clamp(0.0, 1.0),
        Control::PenTilt => {
            if p.tilt_x == 0.0 && p.tilt_y == 0.0 {
                0.0
            } else {
                (-p.tilt_y).atan2(p.tilt_x).to_degrees()
            }
        }
        Control::StylusWheel => 360.0 * p.wheel.clamp(0.0, 1.0),
        Control::Rotation => p.rotation,
        Control::InitialDirection => s.initial_direction.to_degrees(),
        Control::Direction => s.direction.to_degrees(),
    }
}

/// A 0..1 multiplier from a dynamic: control lerps from `minimum` to 1, jitter reduces randomly,
/// never below `minimum`.
fn scalar(d: &Dynamic, s: &StepInput, r: f32) -> f32 {
    if !d.is_active() {
        return 1.0;
    }
    let min = d.minimum.clamp(0.0, 1.0);
    let cv = dynamic_value(d, s);
    let v = (min + (1.0 - min) * cv) * (1.0 - d.jitter.clamp(0.0, 1.0) * r);
    v.max(min)
}

fn rgb_to_hsv(c: [f32; 3]) -> [f32; 3] {
    let mx = c[0].max(c[1]).max(c[2]);
    let mn = c[0].min(c[1]).min(c[2]);
    let d = mx - mn;
    let h = if d <= 0.0 {
        0.0
    } else if mx == c[0] {
        ((c[1] - c[2]) / d).rem_euclid(6.0)
    } else if mx == c[1] {
        (c[2] - c[0]) / d + 2.0
    } else {
        (c[0] - c[1]) / d + 4.0
    } / 6.0;
    [h, if mx > 0.0 { d / mx } else { 0.0 }, mx]
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h6 = h.rem_euclid(1.0) * 6.0;
    let i = h6.floor();
    let f = h6 - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    match i as i32 {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

/// Colour Dynamics for one dab (or once per stroke, with `index = 0`).
pub fn dynamic_color(b: &BrushSettings, s: &StepInput, seed: u64, index: u64) -> [f32; 4] {
    let cd = &b.color_dynamics;
    let (fg, bg) = (b.color, b.background);
    let r = |st: u64| rand01(seed, index, st);
    let base = if cd.fg_bg.control != Control::Off { 1.0 - dynamic_value(&cd.fg_bg, s) } else { 0.0 };
    let t = (base + (1.0 - base) * cd.fg_bg.jitter.clamp(0.0, 1.0) * r(stream::FG_BG)).clamp(0.0, 1.0);
    let rgb = [fg[0] + (bg[0] - fg[0]) * t, fg[1] + (bg[1] - fg[1]) * t, fg[2] + (bg[2] - fg[2]) * t];
    let [mut h, mut sat, mut v] = rgb_to_hsv(rgb);
    h += rand_signed(seed, index, stream::HUE) * cd.hue_jitter.clamp(0.0, 1.0) * 0.5;
    sat = (sat + rand_signed(seed, index, stream::SAT) * cd.saturation_jitter.clamp(0.0, 1.0)).clamp(0.0, 1.0);
    let pu = cd.purity.clamp(-1.0, 1.0);
    sat = if pu >= 0.0 { sat + (1.0 - sat) * pu } else { sat * (1.0 + pu) };
    v = (v + rand_signed(seed, index, stream::BRIGHT) * cd.brightness_jitter.clamp(0.0, 1.0)).clamp(0.0, 1.0);
    let o = hsv_to_rgb(h, sat, v);
    [o[0], o[1], o[2], fg[3] + (bg[3] - fg[3]) * t]
}

/// Base (pre-jitter) diameter at a step: size × size control (pressure etc.). Drives spacing.
pub fn base_diameter(b: &BrushSettings, s: &StepInput) -> f32 {
    let sd = &b.shape_dynamics;
    let mut f = 1.0;
    if sd.enabled && sd.size.control != Control::Off {
        let min = sd.size.minimum.clamp(0.0, 1.0);
        f = min + (1.0 - min) * dynamic_value(&sd.size, s);
    } else if b.pressure_size {
        // The options-bar shortcut is Photoshop's Shape Dynamics › Size › Pen Pressure with
        // the default curve. Skipping the curve here produced a linear response that ignored
        // the user's Preferences › Tools › pressure curve preference entirely — the shortcut
        // felt like a different brush than the same setting in the panel (#____).
        f = sd.size.curve.eval(s.point.pressure.clamp(0.0, 1.0));
    }
    b.size * f
}

/// Builds the primary dabs of a step (Shape Dynamics, Scattering, Transfer, Color, Texture depth).
#[derive(Clone, Debug)]
pub struct DabBuilder {
    pub brush: BrushSettings,
    pub seed: u64,
    /// Per-stroke colour (Color Dynamics with per-tip off), computed at the first step.
    stroke_color: Option<[f32; 4]>,
    next_index: u64,
    thin_run: u32 // >> ADDED
}

impl DabBuilder {
    pub fn new(brush: &BrushSettings) -> Self {
        Self { brush: brush.clone(), seed: brush.seed, stroke_color: None, next_index: 0, thin_run: 0 }
    }

    //////////////////////////////////////////////////////////
    // FIX 2
    pub fn step_len(&self, p: &StrokePoint) -> f64 {
        let s = StepInput { point: *p, direction: 0.0, initial_direction: 0.0, step: 0 };
        let d = base_diameter(&self.brush, &s);
        // Fixed minimum spacing. The 1.5px midpoint in `finish` guarantees
        // the tail is always drawn without breaking the main stroke's spacing.
        f64::from(d * self.brush.spacing.max(0.01)).max(0.5)
    }
    ///////////////////////////////////////////////////////////////

    pub fn build(&mut self, s: &StepInput, out: &mut Vec<Dab>) {
        let b = &self.brush;
        let seed = self.seed;
        let sd = &b.shape_dynamics;
        let sc = &b.scattering;
        let count = if sc.enabled {
            let n = sc.count.clamp(1, 16) as f32;
            let k = scalar(&sc.count_jitter, s, rand01(seed, s.step, stream::COUNT));
            ((n * k).round() as u32).clamp(1, 16)
        } else {
            1
        };
        let base_d = base_diameter(b, s);
        for _ in 0..count {
            let i = self.next_index;
            self.next_index += 1;
            let r = |st: u64| rand01(seed, i, st);
            // Shape.
            let mut diameter = base_d;
            let mut angle = b.angle;
            let mut roundness = b.roundness.clamp(0.01, 1.0);
            let (mut fx, mut fy) = (b.flip_x, b.flip_y);
            if sd.enabled {
                if sd.size.jitter > 0.0 {
                    let min = sd.size.minimum.clamp(0.0, 1.0);
                    let f = (1.0 - sd.size.jitter.clamp(0.0, 1.0) * r(stream::SIZE)).max(min);
                    diameter *= f;
                }
                angle +=
                    control_angle(sd.angle.control, sd.angle.fade_steps, s) + rand_signed(seed, i, stream::ANGLE) * sd.angle.jitter.clamp(0.0, 1.0) * 180.0;
                roundness = (roundness * scalar(&sd.roundness, s, r(stream::ROUNDNESS))).clamp(0.01, 1.0);
                if sd.flip_x_jitter && r(stream::FLIP_X) < 0.5 {
                    fx = !fx;
                }
                if sd.flip_y_jitter && r(stream::FLIP_Y) < 0.5 {
                    fy = !fy;
                }
            }
            // Tilt Scale and Brush Projection (Shape Dynamics).
            let (mut proj_angle, mut proj_scale) = (0.0, 1.0);
            if sd.enabled {
                let tilt = s.point.tilt_x.hypot(s.point.tilt_y).clamp(0.0, 90.0);
                if sd.size.control == Control::PenTilt && sd.tilt_scale > 0.0 {
                    let k = 1.0 - sd.tilt_scale.clamp(0.0, 2.0) * 0.5 * tilt / 90.0;
                    roundness = (roundness * k).clamp(0.01, 1.0);
                }
                if sd.brush_projection {
                    angle += s.point.rotation;
                    if tilt > 0.0 {
                        proj_angle = (-s.point.tilt_y).atan2(s.point.tilt_x);
                        proj_scale = tilt.to_radians().cos().max(0.05);
                    }
                }
            }

            //////////////////////////////////////////////////////////////
            // FIX
            let radius = diameter / 2.0;

            // Pure radius-based fade — no thin_run counter.
            // Thin tips quietly disappear instead of leaving long hairs or abrupt flats.
            let thin_fade = if b.aliased {
                1.0
            } else {
                let thin_r = (b.size * 0.07).clamp(1.2, 3.0);
                let fade_start = thin_r * 1.4;
                let fade_end   = thin_r * 0.2;   // <-- Changed from 0.45 to 0.2
                if radius >= fade_start {
                    1.0
                } else if radius <= fade_end {
                    0.0
                } else {
                    let t = (radius - fade_end) / (fade_start - fade_end);
                    t * t * (3.0 - 2.0 * t)
                }
            };
            if thin_fade <= 0.0 {
                continue;
            }

            // Scatter.
            let mut c = Point::new(s.point.x, s.point.y);
            if sc.enabled && sc.scatter.jitter > 0.0 {
                let cv = dynamic_value(&sc.scatter, s);
                let amt = f64::from(sc.scatter.jitter * cv * radius);
                let (ox, oy) = (
                    rand_signed(seed, i, stream::SCATTER_X) as f64 * amt,
                    rand_signed(seed, i, stream::SCATTER_Y) as f64 * amt,
                );
                if sc.both_axes {
                    c.x += ox;
                    c.y += oy;
                } else {
                    // Perpendicular to the direction of travel (y-down image space).
                    let (dx, dy) = (f64::from(s.direction.cos()), -f64::from(s.direction.sin()));
                    c.x += -dy * ox;
                    c.y += dx * ox;
                }
            }
            // Transfer.
            let tr = &b.transfer;
            let (mut opacity, mut flow) = (1.0, b.flow);
            if tr.enabled {
                opacity = scalar(&tr.opacity, s, r(stream::OPACITY));
                flow *= scalar(&tr.flow, s, r(stream::FLOW));
            }
            let (wet, mix) = if tr.enabled {
                (
                    scalar(&tr.wetness, s, r(stream::WET)),
                    scalar(&tr.mix, s, r(stream::MIX)),
                )
            } else {
                (1.0, 1.0)
            };
            if b.pressure_opacity && !(tr.enabled && tr.opacity.control != Control::Off) {
                opacity *= s.point.pressure.clamp(0.0, 1.0);
            }
            // Colour.
            let color = if b.color_dynamics.enabled {
                if b.color_dynamics.per_tip {
                    dynamic_color(b, s, seed, i)
                } else {
                    *self.stroke_color
                        .get_or_insert_with(|| dynamic_color(b, s, seed, 0))
                }
            } else {
                b.color
            };
            // Texture depth (per tip).
            let tx = &b.texture;
            let depth = if tx.enabled && tx.each_tip {
                tx.depth.clamp(0.0, 1.0) * scalar(&tx.depth_jitter, s, r(stream::DEPTH))
            } else {
                tx.depth.clamp(0.0, 1.0)
            };

            // Dabs under ~1.5px wide would be inflated to a 1px line by the rasterizer's
            // 0.5px radius floor: make them vanish instead.
            let fade_hi = (b.size * 0.25).clamp(0.5, 2.0);
            let fade_lo = fade_hi * 0.1; // <-- Changed from 0.25 to 0.1
            let t = ((radius - fade_lo) / (fade_hi - fade_lo)).clamp(0.0, 1.0);
            let area_scale = if b.aliased {
                1.0
            } else {
                t * t * (3.0 - 2.0 * t) * thin_fade
            };


            out.push(Dab {
                center: c,
                radius,
                alpha: (flow * area_scale).clamp(0.0, 1.0),
                angle: angle.to_radians(),
                roundness,
                flip_x: fx,
                flip_y: fy,
                opacity: (opacity * area_scale).clamp(0.0, 1.0),
                color,
                depth,
                index: i,
                proj_angle,
                proj_scale,
                wet,
                mix,
            });
        }
    }
}

/// Builds Dual Brush dabs (the second tip: size, spacing, scatter, count, flip).
#[derive(Clone, Debug)]
pub struct DualBuilder {
    pub brush: BrushSettings,
    next_index: u64,
}

impl DualBuilder {
    pub fn new(brush: &BrushSettings) -> Self {
        Self { brush: brush.clone(), next_index: 0 }
    }
    pub fn step_len(&self, _p: &StrokePoint) -> f64 {
        let d = &self.brush.dual_brush;
        f64::from(d.size.max(1.0) * d.spacing.max(0.01)).max(0.5)
    }
    pub fn build(&mut self, s: &StepInput, out: &mut Vec<Dab>) {
        let d = &self.brush.dual_brush;
        let seed = self.brush.seed ^ 0xD0A1;
        for _ in 0..d.count.clamp(1, 16) {
            let i = self.next_index;
            self.next_index += 1;
            let radius = (d.size / 2.0).max(0.5);
            let amt = f64::from(d.scatter * radius);
            let (ox, oy) = (rand_signed(seed, i, stream::DUAL_X) as f64 * amt, rand_signed(seed, i, stream::DUAL_Y) as f64 * amt);
            let mut c = Point::new(s.point.x, s.point.y);
            if d.both_axes {
                c.x += ox;
                c.y += oy;
            } else {
                let (dx, dy) = (f64::from(s.direction.cos()), -f64::from(s.direction.sin()));
                c.x += -dy * ox;
                c.y += dx * ox;
            }
            let flip = d.flip && rand01(seed, i, stream::DUAL_FLIP) < 0.5;
            out.push(Dab {
                center: c,
                radius,
                alpha: 1.0,
                angle: d.angle.to_radians(),
                roundness: d.roundness.clamp(0.01, 1.0),
                flip_x: flip,
                flip_y: false,
                opacity: 1.0,
                color: [0.0, 0.0, 0.0, 1.0],
                depth: 1.0,
                index: i,
                proj_angle: 0.0,
                proj_scale: 1.0,
                wet: 1.0,
                mix: 1.0,
            });
        }
    }
}

/// Apply Brush Pose to an input point.
pub fn apply_pose(b: &BrushSettings, mut p: StrokePoint) -> StrokePoint {
    let pose = &b.pose;
    if pose.enabled {
        if pose.override_tilt {
            p.tilt_x = pose.tilt_x;
            p.tilt_y = pose.tilt_y;
        }
        if pose.override_rotation {
            p.rotation = pose.rotation;
        }
        if pose.override_pressure {
            p.pressure = pose.pressure;
        }
    }
    p
}

/// Incremental dab generation for a whole brush (primary and optional dual dabs).
#[derive(Clone, Debug)]
pub struct DabGenerator {
    smoother: Smoother,
    walker: PathWalker,
    dual_walker: Option<PathWalker>,
    builder: DabBuilder,
    dual: Option<DualBuilder>,
    scratch: Vec<StrokePoint>,
}

impl DabGenerator {
    /// Does the stroke change with time while the pointer is held still (airbrush Build-up, or
    /// smoothing still catching up)? The canvas then keeps feeding it the held point.
    pub fn wants_time(&self) -> bool {
        self.walker.interval.is_some() || self.smoother.lagging()
    }

    /// See [`Smoother::string`].
    pub fn smoothing_string(&self) -> Option<([f64; 2], [f64; 2])> {
        self.smoother.string()
    }

    pub fn new(brush: &BrushSettings, zoom: f32) -> Self {
        let brush = brush.bounded_for_render();
        let interval = brush.build_up.then(|| 1000.0 / f64::from(brush.build_up_rate.clamp(0.1, 1000.0)));
        let dual_on = brush.dual_brush.enabled;

        let mut walker = if brush.spacing_enabled { PathWalker::new(interval) }
        else { PathWalker::new(interval).speed_spacing(SPEED_SPACING_MS) };

        // A small aliased tip whose dabs are at most a pixel apart steps pixel by pixel (#2139).
        if brush.aliased && brush.size * brush.spacing.max(0.01) <= 1.0 {
            walker = walker.pixel_grid(f64::from(crate::render::grid_center(0.0, 0.0, brush.size).0));
        }
        Self {
            smoother: Smoother::new(&brush.smoothing, zoom, brush.size), // <-- ADDED brush.size
            walker,
            dual_walker: dual_on.then(|| PathWalker::new(None)),
            builder: DabBuilder::new(&brush),
            dual: dual_on.then(|| DualBuilder::new(&brush)),
            scratch: Vec::new(),
        }
    }

    fn walk(&mut self, pts: &[StrokePoint], dabs: &mut Vec<Dab>, dual: &mut Vec<Dab>) {
        for p in pts {
            let p = apply_pose(&self.builder.brush, *p);
            let b = &self.builder;
            let len = |q: &StrokePoint| b.step_len(q);
            let mut steps = Vec::new();
            self.walker.push(p, &len, &mut |s| steps.push(s));
            for s in &steps {
                self.builder.build(s, dabs);
            }
            if let (Some(w), Some(db)) = (self.dual_walker.as_mut(), self.dual.as_mut()) {
                let mut steps = Vec::new();
                let len = |q: &StrokePoint| db.step_len(q);
                w.push(p, &len, &mut |s| steps.push(s));
                for s in &steps {
                    db.build(s, dual);
                }
            }
        }
    }

    /// Feed input points; appends the new primary and dual dabs.
    pub fn push(&mut self, pts: &[StrokePoint], dabs: &mut Vec<Dab>, dual: &mut Vec<Dab>) {
        let mut sm = std::mem::take(&mut self.scratch);
        sm.clear();
        for p in pts {
            self.smoother.push(*p, &mut sm);
        }
        self.walk(&sm, dabs, dual);
        self.scratch = sm;
    }

    /// End of stroke.
    pub fn finish(&mut self, dabs: &mut Vec<Dab>, dual: &mut Vec<Dab>) {
        let mut sm = Vec::new();
        self.smoother.finish(&mut sm);
        self.walk(&sm, dabs, dual);
        let mut steps = Vec::new();
        self.walker.finish(&mut |s| steps.push(s));
        for s in &steps {
            self.builder.build(s, dabs);
        }
        if let (Some(w), Some(db)) = (self.dual_walker.as_mut(), self.dual.as_mut()) {
            let mut steps = Vec::new();
            w.finish(&mut |s| steps.push(s));
            for s in &steps {
                db.build(s, dual);
            }
        }
    }
}
