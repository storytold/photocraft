//! Regression tests for occurrence identity in the shared brush browser.
//!
//! Keep these tests in a separate module so they can exercise the small pure adapter helpers
//! without coupling identity guarantees to egui frame layout or pointer input.

use photocraft_engine::paint::{BrushPreset, BrushSettings};

use crate::pickers::adapters::brushes::{browser_group_views, is_ambiguous_preset_name, preset_by_identity, preview_identity};

fn preset(name: &str, group: &str, size: f32) -> BrushPreset {
    let brush = BrushSettings { size, ..BrushSettings::default() };
    BrushPreset { name: name.into(), brush, builtin: false, group: group.into(), folder: Vec::new(), captures: Default::default() }
}

#[test]
fn same_name_occurrences_have_distinct_identity_and_resolve_to_their_source() {
    let presets = [preset("Soft Round", "Favorites", 17.0), preset("Soft Round", "Imported", 83.0)];
    let first_key = preview_identity(&presets[0]);
    let second_key = preview_identity(&presets[1]);

    assert_ne!(first_key, second_key, "same display labels must not alias occurrence identity");

    let groups = browser_group_views(&presets);
    let projected: Vec<_> = groups.iter().flat_map(|group| group.items.iter()).collect();
    assert_eq!(projected.len(), 2);
    assert_eq!(projected[0].name, projected[1].name, "labels remain human-readable names");
    assert_ne!(projected[0].key, projected[1].key, "browser item keys must identify occurrences");

    let first = preset_by_identity(&presets, &first_key).expect("first occurrence should resolve");
    let second = preset_by_identity(&presets, &second_key).expect("second occurrence should resolve");
    assert_eq!(first.group, "Favorites");
    assert_eq!(first.brush.size, 17.0);
    assert_eq!(second.group, "Imported");
    assert_eq!(second.brush.size, 83.0);
}

#[test]
fn name_based_mutation_is_guarded_when_duplicate_names_are_ambiguous() {
    let presets = [preset("Soft Round", "Favorites", 17.0), preset("Soft Round", "Imported", 83.0)];

    assert!(is_ambiguous_preset_name(&presets, "Soft Round"));
    assert!(is_ambiguous_preset_name(&presets, "soft round"), "engine name matching is case-insensitive");
    assert!(!is_ambiguous_preset_name(&presets, "Chalk"));
}

#[test]
fn a_unique_name_remains_eligible_for_name_based_mutation() {
    let presets = [preset("Soft Round", "Favorites", 17.0), preset("Chalk", "Imported", 83.0)];

    assert!(!is_ambiguous_preset_name(&presets, "Soft Round"));
    assert!(!is_ambiguous_preset_name(&presets, "chalk"));
}
