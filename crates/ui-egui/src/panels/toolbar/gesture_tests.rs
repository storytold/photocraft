//! Pointer regressions for the main toolbar, separate from shortcut-key groups.
use super::*;
use crate::theme;
use egui_kittest::{Harness, kittest::Queryable};

/// Build the default Studio fixture with accessibility checks enabled.
fn harness() -> Harness<'static, PhotocraftApp> {
    configured_harness(theme::ThemeKind::Studio, false, 1.0)
}

/// Settle a toolbar fixture at the requested theme, column count and display scale.
fn configured_harness(kind: theme::ThemeKind, double: bool, scale: f32) -> Harness<'static, PhotocraftApp> {
    let mut ready = false;
    let builder = Harness::builder().with_size(vec2(1200.0, 1500.0)).with_pixels_per_point(scale).with_step_dt(1.0 / 60.0);
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.ui.panels.toolbar_double = double;
    let mut h = builder.build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if !ready {
                PhotocraftApp::setup_context(ui.ctx(), kind);
                ready = true;
                return;
            }
            toolbar(app, ui);
        },
        app,
    );
    h.run_steps(3);
    h
}

/// Deliver one primary-button transition and advance exactly one frame.
fn pointer(h: &mut Harness<'_, PhotocraftApp>, pos: Pos2, pressed: bool, modifiers: egui::Modifiers) {
    h.event(egui::Event::PointerMoved(pos));
    h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers });
    h.run_steps(1);
}
/// Report whether the fixture currently retains the toolbar flyout area.
fn flyout_open(h: &Harness<'_, PhotocraftApp>) -> bool {
    h.ctx.data(|d| d.get_temp::<(egui::Id, Rect)>(egui::Id::unique("tool-flyout"))).is_some()
}

/// Ensure hold and drag releases remain owned by the flyout and never also cycle.
#[test]
fn alt_hold_and_drag_keep_the_existing_flyout_gesture() {
    for drag in [false, true] {
        let mut h = harness();
        crate::tool_memory::select(h.state_mut(), Tool::Brush);
        h.run_steps(1);
        let at = h.get_by_label("Brush Tool  (B)").rect().center();
        pointer(&mut h, at, true, egui::Modifiers::ALT);
        if drag {
            h.event(egui::Event::PointerMoved(at + vec2(100.0, 0.0)));
            h.run_steps(2);
        } else {
            h.run_steps(24);
        }
        assert!(flyout_open(&h));
        pointer(&mut h, at, false, egui::Modifiers::ALT);
        assert_eq!(h.state().ui.tool, Tool::Brush, "a flyout-owned release must not cycle");
        assert!(flyout_open(&h));
    }
}

/// Reproduce both native and web pointer-emulation ordering for a single touch contact.
fn touch(h: &mut Harness<'_, PhotocraftApp>, at: Pos2, phase: egui::TouchPhase, web: bool) {
    let contact = egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(1), phase, pos: at, force: None };
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    let events = match (phase, web) {
        (egui::TouchPhase::Start, false) => vec![contact, egui::Event::PointerMoved(at), button(true)],
        (egui::TouchPhase::Start, true) => vec![egui::Event::PointerMoved(at), button(true), contact],
        (egui::TouchPhase::End, false) => vec![contact, button(false), egui::Event::PointerGone],
        (egui::TouchPhase::End, true) => vec![button(false), egui::Event::PointerGone, contact],
        (egui::TouchPhase::Cancel, false) => vec![contact, egui::Event::PointerGone],
        (egui::TouchPhase::Cancel, true) => vec![contact],
        _ => vec![contact, egui::Event::PointerMoved(at)],
    };
    h.input_mut().events.extend(events);
    h.run_steps(1);
}

/// A normal touch end hides its pointer after release without breaking an ordinary tap pair.
#[test]
fn touch_double_taps_preserve_hand_and_zoom_view_actions() {
    for web in [false, true] {
        for tool in [Tool::Hand, Tool::Zoom] {
            let mut h = harness();
            h.state_mut().run("file.new", json!({"width": 8, "height": 8})).unwrap();
            crate::tool_memory::select(h.state_mut(), tool);
            h.state_mut().ui.views[0].fit_pending = false;
            h.state_mut().ui.views[0].zoom = 0.5;
            h.run_steps(1);
            let at = h.get_by_label(if tool == Tool::Hand { "Hand Tool  (H)" } else { "Zoom Tool  (Z)" }).rect().center();
            let original = h.state().session.active().unwrap().clone();
            for pair in 0..2 {
                touch(&mut h, at, egui::TouchPhase::Start, web);
                touch(&mut h, at, egui::TouchPhase::End, web);
                assert_eq!(h.state().ui.tool, tool);
                assert_eq!(h.state().ui.views[0].fit_pending, pair == 1 && tool == Tool::Hand, "web={web}, tool={tool:?}");
                assert_eq!(h.state().ui.views[0].zoom, if pair == 1 && tool == Tool::Zoom { 1.0 } else { 0.5 }, "web={web}, tool={tool:?}");
            }
            let doc = h.state().session.active().unwrap();
            assert!(std::sync::Arc::ptr_eq(&doc.doc, &original.doc));
            assert_eq!(doc.revision, original.revision);
            assert_eq!(doc.history.entries(), original.history.entries());
        }
    }
}

/// Native/web cancellation without a primary release clears both a pending press and tap history.
#[test]
fn touch_cancellation_cannot_join_the_next_ordinary_pair() {
    for web in [false, true] {
        let mut h = harness();
        h.state_mut().run("file.new", json!({"width": 8, "height": 8})).unwrap();
        crate::tool_memory::select(h.state_mut(), Tool::Zoom);
        h.state_mut().ui.views[0].fit_pending = false;
        h.state_mut().ui.views[0].zoom = 0.5;
        h.run_steps(1);
        let at = h.get_by_label("Zoom Tool  (Z)").rect().center();
        touch(&mut h, at, egui::TouchPhase::Start, web);
        touch(&mut h, at, egui::TouchPhase::End, web);
        touch(&mut h, at, egui::TouchPhase::Start, web);
        touch(&mut h, at, egui::TouchPhase::Cancel, web);
        for pair in 0..2 {
            touch(&mut h, at, egui::TouchPhase::Start, web);
            touch(&mut h, at, egui::TouchPhase::End, web);
            assert_eq!(h.state().ui.tool, Tool::Zoom);
            assert_eq!(h.state().ui.views[0].zoom, if pair == 1 { 1.0 } else { 0.5 }, "web={web}");
        }
    }
}

/// Pointer disappearance after a held touch release must retain flyout ownership of that release.
#[test]
fn touch_end_selects_the_held_flyout_row() {
    for web in [false, true] {
        let mut h = harness();
        crate::tool_memory::select(h.state_mut(), Tool::Brush);
        h.run_steps(1);
        let at = h.get_by_label("Brush Tool  (B)").rect().center();
        touch(&mut h, at, egui::TouchPhase::Start, web);
        h.run_steps(24);
        assert!(flyout_open(&h));
        let (key, _) = h.ctx.data(|d| d.get_temp::<(egui::Id, Rect)>(egui::Id::unique("tool-flyout"))).unwrap();
        let area = h.ctx.memory(|m| m.area_rect(key.with("flyout"))).unwrap();
        let margin = egui::Frame::popup(&h.ctx.global_style()).total_margin();
        let gap = (area.height() - margin.top - margin.bottom - 4.0 * 26.0) / 3.0;
        let pencil = pos2(area.center().x, area.top() + margin.top + 26.0 + gap + 13.0);
        assert!(area.contains(pencil));
        touch(&mut h, pencil, egui::TouchPhase::Move, web);
        touch(&mut h, pencil, egui::TouchPhase::End, web);
        assert_eq!(h.state().ui.tool, Tool::Pencil, "web={web}");
        assert!(!flyout_open(&h));
    }
}

/// A later cancelled contact cannot erase a completed double-tap or seed the following pair.
#[test]
fn touch_release_before_a_new_cancelled_press_keeps_its_own_result() {
    for web in [false, true] {
        let mut h = harness();
        h.state_mut().run("file.new", json!({"width": 8, "height": 8})).unwrap();
        crate::tool_memory::select(h.state_mut(), Tool::Zoom);
        h.state_mut().ui.views[0].fit_pending = false;
        h.state_mut().ui.views[0].zoom = 0.5;
        h.run_steps(1);
        let at = h.get_by_label("Zoom Tool  (Z)").rect().center();
        touch(&mut h, at, egui::TouchPhase::Start, web);
        touch(&mut h, at, egui::TouchPhase::End, web);
        touch(&mut h, at, egui::TouchPhase::Start, web);
        let contact = |phase| egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(1), phase, pos: at, force: None };
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        let events = if web {
            vec![
                button(false),
                egui::Event::PointerGone,
                contact(egui::TouchPhase::End),
                button(true),
                contact(egui::TouchPhase::Start),
                contact(egui::TouchPhase::Cancel),
            ]
        } else {
            vec![
                contact(egui::TouchPhase::End),
                button(false),
                egui::Event::PointerGone,
                contact(egui::TouchPhase::Start),
                egui::Event::PointerMoved(at),
                button(true),
                contact(egui::TouchPhase::Cancel),
                egui::Event::PointerGone,
            ]
        };
        h.input_mut().events.extend(events);
        h.run_steps(1);
        assert_eq!(h.state().ui.tool, Tool::Zoom);
        assert_eq!(h.state().ui.views[0].zoom, 1.0, "the earlier release completes its pair; web={web}");
        h.state_mut().ui.views[0].zoom = 0.5;
        for pair in 0..2 {
            touch(&mut h, at, egui::TouchPhase::Start, web);
            touch(&mut h, at, egui::TouchPhase::End, web);
            assert_eq!(h.state().ui.views[0].zoom, if pair == 1 { 1.0 } else { 0.5 }, "the cancelled press cannot seed a pair; web={web}");
        }
    }
}

/// A standalone mouse disappearance still breaks the ordinary pair, unlike a touch-end suffix.
#[test]
fn standalone_pointer_gone_breaks_an_ordinary_view_pair() {
    let mut h = harness();
    h.state_mut().run("file.new", json!({"width": 8, "height": 8})).unwrap();
    h.state_mut().ui.views[0].fit_pending = false;
    h.state_mut().ui.views[0].zoom = 0.5;
    let at = h.get_by_label("Zoom Tool  (Z)").rect().center();
    pointer(&mut h, at, true, egui::Modifiers::NONE);
    pointer(&mut h, at, false, egui::Modifiers::NONE);
    h.event(egui::Event::PointerGone);
    h.run_steps(1);
    for pair in 0..2 {
        pointer(&mut h, at, true, egui::Modifiers::NONE);
        pointer(&mut h, at, false, egui::Modifiers::NONE);
        assert_eq!(h.state().ui.views[0].zoom, if pair == 1 { 1.0 } else { 0.5 });
    }
}

/// Repeated modified clicks select tools without invoking either double-click view action.
#[test]
fn rapid_alt_clicks_on_hand_and_a_singleton_do_not_run_double_click_view_actions() {
    let mut h = harness();
    h.state_mut().run("file.new", json!({"width": 8, "height": 8})).unwrap();
    h.state_mut().ui.views[0].fit_pending = false;
    h.state_mut().ui.views[0].zoom = 0.5;
    for (label, expected) in [("Hand Tool  (H)", vec![Tool::RotateView, Tool::Hand, Tool::RotateView]), ("Zoom Tool  (Z)", vec![Tool::Zoom; 3])] {
        let at = h.get_by_label(label).rect().center();
        for tool in expected {
            pointer(&mut h, at, true, egui::Modifiers::ALT);
            pointer(&mut h, at, false, egui::Modifiers::ALT);
            assert_eq!(h.state().ui.tool, tool);
            assert!(!h.state().ui.views[0].fit_pending);
            assert_eq!(h.state().ui.views[0].zoom, 0.5);
        }
    }
}

/// A modifier click and an ordinary click must never combine into a view double-click.
#[test]
fn mixed_modifier_clicks_do_not_run_double_click_view_actions() {
    for hand in [false, true] {
        for alt_first in [false, true] {
            for release_alt_early in [false, true] {
                let mut h = harness();
                h.state_mut().run("file.new", json!({"width": 8, "height": 8})).unwrap();
                h.state_mut().ui.views[0].fit_pending = false;
                h.state_mut().ui.views[0].zoom = 0.5;
                let initial = if hand && alt_first {
                    Tool::RotateView
                } else if hand {
                    Tool::Hand
                } else {
                    Tool::Zoom
                };
                crate::tool_memory::select(h.state_mut(), initial);
                h.run_steps(1);
                let label = match initial {
                    Tool::RotateView => "Rotate View Tool  (R)",
                    Tool::Hand => "Hand Tool  (H)",
                    _ => "Zoom Tool  (Z)",
                };
                let at = h.get_by_label(label).rect().center();
                let modifiers = if alt_first { [egui::Modifiers::ALT, egui::Modifiers::NONE] } else { [egui::Modifiers::NONE, egui::Modifiers::ALT] };
                for modifiers in modifiers {
                    pointer(&mut h, at, true, modifiers);
                    if modifiers.alt && release_alt_early {
                        h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
                        h.run_steps(1);
                    }
                    pointer(&mut h, at, false, if release_alt_early { egui::Modifiers::NONE } else { modifiers });
                    assert!(!h.state().ui.views[0].fit_pending, "hand={hand}, alt_first={alt_first}, early_release={release_alt_early}");
                    assert_eq!(h.state().ui.views[0].zoom, 0.5, "hand={hand}, alt_first={alt_first}, early_release={release_alt_early}");
                }
                assert_eq!(
                    h.state().ui.tool,
                    if release_alt_early {
                        initial
                    } else if !hand {
                        Tool::Zoom
                    } else if alt_first {
                        Tool::Hand
                    } else {
                        Tool::RotateView
                    }
                );
            }
        }
    }
}

/// Preserve genuine ordinary double-clicks after an earlier modifier gesture has timed out.
#[test]
fn ordinary_double_clicks_still_run_view_actions_after_modifier_clicks() {
    for tool in [Tool::Hand, Tool::Zoom] {
        let mut h = harness();
        h.state_mut().run("file.new", json!({"width": 8, "height": 8})).unwrap();
        h.state_mut().ui.views[0].fit_pending = false;
        h.state_mut().ui.views[0].zoom = 0.5;
        let at = h.get_by_label("Zoom Tool  (Z)").rect().center();
        pointer(&mut h, at, true, egui::Modifiers::ALT);
        pointer(&mut h, at, false, egui::Modifiers::ALT);
        h.run_steps(60);
        crate::tool_memory::select(h.state_mut(), tool);
        h.run_steps(1);
        let at = h.get_by_label(if tool == Tool::Hand { "Hand Tool  (H)" } else { "Zoom Tool  (Z)" }).rect().center();
        pointer(&mut h, at, true, egui::Modifiers::NONE);
        pointer(&mut h, at, false, egui::Modifiers::NONE);
        assert!(!h.state().ui.views[0].fit_pending, "the first ordinary click only selects");
        assert_eq!(h.state().ui.views[0].zoom, 0.5);
        pointer(&mut h, at, true, egui::Modifiers::NONE);
        pointer(&mut h, at, false, egui::Modifiers::NONE);
        if tool == Tool::Hand {
            assert!(h.state().ui.views[0].fit_pending, "ordinary Hand double-click still fits the view");
        } else {
            assert_eq!(h.state().ui.views[0].zoom, 1.0, "ordinary Zoom double-click still restores actual pixels");
        }
    }
}

/// Rapid ordinary clicks after Alt must form their own pair, independently of raw click counts.
#[test]
fn rapid_plain_pair_after_alt_runs_hand_and_zoom_view_actions() {
    for tool in [Tool::Hand, Tool::Zoom] {
        let mut h = harness();
        h.state_mut().run("file.new", json!({"width": 8, "height": 8})).unwrap();
        h.state_mut().ui.views[0].fit_pending = false;
        h.state_mut().ui.views[0].zoom = 0.5;
        crate::tool_memory::select(h.state_mut(), if tool == Tool::Hand { Tool::RotateView } else { tool });
        h.run_steps(1);
        let at = h.get_by_label(if tool == Tool::Hand { "Rotate View Tool  (R)" } else { "Zoom Tool  (Z)" }).rect().center();
        for (index, mods) in [egui::Modifiers::ALT, egui::Modifiers::NONE, egui::Modifiers::NONE].into_iter().enumerate() {
            pointer(&mut h, at, true, mods);
            pointer(&mut h, at, false, mods);
            assert_eq!(h.state().ui.tool, tool);
            assert_eq!(h.state().ui.views[0].fit_pending, index == 2 && tool == Tool::Hand);
            assert_eq!(h.state().ui.views[0].zoom, if index == 2 && tool == Tool::Zoom { 1.0 } else { 0.5 });
            h.run_steps(5);
        }
    }
}

/// A new press after a release in one frame must not rewrite the completed gesture's Alt state.
#[test]
fn release_then_next_press_preserves_both_gestures_modifiers() {
    for first_alt in [true, false] {
        let mut h = harness();
        let at = h.get_by_label("Brush Tool  (B)").rect().center();
        let first = if first_alt { egui::Modifiers::ALT } else { egui::Modifiers::NONE };
        let second = if first_alt { egui::Modifiers::NONE } else { egui::Modifiers::ALT };
        pointer(&mut h, at, true, first);
        h.input_mut().events.extend([
            egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: first },
            egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: second },
        ]);
        h.run_steps(1);
        assert_eq!(h.state().ui.tool, if first_alt { Tool::Pencil } else { Tool::Brush }, "completed first gesture");
        pointer(&mut h, at, false, second);
        assert_eq!(h.state().ui.tool, Tool::Pencil, "second gesture retains its own press modifiers");
    }
}

/// Preserve single/double/triple-click semantics and reset the sequence after time or movement.
#[test]
fn ordinary_view_pairs_respect_timing_distance_and_triple_clicks() {
    for distance in [false, true] {
        let mut h = harness();
        h.state_mut().run("file.new", json!({"width": 8, "height": 8})).unwrap();
        h.state_mut().ui.views[0].fit_pending = false;
        h.state_mut().ui.views[0].zoom = 0.5;
        let at = h.get_by_label("Zoom Tool  (Z)").rect().center();
        for index in 0..4 {
            pointer(&mut h, at, true, egui::Modifiers::NONE);
            pointer(&mut h, at, false, egui::Modifiers::NONE);
            assert_eq!(h.state().ui.views[0].zoom, if index == 1 { 1.0 } else { 0.5 }, "ordinary click {index}");
            h.state_mut().ui.views[0].zoom = 0.5;
        }
        let next = if distance {
            at + vec2(8.0, 0.0)
        } else {
            h.run_steps(20);
            at
        };
        pointer(&mut h, next, true, egui::Modifiers::NONE);
        pointer(&mut h, next, false, egui::Modifiers::NONE);
        assert_eq!(h.state().ui.views[0].zoom, 0.5, "a delayed/displaced release starts a fresh pair");
        pointer(&mut h, next, true, egui::Modifiers::NONE);
        pointer(&mut h, next, false, egui::Modifiers::NONE);
        assert_eq!(h.state().ui.views[0].zoom, 1.0);
    }
}

/// An intervening release outside the toolbar must break an ordinary view-action pair.
#[test]
fn release_outside_breaks_the_ordinary_view_pair() {
    let mut h = harness();
    h.state_mut().run("file.new", json!({"width": 8, "height": 8})).unwrap();
    h.state_mut().ui.views[0].fit_pending = false;
    h.state_mut().ui.views[0].zoom = 0.5;
    let at = h.get_by_label("Zoom Tool  (Z)").rect().center();
    pointer(&mut h, at, true, egui::Modifiers::NONE);
    pointer(&mut h, at, false, egui::Modifiers::NONE);
    pointer(&mut h, at, true, egui::Modifiers::NONE);
    pointer(&mut h, at + vec2(100.0, 0.0), false, egui::Modifiers::NONE);
    pointer(&mut h, at, true, egui::Modifiers::NONE);
    pointer(&mut h, at, false, egui::Modifiers::NONE);
    assert_eq!(h.state().ui.views[0].zoom, 0.5);
    assert_eq!(h.state().ui.tool, Tool::Zoom);
}

/// A flyout-owned release cannot become a cycle when the next press shares its frame.
#[test]
fn held_release_then_next_press_keeps_flyout_ownership() {
    let mut h = harness();
    let at = h.get_by_label("Brush Tool  (B)").rect().center();
    pointer(&mut h, at, true, egui::Modifiers::ALT);
    h.run_steps(24);
    assert!(flyout_open(&h));
    h.input_mut().events.extend([
        egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::ALT },
        egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::ALT },
    ]);
    h.run_steps(1);
    assert_eq!(h.state().ui.tool, Tool::Brush);
    assert!(flyout_open(&h));
    pointer(&mut h, at, false, egui::Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::Brush, "release-time modifiers retain main's ordinary selection");
    assert!(!flyout_open(&h));
}

/// A held flyout uses the release position even when a later press changes the frame's pointer.
#[test]
fn flyout_release_uses_its_own_position_before_a_later_press() {
    for next_press in [false, true] {
        let mut h = harness();
        let at = h.get_by_label("Brush Tool  (B)").rect().center();
        pointer(&mut h, at, true, egui::Modifiers::ALT);
        h.run_steps(24);
        let (key, _) = h.ctx.data(|d| d.get_temp::<(egui::Id, Rect)>(egui::Id::unique("tool-flyout"))).unwrap();
        let area = h.ctx.memory(|m| m.area_rect(key.with("flyout"))).unwrap();
        let margin = egui::Frame::popup(&h.ctx.global_style()).total_margin();
        let gap = (area.height() - margin.top - margin.bottom - 4.0 * 26.0) / 3.0;
        assert!(gap >= 0.0);
        let pencil = pos2(area.center().x, area.top() + margin.top + 26.0 + gap + 13.0);
        let replacement = pencil + vec2(0.0, 2.0 * (26.0 + gap));
        assert!(area.contains(pencil) && area.contains(replacement));
        h.event(egui::Event::PointerMoved(pencil));
        h.run_steps(1);
        // Keep all transitions in the raw input queue so their order is preserved.
        h.input_mut().events.push(egui::Event::PointerButton {
            pos: pencil,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        if next_press {
            h.input_mut().events.extend([
                egui::Event::PointerMoved(replacement),
                egui::Event::PointerButton { pos: replacement, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
            ]);
        }
        h.run_steps(1);
        assert_eq!(h.ctx.input(|i| i.pointer.primary_down()), next_press);
        assert_eq!(h.ctx.input(|i| i.pointer.hover_pos()), Some(if next_press { replacement } else { pencil }));
        assert_eq!(h.state().ui.tool, Tool::Pencil, "release chooses Pencil; later press={next_press}");
        assert!(!flyout_open(&h));
        if next_press {
            pointer(&mut h, replacement, false, egui::Modifiers::NONE);
            assert_eq!(h.state().ui.tool, Tool::Pencil);
        }
    }
}

/// Any modifier held during a click excludes it from an ordinary pair, even if released early.
#[test]
fn modifier_releases_cannot_turn_modified_gestures_into_view_pairs() {
    for modifier in [egui::Modifiers::ALT, egui::Modifiers::SHIFT, egui::Modifiers::CTRL, egui::Modifiers::COMMAND] {
        for early in [false, true] {
            let mut h = harness();
            h.state_mut().run("file.new", json!({"width": 8, "height": 8})).unwrap();
            h.state_mut().ui.views[0].zoom = 0.5;
            h.state_mut().ui.views[0].fit_pending = false;
            let at = h.get_by_label("Zoom Tool  (Z)").rect().center();
            pointer(&mut h, at, true, modifier);
            pointer(&mut h, at, false, if early { egui::Modifiers::NONE } else { modifier });
            for pair in 0..2 {
                pointer(&mut h, at, true, egui::Modifiers::NONE);
                pointer(&mut h, at, false, egui::Modifiers::NONE);
                assert_eq!(h.state().ui.views[0].zoom, if pair == 1 { 1.0 } else { 0.5 }, "modifier={modifier:?}, early={early}");
                assert_eq!(h.state().ui.tool, Tool::Zoom);
            }
        }
    }
}

/// Losing focus breaks future double-click history while preserving ordinary clicks after it.
#[test]
fn focus_loss_breaks_the_ordinary_view_pair() {
    let mut h = harness();
    h.state_mut().run("file.new", json!({"width": 8, "height": 8})).unwrap();
    h.state_mut().ui.views[0].fit_pending = false;
    h.state_mut().ui.views[0].zoom = 0.5;
    let at = h.get_by_label("Zoom Tool  (Z)").rect().center();
    pointer(&mut h, at, true, egui::Modifiers::NONE);
    pointer(&mut h, at, false, egui::Modifiers::NONE);
    h.input_mut().events.extend([egui::Event::WindowFocused(false), egui::Event::WindowFocused(true)]);
    h.run_steps(1);
    for pair in 0..2 {
        pointer(&mut h, at, true, egui::Modifiers::NONE);
        pointer(&mut h, at, false, egui::Modifiers::NONE);
        assert_eq!(h.state().ui.views[0].zoom, if pair == 1 { 1.0 } else { 0.5 });
    }
}

/// Keyboard selection cannot complete an earlier mouse pair or seed a later one.
#[test]
fn keyboard_activation_stays_selection_and_breaks_pointer_history() {
    let mut h = harness();
    h.state_mut().run("file.new", json!({"width": 8, "height": 8})).unwrap();
    h.state_mut().ui.views[0].fit_pending = false;
    h.state_mut().ui.views[0].zoom = 0.5;
    let at = h.get_by_label("Zoom Tool  (Z)").rect().center();
    pointer(&mut h, at, true, egui::Modifiers::NONE);
    pointer(&mut h, at, false, egui::Modifiers::NONE);
    h.get_by_label("Zoom Tool  (Z)").focus();
    h.key_press(egui::Key::Space);
    h.run_steps(1);
    assert_eq!(h.state().ui.tool, Tool::Zoom);
    assert_eq!(h.state().ui.views[0].zoom, 0.5);
    for pair in 0..2 {
        pointer(&mut h, at, true, egui::Modifiers::NONE);
        pointer(&mut h, at, false, egui::Modifiers::NONE);
        assert_eq!(h.state().ui.views[0].zoom, if pair == 1 { 1.0 } else { 0.5 });
    }
}

/// Touch cancellation must not turn a stale emulated down button into a later long-press.
#[test]
fn cancelled_touch_cannot_open_a_delayed_flyout() {
    for web in [false, true] {
        for move_after in [false, true] {
            let mut h = harness();
            let at = h.get_by_label("Brush Tool  (B)").rect().center();
            touch(&mut h, at, egui::TouchPhase::Start, web);
            touch(&mut h, at, egui::TouchPhase::Cancel, web);
            if move_after {
                h.event(egui::Event::PointerMoved(at + vec2(100.0, 0.0)));
            }
            h.run_steps(30);
            assert!(!flyout_open(&h), "cancelled contact must not open a flyout; web={web}, move={move_after}");
            assert_eq!(h.state().ui.tool, Tool::Move);
        }
    }
}

/// Both toolbar column counts and every theme/scale retain a usable ordinary pair after Alt.
#[test]
fn rapid_pairs_work_in_all_toolbar_layouts_themes_and_scales() {
    for kind in theme::ThemeKind::ALL {
        for double in [false, true] {
            for scale in [1.0, 1.5, 2.0] {
                let mut h = configured_harness(kind, double, scale);
                h.state_mut().run("file.new", json!({"width": 8, "height": 8})).unwrap();
                h.state_mut().ui.views[0].zoom = 0.5;
                h.state_mut().ui.views[0].fit_pending = false;
                let at = h.get_by_label("Zoom Tool  (Z)").rect().center();
                for mods in [egui::Modifiers::ALT, egui::Modifiers::NONE, egui::Modifiers::NONE] {
                    pointer(&mut h, at, true, mods);
                    pointer(&mut h, at, false, mods);
                }
                assert_eq!(h.state().ui.tool, Tool::Zoom, "{kind:?}, double={double}, scale={scale}");
                assert_eq!(h.state().ui.views[0].zoom, 1.0, "{kind:?}, double={double}, scale={scale}");
            }
        }
    }
}

/// Native event batching must preserve a complete ordinary pair, including a third click.
#[test]
fn multiple_completed_clicks_in_one_frame_still_invoke_once() {
    for count in [2, 3] {
        for alt_first in [false, true] {
            let mut h = harness();
            h.state_mut().run("file.new", json!({"width": 8, "height": 8})).unwrap();
            h.state_mut().ui.views[0].fit_pending = false;
            h.state_mut().ui.views[0].zoom = 0.5;
            let at = h.get_by_label("Zoom Tool  (Z)").rect().center();
            if alt_first {
                pointer(&mut h, at, true, egui::Modifiers::ALT);
                pointer(&mut h, at, false, egui::Modifiers::ALT);
            }
            h.input_mut().events.push(egui::Event::PointerMoved(at));
            for _ in 0..count {
                for pressed in [true, false] {
                    h.input_mut().events.push(egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    });
                }
            }
            h.run_steps(1);
            assert_eq!(h.state().ui.tool, Tool::Zoom);
            assert_eq!(h.state().ui.views[0].zoom, 1.0, "count={count}, alt_first={alt_first}");
            h.state_mut().ui.views[0].zoom = 0.5;
            pointer(&mut h, at, true, egui::Modifiers::NONE);
            pointer(&mut h, at, false, egui::Modifiers::NONE);
            assert_eq!(h.state().ui.views[0].zoom, 0.5, "the next rapid click is not another pair");
        }
    }
}

/// Batched modified clicks and ordinary pairs must apply both selection and view actions in order.
#[test]
fn batched_hand_pairs_and_alt_clicks_keep_the_final_selection() {
    for initial in [Tool::Hand, Tool::RotateView] {
        for alt_first in [false, true] {
            let mut h = harness();
            h.state_mut().run("file.new", json!({"width": 8, "height": 8})).unwrap();
            crate::tool_memory::select(h.state_mut(), initial);
            h.state_mut().ui.views[0].fit_pending = false;
            h.state_mut().ui.views[0].zoom = 0.5;
            h.run_steps(1);
            let label = if initial == Tool::Hand { "Hand Tool  (H)" } else { "Rotate View Tool  (R)" };
            let at = h.get_by_label(label).rect().center();
            h.input_mut().events.push(egui::Event::PointerMoved(at));
            let sequence = if alt_first {
                [egui::Modifiers::ALT, egui::Modifiers::NONE, egui::Modifiers::NONE]
            } else {
                [egui::Modifiers::NONE, egui::Modifiers::NONE, egui::Modifiers::ALT]
            };
            for modifiers in sequence {
                for pressed in [true, false] {
                    h.input_mut().events.push(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers });
                }
            }
            h.run_steps(1);
            let fits = (initial == Tool::Hand) != alt_first;
            assert_eq!(h.state().ui.tool, if initial == Tool::Hand { Tool::RotateView } else { Tool::Hand }, "initial={initial:?}, alt_first={alt_first}");
            assert_eq!(h.state().ui.views[0].fit_pending, fits, "initial={initial:?}, alt_first={alt_first}");
            assert_eq!(h.state().session.journal.iter().filter(|(id, _)| id == "view.fitOnScreen").count(), usize::from(fits));
            h.state_mut().ui.views[0].fit_pending = false;
            pointer(&mut h, at, true, egui::Modifiers::NONE);
            pointer(&mut h, at, false, egui::Modifiers::NONE);
            assert!(!h.state().ui.views[0].fit_pending, "one further click is not a new pair");
        }
    }
}

/// Mouse viewport exit breaks click history without stealing a held flyout drag's later release.
#[test]
fn mouse_leave_and_return_keeps_a_held_flyout_release() {
    let mut h = harness();
    let at = h.get_by_label("Brush Tool  (B)").rect().center();
    pointer(&mut h, at, true, egui::Modifiers::NONE);
    h.run_steps(24);
    assert!(flyout_open(&h));
    let (key, _) = h.ctx.data(|d| d.get_temp::<(egui::Id, Rect)>(egui::Id::unique("tool-flyout"))).unwrap();
    let area = h.ctx.memory(|m| m.area_rect(key.with("flyout"))).unwrap();
    let margin = egui::Frame::popup(&h.ctx.global_style()).total_margin();
    let gap = (area.height() - margin.top - margin.bottom - 4.0 * 26.0) / 3.0;
    let pencil = pos2(area.center().x, area.top() + margin.top + 26.0 + gap + 13.0);
    h.event(egui::Event::PointerGone);
    h.run_steps(1);
    pointer(&mut h, pencil, false, egui::Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::Pencil);
    assert!(!flyout_open(&h));
}

/// Held choices and later complete clicks keep input order, independent of row drawing order.
#[test]
fn held_flyout_release_and_a_later_click_keep_event_order() {
    for target in [0, 1, 2] {
        let mut h = harness();
        let at = h.get_by_label("Brush Tool  (B)").rect().center();
        let marquee = h.get_by_label("Rectangular Marquee Tool  (M)").rect().center();
        pointer(&mut h, at, true, egui::Modifiers::NONE);
        h.run_steps(24);
        assert!(flyout_open(&h));
        let (key, _) = h.ctx.data(|d| d.get_temp::<(egui::Id, Rect)>(egui::Id::unique("tool-flyout"))).unwrap();
        let area = h.ctx.memory(|m| m.area_rect(key.with("flyout"))).unwrap();
        let margin = egui::Frame::popup(&h.ctx.global_style()).total_margin();
        let gap = (area.height() - margin.top - margin.bottom - 4.0 * 26.0) / 3.0;
        let pencil = pos2(area.center().x, area.top() + margin.top + 26.0 + gap + 13.0);
        let (later, expected) = match target {
            0 => (area.right_bottom() + vec2(100.0, 100.0), Tool::Pencil),
            1 => (pos2(area.center().x, area.top() + margin.top + 13.0), Tool::Brush),
            _ => (marquee, Tool::RectMarquee),
        };
        assert!(area.contains(pencil));
        h.input_mut().events.extend([
            egui::Event::PointerMoved(pencil),
            egui::Event::PointerButton { pos: pencil, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
            egui::Event::PointerMoved(later),
            egui::Event::PointerButton { pos: later, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
            egui::Event::PointerButton { pos: later, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
        ]);
        h.run_steps(1);
        assert_eq!(h.state().ui.tool, expected, "later target={target}");
        assert!(!flyout_open(&h));
    }
}

/// Metadata-only cancellation from another contact cannot abort the owner.
/// Different-device id reuse is a synthetic scanner case; winit keys pointer ownership by id.
#[test]
fn cancelling_another_contact_preserves_the_primary_toolbar_hold() {
    for (device, contact) in [(1, 2), (2, 1)] {
        let mut h = harness();
        let at = h.get_by_label("Brush Tool  (B)").rect().center();
        touch(&mut h, at, egui::TouchPhase::Start, false);
        // Different ids match native secondary-contact metadata; device-id reuse also
        // checks scanner isolation without claiming winit emits this exact sequence.
        for phase in [egui::TouchPhase::Start, egui::TouchPhase::Cancel] {
            h.input_mut().events.push(egui::Event::Touch {
                device_id: egui::TouchDeviceId(device),
                id: egui::TouchId(contact),
                phase,
                pos: at + vec2(100.0, 0.0),
                force: None,
            });
            h.run_steps(1);
        }
        h.run_steps(24);
        assert!(flyout_open(&h), "primary remains held after other contact cancels: device={device}, id={contact}");
        assert_eq!(h.state().ui.tool, Tool::Brush);
    }
}

/// Synthetic metadata delayed across frames still identifies the owning cancellation.
/// The pinned web bridge normally emits press and Start in the same callback.
#[test]
fn late_touch_metadata_still_cancels_its_own_pending_hold() {
    let mut h = harness();
    let at = h.get_by_label("Brush Tool  (B)").rect().center();
    pointer(&mut h, at, true, egui::Modifiers::NONE);
    for phase in [egui::TouchPhase::Start, egui::TouchPhase::Cancel] {
        h.input_mut().events.push(egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(1), phase, pos: at, force: None });
        h.run_steps(1);
    }
    h.run_steps(30);
    assert!(!flyout_open(&h), "late metadata must still cancel the owning pointer's pending hold");
    assert_eq!(h.state().ui.tool, Tool::Move);
}

/// A mouse leaving immediately after release interrupts the next pair; only touch-end hides normally.
#[test]
fn mouse_disappearance_after_release_breaks_the_next_view_pair() {
    let mut h = harness();
    h.state_mut().run("file.new", json!({"width": 8, "height": 8})).unwrap();
    crate::tool_memory::select(h.state_mut(), Tool::Zoom);
    h.state_mut().ui.views[0].fit_pending = false;
    h.state_mut().ui.views[0].zoom = 0.5;
    h.run_steps(1);
    let at = h.get_by_label("Zoom Tool  (Z)").rect().center();
    pointer(&mut h, at, true, egui::Modifiers::NONE);
    h.input_mut().events.extend([
        egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
        egui::Event::PointerGone,
    ]);
    h.run_steps(1);
    pointer(&mut h, at, true, egui::Modifiers::NONE);
    pointer(&mut h, at, false, egui::Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::Zoom);
    assert_eq!(h.state().ui.views[0].zoom, 0.5, "a completed mouse release does not make CursorLeft a touch-end suffix");
}

/// Web secondary Start repeats primary-down at the owner's position before new-contact metadata.
/// Cancelling the owner must still abort the hold; cancelling the other contact must preserve it.
#[test]
fn web_secondary_down_retains_the_original_cancellation_owner() {
    for cancel_owner in [true, false] {
        let mut h = harness();
        crate::tool_memory::select(h.state_mut(), Tool::Brush);
        h.run_steps(1);
        let at = h.get_by_label("Brush Tool  (B)").rect().center();
        touch(&mut h, at, egui::TouchPhase::Start, true);
        h.input_mut().events.extend([
            egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
            egui::Event::Touch {
                device_id: egui::TouchDeviceId(1),
                id: egui::TouchId(2),
                phase: egui::TouchPhase::Start,
                pos: at + vec2(100.0, 0.0),
                force: None,
            },
        ]);
        h.run_steps(1);
        h.input_mut().events.push(egui::Event::Touch {
            device_id: egui::TouchDeviceId(1),
            id: egui::TouchId(if cancel_owner { 1 } else { 2 }),
            phase: egui::TouchPhase::Cancel,
            pos: if cancel_owner { at } else { at + vec2(100.0, 0.0) },
            force: None,
        });
        h.run_steps(1);
        h.run_steps(24);
        assert_eq!(flyout_open(&h), !cancel_owner, "original contact retains ownership: cancel_owner={cancel_owner}");
        assert_eq!(h.state().ui.tool, Tool::Brush);
    }
}

/// Another web contact cannot turn an already-held flyout release into an unrelated row click.
#[test]
fn web_secondary_start_preserves_an_open_held_flyout_release() {
    let mut h = harness();
    let at = h.get_by_label("Brush Tool  (B)").rect().center();
    touch(&mut h, at, egui::TouchPhase::Start, true);
    h.run_steps(24);
    assert!(flyout_open(&h));
    let (key, _) = h.ctx.data(|d| d.get_temp::<(egui::Id, Rect)>(egui::Id::unique("tool-flyout"))).unwrap();
    let area = h.ctx.memory(|m| m.area_rect(key.with("flyout"))).unwrap();
    let margin = egui::Frame::popup(&h.ctx.global_style()).total_margin();
    let gap = (area.height() - margin.top - margin.bottom - 4.0 * 26.0) / 3.0;
    let pencil = pos2(area.center().x, area.top() + margin.top + 26.0 + gap + 13.0);
    h.input_mut().events.extend([
        egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
        egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(2), phase: egui::TouchPhase::Start, pos: at + vec2(100.0, 0.0), force: None },
        egui::Event::Touch {
            device_id: egui::TouchDeviceId(1),
            id: egui::TouchId(2),
            phase: egui::TouchPhase::Cancel,
            pos: at + vec2(100.0, 0.0),
            force: None,
        },
    ]);
    h.run_steps(1);
    touch(&mut h, pencil, egui::TouchPhase::End, true);
    assert_eq!(h.state().ui.tool, Tool::Pencil, "the original contact still owns the held row release");
    assert!(!flyout_open(&h));
}

/// A focused slot still supports keyboard selection while its drag-open flyout owns a pointer.
#[test]
fn review_keyboard_activation_while_drag_held_selects_the_slot() {
    let mut h = harness();
    crate::tool_memory::select(h.state_mut(), Tool::Move);
    h.run_steps(1);
    let at = h.get_by_label("Brush Tool  (B)").rect().center();
    pointer(&mut h, at, true, egui::Modifiers::NONE);
    h.event(egui::Event::PointerMoved(at + vec2(40.0, 0.0)));
    h.run_steps(1);
    assert!(flyout_open(&h));
    assert_eq!(h.state().ui.tool, Tool::Move, "drag opening alone does not select");
    h.get_by_label("Brush Tool  (B)").focus();
    h.key_press(egui::Key::Space);
    h.run_steps(1);
    assert_eq!(h.state().ui.tool, Tool::Brush);
}

/// Earlier completed view pairs cannot disappear when a different slot owns the final click.
#[test]
fn review_view_pair_before_another_slot_keeps_both_results() {
    for tool in [Tool::Hand, Tool::Zoom] {
        let mut h = harness();
        h.state_mut().run("file.new", json!({"width": 8, "height": 8})).unwrap();
        crate::tool_memory::select(h.state_mut(), tool);
        h.state_mut().ui.views[0].fit_pending = false;
        h.state_mut().ui.views[0].zoom = 0.5;
        h.run_steps(1);
        let at = h.get_by_label(if tool == Tool::Hand { "Hand Tool  (H)" } else { "Zoom Tool  (Z)" }).rect().center();
        let brush = h.get_by_label("Brush Tool  (B)").rect().center();
        for pos in [at, at, brush] {
            h.input_mut().events.extend([
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
                egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
            ]);
        }
        h.run_steps(1);
        assert_eq!(h.state().ui.tool, Tool::Brush);
        assert_eq!(h.state().ui.views[0].fit_pending, tool == Tool::Hand);
        assert_eq!(h.state().ui.views[0].zoom, if tool == Tool::Zoom { 1.0 } else { 0.5 });
    }
}

/// A secondary web contact cannot restart the original contact's 0.35-second hold interval.
#[test]
fn review_web_secondary_down_preserves_the_hold_deadline() {
    let mut h = harness();
    let at = h.get_by_label("Brush Tool  (B)").rect().center();
    touch(&mut h, at, egui::TouchPhase::Start, true);
    h.run_steps(17);
    assert!(!flyout_open(&h));
    h.input_mut().events.extend([
        egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
        egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(2), phase: egui::TouchPhase::Start, pos: at + vec2(100.0, 0.0), force: None },
    ]);
    h.run_steps(1);
    h.run_steps(5);
    assert!(flyout_open(&h), "the owner has been held beyond 0.35 seconds, the duplicate down has not");
    assert_eq!(h.state().ui.tool, Tool::Brush);
}

/// Keyboard and held-row selection follow event order rather than their row drawing order.
#[test]
fn review_keyboard_row_activation_keeps_its_event_order() {
    for key_after_release in [true, false] {
        let mut h = harness();
        let at = h.get_by_label("Brush Tool  (B)").rect().center();
        pointer(&mut h, at, true, egui::Modifiers::NONE);
        h.run_steps(24);
        let (key, _) = h.ctx.data(|d| d.get_temp::<(egui::Id, Rect)>(egui::Id::unique("tool-flyout"))).unwrap();
        let area = h.ctx.memory(|m| m.area_rect(key.with("flyout"))).unwrap();
        let margin = egui::Frame::popup(&h.ctx.global_style()).total_margin();
        let gap = (area.height() - margin.top - margin.bottom - 4.0 * 26.0) / 3.0;
        let pencil = pos2(area.center().x, area.top() + margin.top + 26.0 + gap + 13.0);
        let brush = pos2(area.center().x, area.top() + margin.top + 13.0);
        let row_id = h.ctx.viewport(|v| {
            v.prev_pass
                .widgets
                .get_layer(egui::LayerId::new(egui::Order::Foreground, key.with("flyout")))
                .filter(|w| w.sense.senses_click() && w.interact_rect.contains(brush))
                .last()
                .map(|w| w.id)
                .unwrap()
        });
        h.ctx.memory_mut(|m| m.request_focus(row_id));
        let keys = [true, false].map(|pressed| egui::Event::Key {
            key: egui::Key::Space,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        let release = [
            egui::Event::PointerMoved(pencil),
            egui::Event::PointerButton { pos: pencil, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
        ];
        if key_after_release {
            h.input_mut().events.extend(release);
            h.input_mut().events.extend(keys);
        } else {
            h.input_mut().events.extend(keys);
            h.input_mut().events.extend(release);
        }
        h.run_steps(1);
        assert_eq!(h.state().ui.tool, if key_after_release { Tool::Brush } else { Tool::Pencil });
        assert!(!flyout_open(&h));
    }
}

/// A secondary row click is not a supported tool-selection gesture and cannot replace a held choice.
#[test]
fn review_secondary_row_click_does_not_select_a_tool() {
    let mut h = harness();
    let at = h.get_by_label("Brush Tool  (B)").rect().center();
    pointer(&mut h, at, true, egui::Modifiers::NONE);
    h.run_steps(24);
    let (key, _) = h.ctx.data(|d| d.get_temp::<(egui::Id, Rect)>(egui::Id::unique("tool-flyout"))).unwrap();
    let area = h.ctx.memory(|m| m.area_rect(key.with("flyout"))).unwrap();
    let margin = egui::Frame::popup(&h.ctx.global_style()).total_margin();
    let gap = (area.height() - margin.top - margin.bottom - 4.0 * 26.0) / 3.0;
    let pencil = pos2(area.center().x, area.top() + margin.top + 26.0 + gap + 13.0);
    let brush = pos2(area.center().x, area.top() + margin.top + 13.0);
    h.input_mut().events.extend([
        egui::Event::PointerMoved(pencil),
        egui::Event::PointerButton { pos: pencil, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
        egui::Event::PointerMoved(brush),
        egui::Event::PointerButton { pos: brush, button: egui::PointerButton::Secondary, pressed: true, modifiers: egui::Modifiers::NONE },
        egui::Event::PointerButton { pos: brush, button: egui::PointerButton::Secondary, pressed: false, modifiers: egui::Modifiers::NONE },
    ]);
    h.run_steps(1);
    assert_eq!(h.state().ui.tool, Tool::Pencil);
    assert!(!flyout_open(&h));
}

/// Endpoint displacement still disqualifies a click when integrations omit an intermediate move.
#[test]
fn review_displaced_releases_without_move_events_cannot_form_a_pair() {
    for web in [false, true] {
        let mut h = harness();
        h.state_mut().run("file.new", json!({"width": 8, "height": 8})).unwrap();
        crate::tool_memory::select(h.state_mut(), Tool::Zoom);
        h.state_mut().ui.views[0].fit_pending = false;
        h.state_mut().ui.views[0].zoom = 0.5;
        h.run_steps(1);
        let rect = h.get_by_label("Zoom Tool  (Z)").rect();
        let delta = h.ctx.options(|o| o.input_options.max_click_dist) * 0.75;
        let start = rect.center() - vec2(delta, 0.0);
        let end = rect.center() + vec2(delta, 0.0);
        assert!(rect.contains(start) && rect.contains(end));
        for _ in 0..2 {
            h.input_mut().events.extend([
                egui::Event::PointerMoved(start),
                egui::Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
            ]);
            if web {
                h.input_mut().events.push(egui::Event::Touch {
                    device_id: egui::TouchDeviceId(1),
                    id: egui::TouchId(1),
                    phase: egui::TouchPhase::Start,
                    pos: start,
                    force: None,
                });
            }
            h.input_mut().events.push(egui::Event::PointerButton {
                pos: end,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            });
            if web {
                h.input_mut().events.extend([
                    egui::Event::PointerGone,
                    egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(1), phase: egui::TouchPhase::End, pos: end, force: None },
                ]);
            }
            h.run_steps(1);
            assert_eq!(h.state().ui.tool, Tool::Zoom);
            assert_eq!(h.state().ui.views[0].zoom, 0.5, "web={web}");
        }
    }
}

/// Batched replay cannot click through foreground windows, modals, or later same-layer controls,
/// including points accepted only by the toolbar button’s near-edge interaction radius.
#[test]
fn review_replay_respects_widget_ownership() {
    let reference = harness();
    let tool_rect = reference.get_by_label("Zoom Tool  (Z)").rect();
    for near_edge in [false, true] {
        let rect = tool_rect.expand(if near_edge { 2.0 } else { 0.0 });
        for mode in 0..5 {
            let mut ready = false;
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            app.run("file.new", json!({"width":8,"height":8})).unwrap();
            app.ui.views[0].fit_pending = false;
            app.ui.views[0].zoom = 0.5;
            let mut h = Harness::builder().with_size(vec2(1200.0, 1500.0)).with_step_dt(1.0 / 60.0).build_ui_state(
                move |ui, app: &mut PhotocraftApp| {
                    if !ready {
                        PhotocraftApp::setup_context(ui.ctx(), theme::ThemeKind::Studio);
                        ready = true;
                        return;
                    }
                    if mode == 4 {
                        ui.disable();
                    }
                    toolbar(app, ui);
                    match mode {
                        0 => {
                            egui::Area::new(egui::Id::unique("ownership-overlay")).order(egui::Order::Foreground).fixed_pos(rect.min).show(ui.ctx(), |ui| {
                                ui.allocate_exact_size(rect.size(), Sense::click());
                            });
                        }
                        1 => {
                            egui::Modal::new(egui::Id::unique("ownership-modal")).show(ui.ctx(), |ui| {
                                ui.label("Modal outside toolbar");
                            });
                        }
                        2 | 3 => {
                            ui.interact(rect, egui::Id::unique("same-layer-overlay"), if mode == 2 { Sense::click() } else { Sense::drag() });
                        }
                        _ => {}
                    }
                },
                app,
            );
            h.run_steps(4);
            let at = if near_edge { pos2(tool_rect.left() - 1.0, tool_rect.center().y) } else { tool_rect.center() };
            for _ in 0..2 {
                h.input_mut().events.extend([
                    egui::Event::PointerMoved(at),
                    egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
                    egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
                ]);
            }
            h.run_steps(1);
            assert_eq!(h.state().ui.tool, Tool::Move, "mode={mode}, near_edge={near_edge}");
            assert_eq!(h.state().ui.views[0].zoom, 0.5, "mode={mode}, near_edge={near_edge}");
        }
    }
}

/// Scrolled-out tool geometry never receives raw release replay beyond its clipped hit rectangle.
#[test]
fn review_replay_respects_scroll_clipping() {
    let reference = harness();
    let at = reference.get_by_label("Zoom Tool  (Z)").rect().center();
    let mut ready = false;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width":8,"height":8})).unwrap();
    app.ui.views[0].fit_pending = false;
    app.ui.views[0].zoom = 0.5;
    let mut h = Harness::builder().with_size(vec2(1200.0, 400.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if !ready {
                PhotocraftApp::setup_context(ui.ctx(), theme::ThemeKind::Studio);
                ready = true;
                return;
            }
            toolbar(app, ui);
        },
        app,
    );
    h.run_steps(4);
    for _ in 0..2 {
        h.input_mut().events.extend([
            egui::Event::PointerMoved(at),
            egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
            egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
        ]);
    }
    h.run_steps(1);
    assert_eq!(h.state().ui.tool, Tool::Move);
    assert_eq!(h.state().ui.views[0].zoom, 0.5);
}

/// An enabling/unclipping frame must not replay input rejected by the toolkit's previous layout.
#[test]
fn review_replay_respects_previous_pass_eligibility() {
    let reference = harness();
    let at = reference.get_by_label("Zoom Tool  (Z)").rect().center();
    let blocked_id = egui::Id::unique("toolbar-test-blocked");
    for clip in [false, true] {
        let mut ready = false;
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width":8,"height":8})).unwrap();
        app.ui.views[0].fit_pending = false;
        app.ui.views[0].zoom = 0.5;
        let mut h = Harness::builder().with_size(vec2(1200.0, 1500.0)).with_step_dt(1.0 / 60.0).build_ui_state(
            move |ui, app: &mut PhotocraftApp| {
                if !ready {
                    PhotocraftApp::setup_context(ui.ctx(), theme::ThemeKind::Studio);
                    ready = true;
                    return;
                }
                if ui.data(|d| d.get_temp::<bool>(blocked_id)).unwrap_or(true) {
                    if clip {
                        ui.set_clip_rect(Rect::from_min_max(Pos2::ZERO, pos2(1200.0, 400.0)));
                    } else {
                        ui.disable();
                    }
                }
                toolbar(app, ui);
            },
            app,
        );
        h.run_steps(4);
        h.ctx.data_mut(|d| d.insert_temp(blocked_id, false));
        for _ in 0..2 {
            h.input_mut().events.extend([
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
                egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
            ]);
        }
        h.run_steps(1);
        assert_eq!(h.state().ui.tool, Tool::Move, "clip={clip}");
        assert_eq!(h.state().ui.views[0].zoom, 0.5, "clip={clip}");
        // Once both layouts are eligible, existing ordinary clicks must still work.
        for pair in 0..2 {
            pointer(&mut h, at, true, egui::Modifiers::NONE);
            pointer(&mut h, at, false, egui::Modifiers::NONE);
            assert_eq!(h.state().ui.tool, Tool::Zoom);
            assert_eq!(h.state().ui.views[0].zoom, if pair == 1 { 1.0 } else { 0.5 });
        }
    }
}

/// Accessibility names the activated widget, preserves event order and ignores foreign trees.
#[test]
fn review_accessibility_activation_obeys_event_order_and_tree() {
    for after_release in [false, true] {
        let mut h = harness();
        let at = h.get_by_label("Zoom Tool  (Z)").rect().center();
        let move_rect = h.get_by_label("Move Tool  (V)").rect();
        let id = h.ctx.viewport(|v| {
            v.prev_pass.widgets.layers().flat_map(|(_, widgets)| widgets.iter()).find(|w| w.sense.senses_click() && w.rect == move_rect).map(|w| w.id).unwrap()
        });
        let activation = egui::Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
            action: egui::accesskit::Action::Click,
            target_tree: egui::accesskit::TreeId::ROOT,
            target_node: id.accesskit_id(),
            data: None,
        });
        let click = [
            egui::Event::PointerMoved(at),
            egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
            egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
        ];
        if after_release {
            h.input_mut().events.extend(click);
            h.input_mut().events.push(activation);
        } else {
            h.input_mut().events.push(activation);
            h.input_mut().events.extend(click);
        }
        h.run_steps(1);
        assert_eq!(h.state().ui.tool, if after_release { Tool::Move } else { Tool::Zoom });
        // A request into another tree must not select this tree's coincident node id.
        crate::tool_memory::select(h.state_mut(), Tool::Zoom);
        h.event(egui::Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
            action: egui::accesskit::Action::Click,
            target_tree: egui::accesskit::TreeId("00000000-0000-0000-0000-000000000001".parse().unwrap()),
            target_node: id.accesskit_id(),
            data: None,
        }));
        h.run_steps(1);
        assert_eq!(h.state().ui.tool, Tool::Zoom);
    }
}

/// The web bridge emits an emulated up for any ended contact, even while the owner stays down.
/// A secondary lift over a held row must not choose that row before the owning finger releases.
#[test]
fn integration_web_secondary_end_cannot_release_the_held_owner() {
    let mut h = harness();
    crate::tool_memory::select(h.state_mut(), Tool::Brush);
    h.run_steps(1);
    let at = h.get_by_label("Brush Tool  (B)").rect().center();
    touch(&mut h, at, egui::TouchPhase::Start, true);
    h.run_steps(24);
    assert!(flyout_open(&h));
    let (key, _) = h.ctx.data(|d| d.get_temp::<(egui::Id, Rect)>(egui::Id::unique("tool-flyout"))).unwrap();
    let area = h.ctx.memory(|m| m.area_rect(key.with("flyout"))).unwrap();
    let margin = egui::Frame::popup(&h.ctx.global_style()).total_margin();
    let gap = (area.height() - margin.top - margin.bottom - 4.0 * 26.0) / 3.0;
    let pencil = pos2(area.center().x, area.top() + margin.top + 26.0 + gap + 13.0);
    let other = at + vec2(300.0, 0.0);
    h.input_mut().events.extend([
        egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
        egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(2), phase: egui::TouchPhase::Start, pos: other, force: None },
    ]);
    h.run_steps(1);
    h.input_mut().events.extend([
        egui::Event::PointerMoved(pencil),
        egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(1), phase: egui::TouchPhase::Move, pos: pencil, force: None },
    ]);
    h.run_steps(1);
    h.input_mut().events.extend([
        egui::Event::PointerButton { pos: pencil, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
        egui::Event::PointerGone,
        egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(2), phase: egui::TouchPhase::End, pos: other, force: None },
    ]);
    h.run_steps(1);
    assert_eq!(h.state().ui.tool, Tool::Brush, "a different finger's up cannot select the owner's row");
    assert!(flyout_open(&h), "the owning contact is still held");
    touch(&mut h, pencil, egui::TouchPhase::End, true);
    assert_eq!(h.state().ui.tool, Tool::Pencil, "the real owner still selects its held row");
    assert!(!flyout_open(&h));
}

/// A secondary web lift before the deadline must not remove the owner's pending hold.
/// Once that original deadline passes, its later row release still selects exactly once.
#[test]
fn integration_web_secondary_end_preserves_the_pending_hold() {
    let mut h = harness();
    crate::tool_memory::select(h.state_mut(), Tool::Move);
    h.run_steps(1);
    let at = h.get_by_label("Brush Tool  (B)").rect().center();
    touch(&mut h, at, egui::TouchPhase::Start, true);
    h.run_steps(10);
    let other = at + vec2(300.0, 0.0);
    h.input_mut().events.extend([
        egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
        egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(2), phase: egui::TouchPhase::Start, pos: other, force: None },
    ]);
    h.run_steps(1);
    h.input_mut().events.extend([
        egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
        egui::Event::PointerGone,
        egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(2), phase: egui::TouchPhase::End, pos: other, force: None },
    ]);
    h.run_steps(1);
    assert_eq!(h.state().ui.tool, Tool::Move, "the owner has not completed a selection");
    h.run_steps(14);
    assert!(flyout_open(&h), "the original contact still owns its hold deadline");
    assert_eq!(h.state().ui.tool, Tool::Brush, "holding still selects the displayed tool");
    let (key, _) = h.ctx.data(|d| d.get_temp::<(egui::Id, Rect)>(egui::Id::unique("tool-flyout"))).unwrap();
    let area = h.ctx.memory(|m| m.area_rect(key.with("flyout"))).unwrap();
    let margin = egui::Frame::popup(&h.ctx.global_style()).total_margin();
    let gap = (area.height() - margin.top - margin.bottom - 4.0 * 26.0) / 3.0;
    let pencil = pos2(area.center().x, area.top() + margin.top + 26.0 + gap + 13.0);
    touch(&mut h, pencil, egui::TouchPhase::End, true);
    assert_eq!(h.state().ui.tool, Tool::Pencil);
    assert!(!flyout_open(&h));
}

/// EgUI's interaction radius intentionally accepts a button click just outside its painted box.
/// Retain both ordinary tool selection and positive view pairs for those acknowledged clicks.
#[test]
fn integration_acknowledged_near_edge_clicks_select_tools_and_run_view_pairs() {
    let mut h = harness();
    h.state_mut().run("file.new", json!({"width": 8, "height": 8})).unwrap();
    h.state_mut().ui.views[0].fit_pending = false;
    h.state_mut().ui.views[0].zoom = 0.5;
    let rect = h.get_by_label("Brush Tool  (B)").rect();
    let at = pos2(rect.left() - 1.0, rect.center().y);
    let id =
        h.ctx.viewport(|v| v.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()).find(|w| w.rect == rect && w.sense.senses_click()).map(|w| w.id).unwrap());
    assert!(!rect.contains(at));
    pointer(&mut h, at, true, egui::Modifiers::NONE);
    pointer(&mut h, at, false, egui::Modifiers::NONE);
    assert!(h.ctx.read_response(id).unwrap().clicked_by(egui::PointerButton::Primary), "the intended slot acknowledged the near hit");
    assert_eq!(h.state().ui.tool, Tool::Brush, "retain the toolkit's acknowledged near hit");
    let rect = h.get_by_label("Zoom Tool  (Z)").rect();
    let at = pos2(rect.left() - 1.0, rect.center().y);
    for pair in 0..2 {
        pointer(&mut h, at, true, egui::Modifiers::NONE);
        pointer(&mut h, at, false, egui::Modifiers::NONE);
        assert_eq!(h.state().ui.tool, Tool::Zoom);
        assert_eq!(h.state().ui.views[0].zoom, if pair == 1 { 1.0 } else { 0.5 });
    }
}

/// Several releases in one frame still use egui's nearest-widget radius at each position.
/// The final response proves the intended slot accepted the hit; the document stays untouched.
#[test]
fn integration_batched_near_edge_pairs_keep_the_toolkit_hit_radius() {
    for tool in [Tool::Hand, Tool::Zoom] {
        for alt_first in [false, true] {
            let mut h = harness();
            h.state_mut().run("file.new", json!({"width": 8, "height": 8})).unwrap();
            crate::tool_memory::select(h.state_mut(), if alt_first && tool == Tool::Hand { Tool::RotateView } else { Tool::Move });
            h.state_mut().ui.views[0].fit_pending = false;
            h.state_mut().ui.views[0].zoom = 0.5;
            h.run_steps(1);
            let label = if tool == Tool::Zoom {
                "Zoom Tool  (Z)"
            } else if alt_first {
                "Rotate View Tool  (R)"
            } else {
                "Hand Tool  (H)"
            };
            let rect = h.get_by_label(label).rect();
            let id =
                h.ctx.viewport(|v| v.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()).find(|w| w.rect == rect && w.sense.senses_click()).unwrap().id);
            let original = h.state().session.active().unwrap().clone();
            let sequence = if alt_first { vec![egui::Modifiers::ALT, egui::Modifiers::NONE, egui::Modifiers::NONE] } else { vec![egui::Modifiers::NONE; 2] };
            for (i, modifiers) in sequence.into_iter().enumerate() {
                let at = pos2(rect.left() - 1.0, rect.center().y + i as f32 * 0.25);
                assert!(!rect.contains(at));
                h.input_mut().events.push(egui::Event::PointerMoved(at));
                for pressed in [true, false] {
                    h.input_mut().events.push(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers });
                }
            }
            h.run_steps(1);
            assert!(h.ctx.read_response(id).unwrap().clicked_by(egui::PointerButton::Primary), "the pinned toolkit accepted the batched near hit");
            assert_eq!(h.state().ui.tool, tool, "tool={tool:?}, alt_first={alt_first}");
            assert_eq!(h.state().ui.views[0].zoom, if tool == Tool::Zoom { 1.0 } else { 0.5 });
            assert_eq!(h.state().ui.views[0].fit_pending, tool == Tool::Hand);
            let doc = h.state().session.active().unwrap();
            assert!(std::sync::Arc::ptr_eq(&doc.doc, &original.doc));
            assert_eq!(doc.revision, original.revision);
            assert_eq!(doc.history.entries(), original.history.entries());
        }
    }
}

/// Nearby releases can target different slots, so the final widget acknowledgment cannot
/// authorize every earlier release. A configured smaller hit radius must also remain effective.
#[test]
fn integration_batched_near_hits_respect_distinct_slots_and_configured_radius() {
    for small_radius in [false, true] {
        let mut h = harness();
        h.state_mut().run("file.new", json!({"width": 8, "height": 8})).unwrap();
        h.state_mut().ui.views[0].fit_pending = false;
        h.state_mut().ui.views[0].zoom = 0.5;
        if small_radius {
            h.ctx.global_style_mut(|s| s.interaction.interact_radius = 0.5);
        }
        h.run_steps(1);
        let hand = h.get_by_label("Hand Tool  (H)").rect();
        let zoom = h.get_by_label("Zoom Tool  (Z)").rect();
        let points = if small_radius {
            [pos2(zoom.left() - 1.0, zoom.center().y); 2]
        } else {
            assert!(hand.bottom() < zoom.top());
            [pos2(hand.center().x, hand.bottom() + 0.25), pos2(zoom.center().x, zoom.top() - 0.25)]
        };
        for at in points {
            h.input_mut().events.push(egui::Event::PointerMoved(at));
            for pressed in [true, false] {
                h.input_mut().events.push(egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                });
            }
        }
        h.run_steps(1);
        assert_eq!(h.state().ui.tool, if small_radius { Tool::Move } else { Tool::Zoom });
        assert_eq!(h.state().ui.views[0].zoom, 0.5, "distinct slots cannot become a Zoom pair");
        assert!(!h.state().ui.views[0].fit_pending);
    }
}

/// The popup margin owns a press outside its child row, so that point is not a near-edge hit.
/// Preserve that non-selection, then prove a real Pencil row click still selects and closes it.
#[test]
fn integration_unacknowledged_popup_margin_preserves_selection_and_real_row_clicks() {
    let mut h = harness();
    h.state_mut().run("file.new", json!({"width": 8, "height": 8})).unwrap();
    h.state_mut().ui.views[0].fit_pending = false;
    h.state_mut().ui.views[0].zoom = 0.5;
    let original = h.state().session.active().unwrap().clone();
    let at = h.get_by_label("Brush Tool  (B)").rect().center();
    for pressed in [true, false] {
        h.input_mut().events.extend([
            egui::Event::PointerMoved(at),
            egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: egui::Modifiers::NONE },
        ]);
        h.run_steps(1);
    }
    h.run_steps(1);
    assert!(flyout_open(&h));
    let (key, _) = h.ctx.data(|d| d.get_temp::<(egui::Id, Rect)>(egui::Id::unique("tool-flyout"))).unwrap();
    let area = h.ctx.memory(|m| m.area_rect(key.with("flyout"))).unwrap();
    let (row_id, row) = h.ctx.viewport(|v| {
        v.prev_pass
            .widgets
            .get_layer(egui::LayerId::new(egui::Order::Foreground, key.with("flyout")))
            .filter(|w| w.sense.senses_click() && w.rect.height() == 26.0)
            .nth(1)
            .map(|w| (w.id, w.rect))
            .unwrap()
    });
    let margin = pos2(row.left() - 1.0, row.center().y);
    assert!(area.contains(margin) && !row.contains(margin));
    pointer(&mut h, margin, true, egui::Modifiers::NONE);
    pointer(&mut h, margin, false, egui::Modifiers::NONE);
    assert!(!h.ctx.read_response(row_id).unwrap().clicked_by(egui::PointerButton::Primary), "the child row did not acknowledge the margin click");
    assert_eq!(h.state().ui.tool, Tool::Move);
    assert!(flyout_open(&h));
    pointer(&mut h, row.center(), true, egui::Modifiers::NONE);
    pointer(&mut h, row.center(), false, egui::Modifiers::NONE);
    assert!(h.ctx.read_response(row_id).unwrap().clicked_by(egui::PointerButton::Primary), "the inside row click is acknowledged");
    assert_eq!(h.state().ui.tool, Tool::Pencil);
    assert!(!flyout_open(&h));
    assert_eq!(h.state().ui.views[0].zoom, 0.5);
    assert!(!h.state().ui.views[0].fit_pending);
    let doc = h.state().session.active().unwrap();
    assert!(std::sync::Arc::ptr_eq(&doc.doc, &original.doc));
    assert_eq!(doc.revision, original.revision);
    assert_eq!(doc.history.entries(), original.history.entries());
}

/// Native metadata can precede its up; a web callback can end several contacts after the up.
/// Both orders must still deliver the real owner's held choice exactly once.
#[test]
fn integration_real_owner_end_survives_batched_contact_metadata() {
    for web in [false, true] {
        let mut h = harness();
        let at = h.get_by_label("Brush Tool  (B)").rect().center();
        touch(&mut h, at, egui::TouchPhase::Start, web);
        h.run_steps(24);
        let (key, _) = h.ctx.data(|d| d.get_temp::<(egui::Id, Rect)>(egui::Id::unique("tool-flyout"))).unwrap();
        let area = h.ctx.memory(|m| m.area_rect(key.with("flyout"))).unwrap();
        let margin = egui::Frame::popup(&h.ctx.global_style()).total_margin();
        let gap = (area.height() - margin.top - margin.bottom - 4.0 * 26.0) / 3.0;
        let pencil = pos2(area.center().x, area.top() + margin.top + 26.0 + gap + 13.0);
        let end = |id, pos| egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(id), phase: egui::TouchPhase::End, pos, force: None };
        let up = egui::Event::PointerButton { pos: pencil, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE };
        let other = at + vec2(300.0, 0.0);
        h.input_mut().events.push(egui::Event::Touch {
            device_id: egui::TouchDeviceId(1),
            id: egui::TouchId(2),
            phase: egui::TouchPhase::Start,
            pos: other,
            force: None,
        });
        h.run_steps(1);
        h.event(egui::Event::PointerMoved(pencil));
        h.run_steps(1);
        if web {
            h.input_mut().events.extend([up, egui::Event::PointerGone, end(2, other), end(1, pencil)]);
        } else {
            h.input_mut().events.extend([end(1, pencil), up, egui::Event::PointerGone, end(2, other)]);
        }
        h.run_steps(1);
        assert_eq!(h.state().ui.tool, Tool::Pencil, "the owner completed its release; web={web}");
        assert!(!flyout_open(&h));
    }
}

/// An extra web Start reuses the owner's current position even after a drag crosses another slot.
/// The toolkit's repeated down cannot transfer the original contact's held popup to that slot.
#[test]
fn integration_duplicate_web_down_cannot_retarget_the_original_hold() {
    let mut h = harness();
    crate::tool_memory::select(h.state_mut(), Tool::Brush);
    h.run_steps(1);
    let at = h.get_by_label("Brush Tool  (B)").rect().center();
    touch(&mut h, at, egui::TouchPhase::Start, true);
    h.run_steps(24);
    let (key, _) = h.ctx.data(|d| d.get_temp::<(egui::Id, Rect)>(egui::Id::unique("tool-flyout"))).unwrap();
    let area = h.ctx.memory(|m| m.area_rect(key.with("flyout"))).unwrap();
    let margin = egui::Frame::popup(&h.ctx.global_style()).total_margin();
    let gap = (area.height() - margin.top - margin.bottom - 4.0 * 26.0) / 3.0;
    let pencil = pos2(area.center().x, area.top() + margin.top + 26.0 + gap + 13.0);
    let eraser = h.get_by_label("Eraser Tool  (E)").rect().center();
    h.input_mut().events.extend([
        egui::Event::PointerMoved(eraser),
        egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(1), phase: egui::TouchPhase::Move, pos: eraser, force: None },
    ]);
    h.run_steps(1);
    h.input_mut().events.extend([
        egui::Event::PointerButton { pos: eraser, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
        egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(2), phase: egui::TouchPhase::Start, pos: at + vec2(300.0, 0.0), force: None },
    ]);
    h.run_steps(1);
    assert_eq!(h.ctx.data(|d| d.get_temp::<(egui::Id, Rect)>(egui::Id::unique("tool-flyout"))).map(|(k, _)| k), Some(key));
    assert_eq!(h.state().ui.tool, Tool::Brush);
    touch(&mut h, pencil, egui::TouchPhase::End, true);
    assert_eq!(h.state().ui.tool, Tool::Pencil);
    assert!(!flyout_open(&h));
}
