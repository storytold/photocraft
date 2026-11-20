//! Ctrl/Cmd + mouse-wheel over a document canvas (#5363).
//!
//! Craft owns wheel classification and has no modifier for this preference. A whole mouse notch is
//! rewritten, before craft's input hook, into the gesture craft already zooms by ×1.1 at once: Alt +
//! wheel, or a plain wheel when Zoom with Scroll Wheel is on. A fractional Ctrl + wheel stays a pinch.

use egui::{Context, Event, Modifiers, MouseWheelUnit, Pos2, RawInput, Vec2};

use crate::PhotocraftApp;
use crate::canvas::document_display::CanvasHost;

/// How long after a pinch step a whole Ctrl + wheel notch still belongs to that pinch, matching craft's fold.
const PINCH_CONTINUES: f64 = 0.25;

/// Rewrites this frame's wheel when Ctrl/Cmd + a whole notch should zoom.
pub(crate) fn apply(app: &PhotocraftApp, ctx: &Context, raw: &mut RawInput) {
    if !app.session.prefs().general.ctrl_wheel_zooms() || app.dialog_open() {
        return;
    }
    let over_canvas = pointer_at(ctx, raw).is_some_and(|at| over_document_canvas(app, at));
    let mut last_pinch = ctx.data(|data| data.get_temp::<f64>(pinch_id()));
    let now = raw.time.unwrap_or_else(|| ctx.input(|input| input.time));
    rewrite_ctrl_notches(&mut raw.events, app.session.prefs().general.zoom_with_scroll_wheel, over_canvas, Some(now), &mut last_pinch);
    if let Some(at) = last_pinch {
        ctx.data_mut(|data| data.insert_temp(pinch_id(), at));
    }
}

fn pinch_id() -> egui::Id {
    egui::Id::new("pc-ctrl-wheel-pinch")
}

/// The pointer this frame will use: the latest move in `raw`, else where it was last frame.
fn pointer_at(ctx: &Context, raw: &RawInput) -> Option<Pos2> {
    let mut at = ctx.input(|input| input.pointer.hover_pos());
    for event in &raw.events {
        match event {
            Event::PointerMoved(pos) | Event::PointerButton { pos, .. } | Event::Touch { pos, .. } => at = Some(*pos),
            _ => {}
        }
    }
    at
}

/// Whether `at` lies on a document canvas shown last pass, in the main area or a document window.
fn over_document_canvas(app: &PhotocraftApp, at: Pos2) -> bool {
    let ctx = app.ctx();
    let hit = |host: CanvasHost| craft::ui::canvas::ToolHost::shown_space(ctx, host.canvas_id()).is_some_and(|space| space.view.rect.contains(at));
    let documents = app.session.documents().len();
    if (0..documents).any(|idx| hit(CanvasHost::main(idx))) {
        return true;
    }
    app.ui.document_windows.floating.iter().any(|(group, _)| {
        let idx = group.shown.or_else(|| group.documents.first().copied()).and_then(|id| crate::canvas::tabs::document_index(app, id));
        idx.is_some_and(|idx| hit(CanvasHost { idx, window: Some(group.id) }))
    })
}

/// Turns whole Ctrl/Cmd mouse notches into craft's zoom gesture when `over_canvas`.
///
/// `zoom_with_wheel` swaps craft's wheel, so the notch has to arrive with no modifiers. Otherwise it arrives as Alt.
/// A fractional notch, and a whole notch within [`PINCH_CONTINUES`] of one, is left for craft's pinch fold
/// (Windows and Linux). `last_pinch` is that fold's clock.
pub(crate) fn rewrite_ctrl_notches(events: &mut [Event], zoom_with_wheel: bool, over_canvas: bool, now: Option<f64>, last_pinch: &mut Option<f64>) {
    for event in events.iter_mut() {
        let Event::MouseWheel { unit, delta, modifiers, .. } = event else { continue };
        let (unit, delta, modifiers) = (*unit, *delta, *modifiers);
        if legacy_pinch_step(unit, delta, modifiers) {
            if now.is_some() {
                *last_pinch = now;
            }
            continue;
        }
        if !over_canvas || !ctrl_mouse_notch(unit, delta, modifiers) {
            continue;
        }
        let continues = cfg!(not(target_os = "macos")) && now.zip(*last_pinch).is_some_and(|(now, last)| now - last <= PINCH_CONTINUES);
        if continues {
            *last_pinch = now;
            continue;
        }
        if let Event::MouseWheel { modifiers, .. } = event {
            // Alt zooms when the plain wheel pans. With Zoom with Scroll Wheel the plain wheel is the zoom.
            modifiers.alt = !zoom_with_wheel;
            modifiers.ctrl = false;
            modifiers.command = false;
            modifiers.mac_cmd = false;
        }
    }
}

/// Ctrl or Cmd + a whole mouse-wheel notch: line units, no fraction, no Alt, no Shift.
fn ctrl_mouse_notch(unit: MouseWheelUnit, delta: Vec2, modifiers: Modifiers) -> bool {
    let command = modifiers.command || modifiers.ctrl || modifiers.mac_cmd;
    unit == MouseWheelUnit::Line
        && command
        && !modifiers.alt
        && !modifiers.shift
        && delta.x.fract() == 0.0
        && delta.y.fract() == 0.0
        && (delta.x != 0.0 || delta.y != 0.0)
}

/// A Ctrl + wheel step craft folds into a pinch before classification. Whole notches are not steps.
fn legacy_pinch_step(unit: MouseWheelUnit, delta: Vec2, modifiers: Modifiers) -> bool {
    if cfg!(target_os = "macos") || !modifiers.ctrl || modifiers.alt || modifiers.shift {
        return false;
    }
    match unit {
        MouseWheelUnit::Line => delta.x.fract() != 0.0 || delta.y.fract() != 0.0,
        MouseWheelUnit::Point => cfg!(target_os = "linux"),
        MouseWheelUnit::Page => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{MouseWheelSource, TouchPhase};

    fn notch(delta: Vec2, modifiers: Modifiers) -> Event {
        Event::MouseWheel { source: MouseWheelSource::Unknown, unit: MouseWheelUnit::Line, delta, phase: TouchPhase::Move, modifiers }
    }

    fn mods_of(event: &Event) -> Modifiers {
        match event {
            Event::MouseWheel { modifiers, .. } => *modifiers,
            _ => Modifiers::NONE,
        }
    }

    fn command(modifiers: Modifiers) -> bool {
        modifiers.command || modifiers.ctrl || modifiers.mac_cmd
    }

    #[test]
    fn whole_ctrl_notch_becomes_alt_zoom_and_a_fraction_stays() {
        let held = [
            Modifiers { ctrl: true, command: true, ..Modifiers::NONE },
            Modifiers { command: true, mac_cmd: true, ..Modifiers::NONE },
            Modifiers { ctrl: true, ..Modifiers::NONE },
        ];
        for modifiers in held {
            let mut events = [notch(Vec2::new(0.0, 1.0), modifiers)];
            let mut last = None;
            rewrite_ctrl_notches(&mut events, false, true, Some(1.0), &mut last);
            let got = mods_of(&events[0]);
            assert!(got.alt && !command(got) && !got.shift, "{modifiers:?} -> {got:?}");
            assert!(last.is_none());
        }

        let mut events = [notch(Vec2::new(0.0, 0.25), Modifiers { ctrl: true, command: true, ..Modifiers::NONE })];
        let mut last = None;
        rewrite_ctrl_notches(&mut events, false, true, Some(1.0), &mut last);
        assert!(command(mods_of(&events[0])), "a fraction stays a pinch");
    }

    #[test]
    fn zoom_with_scroll_wheel_clears_the_notch_instead_of_adding_alt() {
        let mut events = [notch(Vec2::new(0.0, -1.0), Modifiers { ctrl: true, command: true, ..Modifiers::NONE })];
        let mut last = None;
        rewrite_ctrl_notches(&mut events, true, true, Some(1.0), &mut last);
        assert_eq!(mods_of(&events[0]), Modifiers::NONE);
    }

    #[test]
    fn shift_and_off_canvas_notches_stay_sideways() {
        let shift = Modifiers { ctrl: true, command: true, shift: true, ..Modifiers::NONE };
        let mut events = [notch(Vec2::new(0.0, 1.0), shift)];
        let mut last = None;
        rewrite_ctrl_notches(&mut events, false, true, Some(1.0), &mut last);
        assert!(mods_of(&events[0]).shift && command(mods_of(&events[0])));

        let mut events = [notch(Vec2::new(0.0, 1.0), Modifiers { ctrl: true, ..Modifiers::NONE })];
        rewrite_ctrl_notches(&mut events, false, false, Some(1.0), &mut last);
        assert!(command(mods_of(&events[0])) && !mods_of(&events[0]).alt);
    }

    #[test]
    fn a_whole_notch_just_after_a_pinch_stays_a_pinch() {
        let ctrl = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };
        let mut events = [notch(Vec2::new(0.0, 0.25), ctrl), notch(Vec2::new(0.0, 1.0), ctrl)];
        let mut last = None;
        rewrite_ctrl_notches(&mut events, false, true, Some(2.0), &mut last);
        if cfg!(target_os = "macos") {
            assert!(mods_of(&events[1]).alt, "macOS does not fold a whole notch into the pinch");
        } else {
            assert!(command(mods_of(&events[1])) && !mods_of(&events[1]).alt, "same frame as a fraction");
        }

        let mut events = [notch(Vec2::new(0.0, 1.0), ctrl)];
        let mut last = Some(2.0);
        rewrite_ctrl_notches(&mut events, false, true, Some(2.2), &mut last);
        if cfg!(target_os = "macos") {
            assert!(mods_of(&events[0]).alt, "macOS has no legacy pinch fold");
        } else {
            assert!(command(mods_of(&events[0])) && !mods_of(&events[0]).alt);
        }

        let mut events = [notch(Vec2::new(0.0, 1.0), ctrl)];
        let mut last = Some(1.0);
        rewrite_ctrl_notches(&mut events, false, true, Some(1.0 + PINCH_CONTINUES + 0.01), &mut last);
        assert!(mods_of(&events[0]).alt && !command(mods_of(&events[0])));
    }
}
