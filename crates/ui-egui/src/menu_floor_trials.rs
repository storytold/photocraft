//! PhotoCraft's frame floor (frame-design-v2 2.2, 2.3): an idle frame keys the same bytes whatever the size of the
//! documents, presets and recent files, and builds no derived value; a status message runs only the status bar's view;
//! the menu model is built only when a fact it shows changed, never for a tool option or a button release, and the
//! held model always equals a fresh build.

use craft::ui::runtime::measure::{MemoRun, last_frame_derived_builds, last_frame_memo_runs, last_pass_memo_key_bytes};
use egui::{Event, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

type H = Harness<'static, Box<PhotocraftApp>>;

/// The app with `documents` open documents, `recent` recent files and `swatches` extra swatches, the panels not
/// ported to memoized views closed, settled.
fn harness(documents: usize, recent: usize, swatches: usize) -> H {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).with_step_dt(1.0 / 60.0).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut s = photocraft_engine::Session::new();
        for _ in 0..documents {
            s.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
        }
        s.presets.edit(|presets| {
            if let Some(group) = presets.swatches.first_mut()
                && let Some(entry) = group.entries.first().cloned()
            {
                group.entries.extend((0..swatches).map(|i| craft::color_ui::palette::SwatchEntry { name: format!("Swatch {i}"), ..entry.clone() }));
            }
        });
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), s, crate::Services::default());
        app.ui.recent_files.edit(|files| files.extend((0..recent).map(|i| format!("/tmp/recent-{i}.psd"))));
        app.ui.tool = Tool::Brush;
        app
    });
    h.run_steps(8);
    let closes = ["properties", "adjustments", "layers", "channels", "paths"].map(|id| craft::behavior::dock::Action::close(id.to_owned()));
    let _ = std::sync::Arc::make_mut(&mut h.state_mut().ui.docking).apply_all(closes, false, |panel: &String| crate::panels::registry::permissions(panel));
    h.run_steps(8);
    h
}

/// How many times the last frame built the menu model.
fn model_builds(h: &H) -> u32 {
    let id = crate::menus::model_id();
    last_frame_derived_builds(&h.ctx).iter().filter(|(built, _)| *built == id).map(|(_, n)| n).sum()
}

fn press(h: &mut H, at: Pos2, pressed: bool) {
    h.event(Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
}

#[test]
fn an_idle_frame_keys_the_same_bytes_whatever_the_model_size() {
    let idle = |mut h: H| {
        h.run_steps(1);
        last_pass_memo_key_bytes(&h.ctx)
    };
    let small = idle(harness(1, 0, 0));
    let large = idle(harness(10, 40, 500));
    assert!(small > 0, "the idle frame keys its views");
    assert_eq!(large, small, "10 documents, 40 recent files and 500 more swatches key what 1 document does");
}

#[test]
fn a_settled_frame_builds_no_derived_value() {
    let mut h = harness(1, 3, 0);
    for frame in 0..30 {
        h.run_steps(1);
        assert_eq!(last_frame_derived_builds(&h.ctx), [], "settled frame {frame} builds nothing");
    }
}

#[test]
fn a_status_message_runs_only_the_status_bar() {
    let mut h = harness(1, 0, 0);
    h.run_steps(2);
    h.state_mut().ui.status = "Saved".into();
    h.run_steps(1);
    let ran: Vec<&'static str> = last_frame_memo_runs(&h.ctx).into_iter().filter(|(_, _, run)| *run == MemoRun::Ran).map(|(name, _, _)| name).collect();
    assert!(!ran.is_empty() && ran.iter().all(|name| name.contains("status_bar::StatusProps")), "only the status bar's view runs: {ran:?}");
}

#[test]
fn the_menu_model_is_built_only_when_a_fact_it_shows_changed() {
    let mut h = harness(1, 0, 0);
    h.run_steps(2);
    h.state_mut().run("tools.setBrush", json!({"brush": {"size": 42}})).unwrap();
    h.run_steps(1);
    assert_eq!(model_builds(&h), 0, "a tool option builds no menu model");

    h.state_mut().ui.tool = Tool::Hand;
    h.run_steps(2);
    let at = h.state().last_canvas_rect.center();
    h.event(Event::PointerMoved(at));
    h.run_steps(1);
    press(&mut h, at, true);
    h.run_steps(1);
    press(&mut h, at, false);
    h.run_steps(1);
    assert_eq!(model_builds(&h), 0, "a button release builds no menu model");

    let undo = |h: &mut H| {
        let ctx = h.ctx.clone();
        crate::menus::row(crate::menus::model(h.state_mut(), &ctx).rows(), "edit.undo").map(|row| row.enabled)
    };
    assert_eq!(undo(&mut h), Some(false), "nothing to undo in a new document");
    h.state_mut().run("layer.new.layer", json!({})).unwrap();
    h.run_steps(1);
    assert_eq!(model_builds(&h), 1, "a new layer builds the model once");
    assert_eq!(undo(&mut h), Some(true), "and Edit › Undo follows it");
}

#[test]
fn the_held_menu_model_equals_a_fresh_build_after_each_step() {
    let mut h = harness(1, 0, 0);
    let held_is_fresh = |h: &mut H, step: &str| {
        h.run_steps(2);
        let ctx = h.ctx.clone();
        let held = (*crate::menus::model(h.state_mut(), &ctx).bar).clone();
        assert!(held == crate::menus::fresh_bar(h.state()), "after {step}, the held menu model is a fresh build");
    };
    let run = |h: &mut H, id: &str, params: serde_json::Value| {
        h.state_mut().run(id, params).unwrap();
    };
    // The shell's own commands, as a menu pick runs them.
    let pick = |h: &mut H, id: &str| {
        let ctx = h.ctx.clone();
        crate::menus::invoke(h.state_mut(), &ctx, id, json!({})).unwrap();
    };
    held_is_fresh(&mut h, "settling");
    run(&mut h, "layer.new.layer", json!({}));
    held_is_fresh(&mut h, "a new layer");
    run(&mut h, "edit.undo", json!({}));
    held_is_fresh(&mut h, "undo");
    run(&mut h, "edit.redo", json!({}));
    held_is_fresh(&mut h, "redo");
    run(&mut h, "layer.selectBelow", json!({}));
    held_is_fresh(&mut h, "selecting a layer");
    pick(&mut h, "view.rulers");
    held_is_fresh(&mut h, "toggling the rulers");
    h.state_mut().ui.recent_files.edit(|files| files.insert(0, "/tmp/recent.psd".into()));
    held_is_fresh(&mut h, "a recent file");
    run(&mut h, "select.all", json!({}));
    run(&mut h, "edit.copy", json!({}));
    held_is_fresh(&mut h, "a copy");
    pick(&mut h, "edit.freeTransform");
    held_is_fresh(&mut h, "opening Free Transform");
    h.key_press(egui::Key::Escape);
    held_is_fresh(&mut h, "cancelling Free Transform");
    h.state_mut().session.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
    held_is_fresh(&mut h, "opening a document");
    run(&mut h, "file.close", json!({}));
    held_is_fresh(&mut h, "closing it");
    h.state_mut().session.saved_to("/tmp/Renamed.psd".into());
    held_is_fresh(&mut h, "a save that renames the document");
    // Three documents, the first active: moving the second after the third keeps the active document where it was,
    // and the Window menu's second entry opens the other document now (#16252).
    for name in ["Second", "Third"] {
        h.state_mut().session.execute("file.new", json!({"width": 200, "height": 100, "name": name})).unwrap();
    }
    h.state_mut().session.set_active(0);
    held_is_fresh(&mut h, "opening two more documents");
    assert_eq!(h.state_mut().session.move_document(1, 2), Some(2));
    held_is_fresh(&mut h, "reordering the documents behind the active one");
}

#[test]
fn the_status_bar_sits_inside_the_window_when_its_views_run_and_replay() {
    let mut h = harness(1, 0, 0);
    let screen = h.ctx.content_rect();
    // The status bar's zoom field: the lowest node named Zoom that shows the percentage as its value.
    let field = |h: &H| {
        h.query_all_by_label("Zoom")
            .filter(|n| {
                let a = n.accesskit_node();
                a.role() != egui::accesskit::Role::Slider && a.value().is_some() && a.bounding_box().is_some()
            })
            .map(|n| n.rect())
            .max_by(|a, b| a.bottom().total_cmp(&b.bottom()))
    };
    // A settled frame replays the bar's views; a status message runs its body again.
    for step in ["settled", "a status message"] {
        if step == "a status message" {
            h.state_mut().ui.status = "Saved".into();
        }
        h.run_steps(2);
        let Some(field) = field(&h) else { panic!("{step}: the status bar shows its zoom field") };
        assert!(
            field.bottom() <= screen.bottom() && field.top() >= screen.bottom() - 40.0,
            "{step}: the zoom field {field:?} sits in the status bar inside the window {screen:?}, not moved down past its edge"
        );
    }
}

/// The status message and the render-path note at the bar's far end are shown in memoized views, whose content the
/// bar's row does not lay out: each must still stand on the readout's line, in the middle of the bar (#16352).
#[test]
fn the_status_message_and_the_trailing_note_stand_on_the_readouts_line() {
    let mut h = harness(1, 0, 0);
    let message = "Scratch disk is low on free space";
    h.state_mut().ui.status = message.into();
    h.state_mut().ui.status_error = true;
    let doc = h.state().session.active().map(|st| st.doc.id.0).expect("a document is open");
    h.state_mut().perf.gpu_layer_fit = Some(crate::canvas::gpu_canvas::LayerFit { doc, needed: 2 << 30, budget: 1 << 30 });
    h.run_steps(4);
    let centre = |h: &H, label: &str| {
        let Some(node) = h.query_all_by_label(label).max_by(|a, b| a.rect().bottom().total_cmp(&b.rect().bottom())) else {
            panic!("the status bar shows {label:?}")
        };
        node.rect().center().y
    };
    let readout = centre(&h, "400 px x 300 px (72 ppi)");
    let Some(bar) = h.query_all_by_role(egui::accesskit::Role::Status).find(|n| n.accesskit_node().label().as_deref() == Some("Status")) else {
        panic!("the window shows the status bar")
    };
    let middle = bar.rect().center().y;
    assert!((readout - middle).abs() <= 0.5, "the readout is centred at y {readout}, in the middle of the status bar at y {middle}");
    for label in [message, "GPU Memory Full"] {
        let y = centre(&h, label);
        assert!((y - readout).abs() <= 0.5, "{label:?} is centred at y {y}, on the readout's line at y {readout} (#16352)");
    }
}
