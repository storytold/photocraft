//! #14970: what the Layer Style dialog hands the Pattern Overlay page's pattern picker. The page is the
//! framework's `PresetPicker` over the presets this module builds, and it draws exactly what a preset row carries:
//! a row with a picture paints a thumbnail in the field and a tile in the popup, a group row is a folder with its
//! chevron, and the picker's face falls back to the row's name when the row has no picture. The measured page shows
//! the current pattern as a thumbnail and the library as folders with the loose patterns after them, so the presets
//! have to carry the swatch and the folder each pattern belongs to.
use craft::ui::patterns::presets::PresetRow;
use craft::ui::runtime::env::Env;
use photocraft_doc::Effect;
use photocraft_engine::presets::{Group, PresetsExt};
use serde_json::json;

use super::tests::{app_with_layer, cancel, edit, first_of, layer_effects, ok};
use super::{EffectKind, PatternPresets, open, pattern_presets, pattern_rows};
use crate::PhotocraftApp;

/// The six patterns the tests use, by shown name. Built-in patterns only serve as the pixel source: each is taken
/// under a name and an id of its own, so nothing here depends on what the shipped library happens to hold.
const TILES: [&str; 6] = ["Plain", "Stripes", "Dots", "Linen", "Canvas", "Unfiled"];

/// A built-in pattern, renamed to `name` under the id `tile-<its index in TILES>`.
fn renamed(builtins: &mut Vec<photocraft_doc::Pattern>, name: &str) -> photocraft_doc::Pattern {
    let index = TILES.iter().position(|tile| *tile == name).expect("a test pattern name");
    let mut pattern = builtins.remove(0);
    pattern.id = format!("tile-{index}");
    pattern.name = name.to_string();
    pattern
}

/// An app with a Pattern Overlay layer and a library of our own: "Basics" holds the first three patterns, "Cloth" the
/// next two, and `Unfiled` belongs to no folder. The overlay command copies the pattern it uses into the document, so
/// `Unfiled` is a pattern of the document and of the library at once.
fn app_with_patterns() -> Box<PhotocraftApp> {
    let mut app = app_with_layer(16);
    let mut builtins = photocraft_engine::pattern_cmds::builtin();
    let tiles: Vec<photocraft_doc::Pattern> = TILES.into_iter().map(|name| renamed(&mut builtins, name)).collect();
    app.session.patterns.items.clear();
    for tile in tiles {
        app.session.patterns.upsert(tile);
    }
    let presets = app.session.presets_mut();
    presets.pattern_groups = vec![
        Group::new("Basics", vec!["tile-0".to_string(), "tile-1".to_string(), "tile-2".to_string()]),
        Group::new("Cloth", vec!["tile-3".to_string(), "tile-4".to_string()]),
    ];
    app.run("layer.layerStyle.patternOverlay", json!({"pattern": "tile-5"})).expect("a pattern overlay");
    app
}

/// The rows the Pattern field's picker is handed for `app`.
fn rows(app: &PhotocraftApp) -> Vec<PresetRow<String>> {
    let ctx = egui::Context::default();
    let presets: PatternPresets = pattern_presets(app, &ctx);
    pattern_rows(&presets, &Env::of_root(&ctx))
}

/// The labels of the group rows, in order.
fn groups(rows: &[PresetRow<String>]) -> Vec<String> {
    rows.iter().filter(|row| row.is_group()).map(|row| row.label.clone()).collect()
}

/// The rows below the first group row named `group`: its children.
fn children(rows: &[PresetRow<String>], group: &str) -> Vec<(String, usize)> {
    let at = rows.iter().position(|row| row.is_group() && row.label == group).expect("the folder is a row");
    rows[at + 1..].iter().take_while(|row| row.depth > rows[at].depth).map(|row| (row.label.clone(), row.depth)).collect()
}

/// The patterns at the top level: the loose ones, which come after every folder and its children.
fn loose(rows: &[PresetRow<String>]) -> Vec<String> {
    rows.iter().filter(|row| !row.is_group() && row.depth == 0).map(|row| row.label.clone()).collect()
}

/// The id of the pattern the document's Pattern Overlay uses: the one the field holds when it opens.
fn shown_pattern(app: &PhotocraftApp) -> String {
    app.session.active().expect("a document").doc.patterns[0].id.clone()
}

/// The library's folders are rows of their own, in the library's order.
#[test]
fn the_pattern_field_offers_the_library_folders_as_groups() {
    assert_eq!(groups(&rows(&app_with_patterns())), ["Basics", "Cloth"], "the library's folders, in the library's order");
}

/// A folder's patterns are rows below it, one level deeper, in the folder's order.
#[test]
fn a_folder_holds_its_patterns_as_rows_below_it() {
    let rows = rows(&app_with_patterns());
    let at = rows.iter().position(|row| row.is_group() && row.label == "Basics").expect("the folder is a row");
    assert_eq!(rows[at].depth, 0, "a top-level folder");
    assert_eq!(
        children(&rows, "Basics"),
        [("Plain".to_string(), 1), ("Stripes".to_string(), 1), ("Dots".to_string(), 1)],
        "the folder's patterns, nested under it"
    );
}

/// The pattern in no folder comes after every folder and its patterns, and is still a top-level row.
#[test]
fn the_patterns_in_no_folder_come_after_the_folders() {
    let rows = rows(&app_with_patterns());
    assert_eq!(loose(&rows), ["Unfiled"], "the pattern in no folder, at the top level");
    let last_nested = rows.iter().rposition(|row| row.depth > 0).expect("a folder's pattern");
    let first_loose = rows.iter().position(|row| !row.is_group() && row.depth == 0).expect("a loose pattern");
    assert!(first_loose > last_nested, "the loose pattern comes after the folders and their patterns");
}

/// Every pattern paints its swatch, so the field shows the current pattern as a thumbnail and the popup shows tiles;
/// a folder row paints nothing, it is a header.
#[test]
fn every_pattern_row_carries_the_swatch_it_paints() {
    let rows = rows(&app_with_patterns());
    let missing: Vec<&str> = rows.iter().filter(|row| !row.is_group() && row.preview.is_none()).map(|row| row.label.as_str()).collect();
    assert!(missing.is_empty(), "these patterns have no thumbnail: {missing:?}");
    assert!(rows.iter().filter(|row| row.is_group()).all(|row| row.preview.is_none()), "a folder is a header, not a swatch");
}

/// Every offered pattern is a row once: the pattern the document and the library both hold stands once, and the count
/// is what the session offers rather than a number the test carries.
#[test]
fn every_offered_pattern_is_exactly_one_row() {
    let app = app_with_patterns();
    let rows = rows(&app);
    let shared: Vec<&PresetRow<String>> = rows.iter().filter(|row| row.key == "tile-5").collect();
    assert_eq!(shared.len(), 1, "the overlay copied it into the document and the library still has it");
    assert_eq!(shared[0].depth, 0, "and it is in no folder, so it is a loose row");
    let keys: Vec<&str> = rows.iter().filter(|row| !row.is_group()).map(|row| row.key.as_str()).collect();
    let unique: Vec<&str> = {
        let mut seen = std::collections::HashSet::new();
        keys.iter().filter(|key| seen.insert(*key)).copied().collect()
    };
    assert_eq!(keys, unique, "no pattern is a row twice");
    let mut offered = std::collections::HashSet::new();
    offered.extend(app.session.patterns.items.iter().map(|pattern| pattern.id.clone()));
    offered.extend(app.session.active().expect("a document").doc.patterns.iter().map(|pattern| pattern.id.clone()));
    assert_eq!(keys.len(), offered.len(), "one row per offered pattern");
}

/// The page takes an empty Pattern value from the first non-group row, so with the measured order - folders before
/// the loose patterns - an overlay that names no pattern starts on the first pattern of the first folder, which is
/// not the document's own loose pattern the flat list used to start on. Pinned because the order is what the issue
/// measured.
#[test]
fn an_empty_pattern_value_resolves_to_the_first_folder_pattern() {
    let rows = rows(&app_with_patterns());
    let first = rows.iter().find(|row| !row.is_group()).expect("a pattern row");
    assert_eq!(first.key, "tile-0", "the first pattern of the first folder");
    assert_eq!(first.label, "Plain");
    assert_eq!(shown_pattern(&app_with_patterns()), "tile-5", "which is not the pattern the document's overlay carries");
}

/// Two library folders may carry the same name, and their chevrons are their own: their rows need keys of their own.
#[test]
fn two_folders_of_the_same_name_toggle_apart() {
    let mut app = app_with_patterns();
    app.session.presets_mut().pattern_groups.push(Group::new("Basics", vec!["tile-5".to_string()]));
    let rows = rows(&app);
    let named: Vec<&PresetRow<String>> = rows.iter().filter(|row| row.is_group() && row.label == "Basics").collect();
    assert_eq!(named.len(), 2, "both folders show");
    assert_ne!(named[0].key, named[1].key, "and each chevron belongs to one of them");
    assert_eq!(
        children(&rows, "Basics"),
        [("Plain".to_string(), 1), ("Stripes".to_string(), 1), ("Dots".to_string(), 1)],
        "the first of them keeps its patterns"
    );
}

/// A folder that holds none of the picker's patterns gets no header, and a pattern two folders both claim is a row
/// once, under the first of them.
#[test]
fn an_empty_folder_shows_no_header_and_a_shared_pattern_is_a_row_once() {
    let mut app = app_with_patterns();
    let presets = app.session.presets_mut();
    presets.pattern_groups.push(Group::new("Emptied", vec!["no-such-pattern".to_string()]));
    presets.pattern_groups.push(Group::new("Also Basics", vec!["tile-0".to_string()]));
    let rows = rows(&app);
    assert_eq!(groups(&rows), ["Basics", "Cloth"], "the emptied folder shows no header and the shared one adds none");
    let twice: Vec<&PresetRow<String>> = rows.iter().filter(|row| row.key == "tile-0").collect();
    assert_eq!(twice.len(), 1, "the pattern two folders claim is one row");
    assert_eq!(twice[0].depth, 1, "under the folder that came first");
}

/// The pattern the field holds is one of the rows it offers, so its own swatch is what the field paints.
#[test]
fn the_pattern_the_field_holds_is_a_row_with_a_swatch() {
    let mut app = app_with_patterns();
    let pattern = shown_pattern(&app);
    let id = open(&mut app, Some(EffectKind::PatternOverlay.key())).expect("the dialog opens");
    let rows = rows(&app);
    let row = rows.iter().find(|row| row.key == pattern).expect("the pattern is a row");
    assert!(row.preview.is_some(), "the shown pattern paints a thumbnail");
    cancel(&mut app, id);
}

/// Cancel on the dialog: the layer keeps the pattern the overlay had.
#[test]
fn cancel_keeps_the_pattern_the_overlay_had() {
    let mut app = app_with_patterns();
    let (opened, picked) = (shown_pattern(&app), "tile-0".to_string());
    let id = open(&mut app, Some(EffectKind::PatternOverlay.key())).expect("the dialog opens");
    let overlay = first_of(&app, id, EffectKind::PatternOverlay);
    edit(&mut app, id, |draft| draft.set_param(overlay, "pattern", super::StyleValue::Text(picked.clone())));
    cancel(&mut app, id);
    let Effect::PatternOverlay { id: shown, .. } = &layer_effects(&app.session)[0] else {
        panic!("the layer keeps its pattern overlay");
    };
    assert_eq!(shown, &opened, "Cancel restores the pattern the overlay had");
}

/// The other side of the same edit: OK applies the pattern the field picked.
#[test]
fn ok_applies_the_pattern_the_field_picked() {
    let mut app = app_with_patterns();
    let (opened, picked) = (shown_pattern(&app), "tile-0".to_string());
    let id = open(&mut app, Some(EffectKind::PatternOverlay.key())).expect("the dialog opens");
    let overlay = first_of(&app, id, EffectKind::PatternOverlay);
    assert_ne!(picked, opened, "the picked pattern is another one than the overlay had");
    edit(&mut app, id, |draft| draft.set_param(overlay, "pattern", super::StyleValue::Text(picked.clone())));
    ok(&mut app, id);
    let Effect::PatternOverlay { id: shown, .. } = &layer_effects(&app.session)[0] else {
        panic!("the layer keeps its pattern overlay");
    };
    assert_eq!(shown, &picked, "OK applies the picked pattern");
}
