//! The exported menu translated to dbusmenu ids and properties, plus property diffs so
//! updates only push what changed. Pure data: no D-Bus types, unit-testable on any target.

use crate::model::{MenuCommand, MenuEntry, MenuModel, Shortcut, ShortcutMod};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Root item id; the dbusmenu spec fixes it and importers always start here.
pub const ROOT_ID: u32 = 0;

/// A set of dbusmenu item properties (wire-independent values).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Props {
    pub label: Option<String>,
    pub enabled: Option<bool>,
    pub visible: Option<bool>,
    pub separator: Option<bool>,
    pub submenu: Option<bool>,
    /// `Some(state)` only when the item shows a checkbox.
    pub checked: Option<Option<bool>>,
    pub shortcut: Option<Shortcut>,
}

/// One item of the flattened menu.
#[derive(Clone, Debug, PartialEq)]
pub struct FlatItem {
    pub id: u32,
    pub parent: u32,
    /// 0-based position in the parent's `children` array.
    pub pos: u32,
    pub action: String,
    pub props: Props,
}

/// The flattened, id-assigned menu (root first, then depth-first).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct FlatMenu {
    pub items: Vec<FlatItem>,
    /// Maps item id → action token (leaves only); rebuilt with the layout.
    pub actions: BTreeMap<u32, String>,
}

/// How a new model differs from the currently exported one.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MenuDiff {
    /// Item set, order or shape changed: importers must re-fetch the layout.
    pub structure: bool,
    /// Same-id items whose properties changed, with the new values.
    pub updated: Vec<(u32, Props)>,
}

/// The stable-id allocator: keeps dbusmenu item ids meaningful across rebuilds, so a
/// click that races a `LayoutUpdated` still resolves the item it was made on. Items are
/// keyed by identity — leaf items by their `action` token (the stable thing in dynamic
/// menus: "Undo Exposure" keeps its id while its label changes), submenu titles by their
/// label (they have no action), separators by position, repeats by occurrence order. The
/// map is bounded: past the cap it resets (ids renumber once; the shape change makes
/// importers re-fetch, which they would do anyway).
#[derive(Clone, Debug, PartialEq)]
pub struct Ids {
    by_key: HashMap<String, u32>,
    next: u32,
}

/// Beyond this many distinct item identities the allocator starts over (hostile-input
/// bound; a Photoshop-scale catalog has a few hundred items).
const MAX_IDS: u32 = 65_536;

impl Default for Ids {
    fn default() -> Self {
        // 0 is the root's reserved id (the dbusmenu spec fixes it); ids hand out from 1.
        Ids { by_key: HashMap::new(), next: 1 }
    }
}

impl Ids {
    /// Allocates or looks up the id for an item identity.
    fn alloc(&mut self, key: String) -> u32 {
        if let Some(&id) = self.by_key.get(&key) {
            return id;
        }
        if self.next >= MAX_IDS {
            log::warn!("AppMenu: id table capped; item ids renumber");
            self.by_key.clear();
            self.next = 1;
        }
        let id = self.next;
        self.next = id.saturating_add(1);
        self.by_key.insert(key, id);
        id
    }
}

impl FlatMenu {
    /// Maximum submenu nesting accepted when flattening (documents and models are hostile
    /// inputs by the never-crash rule; Photoshop-scale menus never approach this).
    const MAX_DEPTH: u32 = 32;

    /// Flattens a model into ids in depth-first reading order (root = 0), so ids follow
    /// menu order and numbering stays predictable: items number 1, 2, 3, … depth-first.
    /// Only for one-off builds (tests); the exporter builds with a persistent allocator
    /// via [`FlatMenu::build_with_ids`], so ids survive across publishes.
    #[cfg(test)]
    pub fn build(model: &MenuModel) -> FlatMenu {
        FlatMenu::build_with_ids(model, &mut Ids::default())
    }

    /// Like [`FlatMenu::build`], but ids stay stable across calls that share `ids`: an item
    /// keeps the id it was first exported with, so clicks that race a structural rebuild
    /// cannot run the wrong command, importers re-fetch less, and property-only updates
    /// continue to cover the common case.
    pub fn build_with_ids(model: &MenuModel, ids: &mut Ids) -> FlatMenu {
        let mut menu = FlatMenu::default();
        menu.items.push(FlatItem {
            id: ROOT_ID,
            parent: ROOT_ID,
            pos: 0,
            action: String::new(),
            props: Props { submenu: Some(true), visible: Some(true), ..Props::default() },
        });
        let mut occurrence = HashMap::new();
        walk_preorder(&mut menu, ROOT_ID, &model.children, 0, ids, "", &mut occurrence);
        menu.actions = menu.items.iter().filter(|it| !it.action.is_empty()).map(|it| (it.id, it.action.clone())).collect();
        menu
    }

    pub fn item(&self, id: u32) -> Option<&FlatItem> {
        self.items.iter().find(|it| it.id == id)
    }

    /// The children of `parent` in order. With `parent == ROOT_ID` this can match the root
    /// item itself (its parent is the root by convention), so the root is excluded there.
    pub fn children(&self, parent: u32) -> Vec<u32> {
        let mut kids: Vec<&FlatItem> = self.items.iter().filter(|it| it.parent == parent).filter(|it| it.id != it.parent).collect();
        kids.sort_by_key(|it| it.pos);
        kids.iter().map(|it| it.id).collect()
    }
}

/// Maximum number of items one menu can export (root counts).
const MAX_ITEMS: usize = 4096;

/// Depth-first preorder walk assigning ids; bounded by [`FlatMenu::MAX_DEPTH`] and
/// [`MAX_ITEMS`] so an absurd or cyclic model truncates instead of recursing forever
/// (the never-crash rule).
fn walk_preorder(menu: &mut FlatMenu, parent: u32, entries: &[MenuEntry], depth: u32, ids: &mut Ids, parent_key: &str, occurrence: &mut HashMap<String, u32>) {
    if depth > FlatMenu::MAX_DEPTH || menu.items.len() >= MAX_ITEMS {
        log::warn!("AppMenu: menu truncated (nesting past {} or over {} items)", FlatMenu::MAX_DEPTH, MAX_ITEMS);
        return;
    }
    for (pos, entry) in entries.iter().enumerate() {
        let (kind, data) = match entry {
            MenuEntry::Command(c) => {
                if c.children.as_ref().is_none_or(|n| n.is_empty()) {
                    // A leaf keeps its id across label changes (dynamic labels like "Undo
                    // Exposure" only rename the item): the action token identifies it.
                    ("a".to_string(), c.action.clone())
                } else {
                    // A title has no action of its own; its label identifies it (a rename
                    // regenerates ids of its subtree — a structural change the host
                    // re-fetches anyway).
                    ("m".to_string(), c.label.clone())
                }
            }
            // Separators carry no identity beyond their slot: repeats in one menu are
            // distinct items, in order of appearance.
            MenuEntry::Separator => ("sep".to_string(), format!("{pos}")),
        };
        let base = format!("{parent_key}\u{1}{kind}\u{1}{data}");
        let occ = match occurrence.get_mut(&base) {
            Some(n) => {
                let o = *n;
                *n += 1;
                o
            }
            None => {
                occurrence.insert(base.clone(), 0);
                0
            }
        };
        let key = format!("{base}\u{1}{occ}");
        let id = ids.alloc(key.clone());
        match entry {
            MenuEntry::Separator => {
                menu.items.push(FlatItem {
                    id,
                    parent,
                    pos: pos as u32,
                    action: String::new(),
                    props: Props { separator: Some(true), visible: Some(true), enabled: Some(false), ..Props::default() },
                });
            }
            MenuEntry::Command(c) => {
                let has_children = c.children.as_ref().is_some_and(|n| !n.is_empty());
                menu.items.push(FlatItem {
                    id,
                    parent,
                    pos: pos as u32,
                    action: if has_children { String::new() } else { c.action.clone() },
                    props: Props::command(c, has_children),
                });
                if let Some(kids) = c.children.as_ref().filter(|kids| !kids.is_empty()) {
                    walk_preorder(menu, id, kids, depth + 1, ids, &key, occurrence);
                }
            }
        }
    }
}

impl Props {
    fn command(c: &MenuCommand, submenu: bool) -> Props {
        Props {
            label: Some(c.label.clone()),
            enabled: Some(c.enabled),
            visible: Some(true),
            separator: None,
            submenu: Some(submenu),
            checked: c.checked.map(Some),
            shortcut: c.shortcut.clone(),
        }
    }

    /// Subtracts every unset field: returns a copy with only the fields a filtered
    /// `GetLayout`/property query named. Empty `names` means "all".
    pub fn filtered(&self, names: &[String]) -> Props {
        if names.is_empty() {
            return self.clone();
        }
        let want: HashSet<&str> = names.iter().map(String::as_str).collect();
        Props {
            label: if want.contains("label") { self.label.clone() } else { None },
            enabled: if want.contains("enabled") { self.enabled } else { None },
            visible: if want.contains("visible") { self.visible } else { None },
            separator: if want.contains("type") { self.separator } else { None },
            submenu: if want.contains("children-display") { self.submenu } else { None },
            checked: if want.contains("toggle-state") || want.contains("toggle-type") { self.checked } else { None },
            shortcut: if want.contains("shortcut") { self.shortcut.clone() } else { None },
        }
    }

    /// The `(prop → value)` map as it goes on the wire.
    pub fn to_map(&self) -> BTreeMap<&'static str, PropValue> {
        let mut m = BTreeMap::new();
        if let Some(l) = &self.label {
            m.insert("label", PropValue::Str(l.clone()));
        }
        if let Some(e) = self.enabled {
            m.insert("enabled", PropValue::Bool(e));
        }
        if let Some(v) = self.visible {
            m.insert("visible", PropValue::Bool(v));
        }
        if let Some(true) = self.submenu {
            m.insert("children-display", PropValue::Str("submenu".into()));
        }
        if let Some(true) = self.separator {
            m.insert("type", PropValue::Str("separator".into()));
        }
        if let Some(checked) = self.checked {
            m.insert("toggle-type", PropValue::Str("checkmark".into()));
            m.insert("toggle-state", PropValue::Int(i32::from(checked.unwrap_or(false))));
        }
        if let Some(sc) = &self.shortcut {
            let keys = wire_shortcut(sc);
            if !keys.is_empty() {
                m.insert("shortcut", PropValue::Shortcut(keys));
            }
        }
        m
    }
}

/// A property value on the wire.
#[derive(Clone, Debug, PartialEq)]
pub enum PropValue {
    Str(String),
    Bool(bool),
    Int(i32),
    /// Modifiers then key: `[["Control", "Shift", "N"]]` (the dbusmenu shortcut value).
    Shortcut(Vec<Vec<String>>),
}

/// The dbusmenu property diff between the exported and the new tree.
pub fn diff(old: &FlatMenu, new: &FlatMenu) -> MenuDiff {
    let shape = |menu: &FlatMenu| -> Vec<(u32, u32, u32)> { menu.items.iter().map(|it| (it.id, it.parent, it.pos)).collect() };
    let structure = shape(old) != shape(new);
    let mut updated = Vec::new();
    for item in &new.items {
        // On a structure change importers re-fetch everything; property-only updates cover
        // the common case (enable/checked changes while editing).
        if !structure
            && let Some(old_item) = old.item(item.id)
            && old_item.props != item.props
        {
            updated.push((item.id, item.props.clone()));
        }
    }
    MenuDiff { structure, updated }
}

/// Encodes a shortcut for the wire: modifiers first, canonical dbusmenu names
/// (`Control`, `Alt`, `Shift` — `Cmd` is our command modifier and becomes `Control` on
/// Linux), then the key.
pub fn wire_shortcut(sc: &Shortcut) -> Vec<Vec<String>> {
    let Some(key) = wire_key(&sc.key) else {
        return Vec::new();
    };
    let mut keys: Vec<String> = sc
        .modifiers
        .iter()
        .map(|m| match m {
            ShortcutMod::Cmd | ShortcutMod::Ctrl => "Control",
            ShortcutMod::Alt => "Alt",
            ShortcutMod::Shift => "Shift",
        })
        .map(String::from)
        .collect();
    keys.push(key);
    vec![keys]
}

/// The key half: printable ASCII travels as its keysym code point (uppercase letter / digit);
/// named keys travel as the keysym strings importers normalise.
fn wire_key(key: &str) -> Option<String> {
    let mut chars = key.chars();
    let first = chars.next()?;
    if chars.next().is_none() && first.is_ascii_graphic() {
        // A single printable character: its keysym equals its ASCII code point. Letters go
        // uppercase so a shifted chord keeps showing its letter.
        return Some(first.to_ascii_uppercase().to_string());
    }
    let lower = key.to_ascii_lowercase();
    let name = match lower.as_str() {
        "enter" | "return" => "return",
        "tab" => "tab",
        "space" => "space",
        "esc" | "escape" => "escape",
        "backspace" => "backspace",
        "delete" => "delete",
        "pageup" => "page_up",
        "pagedown" => "page_down",
        "home" => "home",
        "end" => "end",
        "left" => "left",
        "right" => "right",
        "up" => "up",
        "down" => "down",
        f if f.len() >= 2 && f.starts_with('f') && f[1..].chars().all(|c| c.is_ascii_digit()) && f[1..].len() <= 2 => {
            // F1…F24: importers normalise `letter + digits` function keys.
            f
        }
        // Wordy keys with no reliable keysym in the importers' tables: no shortcut property.
        _ => return None,
    };
    Some(name.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::MenuEntry;

    fn demo_model() -> MenuModel {
        MenuModel::top(vec![
            MenuEntry::command("File", "", false).submenu(vec![
                MenuEntry::command("New…", "file.new", true),
                MenuEntry::Separator,
                MenuEntry::command("Save", "file.save", true).shortcut("Ctrl+S"),
            ]),
            MenuEntry::command("Help", "", false).submenu(vec![MenuEntry::command("About PhotoCraft", "help.about", true)]),
        ])
    }

    #[test]
    fn ids_survive_structural_changes() {
        // The race the stable ids close: a click carries an id that was assigned against
        // an earlier revision. If ids were renumbered per build, the id would resolve to
        // whatever item now sits in its slot.
        let mut ids = Ids::default();
        let first = FlatMenu::build_with_ids(&demo_model(), &mut ids);
        let save = first.items.iter().find(|i| i.action == "file.save").map(|i| i.id).expect("a Save item");
        let about = first.items.iter().find(|i| i.action == "help.about").map(|i| i.id).expect("an About item");

        // The model changes structurally: the whole File menu (and its separator) goes
        // away and Help grows a second command *after* About, which under per-build
        // numbering would take About's id.
        let changed = MenuModel::top(vec![
            MenuEntry::command("Help", "", false)
                .submenu(vec![MenuEntry::command("About CraftApp", "help.about", true), MenuEntry::command("Handbook", "help.handbook", true)]),
        ]);
        let second = FlatMenu::build_with_ids(&changed, &mut ids);
        let about2 = second.items.iter().find(|i| i.action == "help.about").map(|i| i.id).expect("About still exported");
        let handbook = second.items.iter().find(|i| i.action == "help.handbook").map(|i| i.id).expect("Handbook exported");
        assert_eq!(about2, about, "surviving items keep their id");
        assert_ne!(handbook, about, "the new item does not reuse a live id");
        // And a removed menu's id is not silently handed to an unrelated item.
        assert_ne!(handbook, save, "removed items' ids are not reused");

        // Re-publishing the original model restores the original ids (the identities are
        // still in the allocator).
        let back = FlatMenu::build_with_ids(&demo_model(), &mut ids);
        assert_eq!(back.items.iter().find(|i| i.action == "file.save").map(|i| i.id), Some(save));
    }

    #[test]
    fn ids_renumber_when_the_table_is_capped() {
        // Hostile-input bound: past MAX_IDS the allocator starts over instead of growing
        // without limit (and never hands out the root's reserved 0).
        let mut ids = Ids::default();
        let key = "a:one".to_string();
        let one = ids.alloc(key.clone());
        for i in 0..=MAX_IDS {
            ids.alloc(format!("x:{i}"));
        }
        let one2 = ids.alloc(key);
        assert_ne!(one2, one, "the table reset");
        assert_ne!(one2, ROOT_ID, "0 stays reserved");
        assert_eq!(one, 1, "ids hand out from 1");
    }

    #[test]
    fn ids_never_collide_with_the_root() {
        let m = FlatMenu::build(&demo_model());
        assert_eq!(m.item(ROOT_ID).map(|i| i.id), Some(ROOT_ID), "root stays 0");
        assert!(m.items.iter().skip(1).all(|i| i.id != ROOT_ID), "no other item ever gets the root's id");
    }

    #[test]
    fn build_assigns_ids_depth_first() {
        let m = FlatMenu::build(&demo_model());
        // root 0; children of root: File=1, Help=4 (File's submenu issued first).
        assert_eq!(m.item(0).map(|i| (i.parent, i.pos)), Some((0, 0)));
        let file = m.items.iter().find(|i| i.props.label.as_deref() == Some("File")).cloned();
        let help = m.items.iter().find(|i| i.props.label.as_deref() == Some("Help")).cloned();
        let (file, help) = (file.map(|f| f.id), help.map(|h| h.id));
        assert_eq!(file, Some(1));
        assert_eq!(help, Some(5));
        // New…, separator, Save are ids 2, 3, 4.
        assert_eq!(m.children(1), vec![2, 3, 4]);
        // Actions on leaves only.
        assert_eq!(m.actions.get(&1), None);
        assert_eq!(m.actions.get(&2).map(String::as_str), Some("file.new"));
        assert_eq!(m.actions.get(&4).map(String::as_str), Some("file.save"));
        assert_eq!(m.actions.get(&6).map(String::as_str), Some("help.about"));
    }

    #[test]
    fn child_order_and_depth() {
        let m = FlatMenu::build(&demo_model());
        // Root's children in order; depth is a wire concern (see the dbus tests).
        assert_eq!(m.children(ROOT_ID).len(), 2);
        let file = m.item(1).cloned();
        assert_eq!(file.as_ref().and_then(|f| f.props.submenu), Some(true));
        // But the flat tree itself is flat: every item sits in `items` keyed by parent.
        assert_eq!(m.items.iter().filter(|it| it.parent == 1).count(), 3);
        assert_ne!(m.item(1).map(|it| it.parent), Some(1));
    }

    #[test]
    fn filtered_props_names() {
        let m = FlatMenu::build(&demo_model());
        let props = m.item(2).map(|it| it.props.clone()).unwrap_or_default();
        let f = props.filtered(&["label".into(), "enabled".into()]);
        assert_eq!(f.label.as_deref(), Some("New…"));
        assert_eq!(f.enabled, Some(true));
        assert_eq!(f.visible, None);
        // Empty names = all properties.
        let all = props.filtered(&[]);
        assert_eq!(all.visible, Some(true));
        assert_eq!(all.separator, None);
    }

    #[test]
    fn props_of_items() {
        let m = FlatMenu::build(&MenuModel::top(vec![MenuEntry::command("Show Grid", "view.grid", true).checked(true).shortcut("Ctrl+'")]));
        let p = m.item(1).map(|i| i.props.clone());
        let p = p.unwrap_or_default();
        assert_eq!(p.checked, Some(Some(true)));
        assert_eq!(p.shortcut.as_ref().map(|s| s.key.as_str()), Some("'"));
        let map = p.to_map();
        assert_eq!(map.get("toggle-state"), Some(&PropValue::Int(1)));
        assert_eq!(map.get("shortcut"), Some(&PropValue::Shortcut(vec![vec!["Control".to_string(), "'".to_string()]])));
    }

    #[test]
    fn diff_structure_vs_props() {
        let a = FlatMenu::build(&demo_model());
        let mut b = a.clone();
        if let Some(it) = b.items.iter_mut().find(|it| it.id == 2) {
            it.props.enabled = Some(false);
        }
        let d = diff(&a, &b);
        assert!(!d.structure);
        assert_eq!(d.updated.len(), 1);
        assert_eq!(d.updated[0].0, 2);
        // Adding an item changes the shape.
        let c = FlatMenu::build(&MenuModel::top(vec![
            MenuEntry::command("X", "x", true),
            MenuEntry::command("File", "", false).submenu(vec![
                MenuEntry::command("New…", "file.new", true),
                MenuEntry::Separator,
                MenuEntry::command("Save", "file.save", true).shortcut("Ctrl+S"),
            ]),
            MenuEntry::command("Help", "", false).submenu(vec![MenuEntry::command("About PhotoCraft", "help.about", true)]),
        ]));
        assert!(diff(&a, &c).structure);
        assert!(diff(&a, &c).updated.is_empty());
    }

    #[test]
    fn shortcut_encoding() {
        let sc = Shortcut::parse("Ctrl+Shift+N").unwrap_or(Shortcut { modifiers: vec![], key: String::new() });
        assert_eq!(wire_shortcut(&sc), vec![vec!["Control".to_string(), "Shift".to_string(), "N".to_string()]]);
        // Cmd is the command modifier → Control on Linux; named keys travel as keysym names.
        let sc = Shortcut::parse("Cmd+F1");
        assert_eq!(sc.and_then(|s| wire_shortcut(&s).into_iter().next()), Some(vec!["Control".to_string(), "f1".to_string()]));
        // Unknown wordy key: no shortcut property at all.
        let sc = Shortcut::parse("Ctrl+PanTheStd");
        assert!(sc.as_ref().map(|s| s.key.is_empty() || wire_shortcut(s).is_empty()) == Some(true));
    }

    #[test]
    fn shortcut_parse_forms() {
        assert_eq!(Shortcut::parse("Ctrl+N").as_ref().map(|s| s.key.clone()), Some("N".into()));
        assert_eq!(Shortcut::parse("Ctrl+Alt+Shift+W").map(|s| s.modifiers.len()), Some(3));
        assert_eq!(Shortcut::parse(""), None);
        assert_eq!(Shortcut::parse("Ctrl+X+Y"), None);
        // Duplicates collapse, order is canonical.
        let s = Shortcut::parse("Shift+Ctrl+Shift+S");
        assert_eq!(s.map(|s| s.modifiers), Some(vec![ShortcutMod::Ctrl, ShortcutMod::Shift]));
    }
}
