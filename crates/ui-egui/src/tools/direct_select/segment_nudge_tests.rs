//! Segment keyboard edits go through actual hit selection and the existing anchor command.

use super::*;
use crate::canvas::tool_event;

/// A closed path whose first segment is curved and last segment wraps to the first knot.
/// Distinct external handles expose accidental bending or movement of unrelated knots.
fn new_app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 240, "height": 240})).unwrap();
    app.run(
        "path.set",
        json!({"name":"work", "path":{"subpaths":[{"closed":true,"knots":[
            {"anchor":[40,40],"in":[20,80],"out":[60,0],"smooth":true},
            {"anchor":[160,40],"in":[140,0],"out":[180,80],"smooth":true},
            [160,160], [40,160]
        ]}]}}),
    )
    .unwrap();
    app.ui.tool = Tool::DirectSelection;
    app.ui.extras.snap = false;
    app
}

/// Copies document geometry independently of the UI selection's indices.
fn work(app: &PhotocraftApp) -> Path {
    app.session.active().unwrap().doc.work_path.clone().unwrap()
}

/// A completed canvas click must select the segment rather than inject a synthetic selection.
fn click_segment(app: &mut PhotocraftApp, at: [f64; 2], segment: [usize; 2]) {
    tool_event(app, ToolEvent::Down { x: at[0], y: at[1], pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x: at[0], y: at[1] }, Modifiers::NONE);
    assert_eq!(app.ui.direct_selection.segment, Some(segment));
    assert!(app.ui.direct_selection.anchors.is_empty());
    assert!(app.ui.direct_selection.drag.is_none());
    assert!(still_valid(app, &app.ui.direct_selection));
}

/// Delivers a keyboard event through the canvas nudge dispatcher, with no focused widget.
fn arrow(app: &mut PhotocraftApp, key: egui::Key, modifiers: Modifiers) -> bool {
    let ctx = egui::Context::default();
    ctx.input_mut(|i| {
        i.modifiers = modifiers;
        i.events.push(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
    });
    crate::move_mods::arrow_keys(app, &ctx)
}

/// Delivers a canvas-owned key through normal shortcut dispatch, retaining the shared modal
/// and tool-specific ownership checks rather than calling the Direct Selection handler alone.
fn canvas_shortcut(app: &mut PhotocraftApp, key: egui::Key) {
    let ctx = egui::Context::default();
    let input = egui::RawInput {
        events: vec![egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers: Modifiers::NONE }],
        ..Default::default()
    };
    ctx.run_ui(input, |ui| {
        assert_eq!(crate::shortcut_dispatch::Focus::of(ui.ctx()), crate::shortcut_dispatch::Focus::None);
        crate::shortcuts::handle(app, ui.ctx());
    })
    .textures_delta
    .clear();
}

/// Checks complete knot geometry, including outside handles and every unaffected neighbor.
/// Expected coordinates are computed here rather than by the edit implementation under test.
fn assert_translation(before: &Path, after: &Path, ends: [usize; 2], delta: [f64; 2]) {
    let mut expected = before.clone();
    for index in ends {
        let knot = &mut expected.subpaths[0].knots[index];
        for point in [&mut knot.anchor, &mut knot.in_ctrl, &mut knot.out_ctrl] {
            point.x += delta[0];
            point.y += delta[1];
        }
    }
    assert_eq!(*after, expected);
}

/// Straight, curved and closed-last clicks all move both endpoint knots by document pixels,
/// regardless of zoom; each press is undoable and retains the segment for a repeated nudge.
#[test]
fn clicked_segments_nudge_in_each_direction_and_keep_their_curve() {
    use egui::Key;
    for (at, segment, ends) in [([100.0, 10.0], [0, 0], [0, 1]), ([100.0, 160.0], [0, 2], [2, 3]), ([32.5, 115.0], [0, 3], [3, 0])] {
        for (key, unit) in [(Key::ArrowLeft, [-1.0, 0.0]), (Key::ArrowRight, [1.0, 0.0]), (Key::ArrowUp, [0.0, -1.0]), (Key::ArrowDown, [0.0, 1.0])] {
            for (mods, step) in [(Modifiers::NONE, 1.0), (Modifiers::SHIFT, 10.0)] {
                let mut app = new_app();
                app.ui.views[0].zoom = 2.0;
                assert_eq!(app.current_zoom(), 2.0);
                click_segment(&mut app, at, segment);
                let before = work(&app);
                let n = app.session.active().unwrap().history.past_len();
                let delta = [unit[0] * step, unit[1] * step];
                assert!(arrow(&mut app, key, mods));
                assert_translation(&before, &work(&app), ends, delta);
                assert_eq!(app.session.active().unwrap().history.past_len(), n + 1);
                assert_eq!(app.ui.direct_selection.segment, Some(segment));
                assert!(still_valid(&app, &app.ui.direct_selection));
                assert!(arrow(&mut app, key, mods));
                assert_translation(&before, &work(&app), ends, [delta[0] * 2.0, delta[1] * 2.0]);
                assert_eq!(app.session.active().unwrap().history.past_len(), n + 2);
                assert!(app.session.undo());
                assert!(app.session.undo());
                assert_eq!(work(&app), before);
                assert!(app.session.redo());
                assert_translation(&before, &work(&app), ends, delta);
                app.sync_views();
                assert!(app.ui.direct_selection.target.is_none(), "external history changes invalidate selection");
            }
        }
    }
}

/// At 25% zoom the existing anchor and new segment routes keep document-pixel units, while
/// Move's shared dispatcher advances painted layer pixels by 4 document pixels per plain arrow.
/// Switching tools must preserve that distinction and never translate the unrelated target.
#[test]
fn path_nudges_keep_document_pixels_while_move_uses_screen_pixels() {
    let mut app = new_app();
    app.run("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("Paint nudge fixture", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(8, 8, 24, 24), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.sync_views();
    app.ui.views[0].zoom = 0.25;
    let id = app.session.active().unwrap().active_layer.unwrap();
    let pixel_bounds = |app: &PhotocraftApp| app.session.active().unwrap().doc.layer(id).unwrap().surface().unwrap().content_bounds();
    let pixels = app.session.active().unwrap().doc.layers.clone();
    let before = work(&app);
    let n = app.session.active().unwrap().history.past_len();
    click_segment(&mut app, [100.0, 10.0], [0, 0]);
    assert!(arrow(&mut app, egui::Key::ArrowRight, Modifiers::NONE));
    assert_translation(&before, &work(&app), [0, 1], [1.0, 0.0]);
    assert_eq!(app.session.active().unwrap().doc.layers, pixels);
    assert_eq!(app.session.active().unwrap().history.past_len(), n + 1);

    tool_event(&mut app, ToolEvent::Down { x: 41.0, y: 40.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 41.0, y: 40.0 }, Modifiers::NONE);
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 0]]);
    assert!(app.ui.direct_selection.segment.is_none());
    let mut expected = work(&app);
    let knot = &mut expected.subpaths[0].knots[0];
    for point in [&mut knot.anchor, &mut knot.in_ctrl, &mut knot.out_ctrl] {
        point.x += 1.0;
    }
    assert!(arrow(&mut app, egui::Key::ArrowRight, Modifiers::NONE));
    assert_eq!(work(&app), expected);
    assert_eq!(app.session.active().unwrap().doc.layers, pixels);
    assert_eq!(app.session.active().unwrap().history.past_len(), n + 2);

    app.ui.tool = Tool::Move;
    assert!(arrow(&mut app, egui::Key::ArrowRight, Modifiers::NONE));
    assert_eq!(pixel_bounds(&app), photocraft_geom::Rect::new(12, 8, 28, 24));
    assert_eq!(work(&app), expected);
    assert!(arrow(&mut app, egui::Key::ArrowLeft, Modifiers::SHIFT));
    assert_eq!(pixel_bounds(&app), photocraft_geom::Rect::new(2, 8, 18, 24));
    assert_eq!(work(&app), expected);
    assert_eq!(app.current_zoom(), 0.25);
    assert_eq!(app.ui.tool, Tool::Move);
}

/// Saved paths win over coincident work geometry, and nudging never alters that work path.
#[test]
fn a_saved_segment_nudge_targets_only_the_selected_saved_path() {
    let mut app = new_app();
    let path = app.run("path.info", json!({"name":"work"})).unwrap()["path"].clone();
    app.run("path.set", json!({"name":"Saved", "path":path})).unwrap();
    app.ui.selected_path = Some("Saved".into());
    click_segment(&mut app, [100.0, 10.0], [0, 0]);
    assert_eq!(app.ui.direct_selection.target, Some(PathRef::Saved(0)));
    let before = work(&app);
    let n = app.session.active().unwrap().history.past_len();
    assert!(arrow(&mut app, egui::Key::ArrowDown, Modifiers::SHIFT));
    assert_eq!(work(&app), before);
    assert_translation(&before, &app.session.active().unwrap().doc.paths[0].path, [0, 1], [0.0, 10.0]);
    assert_eq!(app.session.active().unwrap().history.past_len(), n + 1);
    assert!(app.session.undo());
    assert_eq!(app.session.active().unwrap().doc.paths[0].path, before);
}

/// PSD imports may contain a named path whose name the point-edit command reserves for the
/// Work Path. The selected segment must never redirect an arrow to unrelated geometry.
#[test]
fn a_saved_segment_with_a_reserved_name_cannot_nudge_the_work_path() {
    for name in ["WORK", "Work Path", ""] {
        let mut app = new_app();
        let before = work(&app);
        app.session
            .edit("Imported path fixture", |doc, _| {
                doc.paths.push(photocraft_doc::NamedPath { name: name.into(), path: before.clone(), psd_raw: None });
                Ok(())
            })
            .unwrap();
        app.sync_views();
        app.ui.selected_path = Some(name.into());
        click_segment(&mut app, [100.0, 10.0], [0, 0]);
        assert_eq!(app.ui.direct_selection.target, Some(PathRef::Saved(0)));
        let n = app.session.active().unwrap().history.past_len();
        assert!(!arrow(&mut app, egui::Key::ArrowRight, Modifiers::NONE));
        let st = app.session.active().unwrap();
        assert_eq!(st.doc.work_path.as_ref(), Some(&before), "{name:?}");
        assert_eq!(st.doc.paths[0].path, before, "{name:?}");
        assert_eq!(st.history.past_len(), n);
    }
}

/// An unfinished Pen segment has no engine path target. An arrow must neither commit its
/// geometry nor fall through to the existing Work Path at the same knot indices.
#[test]
fn an_unfinished_pen_segment_never_nudges_the_work_path() {
    let mut app = new_app();
    let before = work(&app);
    let n = app.session.active().unwrap().history.past_len();
    app.ui.tool = Tool::Pen;
    app.ui.tool_options.vector_mode = "path".into();
    // Stay clear of the Work Path so Pen Auto Add/Delete cannot edit it.
    for x in [80.0, 135.0] {
        tool_event(&mut app, ToolEvent::Down { x, y: 105.0, pressure: 1.0 }, Modifiers::NONE);
        tool_event(&mut app, ToolEvent::Up { x, y: 105.0 }, Modifiers::NONE);
    }
    let draft = app.ui.pen.clone().unwrap();
    assert_eq!(draft.knots.len(), 2);
    assert_eq!(work(&app), before);
    app.ui.tool = Tool::DirectSelection;
    click_segment(&mut app, [107.5, 105.0], [0, 0]);
    assert_eq!(app.ui.direct_selection.target, Some(PathRef::Pen));
    assert!(!arrow(&mut app, egui::Key::ArrowRight, Modifiers::NONE));
    assert_eq!(app.ui.pen, Some(draft));
    assert_eq!(work(&app), before);
    assert_eq!(app.session.active().unwrap().history.past_len(), n);
}

/// Shape and vector-mask edits retain the existing engine rendering and one-step undo seam.
/// The unrelated Work Path stays unchanged even though it is also a valid command target.
#[test]
fn shape_and_vector_mask_segments_use_their_layer_target() {
    for mask in [false, true] {
        let mut app = new_app();
        let id = if mask {
            let id = app.session.active().unwrap().active_layer.unwrap().0;
            app.run("layer.vectorMask.add", json!({"layer":id,"path":{"subpaths":[{"closed":true,"knots":[[60,60],[180,60],[180,180],[60,180]]}]}})).unwrap();
            app.ui.vector_mask_target = true;
            id
        } else {
            app.run("shape.create", json!({"kind":"rect","rect":[60,60,120,120],"fill":"#3070c0"})).unwrap()["layer"].as_u64().unwrap()
        };
        click_segment(&mut app, [120.0, 60.0], [0, 0]);
        assert_eq!(app.ui.direct_selection.target, Some(PathRef::Layer(id)));
        let doc_before = app.session.active().unwrap().doc.clone();
        let path = |app: &PhotocraftApp| {
            let layer = app.session.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap();
            if mask {
                layer.vector_mask.as_ref().unwrap().path.clone()
            } else {
                let photocraft_doc::LayerContent::Shape(shape) = &layer.content else { panic!("shape fixture") };
                shape.path.clone()
            }
        };
        let before = path(&app);
        let n = app.session.active().unwrap().history.past_len();
        assert!(arrow(&mut app, egui::Key::ArrowLeft, Modifiers::NONE));
        assert_translation(&before, &path(&app), [0, 1], [-1.0, 0.0]);
        assert_eq!(app.session.active().unwrap().doc.work_path, doc_before.work_path);
        assert_eq!(app.session.active().unwrap().history.past_len(), n + 1);
        assert!(app.session.undo());
        assert_eq!(path(&app), before);
    }
}

/// A press before release remains a pointer drag; nudging must not commit or bend it.
/// Once released, a curved pointer drag still bends handles with its anchors stationary.
#[test]
fn active_segment_drag_owns_input_and_keeps_its_existing_bend() {
    let mut app = new_app();
    let before = work(&app);
    let n = app.session.active().unwrap().history.past_len();
    tool_event(&mut app, ToolEvent::Down { x: 100.0, y: 10.0, pressure: 1.0 }, Modifiers::NONE);
    assert_eq!(app.ui.direct_selection.segment, Some([0, 0]));
    assert!(!arrow(&mut app, egui::Key::ArrowDown, Modifiers::SHIFT));
    assert_eq!(work(&app), before);
    assert_eq!(app.session.active().unwrap().history.past_len(), n);
    tool_event(&mut app, ToolEvent::Move { x: 100.0, y: 30.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 100.0, y: 30.0 }, Modifiers::NONE);
    let after = work(&app);
    assert_eq!(after.subpaths[0].knots[0].anchor, before.subpaths[0].knots[0].anchor);
    assert_eq!(after.subpaths[0].knots[1].anchor, before.subpaths[0].knots[1].anchor);
    assert_ne!(after.subpaths[0].knots[0].out_ctrl, before.subpaths[0].knots[0].out_ctrl);
    assert_eq!(app.session.active().unwrap().history.past_len(), n + 1);
}

/// Invalid topology, overflowing indices and locked layers cannot be nudged or retain a
/// misleading segment selection. The writable fallback candidate must not bypass layer locks.
#[test]
fn invalid_segments_and_locked_targets_clear_without_editing() {
    for segment in [[usize::MAX, 0], [0, usize::MAX], [0, 4]] {
        let mut app = new_app();
        click_segment(&mut app, [100.0, 10.0], [0, 0]);
        app.ui.direct_selection.segment = Some(segment);
        let (before, n) = (work(&app), app.session.active().unwrap().history.past_len());
        assert!(!arrow(&mut app, egui::Key::ArrowRight, Modifiers::NONE));
        assert_eq!(work(&app), before);
        assert_eq!(app.session.active().unwrap().history.past_len(), n);
        assert!(app.ui.direct_selection.target.is_none());
    }
    let mut app = new_app();
    app.session
        .edit("Open fixture", |doc, _| {
            doc.work_path.as_mut().unwrap().subpaths[0].closed = false;
            Ok(())
        })
        .unwrap();
    click_segment(&mut app, [100.0, 10.0], [0, 0]);
    app.ui.direct_selection.segment = Some([0, 3]);
    let before = work(&app);
    assert!(!arrow(&mut app, egui::Key::ArrowRight, Modifiers::NONE));
    assert_eq!(work(&app), before);
    assert!(app.ui.direct_selection.target.is_none());

    for lock in ["all", "pixels"] {
        let mut app = new_app();
        let id = app.run("shape.create", json!({"kind":"rect","rect":[60,60,120,120]})).unwrap()["layer"].as_u64().unwrap();
        click_segment(&mut app, [120.0, 60.0], [0, 0]);
        app.session
            .edit("Lock fixture", |doc, _| {
                let layer = doc.layer_mut(photocraft_doc::LayerId(id)).unwrap();
                if lock == "all" {
                    layer.locks.all = true;
                } else {
                    layer.locks.pixels = true;
                }
                Ok(())
            })
            .unwrap();
        // Rebind to the lock revision to exercise writability as well as ordinary stale guards.
        app.ui.direct_selection.owner = Owner::current(&app);
        let before = app.session.active().unwrap().doc.clone();
        let n = app.session.active().unwrap().history.past_len();
        assert!(!arrow(&mut app, egui::Key::ArrowRight, Modifiers::NONE));
        assert_eq!(app.session.active().unwrap().doc.layers, before.layers);
        assert_eq!(app.session.active().unwrap().history.past_len(), n);
        assert!(app.ui.direct_selection.target.is_none());
    }
}

/// Document changes, reopen and settings restoration never transfer segment indices onto
/// another live document, even when a reopened document preserves its logical document id.
#[test]
fn segment_ownership_is_not_reused_after_document_changes_or_settings_restore() {
    #[derive(Debug)]
    enum Change {
        Switch,
        Reopen,
        RestoreSettings,
        EditPath,
        SelectOtherPath,
    }
    for change in [Change::Switch, Change::Reopen, Change::RestoreSettings, Change::EditPath, Change::SelectOtherPath] {
        let mut app = new_app();
        click_segment(&mut app, [100.0, 10.0], [0, 0]);
        match change {
            Change::Switch => {
                app.run("file.new", json!({"width":240,"height":240})).unwrap();
            }
            Change::Reopen => {
                let closed = app.session.close(0).unwrap();
                app.session.add_document((*closed.doc).clone(), None);
            }
            Change::RestoreSettings => {
                app.ui.direct_selection = serde_json::from_value(serde_json::to_value(&app.ui.direct_selection).unwrap()).unwrap();
            }
            Change::EditPath => {
                app.session.execute("path.moveAnchors", json!({"name":"work","anchors":[[0,2]],"move":[3,4]})).unwrap();
            }
            Change::SelectOtherPath => {
                app.ui.selected_path = Some("missing".into());
            }
        }
        let before = app.session.active().unwrap().doc.clone();
        let n = app.session.active().unwrap().history.past_len();
        assert!(!arrow(&mut app, egui::Key::ArrowRight, Modifiers::NONE), "{change:?}");
        assert_eq!(app.session.active().unwrap().doc.work_path, before.work_path, "{change:?}");
        assert_eq!(app.session.active().unwrap().history.past_len(), n);
        assert!(app.ui.direct_selection.target.is_none(), "{change:?}");
    }
}

/// Canvas arrow shortcuts defer to an actual focused text field through normal shortcut
/// dispatch, so editing an options field does not unexpectedly move selected geometry.
#[test]
fn a_focused_text_field_keeps_arrows_while_a_segment_is_selected() {
    let mut app = new_app();
    click_segment(&mut app, [100.0, 10.0], [0, 0]);
    let before = work(&app);
    let n = app.session.active().unwrap().history.past_len();
    let ctx = egui::Context::default();
    let id = egui::Id::unique("segment-nudge-text-field");
    let mut text = String::from("value");
    ctx.run_ui(egui::RawInput::default(), |ui| {
        ui.add(egui::TextEdit::singleline(&mut text).id(id)).request_focus();
    })
    .textures_delta
    .clear();
    let input = egui::RawInput {
        events: vec![egui::Event::Key { key: egui::Key::ArrowRight, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }],
        ..Default::default()
    };
    ctx.run_ui(input, |ui| {
        assert_eq!(crate::shortcut_dispatch::Focus::of(ui.ctx()), crate::shortcut_dispatch::Focus::Text);
        crate::shortcuts::handle(&mut app, ui.ctx());
        ui.add(egui::TextEdit::singleline(&mut text).id(id));
    })
    .textures_delta
    .clear();
    assert_eq!(work(&app), before);
    assert_eq!(app.session.active().unwrap().history.past_len(), n);
    assert_eq!(app.ui.direct_selection.segment, Some([0, 0]));
}

/// Idle frames and key releases preserve segment selection without editing; native auto-repeat
/// press events still nudge and retain ownership for the next repeat.
#[test]
fn idle_and_release_frames_leave_segments_selected_and_auto_repeat_nudges() {
    let mut app = new_app();
    click_segment(&mut app, [100.0, 10.0], [0, 0]);
    let before = work(&app);
    let n = app.session.active().unwrap().history.past_len();
    let ctx = egui::Context::default();
    assert!(!super::arrow_keys(&mut app, &ctx));
    assert_eq!(app.ui.direct_selection.segment, Some([0, 0]));
    for pressed in [false, true, true] {
        ctx.input_mut(|i| {
            i.events = vec![egui::Event::Key { key: egui::Key::ArrowDown, physical_key: None, pressed, repeat: pressed, modifiers: Modifiers::NONE }];
        });
        assert_eq!(super::arrow_keys(&mut app, &ctx), pressed);
    }
    assert_translation(&before, &work(&app), [0, 1], [0.0, 2.0]);
    assert_eq!(app.session.active().unwrap().history.past_len(), n + 2);
    assert!(still_valid(&app, &app.ui.direct_selection));
}

/// One egui frame can contain multiple directions, per-event Shift changes and repeated
/// presses. Each eligible event must have a separate undo step in its original order.
#[test]
fn same_frame_segment_presses_and_repeats_keep_their_order_and_modifiers() {
    use egui::{Event, Key};
    let mut app = new_app();
    click_segment(&mut app, [100.0, 10.0], [0, 0]);
    let before = work(&app);
    let n = app.session.active().unwrap().history.past_len();
    let event = |key, modifiers, pressed, repeat| Event::Key { key, physical_key: Some(key), pressed, repeat, modifiers };
    let ignored = [
        event(Key::ArrowLeft, Modifiers::ALT, true, false),
        event(Key::ArrowUp, Modifiers::COMMAND, true, false),
        event(Key::ArrowDown, Modifiers::NONE, false, false),
    ];
    let input = egui::RawInput {
        events: vec![
            event(Key::ArrowDown, Modifiers::SHIFT, true, false),
            ignored[0].clone(),
            event(Key::ArrowRight, Modifiers::NONE, true, false),
            ignored[1].clone(),
            event(Key::ArrowRight, Modifiers::NONE, true, true),
            ignored[2].clone(),
        ],
        ..Default::default()
    };
    let ctx = egui::Context::default();
    ctx.run_ui(input, |ui| {
        assert!(crate::move_mods::arrow_keys(&mut app, ui.ctx()));
        ui.ctx().input(|i| assert_eq!(i.events, ignored));
    })
    .textures_delta
    .clear();
    assert_eq!(app.ui.direct_selection.segment, Some([0, 0]));
    assert!(still_valid(&app, &app.ui.direct_selection));
    assert_eq!(app.session.active().unwrap().history.past_len(), n + 3);
    assert_translation(&before, &work(&app), [0, 1], [2.0, 10.0]);
    for delta in [[1.0, 10.0], [0.0, 10.0], [0.0, 0.0]] {
        assert!(app.session.undo());
        assert_translation(&before, &work(&app), [0, 1], delta);
    }
}

/// The merged Note deletion route and its modal confirmation own their keys before path
/// nudging. Cancel permits a later canvas nudge; deletion invalidates the old path revision,
/// and switching back to Note never lets a retained segment turn Delete into a path edit.
#[test]
fn note_deletion_and_its_confirmation_preserve_segment_keyboard_ownership() {
    let mut app = new_app();
    app.ui.tool = Tool::Note;
    tool_event(&mut app, ToolEvent::Down { x: 12.0, y: 12.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 12.0, y: 12.0 }, Modifiers::NONE);
    assert_eq!(app.session.active().unwrap().doc.notes.len(), 1);
    assert_eq!(app.ui.analysis.note_selected, Some(0));
    app.ui.tool = Tool::DirectSelection;
    click_segment(&mut app, [100.0, 10.0], [0, 0]);
    let before = work(&app);
    let layers = app.session.active().unwrap().doc.layers.clone();
    let n = app.session.active().unwrap().history.past_len();

    // The Notes panel's production trash request can open its prompt with another tool active.
    crate::analysis_ui::ask_delete_note(&mut app, 0);
    let dialog = app.ui.dialogs.iter().find(|d| crate::analysis_ui::owns_delete_note(&d.fields)).unwrap().id;
    canvas_shortcut(&mut app, egui::Key::ArrowRight);
    assert_eq!(work(&app), before, "an established modal owns the key frame");
    assert_eq!(app.session.active().unwrap().history.past_len(), n);
    assert_eq!(app.session.active().unwrap().doc.layers, layers);
    crate::dialogs::cancel(&mut app, dialog).unwrap();
    assert!(app.ui.dialogs.is_empty());
    canvas_shortcut(&mut app, egui::Key::ArrowRight);
    assert_translation(&before, &work(&app), [0, 1], [1.0, 0.0]);
    assert_eq!(app.session.active().unwrap().history.past_len(), n + 1);

    let moved = work(&app);
    crate::analysis_ui::ask_delete_note(&mut app, 0);
    let dialog = app.ui.dialogs.iter().find(|d| crate::analysis_ui::owns_delete_note(&d.fields)).unwrap().id;
    crate::dialogs::confirm(&mut app, dialog).unwrap();
    assert!(app.session.active().unwrap().doc.notes.is_empty());
    assert_eq!(app.session.active().unwrap().history.past_len(), n + 2);
    canvas_shortcut(&mut app, egui::Key::ArrowRight);
    assert_eq!(work(&app), moved, "note deletion is an external revision, not our own nudge");
    assert_eq!(app.session.active().unwrap().history.past_len(), n + 2);
    assert!(app.ui.direct_selection.target.is_none());
    assert_eq!(app.session.active().unwrap().doc.layers, layers);

    app.ui.tool = Tool::Note;
    tool_event(&mut app, ToolEvent::Down { x: 12.0, y: 12.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 12.0, y: 12.0 }, Modifiers::NONE);
    assert_eq!(app.ui.analysis.note_selected, Some(0));
    let n = app.session.active().unwrap().history.past_len();
    for key in [egui::Key::Delete, egui::Key::Backspace] {
        app.ui.tool = Tool::DirectSelection;
        click_segment(&mut app, [101.0, 10.0], [0, 0]);
        app.ui.tool = Tool::Note;
        canvas_shortcut(&mut app, key);
        let dialog = app.ui.dialogs.iter().find(|d| crate::analysis_ui::owns_delete_note(&d.fields)).unwrap().id;
        assert_eq!(work(&app), moved, "{key:?}: Note keeps the key");
        assert_eq!(app.session.active().unwrap().history.past_len(), n);
        assert_eq!(app.session.active().unwrap().doc.layers, layers);
        assert_eq!(app.session.active().unwrap().doc.notes.len(), 1);
        assert!(app.ui.direct_selection.target.is_none());
        crate::dialogs::cancel(&mut app, dialog).unwrap();
    }
}
