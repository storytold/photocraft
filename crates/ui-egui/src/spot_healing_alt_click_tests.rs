//! #12563: an Alt-click (Option-click) with the Spot Healing Brush heals nothing and opens the
//! reference application's alert with Continue (the default), Cancel and "Don't show again"; the
//! document, History and the tool stay as they were. Once the box is ticked, later Alt-clicks heal
//! nothing with no alert. A plain click, a drag and a Shift-click heal as before, and the Clone
//! Stamp's and the Healing Brush's Alt-click still set their source.

use craft::ui::dialog::{DialogId, StackDialog as _, StandardPrompt};
use egui::Modifiers;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::PhotocraftApp;
use crate::cam_no_pixels_alert_tests::steps;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

const MESSAGE: &str =
    "The Spot Healing Brush Tool automatically samples pixels to correct a spot with one step. To manually set the source, use the Healing Brush Tool.";

/// The centre of the black spot.
const SPOT: [f64; 2] = [48.0, 48.0];

/// A 100 x 100 document whose active pixel layer is mid grey with a black 6 x 6 spot at 45,45, and
/// `tool` with a 20 px tip.
fn app(tool: Tool) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 100, "height": 100, "background": "white"})).unwrap();
    app.run("layer.new.layer", json!({"name": "Spot"})).unwrap();
    app.session
        .edit("paint", |doc, a| {
            let s = doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap();
            let grey = 128.0 / 255.0;
            s.fill_rect(photocraft_geom::Rect::new(0, 0, 100, 100), &[grey, grey, grey, 1.0]);
            s.fill_rect(photocraft_geom::Rect::new(45, 45, 51, 51), &[0.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.session.tools.brush.size = 20.0;
    app.ui.tool = tool;
    app
}

/// A click at `at` with `m` held.
fn click(app: &mut PhotocraftApp, at: [f64; 2], m: Modifiers) {
    let [x, y] = at;
    tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Up { x, y }, m);
}

/// The active layer's pixel at (x, y) as 8-bit RGB.
fn pixel(app: &PhotocraftApp, x: i32, y: i32) -> [u8; 3] {
    let st = app.session.active().unwrap();
    let c = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y);
    [0, 1, 2].map(|i| (c[i] * 255.0).round() as u8)
}

fn healed(app: &PhotocraftApp) -> bool {
    pixel(app, 48, 48) != [0, 0, 0]
}

/// The open alerts of the "Don't show again" family.
fn alerts(app: &PhotocraftApp) -> Vec<DialogId> {
    crate::dialogs::alerts::alerts(app)
}

fn last_step(app: &PhotocraftApp) -> Option<String> {
    app.session.active().unwrap().history.undo_label().map(str::to_owned)
}

/// An Alt-click: one alert, nothing healed, no History state, the tool kept.
fn alt_click_refused(app: &mut PhotocraftApp) {
    let before = steps(app);
    click(app, SPOT, Modifiers::ALT);
    assert_eq!(alerts(app).len(), 1, "the alert");
    assert!(!healed(app), "the spot is still black");
    assert_eq!(steps(app), before, "no History state");
    assert_eq!(app.ui.tool, Tool::SpotHealing, "the tool stays");
}

#[test]
fn an_alt_click_opens_the_alert_with_continue_cancel_and_dont_show_again() {
    let app = app(Tool::SpotHealing);
    let ctx = app.ctx().clone();
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(800.0, 600.0))
        .with_context(ctx.clone())
        .build_ui_state(|ui, app| crate::dialogs::run(app, ui), app);
    alt_click_refused(h.state_mut());
    h.run_steps(5);
    assert!(h.query_by_label(MESSAGE).is_some(), "the alert's text");
    for label in ["Continue", "Cancel", "Don't show again"] {
        assert!(h.query_by_label(label).is_some(), "{label}");
    }
    assert!(h.query_by_label("OK").is_none(), "no OK");
    let fields = h.state().framework.dialogs.get::<StandardPrompt>().and_then(|prompt| prompt.json_fields());
    assert_eq!(fields.map(|f| f["choice"].clone()), Some(json!("proceed")), "Continue is the default");
}

#[test]
fn continue_and_cancel_close_it_with_nothing_healed() {
    for cancel in [false, true] {
        let mut app = app(Tool::SpotHealing);
        let before = steps(&app);
        alt_click_refused(&mut app);
        let (ctx, id) = (app.ctx().clone(), alerts(&app)[0]);
        if cancel {
            crate::dialogs::cancel(&mut app, &ctx, id).unwrap();
        } else {
            crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
        }
        assert!(alerts(&app).is_empty(), "cancel={cancel}: closed");
        assert!(!healed(&app), "cancel={cancel}: nothing healed");
        assert_eq!(steps(&app), before, "cancel={cancel}: no History state");
        assert_eq!(app.ui.tool, Tool::SpotHealing, "cancel={cancel}: the tool stays");
        // Unticked, the next Alt-click alerts again.
        alt_click_refused(&mut app);
    }
}

#[test]
fn once_dont_show_again_is_ticked_alt_clicks_heal_nothing_without_the_alert() {
    let mut app = app(Tool::SpotHealing);
    alt_click_refused(&mut app);
    let (ctx, id) = (app.ctx().clone(), alerts(&app)[0]);
    app.framework.dialogs.set_json_field(&ctx, id, "skip", json!(true)).unwrap();
    crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
    assert_eq!(app.session.prefs().dialogs.get(crate::retouch_ui::SPOT_HEALING_ALT_PREF), Some(&json!({"skip": true})), "kept with the dialog preferences");
    let before = steps(&app);
    for _ in 0..2 {
        click(&mut app, SPOT, Modifiers::ALT);
        assert!(alerts(&app).is_empty(), "no alert any more");
        assert!(!healed(&app), "nothing healed");
        assert_eq!(steps(&app), before, "no History state");
    }
}

#[test]
fn a_plain_click_a_drag_and_a_shift_click_heal_as_before() {
    type Stroke = fn(&mut PhotocraftApp);
    let strokes: [(&str, Stroke); 3] = [
        ("click", |app| click(app, SPOT, Modifiers::NONE)),
        ("drag", |app| {
            tool_event(app, ToolEvent::Down { x: 40.0, y: 48.0, pressure: 1.0 }, Modifiers::NONE);
            tool_event(app, ToolEvent::Move { x: 48.0, y: 48.0, pressure: 1.0 }, Modifiers::NONE);
            tool_event(app, ToolEvent::Move { x: 56.0, y: 48.0, pressure: 1.0 }, Modifiers::NONE);
            tool_event(app, ToolEvent::Up { x: 56.0, y: 48.0 }, Modifiers::NONE);
        }),
        ("shift-click", |app| click(app, SPOT, Modifiers::SHIFT)),
    ];
    for (name, stroke) in strokes {
        let mut app = app(Tool::SpotHealing);
        let before = steps(&app);
        stroke(&mut app);
        assert!(alerts(&app).is_empty(), "{name}: no alert");
        assert!(healed(&app), "{name}: healed");
        assert_eq!(steps(&app), before + 1, "{name}: one History state");
        assert_eq!(last_step(&app).as_deref(), Some("Spot Healing Brush"), "{name}");
    }
}

#[test]
fn the_clone_stamp_and_the_healing_brush_alt_click_still_set_the_source() {
    for tool in [Tool::CloneStamp, Tool::Healing] {
        let mut app = app(tool);
        let before = steps(&app);
        click(&mut app, [10.0, 10.0], Modifiers::ALT);
        assert!(alerts(&app).is_empty() && app.framework.dialogs.open_dialogs().is_empty(), "{tool:?}: no alert");
        assert_eq!(steps(&app), before, "{tool:?}: the Alt-click paints nothing");
        // With the source set, a click paints instead of saying the source is missing.
        click(&mut app, SPOT, Modifiers::NONE);
        assert!(app.framework.dialogs.open_dialogs().is_empty(), "{tool:?}: no missing-source alert");
        assert_eq!(steps(&app), before + 1, "{tool:?}: the click paints");
    }
}

#[test]
fn the_dont_show_again_entry_survives_the_next_launch_move() {
    let mut app = app(Tool::SpotHealing);
    alt_click_refused(&mut app);
    let (ctx, id) = (app.ctx().clone(), alerts(&app)[0]);
    app.framework.dialogs.set_json_field(&ctx, id, "skip", json!(true)).unwrap();
    crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
    // The first frame of the next launch moves the remembered dialog values out of the preferences.
    app.framework.move_memories(&mut app.session);
    assert_eq!(
        app.session.prefs().dialogs.get(crate::retouch_ui::SPOT_HEALING_ALT_PREF),
        Some(&json!({"skip": true})),
        "still kept with the dialog preferences"
    );
    click(&mut app, SPOT, Modifiers::ALT);
    assert!(alerts(&app).is_empty(), "no alert after the move");
}

#[test]
fn an_alt_click_with_a_group_target_shows_the_spot_healing_alert() {
    let mut app = app(Tool::SpotHealing);
    app.run("layer.new.group", json!({})).unwrap();
    assert!(app.session.active().and_then(|st| st.doc.layer(st.active_layer?)).is_some_and(photocraft_doc::Layer::is_group), "a group is the target");
    let ctx = app.ctx().clone();
    let mut h =
        egui_kittest::Harness::builder().with_size(egui::vec2(800.0, 600.0)).with_context(ctx).build_ui_state(|ui, app| crate::dialogs::run(app, ui), app);
    click(h.state_mut(), SPOT, Modifiers::ALT);
    h.run_steps(5);
    assert!(h.query_by_label(MESSAGE).is_some(), "the Spot Healing alert");
    assert!(h.query_by_label_contains("because the target layer is a group").is_none(), "not the group alert");
}
