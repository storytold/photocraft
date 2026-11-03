//! #10883: the Layers panel's Group filter.

use photocraft_doc::Document;
use serde_json::json;

use super::{Filter, FilterBy};
use crate::state::UiState;

fn group_filter() -> Filter {
    Filter::of(&UiState { layer_filter_by: FilterBy::Group, ..UiState::default() })
}

fn new_session() -> photocraft_engine::Session {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    s
}

fn id(doc: &Document, name: &str) -> u64 {
    doc.walk().into_iter().find(|(_, _, l)| l.name == name).map(|(_, _, l)| l.id.0).unwrap_or_else(|| panic!("no layer {name}"))
}

fn select(s: &mut photocraft_engine::Session, name: &str) {
    let layer = id(&s.active().unwrap().doc, name);
    s.execute("layer.select", json!({"layer": layer})).unwrap();
}

/// A new layer above the one selected (inside its group, if it is in one).
fn add(s: &mut photocraft_engine::Session, name: &str) {
    s.execute("layer.new.layer", json!({"name": name})).unwrap();
}

/// Groups the layer `name`.
fn group(s: &mut photocraft_engine::Session, name: &str, group_name: &str) {
    select(s, name);
    s.execute("layer.groupLayers", json!({"name": group_name})).unwrap();
}

fn set_expanded(s: &mut photocraft_engine::Session, name: &str, expanded: bool) {
    let layer = id(&s.active().unwrap().doc, name);
    s.execute("layer.setExpanded", json!({"layer": layer, "expanded": expanded})).unwrap();
}

/// The rows the panel draws under the Group filter, top to bottom, two spaces per level.
fn listed(doc: &Document) -> Vec<String> {
    let f = group_filter();
    let ids = f.group_ids(doc);
    crate::layer_tree_ui::display_rows(doc, f.show_all())
        .into_iter()
        .filter(|(_, l)| f.shows_in(&ids, l))
        .map(|(depth, l)| format!("{}{}", "  ".repeat(depth), l.name))
        .collect()
}

/// Top Pixel · Outer { Inner { Deep } } · Background.
fn nested() -> photocraft_engine::Session {
    let mut s = new_session();
    add(&mut s, "Top Pixel");
    add(&mut s, "Deep");
    group(&mut s, "Deep", "Inner");
    group(&mut s, "Inner", "Outer");
    s.execute("layer.setExpanded", json!({"all": true, "expanded": true})).unwrap();
    s
}

#[test]
fn lists_nested_groups_with_their_contents_and_hides_loose_layers() {
    let s = nested();
    assert_eq!(listed(&s.active().unwrap().doc), ["Outer", "  Inner", "    Deep"]);
}

#[test]
fn collapsed_groups_stay_collapsed() {
    let mut s = nested();
    set_expanded(&mut s, "Inner", false);
    assert_eq!(listed(&s.active().unwrap().doc), ["Outer", "  Inner"], "a collapsed nested group hides its children");
    set_expanded(&mut s, "Outer", false);
    assert_eq!(listed(&s.active().unwrap().doc), ["Outer"], "a collapsed group hides everything inside");
    set_expanded(&mut s, "Outer", true);
    assert_eq!(listed(&s.active().unwrap().doc), ["Outer", "  Inner"], "each group keeps its own state");
}

#[test]
fn an_artboard_is_not_listed_but_its_groups_are() {
    let mut s = new_session();
    add(&mut s, "G child");
    group(&mut s, "G child", "Plain G");
    select(&mut s, "Background");
    add(&mut s, "A child");
    s.execute("layer.new.artboardFromLayers", json!({"name": "Artboard 1"})).unwrap();
    select(&mut s, "A child");
    add(&mut s, "GA child");
    group(&mut s, "GA child", "G in AB");
    s.execute("layer.setExpanded", json!({"all": true, "expanded": true})).unwrap();
    assert_eq!(listed(&s.active().unwrap().doc), ["  G in AB", "    GA child", "Plain G", "  G child"]);
    set_expanded(&mut s, "Artboard 1", false);
    assert_eq!(listed(&s.active().unwrap().doc), ["Plain G", "  G child"], "a collapsed artboard hides the groups inside it");
}

#[test]
fn the_selection_does_not_change_the_list_or_the_selection() {
    let mut s = nested();
    select(&mut s, "Deep");
    let doc_selected = listed(&s.active().unwrap().doc);
    let before = s.active().unwrap().active_layer;
    select(&mut s, "Top Pixel");
    assert_eq!(listed(&s.active().unwrap().doc), doc_selected, "same list whatever is selected");
    select(&mut s, "Deep");
    assert_eq!(s.active().unwrap().active_layer, before, "listing never selects or deselects");
}

#[test]
fn group_is_set_by_name_through_the_control_channel() {
    let ui = UiState::default();
    let f = super::patched(&ui, &json!({"by": "group"})).unwrap();
    assert_eq!(f.by, FilterBy::Group);
    assert!(super::patched(&ui, &json!({"by": "nope"})).is_err());
}

#[test]
fn lists_empty_groups() {
    let mut s = new_session();

    s.execute("layer.new.group", json!({"name": "Empty Group"})).unwrap();

    assert_eq!(
        listed(&s.active().unwrap().doc),
        ["Empty Group"],
        "empty groups should appear in the Group filter"
    );
}