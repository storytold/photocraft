use super::*;

fn paint_app(width: u32, height: u32, fill: Rect) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", json!({"width": width, "height": height, "depth": 8})).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(fill, &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.sync_views();
    app
}

fn puppet_app() -> PhotocraftApp {
    let mut app = paint_app(100, 80, Rect::new(20, 30, 80, 50));
    let ctx = app.ctx().clone();
    begin(&mut app, &ctx, "edit.puppetWarp").unwrap();
    app
}

fn past_len(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

fn pins_of(app: &PhotocraftApp) -> Vec<Pin> {
    app.distort.puppet.as_ref().unwrap().warp.pins().to_vec()
}

fn dsts(app: &PhotocraftApp) -> Vec<[f64; 2]> {
    pins_of(app).iter().map(|p| xy(p.at)).collect()
}

/// Places a pin on the mesh where it shows at `on` and holds it at `to`, as one session step.
fn add_pin(app: &mut PhotocraftApp, on: [f64; 2], to: [f64; 2]) {
    let s = app.distort.puppet.as_ref().unwrap();
    let rest = s.solver.mesh().rest_of(point(on)).expect("on the mesh");
    let before = s.warp.pins().to_vec();
    let mut after = before.clone();
    after.push(Pin::new(rest, point(to)));
    session_apply(app, PinEdit::replace(&before, after, "Add Pin")).unwrap();
}

fn command_z(app: &mut PhotocraftApp, modifiers: egui::Modifiers) {
    let ctx = app.ctx().clone();
    let event = egui::Event::Key { key: egui::Key::Z, physical_key: None, pressed: true, repeat: false, modifiers };
    let input = egui::RawInput { events: vec![event], ..Default::default() };
    ctx.run_ui(input, |ui| assert!(crate::distort_ui::keys(app, ui.ctx()))).textures_delta.clear();
}

fn gap(a: Point, b: Point) -> f64 {
    (a - b).hypot()
}

#[test]
fn session_edits_add_no_document_step_and_commit_adds_one_undoable_step() {
    let mut app = puppet_app();
    let past = past_len(&app);
    add_pin(&mut app, [25.0, 40.0], [25.0, 40.0]);
    add_pin(&mut app, [75.0, 40.0], [75.0, 20.0]);
    let pins = pins_of(&app);
    // The second pin is the placed one, so it is selected.
    control(&mut app, &json!({"deleteSelectedPin": true})).unwrap();
    assert_eq!(pins_of(&app), pins[..1]);
    control(&mut app, &json!({"deleteSelectedPin": true})).unwrap();
    assert_eq!(pins_of(&app), pins[..1], "no selection: no edit");
    assert!(undo(&mut app));
    assert_eq!(pins_of(&app), pins, "the deleted pin returns where it was held");
    assert_eq!(past_len(&app), past, "pin edits stay in the open session");

    let original = app.session.active().unwrap().doc.clone();
    commit(&mut app);
    assert!(app.distort.puppet.is_none());
    assert_eq!(past_len(&app), past + 1);
    let committed = app.session.active().unwrap().doc.clone();
    assert_ne!(committed, original);
    app.session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc, original);
    app.session.execute("edit.redo", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc, committed);
}

#[test]
fn deleting_the_last_pin_restores_the_rest_mesh_and_an_empty_commit_is_not_an_edit() {
    let mut app = puppet_app();
    add_pin(&mut app, [25.0, 40.0], [25.0, 20.0]);
    let past = past_len(&app);
    let pins = pins_of(&app);
    session_apply(&mut app, PinEdit::Select(vec![usize::MAX])).unwrap();
    control(&mut app, &json!({"deleteSelectedPin": true})).unwrap();
    assert_eq!(pins_of(&app), pins, "a stale selection is harmless");
    session_apply(&mut app, PinEdit::Select(vec![0])).unwrap();
    control(&mut app, &json!({"deleteSelectedPin": true})).unwrap();
    assert!(pins_of(&app).is_empty());
    let s = app.distort.puppet.as_ref().unwrap();
    let mesh = s.solver.mesh();
    assert_eq!(mesh.deformed.len(), mesh.rest.len());
    for (deformed, rest) in mesh.deformed.iter().zip(mesh.rest) {
        assert!(gap(*deformed, *rest) < 1e-8, "no constraints: the mesh returns to rest");
    }
    commit(&mut app);
    assert!(app.distort.puppet.is_none());
    assert_eq!(past_len(&app), past);
}

fn long_puppet(mode: PinMode, density: PinDensity) -> PhotocraftApp {
    let mut app = paint_app(1000, 300, Rect::new(40, 180, 960, 220));
    let ctx = app.ctx().clone();
    begin(&mut app, &ctx, "edit.puppetWarp").unwrap();
    for x in [70.0, 160.0, 930.0] {
        add_pin(&mut app, [x, 200.0], [x, 200.0]);
    }
    let mut options = *app.distort.puppet.as_ref().unwrap().warp.options();
    options.mode = mode;
    options.density = density;
    session_apply(&mut app, PinEdit::Options(options)).unwrap();
    app
}

#[test]
fn preview_matches_the_canonical_deformation_and_the_committed_layer() {
    use photocraft_algo::puppet::{deform, render};
    for mode in [PinMode::Rigid, PinMode::Normal, PinMode::Distort] {
        for density in [PinDensity::Fewer, PinDensity::Normal, PinDensity::More] {
            let mut app = long_puppet(mode, density);
            let before = pins_of(&app);
            let mut after = before.clone();
            after[2].at = point([690.0, 80.0]);
            session_apply(&mut app, PinEdit::replace(&before, after, "Move Pin")).unwrap();
            let s = app.distort.puppet.as_ref().unwrap();
            let warp = warp_of(s.warp.pins(), s.warp.options());
            let (_, canonical, canonical_order) = deform(&s.solver.surface, s.solver.bounds, &warp, ITERATIONS);
            let mesh = s.solver.mesh();
            assert_eq!(mesh.deformed.len(), canonical.len());
            for (shown, expected) in mesh.deformed.iter().zip(&canonical) {
                assert!(gap(*shown, point(*expected)) < 1e-8, "{mode:?} {density:?}: the preview is the solve from rest");
            }
            assert_eq!(mesh.order, canonical_order.iter().map(|&t| t as u32).collect::<Vec<_>>());
            // The engine's interpolation isolates geometry from GPU texture sampling.
            let expected =
                render(&s.solver.surface, s.solver.bounds, &s.solver.puppet_mesh, &canonical, &canonical_order, photocraft_algo::transform::Interp::Bicubic);
            let layer = s.layer;
            let past = past_len(&app);
            commit(&mut app);
            let st = app.session.active().unwrap();
            assert_eq!(st.history.past_len(), past + 1);
            assert_eq!(st.doc.layer(layer).unwrap().surface().unwrap(), &expected);
        }
    }
}

/// #5092: a Puppet Warp session left open does not follow the user to a new document.
#[test]
fn session_is_dropped_when_another_document_becomes_active() {
    let mut app = paint_app(100, 80, Rect::new(20, 30, 80, 50));
    let ctx = app.ctx().clone();
    crate::distort_ui::menu(&mut app, &ctx, "edit.puppetWarp", &json!({})).unwrap().unwrap();

    crate::distort_ui::drop_foreign(&mut app, &ctx);
    assert!(app.distort.puppet.is_some());
    app.session.execute("file.new", json!({"width": 32, "height": 32, "depth": 8})).unwrap();
    let active = app.session.active_index().unwrap();
    // The session stays until drop_foreign; the preview must not follow this document (#5115).
    assert!(app.distort.puppet.is_some());
    assert!(crate::distort_ui::display_doc(&app, active).is_none());
    crate::distort_ui::drop_foreign(&mut app, &ctx);
    assert!(app.distort.puppet.is_none());
    assert!(crate::distort_ui::display_doc(&app, active).is_none());
    // Closing the document it was started on drops it too.
    assert!(app.session.set_active(0));
    crate::distort_ui::menu(&mut app, &ctx, "edit.puppetWarp", &json!({})).unwrap().unwrap();
    crate::distort_ui::drop_foreign(&mut app, &ctx);
    assert!(app.distort.puppet.is_some());
    assert!(app.session.close(0).is_some());
    crate::distort_ui::drop_foreign(&mut app, &ctx);
    assert!(app.distort.puppet.is_none());
}

/// #4440: ⌘Z / ⌘⇧Z undo and redo pin edits while Puppet Warp is open, and leave the document's History alone.
#[test]
fn command_z_steps_the_session_not_the_document() {
    let mut app = puppet_app();
    let past = past_len(&app);
    add_pin(&mut app, [25.0, 40.0], [25.0, 40.0]);
    add_pin(&mut app, [75.0, 40.0], [75.0, 40.0]);
    let moved = {
        let before = pins_of(&app);
        let mut after = before.clone();
        after[1].at = point([75.0, 20.0]);
        session_apply(&mut app, PinEdit::replace(&before, after, "Move Pin")).unwrap();
        pins_of(&app)
    };
    session_apply(&mut app, PinEdit::replace(&moved, moved[1..].to_vec(), "Delete Pin")).unwrap();
    assert_eq!(dsts(&app), [[75.0, 20.0]]);
    // ⌘Z through the keyboard: the removed pin comes back, then the drag is undone.
    command_z(&mut app, egui::Modifiers::COMMAND);
    assert_eq!(dsts(&app), [[25.0, 40.0], [75.0, 20.0]]);
    command_z(&mut app, egui::Modifiers::COMMAND);
    assert_eq!(dsts(&app), [[25.0, 40.0], [75.0, 40.0]]);
    command_z(&mut app, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT);
    assert_eq!(dsts(&app), [[25.0, 40.0], [75.0, 20.0]]);
    control(&mut app, &json!({"undo": true})).unwrap();
    control(&mut app, &json!({"undo": true})).unwrap();
    assert_eq!(dsts(&app), [[25.0, 40.0]]);
    // A new edit after an undo drops what was undone.
    add_pin(&mut app, [50.0, 40.0], [50.0, 40.0]);
    control(&mut app, &json!({"redo": true})).unwrap();
    assert_eq!(dsts(&app), [[25.0, 40.0], [50.0, 40.0]], "nothing left to redo");
    for _ in 0..5 {
        control(&mut app, &json!({"undo": true})).unwrap();
    }
    assert!(pins_of(&app).is_empty());
    assert_eq!(past_len(&app), past);
}

#[test]
fn mode_and_density_commit_through_the_engine() {
    let mut app = paint_app(100, 80, Rect::new(20, 30, 80, 50));
    let ctx = app.ctx().clone();
    crate::distort_ui::menu(&mut app, &ctx, "edit.puppetWarp", &json!({})).unwrap().unwrap();
    add_pin(&mut app, [25.0, 40.0], [25.0, 40.0]);
    add_pin(&mut app, [75.0, 40.0], [75.0, 20.0]);
    let v = control(&mut app, &json!({"mode": "rigid", "density": "more"})).unwrap();
    assert_eq!((v["mode"].as_str(), v["density"].as_str()), (Some("rigid"), Some("more")));
    let before = past_len(&app);
    crate::distort_ui::menu(&mut app, &ctx, "edit.puppetWarp", &json!({"ui": {"commit": true}})).unwrap().unwrap();
    assert!(app.distort.puppet.is_none());
    assert_eq!(past_len(&app), before + 1);
    let st = app.session.active().unwrap();
    let b = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds();
    assert!(b.y0 < 25, "the right end went up: {b:?}");
}

#[test]
fn settings_persist_into_the_next_session() {
    let mut app = puppet_app();
    control(&mut app, &json!({"mode": "rigid", "density": "more", "showMesh": false})).unwrap();
    control(&mut app, &json!({"cancel": true})).unwrap();
    let ctx = app.ctx().clone();
    begin(&mut app, &ctx, "edit.puppetWarp").unwrap();
    let s = app.distort.puppet.as_ref().unwrap();
    let v = s.describe();
    assert_eq!((v["mode"].as_str(), v["density"].as_str()), (Some("rigid"), Some("more")));
    assert!(!s.warp.show_mesh());
}

#[test]
fn an_expansion_that_empties_the_mesh_is_refused() {
    // #951: remeshing to no vertices rebound the pins to a vertex that doesn't exist and
    // panicked. -50 is the options bar's minimum and also empties this 20 px tall bar.
    let mut app = puppet_app();
    add_pin(&mut app, [50.0, 40.0], [50.0, 40.0]);
    let before = app.distort.puppet.as_ref().unwrap().describe();
    for e in [-200, -50] {
        assert!(control(&mut app, &json!({"expansion": e})).is_err(), "{e}");
        assert_eq!(app.distort.puppet.as_ref().unwrap().describe(), before, "{e}: the previous settings and mesh stay");
    }
    // A smaller shrink still remeshes.
    let v = control(&mut app, &json!({"expansion": -2})).unwrap();
    assert_eq!(v["expansion"], -2.0);
    assert!(v["triangles"].as_u64().unwrap() > 0);
}
