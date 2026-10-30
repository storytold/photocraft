//! Pointer regressions for the main toolbar, separate from shortcut-key groups.
use super::*;
use crate::{panels::options_bar::options_bar, theme};
use egui_kittest::{Harness, kittest::Queryable};

/// Build the default Studio fixture with accessibility checks enabled.
fn harness() -> Harness<'static, PhotocraftApp> {
    configured_harness(theme::ThemeKind::Studio, false, 1.0, false)
}

/// Settle a toolbar fixture at the requested theme, column count and display scale.
fn configured_harness(kind: theme::ThemeKind, double: bool, scale: f32, gpu: bool) -> Harness<'static, PhotocraftApp> {
    let mut ready = false;
    let mut builder = Harness::builder().with_size(vec2(1200.0, 1500.0)).with_pixels_per_point(scale).with_step_dt(1.0 / 60.0);
    if gpu {
        // Capture options-bar context as well as the toolbar. Its existing spin-button
        // naming is outside this gesture test; toolbar-only interaction fixtures keep
        // the accessibility checker enabled across every theme, layout and scale.
        builder = builder.wgpu().with_accessibility_check(false);
    }
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.ui.panels.toolbar_double = double;
    let mut h = builder.build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if !ready {
                PhotocraftApp::setup_context(ui.ctx(), kind);
                ready = true;
                return;
            }
            if gpu {
                options_bar(app, ui);
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

/// Require release before advancing, including wraparound, without a second selection next frame.
#[test]
fn alt_click_cycles_the_visible_group_and_wraps_once_per_click() {
    let mut h = harness();
    let at = h.get_by_label("Brush Tool  (B)").rect().center();
    for expected in [Tool::Pencil, Tool::MixerBrush, Tool::ColorReplacement, Tool::Brush, Tool::Pencil] {
        let before = h.state().ui.tool;
        pointer(&mut h, at, true, egui::Modifiers::ALT);
        assert_eq!(h.state().ui.tool, before, "selection waits for a completed click");
        pointer(&mut h, at, false, egui::Modifiers::ALT);
        assert_eq!(h.state().ui.tool, expected);
        h.run_steps(1);
        assert_eq!(h.state().ui.tool, expected, "the next frame cannot select the previous icon");
    }
}

/// Cycle from the icon a dormant group displays rather than from the active tool elsewhere.
#[test]
fn inactive_group_starts_from_its_remembered_icon() {
    let mut h = harness();
    crate::tool_memory::select(h.state_mut(), Tool::Pencil);
    h.run_steps(1);
    let at = h.get_by_label("Pencil Tool  (B)").rect().center();
    crate::tool_memory::select(h.state_mut(), Tool::Move);
    h.run_steps(1);
    pointer(&mut h, at, true, egui::Modifiers::ALT);
    pointer(&mut h, at, false, egui::Modifiers::ALT);
    assert_eq!(h.state().ui.tool, Tool::MixerBrush);
}

/// Keep visible flyout order authoritative when letter shortcuts omit tools or are rebound.
#[test]
fn marquee_cycle_includes_tools_without_shortcuts_and_ignores_rebindings() {
    let mut h = harness();
    h.state_mut().session.prefs.edit(|p| {
        p.shortcuts.insert("tools.select.ellipseMarquee".into(), "F6".into());
        p.shortcuts.insert("tools.select.rectMarquee".into(), String::new());
    });
    let at = h.get_by_label("Rectangular Marquee Tool  (M)").rect().center();
    for expected in [Tool::EllipseMarquee, Tool::SingleRowMarquee, Tool::SingleColumnMarquee, Tool::RectMarquee] {
        pointer(&mut h, at, true, egui::Modifiers::ALT);
        pointer(&mut h, at, false, egui::Modifiers::ALT);
        assert_eq!(h.state().ui.tool, expected);
    }
}

/// Exercise stale hidden memory, a single visible choice and a completely hidden group.
#[test]
fn hidden_members_singletons_and_stale_memory_follow_the_visible_slot() {
    let mut h = harness();
    crate::tool_memory::select(h.state_mut(), Tool::Pencil);
    h.run_steps(1);
    crate::tool_memory::select(h.state_mut(), Tool::Move);
    h.state_mut().session.prefs.edit(|p| p.toolbar.hidden = vec!["Pencil".into(), "MixerBrush".into()]);
    h.run_steps(1);
    let at = h.get_by_label("Brush Tool  (B)").rect().center();
    pointer(&mut h, at, true, egui::Modifiers::ALT);
    pointer(&mut h, at, false, egui::Modifiers::ALT);
    assert_eq!(h.state().ui.tool, Tool::ColorReplacement, "hidden remembered Pencil falls back to Brush, then advances");
    h.state_mut().session.prefs.edit(|p| p.toolbar.hidden.push("ColorReplacement".into()));
    h.run_steps(1);
    let at = h.get_by_label("Brush Tool  (B)").rect().center();
    pointer(&mut h, at, true, egui::Modifiers::ALT);
    pointer(&mut h, at, false, egui::Modifiers::ALT);
    assert_eq!(h.state().ui.tool, Tool::Brush);
    h.state_mut().session.prefs.edit(|p| p.toolbar.hidden.push("Brush".into()));
    h.run_steps(1);
    assert!(h.query_by_label("Brush Tool  (B)").is_none());
}

/// Distinguish press-time Alt from release-time Alt and preserve ordinary keyboard selection.
#[test]
fn alt_is_latched_at_press_and_keyboard_activation_is_plain_selection() {
    let mut h = harness();
    let at = h.get_by_label("Brush Tool  (B)").rect().center();
    pointer(&mut h, at, true, egui::Modifiers::ALT);
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    h.run_steps(1);
    pointer(&mut h, at, false, egui::Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::Pencil);
    pointer(&mut h, at, true, egui::Modifiers::NONE);
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::ALT));
    h.run_steps(1);
    pointer(&mut h, at, false, egui::Modifiers::ALT);
    assert_eq!(h.state().ui.tool, Tool::Pencil, "adding Alt after press cannot turn a normal click into cycling");
    h.get_by_label("Pencil Tool  (B)").focus();
    h.key_press(egui::Key::Space);
    h.run_steps(2);
    assert_eq!(h.state().ui.tool, Tool::Pencil);
}

/// Preserve the initiating Alt state when press and release arrive in the same input frame.
#[test]
fn a_same_frame_click_uses_its_press_modifiers() {
    let mut h = harness();
    let at = h.get_by_label("Brush Tool  (B)").rect().center();
    h.input_mut().events.extend([
        egui::Event::PointerMoved(at),
        egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::ALT },
        egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
    ]);
    h.run_steps(1);
    assert_eq!(h.state().ui.tool, Tool::Pencil);
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

/// Close a same-group or other-group flyout before its rows can consume the cycling release.
#[test]
fn alt_click_closes_an_open_flyout_and_cycles_only_once() {
    for other in [false, true] {
        let mut h = harness();
        let brush = h.get_by_label("Brush Tool  (B)").rect().center();
        let at = if other { h.get_by_label("Rectangular Marquee Tool  (M)").rect().center() } else { brush };
        h.event(egui::Event::PointerMoved(at));
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed: false, modifiers: egui::Modifiers::NONE });
        h.run_steps(3);
        assert!(flyout_open(&h));
        pointer(&mut h, brush, true, egui::Modifiers::ALT);
        pointer(&mut h, brush, false, egui::Modifiers::ALT);
        assert_eq!(h.state().ui.tool, Tool::Pencil);
        assert!(!flyout_open(&h));
    }
}

/// Clear spring/stylus overrides, restore the chosen tool and preserve pixels, history and options.
#[test]
fn explicit_cycle_clears_temporary_tools_and_persists_without_document_changes() {
    for stylus in [false, true] {
        let mut h = harness();
        h.state_mut().run("file.new", json!({"width": 8, "height": 8})).unwrap();
        let original = h.state().session.active().unwrap().clone();
        let options = h.state().ui.tool_options.clone();
        if stylus {
            h.state_mut().ui.tool = Tool::Eraser;
            h.state_mut().stylus.tool_before_eraser = Some(Tool::Move);
        } else {
            crate::spring_tools::pressed(h.state_mut(), 'B', Tool::Move, true, 0.0);
            assert!(h.state().ui.spring.is_some());
        }
        let at = h.get_by_label("Brush Tool  (B)").rect().center();
        pointer(&mut h, at, true, egui::Modifiers::ALT);
        pointer(&mut h, at, false, egui::Modifiers::ALT);
        assert_eq!(h.state().ui.tool, Tool::Pencil);
        assert!(h.state().ui.spring.is_none());
        assert!(h.state().stylus.tool_before_eraser.is_none());
        crate::tool_memory::persist(h.state_mut(), &egui::Context::default());
        let prefs = h.state().session.prefs().clone();
        let doc = h.state().session.active().unwrap();
        assert!(std::sync::Arc::ptr_eq(&original.doc, &doc.doc));
        assert_eq!(doc.revision, original.revision);
        assert_eq!(doc.history.entries(), original.history.entries());
        assert_eq!(doc.history.past_len(), original.history.past_len());
        assert_eq!(doc.history.redo_labels().collect::<Vec<_>>(), original.history.redo_labels().collect::<Vec<_>>());
        assert_eq!(h.state().ui.tool_options, options);
        let mut restored = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        restored.session.prefs.edit(|p| {
            p.selected_tool = prefs.selected_tool;
            p.tool_options = prefs.tool_options;
        });
        crate::tool_memory::restore(&mut restored);
        assert_eq!(restored.ui.tool, Tool::Pencil);
        assert_eq!(restored.ui.tool_options, options);
    }
}

/// Preserve every Pen starting tool until the documented exceptions have precise reference evidence.
#[test]
fn pen_group_is_preserved_pending_reference_evidence_for_the_exceptions() {
    // Adobe lists three exceptions without saying whether they are skipped or suppress the
    // gesture from that starting tool. Do not encode an unverified interpretation as parity.
    for tool in [Tool::Pen, Tool::FreeformPen, Tool::CurvaturePen, Tool::AddAnchorPoint, Tool::DeleteAnchorPoint, Tool::ConvertPoint] {
        let mut h = harness();
        crate::tool_memory::select(h.state_mut(), tool);
        h.run_steps(1);
        let label = match crate::tool_shortcuts::shortcut(h.state(), tool) {
            Some(sc) => format!("{}  ({})", tool.label(), crate::shortcuts::pretty(&sc)),
            None => tool.label().to_string(),
        };
        let at = h.get_by_label(&label).rect().center();
        pointer(&mut h, at, true, egui::Modifiers::ALT);
        pointer(&mut h, at, false, egui::Modifiers::ALT);
        assert_eq!(h.state().ui.tool, tool);
    }
}

/// Keep the accessible button in place through cycling across the supported layout matrix.
#[test]
fn cycling_uses_accessible_buttons_in_both_layouts_themes_scales_and_directions() {
    for kind in theme::ThemeKind::ALL {
        for double in [false, true] {
            for scale in [1.0, 1.5, 2.0] {
                for dir in [crate::direction::Dir::Ltr, crate::direction::Dir::Rtl] {
                    let _direction = crate::direction::direction_scope(dir);
                    let mut h = configured_harness(kind, double, scale, false);
                    let at = h.get_by_label("Brush Tool  (B)").rect().center();
                    pointer(&mut h, at, true, egui::Modifiers::ALT);
                    pointer(&mut h, at, false, egui::Modifiers::ALT);
                    assert_eq!(h.state().ui.tool, Tool::Pencil, "{kind:?}, double={double}, scale={scale}, {dir:?}");
                    h.run_steps(1); // The release frame painted the previous icon before selecting the next tool.
                    let rect = h.get_by_label("Pencil Tool  (B)").rect();
                    assert!(rect.contains(at), "cycling must keep its button in place");
                    assert!(rect.left() >= 0.0 && rect.right() <= 1200.0 && rect.top() >= 0.0 && rect.bottom() <= 1500.0);
                }
            }
        }
    }
}

/// Optional real native-wgpu evidence from the same pointer path as the ordinary regressions.
#[test]
fn gpu_capture_toolbar_cycle() {
    let Some(directory) = std::env::var_os("PHOTOCRAFT_TOOLBAR_CYCLE_SNAPSHOTS").map(std::path::PathBuf::from) else { return };
    let _gpu = photocraft_testkit::gpu_lock();
    std::fs::create_dir_all(&directory).unwrap();
    for kind in theme::ThemeKind::ALL {
        for double in [false, true] {
            for scale in [1.0, 1.5, 2.0] {
                let mut h = configured_harness(kind, double, scale, true);
                let at = h.get_by_label("Brush Tool  (B)").rect().center();
                let name = format!("{}-{}col-{scale}x", kind.id(), if double { 2 } else { 1 });
                h.render().unwrap().save(directory.join(format!("{name}-before.png"))).unwrap();
                pointer(&mut h, at, true, egui::Modifiers::ALT);
                pointer(&mut h, at, false, egui::Modifiers::ALT);
                assert_eq!(h.state().ui.tool, Tool::Pencil);
                h.run_steps(1);
                let image = h.render().unwrap();
                assert_eq!((image.width(), image.height()), ((1200.0 * scale) as u32, (1500.0 * scale) as u32));
                image.save(directory.join(format!("{name}-after.png"))).unwrap();
            }
        }
    }
}

/// Handle empty, singleton and stale helper inputs without unchecked indexing.
#[test]
fn next_tool_handles_empty_and_stale_groups_without_indexing() {
    assert_eq!(next_toolbar_tool(&[], Tool::Brush), None);
    assert_eq!(next_toolbar_tool(&[Tool::Brush], Tool::Move), Some(Tool::Brush));
    assert_eq!(next_toolbar_tool(&[Tool::Brush], Tool::Brush), Some(Tool::Brush));
}

/// Prevent rapid modifier-clicks from fitting the view or restoring actual-pixel zoom.
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
                    if !hand {
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
    assert_eq!(h.state().ui.tool, Tool::Pencil);
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
