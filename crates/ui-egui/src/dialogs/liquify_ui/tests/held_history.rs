use super::*;

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

fn button(p: Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE }
}

fn history_key(redo: bool) -> egui::Event {
    egui::Event::Key {
        key: egui::Key::Z,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: if redo { egui::Modifiers::COMMAND | egui::Modifiers::SHIFT } else { egui::Modifiers::COMMAND },
    }
}

fn prepared() -> (PhotocraftApp, egui::Context) {
    let ctx = egui::Context::default();
    let mut app = app_with_layer();
    open(&mut app, &ctx).unwrap();
    control(&mut app, &json!({"tool": "forwardWarp", "size": 30})).unwrap();
    history_frame(&mut app, &ctx, vec![]);
    history_frame(&mut app, &ctx, vec![]);
    (app, ctx)
}

fn stroke(app: &mut PhotocraftApp, ctx: &egui::Context, start: Pos2, end: Pos2, release: bool) {
    history_frame(app, ctx, vec![egui::Event::PointerMoved(start)]);
    history_frame(app, ctx, vec![button(start, true)]);
    history_frame(app, ctx, vec![egui::Event::PointerMoved(end)]);
    if release {
        history_frame(app, ctx, vec![button(end, false)]);
    }
}

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

fn snapshot(app: &PhotocraftApp) -> (LiquifyField, Vec<[u8; 4]>) {
    let d = app.distort.liquify.as_ref().unwrap();
    (d.field.clone(), d.out.clone())
}

fn assert_snapshot(app: &PhotocraftApp, expected: &(LiquifyField, Vec<[u8; 4]>), strokes: usize, redo: usize) {
    let d = app.distort.liquify.as_ref().unwrap();
    assert_eq!(d.field, expected.0);
    assert_eq!(d.out, expected.1);
    assert_eq!((d.strokes.len(), d.redo.len()), (strokes, redo));
    assert!(d.cur.is_none());
    assert_eq!(d.field, LiquifyField::from_strokes(d.canvas, d.cell, &d.strokes));
}

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

#[test]
fn interrupted_history_can_cancel_or_commit_without_extra_document_history() {
    for commit_dialog in [false, true] {
        let (mut app, ctx) = prepared();
        let before = app.session.active().unwrap().history.past_len();
        let original = photocraft_compose::render(&app.session.active().unwrap().doc, Rect::new(0, 0, 120, 80)).px;
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
    history_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start + vec2(20.0, 10.0))]);
    assert_ne!(app.distort.liquify.as_ref().unwrap().center, center);
    app.distort.liquify.as_mut().unwrap().last_dab = f64::NEG_INFINITY;
    history_frame(&mut app, &ctx, vec![]);
    assert_snapshot(&app, &a, 1, 1);
    history_frame(&mut app, &ctx, vec![button(start, false), space(false)]);
    assert_snapshot(&app, &a, 1, 1);
}

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
