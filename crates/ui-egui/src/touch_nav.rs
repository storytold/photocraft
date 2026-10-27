//! Two-or-more-finger canvas navigation.
//!
//! Fingers placed together on the canvas select pan and pinch-zoom until all fingers lift.
//! A second finger can promote an owned live stroke to navigation; the points received so far are
//! committed before the stroke is cut off.
//! Pinch zoom itself is applied by [`crate::wheel_nav`] (egui's multi-touch zoom factor).
//!
//! A grace after a lone Touch Start lets the first finger use its tool immediately while leaving a
//! window in which a staggered second finger can promote the gesture to navigation. The window
//! stays open while that first finger is stationary; moving it starts the short timeout. If
//! promotion happens, the provisional stroke is aborted; if the first finger lifts first, its
//! normal release commits the stroke.

use egui::{Context, Event, Id, Pos2, TouchPhase, Vec2};

use crate::PhotocraftApp;

/// Time to distinguish a staggered pinch from a single-finger tool, independent of frame rate.
const PINCH_GRACE_SECONDS: f64 = 0.05;
/// Ignore small contact jitter while deciding whether the first finger has moved.
const PINCH_GRACE_MOVE_THRESHOLD: f32 = 2.0;

#[derive(Clone, Copy, Default)]
struct Grace {
    /// The first contact's position while it is waiting for a possible second finger.
    origin: Option<Pos2>,
    /// The timeout starts only after the first contact leaves `origin`.
    started_at: Option<f64>,
    /// The first finger owns the tool until every contact is lifted.
    tool_owned: bool,
    /// One-shot fallback if the browser omitted the initial compatibility press.
    deferred_press: bool,
}

fn grace_id() -> Id {
    Id::new("pc-touch-pinch-grace")
}

fn with_grace(ctx: &Context, f: impl FnOnce(&mut Grace)) {
    ctx.data_mut(|d| {
        let mut g = d.get_temp::<Grace>(grace_id()).unwrap_or_default();
        f(&mut g);
        d.insert_temp(grace_id(), g);
    });
}

/// True when the current touch frame cancelled the primary contact and tool input must be ignored.
pub fn tools_suppressed(ctx: &Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(cancelled_id()).unwrap_or(false))
}

fn cancelled_id() -> Id {
    Id::new("pc-touch-pointer-cancelled")
}

/// The first finger already owns this gesture, including while later fingers remain down.
pub fn tool_owned(ctx: &Context) -> bool {
    ctx.data(|d| d.get_temp::<Grace>(grace_id()).is_some_and(|g| g.tool_owned))
}

/// Consume the one-shot deferred press armed when grace ends with a finger still down.
pub fn take_deferred_press(ctx: &Context) -> bool {
    let mut taken = false;
    with_grace(ctx, |g| {
        taken = g.deferred_press;
        g.deferred_press = false;
    });
    taken
}

/// Navigation is active this frame if it already was, or a second finger just landed. The lift
/// frame must still count as navigation so a pointer-up cannot commit the cancelled tool.
pub fn active_this_frame(was_nav: bool, touches: usize) -> bool {
    was_nav || touches >= 2
}

/// Stay in navigation while any finger is still down.
///
/// A leftover Primary after the last finger lifts must *not* keep the latch: that previously
/// delayed or swallowed a rapid second pinch/pan on touch screens.
pub fn next_latch(nav_this_frame: bool, touches: usize, pointer_down: bool) -> bool {
    let _ = pointer_down;
    touches >= 2 || (nav_this_frame && touches > 0)
}

/// How many fingers are down, even before egui has a pointer baseline for multi-touch deltas.
pub fn touch_count(ctx: &Context) -> usize {
    ctx.data(|d| d.get_temp::<usize>(contact_count_id()))
        .unwrap_or_else(|| ctx.input(|i| i.multi_touch().map_or(usize::from(i.any_touches()), |m| m.num_touches)))
}

fn contact_count_id() -> Id {
    Id::new("pc-touch-contact-count")
}

/// Origin eligibility belongs to a canvas, and remains fixed until all contacts are lifted.
#[derive(Clone, Default)]
struct Origins {
    contacts: Vec<(egui::TouchDeviceId, egui::TouchId)>,
    blocked: bool,
}

impl Origins {
    fn frame(&mut self, events: &[Event], free: impl Fn(Pos2) -> bool) -> bool {
        let mut eligible = !self.contacts.is_empty() && !self.blocked;
        for event in events {
            if let Event::Touch { device_id, id, phase, pos, .. } = event {
                let key = (*device_id, *id);
                match phase {
                    TouchPhase::Start => {
                        if self.contacts.is_empty() {
                            self.blocked = false;
                        }
                        if !self.contacts.contains(&key) {
                            self.contacts.push(key);
                            self.blocked |= !free(*pos);
                        }
                        eligible = !self.blocked;
                    }
                    TouchPhase::Move => {}
                    TouchPhase::End | TouchPhase::Cancel => self.contacts.retain(|k| *k != key),
                }
            }
        }
        eligible && !self.blocked
    }
}

/// Both fingers must start on this canvas's unobstructed surface. Moving in from a panel or
/// dialog cannot acquire it; a gesture that started here can continue outside the canvas.
pub fn eligible(ctx: &Context, response: &egui::Response, under_dialog: bool) -> bool {
    let id = response.id.with("touch-origins");
    let mut origins = ctx.data(|d| d.get_temp::<Origins>(id)).unwrap_or_default();
    let events = ctx.input(|i| i.events.clone());
    let eligible = origins.frame(&events, |p| {
        response.rect.contains(p)
            && if under_dialog {
                crate::dialogs::free_position(ctx, response.rect, p)
            } else {
                ctx.layer_id_at(p).is_none_or(|layer| layer == response.layer_id)
            }
    });
    ctx.data_mut(|d| {
        d.insert_temp(contact_count_id(), origins.contacts.len());
        d.insert_temp(id, origins);
    });
    eligible
}

fn clear_grace(ctx: &Context) {
    ctx.data_mut(|d| {
        d.remove::<Grace>(grace_id());
    });
}

/// Update the latch / grace, finish or drop in-progress tools when navigating, and return whether
/// this frame is multi-touch navigation-only (centroid pan / pinch).
///
/// The first finger is allowed to start a tool during grace. A second finger in that window takes
/// over navigation and aborts the provisional tool. Once the timeout expires, the first finger
/// owns a live stroke; a later second finger commits the received points before navigation takes
/// over. Lifting while stationary commits the normal release.
pub fn update(app: &mut PhotocraftApp, ctx: &Context, eligible: bool) -> bool {
    if !eligible {
        app.touch_nav = false;
        clear_grace(ctx);
        return false;
    }
    let touches = touch_count(ctx);
    let (pointer_down, now) = ctx.input(|i| (i.pointer.any_down(), i.time));
    let (lone_start, start_pos) = ctx.input(|i| {
        let start = i.events.iter().find_map(|event| match event {
            Event::Touch { phase: TouchPhase::Start, pos, .. } => Some(*pos),
            _ => None,
        });
        (touches == 1 && start.is_some(), start)
    });

    if touches == 0 && !pointer_down {
        // All contacts and the Primary are up — drop grace. If Primary is still down but the
        // harness/OS omitted a touch snapshot this frame, keep ticking so the fallback press works.
        clear_grace(ctx);
    } else if lone_start && !app.touch_nav {
        with_grace(ctx, |g| {
            if !g.tool_owned && g.origin.is_none() {
                g.origin = start_pos;
            }
        });
    }

    // Keep the grace open while the first finger is still at its origin. Once it moves, start the
    // short timeout; an idle browser repaints when the wait ends so the tool press is not lost.
    let origin = ctx.data(|d| d.get_temp::<Grace>(grace_id()).and_then(|g| g.origin));
    let first_moved = origin.is_some_and(|origin| {
        ctx.input(|i| {
            i.events.iter().any(|event| match event {
                Event::Touch { phase: TouchPhase::Move, pos, .. } => origin.distance(*pos) > PINCH_GRACE_MOVE_THRESHOLD,
                _ => false,
            })
        })
    });
    let mut remaining = None;
    with_grace(ctx, |g| {
        if g.started_at.is_none() && first_moved {
            g.started_at = Some(now);
        }
        if let Some(started_at) = g.started_at {
            let elapsed = now - started_at;
            if (0.0..PINCH_GRACE_SECONDS).contains(&elapsed) {
                remaining = Some(PINCH_GRACE_SECONDS - elapsed);
            } else {
                g.started_at = None;
                g.tool_owned = true;
                g.deferred_press = pointer_down;
            }
        }
    });
    if let Some(remaining) = remaining {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(remaining));
    }

    if tools_suppressed(ctx) {
        abort_tools(app, ctx);
    }

    // A second contact always promotes the gesture to navigation. Before tool ownership this
    // discards the provisional stroke; after ownership it commits the received live stroke first.
    let promote_to_nav = touches >= 2 && !app.touch_nav;
    if promote_to_nav && tool_owned(ctx) {
        crate::canvas::finish_touch_stroke(app);
    }
    let nav = app.touch_nav || active_this_frame(false, touches);
    if nav {
        abort_tools(app, ctx);
        clear_grace(ctx); // multi-touch owns the gesture; no deferred paint after
    }
    app.touch_nav = nav && next_latch(nav, touches, pointer_down);
    nav
}

/// Centroid pan of a two-finger gesture (points). `None` when only one finger remains.
pub fn pan_delta(ctx: &Context) -> Option<Vec2> {
    ctx.input(|i| i.multi_touch().map(|m| m.translation_delta)).filter(|d| d.is_finite())
}

#[cfg(any(target_arch = "wasm32", test))]
type TouchKey = (egui::TouchDeviceId, egui::TouchId);

#[cfg(any(target_arch = "wasm32", test))]
#[derive(Clone, Default)]
struct PointerContacts {
    contacts: Vec<(TouchKey, Pos2)>,
    primary: Option<TouchKey>,
}

#[cfg(any(target_arch = "wasm32", test))]
impl PointerContacts {
    fn cancel(&mut self, events: &mut Vec<Event>, modifiers: egui::Modifiers) -> bool {
        let cancelled = self.primary.is_some();
        for (key, pos) in self.contacts.drain(..) {
            events.push(Event::Touch { device_id: key.0, id: key.1, phase: TouchPhase::Cancel, pos, force: None });
            if self.primary == Some(key) {
                events.push(Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers });
                events.push(Event::PointerGone);
            }
        }
        self.primary = None;
        cancelled
    }
}

/// eframe's web backend emits a compatibility Primary press/release for every finger. Rebuild
/// those events from the first contact so a later finger cannot restart or finish its stroke.
/// The pointer is never transferred to a remaining finger; all contacts must lift first.
/// Returns true when that first contact was cancelled (the caller must abort its tool).
#[cfg(any(target_arch = "wasm32", test))]
pub fn normalize_pointer(ctx: &Context, raw: &mut egui::RawInput) -> bool {
    let id = Id::new("pc-touch-pointer-contacts");
    let mut state = ctx.data(|d| d.get_temp::<PointerContacts>(id)).unwrap_or_default();
    let mut modifiers = ctx.input(|i| i.modifiers);
    ctx.data_mut(|d| d.insert_temp(cancelled_id(), false));
    if !raw.events.iter().any(|e| matches!(e, Event::Touch { .. })) && state.contacts.is_empty() {
        return false;
    }
    let mut cancelled = false;
    for event in std::mem::take(&mut raw.events) {
        match &event {
            Event::PointerButton { button: egui::PointerButton::Primary, modifiers: current, .. } => {
                modifiers = *current;
                continue;
            }
            Event::PointerMoved(_) | Event::PointerGone => continue,
            Event::ModifiersChanged(current) => {
                modifiers = *current;
                raw.events.push(event);
            }
            Event::Touch { device_id, id, phase, pos, .. } => {
                let key = (*device_id, *id);
                let pointer_event = match phase {
                    TouchPhase::Start => {
                        if state.contacts.iter().any(|(k, _)| *k == key) {
                            None
                        } else {
                            if state.contacts.is_empty() {
                                state.primary = Some(key);
                            }
                            state.contacts.push((key, *pos));
                            (state.primary == Some(key)).then_some(Event::PointerButton {
                                pos: *pos,
                                button: egui::PointerButton::Primary,
                                pressed: true,
                                modifiers,
                            })
                        }
                    }
                    TouchPhase::Move => {
                        if let Some((_, last)) = state.contacts.iter_mut().find(|(k, _)| *k == key) {
                            *last = *pos;
                        }
                        (state.primary == Some(key)).then_some(Event::PointerMoved(*pos))
                    }
                    TouchPhase::End | TouchPhase::Cancel => {
                        state.contacts.retain(|(k, _)| *k != key);
                        if state.primary == Some(key) {
                            state.primary = None;
                            cancelled |= *phase == TouchPhase::Cancel;
                            Some(Event::PointerButton { pos: *pos, button: egui::PointerButton::Primary, pressed: false, modifiers })
                        } else {
                            None
                        }
                    }
                };
                raw.events.push(event);
                if let Some(pointer_event) = pointer_event {
                    let released = matches!(pointer_event, Event::PointerButton { pressed: false, .. });
                    raw.events.push(pointer_event);
                    if released {
                        raw.events.push(Event::PointerGone);
                    }
                }
            }
            Event::WindowFocused(false) => {
                raw.events.push(event);
                cancelled |= state.cancel(&mut raw.events, modifiers);
            }
            _ => raw.events.push(event),
        }
    }
    ctx.data_mut(|d| {
        if state.contacts.is_empty() {
            d.remove::<PointerContacts>(id);
        } else {
            d.insert_temp(id, state);
        }
        d.insert_temp(cancelled_id(), cancelled);
    });
    cancelled
}

/// Drop an in-progress tool without committing it.
pub fn abort_tools(app: &mut PhotocraftApp, ctx: &Context) {
    app.drag = None;
    app.live_stroke = None;
    app.press_stroke = false;
    app.trail = None;
    app.move_preview = None;
    app.alt_sampling = false;
    app.brush_resize = None;
    app.brush_resize_armed = false;
    app.secondary_erase = false;
    app.guide_drag = None;
    crate::move_mods::finish(app);
    crate::crop_ui::abort_drag(app);
    crate::gradient_ui::abort_drag(app);
    crate::transform_tool::abort_pointer(app);
    crate::zoom_tool::abort(ctx);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(id: u64, phase: TouchPhase, x: f32) -> Event {
        Event::Touch { device_id: egui::TouchDeviceId(0), id: egui::TouchId(id), phase, pos: egui::pos2(x, 50.0), force: None }
    }

    fn normalized_frame(ctx: &Context, events: Vec<Event>) -> bool {
        let mut raw = egui::RawInput { events, ..Default::default() };
        let cancelled = normalize_pointer(ctx, &mut raw);
        ctx.run_ui(raw, |_| {}).textures_delta.clear();
        cancelled
    }

    #[test]
    fn primary_touch_cancel_releases_pointer_and_suppresses_the_cancel_frame() {
        let ctx = Context::default();
        assert!(!normalized_frame(&ctx, vec![touch(1, TouchPhase::Start, 100.0), touch(2, TouchPhase::Start, 200.0)]));
        assert!(ctx.input(|i| i.pointer.primary_down()));
        assert!(normalized_frame(&ctx, vec![touch(1, TouchPhase::Cancel, 110.0)]));
        assert!(!ctx.input(|i| i.pointer.primary_down()));
        assert!(tools_suppressed(&ctx), "a cancel-generated release must not activate a click tool");
        assert!(!normalized_frame(&ctx, vec![touch(2, TouchPhase::Move, 250.0)]));
        assert!(!ctx.input(|i| i.pointer.primary_down()), "the remaining finger cannot inherit the tool");
        assert!(!tools_suppressed(&ctx));
        normalized_frame(&ctx, vec![touch(2, TouchPhase::End, 250.0)]);
        normalized_frame(&ctx, vec![touch(3, TouchPhase::Start, 300.0)]);
        assert!(ctx.input(|i| i.pointer.primary_down()), "the next gesture must work after all fingers lift");
    }

    #[test]
    fn explicit_focus_loss_cancels_contacts_but_a_focus_snapshot_does_not() {
        let ctx = Context::default();
        // pointerdown can focus the canvas before egui receives WindowFocused(true).
        let mut raw = egui::RawInput { focused: false, events: vec![touch(1, TouchPhase::Start, 100.0)], ..Default::default() };
        assert!(!normalize_pointer(&ctx, &mut raw));
        assert!(raw.events.iter().any(|e| matches!(e, Event::PointerButton { pressed: true, .. })));
        ctx.run_ui(raw, |_| {}).textures_delta.clear();
        normalized_frame(&ctx, vec![Event::WindowFocused(true), touch(2, TouchPhase::Start, 200.0)]);
        assert!(ctx.input(|i| i.pointer.primary_down()));
        assert!(normalized_frame(&ctx, vec![Event::WindowFocused(false)]));
        assert!(!ctx.input(|i| i.pointer.primary_down()));
        assert!(!ctx.input(|i| i.any_touches()), "blur must also clear egui's multi-touch state");
        normalized_frame(&ctx, vec![Event::WindowFocused(true), touch(3, TouchPhase::Start, 300.0)]);
        assert!(ctx.input(|i| i.pointer.primary_down()));
    }

    #[test]
    fn touch_pointer_uses_current_modifiers_and_leaves_mouse_input_intact() {
        let ctx = Context::default();
        let modifiers = egui::Modifiers { shift: true, ..Default::default() };
        let mouse = Event::PointerButton { pos: egui::pos2(100.0, 50.0), button: egui::PointerButton::Primary, pressed: true, modifiers };
        let mut raw = egui::RawInput { events: vec![mouse.clone()], ..Default::default() };
        assert!(!normalize_pointer(&ctx, &mut raw));
        assert_eq!(raw.events, vec![mouse.clone()]);
        raw.events = vec![Event::ModifiersChanged(modifiers), mouse, touch(1, TouchPhase::Start, 100.0), touch(1, TouchPhase::Start, 100.0)];
        assert!(!normalize_pointer(&ctx, &mut raw));
        let presses: Vec<_> = raw
            .events
            .iter()
            .filter_map(|e| match e {
                Event::PointerButton { pressed: true, modifiers, .. } => Some(*modifiers),
                _ => None,
            })
            .collect();
        assert_eq!(presses, vec![modifiers], "duplicate touch/compatibility presses must become one correctly modified tool press");
    }

    #[test]
    fn contacts_keep_pointer_ownership_between_touch_event_frames() {
        let ctx = Context::default();
        let button =
            |pressed| Event::PointerButton { pos: egui::pos2(300.0, 50.0), button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        normalized_frame(&ctx, vec![touch(1, TouchPhase::Start, 100.0), touch(2, TouchPhase::Start, 200.0)]);
        normalized_frame(&ctx, vec![button(false), Event::PointerGone]);
        assert!(ctx.input(|i| i.pointer.primary_down()), "an unrelated pointer-only release cannot finish the first finger's tool");
        normalized_frame(&ctx, vec![touch(1, TouchPhase::End, 110.0)]);
        assert!(!ctx.input(|i| i.pointer.primary_down()));
        normalized_frame(&ctx, vec![Event::PointerMoved(egui::pos2(300.0, 50.0)), button(true)]);
        assert!(!ctx.input(|i| i.pointer.primary_down()), "a remaining finger cannot admit a second tool owner");
        normalized_frame(&ctx, vec![touch(2, TouchPhase::Move, 250.0), button(false)]);
        assert!(!ctx.input(|i| i.pointer.primary_down()));
        normalized_frame(&ctx, vec![touch(2, TouchPhase::End, 250.0)]);
        normalized_frame(&ctx, vec![button(true)]);
        assert!(ctx.input(|i| i.pointer.primary_down()), "mouse/pen input resumes once every contact has lifted");
    }

    #[test]
    fn outside_origins_cannot_acquire_canvas_after_moving_inside() {
        let mut origins = Origins::default();
        let free = |p: Pos2| p.x >= 100.0 && p.x <= 500.0;
        assert!(!origins.frame(&[touch(1, TouchPhase::Start, 50.0)], free));
        assert!(!origins.frame(&[touch(1, TouchPhase::Move, 200.0), touch(2, TouchPhase::Start, 300.0)], free));
        assert!(!origins.frame(&[touch(1, TouchPhase::End, 200.0)], free));
        assert!(!origins.frame(&[touch(3, TouchPhase::Start, 400.0)], free));
        assert!(!origins.frame(&[touch(2, TouchPhase::Cancel, 300.0), touch(3, TouchPhase::End, 400.0)], free));
        assert!(origins.frame(&[touch(1, TouchPhase::Start, 200.0), touch(2, TouchPhase::Start, 300.0)], free));
        assert!(origins.frame(&[touch(1, TouchPhase::Move, 10.0), touch(2, TouchPhase::Move, 600.0)], free));
        assert!(origins.frame(&[touch(1, TouchPhase::End, 10.0), touch(2, TouchPhase::End, 600.0)], free));
        assert!(!origins.frame(&[], free));
    }

    #[test]
    fn second_finger_on_overlay_rejects_even_when_centroid_is_on_canvas() {
        let mut origins = Origins::default();
        let free = |p: Pos2| p.x < 300.0;
        assert!(origins.frame(&[touch(1, TouchPhase::Start, 100.0)], free));
        assert!(!origins.frame(&[touch(2, TouchPhase::Start, 350.0)], free));
        assert!(!origins.frame(&[touch(2, TouchPhase::Move, 150.0)], free));
    }

    #[test]
    fn a_second_finger_enters_navigation() {
        assert!(!active_this_frame(false, 0));
        assert!(!active_this_frame(false, 1));
        assert!(active_this_frame(false, 2));
        assert!(active_this_frame(false, 3));
        assert!(active_this_frame(true, 1));
        assert!(active_this_frame(true, 0), "the lift frame must still block the tool");
    }

    #[test]
    fn latch_holds_while_fingers_remain_not_on_leftover_primary() {
        assert!(next_latch(true, 2, true));
        assert!(next_latch(true, 1, true), "the remaining finger still navigates");
        assert!(!next_latch(true, 0, true), "leftover Primary alone must not keep the latch (blocks consecutive pinch)");
        assert!(!next_latch(true, 0, false));
        assert!(!next_latch(false, 1, true));
        assert!(next_latch(false, 2, true));
    }

    #[test]
    fn hostile_counts_never_enter_navigation_from_one_finger() {
        for n in [0, 1] {
            assert!(!active_this_frame(false, n));
            assert!(!next_latch(false, n, false));
        }
    }
}
