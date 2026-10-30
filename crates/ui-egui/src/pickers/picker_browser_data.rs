//! Persistent, data-only browser state for asset pickers.
//!
//! The ordered mixed-child sequences are the sole structural source of truth: a group's parent
//! is the key of the sequence containing its `Group` entry. This module contains no asset records,
//! preview state, egui types, or rendering data.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Deserializer, Serialize, Serializer, ser::SerializeStruct};
use serde_json::Value;

/// A direct child in one ordered picker-layout sequence. Groups and cards are peers at every
/// parent; the variant identifies which stable key the entry refers to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum HierarchyOrderEntry {
    Item(String),
    /// An item lifted out of its owning group into the root sibling sequence.
    RootItem { group: String, key: String },
    Group(String),
}

/// Stable on-disk representation. RootItem remains encoded as the historical Item token. New
/// literal root Item keys in either reserved token namespace are escaped so their identity is
/// unambiguous without changing the established wire variants.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "key", rename_all = "camelCase")]
enum HierarchyOrderEntryWire {
    Item(String),
    Group(String),
}

const ROOT_ITEM_TOKEN_PREFIX: &str = "__picker-root-item:v1:";
// Both prefixes are reserved only for root-sequence wire tokens; nested item keys are opaque.
const ROOT_ITEM_LITERAL_TOKEN_PREFIX: &str = "__picker-root-item:literal:v1:";

fn encode_root_item_token(group: &str, key: &str) -> String {
    format!("{ROOT_ITEM_TOKEN_PREFIX}{}:{group}{}:{key}", group.len(), key.len())
}

/// Decode the original length-delimited v1 token and its later variant with an extra separator
/// before the key length. This is called only for root entries; nested Item strings stay opaque.
fn decode_root_item_token(token: &str) -> Option<(&str, &str)> {
    let payload = token.strip_prefix(ROOT_ITEM_TOKEN_PREFIX)?;
    let (group_len, rest) = payload.split_once(':')?;
    let group_len = group_len.parse::<usize>().ok()?;
    let group = rest.get(..group_len)?;
    let after_group = rest.get(group_len..)?;
    let key_length_field = after_group.strip_prefix(':').unwrap_or(after_group);
    let (key_len, key) = key_length_field.split_once(':')?;
    let key_len = key_len.parse::<usize>().ok()?;
    (key.len() == key_len).then_some((group, key))
}

fn encode_root_item_literal_token(key: &str) -> String {
    format!("{ROOT_ITEM_LITERAL_TOKEN_PREFIX}{}:{key}", key.len())
}

fn decode_root_item_literal_token(token: &str) -> Option<&str> {
    let payload = token.strip_prefix(ROOT_ITEM_LITERAL_TOKEN_PREFIX)?;
    let (key_len, key) = payload.split_once(':')?;
    let key_len = key_len.parse::<usize>().ok()?;
    (key.len() == key_len).then_some(key)
}

impl HierarchyOrderEntryWire {
    fn into_entry(self, parent_is_root: bool) -> HierarchyOrderEntry {
        match self {
            Self::Item(token) if parent_is_root => decode_root_item_literal_token(&token)
                .map(|key| HierarchyOrderEntry::Item(key.to_owned()))
                .or_else(|| decode_root_item_token(&token).map(|(group, key)| HierarchyOrderEntry::RootItem { group: group.to_owned(), key: key.to_owned() }))
                .unwrap_or(HierarchyOrderEntry::Item(token)),
            Self::Item(key) => HierarchyOrderEntry::Item(key),
            Self::Group(group) => HierarchyOrderEntry::Group(group),
        }
    }

    fn from_entry(entry: &HierarchyOrderEntry, parent_is_root: bool) -> Self {
        match entry {
            HierarchyOrderEntry::Item(key) if parent_is_root && (decode_root_item_token(key).is_some() || decode_root_item_literal_token(key).is_some()) => {
                Self::Item(encode_root_item_literal_token(key))
            }
            HierarchyOrderEntry::Item(key) => Self::Item(key.clone()),
            HierarchyOrderEntry::RootItem { group, key } => Self::Item(encode_root_item_token(group, key)),
            HierarchyOrderEntry::Group(group) => Self::Group(group.clone()),
        }
    }
}

/// The complete persisted layout slice of `PresetUi`.
///
/// `hierarchy_order` owns both sibling order and parentage. `toggled` stores the non-default
/// group-open state. Parent maps and split ordering fields exist only in the serde compatibility
/// wire format and are normalized into `hierarchy_order` when old state is read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PickerBrowserData {
    pub toggled: BTreeSet<String>,
    pub(crate) hierarchy_order: BTreeMap<String, BTreeMap<String, Vec<HierarchyOrderEntry>>>,
}

impl PickerBrowserData {
    /// Return the containing group for a group entry. Root groups have no parent.
    pub(crate) fn parent_of(&self, panel: &str, group: &str) -> Option<&str> {
        self.hierarchy_order.get(panel)?.iter().find_map(|(parent, entries)| {
            (!parent.is_empty() && entries.iter().any(|entry| matches!(entry, HierarchyOrderEntry::Group(name) if name == group)))
                .then_some(parent.as_str())
        })
    }

    /// Materialize the derived parent index once for algorithms that visit many groups.
    pub(crate) fn group_parents(&self, panel: &str) -> BTreeMap<String, String> {
        let mut parents = BTreeMap::new();
        if let Some(sequences) = self.hierarchy_order.get(panel) {
            for (parent, entries) in sequences {
                if parent.is_empty() { continue; }
                for entry in entries {
                    if let HierarchyOrderEntry::Group(group) = entry {
                        parents.entry(group.clone()).or_insert_with(|| parent.clone());
                    }
                }
            }
        }
        parents
    }

    /// Build the legacy `groupParent` view from canonical membership for serialization and
    /// compatibility callers. It is never retained as mutable state.
    fn parent_projection(&self) -> BTreeMap<String, BTreeMap<String, String>> {
        let mut projection = BTreeMap::new();
        for (panel, parents) in &self.hierarchy_order {
            for (parent, entries) in parents {
                if parent.is_empty() { continue; }
                for entry in entries {
                    if let HierarchyOrderEntry::Group(group) = entry {
                        projection.entry(panel.clone()).or_insert_with(BTreeMap::new).entry(group.clone()).or_insert_with(|| parent.clone());
                    }
                }
            }
        }
        projection
    }

    fn has_group_membership(&self, panel: &str, group: &str) -> bool {
        self.hierarchy_order.get(panel).is_some_and(|parents| {
            parents.values().any(|entries| entries.iter().any(|entry| matches!(entry, HierarchyOrderEntry::Group(name) if name == group)))
        })
    }

    fn insert_legacy_group(&mut self, panel: &str, parent: &str, group: &str, before_item: Option<&str>) {
        if self.has_group_membership(panel, group) { return; }
        let entries = self.hierarchy_order.entry(panel.to_string()).or_default().entry(parent.to_string()).or_default();
        let at = before_item.and_then(|key| entries.iter().position(|entry| matches!(entry, HierarchyOrderEntry::Item(item) if item == key))).unwrap_or(entries.len());
        entries.insert(at, HierarchyOrderEntry::Group(group.to_string()));
    }

    fn normalize_group_membership(&mut self) {
        for orders in self.hierarchy_order.values_mut() {
            let mut seen = BTreeSet::new();
            let mut seen_root_items = BTreeSet::new();
            for (parent, entries) in orders.iter_mut() {
                entries.retain(|entry| match entry {
                    HierarchyOrderEntry::Group(group) => seen.insert(group.clone()),
                    HierarchyOrderEntry::Item(_) => true,
                    HierarchyOrderEntry::RootItem { group, key } => parent.is_empty() && seen_root_items.insert((group.clone(), key.clone())),
                });
            }
            orders.retain(|_, entries| !entries.is_empty());
        }
        self.hierarchy_order.retain(|_, orders| !orders.is_empty());
    }

    /// Flatten nested group membership for legacy group-only panels while retaining card entries
    /// in their original parent sequences. Descendants are appended to the root in depth-first
    /// sibling order, matching the visible hierarchy order before flattening.
    pub(crate) fn flatten_group_nesting(&mut self, panel: &str) {
        fn collect_descendants(layout: &PickerBrowserData, panel: &str, parent: &str, seen: &mut BTreeSet<String>, out: &mut Vec<HierarchyOrderEntry>) {
            let mut pending: Vec<String> = layout
                .hierarchy_order
                .get(panel)
                .and_then(|orders| orders.get(parent))
                .into_iter()
                .flatten()
                .rev()
                .filter_map(|entry| if let HierarchyOrderEntry::Group(group) = entry { Some(group.clone()) } else { None })
                .collect();
            while let Some(group) = pending.pop() {
                if !seen.insert(group.clone()) { continue; }
                out.push(HierarchyOrderEntry::Group(group.clone()));
                if let Some(children) = layout.hierarchy_order.get(panel).and_then(|orders| orders.get(&group)) {
                    pending.extend(children.iter().rev().filter_map(|entry| {
                        if let HierarchyOrderEntry::Group(child) = entry { Some(child.clone()) } else { None }
                    }));
                }
            }
        }

        let root_groups: Vec<String> = self.hierarchy_order.get(panel).and_then(|orders| orders.get("")).into_iter().flatten().filter_map(|entry| {
            if let HierarchyOrderEntry::Group(group) = entry { Some(group.clone()) } else { None }
        }).collect();
        let mut seen: BTreeSet<String> = root_groups.iter().cloned().collect();
        let child_groups: BTreeSet<String> = self.hierarchy_order.get(panel).into_iter().flat_map(|orders| orders.values()).flatten().filter_map(|entry| {
            if let HierarchyOrderEntry::Group(group) = entry { Some(group.clone()) } else { None }
        }).collect();
        let mut root_groups = root_groups;
        let mut root_extras = Vec::new();
        if let Some(orders) = self.hierarchy_order.get(panel) {
            for parent in orders.keys().filter(|parent| !parent.is_empty() && !child_groups.contains(*parent)) {
                if seen.insert(parent.clone()) {
                    root_groups.push(parent.clone());
                    root_extras.push(HierarchyOrderEntry::Group(parent.clone()));
                }
            }
        }
        let mut flattened = Vec::new();
        for group in &root_groups {
            collect_descendants(self, panel, group, &mut seen, &mut flattened);
        }
        // Corrupt/orphaned cycles are still made visible at root, in deterministic key order.
        if let Some(orders) = self.hierarchy_order.get(panel) {
            for (parent, entries) in orders {
                if parent.is_empty() { continue; }
                for entry in entries {
                    if let HierarchyOrderEntry::Group(group) = entry
                        && seen.insert(group.clone())
                    {
                        flattened.push(HierarchyOrderEntry::Group(group.clone()));
                        collect_descendants(self, panel, group, &mut seen, &mut flattened);
                    }
                }
            }
        }
        if !root_extras.is_empty() || !flattened.is_empty() {
            let mut root = self.hierarchy_order.get(panel).and_then(|orders| orders.get("")).cloned().unwrap_or_default();
            root.extend(root_extras);
            root.extend(flattened);
            self.hierarchy_order.entry(panel.to_string()).or_default().insert(String::new(), root);
        }
        let remove_panel = if let Some(orders) = self.hierarchy_order.get_mut(panel) {
            for (parent, entries) in orders.iter_mut() {
                if !parent.is_empty() {
                    entries.retain(|entry| matches!(entry, HierarchyOrderEntry::Item(_)));
                }
            }
            orders.retain(|parent, entries| parent.is_empty() || !entries.is_empty());
            orders.is_empty()
        } else { false };
        if remove_panel { self.hierarchy_order.remove(panel); }
    }
}

impl Serialize for PickerBrowserData {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // Keep the existing public PresetUi keys. `groupParent` is emitted as a derived projection;
        // legacy split ordering keys are intentionally not written by new versions.
        // RootItem identity is `(group, key)`: collapse duplicate representations of that exact
        // identity, but never collapse same-key cards owned by different groups. Normalize a clone
        // so even directly-mutated in-memory state cannot persist duplicate root-item tokens.
        let mut normalized = self.clone();
        normalized.normalize_group_membership();
        let parents = normalized.parent_projection();
        let has_order = !normalized.hierarchy_order.is_empty();
        let has_parents = !parents.is_empty();
        let field_count = 1 + (if has_order { 1 } else { 0 }) + (if has_parents { 1 } else { 0 });
        let mut state = serializer.serialize_struct("PickerBrowserData", field_count)?;
        state.serialize_field("toggled", &self.toggled)?;
        if has_order {
            let wire: BTreeMap<String, BTreeMap<String, Vec<HierarchyOrderEntryWire>>> = normalized.hierarchy_order.iter().map(|(panel, parents)| {
                (panel.clone(), parents.iter().map(|(parent, entries)| {
                    (parent.clone(), entries.iter().map(|entry| HierarchyOrderEntryWire::from_entry(entry, parent.is_empty())).collect())
                }).collect())
            }).collect();
            state.serialize_field("hierarchyOrder", &wire)?;
        }
        if has_parents { state.serialize_field("groupParent", &parents)?; }
        state.end()
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PickerBrowserDataWire {
    toggled: BTreeSet<String>,
    #[serde(rename = "hierarchyOrder", alias = "hierarchy_order")]
    hierarchy_order: BTreeMap<String, BTreeMap<String, Vec<HierarchyOrderEntryWire>>>,
    #[serde(rename = "groupParent", alias = "group_parent", deserialize_with = "lenient_group_parent")]
    group_parent: BTreeMap<String, BTreeMap<String, String>>,
    #[serde(rename = "groupOrder", alias = "group_order", deserialize_with = "lenient_group_order")]
    group_order: BTreeMap<String, Vec<String>>,
    #[serde(rename = "groupItemAnchors", alias = "group_item_anchors", deserialize_with = "lenient_group_item_anchors")]
    group_item_anchors: BTreeMap<String, BTreeMap<String, (String, Option<String>)>>,
}

impl<'de> Deserialize<'de> for PickerBrowserData {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = PickerBrowserDataWire::deserialize(deserializer)?;
        let hierarchy_order = wire.hierarchy_order.into_iter().map(|(panel, parents)| {
            (panel, parents.into_iter().map(|(parent, entries)| {
                let parent_is_root = parent.is_empty();
                (parent, entries.into_iter().map(|entry| entry.into_entry(parent_is_root)).collect())
            }).collect())
        }).collect();
        let mut layout = Self { toggled: wire.toggled, hierarchy_order };

        // Migrate old group/item anchors first so they retain their intended position before a
        // card. Explicit hierarchyOrder membership always wins over legacy representations.
        let mut anchored_groups: BTreeMap<(String, String, Option<String>), Vec<String>> = BTreeMap::new();
        for (panel, anchors) in wire.group_item_anchors {
            for (group, (parent, before_item)) in anchors {
                anchored_groups.entry((panel.clone(), parent, before_item)).or_default().push(group);
            }
        }
        for ((panel, parent, before_item), groups) in anchored_groups {
            let missing: Vec<String> = groups.into_iter().filter(|group| !layout.has_group_membership(&panel, group)).collect();
            if missing.is_empty() { continue; }
            let entries = layout.hierarchy_order.entry(panel).or_default().entry(parent).or_default();
            let at = before_item.as_deref().and_then(|key| entries.iter().position(|entry| matches!(entry, HierarchyOrderEntry::Item(item) if item == key))).unwrap_or(entries.len());
            entries.splice(at..at, missing.into_iter().map(HierarchyOrderEntry::Group));
        }

        // Older layouts represented nesting separately. Add only groups not already represented
        // in hierarchyOrder (or migrated from anchors), so a newer canonical move wins conflicts.
        for (panel, parents) in wire.group_parent {
            for (group, parent) in parents {
                layout.insert_legacy_group(&panel, &parent, &group, None);
            }
        }

        // groupOrder is the final fallback for root order. Groups with canonical/legacy parentage
        // are already present and therefore cannot be accidentally lifted to the root.
        for (panel, groups) in wire.group_order {
            for group in groups {
                layout.insert_legacy_group(&panel, "", &group, None);
            }
        }
        // Corrupt duplicate memberships have no meaningful single parent. Keep the first
        // occurrence in deterministic parent-key order; normal saved layouts are unchanged.
        layout.normalize_group_membership();
        Ok(layout)
    }
}

/// Read `groupOrder` leniently so malformed legacy values do not invalidate all saved UI state.
const MAX_GROUP_ORDER: usize = 10_000;
const MAX_LEGACY_GROUP_ENTRIES: usize = 10_000;

fn lenient_group_order<'de, D: Deserializer<'de>>(deserializer: D) -> Result<BTreeMap<String, Vec<String>>, D::Error> {
    let value = Value::deserialize(deserializer)?;
    let mut out = BTreeMap::new();
    for (panel, names) in value.as_object().into_iter().flatten() {
        let Some(names) = names.as_array() else { continue };
        out.insert(panel.clone(), names.iter().filter_map(Value::as_str).take(MAX_GROUP_ORDER).map(str::to_owned).collect());
    }
    Ok(out)
}

/// Legacy compatibility fields are advisory fallbacks. Ignore malformed entries rather than
/// allowing one stale value to prevent the rest of `PresetUi` (including canonical order) loading.
/// Bound each panel independently so corrupt settings cannot cause unbounded migration work.
fn lenient_group_parent<'de, D: Deserializer<'de>>(deserializer: D) -> Result<BTreeMap<String, BTreeMap<String, String>>, D::Error> {
    let value = Value::deserialize(deserializer)?;
    let mut out = BTreeMap::new();
    for (panel, groups) in value.as_object().into_iter().flatten() {
        let Some(groups) = groups.as_object() else { continue };
        let parents: BTreeMap<String, String> = groups
            .iter()
            .filter_map(|(group, parent)| parent.as_str().map(|parent| (group.clone(), parent.to_owned())))
            .take(MAX_LEGACY_GROUP_ENTRIES)
            .collect();
        if !parents.is_empty() { out.insert(panel.clone(), parents); }
    }
    Ok(out)
}

fn lenient_group_item_anchors<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<String, BTreeMap<String, (String, Option<String>)>>, D::Error> {
    let value = Value::deserialize(deserializer)?;
    let mut out = BTreeMap::new();
    for (panel, groups) in value.as_object().into_iter().flatten() {
        let Some(groups) = groups.as_object() else { continue };
        let anchors: BTreeMap<String, (String, Option<String>)> = groups
            .iter()
            .filter_map(|(group, anchor)| {
                let anchor = anchor.as_array()?;
                if anchor.len() != 2 { return None; }
                let parent = anchor.first()?.as_str()?.to_owned();
                let before_item = match anchor.get(1)? {
                    Value::Null => None,
                    Value::String(key) => Some(key.clone()),
                    _ => return None,
                };
                Some((group.clone(), (parent, before_item)))
            })
            .take(MAX_LEGACY_GROUP_ENTRIES)
            .collect();
        if !anchors.is_empty() { out.insert(panel.clone(), anchors); }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{HierarchyOrderEntry as Entry, PickerBrowserData};

    #[test]
    fn legacy_split_fields_migrate_and_parent_map_is_derived_on_write() {
        let saved = serde_json::json!({
            "toggled": ["panel/Parent"],
            "groupOrder": {"panel": ["Parent", "Root sibling", "Child"]},
            "groupParent": {"panel": {"Child": "Parent", "Grandchild": "Child"}},
            "groupItemAnchors": {"panel": {
                "Child": ["Parent", "card-a"],
                "Grandchild": ["Child", null]
            }}
        });
        let layout: PickerBrowserData = serde_json::from_value(saved).unwrap();

        assert_eq!(layout.parent_of("panel", "Child"), Some("Parent"));
        assert_eq!(layout.parent_of("panel", "Grandchild"), Some("Child"));
        assert_eq!(
            layout.hierarchy_order["panel"]["Parent"],
            [Entry::Group("Child".into()), Entry::Item("card-a".into())]
        );
        assert_eq!(layout.hierarchy_order["panel"][""], [Entry::Group("Parent".into()), Entry::Group("Root sibling".into())]);

        let written = serde_json::to_value(layout).unwrap();
        assert_eq!(written["groupParent"]["panel"]["Child"], "Parent");
        assert_eq!(written["groupParent"]["panel"]["Grandchild"], "Child");
        assert!(written.get("groupOrder").is_none());
        assert!(written.get("groupItemAnchors").is_none());
        assert!(written.get("hierarchyOrder").is_some());
    }

    #[test]
    fn hierarchy_order_wins_legacy_parent_conflicts_and_anchor_ties_are_stable() {
        let saved = serde_json::json!({
            "hierarchyOrder": {"panel": {
                "Current parent": [{"type":"group", "key":"Child"}, {"type":"item", "key":"card"}]
            }},
            "groupParent": {"panel": {"Child": "Stale parent"}},
            "groupItemAnchors": {"panel": {
                "Z group": ["Current parent", "card"],
                "A group": ["Current parent", "card"]
            }}
        });
        let layout: PickerBrowserData = serde_json::from_value(saved).unwrap();
        assert_eq!(layout.parent_of("panel", "Child"), Some("Current parent"));
        assert_eq!(
            layout.hierarchy_order["panel"]["Current parent"],
            [Entry::Group("Child".into()), Entry::Group("A group".into()), Entry::Group("Z group".into()), Entry::Item("card".into())]
        );
    }

    #[test]
    fn flatten_group_nesting_preserves_card_membership_and_depth_first_group_order() {
        let mut layout: PickerBrowserData = serde_json::from_value(serde_json::json!({
            "hierarchyOrder": {"panel": {
                "": [{"type":"group", "key":"A"}, {"type":"item", "key":"__picker-root-item:v1:8:Original9:root-card"}],
                "A": [{"type":"item", "key":"a-card"}, {"type":"group", "key":"B"}],
                "B": [{"type":"item", "key":"b-card"}, {"type":"group", "key":"C"}],
                "C": [{"type":"item", "key":"c-card"}]
            }}
        })).unwrap();

        layout.flatten_group_nesting("panel");
        assert_eq!(
            layout.hierarchy_order["panel"][""],
            [Entry::Group("A".into()), Entry::RootItem { group: "Original".into(), key: "root-card".into() }, Entry::Group("B".into()), Entry::Group("C".into())]
        );
        assert_eq!(layout.hierarchy_order["panel"]["A"], [Entry::Item("a-card".into())]);
        assert_eq!(layout.hierarchy_order["panel"]["B"], [Entry::Item("b-card".into())]);
        assert_eq!(layout.hierarchy_order["panel"]["C"], [Entry::Item("c-card".into())]);
        assert_eq!(layout.parent_of("panel", "B"), None);
        assert_eq!(layout.parent_of("panel", "C"), None);
    }

    #[test]
    fn root_item_tokens_migrate_both_v1_spellings_and_dedupe_same_identity() {
        let length_delimited = "__picker-root-item:v1:8:Original9:root-card";
        let separator_variant = "__picker-root-item:v1:8:Original:9:root-card";
        let saved = serde_json::json!({"hierarchyOrder": {"panel": {
            "": [
                {"type":"item", "key": length_delimited},
                {"type":"item", "key": separator_variant}
            ]
        }}});
        let layout: PickerBrowserData = serde_json::from_value(saved.clone()).unwrap();
        assert_eq!(layout.hierarchy_order["panel"][""], [Entry::RootItem { group: "Original".into(), key: "root-card".into() }]);
        let written = serde_json::to_value(layout).unwrap();
        let serialized = written["hierarchyOrder"]["panel"][""].as_array().unwrap();
        assert_eq!(serialized.len(), 1);
        assert_eq!(serialized[0], serde_json::json!({"type":"item", "key":length_delimited}));
    }

    #[test]
    fn root_item_persistence_preserves_same_key_in_distinct_source_groups() {
        let mut layout = PickerBrowserData::default();
        layout.hierarchy_order.insert("panel".into(), [(String::new(), vec![
            Entry::RootItem { group: "First".into(), key: "same".into() },
            Entry::RootItem { group: "First".into(), key: "same".into() },
            Entry::RootItem { group: "Second".into(), key: "same".into() },
        ])].into());

        let written = serde_json::to_value(&layout).unwrap();
        let serialized = written["hierarchyOrder"]["panel"][""].as_array().unwrap();
        assert_eq!(serialized.len(), 2, "duplicate identity is collapsed, distinct owners remain");

        let restored: PickerBrowserData = serde_json::from_value(written).unwrap();
        assert_eq!(restored.hierarchy_order["panel"][""], [
            Entry::RootItem { group: "First".into(), key: "same".into() },
            Entry::RootItem { group: "Second".into(), key: "same".into() },
        ]);
    }

    #[test]
    fn token_looking_nested_item_keys_remain_literal() {
        let token = "__picker-root-item:v1:8:Original9:root-card";
        let layout: PickerBrowserData = serde_json::from_value(serde_json::json!({"hierarchyOrder": {"panel": {
            "Original": [{"type":"item", "key": token}],
            "": [{"type":"item", "key":"__picker-root-item:v1:8:Original9:root-card"}]
        }}})).unwrap();
        assert_eq!(layout.hierarchy_order["panel"]["Original"], [Entry::Item(token.into())]);
        assert_eq!(layout.hierarchy_order["panel"][""], [Entry::RootItem { group: "Original".into(), key: "root-card".into() }]);
    }

    #[test]
    fn token_looking_root_item_keys_round_trip_without_becoming_root_items() {
        let root_item_token = "__picker-root-item:v1:8:Original9:root-card";
        let literal_marker_token = "__picker-root-item:literal:v1:4:card";
        let mut layout = PickerBrowserData::default();
        layout.hierarchy_order.insert("panel".into(), [(String::new(), vec![
            Entry::Item(root_item_token.into()),
            Entry::Item(literal_marker_token.into()),
            Entry::RootItem { group: "Original".into(), key: "root-card".into() },
        ])].into());

        let written = serde_json::to_value(&layout).unwrap();
        let restored: PickerBrowserData = serde_json::from_value(written).unwrap();
        assert_eq!(restored.hierarchy_order["panel"][""], [
            Entry::Item(root_item_token.into()),
            Entry::Item(literal_marker_token.into()),
            Entry::RootItem { group: "Original".into(), key: "root-card".into() },
        ]);
    }

    #[test]
    fn malformed_legacy_hierarchy_entries_are_skipped_without_losing_canonical_data() {
        let layout: PickerBrowserData = serde_json::from_value(serde_json::json!({
            "hierarchyOrder": {"panel": {
                "": [{"type":"group", "key":"Canonical"}]
            }},
            "groupParent": {"panel": {
                "Valid child": "Canonical",
                "Malformed child": 42,
                "Also malformed": null
            }, "wrong panel shape": []},
            "groupItemAnchors": {"panel": {
                "Valid anchored": ["Canonical", null],
                "Wrong parent": [42, "card"],
                "Wrong item": ["Canonical", false],
                "Wrong length": ["Canonical"]
            }, "wrong panel shape": "not an object"}
        })).unwrap();

        assert_eq!(layout.parent_of("panel", "Canonical"), None);
        assert_eq!(layout.parent_of("panel", "Valid child"), Some("Canonical"));
        assert_eq!(layout.hierarchy_order["panel"]["Canonical"], [
            Entry::Group("Valid anchored".into()),
            Entry::Group("Valid child".into()),
        ]);
        let written = serde_json::to_value(layout).unwrap();
        assert!(written.get("groupOrder").is_none());
        assert!(written.get("groupItemAnchors").is_none());
    }
}

