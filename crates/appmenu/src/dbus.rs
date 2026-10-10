//! The `com.canonical.dbusmenu` D-Bus interface served by the exporter, backed by the flat
//! layout. One interface name is served; every importer we target (libdbusmenu,
//! libdbusmenu-qt, Waybar, Gershwin Menu) queries `com.canonical.dbusmenu`.

use crate::MenuWake;
use crate::layout::{FlatMenu, Ids, Props, ROOT_ID};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use zvariant::{OwnedValue, Value};

/// The object path the menu is served at. One window = one exported model, so a single
/// application exports a single object; windows differ only through window registration.
pub const MENU_OBJECT_PATH: &str = "/MenuBar";

// The dbusmenu interface version we implement is the one every importer understands.
const VERSION: u32 = 3;

/// How many pending clicks the queue holds at most. Any session-bus peer can send
/// `Event("clicked")` — local peers are trusted no more than a malformed file — so the
/// queue is bounded: overflow drops the oldest clicks (stale clicks are the worthless
/// ones; the newest stay queued for the UI's next `try_event` batch).
const MAX_ACTIVATIONS: usize = 64;

/// Shared server state: the exported layout plus recorded activations.
#[derive(Default)]
pub struct MenuState {
    /// `None` until the first model is published.
    pub menu: Option<FlatMenu>,
    pub revision: u32,
    /// The stable-id allocator: item ids stay meaningful across rebuilds (a click racing
    /// a `LayoutUpdated` resolves the item it was made on; see [`crate::layout::Ids`]).
    pub ids: Ids,
    pub activations: VecDeque<(u32, String, String)>,
    /// The shell-side waker (see [`crate::MenuWake`]); called when a click is recorded,
    /// so a reactive app renders the frame that reads `try_event`.
    pub wake: Option<MenuWake>,
}

impl MenuState {
    /// Records a click; only enabled leaves with an action fire (submenus and separators
    /// are titles and gaps, not commands).
    pub fn activate(&mut self, id: u32) {
        let Some(item) = self.menu.as_ref().and_then(|m| m.item(id)) else { return };
        if item.action.is_empty() || item.props.enabled == Some(false) {
            return;
        }
        let label = item.props.label.clone().unwrap_or_default();
        self.activations.push_back((id, label, item.action.clone()));
        while self.activations.len() > MAX_ACTIVATIONS {
            self.activations.pop_front();
        }
        // The D-Bus event arrived on the exporter's own thread: without the waker the app
        // (which renders no frames while idle) would only see the click at its next event.
        if let Some(wake) = &self.wake {
            wake();
        }
    }
}

/// Converts a wire-ready property into an owned variant. The contained types are all
/// trivially ownable, so the failure case (only possible with file descriptors) is
/// unreachable in practice; fall back to skipping the caller-tolerant `None` below.
fn owned_prop(value: crate::layout::PropValue) -> Option<OwnedValue> {
    let value = match value {
        crate::layout::PropValue::Str(s) => Value::from(s),
        crate::layout::PropValue::Bool(b) => Value::from(b),
        crate::layout::PropValue::Int(i) => Value::from(i),
        // The dbusmenu shortcut type: an array of arrays of strings (`aas`), i.e.
        // `[["Control", "Shift", "N"]]`. `wire_shortcut` already canonicalises names.
        crate::layout::PropValue::Shortcut(keys) => Value::from(keys),
    };
    value.try_into_owned().ok()
}

/// Converts our property set to the wire `{string → variant}` map; an empty `names` means
/// "all properties" (the spec's unfiltered case).
pub fn wire_props(props: &Props, names: &[String]) -> HashMap<String, OwnedValue> {
    let filtered = props.filtered(names);
    let mut out: HashMap<String, OwnedValue> =
        filtered.to_map().into_iter().filter_map(|(key, value)| owned_prop(value).map(|v| (key.to_string(), v))).collect();
    // Importers apply the "standard" type default, but some don't: state it explicitly.
    if filtered.separator != Some(true)
        && (names.is_empty() || names.iter().any(|n| n == "type"))
        && let Some(v) = owned_prop(crate::layout::PropValue::Str("standard".into()))
    {
        out.insert("type".to_string(), v);
    }
    out
}

/// The wire layout row: `(ia{sv}av)` over the bus — id, property map, children as variants.
#[derive(Debug, zvariant::Type, serde::Serialize)]
pub struct WireRow {
    pub id: i32,
    pub props: HashMap<String, OwnedValue>,
    /// Children wrapped as D-Bus variants (`av`); each is a variant of a row struct
    /// (`(ia{sv}av)`), exactly as the spec types it.
    pub children: Vec<Value<'static>>,
}

/// Builds the wire value for `GetLayout`: `depth < 0` unlimited, each positive step one
/// level deeper; an empty `names` means all properties.
pub fn wire_row(menu: &FlatMenu, id: u32, depth: i32, names: &[String]) -> WireRow {
    let item = menu.item(id);
    let props: Props = item.map_or_else(Props::default, |it| it.props.clone());
    let children = if depth == 0 {
        Vec::new()
    } else {
        // Negative depth is "unlimited": keep it as is, positive counts down.
        let child_depth = if depth > 0 { depth - 1 } else { depth };
        menu.children(id).iter().map(|&child| Value::new(row_value(menu, child, child_depth, names))).collect()
    };
    WireRow { id: id.min(i32::MAX as u32) as i32, props: wire_props(&props, names), children }
}

/// The recursive row as a plain D-Bus structure (for wrapping into child variants). The
/// parts are handed to the `Structure` un-wrapped: `zvariant` types a structure's fields
/// from the raw inputs, but any field already wrapped in a `Value` is itself a variant
/// (`Value::new` re-wraps values whose signature is `"v"`), which would degrade the wire
/// row to `(ivv)` and break every importer's decoder.
fn row_value(menu: &FlatMenu, id: u32, depth: i32, names: &[String]) -> Value<'static> {
    let row = wire_row(menu, id, depth, names);
    Value::Structure(zvariant::Structure::from((row.id, row.props, row.children)))
}

/// The D-Bus interface. Call handlers run on zbus's executor thread; they only lock-shared
/// read the layout, so the UI thread never stalls behind them.
pub struct MenuBarIface {
    pub state: Arc<Mutex<MenuState>>,
}

#[zbus::interface(name = "com.canonical.dbusmenu")]
impl MenuBarIface {
    #[zbus(property)]
    fn version(&self) -> u32 {
        VERSION
    }

    #[zbus(property)]
    fn text_direction(&self) -> String {
        "ltr".into()
    }

    #[zbus(property)]
    fn status(&self) -> String {
        "normal".into()
    }

    #[zbus(property)]
    fn icon_theme_path(&self) -> Vec<String> {
        Vec::new()
    }

    fn get_layout(&self, parent_id: i32, recursion_depth: i32, property_names: Vec<String>) -> zbus::fdo::Result<(u32, WireRow)> {
        let state = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let revision = state.revision;
        let parent = if parent_id < 0 || state.menu.is_none() { ROOT_ID } else { (parent_id.max(0) as u32).min(i32::MAX as u32) };
        let row = match &state.menu {
            Some(menu) => wire_row(menu, parent, recursion_depth, &property_names),
            None => wire_row(&FlatMenu::default(), ROOT_ID, 0, &property_names),
        };
        Ok((revision, row))
    }

    fn get_group_properties(&self, ids: Vec<i32>, property_names: Vec<String>) -> zbus::fdo::Result<Vec<(i32, HashMap<String, OwnedValue>)>> {
        let state = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut out = Vec::new();
        if let Some(menu) = &state.menu {
            // An empty id list means "send all items" (the spec's GetGroupProperties case).
            let ids = if ids.is_empty() { menu.items.iter().map(|it| it.id as i32).collect() } else { ids };
            for id in ids {
                if let Some(item) = menu.item(id.max(0) as u32) {
                    out.push((id, wire_props(&item.props, &property_names)));
                }
            }
        }
        Ok(out)
    }

    fn get_property(&self, id: i32, name: &str) -> zbus::fdo::Result<OwnedValue> {
        let state = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(menu) = &state.menu else {
            return Err(zbus::fdo::Error::InvalidArgs(format!("no menu exported yet (item {id})")));
        };
        let Some(item) = menu.item(id.max(0) as u32) else {
            return Err(zbus::fdo::Error::InvalidArgs(format!("unknown item {id}")));
        };
        let props = wire_props(&item.props, &[name.to_string()]);
        props.into_iter().next().map(|(_, v)| v).ok_or_else(|| zbus::fdo::Error::InvalidArgs(format!("item {id} has no property {name}")))
    }

    fn get_property_names(&self, ids: Vec<i32>) -> zbus::fdo::Result<Vec<(i32, Vec<String>)>> {
        let state = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut out = Vec::new();
        if let Some(menu) = &state.menu {
            for id in ids {
                if let Some(item) = menu.item(id.max(0) as u32) {
                    // The per-item property names: the closure of what we export.
                    let names: &[&str] = if item.props.separator == Some(true) {
                        &["type", "enabled", "visible", "label"]
                    } else {
                        &["type", "label", "enabled", "visible", "children-display", "toggle-type", "toggle-state", "shortcut"]
                    };
                    out.push((id, names.iter().map(|&n| n.to_string()).collect()));
                }
            }
        }
        Ok(out)
    }

    fn event(&self, id: i32, event_id: &str, _data: OwnedValue, _timestamp: u32) -> zbus::fdo::Result<()> {
        // `clicked` activates; `opened`/`closed`/`hovered` just acknowledge (our menus are
        // always complete — nothing lazy to fetch on open).
        if event_id == "clicked" {
            self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).activate(id.max(0) as u32);
        }
        Ok(())
    }

    fn event_group(&self, events: Vec<(i32, String, OwnedValue, u32)>) -> zbus::fdo::Result<Vec<i32>> {
        let mut errors = Vec::new();
        let mut state = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        for (id, event_id, _data, _timestamp) in events {
            if event_id != "clicked" {
                continue;
            }
            let id = id.max(0) as u32;
            let archivable = state.menu.as_ref().and_then(|m| m.item(id)).is_some_and(|it| !it.action.is_empty() && it.props.enabled != Some(false));
            if !archivable {
                errors.push(id as i32);
            }
            state.activate(id);
        }
        Ok(errors)
    }

    fn about_to_show(&self, _id: i32) -> zbus::fdo::Result<bool> {
        // Nothing is lazily added: menus are always complete.
        Ok(false)
    }

    fn about_to_show_group(&self, ids: Vec<i32>) -> zbus::fdo::Result<(Vec<i32>, Vec<i32>)> {
        // No deferred items ever appear — but unknown ids must still be reported so the
        // importer can re-fetch the layout (that's what the first list is for).
        let state = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let stale =
            ids.into_iter().map(|id| id.max(0) as u32).filter(|id| state.menu.as_ref().and_then(|m| m.item(*id)).is_none()).map(|id| id as i32).collect();
        Ok((stale, Vec::new()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{MenuEntry, MenuModel};
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
    use zvariant::signature;

    #[test]
    fn activations_are_capped_and_wake_the_shell() {
        let mut state = MenuState::default();
        let hits = Arc::new(AtomicUsize::new(0));
        {
            let hits = hits.clone();
            state.wake = Some(Arc::new(move || {
                hits.fetch_add(1, AtomicOrdering::Relaxed);
            }));
        }
        state.menu = Some(demo());
        let go = state.menu.as_ref().and_then(|m| m.actions.iter().find(|(_, a)| **a == "file.new")).map(|(id, _)| *id).expect("a New item");
        for _ in 0..MAX_ACTIVATIONS + 5 {
            state.activate(go);
        }
        assert_eq!(state.activations.len(), MAX_ACTIVATIONS, "the queue is capped");
        // Overflow drops the oldest: the queued clicks are exactly the newest batch.
        assert_eq!(state.activations.back().map(|(id, _, _)| (id, go)), Some((&go, go)));
        assert_eq!(hits.load(AtomicOrdering::Relaxed), MAX_ACTIVATIONS + 5, "every recorded click wakes the shell");
    }

    #[test]
    fn dead_and_disabled_clicks_never_wake_the_shell() {
        let mut state = MenuState::default();
        let hits = Arc::new(AtomicUsize::new(0));
        {
            let hits = hits.clone();
            state.wake = Some(Arc::new(move || {
                hits.fetch_add(1, AtomicOrdering::Relaxed);
            }));
        }
        state.menu = Some(demo());
        state.activate(999_999); // no such item
        // The File title: a submenu, not a command.
        let file = state.menu.as_ref().and_then(|m| m.item(1)).expect("File exists");
        assert_eq!(file.action, "", "the File item is a title");
        state.activate(1);
        assert!(state.activations.is_empty(), "no click was recorded");
        assert_eq!(hits.load(AtomicOrdering::Relaxed), 0, "and nothing woke");
    }

    fn demo() -> FlatMenu {
        FlatMenu::build(&MenuModel::top(vec![
            MenuEntry::command("File", "", false).submenu(vec![
                MenuEntry::command("New…", "file.new", true),
                MenuEntry::Separator,
                MenuEntry::command("Save", "file.save", true).shortcut("Ctrl+S"),
            ]),
            MenuEntry::command("Help", "", false).submenu(vec![MenuEntry::command("About PhotoCraft", "help.about", true)]),
        ]))
    }

    fn map_of(props: &crate::layout::Props, names: &[String]) -> HashMap<String, OwnedValue> {
        wire_props(props, names)
    }

    fn one(menu: &FlatMenu, id: u32) -> HashMap<String, OwnedValue> {
        menu.item(id).map(|it| map_of(&it.props, &[])).unwrap_or_default()
    }

    fn variant64(value: u64) -> OwnedValue {
        Value::from(value).try_into_owned().unwrap()
    }

    /// D-Bus variants decode back with a type; small helper for the assertions below.
    fn as_str(value: OwnedValue) -> Option<String> {
        String::try_from(value).ok()
    }
    fn as_bool(value: OwnedValue) -> Option<bool> {
        bool::try_from(value).ok()
    }

    #[test]
    fn reply_signature_is_the_spec_one() {
        // Strict importers (GDBus/dbus-glib) demarshall GetLayout replies against a
        // signature they pinned from the spec: the reply must be (u, (ia{sv}av)).
        assert_eq!(<(u32, WireRow) as zvariant::Type>::SIGNATURE, &signature!("(u(ia{sv}av))"));
    }

    #[test]
    fn wire_props_carry_the_properties() {
        let menu = demo();
        let map = one(&menu, 1);
        assert_eq!(map.get("label").cloned().and_then(as_str), Some("File".into()));
        assert_eq!(map.get("enabled").cloned().and_then(as_bool), Some(false));
        assert_eq!(map.get("type").cloned().and_then(as_str), Some("standard".into()));
        assert_eq!(map.get("children-display").cloned().and_then(as_str), Some("submenu".into()));
        assert!(!map.contains_key("shortcut"));
        assert!(!map.contains_key("toggle-state"));
    }

    #[test]
    fn wire_props_filtering() {
        let menu = demo();
        let props = menu.item(1).map(|it| it.props.clone()).unwrap_or_default();
        let map = map_of(&props, &["type".into()]);
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("type").cloned().and_then(as_str), Some("standard".into()));

        // A separator's only interesting property is its type (it stays enabled=false).
        let props = menu.item(3).map(|it| it.props.clone()).unwrap_or_default();
        let map = map_of(&props, &[]);
        assert_ne!(map.get("type").cloned().and_then(as_str), Some("standard".into()));
        assert_eq!(map.get("type").cloned().and_then(as_str), Some("separator".into()));
        assert_eq!(map.get("enabled").cloned().and_then(as_bool), Some(false));
    }

    #[test]
    fn wire_props_shortcut_is_an_array_of_arrays_of_strings() {
        let menu = demo();
        let props = menu.item(4).map(|it| it.props.clone()).unwrap_or_default();
        let map = map_of(&props, &[]);
        let shortcut = match map.get("shortcut") {
            Some(v) => v.clone(),
            None => panic!("shortcut property missing"),
        };
        let chords = match Value::from(shortcut) {
            Value::Array(a) => a,
            v => panic!("shortcut is not an array: {v}"),
        };
        assert_eq!(chords.signature(), &signature!("aas"));
        // The one chord is `["Control", "S"]` (canonical dbusmenu modifier names).
        let chord = match chords.iter().next() {
            Some(v) => v.clone(),
            None => panic!("no chords"),
        };
        let chord = match chord {
            Value::Array(a) => a,
            v => panic!("chord is not an array: {v}"),
        };
        let keys: Vec<String> = chord.iter().filter_map(|v| String::try_from(v.clone()).ok()).collect();
        assert_eq!(keys, vec!["Control".to_string(), "S".to_string()]);
    }

    #[test]
    fn get_layout_depth_and_ids() {
        let menu = demo();
        let root = wire_row(&menu, ROOT_ID, -1, &[]);
        assert_eq!(root.id, 0);
        assert_eq!(root.children.len(), 2);
        // Depth 0: no children regardless of structure.
        let shallow = wire_row(&menu, ROOT_ID, 0, &[]);
        assert!(shallow.children.is_empty());
    }

    /// Test-side mirror of [`WireRow`] (owned so it can implement `Deserialize`); used to
    /// demarshall the wire exactly as an importer would.
    #[derive(Debug, zvariant::Type, serde::Deserialize)]
    struct DecodedRow {
        id: i32,
        props: HashMap<String, OwnedValue>,
        children: Vec<OwnedValue>,
    }

    #[test]
    fn get_layout_reply_round_trips_on_the_wire() {
        // Byte-level check that even a hinting importer can decode: encode the GetLayout
        // reply shape `(u(ia{sv}av))` and demarshall it back, then unpack rows as the
        // importers do — nested children are variants of `(ia{sv}av)` rows, not of `(ivv)`.
        let menu = demo();
        let reply = (7u32, wire_row(&menu, ROOT_ID, -1, &[]));
        let ctxt = zvariant::serialized::Context::new_dbus(zvariant::LE, 0);
        let sig = signature!("(u(ia{sv}av))");
        let bytes = zvariant::to_bytes_for_signature(ctxt, &sig, &reply).expect("encoding the GetLayout reply");
        let ((rev, row), _): ((u32, DecodedRow), usize) = bytes.deserialize_for_signature(&sig).expect("decoding the GetLayout reply");
        assert_eq!(rev, 7);
        // The unnamed root presents itself as a submenu with no label of its own.
        assert_eq!(row.id, 0);
        assert!(!row.props.contains_key("label"));
        assert_eq!(row.children.len(), 2);

        let (id, props, children) = row_child_like_an_importer(&row.children[0]);
        assert_eq!(id, 1);
        assert_eq!(props.get("label").cloned().and_then(as_str), Some("File".into()));
        assert_eq!(children.len(), 3);

        let (leaf_id, leaf_props, leaf_children) = row_child_like_an_importer(&children[0]);
        assert_eq!(leaf_id, 2);
        assert_eq!(leaf_props.get("label").cloned().and_then(as_str), Some("New…".into()));
        assert_eq!(leaf_children.len(), 0); // "New…" has no submenu
    }

    fn row_child_like_an_importer(child: &OwnedValue) -> (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>) {
        let value = match Value::try_from(child).expect("child value") {
            Value::Value(inner) => *inner,
            row => row,
        };
        <zvariant::Structure>::try_from(value).expect("row structure").try_into().expect("row fields")
    }

    #[test]
    fn event_records_clicks_via_the_interface() {
        let state = Arc::new(Mutex::new(MenuState::default()));
        state.lock().unwrap().menu = Some(demo());
        let iface = MenuBarIface { state: state.clone() };
        iface.event(2, "clicked", variant64(0), 0).unwrap();
        iface.event(3, "hovered", variant64(0), 0).unwrap();
        // One recorded click, carrying the action token (separators don't activate).
        let state = state.lock().unwrap();
        assert_eq!(state.activations.len(), 1);
        assert_eq!(state.activations[0].0, 2);
        assert_eq!(state.activations[0].2, "file.new");
    }

    #[test]
    fn event_group_reports_unknown_ids() {
        let state = Arc::new(Mutex::new(MenuState::default()));
        state.lock().unwrap().menu = Some(demo());
        let iface = MenuBarIface { state };
        let errors = iface.event_group(vec![(2, "clicked".into(), variant64(0), 0), (999, "clicked".into(), variant64(0), 0)]).unwrap();
        assert_eq!(errors, vec![999]);
    }
}
