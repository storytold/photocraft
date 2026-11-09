//! Shared picker search state access, query matching and search-row presentation.

use std::collections::{BTreeMap, BTreeSet};

use crate::pickers::browser::projection::{GroupView, ItemView};
use crate::pickers::state::PresetUi;

/// Return the normalized query used by the shared browser for one picker panel.
pub(crate) fn query(state: &PresetUi, panel: &str) -> String {
    state.search.get(panel).map(|query| query.trim().to_lowercase()).unwrap_or_default()
}

/// Draw the standard search row for panels that expose the common search control.
pub(crate) fn show_search_bar(ui: &mut egui::Ui, state: &mut PresetUi, panel: &str) {
    if matches!(panel, "gradients" | "swatches" | "patterns") && state.search_visible.get(panel).copied().unwrap_or(true) {
        let query = state.search.entry(panel.to_owned()).or_default();
        ui.add(egui::TextEdit::singleline(query).desired_width(ui.available_width()).hint_text(tl!("Search")));
    }
}

pub(crate) fn group_matches_query(group: &GroupView, query: &str) -> bool {
    !query.is_empty() && group.name.to_lowercase().contains(query)
}

pub(crate) fn item_matches_query(item: &ItemView, query: &str) -> bool {
    !query.is_empty() && (item.name.to_lowercase().contains(query) || item.group_path.iter().any(|part| part.to_lowercase().contains(query)))
}

/// Include ancestors of matching groups so search results retain their full hierarchy.
/// Ancestor inclusion affects headers only; card matching remains independently filtered.
pub(crate) fn groups_visible_for_query(groups: &[GroupView], parents: &BTreeMap<String, String>, query: &str) -> BTreeSet<String> {
    let mut visible = BTreeSet::new();
    if query.is_empty() {
        return groups.iter().map(|group| group.name.clone()).collect();
    }

    for group in groups {
        if !group_matches_query(group, query) && !group.items.iter().any(|item| item_matches_query(item, query)) {
            continue;
        }
        let mut current = Some(group.name.as_str());
        while let Some(name) = current {
            if !visible.insert(name.to_owned()) {
                break;
            }
            current = parents.get(name).map(String::as_str);
        }
    }
    visible
}

pub(crate) fn item_visible_for_query(item: &ItemView, query: &str, group_matches: bool) -> bool {
    query.is_empty() || group_matches || item_matches_query(item, query)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::groups_visible_for_query;
    use crate::pickers::browser::projection::{GroupView, ItemView};

    #[test]
    fn matching_child_group_keeps_its_complete_parent_hierarchy_visible() {
        let groups = vec![
            GroupView {
                name: "Group A".into(),
                root_only: false,
                items: vec![ItemView { key: "parent-card".into(), name: "Parent card".into(), group_path: Vec::new() }],
            },
            GroupView {
                name: "Group B".into(),
                root_only: false,
                items: vec![ItemView { key: "child-card".into(), name: "Child card".into(), group_path: Vec::new() }],
            },
            GroupView {
                name: "Unrelated".into(),
                root_only: false,
                items: vec![ItemView { key: "other-card".into(), name: "Other card".into(), group_path: Vec::new() }],
            },
        ];
        let parents = BTreeMap::from([("Group B".into(), "Group A".into())]);
        assert_eq!(groups_visible_for_query(&groups, &parents, "group b"), ["Group A".to_owned(), "Group B".to_owned()].into());
    }

    #[test]
    fn matching_group_item_also_keeps_all_ancestors_visible() {
        let groups = vec![
            GroupView { name: "Root".into(), root_only: false, items: Vec::new() },
            GroupView { name: "Child".into(), root_only: false, items: Vec::new() },
            GroupView {
                name: "Grandchild".into(),
                root_only: false,
                items: vec![ItemView { key: "target".into(), name: "Target asset".into(), group_path: Vec::new() }],
            },
        ];
        let parents = BTreeMap::from([("Child".into(), "Root".into()), ("Grandchild".into(), "Child".into())]);
        assert_eq!(groups_visible_for_query(&groups, &parents, "target"), ["Root".to_owned(), "Child".to_owned(), "Grandchild".to_owned()].into());
    }
}
