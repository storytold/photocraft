//! Persistent and transient state shared by preset pickers and floating preset panels.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

pub(crate) use super::picker_browser_data::HierarchyOrderEntry;
use super::picker_browser_data::PickerBrowserData;
pub(crate) use super::picker_browser_model::MAX_GROUP_DEPTH;

/// Transient rename target for a nested group. Kept separate from the legacy string-keyed rename
/// state so arbitrary group labels remain lossless.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct NestedGroupRenameState {
    pub(crate) panel: String,
    pub(crate) group: String,
    pub(crate) group_path: Vec<String>,
    pub(crate) text: String,
}

/// Transient item rename target. Item keys are only unique inside their owning domain group.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ItemRenameState {
    pub(crate) panel: String,
    pub(crate) group: String,
    pub(crate) group_path: Vec<String>,
    pub(crate) key: String,
    pub(crate) text: String,
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
    /// Brush-picker card presentation. Rendering remains brush-adapter-specific; its preferences
    /// live with the common picker state so the browser has one persistence owner.
    #[serde(default, rename = "brushPicker", skip_serializing_if = "BrushPickerView::is_default")]
    pub brush_picker: BrushPickerView,
    /// Thumbnail size per panel (px; the minimum shows a list).
    pub sizes: BTreeMap<String, f32>,
    /// Selected preset per panel (by name / pattern id).
    pub selected: BTreeMap<String, String>,
    /// Owning root group for selections made by the shared browser. Missing entries preserve
    /// legacy key-only selection behavior for older persisted state and non-browser callers.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub selected_group: BTreeMap<String, String>,
    /// Nested folder path for a browser selection. Missing entries preserve legacy key-only state.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub selected_group_path: BTreeMap<String, Vec<String>>,
    /// Multi-selection owned by the shared browser. Transient because domain adapters still own
    /// the active preset used by commands; these identities describe only the visible UI selection.
    #[serde(skip)]
    pub(crate) browser_selection: BTreeMap<String, std::collections::BTreeSet<crate::pickers::picker_browser_model::NodeId>>,
    #[serde(skip)]
    pub(crate) browser_selection_anchor: BTreeMap<String, crate::pickers::picker_browser_model::NodeId>,
    /// Tool Presets: Current Tool Only.
    pub current_tool_only: bool,
    /// Custom Shape tool: the shape it draws.
    pub custom_shape: String,
    /// Ordered group/card membership, parent relationships and group-open state. Flattened to
    /// preserve the pre-existing top-level serialized `PresetUi` fields.
    #[serde(flatten)]
    pub(crate) layout: PickerBrowserData,
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
    /// Typed item rename target; the legacy tuple above remains for external/legacy callers.
    #[serde(skip)]
    pub(crate) item_renaming: Option<ItemRenameState>,
    /// Remaining targets for a sequential multi-item rename. The browser edits one target at a time.
    #[serde(skip)]
    pub(crate) item_rename_queue: Vec<ItemRenameState>,
}

/// Brush adapter presentation options stored alongside the shared asset-browser state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BrushPickerView {
    pub show_name: bool,
    pub show_stroke: bool,
    pub show_tip: bool,
    pub scale: f32,
}

impl Default for BrushPickerView {
    fn default() -> Self {
        Self { show_name: true, show_stroke: true, show_tip: true, scale: 1.0 }
    }
}

impl BrushPickerView {
    fn is_default(&self) -> bool {
        self == &Self::default()
    }
}

/// Preferences key for the shared picker-browser state. The browser hierarchy lives in `UiState`
/// for the control channel, but that state is not itself part of the preferences file.
const PERSISTED_PICKER_UI: &str = "pickerBrowserUi";

/// Save the serializable picker-browser state after pointer gestures finish. This covers every
/// adapter using the shared browser, including reorder and nesting operations.
pub(crate) fn persist(app: &mut crate::PhotocraftApp, ctx: &egui::Context) {
    if ctx.input(|input| input.pointer.any_down()) {
        return;
    }
    let Ok(now) = serde_json::to_value(&app.ui.presets_ui) else { return };
    if app.session.prefs().dialogs.get(PERSISTED_PICKER_UI) != Some(&now) {
        app.session.prefs.edit(|prefs| {
            prefs.dialogs.insert(PERSISTED_PICKER_UI.into(), now);
        });
    }
}

/// Restore the shared picker-browser state from preferences. Transient rename editors remain
/// excluded by `PresetUi`'s serde contract.
pub(crate) fn restore(app: &mut crate::PhotocraftApp) {
    let mut has_saved_brush_view = false;
    if let Some(saved) = app.session.prefs().dialogs.get(PERSISTED_PICKER_UI)
        && let Ok(state) = serde_json::from_value(saved.clone())
    {
        has_saved_brush_view = saved.get("brushPicker").is_some();
        app.ui.presets_ui = state;
    }

    // Migrate the former standalone brush-picker preference into the common picker state once.
    let legacy = app.session.prefs().brush_picker.clone();
    if legacy.is_object() {
        if !has_saved_brush_view {
            let mut view = BrushPickerView::default();
            if let Some(value) = legacy.get("showName").and_then(serde_json::Value::as_bool) {
                view.show_name = value;
            }
            if let Some(value) = legacy.get("showStroke").and_then(serde_json::Value::as_bool) {
                view.show_stroke = value;
            }
            if let Some(value) = legacy.get("showTip").and_then(serde_json::Value::as_bool) {
                view.show_tip = value;
            }
            if let Some(value) = legacy.get("scale").and_then(serde_json::Value::as_f64)
                && value.is_finite()
            {
                view.scale = value as f32;
            }
            app.ui.presets_ui.brush_picker = view;
        }
        app.session.prefs.edit(|prefs| prefs.brush_picker = serde_json::Value::Null);
    }
    let view = &mut app.ui.presets_ui.brush_picker;
    view.scale = if view.scale.is_finite() { view.scale.clamp(0.15, 2.0) } else { 1.0 };
    if ![view.show_name, view.show_stroke, view.show_tip].into_iter().any(|shown| shown) {
        view.show_tip = true;
    }
}

impl std::ops::Deref for PresetUi {
    type Target = PickerBrowserData;

    fn deref(&self) -> &Self::Target {
        &self.layout
    }
}

impl std::ops::DerefMut for PresetUi {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.layout
    }
}

impl PresetUi {
    pub(crate) fn browser_node_selected(&self, panel: &str, id: &crate::pickers::picker_browser_model::NodeId) -> bool {
        self.browser_selection.get(panel).is_some_and(|selection| selection.contains(id))
    }

    pub(crate) fn select_browser_node(&mut self, panel: &str, id: crate::pickers::picker_browser_model::NodeId, additive: bool) {
        let selection = self.browser_selection.entry(panel.to_owned()).or_default();
        if !additive {
            selection.clear();
        }
        if additive && !selection.insert(id.clone()) {
            selection.remove(&id);
        } else if !additive {
            selection.insert(id.clone());
        }
        if !additive {
            self.browser_selection_anchor.insert(panel.to_owned(), id);
        }
    }

    pub(crate) fn select_browser_range(
        &mut self,
        panel: &str,
        visible: &[crate::pickers::picker_browser_model::NodeId],
        target: &crate::pickers::picker_browser_model::NodeId,
    ) {
        let Some(anchor) = self.browser_selection_anchor.get(panel) else {
            self.select_browser_node(panel, target.clone(), false);
            return;
        };
        let (Some(start), Some(end)) = (visible.iter().position(|id| id == anchor), visible.iter().position(|id| id == target)) else {
            self.select_browser_node(panel, target.clone(), false);
            return;
        };
        let selection = self.browser_selection.entry(panel.to_owned()).or_default();
        selection.clear();
        selection.extend(visible[start.min(end)..=start.max(end)].iter().cloned());
    }

    pub(crate) fn set_selected(&mut self, panel: &str, key: &str) {
        self.selected.insert(panel.to_string(), key.to_string());
        self.selected_group.remove(panel);
        self.selected_group_path.remove(panel);
    }

    pub(crate) fn set_selected_item(&mut self, panel: &str, group: &str, group_path: &[String], key: &str) {
        self.selected.insert(panel.to_string(), key.to_string());
        self.selected_group.insert(panel.to_string(), group.to_string());
        self.selected_group_path.insert(panel.to_string(), group_path.to_vec());
    }

    pub(crate) fn is_selected_item(&self, panel: &str, group: &str, group_path: &[String], key: &str) -> bool {
        self.selected.get(panel).is_some_and(|selected| selected == key)
            && match (self.selected_group.get(panel), self.selected_group_path.get(panel)) {
                (Some(selected_group), Some(selected_path)) => selected_group == group && selected_path == group_path,
                (None, None) => true,
                // A legacy group-only selection does not identify a nested folder safely.
                _ => false,
            }
    }

    pub(crate) fn clear_selected(&mut self, panel: &str) {
        self.selected.remove(panel);
        self.selected_group.remove(panel);
        self.selected_group_path.remove(panel);
    }

    pub fn shape(&self) -> &str {
        if self.custom_shape.is_empty() { tl!("Heart") } else { &self.custom_shape }
    }

    /// Display order of the root groups for `panel`, as library indices. Legacy `groupOrder` is
    /// normalized into the canonical root sequence during deserialization.
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
        for entry in self.hierarchy_order.get(panel).and_then(|parents| parents.get("")).into_iter().flatten() {
            let HierarchyOrderEntry::Group(name) = entry else { continue };
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
    /// Legacy anchors and split parent/order data have already been normalized by PickerBrowserData's
    /// deserializer, so this method only repairs stale/partial membership against live data.
    pub(crate) fn ordered_hierarchy_entries(&self, panel: &str, parent: &str, item_keys: &[String], child_groups: &[String]) -> Vec<HierarchyOrderEntry> {
        let mut fallback = Vec::with_capacity(item_keys.len() + child_groups.len());
        fallback.extend(item_keys.iter().cloned().map(HierarchyOrderEntry::Item));
        fallback.extend(child_groups.iter().cloned().map(HierarchyOrderEntry::Group));
        let Some(saved) = self.hierarchy_order.get(panel).and_then(|parents| parents.get(parent)) else { return fallback };
        let valid_items: BTreeSet<String> = item_keys.iter().cloned().collect();
        let valid_groups: BTreeSet<String> = child_groups.iter().cloned().collect();
        let mut seen_items = BTreeSet::new();
        let mut seen_root_items = BTreeSet::new();
        let mut seen_groups = BTreeSet::new();
        let mut out = Vec::with_capacity(fallback.len());
        for entry in saved {
            let valid = match entry {
                HierarchyOrderEntry::Item(key) => valid_items.contains(key) && seen_items.insert(key.clone()),
                HierarchyOrderEntry::RootItem { group, key } if parent.is_empty() => seen_root_items.insert((group.clone(), key.clone())),
                HierarchyOrderEntry::RootItem { .. } => false,
                HierarchyOrderEntry::Group(key) => valid_groups.contains(key) && seen_groups.insert(key.clone()),
            };
            if valid {
                out.push(entry.clone());
            }
        }
        for entry in fallback {
            let missing = match &entry {
                HierarchyOrderEntry::Item(key) => seen_items.insert(key.clone()),
                HierarchyOrderEntry::RootItem { .. } => false,
                HierarchyOrderEntry::Group(key) => seen_groups.insert(key.clone()),
            };
            if missing {
                out.push(entry);
            }
        }
        out
    }

    /// Persist a complete canonical sibling sequence.
    pub(crate) fn set_hierarchy_order(&mut self, panel: &str, parent: &str, entries: Vec<HierarchyOrderEntry>) {
        let entries =
            if parent.is_empty() { entries } else { entries.into_iter().filter(|entry| !matches!(entry, HierarchyOrderEntry::RootItem { .. })).collect() };
        self.hierarchy_order.entry(panel.to_string()).or_default().insert(parent.to_string(), entries);
    }

    /// Remove a child token from a parent's sibling sequence after it changes parent.
    pub(crate) fn remove_hierarchy_entry(&mut self, panel: &str, parent: &str, entry: &HierarchyOrderEntry) {
        if let Some(entries) = self.hierarchy_order.get_mut(panel).and_then(|parents| parents.get_mut(parent)) {
            entries.retain(|candidate| candidate != entry);
            if entries.is_empty()
                && let Some(parents) = self.hierarchy_order.get_mut(panel)
            {
                parents.remove(parent);
                if parents.is_empty() {
                    self.hierarchy_order.remove(panel);
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
        if !parent.is_empty() && matches!(&moving, HierarchyOrderEntry::RootItem { .. }) {
            return;
        }
        if before == Some(&moving) {
            return;
        }
        entries.retain(|entry| entry != &moving);
        let at = before.and_then(|before| entries.iter().position(|entry| entry == before)).unwrap_or(entries.len());
        entries.insert(at, moving);
        self.set_hierarchy_order(panel, parent, entries);
    }

    /// Keep legacy unqualified Item identities attached to their sibling position after a rename.
    /// RootItem identities require the source-group-aware helper below.
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

    /// Rename one item identity without touching same-key cards in sibling domain groups.
    pub(crate) fn rename_hierarchy_item_in_group(&mut self, panel: &str, group: &str, from: &str, to: &str) {
        if from == to {
            return;
        }
        let parents = self.group_parents(panel);
        let belongs_to_group = |parent: &str| {
            if parent.is_empty() {
                return false;
            }
            let mut current = parent;
            let mut seen = BTreeSet::new();
            loop {
                if current == group {
                    return true;
                }
                if !seen.insert(current.to_string()) {
                    return false;
                }
                let Some(next) = parents.get(current) else { return false };
                current = next;
            }
        };
        if let Some(orders) = self.hierarchy_order.get_mut(panel) {
            for (parent, entries) in orders.iter_mut() {
                if belongs_to_group(parent) {
                    for entry in entries.iter_mut() {
                        if let HierarchyOrderEntry::Item(key) = entry
                            && key == from
                        {
                            *key = to.to_string();
                        }
                    }
                }
                if parent.is_empty() {
                    for entry in entries.iter_mut() {
                        if let HierarchyOrderEntry::RootItem { group: owner, key } = entry
                            && owner == group
                            && key == from
                        {
                            *key = to.to_string();
                        }
                    }
                }
            }
        }
    }

    /// Rename exactly one item occurrence. Nested item tokens live in the immediate parent
    /// sequence, so compare that parent's complete ancestry with the supplied folder path.
    pub(crate) fn rename_hierarchy_item_at_path(&mut self, panel: &str, group: &str, group_path: &[String], from: &str, to: &str) {
        if from == to {
            return;
        }
        let parents = self.group_parents(panel);
        let parent_matches = |parent: &str| {
            if group_path.is_empty() {
                return parent == group;
            }
            let mut path = Vec::new();
            let mut current = parent;
            let mut seen = BTreeSet::new();
            while current != group {
                if !seen.insert(current.to_string()) {
                    return false;
                }
                path.push(current.to_string());
                let Some(next) = parents.get(current) else {
                    return false;
                };
                current = next;
            }
            path.reverse();
            path.as_slice() == group_path
        };
        if let Some(orders) = self.hierarchy_order.get_mut(panel) {
            for (parent, entries) in orders.iter_mut() {
                if parent_matches(parent) {
                    for entry in entries.iter_mut() {
                        if let HierarchyOrderEntry::Item(key) = entry
                            && key == from
                        {
                            *key = to.to_string();
                        }
                    }
                }
                if group_path.is_empty() && parent.is_empty() {
                    for entry in entries.iter_mut() {
                        if let HierarchyOrderEntry::RootItem { group: owner, key } = entry
                            && owner == group
                            && key == from
                        {
                            *key = to.to_string();
                        }
                    }
                }
            }
        }
    }

    /// Remove a legacy unqualified Item from sibling sequences in the panel. RootItem identities
    /// require the source-group-aware helper below.
    #[cfg(test)]
    pub(crate) fn delete_hierarchy_item(&mut self, panel: &str, key: &str) {
        if let Some(orders) = self.hierarchy_order.get_mut(panel) {
            for entries in orders.values_mut() {
                entries.retain(|entry| match entry {
                    HierarchyOrderEntry::Item(item) => item != key,
                    // Lifted root cards are qualified by their source group. Only the scoped
                    // mutation below may rename/delete those identities.
                    HierarchyOrderEntry::RootItem { .. } => true,
                    HierarchyOrderEntry::Group(_) => true,
                });
            }
            orders.retain(|_, entries| !entries.is_empty());
            if orders.is_empty() {
                self.hierarchy_order.remove(panel);
            }
        }
    }

    /// Delete one item identity without removing same-key cards from other domain groups.
    pub(crate) fn delete_hierarchy_item_in_group(&mut self, panel: &str, group: &str, key: &str) {
        let parents = self.group_parents(panel);
        let belongs_to_group = |parent: &str| {
            if parent.is_empty() {
                return false;
            }
            let mut current = parent;
            let mut seen = BTreeSet::new();
            loop {
                if current == group {
                    return true;
                }
                if !seen.insert(current.to_string()) {
                    return false;
                }
                let Some(next) = parents.get(current) else { return false };
                current = next;
            }
        };
        if let Some(orders) = self.hierarchy_order.get_mut(panel) {
            for (parent, entries) in orders.iter_mut() {
                if belongs_to_group(parent) {
                    entries.retain(|entry| !matches!(entry, HierarchyOrderEntry::Item(item) if item == key));
                }
                if parent.is_empty() {
                    entries.retain(|entry| !matches!(entry, HierarchyOrderEntry::RootItem { group: owner, key: item } if owner == group && item == key));
                }
            }
            orders.retain(|_, entries| !entries.is_empty());
            if orders.is_empty() {
                self.hierarchy_order.remove(panel);
            }
        }
    }

    /// Delete exactly one item occurrence, leaving same-key items in other folder paths intact.
    pub(crate) fn delete_hierarchy_item_at_path(&mut self, panel: &str, group: &str, group_path: &[String], key: &str) {
        let parents = self.group_parents(panel);
        let parent_matches = |parent: &str| {
            if group_path.is_empty() {
                return parent == group;
            }
            let mut path = Vec::new();
            let mut current = parent;
            let mut seen = BTreeSet::new();
            while current != group {
                if !seen.insert(current.to_string()) {
                    return false;
                }
                path.push(current.to_string());
                let Some(next) = parents.get(current) else {
                    return false;
                };
                current = next;
            }
            path.reverse();
            path.as_slice() == group_path
        };
        if let Some(orders) = self.hierarchy_order.get_mut(panel) {
            for (parent, entries) in orders.iter_mut() {
                if parent_matches(parent) {
                    entries.retain(|entry| !matches!(entry, HierarchyOrderEntry::Item(item) if item == key));
                }
                if group_path.is_empty() && parent.is_empty() {
                    entries.retain(|entry| !matches!(entry, HierarchyOrderEntry::RootItem { group: owner, key: item } if owner == group && item == key));
                }
            }
            orders.retain(|_, entries| !entries.is_empty());
            if orders.is_empty() {
                self.hierarchy_order.remove(panel);
            }
        }
    }

    /// Move a root group before `before` (`None` = last) in the canonical root sibling sequence.
    pub(crate) fn move_group(&mut self, panel: &str, names: &[String], from: &str, before: Option<&str>) {
        if before == Some(from) {
            return;
        }
        let valid_names: BTreeSet<&str> = names.iter().map(String::as_str).collect();
        let parents = self.group_parents(panel);
        let root_names: Vec<String> =
            names.iter().filter(|name| parents.get(*name).is_none_or(|parent| !valid_names.contains(parent.as_str()))).cloned().collect();
        for name in &root_names {
            if let Some(parent) = parents.get(name).filter(|parent| !valid_names.contains(parent.as_str())) {
                self.remove_hierarchy_entry(panel, parent, &HierarchyOrderEntry::Group(name.clone()));
            }
        }
        let mut entries = self.hierarchy_order.get(panel).and_then(|orders| orders.get("")).cloned().unwrap_or_default();
        entries.retain(|entry| match entry {
            HierarchyOrderEntry::Group(name) => root_names.iter().any(|root| root == name),
            HierarchyOrderEntry::Item(_) | HierarchyOrderEntry::RootItem { .. } => true,
        });
        for name in self.display_order(panel, &root_names).into_iter().filter_map(|index| root_names.get(index)) {
            let entry = HierarchyOrderEntry::Group(name.clone());
            if !entries.contains(&entry) {
                entries.push(entry);
            }
        }
        let moving = HierarchyOrderEntry::Group(from.to_string());
        if !entries.contains(&moving) {
            return;
        }
        entries.retain(|entry| entry != &moving);
        let at = before.and_then(|name| entries.iter().position(|entry| entry == &HierarchyOrderEntry::Group(name.to_string()))).unwrap_or(entries.len());
        entries.insert(at, moving);
        let has_root_items = entries.iter().any(|entry| matches!(entry, HierarchyOrderEntry::Item(_) | HierarchyOrderEntry::RootItem { .. }));
        let group_order: Vec<&str> =
            entries.iter().filter_map(|entry| if let HierarchyOrderEntry::Group(name) = entry { Some(name.as_str()) } else { None }).collect();
        if !has_root_items && group_order.iter().copied().eq(root_names.iter().map(String::as_str)) {
            if let Some(orders) = self.hierarchy_order.get_mut(panel) {
                orders.remove("");
                if orders.is_empty() {
                    self.hierarchy_order.remove(panel);
                }
            }
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
        let parents = self.group_parents(panel);
        let mut ancestor = parent;
        let mut seen_ancestors = BTreeSet::new();
        while let Some(name) = ancestor {
            if name == child {
                return;
            }
            if !seen_ancestors.insert(name.to_string()) {
                return;
            }
            ancestor = parents.get(name).map(String::as_str);
        }
        let old_parent = parents.get(child).map(String::as_str).unwrap_or("").to_string();
        let new_parent = parent.unwrap_or("");
        if old_parent == new_parent {
            return;
        }
        let moving = HierarchyOrderEntry::Group(child.to_string());
        self.remove_hierarchy_entry(panel, &old_parent, &moving);
        let mut destination = self.hierarchy_order.get(panel).and_then(|orders| orders.get(new_parent)).cloned().unwrap_or_default();
        if !destination.contains(&moving) {
            destination.push(moving);
        }
        self.set_hierarchy_order(panel, new_parent, destination);
    }

    /// Keep view-only hierarchy and expansion state attached to a group when its asset-library
    /// name changes. The caller invokes this only after the asset rename command succeeds.
    pub(crate) fn rename_group(&mut self, panel: &str, from: &str, to: &str) {
        if from == to {
            return;
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
                    if let HierarchyOrderEntry::RootItem { group, .. } = entry
                        && group == from
                    {
                        *group = to.to_string();
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
        if let Some(orders) = self.hierarchy_order.get_mut(panel) {
            let promoted: Vec<HierarchyOrderEntry> =
                orders.get(name).into_iter().flatten().filter(|entry| matches!(entry, HierarchyOrderEntry::Group(_))).cloned().collect();
            orders.remove(name);
            for entries in orders.values_mut() {
                entries.retain(|entry| match entry {
                    HierarchyOrderEntry::Group(group) => group != name,
                    HierarchyOrderEntry::RootItem { group, .. } => group != name,
                    HierarchyOrderEntry::Item(_) => true,
                });
            }
            if !promoted.is_empty() {
                let root = orders.entry(String::new()).or_default();
                for entry in promoted {
                    if !root.contains(&entry) {
                        root.push(entry);
                    }
                }
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
    #[cfg(test)]
    pub(crate) fn drop_group_on_header(&mut self, panel: &str, names: &[String], child: &str, target: &str, edge: bool) {
        if child == target || !names.iter().any(|n| n == child) || !names.iter().any(|n| n == target) {
            return;
        }
        if !edge {
            self.nest_group(panel, names, child, Some(target));
            return;
        }
        let parents = self.group_parents(panel);
        let valid_names: BTreeSet<&str> = names.iter().map(String::as_str).collect();
        let target_parent = parents.get(target).filter(|parent| valid_names.contains(parent.as_str())).cloned();
        let mut ancestor = target_parent.as_deref();
        let mut seen_ancestors = BTreeSet::new();
        while let Some(name) = ancestor {
            if name == child {
                return;
            }
            if !seen_ancestors.insert(name.to_string()) {
                return;
            }
            ancestor = parents.get(name).map(String::as_str);
        }
        let destination_parent = target_parent.clone().unwrap_or_default();
        self.nest_group(panel, names, child, target_parent.as_deref());
        let parents = self.group_parents(panel);
        let siblings: Vec<String> = names
            .iter()
            .filter(|name| parents.get(*name).map(String::as_str).filter(|parent| valid_names.contains(parent)).unwrap_or("") == destination_parent)
            .cloned()
            .collect();
        let item_keys: Vec<String> = self
            .hierarchy_order
            .get(panel)
            .and_then(|parents| parents.get(&destination_parent))
            .into_iter()
            .flatten()
            .filter_map(|entry| if let HierarchyOrderEntry::Item(key) = entry { Some(key.clone()) } else { None })
            .collect();
        let entries = self.ordered_hierarchy_entries(panel, &destination_parent, &item_keys, &siblings);
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
        let parents = self.group_parents(panel);
        // Bucket children by parent in display order. The old recursive implementation scanned
        // the complete ordered list for every visited group, quadratic for a deep/large hierarchy.
        let mut children: BTreeMap<Option<&str>, Vec<usize>> = BTreeMap::new();
        for &i in &ordered {
            let Some(name) = names.get(i) else { continue };
            let parent = parents.get(name).map(String::as_str).filter(|p| valid_names.contains(p));
            children.entry(parent).or_default().push(i);
        }
        // The unified sibling sequence is authoritative at every depth; library order is only a
        // fallback for groups without saved membership/order.
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

    #[test]
    fn picker_hierarchy_order_survives_preferences_restart() {
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.ui.presets_ui.set_hierarchy_order("swatches", "", vec![HierarchyOrderEntry::Group("Second".into()), HierarchyOrderEntry::Group("First".into())]);
        app.ui.presets_ui.set_hierarchy_order("swatches", "Second", vec![HierarchyOrderEntry::Item("Blue".into()), HierarchyOrderEntry::Item("Red".into())]);

        super::persist(&mut app, &egui::Context::default());
        let saved = app.session.prefs_to_json();
        let mut restarted = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        restarted.session.load_prefs_json(&saved).unwrap();
        super::restore(&mut restarted);

        assert_eq!(restarted.ui.presets_ui.hierarchy_order, app.ui.presets_ui.hierarchy_order);
    }

    fn names(values: &[&str]) -> Vec<String> {
        values.iter().map(|name| (*name).to_string()).collect()
    }

    #[test]
    fn browser_selection_is_qualified_by_root_group_and_legacy_selection_is_supported() {
        let mut state = PresetUi::default();
        state.set_selected_item("panel", "Group A", &[], "Shared key");
        assert!(state.is_selected_item("panel", "Group A", &[], "Shared key"));
        assert!(!state.is_selected_item("panel", "Group B", &[], "Shared key"));

        state.set_selected("panel", "Shared key");
        assert!(state.is_selected_item("panel", "Group A", &[], "Shared key"));
        assert!(state.is_selected_item("panel", "Group B", &[], "Shared key"));
        state.clear_selected("panel");
        assert!(!state.is_selected_item("panel", "Group A", &[], "Shared key"));
    }

    #[test]
    fn browser_selection_distinguishes_same_key_in_nested_paths() {
        let mut state = PresetUi::default();
        state.set_selected_item("panel", "Root", &["Folder A".into()], "same");
        assert!(state.is_selected_item("panel", "Root", &["Folder A".into()], "same"));
        assert!(!state.is_selected_item("panel", "Root", &["Folder B".into()], "same"));
        state.set_selected("panel", "same");
        assert!(!state.selected_group_path.contains_key("panel"));
    }

    #[test]
    fn browser_multi_selection_toggles_nodes_and_shift_selects_the_visible_interval() {
        use crate::pickers::picker_browser_model::NodeId;

        let first = NodeId::group("First");
        let card_a = NodeId::item("First", "a");
        let collapsed = NodeId::group("Collapsed");
        let hidden_card = NodeId::item("Collapsed", "hidden");
        let card_b = NodeId::item("Last", "b");
        let visible = vec![first.clone(), card_a.clone(), collapsed.clone(), card_b.clone()];
        let mut state = PresetUi::default();

        state.select_browser_node("panel", first.clone(), false);
        state.select_browser_node("panel", card_a.clone(), true);
        assert!(state.browser_node_selected("panel", &first));
        assert!(state.browser_node_selected("panel", &card_a));

        state.select_browser_range("panel", &visible, &card_b);
        assert!(visible.iter().all(|id| state.browser_node_selected("panel", id)));
        assert!(!state.browser_node_selected("panel", &hidden_card));

        state.select_browser_node("panel", card_a.clone(), true);
        assert!(!state.browser_node_selected("panel", &card_a));
        assert!(state.browser_node_selected("panel", &first));
    }

    #[test]
    fn exact_path_item_tokens_do_not_mutate_same_key_in_other_groups_or_folders() {
        let mut state = PresetUi::default();
        state.set_hierarchy_order(
            "panel",
            "",
            vec![
                HierarchyOrderEntry::Group("Root".into()),
                HierarchyOrderEntry::RootItem { group: "Root".into(), key: "same".into() },
                HierarchyOrderEntry::RootItem { group: "Other".into(), key: "same".into() },
            ],
        );
        state.set_hierarchy_order(
            "panel",
            "Root",
            vec![HierarchyOrderEntry::Group("Folder A".into()), HierarchyOrderEntry::Group("Folder B".into()), HierarchyOrderEntry::Item("same".into())],
        );
        state.set_hierarchy_order("panel", "Folder A", vec![HierarchyOrderEntry::Item("same".into())]);
        state.set_hierarchy_order("panel", "Folder B", vec![HierarchyOrderEntry::Item("same".into())]);

        state.rename_hierarchy_item_at_path("panel", "Root", &["Folder A".into()], "same", "renamed");
        assert_eq!(state.hierarchy_order["panel"]["Folder A"], [HierarchyOrderEntry::Item("renamed".into())]);
        assert_eq!(state.hierarchy_order["panel"]["Folder B"], [HierarchyOrderEntry::Item("same".into())]);
        assert!(state.hierarchy_order["panel"][""].contains(&HierarchyOrderEntry::RootItem { group: "Root".into(), key: "same".into() }));
        assert!(state.hierarchy_order["panel"][""].contains(&HierarchyOrderEntry::RootItem { group: "Other".into(), key: "same".into() }));

        state.delete_hierarchy_item_at_path("panel", "Root", &["Folder B".into()], "same");
        assert!(!state.hierarchy_order["panel"].contains_key("Folder B"));
        assert_eq!(state.hierarchy_order["panel"]["Folder A"], [HierarchyOrderEntry::Item("renamed".into())]);

        state.rename_hierarchy_item_at_path("panel", "Root", &[], "same", "root-renamed");
        assert!(state.hierarchy_order["panel"][""].contains(&HierarchyOrderEntry::RootItem { group: "Root".into(), key: "root-renamed".into() }));
        assert!(state.hierarchy_order["panel"][""].contains(&HierarchyOrderEntry::RootItem { group: "Other".into(), key: "same".into() }));
    }

    #[test]
    fn display_order_keeps_saved_order_and_appends_unlisted_names() {
        let state: PresetUi = serde_json::from_value(serde_json::json!({
            "groupOrder": {"panel": ["gone", "third", "third", "first"]}
        }))
        .unwrap();

        assert_eq!(state.display_order("panel", &names(&["first", "second", "third"])), vec![2, 0, 1]);
    }

    #[test]
    fn preset_ui_flatten_keeps_legacy_layout_keys_compatible() {
        let restored: PresetUi = serde_json::from_value(serde_json::json!({
            "sizes": {"panel": 42.0},
            "toggled": ["panel/Parent"],
            "groupOrder": {"panel": ["Parent", "Child"]},
            "groupParent": {"panel": {"Child": "Parent"}}
        }))
        .unwrap();
        assert_eq!(restored.parent_of("panel", "Child"), Some("Parent"));
        assert_eq!(restored.sizes["panel"], 42.0);

        let saved = serde_json::to_value(restored).unwrap();
        assert_eq!(saved["groupParent"]["panel"]["Child"], "Parent");
        assert!(saved["hierarchyOrder"]["panel"][""].is_array());
        assert!(saved.get("groupOrder").is_none());
        assert_eq!(saved["toggled"][0], "panel/Parent");
        assert_eq!(saved["sizes"]["panel"], 42.0);
    }

    #[test]
    fn nested_display_order_keeps_sibling_order_and_detaches_cycles() {
        let state: PresetUi = serde_json::from_value(serde_json::json!({
            "groupOrder": {"panel": ["root-b", "child", "root-a", "cycle-a", "cycle-b"]},
            "groupParent": {"panel": {"child": "root-a", "cycle-a": "cycle-b", "cycle-b": "cycle-a"}}
        }))
        .unwrap();
        let names = names(&["root-b", "child", "root-a", "cycle-a", "cycle-b"]);

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
    fn moving_root_groups_preserves_root_card_tokens_in_the_mixed_sequence() {
        let mut state = PresetUi::default();
        state.set_hierarchy_order(
            "panel",
            "",
            vec![
                HierarchyOrderEntry::Group("A".into()),
                HierarchyOrderEntry::RootItem { group: "Original".into(), key: "root-card".into() },
                HierarchyOrderEntry::Group("B".into()),
            ],
        );

        state.move_group("panel", &names(&["A", "B"]), "B", Some("A"));
        assert_eq!(
            state.hierarchy_order["panel"][""],
            [
                HierarchyOrderEntry::Group("B".into()),
                HierarchyOrderEntry::Group("A".into()),
                HierarchyOrderEntry::RootItem { group: "Original".into(), key: "root-card".into() }
            ]
        );
    }

    #[test]
    fn moving_back_to_default_root_order_elides_only_group_only_sequences() {
        let mut groups_only = PresetUi::default();
        groups_only.set_hierarchy_order("panel", "", vec![HierarchyOrderEntry::Group("B".into()), HierarchyOrderEntry::Group("A".into())]);
        groups_only.move_group("panel", &names(&["A", "B"]), "B", None);
        assert!(!groups_only.hierarchy_order.get("panel").is_some_and(|orders| orders.contains_key("")));

        let mut mixed = PresetUi::default();
        mixed.set_hierarchy_order(
            "panel",
            "",
            vec![
                HierarchyOrderEntry::Group("B".into()),
                HierarchyOrderEntry::RootItem { group: "Original".into(), key: "root-card".into() },
                HierarchyOrderEntry::Group("A".into()),
            ],
        );
        mixed.move_group("panel", &names(&["A", "B"]), "B", None);
        assert_eq!(
            mixed.hierarchy_order["panel"][""],
            [
                HierarchyOrderEntry::RootItem { group: "Original".into(), key: "root-card".into() },
                HierarchyOrderEntry::Group("A".into()),
                HierarchyOrderEntry::Group("B".into())
            ]
        );
    }

    #[test]
    fn root_edge_group_drop_inserts_before_target_and_preserves_root_cards() {
        let mut state = PresetUi::default();
        let names = names(&["A", "B", "C"]);
        state.set_hierarchy_order(
            "panel",
            "",
            vec![
                HierarchyOrderEntry::Group("A".into()),
                HierarchyOrderEntry::RootItem { group: "Original".into(), key: "root-card".into() },
                HierarchyOrderEntry::Group("B".into()),
            ],
        );
        state.nest_group("panel", &names, "C", Some("A"));
        state.drop_group_on_header("panel", &names, "C", "B", true);

        assert_eq!(state.parent_of("panel", "C"), None);
        assert_eq!(
            state.hierarchy_order["panel"][""],
            [
                HierarchyOrderEntry::Group("A".into()),
                HierarchyOrderEntry::RootItem { group: "Original".into(), key: "root-card".into() },
                HierarchyOrderEntry::Group("C".into()),
                HierarchyOrderEntry::Group("B".into())
            ]
        );
    }

    #[test]
    fn malformed_legacy_parent_cycle_rejects_nest_and_header_drop() {
        let names = names(&["A", "B", "C"]);
        let mut state: PresetUi = serde_json::from_value(serde_json::json!({
            "groupOrder": {"panel": ["A", "B", "C"]},
            "groupParent": {"panel": {"A": "B", "B": "A"}}
        }))
        .unwrap();

        state.nest_group("panel", &names, "C", Some("A"));
        state.drop_group_on_header("panel", &names, "C", "A", true);
        assert_eq!(state.parent_of("panel", "C"), None);
    }

    #[test]
    fn deleting_group_promotes_direct_children_to_root_in_their_previous_order() {
        let mut state = PresetUi::default();
        state.set_hierarchy_order("panel", "", vec![HierarchyOrderEntry::Group("A".into()), HierarchyOrderEntry::Group("Sibling".into())]);
        state.set_hierarchy_order(
            "panel",
            "A",
            vec![HierarchyOrderEntry::Group("Child C".into()), HierarchyOrderEntry::Item("card-a".into()), HierarchyOrderEntry::Group("Child B".into())],
        );
        state.set_hierarchy_order("panel", "Child C", vec![HierarchyOrderEntry::Group("Grandchild".into())]);

        state.delete_group("panel", "A");
        assert_eq!(
            state.hierarchy_order["panel"][""],
            [HierarchyOrderEntry::Group("Sibling".into()), HierarchyOrderEntry::Group("Child C".into()), HierarchyOrderEntry::Group("Child B".into())]
        );
        assert_eq!(state.parent_of("panel", "Child C"), None);
        assert_eq!(state.parent_of("panel", "Child B"), None);
        assert_eq!(state.parent_of("panel", "Grandchild"), Some("Child C"));
    }

    #[test]
    fn root_items_survive_root_order_repair_but_never_enter_nested_sequences() {
        let mut state = PresetUi::default();
        let root_item = HierarchyOrderEntry::RootItem { group: "Original".into(), key: "lifted".into() };
        state.set_hierarchy_order("panel", "", vec![root_item.clone(), HierarchyOrderEntry::Group("A".into())]);
        state.hierarchy_order.entry("panel".into()).or_default().insert("A".into(), vec![root_item.clone(), HierarchyOrderEntry::Item("nested".into())]);

        let root = state.ordered_hierarchy_entries("panel", "", &[], &names(&["A"]));
        let nested = state.ordered_hierarchy_entries("panel", "A", &names(&["nested"]), &[]);
        assert_eq!(root, [root_item, HierarchyOrderEntry::Group("A".into())]);
        assert_eq!(nested, [HierarchyOrderEntry::Item("nested".into())]);
    }

    #[test]
    fn item_and_source_group_renames_and_deletes_update_lifted_root_identity() {
        let mut state = PresetUi::default();
        state.set_hierarchy_order("panel", "", vec![HierarchyOrderEntry::RootItem { group: "Original".into(), key: "before".into() }]);

        state.rename_hierarchy_item_in_group("panel", "Original", "before", "after");
        state.rename_group("panel", "Original", "Renamed");
        assert_eq!(state.hierarchy_order["panel"][""], [HierarchyOrderEntry::RootItem { group: "Renamed".into(), key: "after".into() }]);

        state.delete_hierarchy_item_in_group("panel", "Renamed", "after");
        assert!(
            !state.hierarchy_order.get("panel").is_some_and(|parents| parents
                .get("")
                .is_some_and(|entries| entries.iter().any(|entry| matches!(entry, HierarchyOrderEntry::RootItem { .. }))))
        );

        state.set_hierarchy_order(
            "panel",
            "",
            vec![
                HierarchyOrderEntry::RootItem { group: "Renamed".into(), key: "another".into() },
                HierarchyOrderEntry::RootItem { group: "Keep".into(), key: "keep".into() },
            ],
        );
        state.delete_group("panel", "Renamed");
        assert_eq!(state.hierarchy_order["panel"][""], [HierarchyOrderEntry::RootItem { group: "Keep".into(), key: "keep".into() }]);
    }

    #[test]
    fn rejected_group_cycle_does_not_create_or_persist_an_empty_hierarchy() {
        let names = names(&["A", "B"]);
        let mut state = PresetUi::default();
        state.nest_group("panel", &names, "A", Some("B"));
        state.nest_group("panel", &names, "B", Some("A"));

        assert_eq!(state.parent_of("panel", "A"), Some("B"));
        assert_eq!(state.parent_of("panel", "B"), None);
    }

    #[test]
    fn group_scoped_item_rename_and_delete_preserve_duplicate_keys_in_other_groups() {
        let mut state = PresetUi::default();
        state.set_hierarchy_order(
            "panel",
            "",
            vec![
                HierarchyOrderEntry::RootItem { group: "A".into(), key: "same".into() },
                HierarchyOrderEntry::RootItem { group: "B".into(), key: "same".into() },
                HierarchyOrderEntry::Group("A".into()),
                HierarchyOrderEntry::Group("B".into()),
            ],
        );
        state.set_hierarchy_order("panel", "A", vec![HierarchyOrderEntry::Group("A nested".into())]);
        state.set_hierarchy_order("panel", "A nested", vec![HierarchyOrderEntry::Item("same".into())]);
        state.set_hierarchy_order("panel", "B", vec![HierarchyOrderEntry::Item("same".into())]);

        state.rename_hierarchy_item_in_group("panel", "A", "same", "renamed");
        assert_eq!(state.hierarchy_order["panel"]["A nested"], [HierarchyOrderEntry::Item("renamed".into())]);
        assert_eq!(state.hierarchy_order["panel"]["B"], [HierarchyOrderEntry::Item("same".into())]);
        assert!(state.hierarchy_order["panel"][""].contains(&HierarchyOrderEntry::RootItem { group: "A".into(), key: "renamed".into() }));
        assert!(state.hierarchy_order["panel"][""].contains(&HierarchyOrderEntry::RootItem { group: "B".into(), key: "same".into() }));

        state.delete_hierarchy_item_in_group("panel", "A", "renamed");
        assert!(state.hierarchy_order["panel"].get("A nested").is_none_or(Vec::is_empty));
        assert_eq!(state.hierarchy_order["panel"]["B"], [HierarchyOrderEntry::Item("same".into())]);
        assert!(!state.hierarchy_order["panel"][""].contains(&HierarchyOrderEntry::RootItem { group: "A".into(), key: "renamed".into() }));
        assert!(state.hierarchy_order["panel"][""].contains(&HierarchyOrderEntry::RootItem { group: "B".into(), key: "same".into() }));
    }

    #[test]
    fn legacy_key_only_item_mutations_leave_lifted_root_items_untouched() {
        let mut state = PresetUi::default();
        state.set_hierarchy_order(
            "panel",
            "",
            vec![
                HierarchyOrderEntry::RootItem { group: "A".into(), key: "same".into() },
                HierarchyOrderEntry::RootItem { group: "B".into(), key: "same".into() },
            ],
        );

        state.rename_hierarchy_item("panel", "same", "renamed");
        state.delete_hierarchy_item("panel", "same");
        assert_eq!(
            state.hierarchy_order["panel"][""],
            [HierarchyOrderEntry::RootItem { group: "A".into(), key: "same".into() }, HierarchyOrderEntry::RootItem { group: "B".into(), key: "same".into() },]
        );
    }

    #[test]
    fn moving_a_root_card_reorders_the_mixed_root_sequence_without_losing_groups() {
        let mut state = PresetUi::default();
        let first = HierarchyOrderEntry::RootItem { group: "A".into(), key: "first".into() };
        let second = HierarchyOrderEntry::RootItem { group: "B".into(), key: "second".into() };
        let entries = vec![HierarchyOrderEntry::Group("Before".into()), first.clone(), HierarchyOrderEntry::Group("After".into()), second.clone()];

        state.move_hierarchy_entry("panel", "", entries.clone(), second.clone(), Some(&first));
        assert_eq!(
            state.hierarchy_order["panel"][""],
            [HierarchyOrderEntry::Group("Before".into()), second.clone(), first.clone(), HierarchyOrderEntry::Group("After".into()),]
        );

        let current = state.hierarchy_order["panel"][""].clone();
        state.move_hierarchy_entry("panel", "", current, first.clone(), None);
        assert_eq!(
            state.hierarchy_order["panel"][""],
            [HierarchyOrderEntry::Group("Before".into()), second, HierarchyOrderEntry::Group("After".into()), first,]
        );
    }

    #[test]
    fn root_cards_cannot_be_inserted_into_a_nested_group_sequence() {
        let mut state = PresetUi::default();
        let root = HierarchyOrderEntry::RootItem { group: "Owner".into(), key: "card".into() };
        state.set_hierarchy_order("panel", "", vec![root.clone(), HierarchyOrderEntry::Group("Folder".into())]);

        state.move_hierarchy_entry("panel", "Folder", vec![HierarchyOrderEntry::Item("nested".into())], root.clone(), None);
        state.set_hierarchy_order("panel", "Folder", vec![root.clone(), HierarchyOrderEntry::Item("nested".into())]);

        assert_eq!(state.hierarchy_order["panel"][""], [root.clone(), HierarchyOrderEntry::Group("Folder".into())]);
        assert_eq!(state.hierarchy_order["panel"]["Folder"], [HierarchyOrderEntry::Item("nested".into())]);
    }

    #[test]
    fn root_card_selection_is_qualified_by_owner_even_when_keys_match() {
        let mut state = PresetUi::default();
        state.set_selected_item("panel", "First", &[], "shared");

        assert!(state.is_selected_item("panel", "First", &[], "shared"));
        assert!(!state.is_selected_item("panel", "Second", &[], "shared"));
        state.set_selected_item("panel", "Second", &[], "shared");
        assert!(!state.is_selected_item("panel", "First", &[], "shared"));
        assert!(state.is_selected_item("panel", "Second", &[], "shared"));
    }

    #[test]
    fn deleting_a_root_card_removes_only_the_matching_owner_and_key() {
        let mut state = PresetUi::default();
        state.set_hierarchy_order(
            "panel",
            "",
            vec![
                HierarchyOrderEntry::RootItem { group: "First".into(), key: "shared".into() },
                HierarchyOrderEntry::RootItem { group: "Second".into(), key: "shared".into() },
            ],
        );

        state.delete_hierarchy_item_in_group("panel", "First", "shared");

        assert_eq!(state.hierarchy_order["panel"][""], [HierarchyOrderEntry::RootItem { group: "Second".into(), key: "shared".into() }]);
    }

    #[test]
    fn root_card_identity_and_mixed_order_survive_the_preferences_round_trip() {
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.ui.presets_ui.set_hierarchy_order(
            "gradients",
            "",
            vec![
                HierarchyOrderEntry::Group("First group".into()),
                HierarchyOrderEntry::RootItem { group: "Root owner".into(), key: "same:key".into() },
                HierarchyOrderEntry::RootItem { group: "Other owner".into(), key: "same:key".into() },
                HierarchyOrderEntry::Group("Second group".into()),
            ],
        );

        super::persist(&mut app, &egui::Context::default());
        let saved = app.session.prefs_to_json();
        let mut restarted = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        restarted.session.load_prefs_json(&saved).unwrap();
        super::restore(&mut restarted);

        assert_eq!(restarted.ui.presets_ui.hierarchy_order["gradients"][""], app.ui.presets_ui.hierarchy_order["gradients"][""]);
    }

    #[test]
    fn persist_does_not_save_picker_state_until_the_pointer_is_released() {
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        app.ui.presets_ui.set_hierarchy_order("panel", "", vec![HierarchyOrderEntry::Group("A".into())]);

        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::PointerButton {
                    pos: egui::Pos2::new(10.0, 10.0),
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            },
            |_| {},
        );
        output.textures_delta.clear();
        super::persist(&mut app, &ctx);
        assert!(!app.session.prefs().dialogs.contains_key("pickerBrowserUi"), "an in-progress drag must not persist an intermediate hierarchy");

        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::PointerButton {
                    pos: egui::Pos2::new(10.0, 10.0),
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            },
            |_| {},
        );
        output.textures_delta.clear();
        super::persist(&mut app, &ctx);
        assert!(app.session.prefs().dialogs.contains_key("pickerBrowserUi"), "the completed hierarchy is persisted after release");
    }

    #[test]
    fn legacy_brush_view_migrates_without_overwriting_saved_hierarchy_or_valid_defaults() {
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.prefs.edit(|prefs| {
            prefs.dialogs.insert(
                "pickerBrowserUi".into(),
                serde_json::json!({
                    "hierarchyOrder": {"gradients": {"": [{"type": "group", "key": "Existing group"}]}}
                }),
            );
            prefs.brush_picker = serde_json::json!({
                "showName": false,
                "showStroke": "not a boolean",
                "showTip": false,
                "scale": "not a number"
            });
        });

        super::restore(&mut app);

        assert_eq!(
            app.ui.presets_ui.hierarchy_order["gradients"][""],
            [HierarchyOrderEntry::Group("Existing group".into())],
            "restoring the legacy brush view must retain the common picker's saved hierarchy"
        );
        assert!(!app.ui.presets_ui.brush_picker.show_name);
        assert!(app.ui.presets_ui.brush_picker.show_stroke, "a mistyped field retains its default");
        assert!(!app.ui.presets_ui.brush_picker.show_tip);
        assert_eq!(app.ui.presets_ui.brush_picker.scale, 1.0, "a mistyped scale retains its default");
        assert!(app.session.prefs().brush_picker.is_null(), "the legacy preference is consumed after migration");
    }

    #[test]
    fn legacy_brush_view_restores_one_visible_part_and_clamps_its_scale() {
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.prefs.edit(|prefs| {
            prefs.brush_picker = serde_json::json!({"showName": false, "showStroke": false, "showTip": false, "scale": -4.0});
        });

        super::restore(&mut app);

        assert!(!app.ui.presets_ui.brush_picker.show_name);
        assert!(!app.ui.presets_ui.brush_picker.show_stroke);
        assert!(app.ui.presets_ui.brush_picker.show_tip, "the card must retain at least one visible part");
        assert_eq!(app.ui.presets_ui.brush_picker.scale, 0.15);
        assert!(app.session.prefs().brush_picker.is_null());
    }
}
