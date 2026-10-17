//! #3631: hovering a Layers panel row outlines the layer on the canvas; an Auto-Select pick in a
//! closed group opens it; Show Transform Controls is remembered between launches.

use egui::{Rect, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use photocraft_doc::{LayerContent, LayerId};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::ToolEvent;
use crate::dock::{Group, last_rects};
use crate::state::Tool;
use crate::theme::ThemeKind;

/// A painted layer "Inner" ((8..24)²) inside a closed group, a painted "Top" ((40..56)²) above.
fn doc() -> (PhotocraftApp, LayerId, LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    let paint = |app: &mut PhotocraftApp, at: i32| {
        app.session
            .edit("paint", |doc, a| {
                doc.layer_mut(a.unwrap())
                    .unwrap()
                    .surface_mut()
                    .unwrap()
                    .fill_rect(photocraft_geom::Rect::new(at, at, at + 16, at + 16), &[1.0, 0.0, 0.0, 1.0]);
                Ok(())
            })
            .unwrap();
    };
    app.run("layer.new.layer", json!({"name": "Inner"})).unwrap();
    paint(&mut app, 8);
    let inner = app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.groupLayers", json!({})).unwrap();
    let group = app.session.active().unwrap().doc.walk().into_iter().find(|(_, _, l)| l.is_group()).map(|(_, _, l)| l.id).unwrap();
    app.run("layer.setExpanded", json!({"layer": group.0, "expanded": false})).unwrap();
    app.run("layer.new.layer", json!({"name": "Top"})).unwrap();
    paint(&mut app, 40);
    app.sync_views();
    app.ui.tool = Tool::Move;
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    (app, inner, group)
}

fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::panels::right_dock(app, ui);
            egui::CentralPanel::default().show(ui, |_| {});
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, ThemeKind::ProMedium);
    h.state_mut().ui.theme = ThemeKind::ProMedium;
    h.run_steps(4);
    h
}

fn row(h: &Harness<'static, PhotocraftApp>, name: &str) -> Option<Rect> {
    let layers = last_rects(&h.ctx).into_iter().find(|(g, _)| *g == Group::Layers)?.1;
    h.query_all_by_label(name).map(|n| n.rect()).find(|r| layers.contains(r.center()))
}

fn target(h: &Harness<'static, PhotocraftApp>) -> Option<photocraft_geom::Rect> {
    super::target(h.state(), h.ctx.cumulative_pass_nr())
}

#[test]
fn hovering_a_layer_row_outlines_it_with_any_tool() {
    let (app, _, _) = doc();
    let mut h = harness(app);
    let top = row(&h, "Top").expect("row drawn");
    h.hover_at(top.center());
    h.run_steps(2);
    let bounds = Some(photocraft_geom::Rect::new(40, 40, 56, 56));
    assert_eq!(target(&h), bounds, "on by default (Show Highlight on Rollover)");
    // Any tool, not only the Move tool.
    h.state_mut().ui.tool = Tool::Brush;
    h.run_steps(2);
    assert_eq!(target(&h), bounds);
    // Nothing while transforming or with the preference off.
    h.state_mut().session.edit_prefs(|p| p.tools.show_highlight_on_rollover = false);
    h.run_steps(2);
    assert_eq!(target(&h), None, "preference off");
    h.state_mut().session.edit_prefs(|p| p.tools.show_highlight_on_rollover = true);
    h.run_steps(2);
    assert_eq!(target(&h), bounds);
    // The pointer leaves the panel: the outline goes.
    h.hover_at(egui::pos2(300.0, 400.0));
    h.run_steps(3);
    assert_eq!(target(&h), None);
}

#[test]
fn auto_select_into_a_closed_group_opens_it_and_shows_the_row() {
    let (mut app, inner, group) = doc();
    // Main now defaults Auto-Select to Group; this case specifically exercises Layer mode.
    app.ui.tool_options.move_target = "layer".into();
    let mut h = harness(app);
    assert!(row(&h, "Inner").is_none(), "hidden in its closed group");
    let app = h.state_mut();
    crate::canvas::tool_event(app, ToolEvent::Down { x: 12.0, y: 12.0, pressure: 1.0 }, egui::Modifiers::NONE);
    crate::canvas::tool_event(app, ToolEvent::Up { x: 12.0, y: 12.0 }, egui::Modifiers::NONE);
    h.run_steps(4);
    assert_eq!(h.state().session.active().unwrap().active_layer, Some(inner), "Auto-Select picked it");
    assert!(matches!(&h.state().session.active().unwrap().doc.layer(group).unwrap().content, LayerContent::Group(g) if g.expanded));
    assert!(row(&h, "Inner").is_some(), "its row is listed");
}

#[test]
fn show_transform_controls_is_remembered_between_launches() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.ui.tool_options.move_show_transform = true;
    crate::tool_memory::persist(&mut app, &egui::Context::default());
    let saved = app.session.prefs().clone();
    let mut next = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    next.session.edit_prefs(|p| *p = saved);
    crate::tool_memory::restore(&mut next);
    assert!(next.ui.tool_options.move_show_transform);
}

/// #3631: Path Selection › Select: All Layers picks a shape layer inside a closed group on click,
/// opens the group and lists its row, like the Move tool's Auto-Select.
#[test]
fn path_selection_pick_into_a_closed_group_opens_it_and_shows_the_row() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.run("shape.create", json!({"kind": "rect", "rect": [8, 8, 16, 16], "fill": "#ff0000"})).unwrap();
    let inner = app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.setName", json!({"layer": inner.0, "name": "Inner"})).ok();
    app.run("layer.groupLayers", json!({})).unwrap();
    let group = app.session.active().unwrap().doc.walk().into_iter().find(|(_, _, l)| l.is_group()).map(|(_, _, l)| l.id).unwrap();
    app.run("layer.setExpanded", json!({"layer": group.0, "expanded": false})).unwrap();
    app.run("shape.create", json!({"kind": "ellipse", "rect": [40, 40, 16, 16], "fill": "#0000ff"})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::PathSelection;
    app.ui.tool_options.path_select_all = true;
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    let name = app.session.active().unwrap().doc.layer(inner).unwrap().name.clone();
    let mut h = harness(app);
    assert!(row(&h, &name).is_none(), "hidden in its closed group");
    let app = h.state_mut();
    crate::canvas::tool_event(app, ToolEvent::Down { x: 12.0, y: 12.0, pressure: 1.0 }, egui::Modifiers::NONE);
    crate::canvas::tool_event(app, ToolEvent::Up { x: 12.0, y: 12.0 }, egui::Modifiers::NONE);
    h.run_steps(4);
    assert_eq!(h.state().session.active().unwrap().active_layer, Some(inner), "the click picked the shape");
    assert!(matches!(&h.state().session.active().unwrap().doc.layer(group).unwrap().content, LayerContent::Group(g) if g.expanded));
    assert!(row(&h, &name).is_some(), "its row is listed");
}
