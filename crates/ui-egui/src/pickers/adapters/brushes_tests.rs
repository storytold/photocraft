use super::super::brush_engine_adapter::{project_brush_browser, preview_identity, root_group_key};
use super::super::brush_engine_adapter::brush_item_move_action;
use photocraft_engine::paint::BrushPreset;

fn preset(name: &str, group: &str, folder: &[&str]) -> BrushPreset {
    let mut preset = photocraft_engine::Session::new().tools.presets[0].clone();
    preset.name = name.into();
    preset.group = group.into();
    preset.folder = folder.iter().map(|part| (*part).to_owned()).collect();
    preset
}

#[test]
fn folders_project_as_shared_group_views_and_items_keep_name_keys() {
    let presets = vec![preset("Ink", "Pens", &["Fine"]), preset("Marker", "Pens", &["Fine"]), preset("Pencil", "Pens", &[])];
    let mut state = crate::pickers::state::PresetUi::default();
    let projection = project_brush_browser(&presets, &mut state, "brushes");

    let folder = projection.groups.iter().find(|group| group.label == "Fine").expect("nested folder group");
    assert!(!folder.root_only);
    assert_eq!(folder.items.iter().map(|item| item.key.as_str()).collect::<Vec<_>>(), vec!["Ink", "Marker"]);
    assert_eq!(preview_identity(&presets[0]), "Ink");
    assert!(projection.groups.iter().any(|group| group.label == "Pens" && group.items.iter().any(|item| item.key == "Pencil")));
}

#[test]
fn empty_group_is_persisted_in_shared_layout_and_reprojected() {
    let mut state = crate::pickers::state::PresetUi::default();
    let empty_key = root_group_key("Empty");
    let pens_key = root_group_key("Pens");
    let folder_key = super::super::brush_engine_adapter::group_view_key("Pens", &["Fine".to_owned()]);
    state.set_hierarchy_order("brushes", "", vec![
        crate::pickers::state::HierarchyOrderEntry::Group(empty_key.clone()),
        crate::pickers::state::HierarchyOrderEntry::Group(pens_key.clone()),
    ]);
    state.set_hierarchy_order("brushes", &pens_key, vec![crate::pickers::state::HierarchyOrderEntry::Group(folder_key.clone())]);

    let projection = project_brush_browser(&[], &mut state, "brushes");
    assert!(projection.groups.iter().any(|group| group.name == empty_key && group.label == "Empty" && group.items.is_empty()));
    assert!(projection.groups.iter().any(|group| group.name == folder_key && group.label == "Fine" && group.items.is_empty()));

    let encoded = serde_json::to_value(&state).unwrap();
    let mut restored: crate::pickers::state::PresetUi = serde_json::from_value(encoded).unwrap();
    let restored_projection = project_brush_browser(&[], &mut restored, "brushes");
    assert!(restored_projection.groups.iter().any(|group| group.name == empty_key && group.items.is_empty()));
    assert!(restored_projection.groups.iter().any(|group| group.name == folder_key && group.items.is_empty()));
}

#[test]
fn item_move_resolves_folder_container_and_before_anchor() {
    let presets = vec![preset("First", "Pens", &["Fine"]), preset("Second", "Pens", &["Fine"]), preset("Other", "Pens", &[])];
    let mut state = crate::pickers::state::PresetUi::default();
    let projection = project_brush_browser(&presets, &mut state, "brushes");
    let folder = projection.containers.values().find(|container| container.engine_group == "Pens" && container.folder == vec!["Fine".to_owned()])
        .expect("folder container");
    let groups = projection.groups.clone();
    let action = brush_item_move_action("brushes", &projection, &groups, "Other", &folder.key, 1).unwrap();
    assert!(matches!(action, super::Action::MoveItem { name, destination, before, .. }
        if name == "Other" && destination.engine_group == "Pens" && destination.folder == ["Fine"] && before.as_deref() == Some("Second")));
}
