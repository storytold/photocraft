//! A WebAssembly plug-in's smart filter re-opens the plug-in's own dialog (#16425).

use photocraft_plugins::{Limits, Plugin, registry};
use serde_json::json;

use crate::smart_filter_dialog::tests::{app, smart};

const ID: &str = "test.smartfilter.edit";

/// Installs a plug-in with two parameters, `amount` (0..10, default 5) and one that is itself
/// called `params` (0..1, default 0), that leaves the pixels as they are.
fn install() {
    let manifest = format!(
        r#"{{"id":"{ID}","name":"Amount Test","version":"1","kind":"filter","params":{{"amount":{{"type":"number","min":0,"max":10,"default":5}},"params":{{"type":"number","min":0,"max":1,"default":0}}}}}}"#
    );
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

/// The settings a plug-in smart filter stores, as `plugin.run` records them: `{id, params}`, the
/// re-render reading only `params`. `amount` and the plug-in's own `params` key sit inside it.
fn assert_stored(stored: &serde_json::Value, amount: f64) {
    assert_eq!(stored["id"], ID);
    assert_eq!(stored["params"]["amount"].as_f64(), Some(amount), "the edit is where the re-render reads it: {stored}");
    assert_eq!(stored["params"]["params"].as_f64(), Some(1.0), "a plug-in key called `params` is kept: {stored}");
    assert!(stored.get("amount").is_none(), "no plug-in key beside `params`: {stored}");
}

/// Double-clicks the last smart filter of `layer`: the plug-in dialog it opens and the values shown.
fn open_last(app: &mut crate::PhotocraftApp, layer: photocraft_doc::LayerId) -> (craft::ui::dialog::DialogId, serde_json::Value) {
    let sm = smart(app, layer).clone();
    let ctx = app.ctx().clone();
    super::open_editor(app, &ctx, layer.0, &sm, sm.smart_filters.len() - 1);
    crate::dialogs::testing::show(app);
    let open = app.framework.dialogs.open_dialogs().pop().expect("the plug-in's dialog opens");
    (open.id, open.fields.expect("a dialog with fields"))
}

/// Double-clicking a plug-in's smart filter opens the plug-in's dialog on the recorded settings,
/// previews the edit on the canvas, and OK updates that filter in one step, stored the way
/// `plugin.run` records it, so the re-render reads the edit and the dialog re-opens on it.
#[test]
fn a_plugin_smart_filter_reopens_its_dialog_and_ok_updates_it() {
    let _registry = crate::plugin_ui::registry_guard();
    install();
    let (mut app, layer) = app(8, "rgb");
    app.run("plugin.run", json!({"id": ID, "params": {"amount": 7, "params": 1}})).unwrap();
    let sm = smart(&app, layer).clone();
    let index = sm.smart_filters.len() - 1;
    assert_eq!(sm.smart_filters[index].command, "plugin.run");

    let ctx = app.ctx().clone();
    let (dialog, shown) = open_last(&mut app, layer);
    assert_eq!(shown["amount"].as_f64(), Some(7.0), "the recorded value, not the default 5");
    app.framework.dialogs.set_json_field(&ctx, dialog, "amount", json!(3)).unwrap();

    let idx = app.session.active_index().unwrap();
    crate::canvas::ensure_filter_preview(&mut app, idx, &ctx).expect("the edit previews");
    let preview = app.filter_preview.as_ref().unwrap();
    assert_eq!(preview.key.command, "layer.smartFilter.setParams");
    let previewed = match &preview.result.as_ref().expect("the preview's document").layer(layer).unwrap().content {
        photocraft_doc::LayerContent::Smart(sm) => sm.smart_filters[index].params.clone(),
        other => panic!("expected a smart object, got {}", other.kind_name()),
    };
    assert_stored(&previewed, 3.0);

    let steps = app.session.active().unwrap().history.past_len();
    crate::dialogs::testing::confirm(&mut app, dialog).unwrap();
    let after = smart(&app, layer);
    assert_eq!(after.smart_filters.len(), sm.smart_filters.len(), "the filter is edited, not added again");
    assert_stored(&after.smart_filters[index].params, 3.0);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);

    let (_, reopened) = open_last(&mut app, layer);
    assert_eq!(reopened["amount"].as_f64(), Some(3.0), "the edit, re-opened");
    assert_eq!(reopened["params"].as_f64(), Some(1.0), "the plug-in's `params` setting, re-opened");
    registry::remove(ID);
}

/// The edit stays on the plug-in the filter recorded: a stored setting that names another plug-in
/// (`layer.smartFilter.setParams` stores what it is given) is not read as the filter's plug-in.
#[test]
fn a_setting_called_id_does_not_change_the_plugin_an_edit_stores() {
    let _registry = crate::plugin_ui::registry_guard();
    install();
    let (mut app, layer) = app(8, "rgb");
    app.run("plugin.run", json!({"id": ID, "params": {"amount": 7, "params": 1}})).unwrap();
    let planted = json!({"id": ID, "params": {"amount": 7, "params": 1, "id": "another.plugin"}});
    app.run("layer.smartFilter.setParams", json!({"layer": layer.0, "params": planted, "replace": true})).unwrap();

    let ctx = app.ctx().clone();
    let (dialog, _) = open_last(&mut app, layer);
    app.framework.dialogs.set_json_field(&ctx, dialog, "amount", json!(3)).unwrap();
    crate::dialogs::testing::confirm(&mut app, dialog).unwrap();
    let after = smart(&app, layer);
    let stored = &after.smart_filters[after.smart_filters.len() - 1].params;
    assert_stored(stored, 3.0);
    assert!(stored["params"].get("id").is_none(), "only the plug-in's own settings are stored: {stored}");
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
