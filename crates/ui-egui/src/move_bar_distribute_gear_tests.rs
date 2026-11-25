//! #11381: the Move tool's options bar. The Distribute buttons gated by the layer count, the
//! gear's switches (rollover outlines) and the group expansion on a click.

use craft::layers::ui::panel::Groups;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::LayerId;
use photocraft_geom::Rect;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

fn paint(app: &mut PhotocraftApp, r: Rect) -> LayerId {
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, a| {
            doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(r, &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.session.active().unwrap().active_layer.unwrap()
}

fn bounds(app: &PhotocraftApp, id: LayerId) -> Rect {
    let doc = &app.session.active().unwrap().doc;
    photocraft_compose::bounds::content_bounds(doc.layer(id).unwrap().surface().unwrap())
}

fn select_all(app: &mut PhotocraftApp, ids: &[LayerId]) {
    app.session.execute("layer.select", json!({"layer": ids[0].0})).unwrap();
    for id in &ids[1..] {
        app.session.execute("layer.select", json!({"layer": id.0, "mode": "toggle"})).unwrap();
    }
}

/// A 120x120 document with three layers side by side and on top of each other unevenly, the
/// Move tool current. Returns the app and the layers, bottom to top.
fn app() -> (Box<PhotocraftApp>, [LayerId; 3]) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 120, "height": 120})).unwrap();
    let a = paint(&mut app, Rect::new(0, 0, 10, 10));
    let b = paint(&mut app, Rect::new(12, 50, 32, 54));
    let c = paint(&mut app, Rect::new(70, 60, 100, 80));
    app.sync_views();
    app.ui.tool = Tool::Move;
    (app, [a, b, c])
}

/// The Move tool's align controls and gear, on the app's own context so command states reach them
/// as in the running app.
fn harness(app: Box<PhotocraftApp>) -> Harness<'static, Box<PhotocraftApp>> {
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(700.0, 200.0)).build_ui_state(
        |ui, app: &mut Box<PhotocraftApp>| {
            egui::CentralPanel::default().show(ui, |ui| {
                ui.horizontal(|ui| crate::align_bar::show(app, ui));
            });
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
    steps(&mut h, 4);
    h
}

/// `n` frames, each with the command states published before its input, as the app's frame does.
fn steps(h: &mut Harness<'static, Box<PhotocraftApp>>, n: usize) {
    for _ in 0..n {
        let ctx = h.ctx.clone();
        h.state_mut().framework_before_input(&ctx);
        h.step();
    }
}

#[test]
fn distribute_buttons_are_greyed_with_too_few_layers_like_the_menu() {
    let (mut app, [a, b, c]) = app();
    select_all(&mut app, &[a, b]);
    let mut h = harness(app);
    for label in ["Distribute Vertically", "Distribute Horizontally"] {
        assert!(h.get_by_label(label).accesskit_node().is_disabled(), "{label} with two layers");
    }
    // Align still works with two layers.
    assert!(!h.get_by_label("Align Left Edges").accesskit_node().is_disabled());
    select_all(h.state_mut(), &[a, b, c]);
    steps(&mut h, 2);
    assert!(!h.get_by_label("Distribute Vertically").accesskit_node().is_disabled());
}

#[test]
fn show_layer_bounds_on_hover_switches_the_canvas_outline() {
    let (mut app, [_, b, _]) = app();
    let b_bounds = bounds(&app, b);
    assert_eq!(crate::move_hover::target(&mut app, Some([14.0, 52.0]), false, false), Some(b_bounds));
    app.ui.tool_options.move_show_hover_bounds = false;
    assert_eq!(crate::move_hover::target(&mut app, Some([14.0, 52.0]), false, false), None);
    assert_eq!(app.canvas_hover_layer, None);
    app.ui.tool_options.move_show_hover_bounds = true;
    assert_eq!(crate::move_hover::target(&mut app, Some([14.0, 52.0]), false, false), Some(b_bounds));
}

#[test]
fn show_hover_bounds_from_layers_switches_the_layers_panel_outline() {
    let (mut app, [a, _, _]) = app();
    app.hover_layer = Some(a);
    let a_bounds = bounds(&app, a);
    assert_eq!(crate::move_hover::active_outline(&mut app, None, false, false), Some(a_bounds));
    app.ui.tool_options.move_show_layers_hover_bounds = false;
    assert_eq!(crate::move_hover::active_outline(&mut app, None, false, false), None);
    // The canvas outline is a separate switch.
    assert!(crate::move_hover::active_outline(&mut app, Some([14.0, 52.0]), false, false).is_some());
    // Preferences > Show Highlight on Rollover stays the master switch.
    app.ui.tool_options.move_show_layers_hover_bounds = true;
    app.session.edit_prefs(|p| p.tools.show_highlight_on_rollover = false);
    assert_eq!(crate::move_hover::active_outline(&mut app, None, false, false), None);
}

/// The reveal the Layers panel's next draw takes.
fn track(app: &mut PhotocraftApp) -> Option<(LayerId, Groups)> {
    crate::layer_reveal::take(app)
}

/// Selects `layer` the way a command does: the reveal is filed around it.
fn select(app: &mut PhotocraftApp, layer: LayerId) {
    let before = crate::layer_reveal::mark(app);
    app.run("layer.select", json!({"layer": layer.0})).unwrap();
    crate::layer_reveal::file(app, before);
}

#[test]
fn expand_layer_group_on_click_switches_the_opening_of_closed_parent_groups() {
    for on in [true, false] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
        let inner = paint(&mut app, Rect::new(8, 8, 24, 24));
        app.session.execute("layer.groupLayers", json!({})).unwrap();
        let group = app.session.active().unwrap().doc.walk().into_iter().find(|(_, _, l)| l.is_group()).map(|(_, _, l)| l.id).unwrap();
        app.session.execute("layer.setExpanded", json!({"layer": group.0, "expanded": false})).unwrap();
        app.session.execute("layer.select", json!({"layer": group.0})).unwrap();
        app.sync_views();
        app.ui.tool = Tool::Move;
        app.ui.tool_options.move_expand_group_on_click = on;
        // Nothing is filed yet; then a canvas click picks the inner one.
        assert_eq!(track(&mut app), None);
        // The Move tool picks the layer under the pointer (Auto-Select targets layers, not groups).
        app.ui.tool_options.move_target = "layer".into();
        crate::canvas::tool_event(&mut app, crate::canvas::ToolEvent::Down { x: 12.0, y: 12.0, pressure: 1.0 }, egui::Modifiers::default());
        crate::canvas::tool_event(&mut app, crate::canvas::ToolEvent::Up { x: 12.0, y: 12.0 }, egui::Modifiers::default());
        // The row is still revealed (scrolled to) either way.
        assert_eq!(track(&mut app), Some((inner, if on { Groups::Open } else { Groups::Keep })), "Expand layer group on click = {on}");
    }
}

/// A 64x64 document: `inner` painted at (8..24)^2 inside a closed group, the Background-less
/// group selected, `tool` current and the gear option set to `on`. The reveal has seen the frame.
fn closed_group_doc(tool: Tool, on: bool) -> (Box<PhotocraftApp>, LayerId, LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    let inner = paint(&mut app, Rect::new(8, 8, 24, 24));
    app.session.execute("layer.groupLayers", json!({})).unwrap();
    let group = app.session.active().unwrap().doc.walk().into_iter().find(|(_, _, l)| l.is_group()).map(|(_, _, l)| l.id).unwrap();
    app.session.execute("layer.setExpanded", json!({"layer": group.0, "expanded": false})).unwrap();
    app.session.execute("layer.select", json!({"layer": group.0})).unwrap();
    app.sync_views();
    app.ui.tool = tool;
    app.ui.tool_options.move_expand_group_on_click = on;
    app.ui.tool_options.move_target = "layer".into();
    assert_eq!(track(&mut app), None);
    (app, inner, group)
}

/// A press and release at (12, 12) with Ctrl/Cmd held (a temporary Move from another tool).
fn ctrl_click(app: &mut PhotocraftApp) {
    use crate::canvas::{ToolEvent, tool_event};
    let mods = egui::Modifiers { command: true, ctrl: true, ..Default::default() };
    tool_event(app, ToolEvent::Down { x: 12.0, y: 12.0, pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x: 12.0, y: 12.0 }, mods);
}

#[test]
fn expand_layer_group_on_click_off_keeps_the_group_closed_for_a_ctrl_click_from_another_tool() {
    // Auto-Select off, so the Ctrl-click is the Move way of picking from the Brush.
    let (mut app, inner, _) = closed_group_doc(Tool::Brush, false);
    app.ui.tool_options.move_auto_select = false;
    ctrl_click(&mut app);
    assert_eq!(app.session.active().unwrap().active_layer, Some(inner), "the Ctrl-click picked the layer");
    assert_eq!(track(&mut app), Some((inner, Groups::Keep)), "the group stays closed");
}

#[test]
fn expand_layer_group_on_click_on_opens_the_group_for_a_ctrl_click_from_another_tool() {
    let (mut app, inner, _) = closed_group_doc(Tool::Brush, true);
    app.ui.tool_options.move_auto_select = false;
    ctrl_click(&mut app);
    assert_eq!(app.session.active().unwrap().active_layer, Some(inner));
    assert_eq!(track(&mut app), Some((inner, Groups::Open)), "the group opens");
}

#[test]
fn expand_layer_group_on_click_off_keeps_the_group_closed_for_a_move_tool_click() {
    let (mut app, inner, _) = closed_group_doc(Tool::Move, false);
    use crate::canvas::{ToolEvent, tool_event};
    tool_event(&mut app, ToolEvent::Down { x: 12.0, y: 12.0, pressure: 1.0 }, egui::Modifiers::default());
    tool_event(&mut app, ToolEvent::Up { x: 12.0, y: 12.0 }, egui::Modifiers::default());
    assert_eq!(app.session.active().unwrap().active_layer, Some(inner));
    assert_eq!(track(&mut app), Some((inner, Groups::Keep)));
}

#[test]
fn expand_layer_group_on_click_off_still_opens_the_group_for_other_active_layer_changes() {
    let (mut app, inner, _) = closed_group_doc(Tool::Brush, false);
    select(&mut app, inner);
    assert_eq!(track(&mut app), Some((inner, Groups::Open)), "a command that changes the active layer reveals it");
}

#[test]
fn expand_layer_group_on_click_governs_a_ctrl_click_pick_of_a_position_locked_layer() {
    for on in [false, true] {
        let (mut app, inner, _) = closed_group_doc(Tool::Brush, on);
        app.ui.tool_options.move_auto_select = false;
        app.session.execute("layer.setProps", json!({"layer": inner.0, "locks": {"position": true}})).unwrap();
        ctrl_click(&mut app);
        assert_eq!(app.session.active().unwrap().active_layer, Some(inner), "the Ctrl-click picked the locked layer (on = {on})");
        assert_eq!(track(&mut app), Some((inner, if on { Groups::Open } else { Groups::Keep })), "Expand layer group on click = {on}");
    }
}

#[test]
fn a_pick_marker_the_hidden_layers_panel_never_read_does_not_hide_a_later_command_change() {
    let (mut app, inner, group) = closed_group_doc(Tool::Brush, false);
    app.ui.tool_options.move_auto_select = false;
    // A Move-way pick of the inner layer while the Layers panel draws nothing (no `track`).
    ctrl_click(&mut app);
    assert_eq!(app.session.active().unwrap().active_layer, Some(inner));
    // Frames pass without the panel, then commands change the layer twice and land on it again.
    app.frame += 5;
    select(&mut app, group);
    select(&mut app, inner);
    // The panel is shown again: the change was a command's, so the group opens.
    app.frame += 1;
    assert_eq!(track(&mut app), Some((inner, Groups::Open)), "a command change after a stale pick expands the group");
}

#[test]
fn a_pick_marker_survives_any_number_of_frames_while_the_layers_panel_is_hidden() {
    let (mut app, inner, _) = closed_group_doc(Tool::Brush, false);
    app.ui.tool_options.move_auto_select = false;
    ctrl_click(&mut app);
    app.frame += 5;
    assert_eq!(track(&mut app), Some((inner, Groups::Keep)), "the pick is still the latest change of the active layer");
}

#[test]
fn commands_after_a_pick_in_the_same_frame_or_the_next_make_it_a_command_change() {
    for gap in [0, 1] {
        let (mut app, inner, group) = closed_group_doc(Tool::Brush, false);
        app.ui.tool_options.move_auto_select = false;
        ctrl_click(&mut app);
        app.frame += gap;
        select(&mut app, group);
        select(&mut app, inner);
        assert_eq!(track(&mut app), Some((inner, Groups::Open)), "selects {gap} frame(s) after the pick reveal the layer");
    }
}

#[test]
fn a_pick_marker_is_read_once_and_governs_the_next_change_only() {
    let (mut app, inner, group) = closed_group_doc(Tool::Brush, false);
    app.ui.tool_options.move_auto_select = false;
    ctrl_click(&mut app);
    assert_eq!(track(&mut app), Some((inner, Groups::Keep)));
    select(&mut app, group);
    assert_eq!(track(&mut app), Some((group, Groups::Open)));
    select(&mut app, inner);
    assert_eq!(track(&mut app), Some((inner, Groups::Open)), "a later command change opens the group");
}
