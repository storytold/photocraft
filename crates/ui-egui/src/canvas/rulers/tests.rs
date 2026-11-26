use egui::{Modifiers, Pos2, pos2, vec2};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::testing;
use crate::state::Tool;

type AppHarness = egui_kittest::Harness<'static, Box<PhotocraftApp>>;

/// The real canvas on a `size` document at 100 %, rulers on, snapping off, with `tool`.
fn canvas_harness(size: [u32; 2], tool: Tool) -> AppHarness {
    harness_with_passes(size, tool, false)
}

/// [`canvas_harness`]; with `second_pass`, every frame runs a second pass after the one that took the input.
fn harness_with_passes(size: [u32; 2], tool: Tool, second_pass: bool) -> AppHarness {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": size[0], "height": size[1], "background": "transparent"})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = true;
    app.ui.extras.snap = false;
    app.ui.tool = tool;
    let mut h = egui_kittest::Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1000.0, 700.0)).build_ui_state(
        move |ui, app: &mut Box<PhotocraftApp>| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
            if second_pass && ui.ctx().current_pass_index() == 0 {
                ui.ctx().request_discard("a second pass of the frame");
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
    h.run_steps(4);
    let v = &mut h.state_mut().ui.views[0];
    (v.zoom, v.center) = (1.0, [size[0] as f32 / 2.0, size[1] as f32 / 2.0]);
    v.clear_pending();
    h.run_steps(2);
    h
}

/// Screen point of document point `(x, y)` on the harness's canvas, through the view its last pass showed.
fn canvas_at(h: &AppHarness, x: f64, y: f64) -> Pos2 {
    testing::to_screen(h.state(), [x, y])
}

/// A point on the top ruler (`top`) or the left one, level with document point `(x, y)`.
fn ruler_at(h: &AppHarness, top: bool, x: f64, y: f64) -> Pos2 {
    let rect = testing::view(h.state()).expect("the canvas showed").rect;
    let at = canvas_at(h, x, y);
    if top { pos2(at.x, rect.top() - 4.0) } else { pos2(rect.left() - 4.0, at.y) }
}

/// Presses at `from`, moves through `path` and lets go at its last point, `modifiers` held throughout. With `release`
/// false the button stays down.
fn drag(h: &mut AppHarness, from: Pos2, path: &[Pos2], modifiers: Modifiers, release: bool) {
    h.event_modifiers(egui::Event::PointerMoved(from), modifiers);
    h.run_steps(1);
    h.event_modifiers(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers }, modifiers);
    h.run_steps(1);
    let mut at = from;
    for &to in path {
        at = to;
        h.event_modifiers(egui::Event::PointerMoved(at), modifiers);
        h.run_steps(2);
    }
    if release {
        h.event_modifiers(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers }, modifiers);
        h.run_steps(2);
    }
}

/// The steps between `a` and `b`, past the drag threshold from the first one.
fn steps(a: Pos2, b: Pos2) -> Vec<Pos2> {
    (1..=4).map(|i| a + (b - a) * (i as f32 / 4.0)).collect()
}

fn guides(app: &PhotocraftApp) -> (Vec<f64>, Vec<f64>) {
    let g = &app.session.active().unwrap().doc.guides;
    (g.vertical.clone(), g.horizontal.clone())
}

/// The Info panel's X and Y as it would word them now.
fn info_position(app: &mut PhotocraftApp) -> [String; 2] {
    let rows = crate::panels::info::sampler_rows(app);
    let readings = crate::panels::info::readings(app, &rows);
    readings.position.expect("a position").map(|label| label.as_str().to_owned())
}

/// #2955: the Info panel's X/Y are measured from the rulers' zero point.
#[test]
fn info_panel_position_reads_from_the_zero_point() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("view.setRulerOrigin", json!({"x": 20, "y": 5})).unwrap();
    app.hover_doc = Some([50.5, 20.5]);
    let [x, y] = info_position(&mut app);
    assert!(x.starts_with("30") && y.starts_with("15"), "X 30, Y 15 from the zero point at (20, 5): {x}, {y}");
    assert!(!x.starts_with("50"), "not the raw image x");
}

/// #5728: a guide dragged out of a ruler shows its coordinate in the Info panel on its own axis, read from the
/// running guide drag and not the pointer.
#[test]
fn info_panel_shows_guide_drag_position_outside_canvas() {
    let mut h = canvas_harness([200, 100], Tool::Move);
    let digits = |s: &str| s.chars().any(|c| c.is_ascii_digit());
    for (top, at, axis, want) in [(false, [42.0, 60.0], 0, "42"), (true, [80.0, 17.0], 1, "17")] {
        let from = ruler_at(&h, top, at[0], at[1]);
        let to = canvas_at(&h, at[0], at[1]);
        drag(&mut h, from, &steps(from, to), Modifiers::NONE, false);
        // The pointer's own reading is gone: only the guide drag can give the axis a value.
        h.state_mut().hover_doc = None;
        let position = info_position(h.state_mut());
        assert!(position[axis].starts_with(want), "the dragged guide's axis reads {want}: {position:?}");
        assert!(!digits(&position[1 - axis]), "the other axis has no reading: {position:?}");
        h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(2);
    }
}

/// View › Rulers is remembered: the next launch, and every document created or opened in it,
/// shows the rulers as last set, on or off (#2918).
#[test]
fn rulers_open_as_last_set_after_a_restart() {
    use std::sync::Arc;
    let files = Arc::new(craft::settings::testing::MemoryFiles::new());
    let scratch = std::env::temp_dir().join("photocraft-rulers-restart");
    let launch = |files: &Arc<craft::settings::testing::MemoryFiles>| {
        let options = craft::settings::OpenOptions::new(craft::settings::Location::Dir(scratch.clone())).files(files.clone());
        PhotocraftApp::new(
            photocraft_engine::Session::new(),
            crate::Services { settings: Some(craft::settings::SettingsStore::open_with(options)), ..Default::default() },
        )
    };
    for on in [true, false] {
        let mut app = launch(&files);
        let app_ctx = app.ctx().clone();
        assert_eq!(crate::menus::invoke(&mut app, &app_ctx, "view.rulers", json!({})).unwrap(), json!(on));
        crate::prefs_ui::save_preferences(&mut app).unwrap();
        let mut next = launch(&files);
        next.session.execute("file.new", json!({"width": 20, "height": 10})).unwrap();
        next.sync_views();
        assert_eq!(next.ui.extras.rulers, on, "a new document after a restart shows the rulers as last set");
    }
}

/// Each guide edit runs its engine command on the guide it names, one undo step each: a ruler drag places a guide
/// (`view.newGuide`), a drag moves the second guide of an orientation (`view.moveGuide`, index 1, not 0), and a guide
/// dropped on a ruler goes (`view.deleteGuide`, index 1). The keys `Guides` names guides by map back to the engine's
/// per-orientation index. Every frame runs a second pass, which must not run a command again.
#[test]
fn guide_edits_run_their_view_commands() {
    type Edit = fn(&mut AppHarness);
    let place_horizontal: Edit = |h| {
        let (from, to) = (ruler_at(h, true, 100.0, 40.0), canvas_at(h, 100.0, 45.0));
        drag(h, from, &steps(from, to), Modifiers::NONE, true);
    };
    let place_vertical: Edit = |h| {
        let (from, to) = (ruler_at(h, false, 30.0, 100.0), canvas_at(h, 30.0, 100.0));
        drag(h, from, &steps(from, to), Modifiers::NONE, true);
    };
    let move_second_vertical: Edit = |h| {
        let (from, to) = (canvas_at(h, 50.0, 100.0), canvas_at(h, 70.0, 100.0));
        drag(h, from, &steps(from, to), Modifiers::NONE, true);
    };
    let move_second_horizontal: Edit = |h| {
        let (from, to) = (canvas_at(h, 150.0, 40.0), canvas_at(h, 150.0, 90.0));
        drag(h, from, &steps(from, to), Modifiers::NONE, true);
    };
    let drop_second_horizontal_on_a_ruler: Edit = |h| {
        let (from, to) = (canvas_at(h, 150.0, 40.0), ruler_at(h, true, 150.0, 40.0));
        drag(h, from, &steps(from, to), Modifiers::NONE, true);
    };
    type Guides = (Vec<f64>, Vec<f64>);
    let table: [(&str, Edit, Guides); 5] = [
        ("view.newGuide horizontal", place_horizontal, (vec![20.0, 50.0], vec![10.0, 40.0, 45.0])),
        ("view.newGuide vertical", place_vertical, (vec![20.0, 50.0, 30.0], vec![10.0, 40.0])),
        ("view.moveGuide vertical 1", move_second_vertical, (vec![20.0, 70.0], vec![10.0, 40.0])),
        ("view.moveGuide horizontal 1", move_second_horizontal, (vec![20.0, 50.0], vec![10.0, 90.0])),
        ("view.deleteGuide horizontal 1", drop_second_horizontal_on_a_ruler, (vec![20.0, 50.0], vec![10.0])),
    ];
    for (name, edit, want) in table {
        let mut h = harness_with_passes([200, 150], Tool::Move, true);
        for (orientation, position) in [("vertical", 20), ("vertical", 50), ("horizontal", 10), ("horizontal", 40)] {
            h.state_mut().run("view.newGuide", json!({"orientation": orientation, "position": position})).unwrap();
        }
        h.run_steps(2);
        let before = guides(h.state());
        edit(&mut h);
        let mut got = guides(h.state());
        got.0.sort_by(f64::total_cmp);
        got.1.sort_by(f64::total_cmp);
        let mut want = want;
        want.0.sort_by(f64::total_cmp);
        want.1.sort_by(f64::total_cmp);
        assert_eq!(got, want, "{name}");
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(guides(h.state()), before, "{name}: one undo step");
    }
}

/// ⌘ is the Move tool with any tool, so a ⌘-drag over a guide moves the guide, never the layer
/// (#2690); with Lock Guides a ⌘-drag with a marquee moves the layer as before.
#[test]
fn command_drag_over_a_guide_moves_the_guide_with_any_tool() {
    for (tool, locked) in [
        (Tool::Brush, false),
        (Tool::Eraser, false),
        (Tool::Gradient, false),
        (Tool::Frame, false),
        (Tool::Artboard, false),
        (Tool::RectMarquee, false),
        (Tool::Lasso, false),
        (Tool::MagicWand, false),
        (Tool::RectMarquee, true),
    ] {
        let mut h = canvas_harness([80, 60], tool);
        let app = h.state_mut();
        app.ui.extras.lock_guides = locked;
        let layer = app.session.active().unwrap().active_layer.unwrap();
        app.session
            .edit("paint", |doc, _| {
                doc.layer_mut(layer).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(10, 10, 30, 30), &[1.0, 0.0, 0.0, 1.0]);
                Ok(())
            })
            .unwrap();
        app.run("view.newGuide", json!({"orientation": "vertical", "position": 50})).unwrap();
        h.run_steps(2);
        let (from, to) = (canvas_at(&h, 50.0, 40.0), canvas_at(&h, 65.0, 40.0));
        h.event_modifiers(egui::Event::PointerMoved(from), Modifiers::COMMAND);
        h.run_steps(1);
        h.event_modifiers(
            egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::COMMAND },
            Modifiers::COMMAND,
        );
        h.run_steps(1);
        for at in steps(from, to) {
            h.event_modifiers(egui::Event::PointerMoved(at), Modifiers::COMMAND);
            h.run_steps(2);
        }
        h.event_modifiers(
            egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::COMMAND },
            Modifiers::COMMAND,
        );
        h.run_steps(2);
        let doc = &h.state().session.active().unwrap().doc;
        let moved = doc.layer(layer).unwrap().surface().unwrap().rgba(12, 12)[3] == 0.0;
        if locked {
            assert_eq!((doc.guides.vertical.clone(), moved), (vec![50.0], true), "{tool:?}, guides locked: the layer moves");
        } else {
            assert_eq!((doc.guides.vertical.clone(), moved), (vec![65.0], false), "{tool:?}: the guide moves, the layer stays");
        }
    }
}
