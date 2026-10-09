//! Mouse wheel and trackpad navigation over the canvas (#293, #635), as in Photoshop:
//!
//! - The wheel scrolls up and down; ⇧ or ⌘/Ctrl + wheel scrolls sideways.
//! - ⌥/Alt + wheel zooms around the pointer in gentle steps, about 5% per wheel notch.
//! - Preferences › General › Zoom with Scroll Wheel swaps the two: the wheel zooms and
//!   ⌥/Alt + wheel scrolls.
//! - A trackpad pinch zooms around the pointer.
//!
//! [`configure`] makes egui fold ⌘/Ctrl + wheel into a sideways scroll as it does ⇧ + wheel, so
//! only a real pinch (egui's `Event::Zoom`, or a touch screen) reaches `zoom_delta`. macOS and
//! browsers deliver a pinch as its own event, but Windows hands a precision touchpad pinch to apps
//! as Ctrl + wheel in fractions of a notch: [`fold_legacy_pinch`] turns those back into a pinch
//! before egui sees them, so Ctrl + a notched mouse wheel scrolls sideways and the pinch still
//! zooms.
//!
//! egui smooths a wheel notch over several frames, so the Alt state is taken from the wheel
//! events themselves and kept until the next wheel event: releasing Alt while the last notch
//! is still settling must not turn the rest of it into a pan.

use egui::{Context, Event, Id, Modifiers, MouseWheelUnit, RawInput, Vec2};

/// Zoom factor of one wheel notch with ⌥/Alt held.
pub const ALT_NOTCH: f32 = 1.05;

/// A Ctrl + wheel event this soon (seconds) after a pinch step still belongs to the pinch, even
/// when it happens to be a whole notch.
const PINCH_GAP: f64 = 0.25;

/// What the wheel does to the view this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Wheel {
    /// Multiply the zoom by this factor around the pointer.
    Zoom(f32),
    /// Move the image by this many screen points (egui's content direction).
    Pan(Vec2),
}

/// Inputs of one frame's wheel gesture.
#[derive(Clone, Copy, Debug)]
pub struct Input {
    /// Smoothed scroll delta (points, content direction).
    pub scroll: Vec2,
    /// egui's pinch zoom factor (1 = none).
    pub zoom_delta: f32,
    /// ⌥/Alt was held for the wheel gesture in progress.
    pub alt: bool,
    /// Preferences › General › Zoom with Scroll Wheel.
    pub zoom_with_wheel: bool,
    /// Points per wheel notch (egui's `line_scroll_speed`).
    pub notch: f32,
}

/// The view change for one frame of wheel input, if any.
pub fn classify(i: Input) -> Option<Wheel> {
    if i.zoom_delta.is_finite() && i.zoom_delta > 0.0 && i.zoom_delta != 1.0 {
        return Some(Wheel::Zoom(i.zoom_delta));
    }
    if !(i.scroll.x.is_finite() && i.scroll.y.is_finite()) || i.scroll == Vec2::ZERO {
        return None;
    }
    let f = match (i.alt, i.zoom_with_wheel) {
        (true, false) => {
            // egui folds a scroll with Alt held into y. One notch (`notch` points) is 5%.
            let notch = if i.notch.is_finite() && i.notch > 0.0 { i.notch } else { 40.0 };
            ALT_NOTCH.powf((i.scroll.y + i.scroll.x) / notch)
        }
        // ⇧ and ⌘/Ctrl fold the wheel into x: those still scroll sideways.
        (false, true) if i.scroll.y != 0.0 => (i.scroll.y / 200.0).exp(),
        _ => return Some(Wheel::Pan(i.scroll)),
    };
    (f.is_finite() && f > 0.0 && f != 1.0).then_some(Wheel::Zoom(f))
}

/// Set egui's wheel modifiers (once, at setup): ⌘/Ctrl + wheel scrolls sideways like ⇧ + wheel
/// instead of zooming, everywhere in the app.
pub fn configure(ctx: &Context) {
    ctx.options_mut(|o| {
        o.input_options.zoom_modifier = Modifiers::NONE;
        o.input_options.horizontal_scroll_modifier = Modifiers::SHIFT | Modifiers::COMMAND;
    });
}

/// Turn a Windows touchpad pinch, which arrives as Ctrl + wheel in fractions of a notch, into
/// pinch events (`Event::Zoom`) at egui's Ctrl-scroll zoom speed (`notch` points per line,
/// `speed` per point). Ctrl + wheel in whole notches (a mouse wheel) is left to scroll sideways,
/// unless it comes within [`PINCH_GAP`] of a pinch step. `last` is the time of the previous pinch
/// step; returns the time of the latest one.
pub fn legacy_pinch_to_zoom(events: &mut [Event], now: f64, last: Option<f64>, notch: f32, speed: f32) -> Option<f64> {
    let ctrl_wheel = |e: &Event| match e {
        Event::MouseWheel { unit: MouseWheelUnit::Line, delta, modifiers, .. } if modifiers.ctrl => Some(*delta),
        _ => None,
    };
    let fraction = |d: Vec2| d.x.fract() != 0.0 || d.y.fract() != 0.0;
    let pinching = last.is_some_and(|t| (0.0..PINCH_GAP).contains(&(now - t))) || events.iter().filter_map(ctrl_wheel).any(fraction);
    if !pinching {
        return last;
    }
    let mut latest = last;
    for e in events.iter_mut() {
        if let Some(d) = ctrl_wheel(e) {
            let f = (speed * notch * (d.x + d.y)).exp();
            *e = Event::Zoom(if f.is_finite() && f > 0.0 { f } else { 1.0 });
            latest = Some(now);
        }
    }
    latest
}

fn pinch_id() -> Id {
    Id::new("pc-wheel-legacy-pinch")
}

/// [`legacy_pinch_to_zoom`] for one frame's raw input (`raw_input_hook`, on Windows).
pub fn fold_legacy_pinch(ctx: &Context, raw: &mut RawInput) {
    let (notch, speed) = ctx.options(|o| (o.input_options.line_scroll_speed, o.input_options.scroll_zoom_speed));
    let now = raw.time.unwrap_or_else(|| ctx.input(|i| i.time));
    let last = ctx.data(|d| d.get_temp::<f64>(pinch_id()));
    let latest = legacy_pinch_to_zoom(&mut raw.events, now, last, notch, speed);
    if latest != last
        && let Some(t) = latest
    {
        ctx.data_mut(|d| d.insert_temp(pinch_id(), t));
    }
}

fn alt_id() -> Id {
    Id::new("pc-wheel-alt")
}

/// Read this frame's wheel input. Call it every frame (hovered or not) so the Alt state of the
/// gesture follows the latest wheel event.
pub fn read(ctx: &Context, zoom_with_wheel: bool) -> Option<Wheel> {
    let notch = ctx.options(|o| o.input_options.line_scroll_speed);
    let (scroll, zoom_delta, latest) = ctx.input(|i| {
        let latest = i.events.iter().rev().find_map(|e| match e {
            Event::MouseWheel { modifiers, .. } => Some(modifiers.alt && !modifiers.command && !modifiers.ctrl),
            _ => None,
        });
        (i.smooth_scroll_delta, i.zoom_delta(), latest)
    });
    let alt = match latest {
        Some(a) => {
            ctx.data_mut(|d| d.insert_temp(alt_id(), a));
            a
        }
        None => ctx.data(|d| d.get_temp::<bool>(alt_id())).unwrap_or(false),
    };
    classify(Input { scroll, zoom_delta, alt, zoom_with_wheel, notch })
}

#[cfg(test)]
mod tests {
    use egui::TouchPhase;

    use super::*;

    fn input(scroll: Vec2, alt: bool) -> Input {
        Input { scroll, zoom_delta: 1.0, alt, zoom_with_wheel: false, notch: 40.0 }
    }

    fn wheel(delta: Vec2, modifiers: Modifiers) -> Event {
        Event::MouseWheel { unit: MouseWheelUnit::Line, delta, phase: TouchPhase::Move, modifiers }
    }

    const CTRL: Modifiers = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };
    const CMD: Modifiers = Modifiers { mac_cmd: true, command: true, ..Modifiers::NONE };

    /// Feed `events` through an egui context set up like the app's (`legacy`: the Windows pinch
    /// fold on) and add up what the wheel did to the view over the following second.
    fn gesture(frames: Vec<Vec<Event>>, zoom_with_wheel: bool, legacy: bool) -> (Vec2, f32) {
        let ctx = Context::default();
        configure(&ctx);
        let (mut pan, mut zoom) = (Vec2::ZERO, 1.0);
        let mut frames = frames.into_iter();
        for n in 0..60 {
            let mut raw =
                RawInput { time: Some(f64::from(n) / 60.0), predicted_dt: 1.0 / 60.0, events: frames.next().unwrap_or_default(), ..Default::default() };
            if legacy {
                fold_legacy_pinch(&ctx, &mut raw);
            }
            let mut out = ctx.run_ui(raw, |ui| match read(ui.ctx(), zoom_with_wheel) {
                Some(Wheel::Pan(d)) => pan += d,
                Some(Wheel::Zoom(f)) => zoom *= f,
                None => {}
            });
            out.textures_delta.clear();
        }
        (pan, zoom)
    }

    fn near(a: Vec2, b: Vec2) -> bool {
        (a - b).length() < 0.01
    }

    #[test]
    fn plain_scroll_pans() {
        assert_eq!(classify(input(Vec2::new(3.0, -40.0), false)), Some(Wheel::Pan(Vec2::new(3.0, -40.0))));
        assert_eq!(classify(input(Vec2::ZERO, false)), None);
    }

    #[test]
    fn alt_scroll_zooms_five_percent_per_notch() {
        let Some(Wheel::Zoom(f)) = classify(input(Vec2::new(0.0, 40.0), true)) else { panic!("zoom") };
        assert!((f - 1.05).abs() < 1e-5, "{f}");
        let Some(Wheel::Zoom(f)) = classify(input(Vec2::new(0.0, -40.0), true)) else { panic!("zoom") };
        assert!((f - 1.0 / 1.05).abs() < 1e-5, "{f}");
        // Smoothed over frames, the parts multiply back to one notch.
        let parts = [12.0, 16.0, 8.0, 4.0];
        let total: f32 = parts
            .iter()
            .map(|d| match classify(input(Vec2::new(0.0, *d), true)) {
                Some(Wheel::Zoom(f)) => f,
                _ => 1.0,
            })
            .product();
        assert!((total - 1.05).abs() < 1e-4, "{total}");
    }

    #[test]
    fn pinch_zooms() {
        let i = Input { zoom_delta: 1.2, ..input(Vec2::ZERO, false) };
        assert_eq!(classify(i), Some(Wheel::Zoom(1.2)));
        // The pinch wins over Alt.
        let i = Input { zoom_delta: 0.8, ..input(Vec2::new(0.0, 40.0), true) };
        assert_eq!(classify(i), Some(Wheel::Zoom(0.8)));
    }

    #[test]
    fn zoom_with_scroll_wheel_preference_swaps_wheel_and_alt_wheel() {
        let pref = |scroll, alt| Input { zoom_with_wheel: true, ..input(scroll, alt) };
        let Some(Wheel::Zoom(f)) = classify(pref(Vec2::new(0.0, 200.0), false)) else { panic!("zoom") };
        assert!((f - std::f32::consts::E).abs() < 1e-4);
        assert_eq!(classify(pref(Vec2::new(0.0, 40.0), true)), Some(Wheel::Pan(Vec2::new(0.0, 40.0))), "⌥ + wheel scrolls");
        assert_eq!(classify(pref(Vec2::new(40.0, 0.0), false)), Some(Wheel::Pan(Vec2::new(40.0, 0.0))), "sideways still scrolls");
    }

    #[test]
    fn each_modifier_maps_like_photoshop() {
        let notch = Vec2::new(0.0, 1.0);
        let one = |m: Modifiers| gesture(vec![vec![wheel(notch, m)]], false, false);
        let (pan, zoom) = one(Modifiers::NONE);
        assert!(near(pan, Vec2::new(0.0, 40.0)) && zoom == 1.0, "wheel scrolls vertically: {pan:?} {zoom}");
        for m in [Modifiers::SHIFT, CTRL, CMD, Modifiers::CTRL] {
            let (pan, zoom) = one(m);
            assert!(near(pan, Vec2::new(40.0, 0.0)) && zoom == 1.0, "{m:?} + wheel scrolls sideways: {pan:?} {zoom}");
        }
        let (pan, zoom) = one(Modifiers::ALT);
        assert!(pan == Vec2::ZERO && (zoom - 1.05).abs() < 1e-4, "⌥ + wheel zooms 5%: {pan:?} {zoom}");
        // Zoom with Scroll Wheel on: swapped.
        let (pan, zoom) = gesture(vec![vec![wheel(notch, Modifiers::ALT)]], true, false);
        assert!(near(pan, Vec2::new(0.0, 40.0)) && zoom == 1.0, "{pan:?} {zoom}");
        let (pan, zoom) = gesture(vec![vec![wheel(notch, Modifiers::NONE)]], true, false);
        assert!(pan == Vec2::ZERO && zoom > 1.1, "{pan:?} {zoom}");
        let (pan, zoom) = gesture(vec![vec![wheel(notch, CTRL)]], true, false);
        assert!(near(pan, Vec2::new(40.0, 0.0)) && zoom == 1.0, "⌘/Ctrl + wheel still scrolls sideways: {pan:?} {zoom}");
    }

    #[test]
    fn pinch_zooms_and_ctrl_wheel_pans() {
        // macOS trackpad and browsers: the pinch is its own event.
        let (pan, zoom) = gesture(vec![vec![Event::Zoom(1.1)], vec![Event::Zoom(1.1)]], false, false);
        assert!(pan == Vec2::ZERO && (zoom - 1.21).abs() < 1e-4, "{pan:?} {zoom}");
        // Windows: a touchpad pinch is Ctrl + wheel in fractions of a notch, a mouse wheel whole notches.
        let (pan, zoom) = gesture(vec![vec![wheel(Vec2::new(0.0, 0.25), CTRL)], vec![wheel(Vec2::new(0.0, 0.5), CTRL)]], false, true);
        let speed = egui::InputOptions::default().scroll_zoom_speed;
        assert!(pan == Vec2::ZERO && (zoom - (speed * 40.0 * 0.75).exp()).abs() < 1e-4, "the pinch zooms: {pan:?} {zoom}");
        let (pan, zoom) = gesture(vec![vec![wheel(Vec2::new(0.0, 1.0), CTRL)]], false, true);
        assert!(near(pan, Vec2::new(40.0, 0.0)) && zoom == 1.0, "Ctrl + mouse wheel scrolls sideways: {pan:?} {zoom}");
    }

    #[test]
    fn a_whole_notch_inside_a_pinch_stays_a_pinch() {
        let mut ev = [wheel(Vec2::new(0.0, 0.5), CTRL)];
        let t = legacy_pinch_to_zoom(&mut ev, 1.0, None, 40.0, 0.005);
        assert_eq!(t, Some(1.0));
        assert!(matches!(ev[0], Event::Zoom(f) if (f - 0.1f32.exp()).abs() < 1e-5), "{ev:?}");
        // The next step lands on a whole notch: still the pinch.
        let mut ev = [wheel(Vec2::new(0.0, -1.0), CTRL)];
        assert_eq!(legacy_pinch_to_zoom(&mut ev, 1.1, t, 40.0, 0.005), Some(1.1));
        assert!(matches!(ev[0], Event::Zoom(f) if f < 1.0), "{ev:?}");
        // Long after: a mouse wheel notch, untouched.
        let mut ev = [wheel(Vec2::new(0.0, -1.0), CTRL)];
        assert_eq!(legacy_pinch_to_zoom(&mut ev, 2.0, Some(1.1), 40.0, 0.005), Some(1.1));
        assert!(matches!(ev[0], Event::MouseWheel { .. }));
        // Without Ctrl nothing is a pinch.
        let mut ev = [wheel(Vec2::new(0.0, 0.5), Modifiers::NONE)];
        assert_eq!(legacy_pinch_to_zoom(&mut ev, 2.0, None, 40.0, 0.005), None);
        assert!(matches!(ev[0], Event::MouseWheel { .. }));
    }

    #[test]
    fn hostile_numbers_never_produce_a_bad_zoom() {
        for s in [f32::NAN, f32::INFINITY, -f32::INFINITY, 1e30, -1e30] {
            for alt in [false, true] {
                for notch in [0.0, -1.0, f32::NAN, 40.0] {
                    for zd in [f32::NAN, 0.0, -1.0, f32::INFINITY, 1.0] {
                        let i = Input { scroll: Vec2::new(0.0, s), zoom_delta: zd, alt, zoom_with_wheel: !alt, notch };
                        if let Some(Wheel::Zoom(f)) = classify(i) {
                            assert!(f.is_finite() && f > 0.0, "{f} from {i:?}");
                        }
                    }
                    let mut ev = [wheel(Vec2::new(s, 0.5), CTRL)];
                    let _ = legacy_pinch_to_zoom(&mut ev, f64::NAN, Some(f64::NAN), notch, 0.005);
                    let _ = legacy_pinch_to_zoom(&mut ev, 0.0, None, notch, f32::NAN);
                    if let Event::Zoom(f) = ev[0] {
                        assert!(f.is_finite() && f > 0.0, "{f}");
                    }
                }
            }
        }
    }
}
