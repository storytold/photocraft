//! #12008: while a Gradient fill layer is the active layer, the Gradient tool's Gradient / Classic
//! gradient dropdown is greyed and reads "Gradient", and a drag edits that fill layer even when
//! Classic gradient was chosen. Selecting a pixel layer brings the stored choice back.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::{Fill, LayerContent, LayerId};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::ToolEvent;
use crate::state::Tool;

/// A document with a pixel layer below a Gradient fill layer; returns (app, pixel layer, fill layer).
fn doc() -> (PhotocraftApp, LayerId, LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let pixels = LayerId(app.run("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap());
    let fill = LayerId(app.run("layer.newFillLayer.gradient", json!({"from": "#235b49", "to": "#a7d4ab", "angle": 90})).unwrap()["layer"].as_u64().unwrap());
    app.sync_views();
    app.ui.tool = Tool::Gradient;
    (app, pixels, fill)
}

fn select(app: &mut PhotocraftApp, id: LayerId) {
    app.run("layer.select", json!({"layer": id.0})).unwrap();
    assert_eq!(app.session.active().unwrap().active_layer, Some(id));
}

fn bar(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let ctx = app.ctx().clone();
    let mut h = Harness::builder().with_size(vec2(1600.0, 300.0)).with_context(ctx).build_ui_state(|ui, app| crate::panels::options_bar(app, ui), app);
    h.run_steps(4);
    h
}

/// The mode dropdown, found by its role and options rather than its label (the blend Mode label
/// reads "Mode:" or "Mode" by theme): (greyed, the option it shows).
fn dropdown(h: &Harness<'static, PhotocraftApp>) -> (bool, String) {
    let modes = ["Gradient", "Classic gradient"];
    let mut combos = h.query_all_by_role(Role::ComboBox).filter(|n| n.accesskit_node().value().is_some_and(|v| modes.contains(&v.as_str())));
    let node = combos.next().expect("the mode dropdown");
    assert!(combos.next().is_none(), "one mode dropdown");
    let node = node.accesskit_node();
    (node.is_disabled(), node.value().unwrap_or_default())
}

fn gradient_of(app: &PhotocraftApp, id: LayerId) -> Fill {
    match &app.session.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Fill(f @ Fill::Gradient { .. }) => f.clone(),
        other => panic!("{}", other.kind_name()),
    }
}

/// On a fill layer the Mode choice is disabled and reads Gradient while the stored choice is
/// kept; a pixel layer enables it with the stored choice.
#[test]
fn a_gradient_fill_layer_greys_the_choice_and_a_pixel_layer_brings_it_back() {
    for (classic, stored) in [(false, "Gradient"), (true, "Classic gradient")] {
        let (mut app, pixels, _) = doc();
        app.ui.tool_options.gradient_classic = classic;
        let mut h = bar(app);
        assert_eq!(dropdown(&h), (true, "Gradient".into()), "fill layer, Classic chosen: {classic}");
        assert_eq!(h.state().ui.tool_options.gradient_classic, classic, "the stored choice is kept");
        select(h.state_mut(), pixels);
        h.run_steps(2);
        assert_eq!(dropdown(&h), (false, stored.into()), "pixel layer, Classic chosen: {classic}");
        assert_eq!(h.state().ui.tool_options.gradient_classic, classic);
    }
}

/// With Classic gradient chosen, a drag on the canvas edits the active fill layer's gradient instead
/// of painting pixels.
#[test]
fn a_drag_edits_the_fill_layer_with_classic_chosen() {
    let (mut app, pixels, fill) = doc();
    app.ui.tool_options.gradient_classic = true;
    assert!(crate::gradient_ui::live_mode(&app), "live on the fill layer");
    let before = gradient_of(&app, fill);
    let steps = app.session.active().unwrap().history.entries().len();
    // Through the canvas's own input path, as a mouse drag arrives.
    let none = egui::Modifiers::NONE;
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 100.0, pressure: 1.0 }, none);
    crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 150.0, y: 120.0, pressure: 1.0 }, none);
    crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 280.0, y: 140.0 }, none);
    assert!(!app.dialog_open(), "no rasterize prompt");
    assert_ne!(gradient_of(&app, fill), before, "the fill layer's gradient changed");
    assert_eq!(app.session.active().unwrap().history.entries().len(), steps + 1, "one history step");
    // A pixel layer goes back to the stored Classic choice.
    select(&mut app, pixels);
    assert!(!crate::gradient_ui::live_mode(&app));
}

/// With Classic chosen the fill layer is edited by the live bar's options: Reverse still edits
/// the fill layer (#12007).
#[test]
fn the_fill_layer_is_edited_by_the_bar_options_with_classic_chosen() {
    let (mut app, _, fill) = doc();
    app.ui.tool_options.gradient_classic = true;
    let before = app.ui.tool_options.clone();
    app.ui.tool_options.gradient_reverse = !before.gradient_reverse;
    crate::gradient_ui::options_changed(&mut app, &before);
    assert!(matches!(gradient_of(&app, fill), Fill::Gradient { reverse, .. } if reverse != before.gradient_reverse), "Reverse");
}

/// With Classic chosen, the targets that keep the stored choice (the Quick Mask, which the tool
/// paints instead of the layer; a hidden, locked or hidden-group fill layer, none of them shown
/// or editable) and the one that greys it (a fill layer in a visible group).
#[test]
fn only_a_shown_editable_fill_layer_greys_the_choice() {
    for (target, greyed) in [("quick mask", false), ("hidden", false), ("locked", false), ("hidden group", false), ("visible group", true)] {
        let (mut app, _, fill) = doc();
        app.ui.tool_options.gradient_classic = true;
        match target {
            "quick mask" => {
                app.run("select.editInQuickMaskMode", json!({})).unwrap();
            }
            "hidden" => {
                app.run("layer.setProps", json!({"layer": fill.0, "visible": false})).unwrap();
            }
            "locked" => {
                app.run("layer.setProps", json!({"layer": fill.0, "locks": {"all": true}})).unwrap();
            }
            _ => {
                app.run("layer.groupLayers", json!({})).unwrap();
                let group = app.session.active().unwrap().active_layer.unwrap();
                app.run("layer.setProps", json!({"layer": group.0, "visible": target == "visible group"})).unwrap();
                select(&mut app, fill);
            }
        }
        assert_eq!(crate::gradient_ui::fill_layer_active(&app), greyed, "{target}: fill_layer_active");
        assert_eq!(crate::gradient_ui::live_mode(&app), greyed, "{target}: live_mode with Classic chosen");
        assert!(app.ui.tool_options.gradient_classic, "{target}: the stored choice is kept");
    }
}
