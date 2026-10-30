//! Persistent and transient state shared by preset pickers and floating preset panels.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub(crate) const MAX_GROUP_DEPTH: usize = 32;

/// Transient rename target for a nested group. Kept separate from the legacy string-keyed rename
/// state so arbitrary group labels remain lossless.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct NestedGroupRenameState {
    pub(crate) panel: String,
    pub(crate) group: String,
    pub(crate) group_path: Vec<String>,
    pub(crate) text: String,
}

/// A direct child in the picker's single ordered hierarchy space. Groups and cards are peers at
/// every parent; the variant identifies what the stable key refers to, not a separate layout tier.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "key", rename_all = "camelCase")]
pub(crate) enum HierarchyOrderEntry {
    Item(String),
    Group(String),
}

/// Panel state (serde, so the control channel can read and drive it).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PresetUi {
    /// Floating panels.
    #[serde(default)]
    pub gradients: bool,
    #[serde(default)]
    pub swatches: bool,
    #[serde(default)]
    pub patterns: bool,
    pub styles: bool,
    pub shapes: bool,
    pub tool_presets: bool,
    pub clone_source: bool,
    /// Thumbnail size per panel (px; the minimum shows a list).
    pub sizes: BTreeMap<String, f32>,
    /// Groups whose open state differs from the default (only the first group starts open).
    pub toggled: BTreeSet<String>,
    /// Selected preset per panel (by name / pattern id).
    pub selected: BTreeMap<String, String>,
    /// Tool Presets: Current Tool Only.
    pub current_tool_only: bool,
    /// Custom Shape tool: the shape it draws.
    pub custom_shape: String,
    /// Group order per panel, by name (#3658), like the dock's `tabOrder` (#2272). A panel
    /// with no entry keeps the library's order; unknown names are ignored and groups missing from
    /// a list follow it in library order. Saved with the UI state.
    #[serde(rename = "groupOrder", alias = "group_order", deserialize_with = "lenient_group_order", skip_serializing_if = "BTreeMap::is_empty")]
    pub group_order: BTreeMap<String, Vec<String>>,
    /// Parent group for nested asset groups, scoped per picker. Expansion stays keyed by group name,
    /// so moving a group under a collapsed parent never resets the child's own open state.
    #[serde(rename = "groupParent", alias = "group_parent", default, skip_serializing_if = "BTreeMap::is_empty")]
    pub group_parent: BTreeMap<String, BTreeMap<String, String>>,
    /// Legacy group-to-item anchors, read only to migrate older saved picker layouts into
    /// `hierarchy_order`. New state never serializes this split ordering representation.
    #[serde(rename = "groupItemAnchors", alias = "group_item_anchors", default, skip_serializing_if = "BTreeMap::is_empty")]
    pub group_item_anchors: BTreeMap<String, BTreeMap<String, (String, Option<String>)>>,
    /// Canonical direct-child order for each parent group. Entries contain both cards and groups,
    /// so positioning, rendering, and drag insertion all operate in the same sibling sequence.
    #[serde(rename = "hierarchyOrder", alias = "hierarchy_order", default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) hierarchy_order: BTreeMap<String, BTreeMap<String, Vec<HierarchyOrderEntry>>>,
    /// Visibility of the search row per picker. Missing entries default to visible.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub search_visible: BTreeMap<String, bool>,
    /// Case-insensitive asset-name filter per picker.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub search: BTreeMap<String, String>,
    /// In-place rename: (panel, preset or "group:<name>", text).
    #[serde(skip)]
    pub renaming: Option<(String, String, String)>,
    /// Typed in-place nested-group rename. Transient and intentionally excluded from serde.
    #[serde(skip)]
    pub(crate) group_renaming: Option<NestedGroupRenameState>,
}

/// Read `groupOrder` keeping what makes sense: non-object values, non-list entries and non-string
/// names are dropped instead of failing the whole saved UI state.
const MAX_GROUP_ORDER: usize = 10_000;
fn lenient_group_order<'de, D: serde::Deserializer<'de>>(d: D) -> Result<BTreeMap<String, Vec<String>>, D::Error> {
    let v = Value::deserialize(d)?;
    let mut out = BTreeMap::new();
    for (panel, names) in v.as_object().into_iter().flatten() {
        let Some(names) = names.as_array() else { continue };
        out.insert(panel.clone(), names.iter().filter_map(Value::as_str).take(MAX_GROUP_ORDER).map(str::to_owned).collect());
    }
    Ok(out)
}

impl PresetUi {
    pub fn shape(&self) -> &str {
        if self.custom_shape.is_empty() { tl!("Heart") } else { &self.custom_shape }
    }

    /// Display order of the root groups for `panel`, as library indices: canonical hierarchy order
    /// first, then legacy groupOrder, then groups missing from either in library order.
    pub(crate) fn display_order(&self, panel: &str, names: &[String]) -> Vec<usize> {
        let mut out: Vec<usize> = Vec::with_capacity(names.len());
        // Build this once: saved order and library names can both contain thousands of entries.
        // Repeated `position`/`contains` scans made this path quadratic.
        let mut indices: BTreeMap<&str, usize> = BTreeMap::new();
        for (i, name) in names.iter().enumerate() {
            indices.entry(name.as_str()).or_insert(i);
        }
        let mut included = vec![false; names.len()];
        // Root groups use the same canonical hierarchy sequence as nested group/card siblings.
        // The old groupOrder list remains a fallback for saved UI states written before
        // hierarchyOrder existed.
        for entry in self.hierarchy_order.get(panel).and_then(|parents| parents.get("")).into_iter().flatten() {
            let HierarchyOrderEntry::Group(name) = entry else { continue };
            if let Some(&i) = indices.get(name.as_str())
                && !included[i]
            {
                included[i] = true;
                out.push(i);
            }
        }
        for name in self.group_order.get(panel).into_iter().flatten() {
            if let Some(&i) = indices.get(name.as_str())
                && !included[i]
            {
                included[i] = true;
                out.push(i);
            }
        }
        for (i, is_included) in included.iter().enumerate() {
            if !is_included {
                out.push(i);
            }
        }
        out
    }

    /// Resolve one parent's canonical sibling sequence. Saved entries are validated against the
    /// current model, stale/duplicate keys are discarded, and newly-created entries are appended.
    /// When no canonical sequence exists, legacy item anchors are migrated deterministically:
    /// anchored groups are inserted immediately before their item, then unanchored groups append.
    pub(crate) fn ordered_hierarchy_entries(&self, panel: &str, parent: &str, item_keys: &[String], child_groups: &[String]) -> Vec<HierarchyOrderEntry> {
        let mut legacy = Vec::with_capacity(item_keys.len() + child_groups.len());
        let anchors = self.group_item_anchors.get(panel);
        for item in item_keys {
            for child in child_groups {
                if anchors
                    .and_then(|anchors| anchors.get(child))
                    .is_some_and(|(anchor_parent, before)| anchor_parent == parent && before.as_deref() == Some(item.as_str()))
                {
                    legacy.push(HierarchyOrderEntry::Group(child.clone()));
                }
            }
            legacy.push(HierarchyOrderEntry::Item(item.clone()));
        }
        for child in child_groups {
            let has_valid_anchor = anchors
                .and_then(|anchors| anchors.get(child))
                .is_some_and(|(anchor_parent, before)| anchor_parent == parent && before.as_ref().is_some_and(|key| item_keys.contains(key)));
            if !has_valid_anchor {
                legacy.push(HierarchyOrderEntry::Group(child.clone()));
            }
        }

        let Some(saved) = self.hierarchy_order.get(panel).and_then(|parents| parents.get(parent)) else {
            return legacy;
        };
        let valid_items: BTreeSet<String> = item_keys.iter().cloned().collect();
        let valid_groups: BTreeSet<String> = child_groups.iter().cloned().collect();
        let mut seen_items = BTreeSet::new();
        let mut seen_groups = BTreeSet::new();
        let mut out = Vec::with_capacity(legacy.len());
        for entry in saved {
            let valid = match entry {
                HierarchyOrderEntry::Item(key) => valid_items.contains(key) && seen_items.insert(key.clone()),
                HierarchyOrderEntry::Group(key) => valid_groups.contains(key) && seen_groups.insert(key.clone()),
            };
            if valid {
                out.push(entry.clone());
            }
        }
        for entry in legacy {
            let missing = match &entry {
                HierarchyOrderEntry::Item(key) => seen_items.insert(key.clone()),
                HierarchyOrderEntry::Group(key) => seen_groups.insert(key.clone()),
            };
            if missing {
                out.push(entry);
            }
        }
        out
    }

    /// Persist a complete sibling sequence and retire legacy anchors for that parent.
    pub(crate) fn set_hierarchy_order(&mut self, panel: &str, parent: &str, entries: Vec<HierarchyOrderEntry>) {
        self.hierarchy_order.entry(panel.to_string()).or_default().insert(parent.to_string(), entries);
        if parent.is_empty() {
            self.group_order.remove(panel);
        }
        if let Some(anchors) = self.group_item_anchors.get_mut(panel) {
            anchors.retain(|_, (anchor_parent, _)| anchor_parent != parent);
            if anchors.is_empty() {
                self.group_item_anchors.remove(panel);
            }
        }
    }

    /// Remove a child token from a parent's sibling sequence after it changes parent.
    pub(crate) fn remove_hierarchy_entry(&mut self, panel: &str, parent: &str, entry: &HierarchyOrderEntry) {
        if let Some(entries) = self.hierarchy_order.get_mut(panel).and_then(|parents| parents.get_mut(parent)) {
            entries.retain(|candidate| candidate != entry);
            if entries.is_empty() {
                if let Some(parents) = self.hierarchy_order.get_mut(panel) {
                    parents.remove(parent);
                    if parents.is_empty() {
                        self.hierarchy_order.remove(panel);
                    }
                }
            }
        }
    }

    /// Move one peer before another peer in the same parent, or append it when `before` is None.
    pub(crate) fn move_hierarchy_entry(
        &mut self,
        panel: &str,
        parent: &str,
        mut entries: Vec<HierarchyOrderEntry>,
        moving: HierarchyOrderEntry,
        before: Option<&HierarchyOrderEntry>,
    ) {
        if before == Some(&moving) {
            return;
        }
        entries.retain(|entry| entry != &moving);
        let at = before.and_then(|before| entries.iter().position(|entry| entry == before)).unwrap_or(entries.len());
        entries.insert(at, moving);
        self.set_hierarchy_order(panel, parent, entries);
    }

    /// Keep name-keyed card identities attached to their sibling position after a successful rename.
    pub(crate) fn rename_hierarchy_item(&mut self, panel: &str, from: &str, to: &str) {
        if from == to {
            return;
        }
        if let Some(orders) = self.hierarchy_order.get_mut(panel) {
            for entries in orders.values_mut() {
                for entry in entries {
                    if let HierarchyOrderEntry::Item(key) = entry
                        && key == from
                    {
                        *key = to.to_string();
                    }
                }
            }
        }
    }

    /// Remove a deleted card from all sibling sequences in the panel.
    pub(crate) fn delete_hierarchy_item(&mut self, panel: &str, key: &str) {
        if let Some(orders) = self.hierarchy_order.get_mut(panel) {
            for entries in orders.values_mut() {
                entries.retain(|entry| !matches!(entry, HierarchyOrderEntry::Item(item) if item == key));
            }
            orders.retain(|_, entries| !entries.is_empty());
            if orders.is_empty() {
                self.hierarchy_order.remove(panel);
            }
        }
    }

    /// Move a root group before `before` (`None` = last) in the canonical root sibling sequence.
    /// Legacy `groupOrder` is read as a fallback but new moves persist only `hierarchyOrder`.
    pub(crate) fn move_group(&mut self, panel: &str, names: &[String], from: &str, before: Option<&str>) {
        if before == Some(from) {
            return;
        }
        let valid_names: BTreeSet<&str> = names.iter().map(String::as_str).collect();
        let root_names: Vec<String> = names
            .iter()
            .filter(|name| self.group_parent.get(panel).and_then(|parents| parents.get(*name)).is_none_or(|parent| !valid_names.contains(parent.as_str())))
            .cloned()
            .collect();
        let mut order: Vec<String> = self.display_order(panel, &root_names).into_iter().filter_map(|i| root_names.get(i).cloned()).collect();
        let Some(at) = order.iter().position(|n| n == from) else { return };
        let moved = order.remove(at);
        let to = before.and_then(|b| order.iter().position(|n| n == b)).unwrap_or(order.len());
        order.insert(to, moved);
        let entries: Vec<HierarchyOrderEntry> = order.into_iter().map(HierarchyOrderEntry::Group).collect();
        if entries
            .iter()
            .map(|entry| match entry {
                HierarchyOrderEntry::Group(name) => name.as_str(),
                HierarchyOrderEntry::Item(_) => unreachable!(),
            })
            .eq(root_names.iter().map(String::as_str))
        {
            if let Some(orders) = self.hierarchy_order.get_mut(panel) {
                orders.remove("");
                if orders.is_empty() {
                    self.hierarchy_order.remove(panel);
                }
            }
            self.group_order.remove(panel);
        } else {
            self.set_hierarchy_order(panel, "", entries);
        }
    }

    /// Make `child` a child of `parent`, or a root group when `parent` is `None`. Reject cycles and
    /// stale names; expansion keys intentionally remain unchanged across the move.
    pub(crate) fn nest_group(&mut self, panel: &str, names: &[String], child: &str, parent: Option<&str>) {
        if !names.iter().any(|n| n == child) || parent.is_some_and(|p| !names.iter().any(|n| n == p) || p == child) {
            return;
        }
        let mut ancestor = parent;
        while let Some(name) = ancestor {
            if name == child {
                return;
            }
            ancestor = self.group_parent.get(panel).and_then(|parents| parents.get(name)).map(String::as_str);
        }
        match parent {
            Some(parent) => {
                self.group_parent.entry(panel.to_string()).or_default().insert(child.to_string(), parent.to_string());
                self.remove_hierarchy_entry(panel, "", &HierarchyOrderEntry::Group(child.to_string()));
            }
            None => {
                if let Some(parents) = self.group_parent.get_mut(panel) {
                    parents.remove(child);
                }
            }
        }
        if let Some(anchors) = self.group_item_anchors.get_mut(panel) {
            anchors.remove(child);
            if anchors.is_empty() {
                self.group_item_anchors.remove(panel);
            }
        }
        if self.group_parent.get(panel).is_some_and(BTreeMap::is_empty) {
            self.group_parent.remove(panel);
        }
    }

    /// Keep view-only hierarchy and expansion state attached to a group when its asset-library
    /// name changes. The caller invokes this only after the asset rename command succeeds.
    pub(crate) fn rename_group(&mut self, panel: &str, from: &str, to: &str) {
        if from == to {
            return;
        }
        if let Some(parents) = self.group_parent.get_mut(panel) {
            if let Some(parent) = parents.remove(from) {
                parents.insert(to.to_string(), parent);
            }
            for parent in parents.values_mut() {
                if parent == from {
                    *parent = to.to_string();
                }
            }
            if parents.is_empty() {
                self.group_parent.remove(panel);
            }
        }
        if let Some(anchors) = self.group_item_anchors.get_mut(panel) {
            if let Some(anchor) = anchors.remove(from) {
                anchors.insert(to.to_string(), anchor);
            }
            for (parent, _) in anchors.values_mut() {
                if parent == from {
                    *parent = to.to_string();
                }
            }
            if anchors.is_empty() {
                self.group_item_anchors.remove(panel);
            }
        }
        if let Some(order) = self.group_order.get_mut(panel) {
            for name in order {
                if name == from {
                    *name = to.to_string();
                }
            }
        }
        if let Some(orders) = self.hierarchy_order.get_mut(panel) {
            if let Some(entries) = orders.remove(from) {
                orders.insert(to.to_string(), entries);
            }
            for entries in orders.values_mut() {
                for entry in entries {
                    if let HierarchyOrderEntry::Group(name) = entry
                        && name == from
                    {
                        *name = to.to_string();
                    }
                }
            }
        }
        let old_prefix = format!("{panel}/{}", escape_hierarchy_key_part(from));
        let new_prefix = format!("{panel}/{}", escape_hierarchy_key_part(to));
        let renamed: Vec<String> = self
            .toggled
            .iter()
            .filter_map(|key| key.strip_prefix(&old_prefix).filter(|tail| tail.is_empty() || tail.starts_with('/')).map(|tail| format!("{new_prefix}{tail}")))
            .collect();
        self.toggled.retain(|key| !key.strip_prefix(&old_prefix).is_some_and(|tail| tail.is_empty() || tail.starts_with('/')));
        self.toggled.extend(renamed);
    }

    /// Detach children of a removed group and discard its ordering/expansion entries.
    pub(crate) fn delete_group(&mut self, panel: &str, name: &str) {
        if let Some(parents) = self.group_parent.get_mut(panel) {
            parents.remove(name);
            parents.retain(|_, parent| parent != name);
            if parents.is_empty() {
                self.group_parent.remove(panel);
            }
        }
        if let Some(anchors) = self.group_item_anchors.get_mut(panel) {
            anchors.remove(name);
            anchors.retain(|_, (parent, _)| parent != name);
            if anchors.is_empty() {
                self.group_item_anchors.remove(panel);
            }
        }
        if let Some(order) = self.group_order.get_mut(panel) {
            order.retain(|group| group != name);
            if order.is_empty() {
                self.group_order.remove(panel);
            }
        }
        if let Some(orders) = self.hierarchy_order.get_mut(panel) {
            orders.remove(name);
            for entries in orders.values_mut() {
                entries.retain(|entry| !matches!(entry, HierarchyOrderEntry::Group(group) if group == name));
            }
            orders.retain(|_, entries| !entries.is_empty());
            if orders.is_empty() {
                self.hierarchy_order.remove(panel);
            }
        }
        let prefix = format!("{panel}/{}", escape_hierarchy_key_part(name));
        self.toggled.retain(|key| !key.strip_prefix(&prefix).is_some_and(|tail| tail.is_empty() || tail.starts_with('/')));
    }

    /// Apply the Brushes panel's header-drop convention to asset groups: the upper third inserts
    /// before the target at its sibling level; the remainder nests into the target. A root group
    /// dropped onto a nested header's edge becomes a sibling of that nested group, not a root.
    pub(crate) fn drop_group_on_header(&mut self, panel: &str, names: &[String], child: &str, target: &str, edge: bool) {
        if child == target || !names.iter().any(|n| n == child) || !names.iter().any(|n| n == target) {
            return;
        }
        if !edge {
            self.nest_group(panel, names, child, Some(target));
            return;
        }
        let target_parent = self.group_parent.get(panel).and_then(|parents| parents.get(target)).cloned();
        let mut ancestor = target_parent.as_deref();
        while let Some(name) = ancestor {
            if name == child {
                return;
            }
            ancestor = self.group_parent.get(panel).and_then(|parents| parents.get(name)).map(String::as_str);
        }
        let destination_parent = target_parent.unwrap_or_default();
        self.nest_group(panel, names, child, Some(&destination_parent));
        let siblings: Vec<String> = names
            .iter()
            .filter(|name| self.group_parent.get(panel).and_then(|parents| parents.get(*name)).is_some_and(|parent| parent == &destination_parent))
            .cloned()
            .collect();
        let entries = self
            .hierarchy_order
            .get(panel)
            .and_then(|parents| parents.get(&destination_parent))
            .cloned()
            .unwrap_or_else(|| siblings.into_iter().map(HierarchyOrderEntry::Group).collect());
        self.move_hierarchy_entry(
            panel,
            &destination_parent,
            entries,
            HierarchyOrderEntry::Group(child.to_string()),
            Some(&HierarchyOrderEntry::Group(target.to_string())),
        );
    }

    /// Depth-first display order. Sibling order follows the existing per-panel order and corrupt
    /// cycles/stale parent references safely fall back to root-level entries.
    pub(crate) fn nested_display_order(&self, panel: &str, names: &[String]) -> Vec<(usize, usize)> {
        let ordered = self.display_order(panel, names);
        let mut out = Vec::new();
        let mut seen = BTreeSet::new();
        let valid_names: BTreeSet<&str> = names.iter().map(String::as_str).collect();
        let parents = self.group_parent.get(panel);
        // Bucket children by parent in display order. The old recursive implementation scanned
        // the complete ordered list for every visited group, quadratic for a deep/large hierarchy.
        let mut children: BTreeMap<Option<&str>, Vec<usize>> = BTreeMap::new();
        for &i in &ordered {
            let Some(name) = names.get(i) else { continue };
            let parent = parents.and_then(|m| m.get(name)).map(String::as_str).filter(|p| valid_names.contains(p));
            children.entry(parent).or_default().push(i);
        }
        // The unified sibling sequence is authoritative at every depth. `groupOrder` and
        // library order above are only fallback inputs for groups without a saved sequence.
        if let Some(panel_orders) = self.hierarchy_order.get(panel) {
            for (parent, siblings) in &mut children {
                let Some(entries) = panel_orders.get(parent.unwrap_or("")) else { continue };
                let mut positions = BTreeMap::new();
                for (position, entry) in entries.iter().enumerate() {
                    if let HierarchyOrderEntry::Group(name) = entry {
                        positions.entry(name.as_str()).or_insert(position);
                    }
                }
                siblings.sort_by_key(|index| names.get(*index).and_then(|name| positions.get(name.as_str()).copied()).unwrap_or(usize::MAX));
            }
        }
        fn visit(
            parent: Option<&str>,
            depth: usize,
            names: &[String],
            children: &BTreeMap<Option<&str>, Vec<usize>>,
            seen: &mut BTreeSet<String>,
            out: &mut Vec<(usize, usize)>,
        ) {
            let Some(siblings) = children.get(&parent) else { return };
            for &i in siblings {
                let Some(name) = names.get(i) else { continue };
                if !seen.insert(name.clone()) {
                    continue;
                }
                out.push((i, depth));
                if depth < MAX_GROUP_DEPTH {
                    visit(Some(name), depth + 1, names, children, seen, out);
                }
            }
        }
        visit(None, 0, names, &children, &mut seen, &mut out);
        // Any malformed cycle is detached for display rather than disappearing.
        for &i in &ordered {
            if let Some(name) = names.get(i)
                && !seen.contains(name)
            {
                out.push((i, 0));
            }
        }
        out
    }
}

/// Escape one path component in persisted picker expansion keys. `%` is escaped first so the
/// encoding is reversible; `/` can then safely remain the component separator.
pub(crate) fn escape_hierarchy_key_part(part: &str) -> String {
    part.replace('%', "%25").replace('/', "%2F")
}

#[cfg(test)]
mod tests {
    use super::{HierarchyOrderEntry, PresetUi};

    fn names(values: &[&str]) -> Vec<String> {
        values.iter().map(|name| (*name).to_string()).collect()
    }

    #[test]
    fn display_order_keeps_saved_order_and_appends_unlisted_names() {
        let mut state = PresetUi::default();
        state.group_order.insert("panel".into(), names(&["gone", "third", "third", "first"]));

        assert_eq!(state.display_order("panel", &names(&["first", "second", "third"])), vec![2, 0, 1]);
    }

    #[test]
    fn nested_display_order_keeps_sibling_order_and_detaches_cycles() {
        let mut state = PresetUi::default();
        let names = names(&["root-b", "child", "root-a", "cycle-a", "cycle-b"]);
        state.group_order.insert("panel".into(), names.clone());
        state.group_parent.insert(
            "panel".into(),
            [("child".into(), "root-a".into()), ("cycle-a".into(), "cycle-b".into()), ("cycle-b".into(), "cycle-a".into())].into_iter().collect(),
        );

        assert_eq!(state.nested_display_order("panel", &names), vec![(0, 0), (2, 0), (1, 1), (3, 0), (4, 0)]);
    }

    #[test]
    fn legacy_group_item_anchors_migrate_to_unified_order_and_rename_with_groups() {
        let legacy = serde_json::json!({
            "groupParent": {"panel": {"Child": "Parent", "Grandchild": "Child"}},
            "groupItemAnchors": {"panel": {
                "Child": ["Parent", "stable-item-key"],
                "Grandchild": ["Child", null]
            }}
        });
        let mut restored: PresetUi = serde_json::from_value(legacy).unwrap();
        let parent_order = restored.ordered_hierarchy_entries("panel", "Parent", &names(&["stable-item-key"]), &names(&["Child"]));
        let child_order = restored.ordered_hierarchy_entries("panel", "Child", &[], &names(&["Grandchild"]));
        assert_eq!(parent_order, [HierarchyOrderEntry::Group("Child".into()), HierarchyOrderEntry::Item("stable-item-key".into())]);
        assert_eq!(child_order, [HierarchyOrderEntry::Group("Grandchild".into())]);

        restored.set_hierarchy_order("panel", "Parent", parent_order);
        restored.set_hierarchy_order("panel", "Child", child_order);
        let saved = serde_json::to_value(&restored).unwrap();
        assert!(saved.get("groupItemAnchors").is_none(), "migrated parents no longer save the split anchor representation");
        assert!(saved["hierarchyOrder"]["panel"]["Parent"].is_array());

        restored.rename_group("panel", "Parent", "Renamed parent");
        restored.rename_group("panel", "Child", "Renamed child");
        assert_eq!(restored.hierarchy_order["panel"]["Renamed parent"][0], HierarchyOrderEntry::Group("Renamed child".into()));
        assert_eq!(restored.hierarchy_order["panel"]["Renamed child"][0], HierarchyOrderEntry::Group("Grandchild".into()));

        restored.delete_group("panel", "Renamed child");
        assert!(!restored.hierarchy_order["panel"].contains_key("Renamed child"));
        assert!(!restored.hierarchy_order["panel"]["Renamed parent"].contains(&HierarchyOrderEntry::Group("Renamed child".into())));
    }

    #[test]
    fn unified_order_repositions_cards_and_groups_and_discards_stale_tokens() {
        let mut state = PresetUi::default();
        state.hierarchy_order.insert(
            "panel".into(),
            [(
                "Parent".into(),
                vec![
                    HierarchyOrderEntry::Group("Stale".into()),
                    HierarchyOrderEntry::Item("b".into()),
                    HierarchyOrderEntry::Group("Child".into()),
                    HierarchyOrderEntry::Item("gone".into()),
                    HierarchyOrderEntry::Item("a".into()),
                ],
            )]
            .into(),
        );
        let current = state.ordered_hierarchy_entries("panel", "Parent", &names(&["a", "b"]), &names(&["Child"]));
        assert_eq!(current, [HierarchyOrderEntry::Item("b".into()), HierarchyOrderEntry::Group("Child".into()), HierarchyOrderEntry::Item("a".into())]);

        state.move_hierarchy_entry("panel", "Parent", current, HierarchyOrderEntry::Group("Child".into()), Some(&HierarchyOrderEntry::Item("b".into())));
        assert_eq!(
            state.hierarchy_order["panel"]["Parent"],
            [HierarchyOrderEntry::Group("Child".into()), HierarchyOrderEntry::Item("b".into()), HierarchyOrderEntry::Item("a".into())]
        );
    }

    #[test]
    fn rejected_group_cycle_does_not_create_or_persist_an_empty_hierarchy() {
        let names = names(&["A", "B"]);
        let mut state = PresetUi::default();
        state.nest_group("panel", &names, "A", Some("B"));
        state.nest_group("panel", &names, "B", Some("A"));

        assert_eq!(state.group_parent["panel"]["A"], "B");
        assert!(!state.group_parent["panel"].contains_key("B"));
    }
}
