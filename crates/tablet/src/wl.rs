//! Pure mapping of Wayland tablet-tool events (`zwp_tablet_tool_v2`) to pointer events and pen
//! samples. The protocol glue in `wayland.rs` feeds [`Raw`] events in protocol order; nothing
//! here touches a Wayland connection, so it is tested on every platform.
//!
//! The protocol groups a tool's events into frames: everything up to a `frame` event happened at
//! once. [`Tool::handle`] collects them and, at the frame, reports the pointer events (the surface
//! the pen entered, motion, the tip and the barrel buttons) and the pen sample (pressure, tilt,
//! rotation, eraser end). Which window a surface is, is the UI's to decide.
//!
//! [`argb8888`] and [`cursor_size`] prepare a UI cursor bitmap for the pen's cursor surface.

use crate::{Sample, Update};

/// `BTN_STYLUS` and `BTN_STYLUS2` (linux/input-event-codes.h): the pen's barrel buttons.
pub const BTN_STYLUS: u32 = 0x14b;
pub const BTN_STYLUS2: u32 = 0x14c;

/// The pressure axis' full scale (the protocol normalises it to 0..65535).
const PRESSURE_MAX: f64 = 65535.0;

/// One tablet-tool event, as far as this mapping needs it. Positions are surface-local
/// coordinates (logical pixels) of the surface the tool is over.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Raw {
    /// The tool came into proximity of one of the client's surfaces (its `wl_surface*` address,
    /// an identity only).
    ProximityIn {
        surface: u64,
    },
    ProximityOut,
    /// The tip (or eraser end) touched the surface.
    Down,
    Up,
    Motion {
        x: f64,
        y: f64,
    },
    /// 0..65535.
    Pressure(u32),
    /// Degrees, -90..90.
    Tilt {
        x: f64,
        y: f64,
    },
    /// Degrees, clockwise.
    Rotation(f64),
    /// A barrel button (`BTN_STYLUS`, `BTN_STYLUS2`, …) changed.
    Button {
        code: u32,
        pressed: bool,
    },
    /// The end of a group of events.
    Frame,
}

/// Which pen button a [`PenEvent::Button`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PenButton {
    /// The tip (or eraser end) touching the surface: the primary button.
    Tip,
    /// The first barrel button: the secondary (right) button.
    Barrel,
    /// The second barrel button: the middle button.
    Barrel2,
}

/// A pointer event, in surface-local logical pixels of the surface the pen last entered.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PenEvent {
    /// The pen came over `surface` (a `wl_surface*` address): the events up to the next `Leave`
    /// are that surface's.
    Enter {
        surface: u64,
    },
    Move {
        x: f64,
        y: f64,
    },
    Button {
        button: PenButton,
        pressed: bool,
        x: f64,
        y: f64,
    },
    /// The pen left the surface (out of proximity, or over another surface).
    Leave,
}

/// What one frame of tool events means.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameOut {
    pub events: Vec<PenEvent>,
    pub sample: Update,
}

/// A pointer change waiting for the frame that completes it. Button positions are resolved at
/// the frame (the protocol's frame is one hardware event, so a motion after a press in the same
/// frame is where the press happened), or when the pen leaves the surface.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Pending {
    Enter(u64),
    Button(PenButton, bool, Option<[f64; 2]>),
    Leave,
}

/// The state of one tablet tool (a pen; its eraser end is a separate tool on most tablets).
#[derive(Clone, Debug, Default)]
pub struct Tool {
    /// The tool is an eraser (its `type` event said so).
    pub eraser: bool,
    /// The tool reports pressure (its `capability` events said so). One that doesn't paints at
    /// full pressure while touching.
    pub pressure_axis: bool,
    /// The surface the tool is in proximity of.
    surface: Option<u64>,
    down: bool,
    barrel: bool,
    barrel2: bool,
    x: f64,
    y: f64,
    /// A motion arrived since the last frame, on the current surface.
    moved: bool,
    /// A motion arrived since the tool came over the current surface: `x`, `y` are its.
    placed: bool,
    /// The tip touched before the tool's first motion on the surface: the press waits for it (a
    /// press at a stale position would click whatever is there).
    unplaced_down: bool,
    /// Barrel buttons (1, 2) pressed before that motion: like the tip, they wait for it.
    unplaced_barrel: [bool; 2],
    pressure: f64,
    tilt_x: f64,
    tilt_y: f64,
    rotation: f64,
    pending: Vec<Pending>,
    /// What the UI was told as of the last frame: the surface the pen is over and the buttons
    /// held (tip, barrel, barrel 2), so an overlong frame can be reduced to the net change.
    ui_surface: Option<u64>,
    ui_held: [bool; 3],
    /// The buttons (tip, barrel, barrel 2) whose press reached the UI and whose release hasn't:
    /// while another tool is the one the UI follows, only their releases go in ([`Self::deliver`]).
    delivered: [bool; 3],
    left: bool,
    changed: bool,
}

/// Cap on changes queued within one frame: past it they are reduced to the net change since the
/// last frame (a compositor that never sends `frame` must not grow memory without bound).
const MAX_PENDING: usize = 64;

impl Tool {
    /// Take one event; at a `Frame` return what changed (`None` when nothing did).
    pub fn handle(&mut self, raw: Raw) -> Option<FrameOut> {
        match raw {
            Raw::ProximityIn { surface } => {
                // A missed proximity-out (the compositor moved the pen straight to another
                // surface) still leaves the old surface first.
                self.leave();
                self.surface = Some(surface);
                self.moved = false;
                self.placed = false;
                self.push(Pending::Enter(surface));
            }
            Raw::ProximityOut => self.leave(),
            Raw::Down => {
                if self.surface.is_some() && !self.down {
                    self.down = true;
                    if self.placed {
                        self.push(Pending::Button(PenButton::Tip, true, None));
                    } else {
                        self.unplaced_down = true;
                    }
                }
            }
            Raw::Up => {
                // A press still waiting for a position is dropped with its release.
                if std::mem::take(&mut self.down) && !std::mem::take(&mut self.unplaced_down) {
                    self.push(Pending::Button(PenButton::Tip, false, None));
                }
            }
            Raw::Motion { x, y } => {
                if x.is_finite() && y.is_finite() {
                    self.x = x;
                    self.y = y;
                    self.moved |= self.surface.is_some();
                    self.placed |= self.surface.is_some();
                    self.changed = true;
                    if std::mem::take(&mut self.unplaced_down) {
                        self.push(Pending::Button(PenButton::Tip, true, None));
                    }
                    for (i, button) in [PenButton::Barrel, PenButton::Barrel2].into_iter().enumerate() {
                        if std::mem::take(&mut self.unplaced_barrel[i]) {
                            *[&mut self.barrel, &mut self.barrel2][i] = true;
                            self.push(Pending::Button(button, true, None));
                        }
                    }
                }
            }
            Raw::Pressure(p) => {
                self.pressure = f64::from(p) / PRESSURE_MAX;
                self.changed = true;
            }
            Raw::Tilt { x, y } => {
                self.tilt_x = x;
                self.tilt_y = y;
                self.changed = true;
            }
            Raw::Rotation(r) => {
                self.rotation = r;
                self.changed = true;
            }
            Raw::Button { code, pressed } => {
                let (button, held, unplaced) = match code {
                    BTN_STYLUS => (PenButton::Barrel, &mut self.barrel, &mut self.unplaced_barrel[0]),
                    BTN_STYLUS2 => (PenButton::Barrel2, &mut self.barrel2, &mut self.unplaced_barrel[1]),
                    _ => return None,
                };
                // A press needs the pen over a surface, at a known position there (never a click
                // at a stale one): before the first motion it waits for one, and a release
                // before then drops it.
                if pressed {
                    if self.surface.is_none() || *held || *unplaced {
                        return None;
                    }
                    if self.placed {
                        *held = true;
                        self.push(Pending::Button(button, true, None));
                    } else {
                        *unplaced = true;
                    }
                } else if !std::mem::take(unplaced) && std::mem::take(held) {
                    self.push(Pending::Button(button, false, None));
                }
            }
            Raw::Frame => return self.frame(),
        }
        None
    }

    /// Queue a change for the next frame, reduced to the net change past [`MAX_PENDING`].
    fn push(&mut self, p: Pending) {
        self.pending.push(p);
        self.changed = true;
        if self.pending.len() > MAX_PENDING {
            self.reconcile();
        }
    }

    /// Replace the queued changes by the net change from what the UI was last told to now:
    /// release what it holds that isn't held (leaving a surface it is no longer over), then enter
    /// the current surface and press what is held now.
    fn reconcile(&mut self) {
        let held = [self.down && !self.unplaced_down, self.barrel, self.barrel2];
        let buttons = [PenButton::Tip, PenButton::Barrel, PenButton::Barrel2];
        let at = Some([self.x, self.y]);
        let same = self.ui_surface.is_some() && self.ui_surface == self.surface;
        let mut p = Vec::new();
        if self.ui_surface.is_some() {
            for (i, b) in buttons.iter().enumerate() {
                if self.ui_held[i] && (!same || !held[i]) {
                    p.push(Pending::Button(*b, false, at));
                }
            }
            if !same {
                p.push(Pending::Leave);
            }
        }
        if let Some(s) = self.surface {
            if !same {
                p.push(Pending::Enter(s));
            }
            for (i, b) in buttons.iter().enumerate() {
                if held[i] && (!same || !self.ui_held[i]) {
                    p.push(Pending::Button(*b, true, None));
                }
            }
        }
        self.pending = p;
    }

    /// Out of proximity: release whatever is held so the UI never sees a stuck button, then leave.
    fn leave(&mut self) {
        if self.surface.is_none() {
            return;
        }
        let at = [self.x, self.y];
        if std::mem::take(&mut self.unplaced_down) {
            self.down = false;
        }
        self.unplaced_barrel = [false; 2];
        for p in &mut self.pending {
            if let Pending::Button(_, _, pos @ None) = p {
                *pos = Some(at);
            }
        }
        for (held, button) in [(&mut self.down, PenButton::Tip), (&mut self.barrel, PenButton::Barrel), (&mut self.barrel2, PenButton::Barrel2)] {
            if std::mem::take(held) {
                self.pending.push(Pending::Button(button, false, Some(at)));
            }
        }
        self.surface = None;
        self.placed = false;
        self.moved = false;
        self.left = true;
        self.push(Pending::Leave);
    }

    /// The frame event: the queued changes as pointer events, at the frame's position, and the
    /// sample (`None` when nothing changed).
    fn frame(&mut self) -> Option<FrameOut> {
        if !std::mem::take(&mut self.changed) {
            return None;
        }
        let (x, y) = (self.x, self.y);
        let pending = std::mem::take(&mut self.pending);
        // The frame's one motion, on the surface entered last (where the frame's motion is).
        let move_at = pending.iter().rposition(|p| matches!(p, Pending::Enter(_))).map_or(0, |i| i + 1);
        let moved = std::mem::take(&mut self.moved) && self.surface.is_some();
        let mut events = Vec::with_capacity(pending.len() + 1);
        for (i, p) in pending.into_iter().enumerate() {
            if i == move_at && moved {
                events.push(PenEvent::Move { x, y });
            }
            events.push(match p {
                Pending::Enter(surface) => PenEvent::Enter { surface },
                Pending::Button(button, pressed, pos) => {
                    let [x, y] = pos.unwrap_or([x, y]);
                    PenEvent::Button { button, pressed, x, y }
                }
                Pending::Leave => PenEvent::Leave,
            });
        }
        if moved && !events.iter().skip(move_at).any(|e| matches!(e, PenEvent::Move { .. })) {
            events.insert(move_at.min(events.len()), PenEvent::Move { x, y });
        }
        self.ui_surface = self.surface;
        self.ui_held = [self.down && !self.unplaced_down, self.barrel, self.barrel2];
        let left = std::mem::take(&mut self.left);
        let sample = if self.surface.is_some() {
            // Pressure 0 while hovering, like the X11 and macOS readers; a tool without the
            // pressure axis touches at full pressure.
            let pressure = match (self.down && !self.unplaced_down, self.pressure_axis) {
                (false, _) => 0.0,
                (true, true) => self.pressure as f32,
                (true, false) => 1.0,
            };
            Update::Set(Some(Sample { pressure, tilt_x: self.tilt_x as f32, tilt_y: self.tilt_y as f32, rotation: self.rotation as f32, eraser: self.eraser }))
        } else if left {
            Update::Set(None)
        } else {
            Update::Keep
        };
        Some(FrameOut { events, sample })
    }
}

/// Two tools over the tablet at once (two pens, or a pen and a mouse tool): the UI follows the
/// one in proximity last, the active one.
impl Tool {
    /// What of `out`, one of this tool's frames, goes into the UI: all of it while the tool is the
    /// `active` one; otherwise only the releases of presses the UI got from it (a press never
    /// loses its release, and a release never ends another tool's press), without the sample.
    pub fn deliver(&mut self, out: FrameOut, active: bool) -> Option<FrameOut> {
        let delivered = &mut self.delivered;
        let events: Vec<_> = out
            .events
            .into_iter()
            .filter(|e| match *e {
                PenEvent::Button { button, pressed, .. } => {
                    let sent = &mut delivered[button as usize];
                    let keep = active || (!pressed && *sent);
                    if keep {
                        *sent = pressed;
                    }
                    keep
                }
                _ => active,
            })
            .collect();
        if active { Some(FrameOut { events, sample: out.sample }) } else { (!events.is_empty()).then_some(FrameOut { events, sample: Update::Keep }) }
    }

    /// Another tool came into proximity, and the UI follows it from now on: this one leaves the
    /// UI, releasing what it pressed there (in the window it pressed it in, before the other
    /// tool enters its own), though it stays in proximity. `None` when it wasn't over a surface.
    pub fn hand_off(&mut self) -> Option<FrameOut> {
        self.ui_surface?;
        let (x, y) = (self.x, self.y);
        let buttons = [PenButton::Tip, PenButton::Barrel, PenButton::Barrel2];
        let mut events: Vec<_> = buttons
            .into_iter()
            .filter(|b| std::mem::take(&mut self.delivered[*b as usize]))
            .map(|button| PenEvent::Button { button, pressed: false, x, y })
            .collect();
        events.push(PenEvent::Leave);
        Some(FrameOut { events, sample: Update::Set(None) })
    }
}

/// The largest cursor bitmap side the pen takes (the UI's own cap on Linux is 255).
pub const MAX_CURSOR_SIDE: u16 = 1024;

/// A UI cursor bitmap (straight-alpha RGBA, rows top to bottom) as `wl_shm` `ARGB8888`:
/// premultiplied, little-endian `0xAARRGGBB`. `None` when the size is 0, too large or doesn't
/// match the data.
pub fn argb8888(rgba: &[u8], width: u16, height: u16) -> Option<Vec<u8>> {
    if width == 0 || height == 0 || width > MAX_CURSOR_SIDE || height > MAX_CURSOR_SIDE {
        return None;
    }
    let len = usize::from(width).checked_mul(usize::from(height))?.checked_mul(4)?;
    if rgba.len() != len {
        return None;
    }
    let premultiply = |c: u8, a: u8| ((u16::from(c) * u16::from(a) + 127) / 255) as u8;
    let (pixels, _) = rgba.as_chunks::<4>();
    Some(pixels.iter().flat_map(|&[r, g, b, a]| [premultiply(b, a), premultiply(g, a), premultiply(r, a), a]).collect())
}

/// The cursor surface's size and hotspot in surface-local (logical) pixels for a bitmap of
/// `width`×`height` physical pixels at the window's `scale`: the bitmap is shown at its physical
/// size. A broken scale counts as 1; sizes are at least 1.
pub fn cursor_size(width: u16, height: u16, hotspot: [u16; 2], scale: f32) -> ([i32; 2], [i32; 2]) {
    let scale = if scale.is_finite() && scale > 0.0 { f64::from(scale) } else { 1.0 };
    let logical = |v: u16, min: i32| ((f64::from(v) / scale).round() as i32).max(min);
    ([logical(width, 1), logical(height, 1)], [logical(hotspot[0], 0), logical(hotspot[1], 0)])
}

/// Without viewporter, the cursor buffer's integer scale for a window `scale`: that scale when it
/// is a whole number (1..=8) dividing the bitmap, else 1 (the cursor then shows larger on a
/// scaled output, as winit's own).
pub fn buffer_scale(width: u16, height: u16, scale: f32) -> u16 {
    let k = scale.round();
    let k = if (scale - k).abs() < 0.01 && (1.0..=8.0).contains(&k) { k as u16 } else { 1 };
    if width.is_multiple_of(k) && height.is_multiple_of(k) { k } else { 1 }
}

/// A 15×15 crosshair (black on a white outline), hotspot in the middle: the pen's cursor when the
/// compositor has no cursor shapes. Straight-alpha RGBA.
pub fn crosshair() -> (Vec<u8>, u16, u16, [u16; 2]) {
    const N: usize = 15;
    const C: usize = N / 2;
    let mut rgba = vec![0u8; N * N * 4];
    let line = |x: usize, y: usize| x == C || y == C;
    for y in 0..N {
        for x in 0..N {
            let near = |dx: isize, dy: isize| {
                let (nx, ny) = (x as isize + dx, y as isize + dy);
                (0..N as isize).contains(&nx) && (0..N as isize).contains(&ny) && line(nx as usize, ny as usize)
            };
            let px = if line(x, y) {
                [0, 0, 0, 255]
            } else if near(-1, 0) || near(1, 0) || near(0, -1) || near(0, 1) {
                [255, 255, 255, 255]
            } else {
                continue;
            };
            if let Some(p) = rgba.get_mut((y * N + x) * 4..(y * N + x) * 4 + 4) {
                p.copy_from_slice(&px);
            }
        }
    }
    (rgba, N as u16, N as u16, [C as u16; 2])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(tool: &mut Tool, raws: &[Raw]) -> Vec<FrameOut> {
        raws.iter().filter_map(|r| tool.handle(*r)).collect()
    }

    fn pen() -> Tool {
        Tool { pressure_axis: true, ..Default::default() }
    }

    fn press(button: PenButton, pressed: bool, x: f64, y: f64) -> PenEvent {
        PenEvent::Button { button, pressed, x, y }
    }

    /// A hover, a press, a move and a lift map to pointer events with their pressure and tilt.
    #[test]
    fn a_stroke_moves_presses_and_releases_with_pressure() {
        let mut t = pen();
        let out = run(
            &mut t,
            &[
                Raw::ProximityIn { surface: 7 },
                Raw::Motion { x: 10.0, y: 20.0 },
                Raw::Frame,
                Raw::Down,
                Raw::Pressure(32768),
                Raw::Tilt { x: 15.0, y: -5.0 },
                Raw::Frame,
                Raw::Motion { x: 12.0, y: 21.0 },
                Raw::Pressure(65535),
                Raw::Frame,
                Raw::Up,
                Raw::Pressure(0),
                Raw::Frame,
            ],
        );
        assert_eq!(out.len(), 4);
        assert_eq!(out[0].events, vec![PenEvent::Enter { surface: 7 }, PenEvent::Move { x: 10.0, y: 20.0 }]);
        assert!(matches!(out[0].sample, Update::Set(Some(s)) if s.pressure == 0.0), "hovering is pressure 0");
        assert_eq!(out[1].events, vec![press(PenButton::Tip, true, 10.0, 20.0)]);
        let Update::Set(Some(s)) = out[1].sample else { panic!("{:?}", out[1].sample) };
        assert!((s.pressure - 0.5).abs() < 0.001 && s.tilt_x == 15.0 && s.tilt_y == -5.0);
        assert_eq!(out[2].events, vec![PenEvent::Move { x: 12.0, y: 21.0 }]);
        assert!(matches!(out[2].sample, Update::Set(Some(s)) if s.pressure == 1.0));
        assert_eq!(out[3].events, vec![press(PenButton::Tip, false, 12.0, 21.0)]);
    }

    /// A press lands where the frame ends, whether its motion came before or after it.
    #[test]
    fn a_press_is_where_the_frame_ends_whatever_the_event_order() {
        for order in [[Raw::Down, Raw::Motion { x: 100.0, y: 200.0 }], [Raw::Motion { x: 100.0, y: 200.0 }, Raw::Down]] {
            let mut t = pen();
            run(&mut t, &[Raw::ProximityIn { surface: 1 }, Raw::Motion { x: 10.0, y: 20.0 }, Raw::Frame]);
            let out = run(&mut t, &[order[0], order[1], Raw::Frame]);
            assert_eq!(out[0].events, vec![PenEvent::Move { x: 100.0, y: 200.0 }, press(PenButton::Tip, true, 100.0, 200.0)], "{order:?}");
            let out = run(&mut t, &[Raw::Button { code: BTN_STYLUS, pressed: true }, Raw::Up, Raw::Motion { x: 5.0, y: 6.0 }, Raw::Frame]);
            assert_eq!(
                out[0].events,
                vec![PenEvent::Move { x: 5.0, y: 6.0 }, press(PenButton::Barrel, true, 5.0, 6.0), press(PenButton::Tip, false, 5.0, 6.0)]
            );
        }
    }

    /// A touch before the pen's first motion on a surface presses where that motion is, never
    /// at a stale position; one that lifts first presses nothing.
    #[test]
    fn a_tip_press_waits_for_the_pens_position() {
        let mut t = pen();
        run(&mut t, &[Raw::ProximityIn { surface: 1 }, Raw::Motion { x: 50.0, y: 60.0 }, Raw::Frame, Raw::ProximityOut, Raw::Frame]);
        let out = run(&mut t, &[Raw::ProximityIn { surface: 2 }, Raw::Down, Raw::Frame]);
        assert_eq!(out[0].events, vec![PenEvent::Enter { surface: 2 }], "no press at the other surface's position");
        let out = run(&mut t, &[Raw::Motion { x: 3.0, y: 4.0 }, Raw::Frame]);
        assert_eq!(out[0].events, vec![PenEvent::Move { x: 3.0, y: 4.0 }, press(PenButton::Tip, true, 3.0, 4.0)]);
        let out = run(&mut t, &[Raw::Up, Raw::Frame]);
        assert_eq!(out[0].events, vec![press(PenButton::Tip, false, 3.0, 4.0)]);

        // Touch and lift (or leave) without a motion: no click anywhere.
        for end in [Raw::Up, Raw::ProximityOut] {
            let mut t = pen();
            let out = run(&mut t, &[Raw::ProximityIn { surface: 2 }, Raw::Down, Raw::Frame, end, Raw::Frame]);
            assert!(out.iter().flat_map(|f| &f.events).all(|e| !matches!(e, PenEvent::Button { .. })), "{end:?}: {out:?}");
        }
    }

    /// Another tool's frame while the UI follows the last tool in proximity: only the releases of
    /// presses the UI got from it, no sample.
    #[test]
    fn a_background_tools_frame_keeps_only_releases_of_its_delivered_presses() {
        let mut t = pen();
        let down = FrameOut { events: vec![PenEvent::Enter { surface: 1 }, press(PenButton::Tip, true, 1.0, 1.0)], sample: Update::Set(None) };
        assert_eq!(t.deliver(down.clone(), true), Some(down));
        let f = FrameOut {
            events: vec![
                PenEvent::Move { x: 1.0, y: 1.0 },
                press(PenButton::Tip, false, 1.0, 1.0),
                press(PenButton::Barrel, true, 1.0, 1.0),
                press(PenButton::Barrel, false, 1.0, 1.0),
                PenEvent::Leave,
            ],
            sample: Update::Set(None),
        };
        assert_eq!(t.deliver(f.clone(), false), Some(FrameOut { events: vec![press(PenButton::Tip, false, 1.0, 1.0)], sample: Update::Keep }));
        // The tip's release went in once: a second one, like the barrel's, ended no press it made.
        assert_eq!(t.deliver(f, false), None);
        let hover = FrameOut { events: vec![PenEvent::Enter { surface: 1 }, PenEvent::Move { x: 1.0, y: 1.0 }], sample: Update::Set(None) };
        assert_eq!(t.deliver(hover, false), None);
    }

    /// When another tool comes into proximity, the UI sees this one leave, releasing what it
    /// pressed there; its own release later goes nowhere, so it can't end the new tool's press.
    #[test]
    fn a_tool_handed_off_releases_its_presses_and_leaves() {
        let mut t = pen();
        assert_eq!(t.hand_off(), None, "never over a surface");
        let out = run(
            &mut t,
            &[Raw::ProximityIn { surface: 2 }, Raw::Motion { x: 3.0, y: 4.0 }, Raw::Down, Raw::Button { code: BTN_STYLUS, pressed: true }, Raw::Frame],
        );
        assert!(t.deliver(out[0].clone(), true).is_some());
        assert_eq!(
            t.hand_off(),
            Some(FrameOut {
                events: vec![press(PenButton::Tip, false, 3.0, 4.0), press(PenButton::Barrel, false, 3.0, 4.0), PenEvent::Leave],
                sample: Update::Set(None)
            })
        );
        let out = run(&mut t, &[Raw::Up, Raw::ProximityOut, Raw::Frame]);
        assert_eq!(t.deliver(out[0].clone(), false), None, "its releases already went in");
    }

    #[test]
    fn leaving_releases_held_buttons_and_reports_a_mouse() {
        let mut t = pen();
        run(&mut t, &[Raw::ProximityIn { surface: 7 }, Raw::Motion { x: 1.0, y: 2.0 }, Raw::Down, Raw::Button { code: BTN_STYLUS, pressed: true }, Raw::Frame]);
        let out = run(&mut t, &[Raw::ProximityOut, Raw::Frame]);
        assert_eq!(out[0].events, vec![press(PenButton::Tip, false, 1.0, 2.0), press(PenButton::Barrel, false, 1.0, 2.0), PenEvent::Leave]);
        assert_eq!(out[0].sample, Update::Set(None));
    }

    #[test]
    fn barrel_buttons_map_and_other_codes_are_ignored() {
        let mut t = pen();
        let out = run(
            &mut t,
            &[
                Raw::ProximityIn { surface: 7 },
                Raw::Motion { x: 3.0, y: 4.0 },
                Raw::Button { code: BTN_STYLUS2, pressed: true },
                Raw::Button { code: 0x110, pressed: true },
                Raw::Button { code: BTN_STYLUS2, pressed: true },
                Raw::Frame,
            ],
        );
        assert_eq!(out[0].events, vec![PenEvent::Enter { surface: 7 }, PenEvent::Move { x: 3.0, y: 4.0 }, press(PenButton::Barrel2, true, 3.0, 4.0)]);
    }

    /// A barrel press before the pen's first position on the surface waits for it, like the tip;
    /// one released or taken out of proximity before then never reaches the UI, and one outside
    /// proximity is ignored.
    #[test]
    fn a_barrel_press_waits_for_the_pens_position() {
        let (b, b2) = (|pressed| Raw::Button { code: BTN_STYLUS, pressed }, |pressed| Raw::Button { code: BTN_STYLUS2, pressed });
        let mut t = pen();
        let out = run(&mut t, &[Raw::ProximityIn { surface: 7 }, b(true), b2(true), Raw::Frame]);
        assert_eq!(out[0].events, vec![PenEvent::Enter { surface: 7 }], "no press before the position is known");
        let out = run(&mut t, &[Raw::Motion { x: 5.0, y: 6.0 }, Raw::Frame]);
        assert_eq!(out[0].events, vec![PenEvent::Move { x: 5.0, y: 6.0 }, press(PenButton::Barrel, true, 5.0, 6.0), press(PenButton::Barrel2, true, 5.0, 6.0)]);
        let out = run(&mut t, &[b(false), Raw::ProximityOut, Raw::Frame]);
        assert_eq!(out[0].events, vec![press(PenButton::Barrel, false, 5.0, 6.0), press(PenButton::Barrel2, false, 5.0, 6.0), PenEvent::Leave]);

        // Pressed and released, or pressed and out of proximity, before any motion: nothing.
        let mut t = pen();
        let out = run(&mut t, &[Raw::ProximityIn { surface: 7 }, b(true), b(false), Raw::Frame]);
        assert_eq!(out[0].events, vec![PenEvent::Enter { surface: 7 }]);
        let out = run(&mut t, &[b(true), Raw::ProximityOut, Raw::ProximityIn { surface: 8 }, Raw::Motion { x: 1.0, y: 2.0 }, Raw::Frame]);
        assert_eq!(out[0].events, vec![PenEvent::Leave, PenEvent::Enter { surface: 8 }, PenEvent::Move { x: 1.0, y: 2.0 }]);

        // Outside proximity a press is ignored, and so is its release.
        let mut t = pen();
        assert!(run(&mut t, &[b(true), Raw::Frame]).is_empty());
        let out = run(&mut t, &[Raw::ProximityIn { surface: 1 }, Raw::Motion { x: 1.0, y: 2.0 }, b(false), Raw::Frame]);
        assert_eq!(out[0].events, vec![PenEvent::Enter { surface: 1 }, PenEvent::Move { x: 1.0, y: 2.0 }]);
    }

    /// Entering another surface without proximity-out leaves the first, releasing its buttons;
    /// nothing comes before proximity.
    #[test]
    fn moving_between_surfaces_leaves_the_first_and_events_outside_proximity_are_dropped() {
        let mut t = pen();
        assert!(
            run(&mut t, &[Raw::Motion { x: 5.0, y: 5.0 }, Raw::Down, Raw::Frame]).iter().all(|f| f.events.is_empty() && f.sample == Update::Keep),
            "nothing before proximity"
        );
        let out = run(
            &mut t,
            &[
                Raw::ProximityIn { surface: 1 },
                Raw::Motion { x: 5.0, y: 5.0 },
                Raw::Down,
                Raw::ProximityIn { surface: 2 },
                Raw::Motion { x: 3.0, y: 4.0 },
                Raw::Frame,
            ],
        );
        assert_eq!(
            out[0].events,
            vec![
                PenEvent::Enter { surface: 1 },
                press(PenButton::Tip, true, 5.0, 5.0),
                press(PenButton::Tip, false, 5.0, 5.0),
                PenEvent::Leave,
                PenEvent::Enter { surface: 2 },
                PenEvent::Move { x: 3.0, y: 4.0 },
            ],
            "a missed proximity-out still releases and leaves"
        );
        assert!(matches!(out[0].sample, Update::Set(Some(_))));
        let out = run(&mut t, &[Raw::ProximityOut, Raw::ProximityOut, Raw::Frame]);
        assert_eq!(out[0].events, vec![PenEvent::Leave], "one Leave for a repeated proximity-out");
        assert_eq!(out[0].sample, Update::Set(None));
    }

    #[test]
    fn the_eraser_tool_reports_its_end_and_hostile_values_are_contained() {
        let mut t = Tool { eraser: true, ..pen() };
        let out = run(
            &mut t,
            &[
                Raw::ProximityIn { surface: 7 },
                Raw::Motion { x: f64::NAN, y: 3.0 },
                Raw::Motion { x: 1.0, y: 2.0 },
                Raw::Down,
                Raw::Pressure(u32::MAX),
                Raw::Frame,
            ],
        );
        let Update::Set(Some(s)) = out[0].sample else { panic!() };
        assert!(s.eraser);
        assert_eq!(
            out[0].events,
            vec![PenEvent::Enter { surface: 7 }, PenEvent::Move { x: 1.0, y: 2.0 }, press(PenButton::Tip, true, 1.0, 2.0)],
            "NaN motion is dropped"
        );
        assert_eq!(s.sanitized().pressure, 1.0);
    }

    /// A compositor that never sends `frame` keeps the queue bounded, reduced to the net change.
    #[test]
    fn a_frame_that_never_comes_is_reduced_to_the_net_change() {
        let mut t = pen();
        run(&mut t, &[Raw::ProximityIn { surface: 1 }, Raw::Motion { x: 1.0, y: 1.0 }, Raw::Down, Raw::Frame]);
        for _ in 0..10_000 {
            t.handle(Raw::Up);
            t.handle(Raw::Down);
            t.handle(Raw::Motion { x: 1.0, y: 1.0 });
            t.handle(Raw::ProximityIn { surface: 2 });
            t.handle(Raw::ProximityIn { surface: 1 });
        }
        assert!(t.pending.len() <= MAX_PENDING + 1);
        t.handle(Raw::Motion { x: 1.0, y: 1.0 });
        t.handle(Raw::Down);
        let out = t.handle(Raw::Frame).unwrap();
        assert!(out.events.len() <= MAX_PENDING + 2, "{}", out.events.len());
        // The UI held the tip; the pen ends with it down: tip events alternate release, press, …
        // and end pressed, so the UI never sees a double press or a stuck button.
        let tip: Vec<bool> =
            out.events.iter().filter_map(|e| if let PenEvent::Button { button: PenButton::Tip, pressed, .. } = e { Some(*pressed) } else { None }).collect();
        assert!(tip.iter().enumerate().all(|(i, p)| *p == (i % 2 == 1)) && tip.last() == Some(&true), "{tip:?}");
        assert_eq!(out.events.iter().rev().find_map(|e| if let PenEvent::Enter { surface } = e { Some(*surface) } else { None }), Some(1));
        // Buttons spammed without frames stay bounded too.
        for _ in 0..10_000 {
            t.handle(Raw::Button { code: BTN_STYLUS, pressed: true });
            t.handle(Raw::Button { code: BTN_STYLUS, pressed: false });
        }
        assert!(t.pending.len() <= MAX_PENDING + 1);
    }

    /// A tool without the pressure axis touches at full pressure; a pressure pen at 0 until it reports.
    #[test]
    fn a_tool_without_pressure_touches_at_full_pressure() {
        let mut t = Tool::default();
        let out = run(&mut t, &[Raw::ProximityIn { surface: 1 }, Raw::Motion { x: 1.0, y: 1.0 }, Raw::Frame, Raw::Down, Raw::Frame]);
        assert!(matches!(out[0].sample, Update::Set(Some(s)) if s.pressure == 0.0), "hovering");
        assert!(matches!(out[1].sample, Update::Set(Some(s)) if s.pressure == 1.0), "touching: {:?}", out[1].sample);
        let mut t = pen();
        let out = run(&mut t, &[Raw::ProximityIn { surface: 1 }, Raw::Motion { x: 1.0, y: 1.0 }, Raw::Down, Raw::Frame]);
        assert!(matches!(out[0].sample, Update::Set(Some(s)) if s.pressure == 0.0), "a pressure pen's first contact before its pressure event");
    }

    #[test]
    fn a_frame_without_changes_reports_nothing() {
        let mut t = Tool::default();
        assert_eq!(t.handle(Raw::Frame), None);
    }

    /// Cursor bitmaps convert to premultiplied ARGB8888, and bad sizes are refused.
    #[test]
    fn cursor_bitmaps_premultiply_into_argb8888_and_reject_bad_sizes() {
        let px = argb8888(&[255, 128, 0, 128, 10, 20, 30, 255], 2, 1).unwrap();
        assert_eq!(px, vec![0, 64, 128, 128, 30, 20, 10, 255]);
        assert_eq!(argb8888(&[0; 8], 1, 1), None, "data doesn't match the size");
        assert_eq!(argb8888(&[], 0, 0), None);
        assert_eq!(argb8888(&[0; 4], MAX_CURSOR_SIDE + 1, 1), None);
        assert_eq!(cursor_size(64, 32, [32, 16], 2.0), ([32, 16], [16, 8]));
        assert_eq!(cursor_size(3, 3, [1, 1], 1.5), ([2, 2], [1, 1]));
        // A broken window scale counts as 1, so the brush outline never vanishes or blows up.
        for scale in [0.0, -2.0, f32::NAN, f32::INFINITY] {
            assert_eq!(cursor_size(64, 32, [32, 16], scale), ([64, 32], [32, 16]), "{scale}");
            assert_eq!(buffer_scale(64, 32, scale), 1, "{scale}");
        }
        assert_eq!(cursor_size(1, 1, [0, 0], 4.0), ([1, 1], [0, 0]), "never empty");
        for scale in [0.0, -2.0, f32::NAN, f32::INFINITY] {
            assert_eq!(cursor_size(10, 10, [5, 5], scale), ([10, 10], [5, 5]));
        }
        assert_eq!(buffer_scale(64, 32, 2.0), 2);
        assert_eq!(buffer_scale(63, 32, 2.0), 1, "doesn't divide the bitmap");
        assert_eq!(buffer_scale(64, 64, 1.5), 1, "fractional");
        for scale in [0.0, -2.0, 9.0, f32::NAN, f32::INFINITY] {
            assert_eq!(buffer_scale(72, 72, scale), 1);
        }
    }

    #[test]
    fn the_fallback_crosshair_is_a_valid_cursor() {
        let (rgba, w, h, hotspot) = crosshair();
        assert!(argb8888(&rgba, w, h).is_some());
        let at = |x: usize, y: usize| &rgba[(y * usize::from(w) + x) * 4..][..4];
        assert_eq!(at(7, 7), [0, 0, 0, 255], "the hotspot is on the lines");
        assert_eq!(at(8, 0), [255, 255, 255, 255], "white outline");
        assert_eq!(at(0, 0)[3], 0, "transparent elsewhere");
        assert_eq!(hotspot, [7, 7]);
    }
}
