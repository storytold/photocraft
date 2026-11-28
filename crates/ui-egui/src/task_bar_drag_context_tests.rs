//! A dragged, unpinned contextual task bar goes back to automatic placement on the next context change (#16758).

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::app::framework::BuildApp as _;
use crate::control::{ControlRequest, Reply, handle};
use crate::state::Tool;
use craft::behavior::geometry::CropBox;
use craft::behavior::kurbo::Rect as DocRect;

type AppHarness = Harness<'static, Box<PhotocraftApp>>;

fn harness() -> AppHarness {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 100, "y": 100, "width": 100, "height": 60})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = false;
    app.ui.tool = Tool::Move;
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(egui::vec2(1000.0, 700.0)).build_app(
        |ui, app: &mut PhotocraftApp| {
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::testing::document_area(app, ui));
        },
        app,
    );
    h.run_steps(4);
    let v = &mut h.state_mut().ui.views[0];
    v.zoom = 1.0;
    v.center = [200.0, 150.0];
    v.clear_pending();
    h.run_steps(2);
    h
}

fn control(h: &mut AppHarness, method: &str, params: Value) -> Value {
    let ctx = h.ctx.clone();
    let (request, _reply) = ControlRequest::new(method, params);
    let Reply::Done(reply) = handle(h.state_mut(), &ctx, &request) else { panic!("{method} should reply immediately") };
    assert_eq!(reply["ok"], true, "{method}: {reply}");
    reply["result"].clone()
}

fn bar(h: &mut AppHarness) -> Value {
    control(h, "ui.inspect", json!({}))["taskBar"].clone()
}

fn free_transform(h: &mut AppHarness) {
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "edit.freeTransform", json!({})).unwrap();
    h.run_steps(6);
}

fn drag_grip(h: &mut AppHarness) {
    let from = h.get_by_label("Move bar").rect().center();
    let to = from + egui::vec2(80.0, -40.0);
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    h.event(egui::Event::PointerMoved(from));
    h.run_steps(1);
    h.event(button(from, true));
    h.run_steps(1);
    for step in 1..=6 {
        h.event(egui::Event::PointerMoved(from + (to - from) * (step as f32 / 6.0)));
        h.run_steps(1);
    }
    h.event(button(to, false));
    h.run_steps(3);
}

fn crop_frame(h: &mut AppHarness) {
    control(h, "ui.set", json!({"tool": "crop"}));
    h.run_steps(2);
    h.state_mut().canvas_tools.crop.tool.crop_to(CropBox::new(DocRect::new(100.0, 100.0, 200.0, 160.0), 0.0));
    h.run_steps(6);
}

fn place(h: &AppHarness) -> crate::state::TaskBarPlace {
    h.state().ui.panels.task_bar
}

/// Free Transform with its bar dragged by the grip: the drop is the bar's offset, unpinned.
fn dragged_transform() -> AppHarness {
    let mut h = harness();
    free_transform(&mut h);
    drag_grip(&mut h);
    let p = place(&h);
    assert!(!p.pinned && p.at.is_none() && p.offset.is_some(), "an unpinned drag keeps an offset: {p:?}");
    h
}

fn leave_transform(h: &mut AppHarness) {
    crate::transform_tool::cancel(h.state_mut());
    h.run_steps(4);
    assert!(h.state().ui.transform.is_none());
}

/// Where the crop bar shows with automatic placement for the same frame.
fn automatic_crop_rect() -> Value {
    let mut h = harness();
    crop_frame(&mut h);
    let shown = bar(&mut h);
    assert!(shown["offset"].is_null() && shown["at"].is_null());
    shown["rect"].clone()
}

/// The transform bar's drag is cleared once the crop bar shows: another bar kind is another context.
#[test]
fn a_dragged_bar_goes_back_to_automatic_placement_for_another_bar() {
    let mut h = dragged_transform();
    leave_transform(&mut h);
    crop_frame(&mut h);
    let crop = bar(&mut h);
    assert_eq!(crop["visible"], true);
    assert!(crop["offset"].is_null() && crop["at"].is_null(), "the drag is cleared on the context change: {crop}");
    assert_eq!(crop["rect"], automatic_crop_rect(), "the bar shows under the crop frame again");
}

/// A new transform box is a new context too.
#[test]
fn a_dragged_bar_goes_back_to_automatic_placement_for_a_new_transform() {
    let mut h = dragged_transform();
    leave_transform(&mut h);
    free_transform(&mut h);
    let p = place(&h);
    assert!(p.offset.is_none() && p.at.is_none(), "{p:?}");
}

/// Within one context the drop stays: later passes keep the offset and the bar where it was dropped.
#[test]
fn a_dragged_bar_stays_where_it_was_dropped_in_the_same_context() {
    let mut h = dragged_transform();
    let (kept, rect) = (place(&h), bar(&mut h)["rect"].clone());
    h.run_steps(6);
    assert_eq!(place(&h), kept);
    assert_eq!(bar(&mut h)["rect"], rect);
}

/// A pinned bar keeps its place across contexts.
#[test]
fn a_pinned_bar_keeps_its_place_across_contexts() {
    let mut h = dragged_transform();
    h.get_by_label("More options").click();
    h.run_steps(4);
    h.get_by_label("Pin bar position").click();
    h.run_steps(4);
    let pinned = place(&h);
    assert!(pinned.pinned && pinned.at.is_some(), "{pinned:?}");
    leave_transform(&mut h);
    crop_frame(&mut h);
    assert_eq!(place(&h), pinned, "the pinned place is kept for the crop bar");
    assert_eq!(bar(&mut h)["visible"], true);
}

/// Reset bar position still puts the bar back to automatic placement, unpinned.
#[test]
fn reset_bar_position_is_unchanged() {
    let mut h = dragged_transform();
    h.get_by_label("More options").click();
    h.run_steps(4);
    h.get_by_label("Reset bar position").click();
    h.run_steps(4);
    assert_eq!(place(&h), crate::state::TaskBarPlace::default());
}

/// A crop frame applied or discarded ends the context: the next frame's bar is placed automatically.
#[test]
fn a_new_crop_frame_is_a_new_context() {
    let mut h = harness();
    crop_frame(&mut h);
    assert!(crate::crop_ui::pending(h.state()), "the frame drawn with crop_to is pending");
    drag_grip(&mut h);
    assert!(place(&h).offset.is_some());
    h.run_steps(4);
    assert!(place(&h).offset.is_some(), "the same frame keeps the drop");
    crate::crop_ui::discard(h.state_mut());
    h.run_steps(4);
    crop_frame(&mut h);
    let p = place(&h);
    assert!(p.offset.is_none() && p.at.is_none(), "{p:?}");
    assert_eq!(bar(&mut h)["rect"], automatic_crop_rect());
}

/// A crop discarded while the bar is hidden still ends that frame's context: the next frame's bar is automatic.
#[test]
fn a_crop_discarded_while_the_bar_is_hidden_is_a_new_context() {
    let mut h = harness();
    crop_frame(&mut h);
    drag_grip(&mut h);
    assert!(place(&h).offset.is_some());
    control(&mut h, "ui.set", json!({"panels": {"context_task_bar": false}}));
    h.run_steps(4);
    crate::crop_ui::discard(h.state_mut());
    h.run_steps(4);
    h.state_mut().canvas_tools.crop.tool.crop_to(CropBox::new(DocRect::new(100.0, 100.0, 200.0, 160.0), 0.0));
    h.run_steps(4);
    assert!(crate::crop_ui::pending(h.state()), "a new pending frame while the bar is hidden");
    control(&mut h, "ui.set", json!({"panels": {"context_task_bar": true}}));
    h.run_steps(6);
    let p = place(&h);
    assert!(p.offset.is_none() && p.at.is_none(), "{p:?}");
    assert_eq!(bar(&mut h)["rect"], automatic_crop_rect());
}

/// The Frame tool's bar: another frame layer is another context.
#[test]
fn another_frame_layer_is_a_new_context() {
    let mut h = harness();
    let a = h.state_mut().run("layer.new.frame", json!({"x": 40, "y": 40, "width": 100, "height": 80})).unwrap()["layer"].as_u64().unwrap();
    let b = h.state_mut().run("layer.new.frame", json!({"x": 220, "y": 120, "width": 100, "height": 80})).unwrap()["layer"].as_u64().unwrap();
    h.state_mut().run("layer.select", json!({"layer": a})).unwrap();
    control(&mut h, "ui.set", json!({"tool": "frame"}));
    h.run_steps(6);
    drag_grip(&mut h);
    assert!(place(&h).offset.is_some(), "the frame bar's drag is kept: {:?}", place(&h));
    h.run_steps(4);
    assert!(place(&h).offset.is_some(), "the same frame keeps the drop");
    h.state_mut().run("layer.select", json!({"layer": b})).unwrap();
    h.run_steps(6);
    let p = place(&h);
    assert!(p.offset.is_none() && p.at.is_none(), "{p:?}");
}

/// A pinned bar ignores the context: `enter_task_bar_context` never touches its place, an unpinned one is cleared
/// only when the context differs.
#[test]
fn enter_task_bar_context_clears_only_an_unpinned_place_on_a_change() {
    use crate::state::{TaskBarContext, UiState};
    let mut ui = UiState::default();
    let first = TaskBarContext::Transform { doc: 1, session: 7 };
    ui.enter_task_bar_context(first);
    ui.panels.task_bar.offset = Some([5.0, 6.0]);
    ui.enter_task_bar_context(first);
    assert_eq!(ui.panels.task_bar.offset, Some([5.0, 6.0]), "same context: kept");
    ui.enter_task_bar_context(TaskBarContext::Crop { doc: 1, frame: 0 });
    assert_eq!(ui.panels.task_bar.offset, None, "another context: cleared");
    ui.panels.task_bar.pinned = true;
    ui.panels.task_bar.at = Some([1.0, 2.0]);
    ui.enter_task_bar_context(TaskBarContext::Frame { doc: 1, layer: 3 });
    assert_eq!((ui.panels.task_bar.pinned, ui.panels.task_bar.at), (true, Some([1.0, 2.0])), "pinned: kept");
}
