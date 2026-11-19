//! Layer › Layer Style shows a check mark in front of each effect the active layer already has
//! (#14844). The ten effect entries report `Some(true)` while the active layer holds at least one
//! effect of that kind, switched on or off, and `Some(false)` with no document or no active layer.
//! The submenu's other entries are not effect entries and keep the state they have today.

use photocraft_doc::LayerId;
use serde_json::json;

use super::*;
// `checked` lives in `menus::state` on main; the rows come from `items::menu_rows`, which replaced
// the old `menu_items` list (its rows name the command through `command.id` and their state
// through `kind`).
use super::state::checked;

/// The ten effect entries of Layer › Layer Style, in menu order (`menu_catalog.rs`).
const EFFECTS: [&str; 10] = [
    "layer.layerStyle.bevelEmboss",
    "layer.layerStyle.stroke",
    "layer.layerStyle.innerShadow",
    "layer.layerStyle.innerGlow",
    "layer.layerStyle.satin",
    "layer.layerStyle.colorOverlay",
    "layer.layerStyle.gradientOverlay",
    "layer.layerStyle.patternOverlay",
    "layer.layerStyle.outerGlow",
    "layer.layerStyle.dropShadow",
];

/// The submenu's other entries: no effect behind them, so their `checked` stays as it is today.
const OTHERS: [&str; 8] = [
    "layer.layerStyle.blendingOptions",
    "layer.layerStyle.copyLayerStyle",
    "layer.layerStyle.pasteLayerStyle",
    "layer.layerStyle.clear",
    "layer.layerStyle.globalLight",
    "layer.layerStyle.createLayer",
    "layer.layerStyle.hideAllEffects",
    "layer.layerStyle.scaleEffects",
];

/// A 64 × 64 document with a layer "Paint" above the background, "Paint" active.
fn doc_with_layer() -> (Box<PhotocraftApp>, LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64, "background": "white"})).unwrap();
    app.sync_views();
    let paint = LayerId(app.run("layer.new.layer", json!({"name": "Paint"})).unwrap()["layer"].as_u64().unwrap());
    (app, paint)
}

/// The ten entries' check state.
fn marks(app: &PhotocraftApp) -> Vec<Option<bool>> {
    EFFECTS.iter().map(|id| checked(app, id)).collect()
}

/// The effect entries that report a mark, in menu order.
fn marked(app: &PhotocraftApp) -> Vec<&'static str> {
    EFFECTS.iter().copied().filter(|id| checked(app, id) == Some(true)).collect()
}

/// The same ten marks as the Layer menu itself builds them, which is what `ui.menu.list` reports.
fn menu_marks(app: &PhotocraftApp) -> Vec<(&'static str, Option<bool>)> {
    let rows = menu_rows(app);
    EFFECTS
        .iter()
        .map(|id| {
            let found = rows.iter().find(|i| i.path == ["Layer", "Layer Style"] && i.command.id == *id);
            (*id, found.unwrap_or_else(|| panic!("{id} is in Layer › Layer Style")).kind.checked())
        })
        .collect()
}

#[test]
fn a_layer_without_effects_marks_none_of_the_ten() {
    let (app, _) = doc_with_layer();
    assert_eq!(marks(&app), vec![Some(false); EFFECTS.len()], "a plain layer: every effect entry unchecked");
    assert!(marked(&app).is_empty(), "no mark on a layer without a style");
}

/// The Done-when case: a Bevel & Emboss and a Drop Shadow are the only two marks.
#[test]
fn a_bevel_and_a_drop_shadow_are_the_only_two_marks() {
    let (mut app, _) = doc_with_layer();
    app.run("layer.layerStyle.bevelEmboss", json!({})).unwrap();
    app.run("layer.layerStyle.dropShadow", json!({"distance": 6, "size": 3})).unwrap();
    assert_eq!(marked(&app), vec!["layer.layerStyle.bevelEmboss", "layer.layerStyle.dropShadow"], "exactly the two effects the layer has");
    let from_menu: Vec<&str> = menu_marks(&app).into_iter().filter(|(_, c)| *c == Some(true)).map(|(id, _)| id).collect();
    assert_eq!(from_menu, vec!["layer.layerStyle.bevelEmboss", "layer.layerStyle.dropShadow"], "the Layer menu rows carry both marks");
}

#[test]
fn clearing_the_layer_style_clears_the_marks() {
    let (mut app, _) = doc_with_layer();
    app.run("layer.layerStyle.bevelEmboss", json!({})).unwrap();
    app.run("layer.layerStyle.dropShadow", json!({})).unwrap();
    assert_eq!(marked(&app).len(), 2, "both marks are on before Clear Layer Style");
    app.run("layer.layerStyle.clear", json!({})).unwrap();
    assert_eq!(marks(&app), vec![Some(false); EFFECTS.len()], "Clear Layer Style takes both marks away");
}

/// Several instances of one kind are allowed: one mark for the kind, and the mark stays until the
/// last instance is gone.
#[test]
fn several_instances_of_a_kind_share_one_mark() {
    let (mut app, _) = doc_with_layer();
    app.run("layer.layerStyle.dropShadow", json!({})).unwrap();
    app.run("layer.layerStyle.dropShadow", json!({"distance": 12, "add": true})).unwrap();
    assert_eq!(marked(&app), vec!["layer.layerStyle.dropShadow"], "two shadows, one mark");
    app.run("layer.layerStyle.clear", json!({"effect": 0})).unwrap();
    assert_eq!(marked(&app), vec!["layer.layerStyle.dropShadow"], "the shadow that is left keeps the mark");
    app.run("layer.layerStyle.clear", json!({"effect": 0})).unwrap();
    assert!(marked(&app).is_empty(), "and it goes with the last shadow");
}

/// An effect the layer has but has switched off keeps its mark.
#[test]
fn an_effect_switched_off_keeps_its_mark() {
    let (mut app, _) = doc_with_layer();
    app.run("layer.layerStyle.stroke", json!({"size": 3, "enabled": false})).unwrap();
    assert_eq!(marked(&app), vec!["layer.layerStyle.stroke"], "the mark follows the effect existing, not it being on");
}

/// The mark follows the active layer, which is what several selected layers use.
#[test]
fn the_marks_follow_the_active_layer() {
    let (mut app, paint) = doc_with_layer();
    app.run("layer.layerStyle.dropShadow", json!({})).unwrap();
    let other = LayerId(app.session.active().unwrap().doc.layers.iter().map(|l| l.id).find(|l| *l != paint).unwrap().0);
    app.run("layer.select", json!({"layer": other.0})).unwrap();
    assert!(marked(&app).is_empty(), "the layer underneath has no style: no mark");
    app.run("layer.select", json!({"layer": paint.0})).unwrap();
    assert_eq!(marked(&app), vec!["layer.layerStyle.dropShadow"], "the active layer's own effect is marked");
}

/// Check items stay check items with nothing to read: `Some(false)`, not `None` (a native menu
/// cannot change an item's kind in place).
#[test]
fn the_ten_stay_unchecked_without_a_document_or_an_active_layer() {
    let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    assert_eq!(marks(&app), vec![Some(false); EFFECTS.len()], "no document open");
    let (mut app, _) = doc_with_layer();
    app.run("layer.layerStyle.colorOverlay", json!({"color": "#ff0000"})).unwrap();
    app.session.active_mut().unwrap().active_layer = None;
    assert_eq!(marks(&app), vec![Some(false); EFFECTS.len()], "a document with no active layer");
}

/// Only the effect entries carry marks; the rest of the submenu is untouched.
#[test]
fn the_other_submenu_entries_keep_their_state() {
    let (mut app, _) = doc_with_layer();
    app.run("layer.layerStyle.bevelEmboss", json!({})).unwrap();
    for id in OTHERS {
        assert_eq!(checked(&app, id), None, "{id} is not an effect entry");
    }
}
