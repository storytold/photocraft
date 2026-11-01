//! #5048: the Layers panel's filter-type dropdown (Kind, Name, Mode, Smart Object).

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_color::BlendMode;
use photocraft_doc::Document;
use serde_json::json;

use super::{Filter, FilterBy};
use crate::PhotocraftApp;
use crate::state::UiState;

/// Background, "Ink" (Multiply) and "Paper" in a group "Sheets", and "ink notes" (type) on top.
fn session() -> photocraft_engine::Session {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Paper"})).unwrap();
    s.execute("layer.groupLayers", json!({"name": "Sheets"})).unwrap();
    let paper = layer(&s.active().unwrap().doc, "Paper");
    s.execute("layer.select", json!({"layer": paper})).unwrap();
    let ink = s.execute("layer.new.layer", json!({"name": "Ink"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.setProps", json!({"layer": ink, "blend": "Multiply"})).unwrap();
    let sheets = layer(&s.active().unwrap().doc, "Sheets");
    s.execute("layer.select", json!({"layer": sheets})).unwrap();
    let text = s.execute("type.create", json!({"text": "ink notes", "x": 4, "y": 20, "size": 12})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.setProps", json!({"layer": text, "name": "ink notes"})).unwrap();
    // Opened last: an open active group would take the type layer inside it, as in Photoshop.
    s.execute("layer.setExpanded", json!({"all": true, "expanded": true})).unwrap();
    s
}

fn layer(doc: &Document, name: &str) -> u64 {
    doc.walk().into_iter().find(|(_, _, l)| l.name == name).map(|(_, _, l)| l.id.0).unwrap_or_else(|| panic!("no layer {name}"))
}

/// The names `filter` lists, top to bottom.
fn listed(doc: &Document, filter: &Filter) -> Vec<String> {
    crate::layer_tree_ui::display_rows(doc, filter.active()).into_iter().filter(|(_, l)| filter.shows(l)).map(|(_, l)| l.name.clone()).collect()
}

fn filter(by: FilterBy, kinds: &[&str], name: &str, mode: BlendMode) -> Filter {
    Filter::of(&UiState {
        layer_filter_by: by,
        layer_filter: kinds.iter().map(|k| k.to_string()).collect(),
        layer_filter_name: name.into(),
        layer_filter_mode: mode,
        ..UiState::default()
    })
}

#[test]
fn each_filter_type_lists_its_matches() {
    let s = session();
    let doc = &s.active().unwrap().doc;
    let all = listed(doc, &Filter::of(&UiState::default()));
    assert_eq!(all, ["ink notes", "Sheets", "Ink", "Paper", "Background"]);
    // Kind: groups match no kind, as before.
    assert_eq!(listed(doc, &filter(FilterBy::Kind, &["pixel"], "", BlendMode::Normal)), ["Ink", "Paper", "Background"]);
    assert_eq!(listed(doc, &filter(FilterBy::Kind, &["type"], "", BlendMode::Normal)), ["ink notes"]);
    // Name: part of the name, ignoring case and surrounding spaces; groups match too.
    assert_eq!(listed(doc, &filter(FilterBy::Name, &[], " INK ", BlendMode::Normal)), ["ink notes", "Ink"]);
    assert_eq!(listed(doc, &filter(FilterBy::Name, &[], "sheet", BlendMode::Normal)), ["Sheets"]);
    assert_eq!(listed(doc, &filter(FilterBy::Name, &[], "nothing like it", BlendMode::Normal)), Vec::<String>::new());
    // Mode: one blend mode; a group's own Pass Through counts.
    assert_eq!(listed(doc, &filter(FilterBy::Mode, &[], "", BlendMode::Multiply)), ["Ink"]);
    assert_eq!(listed(doc, &filter(FilterBy::Mode, &[], "", BlendMode::PassThrough)), ["Sheets"]);
    assert_eq!(listed(doc, &filter(FilterBy::Mode, &[], "", BlendMode::Normal)), ["ink notes", "Paper", "Background"]);
}

#[test]
fn only_a_criterion_turns_kind_and_name_on_while_mode_always_filters() {
    let kinds_left_on = filter(FilterBy::Name, &["pixel"], "", BlendMode::Multiply);
    assert!(!kinds_left_on.active(), "the other types' criteria don't apply");
    assert!(!filter(FilterBy::Name, &[], "   ", BlendMode::Normal).active(), "spaces are no criterion");
    assert!(!filter(FilterBy::Kind, &[], "ink", BlendMode::Normal).active());
    assert!(filter(FilterBy::Mode, &[], "", BlendMode::Normal).active(), "Mode filters by Normal at first, like Photoshop");
}

#[test]
fn saved_settings_without_the_new_fields_still_load() {
    let mut saved = serde_json::to_value(UiState::default()).unwrap();
    let fields = saved.as_object_mut().unwrap();
    for key in ["layer_filter_by", "layer_filter_name", "layer_filter_mode"] {
        assert!(fields.remove(key).is_some(), "{key} is saved");
    }
    let ui: UiState = serde_json::from_value(saved).unwrap();
    assert_eq!((ui.layer_filter_by, ui.layer_filter_name.as_str(), ui.layer_filter_mode), (FilterBy::Kind, "", BlendMode::Normal));
    let round = serde_json::to_value(UiState { layer_filter_by: FilterBy::Mode, ..UiState::default() }).unwrap();
    assert_eq!(round["layer_filter_by"], "mode");
}

fn harness() -> Harness<'static, PhotocraftApp> {
    let session = session();
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(session, crate::Services::default())
    });
    h.run_steps(8);
    h
}

/// The names of the rows the Layers panel drew, top to bottom.
fn drawn(h: &Harness<'_, PhotocraftApp>) -> Vec<String> {
    let doc = &h.state().session.active().unwrap().doc;
    let mut rows = crate::layer_row_ui::recorded(&h.ctx);
    rows.sort_by(|a, b| a.row.top().total_cmp(&b.row.top()));
    rows.iter().filter_map(|r| doc.walk().into_iter().find(|(_, _, l)| l.id.0 == r.layer).map(|(_, _, l)| l.name.clone())).collect()
}

#[test]
fn the_dropdown_switches_the_panel_between_filter_types() {
    let mut h = harness();
    assert_eq!(drawn(&h), ["ink notes", "Sheets", "Ink", "Paper", "Background"]);
    // The type is a real dropdown now: open it and pick Name.
    // A dropdown shows its choice as the combo box's value.
    h.get_by_value("Kind").click();
    h.run_steps(4);
    h.query_all_by_label("Name").last().expect("Name in the open list").click();
    h.run_steps(4);
    assert_eq!(h.state().ui.layer_filter_by, FilterBy::Name);
    assert_eq!(drawn(&h), ["ink notes", "Sheets", "Ink", "Paper", "Background"], "no text yet: every layer");
    h.state_mut().ui.layer_filter_name = "ink".into();
    h.run_steps(4);
    assert_eq!(drawn(&h), ["ink notes", "Ink"]);
    // Mode starts at Normal; the kind toggles left on under Kind don't leak into it.
    h.state_mut().ui.layer_filter = vec!["type".into()];
    h.state_mut().ui.layer_filter_by = FilterBy::Mode;
    h.run_steps(4);
    assert_eq!(drawn(&h), ["ink notes", "Paper", "Background"]);
    h.state_mut().ui.layer_filter_mode = BlendMode::Multiply;
    h.run_steps(4);
    assert_eq!(drawn(&h), ["Ink"]);
    // Back to Kind: its own criterion applies again.
    h.state_mut().ui.layer_filter_by = FilterBy::Kind;
    h.run_steps(4);
    assert_eq!(drawn(&h), ["ink notes"]);
}

/// Creates a PNG fixture and returns its temporary directory with the file path.
fn art_file() -> (tempfile::TempDir, std::path::PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("art.png");
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8, "background": "#ff0000"})).unwrap();
    s.execute("file.saveACopy", json!({"path": file.to_string_lossy(), "layers": false})).unwrap();
    (directory, file)
}

/// "Linked art" (linked to `file`) above "Embedded art" (a copy of `file`), "Plain" and
/// "Background".
fn smart_session(file: &std::path::Path) -> photocraft_engine::Session {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Plain"})).unwrap();
    for (command, name) in [("file.placeEmbedded", "Embedded art"), ("file.placeLinked", "Linked art")] {
        s.execute(command, json!({"path": file.to_string_lossy()})).unwrap();
        let id = s.active().unwrap().active_layer.unwrap().0;
        s.execute("layer.setProps", json!({"layer": id, "name": name})).unwrap();
    }
    s
}

fn by_smart(smart: &[&str], missing: std::collections::HashSet<String>) -> Filter {
    Filter { smart: smart.iter().map(|k| k.to_string()).collect(), missing, ..filter(FilterBy::SmartObject, &[], "", BlendMode::Normal) }
}

#[test]
fn the_smart_object_filter_lists_linked_missing_and_embedded_smart_objects() {
    let (_directory, file) = art_file();
    let mut s = smart_session(&file);
    let doc = &s.active().unwrap().doc;
    let none = std::collections::HashSet::new;
    assert!(!by_smart(&[], none()).active(), "no toggle on: every layer");
    assert_eq!(listed(doc, &by_smart(&[], none())), ["Linked art", "Embedded art", "Plain", "Background"]);
    assert_eq!(listed(doc, &by_smart(&["linked"], none())), ["Linked art"]);
    assert_eq!(listed(doc, &by_smart(&["embedded"], none())), ["Embedded art"]);
    assert_eq!(listed(doc, &by_smart(&["linked", "embedded"], none())), ["Linked art", "Embedded art"]);
    assert_eq!(listed(doc, &by_smart(&["missing"], s.missing_links())), Vec::<String>::new(), "the file is there");
    std::fs::remove_file(&file).unwrap();
    s.execute("layer.smartObjects.updateChanged", json!({})).unwrap();
    let doc = &s.active().unwrap().doc;
    assert_eq!(listed(doc, &by_smart(&["missing"], s.missing_links())), ["Linked art"]);
    assert_eq!(listed(doc, &by_smart(&["missing", "embedded"], s.missing_links())), ["Linked art", "Embedded art"]);
}

#[test]
fn saved_settings_without_the_smart_object_toggles_still_load() {
    let mut saved = serde_json::to_value(UiState::default()).unwrap();
    assert!(saved.as_object_mut().unwrap().remove("layer_filter_smart").is_some(), "layer_filter_smart is saved");
    let ui: UiState = serde_json::from_value(saved).unwrap();
    assert!(ui.layer_filter_smart.is_empty());
    let round =
        serde_json::to_value(UiState { layer_filter_by: FilterBy::SmartObject, layer_filter_smart: vec!["missing".into()], ..UiState::default() }).unwrap();
    let back: UiState = serde_json::from_value(round.clone()).unwrap();
    assert_eq!(
        (round["layer_filter_by"].as_str(), back.layer_filter_by, back.layer_filter_smart),
        (Some("smartObject"), FilterBy::SmartObject, vec!["missing".to_string()])
    );
}

#[test]
fn the_smart_object_toggles_filter_the_panel() {
    let (_directory, file) = art_file();
    let session = smart_session(&file);
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(session, crate::Services::default())
    });
    h.run_steps(8);
    let all = ["Linked art", "Embedded art", "Plain", "Background"];
    assert_eq!(drawn(&h), all);
    h.get_by_value("Kind").click();
    h.run_steps(4);
    h.query_all_by_label("Smart Object").last().expect("Smart Object in the open list").click();
    h.run_steps(4);
    assert_eq!(h.state().ui.layer_filter_by, FilterBy::SmartObject);
    assert_eq!(drawn(&h), all, "no toggle on yet: every layer");
    h.get_by_label("Filter for embedded smart objects").click();
    h.run_steps(4);
    assert_eq!(h.state().ui.layer_filter_smart, ["embedded"]);
    assert_eq!(drawn(&h), ["Embedded art"]);
    h.get_by_label("Filter for embedded smart objects").click();
    h.get_by_label("Filter for smart objects linked locally").click();
    h.run_steps(4);
    assert_eq!(drawn(&h), ["Linked art"]);
    h.get_by_label("Filter for smart objects linked locally").click();
    h.get_by_label("Filter for linked smart objects that are missing").click();
    h.run_steps(4);
    assert_eq!(h.state().ui.layer_filter_smart, ["missing"]);
    assert_eq!(drawn(&h), Vec::<String>::new(), "the file is there");
    std::fs::remove_file(&file).unwrap();
    h.state_mut().session.execute("layer.smartObjects.updateChanged", json!({})).unwrap();
    h.run_steps(4);
    assert_eq!(drawn(&h), ["Linked art"]);
}
