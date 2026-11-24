//! A WebAssembly plug-in's smart filter re-opens the plug-in's own dialog (#16425).

use photocraft_plugins::{Limits, Plugin, registry};
use serde_json::json;

use crate::smart_filter_dialog::tests::{app, smart};

const ID: &str = "test.smartfilter.edit";

/// Installs a plug-in with one parameter, `amount` (0..10, default 5), that leaves the pixels as
/// they are.
fn install() {
    let manifest =
        format!(r#"{{"id":"{ID}","name":"Amount Test","version":"1","kind":"filter","params":{{"amount":{{"type":"number","min":0,"max":10,"default":5}}}}}}"#);
    let src = format!(
        r#"(module
  (memory (export "memory") 1)
  (data (i32.const 16) "{escaped}")
  (func (export "pc_abi_version") (result i32) (i32.const 1))
  (func (export "pc_manifest") (result i64)
    (i64.or (i64.shl (i64.const {len}) (i64.const 32)) (i64.const 16)))
  (func (export "pc_alloc") (param $n i32) (result i32) (local $old i32)
    (local.set $old (memory.grow (i32.shr_u (i32.add (local.get $n) (i32.const 65535)) (i32.const 16))))
    (if (result i32) (i32.eq (local.get $old) (i32.const -1))
      (then (i32.const 0)) (else (i32.mul (local.get $old) (i32.const 65536)))))
  (func (export "pc_filter") (param $buf i32) (param $len i32) (param $w i32) (param $h i32)
    (param $ch i32) (param $fmt i32) (param $pp i32) (param $pl i32) (result i32)
    (i32.const 0))
)"#,
        escaped = manifest.replace('"', "\\\""),
        len = manifest.len()
    );
    let bytes = wat::parse_str(&src).unwrap();
    registry::install(Plugin::load(&bytes, Limits::default()).unwrap());
}

/// Double-clicking a plug-in's smart filter opens the plug-in's dialog on the recorded settings,
/// and OK updates that filter in one step, stored the way `plugin.run` records it, so the
/// re-render reads the edit.
#[test]
fn a_plugin_smart_filter_reopens_its_dialog_and_ok_updates_it() {
    let _registry = crate::plugin_ui::registry_guard();
    install();
    let (mut app, layer) = app(8, "rgb");
    app.run("plugin.run", json!({"id": ID, "amount": 7})).unwrap();
    let sm = smart(&app, layer).clone();
    let index = sm.smart_filters.len() - 1;
    assert_eq!(sm.smart_filters[index].command, "plugin.run");

    let ctx = egui::Context::default();
    super::open_editor(&mut app, &ctx, layer.0, &sm, index);
    crate::dialogs::testing::show(&mut app);
    let open = app.framework.dialogs.open_dialogs().pop().expect("the plug-in's dialog opens");
    let shown = open.fields.expect("a dialog with fields");
    assert_eq!(shown["amount"].as_f64(), Some(7.0), "the recorded value, not the default 5");

    app.framework.dialogs.set_json_field(&ctx, open.id, "amount", json!(3)).unwrap();
    let steps = app.session.active().unwrap().history.past_len();
    crate::dialogs::testing::confirm(&mut app, open.id).unwrap();
    let after = smart(&app, layer);
    assert_eq!(after.smart_filters.len(), sm.smart_filters.len(), "the filter is edited, not added again");
    let stored = &after.smart_filters[index].params;
    assert_eq!(stored["id"], ID);
    assert_eq!(stored["params"]["amount"].as_f64(), Some(3.0), "the edit is where the re-render reads it: {stored}");
    assert!(stored.get("amount").is_none(), "no plug-in key beside `params`: {stored}");
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
    registry::remove(ID);
}

/// A plug-in that is no longer installed says so instead of doing nothing.
#[test]
fn a_smart_filter_of_a_removed_plugin_says_it_is_not_installed() {
    let _registry = crate::plugin_ui::registry_guard();
    install();
    let (mut app, layer) = app(8, "rgb");
    app.run("plugin.run", json!({"id": ID, "amount": 7})).unwrap();
    registry::remove(ID);
    let sm = smart(&app, layer).clone();
    let index = sm.smart_filters.len() - 1;

    super::open_editor(&mut app, &egui::Context::default(), layer.0, &sm, index);
    crate::dialogs::testing::show(&mut app);
    assert!(app.framework.dialogs.open_dialogs().is_empty(), "no dialog without the plug-in");
    assert!(app.ui.status_error && app.ui.status.contains(ID), "the reason is shown: {:?}", app.ui.status);
}
