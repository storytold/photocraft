//! #12519: the Adjustment Brush Tool in the shell: its toolbar slot between the Blur and Dodge
//! slots (no letter), its options bar, and strokes through the real pointer path that make an
//! adjustment layer whose mask is the stroke, or paint into it.

use egui::accesskit::Role;
use egui::{Modifiers, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::{Adjustment, LayerContent};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::panels::toolbar::TOOL_SECTIONS;
use crate::state::Tool;

/// A 200 x 200 document with a painted layer active and the Adjustment Brush picked, its brush
/// swapped in as a frame does.
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 200, "background": "white"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#808080"})).unwrap();
    pick(&mut app);
    app
}

fn pick(app: &mut PhotocraftApp) {
    app.ui.tool = Tool::AdjustmentBrush;
    crate::paint_mouse::sync_tool_brush(app);
    app.sync_views();
}

fn stroke(app: &mut PhotocraftApp, y: f64) {
    tool_event(app, ToolEvent::Down { x: 40.0, y, pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Move { x: 160.0, y, pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x: 160.0, y }, Modifiers::NONE);
    app.sync_views();
}

fn active(app: &PhotocraftApp) -> &photocraft_doc::Layer {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap()
}

fn layers(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.walk().len()
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

fn mask_at(app: &PhotocraftApp, x: i32, y: i32) -> f32 {
    active(app).mask.as_ref().expect("a mask").surface.sample_channel(x, y, 0)
}

/// The messages of the open framework alerts.
fn alerts(app: &PhotocraftApp) -> Vec<String> {
    app.framework.dialogs.open_dialogs().iter().filter_map(|d| d.fields.as_ref()?.get("message")?.as_str().map(str::to_string)).collect()
}

#[test]
fn its_own_slot_sits_between_the_blur_and_dodge_slots() {
    let slots: Vec<&[Tool]> = TOOL_SECTIONS.iter().flat_map(|s| s.iter().copied()).collect();
    let i = slots.iter().position(|s| *s == [Tool::AdjustmentBrush]).expect("an own slot");
    assert_eq!(slots[i - 1], [Tool::Blur, Tool::Sharpen, Tool::Smudge]);
    assert_eq!(slots[i + 1], [Tool::Dodge, Tool::Burn, Tool::Sponge]);
    assert_eq!(Tool::AdjustmentBrush.key(), '\0', "no letter");
    assert_eq!(Tool::from_name("adjustmentBrush"), Some(Tool::AdjustmentBrush));
    let command = crate::panels::tool_commands::command_of(Tool::AdjustmentBrush);
    assert_eq!(crate::panels::tool_commands::tool_of(command), Tool::AdjustmentBrush);
    assert_eq!(crate::icons::tool_icon(Tool::AdjustmentBrush), craft::behavior::IconId::new("adjustment_layer"));
}

#[test]
fn it_starts_with_a_100_px_soft_round() {
    let app = app();
    let b = &app.session.tools.brush;
    assert_eq!((b.size, b.hardness), (100.0, 0.0));
}

fn bar(app: Box<PhotocraftApp>) -> Harness<'static, Box<PhotocraftApp>> {
    crate::theme::install_fonts(app.ctx());
    crate::theme::apply(app.ctx(), &crate::theme::AppTheme::pro());
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1800.0, 200.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut Box<PhotocraftApp>| {
            crate::panels::options_bar(app, ui);
            crate::dialogs::run(app, ui);
        },
        app,
    );
    h.run_steps(4);
    h
}

/// The centre x of the control named `label` (not a caption's text node).
fn x_of(h: &Harness<'static, Box<PhotocraftApp>>, label: &str) -> f32 {
    let named = h.query_all_by_label(label).find(|n| n.accesskit_node().role() != Role::Label);
    let all = || h.query_all_by_role(Role::Button).filter_map(|n| n.accesskit_node().label()).collect::<Vec<_>>();
    named.unwrap_or_else(|| panic!("no {label:?} in {:?}", all())).rect().center().x
}

#[test]
fn the_options_bar_shows_its_controls_in_order() {
    let h = bar(app());
    let xs = [
        x_of(&h, "Adjustment:"),
        x_of(&h, "Subtract from mask areas to remove adjustment from areas of the image"),
        x_of(&h, "Add to mask areas to apply adjustment to more of the image"),
        x_of(&h, "Always use pressure for size"),
        x_of(&h, "Choose an object and have the adjustment applied to it"),
        x_of(&h, "Overlay"),
        x_of(&h, "Opacity:"),
        x_of(&h, "Always use pressure for opacity"),
        x_of(&h, "Flow:"),
        x_of(&h, "Enable airbrush-style build-up effects"),
    ];
    assert!(xs.windows(2).all(|w| w[0] < w[1]), "left to right: {xs:?}");
}

#[test]
fn subtract_and_add_swap_the_colours_and_the_paint() {
    let mut h = bar(app());
    h.get_by_label("Subtract from mask areas to remove adjustment from areas of the image").click();
    h.run_steps(2);
    let app = h.state();
    assert!(app.ui.tool_options.adjustment_brush_subtract);
    assert_eq!((app.session.tools.foreground, app.session.tools.background), ([0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0]));
    h.get_by_label("Add to mask areas to apply adjustment to more of the image").click();
    h.run_steps(2);
    let app = h.state();
    assert!(!app.ui.tool_options.adjustment_brush_subtract);
    assert_eq!((app.session.tools.foreground, app.session.tools.background), ([1.0, 1.0, 1.0, 1.0], [0.0, 0.0, 0.0, 1.0]));
}

#[test]
fn the_first_stroke_makes_the_layer_in_one_history_state() {
    let mut app = app();
    let (n, past) = (layers(&app), steps(&app));
    stroke(&mut app, 100.0);
    assert_eq!((layers(&app), steps(&app)), (n + 1, past + 1), "{}", app.ui.status);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("New Adjustment Layer"));
    assert_eq!(active(&app).name, "Levels 1");
    assert!(matches!(active(&app).content, LayerContent::Adjustment(Adjustment::Levels { .. })));
    assert!(mask_at(&app, 100, 100) > 0.98 && mask_at(&app, 100, 10) == 0.0);
    assert!(app.ui.mask_target, "the new mask is targeted");
    // A second stroke paints the same mask.
    stroke(&mut app, 30.0);
    assert_eq!((layers(&app), steps(&app)), (n + 1, past + 2));
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Adjustment Brush"));
    assert!(mask_at(&app, 100, 30) > 0.98);
}

#[test]
fn a_hidden_locked_or_group_layer_does_not_refuse_the_stroke() {
    for setup in ["hidden", "locked", "group", "type"] {
        let mut app = app();
        let id = active(&app).id.0;
        match setup {
            "hidden" => app.run("layer.setProps", json!({"layer": id, "visible": false})).map(|_| ()),
            "locked" => app.run("layer.lockLayers", json!({"all": true})).map(|_| ()),
            "group" => app.run("layer.new.group", json!({})).map(|_| ()),
            _ => app.run("type.create", json!({"text": "Hi", "x": 20, "y": 40})).map(|_| ()),
        }
        .unwrap_or_else(|e| panic!("{setup}: {e}"));
        pick(&mut app);
        let n = layers(&app);
        stroke(&mut app, 100.0);
        assert!(app.framework.dialogs.open_dialogs().is_empty(), "{setup}: {:?}", alerts(&app));
        assert_eq!(layers(&app), n + 1, "{setup}: {}", app.ui.status);
        assert!(matches!(active(&app).content, LayerContent::Adjustment(_)), "{setup}");
    }
}

#[test]
fn selecting_an_adjustment_layer_sets_the_dropdown() {
    let mut app = app();
    app.run("layer.newAdjustmentLayer.curves", json!({})).unwrap();
    crate::adjustment_brush::follow_active_layer(&mut app);
    assert_eq!(crate::adjustment_brush::kind(&app), "curves");
    // A pick after that sticks while the layer stays active.
    app.ui.tool_options.adjustment_brush_kind = "exposure".into();
    crate::adjustment_brush::follow_active_layer(&mut app);
    assert_eq!(crate::adjustment_brush::kind(&app), "exposure");
    // And a stroke with it makes an Exposure layer above the Curves one.
    let n = layers(&app);
    stroke(&mut app, 100.0);
    assert_eq!((layers(&app), active(&app).name.as_str()), (n + 1, "Exposure 1"));
}

#[test]
fn greyed_entries_fall_back_to_the_first_offered_one() {
    let mut app = app();
    app.run("image.mode.bits32", json!({})).unwrap();
    app.ui.tool_options.adjustment_brush_kind = "posterize".into();
    assert_eq!(crate::adjustment_brush::kind(&app), "levels");
    stroke(&mut app, 100.0);
    assert_eq!(active(&app).name, "Levels 1", "{}", app.ui.status);
}

#[test]
fn indexed_color_alerts_at_the_press_and_changes_nothing() {
    let mut app = app();
    app.run("layer.flatten", json!({})).ok();
    app.run("image.mode.indexedColor", json!({})).unwrap();
    pick(&mut app);
    let (n, past) = (layers(&app), steps(&app));
    stroke(&mut app, 100.0);
    assert_eq!(alerts(&app), [photocraft_engine::adjustment_brush_cmds::INDEXED_ALERT]);
    assert_eq!((layers(&app), steps(&app)), (n, past));
}
