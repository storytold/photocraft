//! #10884: Select › Find Layers opens the Layers panel's Name filter, against the cases measured in
//! Photoshop.

use egui::vec2;
use egui_kittest::Harness;
use photocraft_doc::LayerId;
use serde_json::json;

use super::{Filter, FilterBy};
use crate::PhotocraftApp;

/// "LinkA", "Other" and "LinkB" over the Background, "Other" selected.
fn harness() -> Harness<'static, PhotocraftApp> {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    for name in ["LinkA", "Other", "LinkB"] {
        s.execute("layer.new.layer", json!({"name": name})).unwrap();
    }
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(s, crate::Services::default())
    });
    h.run_steps(8);
    let other = id(&h, "Other");
    h.state_mut().session.execute("layer.select", json!({"layer": other.0})).unwrap();
    h.run_steps(4);
    h
}

fn id(h: &Harness<'_, PhotocraftApp>, name: &str) -> LayerId {
    h.state()
        .session
        .active()
        .unwrap()
        .doc
        .walk()
        .into_iter()
        .find(|(_, _, l)| l.name == name)
        .map(|(_, _, l)| l.id)
        .unwrap_or_else(|| panic!("no layer {name}"))
}

fn selected(h: &Harness<'_, PhotocraftApp>) -> Vec<LayerId> {
    h.state().session.active().unwrap().selected_layers()
}

/// The names the panel's filter lists, top to bottom.
fn listed(h: &Harness<'_, PhotocraftApp>) -> Vec<String> {
    let f = Filter::of_app(h.state());
    let doc = &h.state().session.active().unwrap().doc;
    crate::layer_tree_ui::display_rows(doc, f.active()).into_iter().filter(|(_, l)| f.shows(l)).map(|(_, l)| l.name.clone()).collect()
}

fn find(h: &mut Harness<'_, PhotocraftApp>, params: serde_json::Value) -> serde_json::Value {
    let ctx = h.ctx.clone();
    let reply = crate::menus::invoke(h.state_mut(), &ctx, "select.findLayers", params).unwrap();
    h.run_steps(4);
    reply
}

#[test]
fn the_menu_item_shows_the_layers_panel_and_focuses_the_name_field() {
    let mut h = harness();
    h.state_mut().ui.panels.layers = false;
    h.state_mut().ui.layer_filter_name = "old".into();
    h.run_steps(4);
    let before = selected(&h);
    find(&mut h, json!({}));
    assert!(h.state().ui.panels.layers, "the Layers panel is shown");
    assert_eq!(h.state().ui.layer_filter_by, FilterBy::Name);
    assert!(h.ctx.memory(|m| m.focused()).is_some(), "the caret is in the Name field");
    // The existing text is selected, so typing replaces it.
    h.event(egui::Event::Text("lin".into()));
    h.run_steps(4);
    assert_eq!(h.state().ui.layer_filter_name, "lin");
    assert_eq!(listed(&h), ["LinkB", "LinkA"]);
    assert_eq!(selected(&h), before, "Find Layers does not change the layer selection");
}

#[test]
fn a_name_param_sets_the_filter_text_without_changing_the_selection() {
    let mut h = harness();
    let before = selected(&h);
    let reply = find(&mut h, json!({"name": "link"}));
    assert_eq!((h.state().ui.layer_filter_by, h.state().ui.layer_filter_name.as_str()), (FilterBy::Name, "link"));
    assert_eq!(reply["selected"], json!([id(&h, "LinkA").0, id(&h, "LinkB").0]), "the reply lists the matching layers");
    assert_eq!(listed(&h), ["LinkB", "LinkA"]);
    assert_eq!(selected(&h), before);
    assert!(h.ctx.memory(|m| m.focused()).is_none(), "a script does not take the keyboard focus");
}

#[test]
fn the_shortcut_runs_find_layers() {
    let mut h = harness();
    h.key_press_modifiers(egui::Modifiers::COMMAND | egui::Modifiers::ALT | egui::Modifiers::SHIFT, egui::Key::F);
    h.run_steps(4);
    assert_eq!(h.state().ui.layer_filter_by, FilterBy::Name);
    assert!(h.ctx.memory(|m| m.focused()).is_some(), "the caret is in the Name field");
}

#[test]
fn clicking_the_menu_item_leaves_the_caret_in_the_name_field() {
    use egui_kittest::kittest::Queryable;
    let mut h = harness();
    h.get_by_label("Select").click();
    h.run_steps(4);
    h.get_by_label("Find Layers Ctrl+Alt+Shift+F").click();
    h.run_steps(6);
    assert_eq!(h.state().ui.layer_filter_by, FilterBy::Name);
    h.event(egui::Event::Text("lin".into()));
    h.run_steps(4);
    assert_eq!(h.state().ui.layer_filter_name, "lin", "typing goes to the Name field");
}

#[test]
fn the_shortcut_shows_a_layers_tab_closed_from_the_window_menu() {
    let mut h = harness();
    use egui_kittest::kittest::Queryable;
    h.get_by_label("Window").click();
    h.run_steps(4);
    h.get_by_label("✔ Layers F7").click();
    h.run_steps(4);
    assert!(!h.state().ui.panels.layers, "Window › Layers closed the panel");
    h.state_mut().ui.layer_filter_name = "lin".into();
    h.key_press_modifiers(egui::Modifiers::COMMAND | egui::Modifiers::ALT | egui::Modifiers::SHIFT, egui::Key::F);
    h.run_steps(6);
    assert_eq!(h.state().ui.layer_filter_by, FilterBy::Name);
    h.event(egui::Event::Text("other".into()));
    h.run_steps(4);
    assert_eq!(h.state().ui.layer_filter_name, "other", "the panel is back, focused, with its text selected");
}
