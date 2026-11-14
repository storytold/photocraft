use super::*;

/// Dispatch shortcuts before rendering the preview, matching the desktop frame order.
/// Preserve focus across frames unless an explicit focus event changes it.
fn history_frame(app: &mut PhotocraftApp, ctx: &egui::Context, events: Vec<egui::Event>) {
    let focused = events
        .iter()
        .rev()
        .find_map(|e| if let egui::Event::WindowFocused(focused) = e { Some(*focused) } else { None })
        .unwrap_or_else(|| ctx.input(|i| i.focused));
    let raw = egui::RawInput { screen_rect: Some(ERect::from_min_size(Pos2::ZERO, vec2(1000.0, 700.0))), events, focused, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| {
        crate::shortcuts::handle(app, ui.ctx());
        show(app, ui.ctx());
    });
    out.textures_delta.clear();
}

/// Build an unmodified primary transition so resize or pan modifiers cannot leak
/// into the ordinary-stroke fixtures.
fn button(p: Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE }
}

/// Drive the real Undo or Redo shortcut with command modifiers, rather than
/// calling dialog history helpers directly.
fn history_key(redo: bool) -> egui::Event {
    egui::Event::Key {
        key: egui::Key::Z,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: if redo { egui::Modifiers::COMMAND | egui::Modifiers::SHIFT } else { egui::Modifiers::COMMAND },
    }
}

/// Open a deterministic textured layer and settle the preview before gestures.
/// The small brush keeps the two independent stroke regions distinguishable.
fn prepared() -> (PhotocraftApp, egui::Context) {
    let ctx = egui::Context::default();
    let mut app = app_with_layer();
    open(&mut app, &ctx).unwrap();
    control(&mut app, &json!({"tool": "forwardWarp", "size": 30})).unwrap();
    history_frame(&mut app, &ctx, vec![]);
    history_frame(&mut app, &ctx, vec![]);
    (app, ctx)
}

/// Feed physical press, movement and optional release in separate frames.
/// Omitting release leaves the primary button held through history navigation.
fn stroke(app: &mut PhotocraftApp, ctx: &egui::Context, start: Pos2, end: Pos2, release: bool) {
    history_frame(app, ctx, vec![egui::Event::PointerMoved(start)]);
    history_frame(app, ctx, vec![button(start, true)]);
    history_frame(app, ctx, vec![egui::Event::PointerMoved(end)]);
    if release {
        history_frame(app, ctx, vec![button(end, false)]);
    }
}

/// Protect the original failure: shortcut Undo closes stroke B, then the same
/// render must restore A without beginning another stroke or clearing Redo.
#[test]
fn undo_during_a_held_stroke_does_not_restart_it_in_the_show_frame() {
    let (mut app, ctx) = prepared();
    stroke(&mut app, &ctx, pos2(240.0, 365.0), pos2(280.0, 365.0), true);
    let a = app.distort.liquify.as_ref().unwrap().field.clone();
    let pixels_a = app.distort.liquify.as_ref().unwrap().out.clone();
    stroke(&mut app, &ctx, pos2(400.0, 365.0), pos2(450.0, 365.0), false);
    assert!(ctx.input(|i| i.pointer.primary_down()));
    assert!(app.distort.liquify.as_ref().unwrap().cur.is_some());
    assert_ne!(app.distort.liquify.as_ref().unwrap().field, a);

    // Shortcut dispatch precedes the actual dialog render, as in the desktop app.
    history_frame(&mut app, &ctx, vec![history_key(false)]);
    let d = app.distort.liquify.as_ref().unwrap();
    assert_eq!(d.field, a);
    assert_eq!(d.out, pixels_a);
    assert_eq!(d.strokes.len(), 1);
    assert_eq!(d.redo.len(), 1, "the still-held button must not clear Redo");
    assert!(d.cur.is_none(), "Undo must leave the interrupted stroke closed");
}

/// Capture both the deformation field and rendered proxy pixels, so correct
/// history counts cannot conceal an incorrect visible restoration.
fn snapshot(app: &PhotocraftApp) -> (LiquifyField, Vec<[u8; 4]>) {
    let d = app.distort.liquify.as_ref().unwrap();
    (d.field.clone(), d.out.clone())
}

/// Require exact field/pixel restoration, closed brush state and history counts.
/// Independent stroke replay must also reconstruct the current deformation field.
fn assert_snapshot(app: &PhotocraftApp, expected: &(LiquifyField, Vec<[u8; 4]>), strokes: usize, redo: usize) {
    let d = app.distort.liquify.as_ref().unwrap();
    assert_eq!(d.field, expected.0);
    assert_eq!(d.out, expected.1);
    assert_eq!((d.strokes.len(), d.redo.len()), (strokes, redo));
    assert!(d.cur.is_none());
    assert_eq!(d.field, LiquifyField::from_strokes(d.canvas, d.cell, &d.strokes));
}

/// Keep restored pixels and Redo stable through held movement and timed dabs.
/// Repeated Undo/Redo must replay exactly for both motion and stationary tools.
#[test]
fn held_motion_stationary_ticks_and_repeated_history_preserve_redo_and_replay() {
    for tool in ["forwardWarp", "twirlCw"] {
        let (mut app, ctx) = prepared();
        let empty = snapshot(&app);
        stroke(&mut app, &ctx, pos2(240.0, 365.0), pos2(280.0, 365.0), true);
        let a = snapshot(&app);
        control(&mut app, &json!({"tool": tool})).unwrap();
        stroke(&mut app, &ctx, pos2(400.0, 365.0), pos2(450.0, 365.0), false);
        let b = snapshot(&app);

        history_frame(&mut app, &ctx, vec![history_key(false), egui::Event::PointerMoved(pos2(460.0, 370.0))]);
        assert_snapshot(&app, &a, 1, 1);
        for _ in 0..3 {
            app.distort.liquify.as_mut().unwrap().last_dab = f64::NEG_INFINITY;
            history_frame(&mut app, &ctx, vec![]);
            assert_snapshot(&app, &a, 1, 1);
        }
        history_frame(&mut app, &ctx, vec![history_key(false)]);
        assert_snapshot(&app, &empty, 0, 2);
        history_frame(&mut app, &ctx, vec![history_key(false)]);
        assert_snapshot(&app, &empty, 0, 2);
        history_frame(&mut app, &ctx, vec![history_key(true)]);
        assert_snapshot(&app, &a, 1, 1);
        history_frame(&mut app, &ctx, vec![history_key(true)]);
        assert_snapshot(&app, &b, 2, 0);
        history_frame(&mut app, &ctx, vec![history_key(true), egui::Event::PointerMoved(pos2(480.0, 380.0))]);
        assert_snapshot(&app, &b, 2, 0);
        history_frame(&mut app, &ctx, vec![button(pos2(480.0, 380.0), false)]);
        assert_snapshot(&app, &b, 2, 0);
    }
}

/// Release anywhere must end suppression; one later physical gesture creates
/// one stroke and clears Redo as an intentional edit.
#[test]
fn releasing_inside_or_outside_allows_one_new_intentional_stroke() {
    for release in [pos2(450.0, 365.0), pos2(1100.0, 750.0)] {
        let (mut app, ctx) = prepared();
        stroke(&mut app, &ctx, pos2(240.0, 365.0), pos2(280.0, 365.0), false);
        history_frame(&mut app, &ctx, vec![history_key(false)]);
        let empty = snapshot(&app);
        history_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(release), button(release, false)]);
        assert_snapshot(&app, &empty, 0, 1);
        stroke(&mut app, &ctx, pos2(400.0, 365.0), pos2(450.0, 365.0), true);
        let d = app.distort.liquify.as_ref().unwrap();
        assert_eq!((d.strokes.len(), d.redo.len()), (1, 0), "an intentional edit clears Redo");
        assert_eq!(d.field, LiquifyField::from_strokes(d.canvas, d.cell, &d.strokes));
        assert!(d.cur.is_none());
    }
}

/// A new physical press after release in the Undo frame is an intentional edit,
/// so it must create one fresh stroke instead of remaining suppressed.
#[test]
fn a_release_and_new_press_in_the_history_frame_starts_one_new_stroke() {
    let (mut app, ctx) = prepared();
    stroke(&mut app, &ctx, pos2(240.0, 365.0), pos2(280.0, 365.0), false);
    let next = pos2(400.0, 365.0);
    history_frame(&mut app, &ctx, vec![history_key(false), button(pos2(280.0, 365.0), false), egui::Event::PointerMoved(next), button(next, true)]);
    assert!(app.distort.liquify.as_ref().unwrap().cur.is_some(), "a genuine new press is intentional");
    history_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(pos2(450.0, 365.0))]);
    history_frame(&mut app, &ctx, vec![button(pos2(450.0, 365.0), false)]);
    let d = app.distort.liquify.as_ref().unwrap();
    assert_eq!((d.strokes.len(), d.redo.len()), (1, 0));
    assert_eq!(d.field, LiquifyField::from_strokes(d.canvas, d.cell, &d.strokes));
}

/// Two physical gestures sharing a frame must remain separate in history.
/// Undoing the second must restore the exact first-stroke field and pixels.
#[test]
fn a_release_and_new_press_without_history_records_two_separate_brush_strokes() {
    let (mut app, ctx) = prepared();
    stroke(&mut app, &ctx, pos2(240.0, 365.0), pos2(280.0, 365.0), false);
    let a = snapshot(&app);
    let first = app.distort.liquify.as_ref().unwrap().cur.as_ref().unwrap().0.clone();
    let next = pos2(400.0, 365.0);
    history_frame(&mut app, &ctx, vec![button(pos2(280.0, 365.0), false), egui::Event::PointerMoved(next), button(next, true)]);
    let d = app.distort.liquify.as_ref().unwrap();
    assert_eq!(d.strokes.len(), 1, "the first physical stroke is closed");
    assert_eq!(d.strokes[0], first, "the new press must not extend the first stroke");
    assert_eq!(d.cur.as_ref().unwrap().0.points.len(), 1);
    history_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(pos2(450.0, 365.0))]);
    history_frame(&mut app, &ctx, vec![button(pos2(450.0, 365.0), false)]);
    let d = app.distort.liquify.as_ref().unwrap();
    assert_eq!(d.strokes.len(), 2);
    assert_eq!(d.out, replayed_out(d));
    history_frame(&mut app, &ctx, vec![history_key(false)]);
    assert_snapshot(&app, &a, 1, 1);
}

/// Keep same-frame lasso gestures as two polygons with an unfrozen gap.
/// Undo must remove only the second polygon, preserving the first mask region.
#[test]
fn a_release_and_new_press_without_history_keeps_lasso_polygons_separate() {
    let (mut app, ctx) = prepared();
    control(&mut app, &json!({"tool": "lassoMask"})).unwrap();
    stroke(&mut app, &ctx, pos2(240.0, 330.0), pos2(320.0, 330.0), false);
    for p in [pos2(320.0, 400.0), pos2(240.0, 400.0)] {
        history_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(p)]);
    }
    history_frame(&mut app, &ctx, vec![button(pos2(240.0, 400.0), false), egui::Event::PointerMoved(pos2(400.0, 330.0)), button(pos2(400.0, 330.0), true)]);
    assert_eq!(app.distort.liquify.as_ref().unwrap().strokes.len(), 1);
    assert_eq!(app.distort.liquify.as_ref().unwrap().lasso.as_ref().unwrap().1.len(), 1);
    for p in [pos2(480.0, 330.0), pos2(480.0, 400.0), pos2(400.0, 400.0)] {
        history_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(p)]);
    }
    history_frame(&mut app, &ctx, vec![button(pos2(400.0, 400.0), false)]);
    let d = app.distort.liquify.as_ref().unwrap();
    assert_eq!(d.strokes.len(), 2);
    assert_eq!(d.field.freeze_at(35.5, 40.0), 1.0);
    assert_eq!(d.field.freeze_at(55.5, 40.0), 0.0, "separate polygons leave the gap unfrozen");
    assert_eq!(d.field.freeze_at(75.5, 40.0), 1.0);
    assert_eq!(d.field, LiquifyField::from_strokes(d.canvas, d.cell, &d.strokes));
    history_frame(&mut app, &ctx, vec![history_key(false)]);
    let d = app.distort.liquify.as_ref().unwrap();
    assert_eq!(d.field.freeze_at(35.5, 40.0), 1.0);
    assert_eq!(d.field.freeze_at(75.5, 40.0), 0.0);
}

/// A press owned by the properties pane cannot restart preview painting.
/// Only a subsequent deliberate preview gesture may consume Redo.
#[test]
fn a_new_press_in_properties_does_not_resume_the_interrupted_preview_stroke() {
    let (mut app, ctx) = prepared();
    stroke(&mut app, &ctx, pos2(240.0, 365.0), pos2(280.0, 365.0), true);
    let a = snapshot(&app);
    stroke(&mut app, &ctx, pos2(400.0, 365.0), pos2(450.0, 365.0), false);
    history_frame(
        &mut app,
        &ctx,
        vec![history_key(false), button(pos2(450.0, 365.0), false), button(pos2(940.0, 74.0), true), egui::Event::PointerMoved(pos2(440.0, 365.0))],
    );
    assert_snapshot(&app, &a, 1, 1);
    history_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(pos2(470.0, 365.0))]);
    assert_snapshot(&app, &a, 1, 1);
    history_frame(&mut app, &ctx, vec![button(pos2(470.0, 365.0), false)]);
    stroke(&mut app, &ctx, pos2(400.0, 365.0), pos2(450.0, 365.0), true);
    assert_eq!(app.distort.liquify.as_ref().unwrap().strokes.len(), 2);
    assert!(app.distort.liquify.as_ref().unwrap().redo.is_empty());
}

/// Leaving and regaining focus while held must preserve the undone image and
/// Redo; release followed by a fresh preview press still paints normally.
#[test]
fn leaving_the_preview_and_losing_focus_do_not_resume_an_interrupted_hold() {
    let (mut app, ctx) = prepared();
    stroke(&mut app, &ctx, pos2(240.0, 365.0), pos2(280.0, 365.0), false);
    history_frame(&mut app, &ctx, vec![history_key(false)]);
    let empty = snapshot(&app);
    history_frame(&mut app, &ctx, vec![egui::Event::PointerGone, egui::Event::WindowFocused(false)]);
    assert!(!ctx.input(|i| i.focused));
    assert_snapshot(&app, &empty, 0, 1);
    history_frame(&mut app, &ctx, vec![egui::Event::WindowFocused(true), egui::Event::PointerMoved(pos2(450.0, 365.0))]);
    assert!(ctx.input(|i| i.focused));
    assert_snapshot(&app, &empty, 0, 1);
    history_frame(&mut app, &ctx, vec![button(pos2(450.0, 365.0), false)]);
    stroke(&mut app, &ctx, pos2(400.0, 365.0), pos2(450.0, 365.0), true);
    assert_eq!(app.distort.liquify.as_ref().unwrap().strokes.len(), 1);
}

/// Cancel discards the preview and OK contributes exactly one document edit.
/// A reopened dialog must not inherit the interrupted gesture from its predecessor.
/// The original pixel snapshot outlives the temporary compositor buffer.
#[test]
fn interrupted_history_can_cancel_or_commit_without_extra_document_history() {
    for commit_dialog in [false, true] {
        let (mut app, ctx) = prepared();
        let before = app.session.active().unwrap().history.past_len();
        let original = photocraft_compose::render(&app.session.active().unwrap().doc, Rect::new(0, 0, 120, 80)).px.clone();
        stroke(&mut app, &ctx, pos2(240.0, 365.0), pos2(280.0, 365.0), true);
        stroke(&mut app, &ctx, pos2(400.0, 365.0), pos2(450.0, 365.0), false);
        let painted = snapshot(&app);
        history_frame(&mut app, &ctx, vec![history_key(false)]);
        if commit_dialog {
            history_frame(&mut app, &ctx, vec![history_key(true)]);
            assert_snapshot(&app, &painted, 2, 0);
            commit(&mut app).unwrap();
        } else {
            cancel(&mut app);
        }
        assert!(app.distort.liquify.is_none());
        assert_eq!(app.session.active().unwrap().history.past_len(), before + usize::from(commit_dialog));
        if commit_dialog {
            app.session.execute("edit.undo", json!({})).unwrap();
        }
        assert_eq!(photocraft_compose::render(&app.session.active().unwrap().doc, Rect::new(0, 0, 120, 80)).px, original);
        open(&mut app, &ctx).unwrap();
        history_frame(&mut app, &ctx, vec![button(pos2(450.0, 365.0), false)]);
        stroke(&mut app, &ctx, pos2(240.0, 365.0), pos2(280.0, 365.0), true);
        assert_eq!(app.distort.liquify.as_ref().unwrap().strokes.len(), 1, "new dialogs have fresh gesture state");
    }
}

/// Empty history navigation is harmless, while a same-frame first press still
/// works; Redo with no saved stroke closes an active brush without restarting it.
#[test]
fn empty_history_and_redo_during_an_active_stroke_are_safe() {
    let (mut app, ctx) = prepared();
    let empty = snapshot(&app);
    for redo in [false, true, true, false] {
        history_frame(&mut app, &ctx, vec![history_key(redo)]);
        assert_snapshot(&app, &empty, 0, 0);
    }
    // With no old gesture, a press in the same frame as Undo still starts normally.
    let at = pos2(240.0, 365.0);
    history_frame(&mut app, &ctx, vec![history_key(false), egui::Event::PointerMoved(at), button(at, true)]);
    history_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(pos2(280.0, 365.0))]);
    let painted = snapshot(&app);
    history_frame(&mut app, &ctx, vec![history_key(true)]);
    assert_snapshot(&app, &painted, 1, 0);
    history_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(pos2(300.0, 365.0))]);
    assert_snapshot(&app, &painted, 1, 0);
}

/// Changing tools during an interrupted hold cannot create a lasso implicitly.
/// A later deliberate lasso gesture still freezes its polygon and clears Redo.
#[test]
fn interrupted_brush_does_not_turn_into_a_lasso_and_a_new_lasso_still_works() {
    let (mut app, ctx) = prepared();
    stroke(&mut app, &ctx, pos2(240.0, 365.0), pos2(280.0, 365.0), false);
    history_frame(&mut app, &ctx, vec![history_key(false)]);
    let empty = snapshot(&app);
    control(&mut app, &json!({"tool": "lassoMask"})).unwrap();
    history_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(pos2(400.0, 365.0))]);
    assert_snapshot(&app, &empty, 0, 1);
    assert!(app.distort.liquify.as_ref().unwrap().lasso.is_none());
    history_frame(&mut app, &ctx, vec![button(pos2(400.0, 365.0), false)]);
    stroke(&mut app, &ctx, pos2(240.0, 330.0), pos2(400.0, 330.0), false);
    history_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(pos2(400.0, 400.0))]);
    history_frame(&mut app, &ctx, vec![button(pos2(240.0, 400.0), false)]);
    let d = app.distort.liquify.as_ref().unwrap();
    assert_eq!((d.strokes.len(), d.redo.len()), (1, 0));
    assert_eq!(d.strokes[0].tool, LiquifyTool::LassoMask);
    assert!(d.lasso.is_none());
    assert!(d.field.freeze.iter().any(|&f| f > 0.0), "the intentional lasso freezes its polygon");
    assert_eq!(d.field, LiquifyField::from_strokes(d.canvas, d.cell, &d.strokes));
}

/// A same-frame release and resize press must resize rather than paint.
/// Movement, stationary ticks and release must preserve restored pixels and Redo.
#[test]
fn a_release_and_fresh_resize_press_in_one_frame_preserves_redo_without_painting() {
    let (mut app, ctx) = prepared();
    stroke(&mut app, &ctx, pos2(240.0, 365.0), pos2(280.0, 365.0), true);
    let a = snapshot(&app);
    control(&mut app, &json!({"tool": "twirlCw"})).unwrap();
    stroke(&mut app, &ctx, pos2(400.0, 365.0), pos2(450.0, 365.0), false);
    history_frame(&mut app, &ctx, vec![history_key(false)]);
    let start = pos2(400.0, 365.0);
    let modifiers = egui::Modifiers::CTRL | egui::Modifiers::ALT;
    let press = egui::Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers };
    history_frame(&mut app, &ctx, vec![button(pos2(450.0, 365.0), false), egui::Event::PointerMoved(start), egui::Event::ModifiersChanged(modifiers), press]);
    let d = app.distort.liquify.as_ref().unwrap();
    assert!(d.brush_resize.is_some(), "the new press must resize: active brush={}, redo={}", d.cur.is_some(), d.redo.len());
    assert_snapshot(&app, &a, 1, 1);
    history_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start + vec2(20.0, 10.0))]);
    assert!(app.distort.liquify.as_ref().unwrap().opts.size > 30.0);
    app.distort.liquify.as_mut().unwrap().last_dab = f64::NEG_INFINITY;
    history_frame(&mut app, &ctx, vec![]);
    assert_snapshot(&app, &a, 1, 1);
    history_frame(&mut app, &ctx, vec![button(start, false)]);
    assert!(app.distort.liquify.as_ref().unwrap().brush_resize.is_none());
    assert_snapshot(&app, &a, 1, 1);
}

/// A same-frame release and Space-drag must pan the preview without painting
/// or consuming Redo, including a pause before the first movement and later repaint ticks.
#[test]
fn a_release_and_fresh_pan_press_in_one_frame_preserves_redo_without_painting() {
    let (mut app, ctx) = prepared();
    stroke(&mut app, &ctx, pos2(240.0, 365.0), pos2(280.0, 365.0), true);
    let a = snapshot(&app);
    control(&mut app, &json!({"tool": "twirlCw"})).unwrap();
    stroke(&mut app, &ctx, pos2(400.0, 365.0), pos2(450.0, 365.0), false);
    history_frame(&mut app, &ctx, vec![history_key(false)]);
    let start = pos2(400.0, 365.0);
    let space = |pressed| egui::Event::Key { key: egui::Key::Space, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::NONE };
    history_frame(&mut app, &ctx, vec![button(pos2(450.0, 365.0), false), space(true), egui::Event::PointerMoved(start), button(start, true)]);
    assert_snapshot(&app, &a, 1, 1);
    let center = app.distort.liquify.as_ref().unwrap().center;
    // A pan owns the hold before egui has recognized its first drag movement.
    app.distort.liquify.as_mut().unwrap().last_dab = f64::NEG_INFINITY;
    history_frame(&mut app, &ctx, vec![]);
    assert_eq!(app.distort.liquify.as_ref().unwrap().center, center);
    assert_snapshot(&app, &a, 1, 1);
    history_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start + vec2(20.0, 10.0))]);
    assert_ne!(app.distort.liquify.as_ref().unwrap().center, center);
    app.distort.liquify.as_mut().unwrap().last_dab = f64::NEG_INFINITY;
    history_frame(&mut app, &ctx, vec![]);
    assert_snapshot(&app, &a, 1, 1);
    history_frame(&mut app, &ctx, vec![button(start, false), space(false)]);
    assert_snapshot(&app, &a, 1, 1);
}

/// Resize and pan retain their normal behavior after interrupted painting.
/// Releasing a secondary resize button cannot release the still-held primary gesture.
#[test]
fn brush_resize_and_pan_still_work_after_an_interrupted_hold() {
    let (mut app, ctx) = prepared();
    stroke(&mut app, &ctx, pos2(240.0, 365.0), pos2(280.0, 365.0), false);
    history_frame(&mut app, &ctx, vec![history_key(false)]);
    let empty = snapshot(&app);
    let start = pos2(280.0, 365.0);
    // Ctrl+Alt is brush resizing, even when an interrupted painting hold still owns the button.
    history_frame(&mut app, &ctx, vec![egui::Event::ModifiersChanged(egui::Modifiers::CTRL | egui::Modifiers::ALT)]);
    history_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start + vec2(20.0, 10.0))]);
    assert_snapshot(&app, &empty, 0, 1);
    assert!(app.distort.liquify.as_ref().unwrap().opts.size > 30.0);
    // Releasing a secondary resize button must not release the interrupted primary hold.
    history_frame(&mut app, &ctx, vec![egui::Event::ModifiersChanged(egui::Modifiers::ALT)]);
    let secondary = |pressed| egui::Event::PointerButton { pos: start, button: egui::PointerButton::Secondary, pressed, modifiers: egui::Modifiers::ALT };
    history_frame(&mut app, &ctx, vec![secondary(true)]);
    history_frame(&mut app, &ctx, vec![secondary(false)]);
    history_frame(&mut app, &ctx, vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE), egui::Event::PointerMoved(start)]);
    assert!(ctx.input(|i| i.pointer.primary_down()));
    assert_snapshot(&app, &empty, 0, 1);
    history_frame(&mut app, &ctx, vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE), button(start, false)]);
    assert!(app.distort.liquify.as_ref().unwrap().brush_resize.is_none());

    let space = |pressed| egui::Event::Key { key: egui::Key::Space, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::NONE };
    history_frame(&mut app, &ctx, vec![space(true)]);
    let center = app.distort.liquify.as_ref().unwrap().center;
    stroke(&mut app, &ctx, pos2(400.0, 365.0), pos2(450.0, 365.0), true);
    assert_ne!(app.distort.liquify.as_ref().unwrap().center, center);
    assert_eq!(app.distort.liquify.as_ref().unwrap().field, empty.0, "panning does not deform the image");
    history_frame(&mut app, &ctx, vec![space(false)]);
}

/// Releasing a resize button and pressing plain primary in one frame must paint.
/// Brush settings stay fixed and Undo/Redo replay the new stroke exactly.
fn resize_release_then_brush(owner: egui::PointerButton, modifiers: egui::Modifiers) {
    let (mut app, ctx) = prepared();
    let empty = snapshot(&app);
    let d = app.distort.liquify.as_ref().unwrap();
    let initial = (d.opts.size, d.opts.density);
    let zoom = d.zoom;
    let start = pos2(240.0, 365.0);
    let press = egui::Event::PointerButton { pos: start, button: owner, pressed: true, modifiers };
    history_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start), egui::Event::ModifiersChanged(modifiers), press]);
    let end = start + vec2(20.0, 10.0);
    history_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(end)]);
    let d = app.distort.liquify.as_ref().unwrap();
    assert!(d.brush_resize.is_some());
    let moved = (d.opts.size, d.opts.density);
    assert!(moved.0 > initial.0);
    assert_snapshot(&app, &empty, 0, 0);
    let next = pos2(400.0, 365.0);
    let release_at = start + vec2(30.0, 0.0);
    let resized = resized_brush(initial.0, initial.1, release_at.x - start.x, release_at.y - start.y, zoom);
    assert_ne!(resized, moved, "the owner release includes movement after the preceding frame");
    let release = egui::Event::PointerButton { pos: release_at, button: owner, pressed: false, modifiers };
    history_frame(&mut app, &ctx, vec![release, egui::Event::ModifiersChanged(egui::Modifiers::NONE), egui::Event::PointerMoved(next), button(next, true)]);
    let d = app.distort.liquify.as_ref().unwrap();
    assert!(d.brush_resize.is_none(), "a new plain press must end the preceding {owner:?} resize");
    assert!(d.cur.is_some(), "the fresh press must start an intentional brush stroke");
    assert_eq!(d.cur.as_ref().unwrap().0.points.len(), 1);
    assert_eq!((d.opts.size, d.opts.density), resized);
    history_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(next + vec2(40.0, 0.0))]);
    let d = app.distort.liquify.as_ref().unwrap();
    assert_eq!((d.opts.size, d.opts.density), resized);
    assert_ne!(d.field, empty.0, "the new brush must deform the preview");
    assert!(d.cur.as_ref().unwrap().0.points.len() > 1);
    history_frame(&mut app, &ctx, vec![button(next + vec2(40.0, 0.0), false)]);
    let painted = snapshot(&app);
    assert_snapshot(&app, &painted, 1, 0);
    history_frame(&mut app, &ctx, vec![history_key(false)]);
    assert_snapshot(&app, &empty, 0, 1);
    history_frame(&mut app, &ctx, vec![history_key(true)]);
    assert_snapshot(&app, &painted, 1, 0);
    let d = app.distort.liquify.as_ref().unwrap();
    assert_eq!((d.opts.size, d.opts.density), resized);
}

/// A coalesced Control-Alt primary release and plain press starts a movable brush.
#[test]
fn a_primary_resize_release_and_new_brush_press_starts_a_stroke() {
    resize_release_then_brush(egui::PointerButton::Primary, egui::Modifiers::CTRL | egui::Modifiers::ALT);
}

/// A coalesced Alt-secondary release and plain primary press starts a movable brush.
#[test]
fn a_secondary_resize_release_and_new_brush_press_starts_a_stroke() {
    resize_release_then_brush(egui::PointerButton::Secondary, egui::Modifiers::ALT);
}

/// A resize retains its original button through modifier changes and unrelated presses.
/// Only its owner's release ends resizing, without brush dabs or local-history changes.
#[test]
fn an_ongoing_resize_survives_modifier_changes_and_an_unrelated_press() {
    for (owner, other, modifiers) in [
        (egui::PointerButton::Primary, egui::PointerButton::Secondary, egui::Modifiers::CTRL | egui::Modifiers::ALT),
        (egui::PointerButton::Secondary, egui::PointerButton::Primary, egui::Modifiers::ALT),
    ] {
        let (mut app, ctx) = prepared();
        let empty = snapshot(&app);
        let start = pos2(240.0, 365.0);
        let press = egui::Event::PointerButton { pos: start, button: owner, pressed: true, modifiers };
        history_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start), egui::Event::ModifiersChanged(modifiers), press]);
        history_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start + vec2(20.0, 10.0))]);
        let previous_size = app.distort.liquify.as_ref().unwrap().opts.size;
        let unrelated = |pressed| egui::Event::PointerButton { pos: start + vec2(40.0, 20.0), button: other, pressed, modifiers: egui::Modifiers::NONE };
        history_frame(
            &mut app,
            &ctx,
            vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE), egui::Event::PointerMoved(start + vec2(40.0, 20.0)), unrelated(true)],
        );
        assert!(app.distort.liquify.as_ref().unwrap().brush_resize.is_some());
        assert!(app.distort.liquify.as_ref().unwrap().opts.size > previous_size);
        assert_snapshot(&app, &empty, 0, 0);
        let previous = (app.distort.liquify.as_ref().unwrap().opts.size, app.distort.liquify.as_ref().unwrap().opts.density);
        history_frame(&mut app, &ctx, vec![unrelated(false), egui::Event::PointerMoved(start + vec2(50.0, 25.0))]);
        assert!(app.distort.liquify.as_ref().unwrap().brush_resize.is_some(), "releasing the other button must retain {owner:?} resize ownership");
        history_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start + vec2(60.0, 30.0))]);
        let d = app.distort.liquify.as_ref().unwrap();
        assert!(d.opts.size > previous.0);
        assert!(d.opts.density > previous.1);
        assert_ne!((d.opts.size, d.opts.density), previous, "movement after an unrelated release must still resize");
        assert_snapshot(&app, &empty, 0, 0);
        let release = egui::Event::PointerButton { pos: start + vec2(60.0, 30.0), button: owner, pressed: false, modifiers: egui::Modifiers::NONE };
        history_frame(&mut app, &ctx, vec![release]);
        assert!(app.distort.liquify.as_ref().unwrap().brush_resize.is_none());
        assert_snapshot(&app, &empty, 0, 0);
    }
}

/// A resize release lets a new resize choose its own anchor or a new pan move the view.
/// Neither transition may jump brush settings or paint while waiting for its first movement.
#[test]
fn a_resize_release_allows_a_fresh_resize_anchor_or_pan() {
    for (owner, modifiers) in
        [(egui::PointerButton::Primary, egui::Modifiers::CTRL | egui::Modifiers::ALT), (egui::PointerButton::Secondary, egui::Modifiers::ALT)]
    {
        for next_resizes in [false, true] {
            let (mut app, ctx) = prepared();
            control(&mut app, &json!({"tool": "twirlCw"})).unwrap();
            let empty = snapshot(&app);
            let start = pos2(240.0, 365.0);
            let press = egui::Event::PointerButton { pos: start, button: owner, pressed: true, modifiers };
            history_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start), egui::Event::ModifiersChanged(modifiers), press]);
            let end = start + vec2(20.0, 10.0);
            history_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(end)]);
            let d = app.distort.liquify.as_ref().unwrap();
            let settings = (d.opts.size, d.opts.density);
            let center = d.center;
            let next = pos2(400.0, 365.0);
            let next_mods = if next_resizes { egui::Modifiers::CTRL | egui::Modifiers::ALT } else { egui::Modifiers::NONE };
            let space = |pressed| egui::Event::Key { key: egui::Key::Space, physical_key: None, pressed, repeat: false, modifiers: next_mods };
            let release = egui::Event::PointerButton { pos: end, button: owner, pressed: false, modifiers };
            let press = egui::Event::PointerButton { pos: next, button: egui::PointerButton::Primary, pressed: true, modifiers: next_mods };
            history_frame(
                &mut app,
                &ctx,
                vec![release, egui::Event::ModifiersChanged(next_mods), space(!next_resizes), egui::Event::PointerMoved(next), press],
            );
            let d = app.distort.liquify.as_ref().unwrap();
            assert_eq!((d.opts.size, d.opts.density), settings, "a fresh gesture must not reuse the preceding resize anchor");
            assert_eq!(d.brush_resize.is_some(), next_resizes);
            assert_snapshot(&app, &empty, 0, 0);
            app.distort.liquify.as_mut().unwrap().last_dab = f64::NEG_INFINITY;
            history_frame(&mut app, &ctx, vec![]);
            assert_eq!(app.distort.liquify.as_ref().unwrap().center, center);
            assert_snapshot(&app, &empty, 0, 0);
            history_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(next + vec2(20.0, 10.0))]);
            let d = app.distort.liquify.as_ref().unwrap();
            if next_resizes {
                assert!(d.opts.size > settings.0);
                assert_eq!(d.center, center);
            } else {
                assert_ne!(d.center, center);
                assert_eq!((d.opts.size, d.opts.density), settings);
            }
            assert_snapshot(&app, &empty, 0, 0);
            let release =
                egui::Event::PointerButton { pos: next + vec2(20.0, 10.0), button: egui::PointerButton::Primary, pressed: false, modifiers: next_mods };
            history_frame(&mut app, &ctx, vec![release, space(false), egui::Event::ModifiersChanged(egui::Modifiers::NONE)]);
            assert!(app.distort.liquify.as_ref().unwrap().brush_resize.is_none());
            assert_snapshot(&app, &empty, 0, 0);
        }
    }
}

/// Both resize routes can hand off to a new Alt-right resize in one input frame.
/// Its next frame can be the release itself, so waiting for a drag would lose the resize.
#[test]
fn a_resize_release_can_handoff_to_a_fresh_secondary_resize() {
    let alt = egui::Modifiers::ALT;
    for (first_button, first_mods) in [(egui::PointerButton::Primary, egui::Modifiers::CTRL | alt), (egui::PointerButton::Secondary, alt)] {
        let (mut app, ctx) = prepared();
        control(&mut app, &json!({"tool": "twirlCw", "density": 50})).unwrap();
        let empty = snapshot(&app);
        let d = app.distort.liquify.as_ref().unwrap();
        let (size, density, zoom) = (d.opts.size, d.opts.density, d.zoom);
        let first = pos2(240.0, 365.0);
        let second = pos2(400.0, 365.0);
        let press = egui::Event::PointerButton { pos: first, button: first_button, pressed: true, modifiers: first_mods };
        history_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(first), egui::Event::ModifiersChanged(first_mods), press]);
        assert!(app.distort.liquify.as_ref().unwrap().brush_resize.is_some());
        let release = egui::Event::PointerButton { pos: first + vec2(20.0, -12.0), button: first_button, pressed: false, modifiers: first_mods };
        let press = egui::Event::PointerButton { pos: second, button: egui::PointerButton::Secondary, pressed: true, modifiers: alt };
        history_frame(&mut app, &ctx, vec![release, egui::Event::ModifiersChanged(alt), egui::Event::PointerMoved(second), press]);
        let d = app.distort.liquify.as_ref().unwrap();
        let first_settings = resized_brush(size, density, 20.0, -12.0, zoom);
        assert_eq!((d.opts.size, d.opts.density), first_settings);
        assert!(d.brush_resize.is_some(), "the new secondary press owns the resize before its first movement");
        let release = egui::Event::PointerButton { pos: second + vec2(27.0, 9.0), button: egui::PointerButton::Secondary, pressed: false, modifiers: alt };
        history_frame(&mut app, &ctx, vec![release]);
        let d = app.distort.liquify.as_ref().unwrap();
        assert_eq!((d.opts.size, d.opts.density), resized_brush(first_settings.0, first_settings.1, 27.0, 9.0, zoom));
        assert!(d.brush_resize.is_none());
        assert_snapshot(&app, &empty, 0, 0);
    }
}

/// A secondary hold or later press cannot borrow the anchor of a fresh Ctrl+Alt-primary resize.
/// The new primary's own release applies its final delta and ends the resize.
#[test]
fn a_fresh_primary_resize_owns_its_release_with_secondary_down() {
    let chord = egui::Modifiers::CTRL | egui::Modifiers::ALT;
    let primary = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: chord };
    let secondary = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Secondary, pressed, modifiers: chord };
    for secondary_already_held in [true, false] {
        let (mut app, ctx) = prepared();
        control(&mut app, &json!({"tool": "twirlCw", "density": 50})).unwrap();
        let empty = snapshot(&app);
        let d = app.distort.liquify.as_ref().unwrap();
        let (size, density, zoom) = (d.opts.size, d.opts.density, d.zoom);
        let first = pos2(240.0, 365.0);
        let second = pos2(400.0, 365.0);
        history_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(first), egui::Event::ModifiersChanged(chord), primary(first, true)]);
        assert!(app.distort.liquify.as_ref().unwrap().brush_resize.is_some());
        if secondary_already_held {
            history_frame(&mut app, &ctx, vec![secondary(first, true)]);
        }
        let mut handoff = vec![primary(first + vec2(20.0, -12.0), false), egui::Event::PointerMoved(second), primary(second, true)];
        if !secondary_already_held {
            handoff.push(secondary(second, true));
        }
        history_frame(&mut app, &ctx, handoff);
        let first_settings = resized_brush(size, density, 20.0, -12.0, zoom);
        let d = app.distort.liquify.as_ref().unwrap();
        assert_eq!((d.opts.size, d.opts.density), first_settings);
        assert!(d.brush_resize.is_some());
        history_frame(&mut app, &ctx, vec![primary(second + vec2(27.0, 9.0), false)]);
        let d = app.distort.liquify.as_ref().unwrap();
        assert!(ctx.input(|i| i.pointer.button_down(egui::PointerButton::Secondary)));
        assert!(d.brush_resize.is_none(), "the actual primary release ends the resize with secondary still held");
        let final_settings = resized_brush(first_settings.0, first_settings.1, 27.0, 9.0, zoom);
        assert_eq!((d.opts.size, d.opts.density), final_settings);
        history_frame(&mut app, &ctx, vec![secondary(second + vec2(45.0, 18.0), false), egui::Event::ModifiersChanged(egui::Modifiers::NONE)]);
        let d = app.distort.liquify.as_ref().unwrap();
        assert_eq!((d.opts.size, d.opts.density), final_settings);
        assert_snapshot(&app, &empty, 0, 0);
    }
}
