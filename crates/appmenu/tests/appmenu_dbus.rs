//! A session-bus round trip: the exporter publishes a model, a client (this test's second
//! connection) fetches `GetLayout`, reads properties and sends `Event` clicks.
//! Skips gracefully when no session bus is available (never fails on a bare CI box).
//!
//! The whole file is the Linux backend's: zbus/zvariant/serde are target-gated dependencies,
//! so without this gate `cargo test --workspace` / `clippy --all-targets` fail to compile the
//! test crate on macOS, Windows and wasm (where the exporter itself is the no-op stub).
#![cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]

use std::collections::HashMap;
use std::time::Duration;

use photocraft_appmenu::{AppMenu, MenuEntry, MenuModel};
use zbus::blocking::Proxy;
use zvariant::OwnedValue;

/// `GetLayout`'s row type, exactly `(ia{sv}av)`.
#[derive(Debug, zvariant::Type, serde::Deserialize)]
struct TestRow {
    id: i32,
    props: HashMap<String, OwnedValue>,
    children: Vec<OwnedValue>,
}

fn demo_model() -> MenuModel {
    MenuModel::top(vec![
        MenuEntry::command("File", "", false).submenu(vec![
            MenuEntry::command("New…", "file.new", true),
            MenuEntry::Separator,
            MenuEntry::command("Open…", "file.open", true),
        ]),
        MenuEntry::command("View", "", false).submenu(vec![MenuEntry::command("Show Grid", "view.grid", true).checked(true).shortcut("Ctrl+'")]),
    ])
}

fn property(proxy: &Proxy<'_>, id: u32, name: &str) -> Option<OwnedValue> {
    proxy.call("GetProperty", &(id as i32, name)).ok()
}

fn wait_reply() {
    // The worker publishes a coalesced batch within a scan tick; waiting generously keeps
    // the test stable on loaded CI machines without slowsleep anywhere else.
    std::thread::sleep(Duration::from_millis(300));
}

/// Unwraps an owned variant of `(ia{sv}av)` back into its parts (test-side decode of what
/// importers receive as `GetLayout` children).
// A decode helper: the workspace test policy (clippy.toml `allow-*-in-tests`) covers
// `#[test]` functions, not plain helpers, so the fails-loudly expectations are marked here.
#[allow(clippy::expect_used, clippy::panic)]
fn row_child(child: &OwnedValue) -> (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>) {
    let value = zvariant::Value::try_from(child).expect("child value");
    let row = match value {
        zvariant::Value::Value(inner) => *inner,
        row => row,
    };
    let structure = zvariant::Structure::try_from(row).expect("row structure");
    <(i32, HashMap<String, OwnedValue>, Vec<OwnedValue>)>::try_from(structure).expect("row fields")
}

#[test]
fn client_sees_layout_and_clicks_come_back() {
    // No session bus (bare CI): skip instead of failing.
    let menu = match AppMenu::start("photocraft-test", std::sync::Arc::new(|| {})) {
        Ok(menu) => menu,
        Err(e) => {
            eprintln!("skipping (no usable session bus): {e}");
            return;
        }
    };
    menu.replace(&demo_model());
    // The worker reports its unique name once the connection is up.
    let mut name = None;
    for _ in 0..100 {
        if let Some(n) = menu.bus_name() {
            name = Some(n);
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let Some(name) = name else {
        eprintln!("skipping (exporter never connected)");
        return;
    };

    let client = zbus::blocking::Connection::session().expect("client connection");
    let proxy = Proxy::new(&client, name.as_str(), "/MenuBar", "com.canonical.dbusmenu").expect("proxy");

    // The layout: root row with the two top menus deeper in, full property detail.
    let (revision, row): (u32, TestRow) = proxy.call("GetLayout", &(0i32, -1i32, Vec::<String>::new())).expect("GetLayout");
    assert!(revision >= 1);
    assert_eq!(row.id, 0);
    assert_eq!(row.children.len(), 2);
    // The root item presents itself as a submenu (the menu bar).
    assert_eq!(row.props.get("children-display").and_then(|v| String::try_from(v.clone()).ok()), Some("submenu".into()));

    // The first top menu ("File") is itself a row with three children, arriving as a
    // variant-wrapped `(ia{sv}av)` — exactly what importers unpack.
    let file = row_child(&row.children[0]);
    assert_eq!(file.0, 1);
    assert_eq!(file.1.get("label").and_then(|v| String::try_from(v.clone()).ok()), Some("File".into()));
    assert_eq!(file.2.len(), 3);
    let new_item = row_child(&file.2[0]);
    assert_eq!(new_item.0, 2);
    assert_eq!(new_item.1.get("label").and_then(|v| String::try_from(v.clone()).ok()), Some("New…".into()));

    // read property semantics on the leaves (ids assigned in reading order).
    assert_eq!(property(&proxy, 2, "label").and_then(|v| String::try_from(v).ok()), Some("New…".into()));
    // Separators exist as items too (id 3), but can't be clicked.
    assert!(menu.try_event().is_none());

    // A click on New… comes back as one activation.
    proxy.call::<_, _, ()>("Event", &(2i32, "clicked", zvariant::Value::from(""), 0u32)).expect("Event");
    match menu.try_event().expect("the click should arrive") {
        photocraft_appmenu::MenuEvent::Activated { label, action, id } => {
            assert_eq!(action, "file.new");
            assert_eq!(id, 2);
            assert_eq!(label, "New…");
        }
    }

    // Non-click events are acknowledged without an activation.
    proxy.call::<_, _, ()>("Event", &(2i32, "opened", zvariant::Value::from(""), 0u32)).expect("Event");
    assert!(menu.try_event().is_none());

    // A revision bump on re-publish: replace with a single different item, wait for the
    // worker tick and the next GetLayout reports a higher revision.
    let before = revision;
    menu.replace(&demo_model());
    wait_reply();
    let (after, _): (u32, TestRow) = proxy.call("GetLayout", &(0i32, 0i32, Vec::<String>::new())).expect("GetLayout after republish");
    assert!(after > before, "revision should advance: {before} → {after}");

    menu.shutdown();
}
