//! Plug-in runs against a real session, with modules assembled from WebAssembly text.

use std::sync::Arc;

use serde_json::{Value, json};

use crate::Session;
use crate::plugin::craft_plugin::bundle::Bundle;

/// A command module that calls `calls` (command, params JSON) in order. A refused or failed call
/// fails the run when `strict`; otherwise its message becomes the output and the run goes on.
/// `tail` is WAT run after the calls (a trap, an endless loop), before returning 0.
fn module(calls: &[(&str, &str)], strict: bool, tail: &str) -> Vec<u8> {
    let mut data = String::new();
    let mut body = String::new();
    let mut at = 0usize;
    for (command, params) in calls {
        let (c, p) = (at, at + command.len());
        at = p + params.len();
        let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
        data.push_str(&format!(r#"(data (i32.const {c}) "{}") (data (i32.const {p}) "{}")"#, esc(command), esc(params)));
        let on_error = if strict {
            "(return (i32.const 1))".to_string()
        } else {
            "(call $set_output (i32.const 8192) (call $reply_read (i32.const 8192) (call $reply_len)))".to_string()
        };
        body.push_str(&format!(
            "(if (i32.ne (call $call (i32.const {c}) (i32.const {}) (i32.const {p}) (i32.const {})) (i32.const 0)) (then {on_error}))\n",
            command.len(),
            params.len()
        ));
    }
    let wat = format!(
        r#"(module
          (import "craft" "call" (func $call (param i32 i32 i32 i32) (result i32)))
          (import "craft" "reply_len" (func $reply_len (result i32)))
          (import "craft" "reply_read" (func $reply_read (param i32 i32) (result i32)))
          (import "craft" "set_output" (func $set_output (param i32 i32)))
          (memory (export "memory") 1)
          {data}
          (func (export "craft_run") (param $n i32) (result i32) {body} {tail} (i32.const 0)))"#
    );
    wat::parse_str(&wat).unwrap_or_else(|e| panic!("{e}\n{wat}"))
}

/// The `.craftplugin` of command plug-in `id` for PhotoCraft, asking for `access`.
fn bundle(id: &str, access: &str, wasm: Vec<u8>) -> Vec<u8> {
    let manifest = json!({
        "id": id, "name": "Test Plug-in", "version": "1.0.0", "main": "plugin.wasm",
        "apps": {"photocraft": {"api": 6}},
        "permissions": {"access": access},
        "commands": [{"id": "go", "label": "Go"}]
    });
    Bundle { manifest: manifest.to_string(), wasm, assets: vec![] }.encode()
}

/// A filter module (PhotoCraft's own ABI 1) calling itself `id` that sets every sample to 0.5.
fn grey_filter(id: &str) -> Vec<u8> {
    let manifest = format!(r#"{{"id":"{id}","name":"Grey {id}","version":"1","kind":"filter","params":{{}}}}"#);
    let src = format!(
        r#"(module
  (memory (export "memory") 1)
  (data (i32.const 16) "{data}")
  (func (export "pc_abi_version") (result i32) (i32.const 1))
  (func (export "pc_manifest") (result i64) (i64.or (i64.shl (i64.const {len}) (i64.const 32)) (i64.const 16)))
  (func (export "pc_alloc") (param $n i32) (result i32) (local $old i32)
    (local.set $old (memory.grow (i32.shr_u (i32.add (local.get $n) (i32.const 65535)) (i32.const 16))))
    (if (result i32) (i32.eq (local.get $old) (i32.const -1)) (then (i32.const 0)) (else (i32.mul (local.get $old) (i32.const 65536)))))
  (func (export "pc_filter") (param $buf i32) (param $len i32) (param $w i32) (param $h i32)
    (param $ch i32) (param $fmt i32) (param $pp i32) (param $pl i32) (result i32) (local $i i32)
    (block $done (loop $l
      (br_if $done (i32.ge_u (local.get $i) (local.get $len)))
      (f32.store (i32.add (local.get $buf) (local.get $i)) (f32.const 0.5))
      (local.set $i (i32.add (local.get $i) (i32.const 4)))
      (br $l)))
    (i32.const 0)))"#,
        data = manifest.replace('"', "\\\""),
        len = manifest.len()
    );
    wat::parse_str(src).unwrap()
}

fn base64(bytes: &[u8]) -> String {
    photocraft_paint::tile::b64_encode(bytes)
}

/// A session with a document and command plug-in `id` installed for the session.
fn with_plugin(id: &str, access: &str, wasm: Vec<u8>) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 32, "height": 24})).unwrap();
    let r = s.execute("plugin.install", json!({"data": base64(&bundle(id, access, wasm)), "name": "test.craftplugin"})).unwrap();
    assert_eq!(r["installed"], true);
    assert_eq!(r["stored"], false, "without a store, a plug-in is kept for the session");
    s
}

fn layers(s: &Session) -> usize {
    s.active().unwrap().doc.walk().len()
}

fn steps(s: &Session) -> usize {
    s.active().unwrap().history.past_len()
}

const NEW_LAYER: (&str, &str) = ("layer.new.layer", r#"{"name":"From a plug-in"}"#);

#[test]
fn a_write_plugin_runs_real_commands_as_one_undo_step_named_after_it() {
    let mut s = with_plugin("io.example.layers", "write", module(&[NEW_LAYER, NEW_LAYER], true, ""));
    let (before, undo) = (layers(&s), steps(&s));
    let r = s.execute("plugin.run", json!({"id": "io.example.layers"})).unwrap();
    assert_eq!(r["calls"], 2);
    assert_eq!(r["command"], "go");
    assert_eq!(layers(&s), before + 2);
    assert_eq!(steps(&s), undo + 1, "two commands, one undo step");
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Test Plug-in"));
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(layers(&s), before, "one Undo takes the whole run back");
    // The run is recorded once, not its steps.
    assert_eq!(s.journal.iter().filter(|(id, _)| id == "layer.new.layer").count(), 0);
    assert_eq!(s.journal.iter().filter(|(id, _)| id == "plugin.run").count(), 1);
}

#[test]
fn a_read_plugin_that_edits_is_put_back_and_refused() {
    let mut s = with_plugin("io.example.reader", "read", module(&[NEW_LAYER], false, ""));
    let (before, undo) = (layers(&s), steps(&s));
    let doc = s.active().unwrap().doc.clone();
    let r = s.execute("plugin.run", json!({"id": "io.example.reader"})).unwrap();
    let message = r["output"].as_str().unwrap();
    assert!(message.contains("may only read it"), "{message}");
    assert_eq!(layers(&s), before);
    assert!(Arc::ptr_eq(&doc, &s.active().unwrap().doc), "the document is the one from before the call");
    assert_eq!(steps(&s), undo, "nothing to undo");
    assert!(!s.journal.iter().any(|(id, _)| id == "layer.new.layer"), "the refused call isn't journaled");
}

#[test]
fn a_read_plugin_may_read_and_leaves_no_step() {
    let mut s = with_plugin("io.example.inspect", "read", module(&[("document.inspect", "{}"), ("layer.list", "{}")], true, ""));
    let (doc, undo) = (s.active().unwrap().doc.clone(), steps(&s));
    let r = s.execute("plugin.run", json!({"id": "io.example.inspect"})).unwrap();
    assert_eq!(r["calls"], 2);
    assert!(Arc::ptr_eq(&doc, &s.active().unwrap().doc));
    assert_eq!(steps(&s), undo, "a run that changed nothing leaves no step");
}

#[test]
fn refused_commands_and_file_paths_are_refused() {
    for (call, why) in [
        (("file.save", "{}"), "isn't available to plugins"),
        (("file.open", r#"{"path":"/etc/hosts"}"#), "isn't available to plugins"),
        (("edit.undo", "{}"), "isn't available to plugins"),
        (("prefs.set", "{}"), "isn't available to plugins"),
        (("history.newSnapshot", "{}"), "isn't available to plugins"),
        (("document.activate", "{}"), "isn't available to plugins"),
        (("actions.play", "{}"), "isn't available to plugins"),
        (("edit.copy", "{}"), "isn't available to plugins"),
        (("plugin.list", "{}"), "isn't available to plugins"),
        (("layer.smartObjects.replaceContents", r#"{"path":"/etc/hosts"}"#), "can't name files"),
        (("edit.assignProfile", r#"{"profile":"/tmp/x.icc"}"#), "can't name files"),
        (("layer.duplicate", r#"{"document":"new"}"#), "active document only"),
        (("image.applyImage", r#"{"source":{"document":1}}"#), "active document only"),
        (("swatches.add", "{}"), "isn't available to plugins"),
    ] {
        let mut s = with_plugin("io.example.sneaky", "write", module(&[call], false, ""));
        let undo = steps(&s);
        let r = s.execute("plugin.run", json!({"id": "io.example.sneaky"})).unwrap();
        let message = r["output"].as_str().unwrap_or_default();
        assert!(message.contains(why), "{}: {message}", call.0);
        assert_eq!(steps(&s), undo, "{}", call.0);
    }
}

#[test]
fn a_command_outside_the_manifest_allowlist_is_refused() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    let manifest = json!({"id": "io.example.narrow", "name": "Narrow", "version": "1.0.0", "main": "p.wasm",
        "apps": {"photocraft": {"api": 6}}, "permissions": {"access": "write", "commands": ["select.*"]}});
    let bytes = Bundle { manifest: manifest.to_string(), wasm: module(&[NEW_LAYER], false, ""), assets: vec![] }.encode();
    let before = layers(&s);
    s.execute("plugin.install", json!({"data": base64(&bytes)})).unwrap();
    let r = s.execute("plugin.run", json!({"id": "io.example.narrow"})).unwrap();
    assert!(r["output"].as_str().unwrap().contains("asked permission for"), "{r}");
    assert_eq!(layers(&s), before);
}

#[test]
fn a_trapping_or_endless_plugin_leaves_the_document_unchanged() {
    for (tail, fuel) in [("unreachable", None), ("(loop $l (br $l))", Some(200_000))] {
        let mut s = with_plugin("io.example.crash", "write", module(&[NEW_LAYER, NEW_LAYER], true, tail));
        let (doc, undo) = (s.active().unwrap().doc.clone(), steps(&s));
        let journal = s.journal.len();
        let mut params = json!({"id": "io.example.crash"});
        if let Some(f) = fuel {
            params["fuel"] = json!(f);
        }
        let e = s.execute("plugin.run", params).unwrap_err().to_string();
        assert!(e.contains("Test Plug-in"), "{e}");
        assert!(Arc::ptr_eq(&doc, &s.active().unwrap().doc), "{tail}: the edits before the failure are rolled back");
        assert_eq!(steps(&s), undo);
        assert_eq!(s.journal.len(), journal, "the rolled-back calls aren't journaled");
    }
}

#[test]
fn a_command_plugin_needs_no_pixel_layer_or_document() {
    // A command plug-in isn't a filter: no pixel layer (a Bitmap document refuses filters) and no
    // document are needed to start one; its calls have their own preconditions.
    let mut s = Session::new();
    s.execute("plugin.install", json!({"data": base64(&bundle("io.example.idle", "read", module(&[], true, "")))})).unwrap();
    assert!(s.is_enabled_with("plugin.run", &json!({"id": "io.example.idle"})));
    assert!(!s.is_enabled_with("plugin.run", &json!({"id": "no.such.filter"})));
    let r = s.execute("plugin.run", json!({"id": "io.example.idle"})).unwrap();
    assert_eq!(r["calls"], 0);
    assert!(s.execute("plugin.run", json!({"id": "io.example.idle", "command": "nope"})).is_err());
    assert!(s.execute("plugin.run", json!({"id": "io.example.idle", "command": 3})).is_err());
}

#[test]
fn settings_are_bounded_and_kept_per_plugin() {
    let mut all = Default::default();
    super::set_setting(&mut all, "a", "k", Some("{\"x\": 1}")).unwrap();
    assert_eq!(all["a"]["k"], r#"{"x":1}"#);
    assert!(super::set_setting(&mut all, "a", &"k".repeat(129), Some("1")).is_err());
    assert!(super::set_setting(&mut all, "a", "big", Some(&format!("\"{}\"", "x".repeat(64 * 1024)))).is_err());
    assert!(super::set_setting(&mut all, "a", "bad", Some("not json")).is_err());
    super::set_setting(&mut all, "a", "k", None).unwrap();
    assert!(!all.contains_key("a"));
}

#[test]
fn settings_and_catalogs_round_trip_through_the_settings_file() {
    let mut s = Session::new();
    s.plugins.saved.catalogs.push("https://raw.githubusercontent.com/example/catalog/main/index.json".into());
    super::set_setting(&mut s.plugins.saved.settings, "io.example.p", "k", Some("[1,null]")).unwrap();
    let saved = s.saved_prefs();
    let text = serde_json::to_value(&saved).unwrap();
    assert_eq!(text["plugins"]["settings"]["io.example.p"]["k"], "[1,null]", "kept as text: the settings file has no null");
    let back: crate::prefs::SavedPrefs = serde_json::from_value(text).unwrap();
    let mut t = Session::new();
    t.set_saved_prefs(back);
    assert_eq!(t.plugins.saved, s.plugins.saved);
    // Without plug-ins the file has no `plugins` section.
    assert!(serde_json::to_value(Session::new().saved_prefs()).unwrap().get("plugins").is_none());
}

#[test]
fn install_review_list_and_remove_through_the_store() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Session::new();
    s.plugins.open_store(dir.path());
    let bytes = bundle("io.example.stored", "write", module(&[], true, ""));
    let data = base64(&bytes);
    // The confirmation: described, nothing written.
    let review = s.execute("plugin.install", json!({"data": data, "review": true})).unwrap();
    assert_eq!(review["installed"], false);
    assert_eq!(review["permissions"]["access"], "write");
    assert_eq!(review["kind"], "command");
    assert_eq!(review["sha256"], crate::plugin::craft_plugin::sha256_hex(&bytes));
    assert!(s.plugins.list().is_empty());
    assert!(std::fs::read_dir(dir.path()).map_or(true, |mut d| d.next().is_none()), "nothing is written before the install");
    // Another bundle than the one reviewed isn't installed.
    let e = s.execute("plugin.install", json!({"data": data, "sha256": "0".repeat(64)})).unwrap_err();
    assert!(e.to_string().contains("reviewed"), "{e}");
    s.execute("plugin.install", json!({"data": data, "review": true})).unwrap();
    let r = s.execute("plugin.install", json!({"data": data, "sha256": review["sha256"]})).unwrap();
    assert_eq!((r["installed"].clone(), r["stored"].clone()), (json!(true), json!(true)));
    // A new session reads it back from the folder.
    let mut again = Session::new();
    again.plugins.open_store(dir.path());
    let list = again.execute("plugin.list", json!({})).unwrap();
    let mine = |list: &Value| list["plugins"].as_array().unwrap().iter().find(|p| p["id"] == "io.example.stored").cloned();
    let p = mine(&list).expect("listed");
    assert_eq!(p["kind"], "command");
    assert_eq!(p["origin"]["type"], "file");
    assert_eq!(p["stored"], true);
    assert_eq!(list["store"], json!(dir.path().join("Plug-ins").display().to_string()));
    again.execute("plugin.remove", json!({"id": "io.example.stored"})).unwrap();
    assert!(mine(&again.execute("plugin.list", json!({})).unwrap()).is_none());
    let mut third = Session::new();
    third.plugins.open_store(dir.path());
    assert!(third.plugins.list().is_empty(), "removed from the folder");
}

#[test]
fn a_filter_in_a_bundle_installs_into_the_store_and_runs_as_a_filter() {
    let id = "test.bundle.grey";
    let dir = tempfile::tempdir().unwrap();
    let manifest = json!({"id": id, "name": "Grey", "version": "1.0.0", "main": "grey.wasm", "kind": "filter",
        "apps": {"photocraft": {"api": 1}}});
    let bytes = Bundle { manifest: manifest.to_string(), wasm: grey_filter(id), assets: vec![] }.encode();
    let mut s = Session::new();
    s.plugins.open_store(dir.path());
    let r = s.execute("plugin.install", json!({"data": base64(&bytes)})).unwrap();
    assert_eq!((r["kind"].clone(), r["stored"].clone()), (json!("filter"), json!(true)));
    assert!(photocraft_plugins::registry::get(id).is_some(), "registered for Filter › Plug-ins");
    s.execute("file.new", json!({"width": 8, "height": 8, "background": "white"})).unwrap();
    s.execute("plugin.run", json!({"id": id})).unwrap();
    let st = s.active().unwrap();
    let px = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().pixel(1, 1);
    assert!((px[0] - 0.5).abs() < 0.01, "{px:?}");
    let listed = s.execute("plugin.list", json!({})).unwrap();
    let p = listed["plugins"].as_array().unwrap().iter().find(|p| p["id"] == id).cloned().unwrap();
    assert_eq!(p["stored"], true);
    assert!(p["overlap"].is_number(), "described as a filter: {p}");
    // A filter whose module names another plug-in, or targets another filter ABI, is refused.
    let other = Bundle { manifest: manifest.to_string(), wasm: grey_filter("test.bundle.other"), assets: vec![] }.encode();
    assert!(s.execute("plugin.install", json!({"data": base64(&other)})).unwrap_err().to_string().contains("calls itself"));
    let mut abi2 = manifest.clone();
    abi2["apps"]["photocraft"]["api"] = json!(2);
    let abi2 = Bundle { manifest: abi2.to_string(), wasm: grey_filter(id), assets: vec![] }.encode();
    assert!(s.execute("plugin.install", json!({"data": base64(&abi2)})).unwrap_err().to_string().contains("filter ABI"));
    // A new session finds it in the folder and registers it again.
    photocraft_plugins::registry::remove(id);
    let mut again = Session::new();
    again.plugins.open_store(dir.path());
    assert!(photocraft_plugins::registry::get(id).is_some());
    again.execute("plugin.remove", json!({"id": id})).unwrap();
    assert!(photocraft_plugins::registry::get(id).is_none());
}

#[test]
fn plugins_for_other_apps_or_not_bundles_are_refused() {
    let mut s = Session::new();
    let manifest = json!({"id": "io.example.other", "name": "Other", "version": "1.0.0", "main": "p.wasm", "apps": {"figcraft": {"api": 6}}});
    let bytes = Bundle { manifest: manifest.to_string(), wasm: module(&[], true, ""), assets: vec![] }.encode();
    let e = s.execute("plugin.install", json!({"data": base64(&bytes)})).unwrap_err().to_string();
    assert!(e.contains("written for figcraft"), "{e}");
    // A manifest that names no app is refused too (PhotoCraft has no legacy command plug-ins).
    let manifest = json!({"id": "io.example.none", "name": "None", "version": "1.0.0", "main": "p.wasm"});
    let bytes = Bundle { manifest: manifest.to_string(), wasm: module(&[], true, ""), assets: vec![] }.encode();
    assert!(s.execute("plugin.install", json!({"data": base64(&bytes)})).is_err());
    assert!(s.execute("plugin.install", json!({})).is_err());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("broken.craftplugin");
    std::fs::write(&path, b"not a bundle").unwrap();
    let e = s.execute("plugin.install", json!({"path": path.to_string_lossy()})).unwrap_err().to_string();
    assert!(e.contains("isn't a Craft plugin bundle"), "{e}");
}

#[test]
fn a_local_catalog_installs_its_package() {
    let dir = tempfile::tempdir().unwrap();
    let bytes = bundle("io.example.listed", "write", module(&[], true, ""));
    let file = dir.path().join("listed.craftplugin");
    std::fs::write(&file, &bytes).unwrap();
    let url = |p: &std::path::Path| format!("file://{}", p.to_string_lossy().replace('\\', "/"));
    let index = json!({"schema": 1, "packages": [{"id": "io.example.listed", "name": "Listed", "repo": "https://github.com/example/listed",
        "versions": [{"version": "1.0.0", "apps": {"photocraft": {"api": 6}}, "url": url(&file),
            "sha256": crate::plugin::craft_plugin::sha256_hex(&bytes), "size": bytes.len()}]}]});
    let catalog = dir.path().join("index.json");
    std::fs::write(&catalog, index.to_string()).unwrap();
    let mut s = Session::new();
    let rev = s.prefs.rev();
    s.execute("plugin.catalog.add", json!({"url": url(&catalog)})).unwrap();
    assert!(s.prefs.rev() > rev, "the catalog list is saved with the preferences");
    let listed = s.execute("plugin.catalog.list", json!({})).unwrap();
    assert_eq!(listed["packages"][0]["id"], "io.example.listed", "{listed}");
    let r = s.execute("plugin.install", json!({"catalog": url(&catalog), "id": "io.example.listed"})).unwrap();
    assert_eq!(r["inCatalog"], true);
    assert_eq!(r["origin"]["type"], "catalog");
    assert_eq!(s.execute("plugin.update", json!({})).unwrap()["updates"], json!([]), "up to date");
    assert!(s.execute("plugin.catalog.add", json!({"url": "https://example.com/index.json"})).is_err(), "GitHub's hosts only");
    s.execute("plugin.catalog.add", json!({"url": url(&catalog), "remove": true})).unwrap();
    assert_eq!(s.plugins.saved.catalogs, Vec::<String>::new());
}

#[test]
fn automation_gets_no_local_catalog() {
    fn gate(_: &str, _: &Value) -> crate::Result<()> {
        Ok(())
    }
    let mut s = Session::new();
    s.authorize = Some(gate);
    let e = s.execute("plugin.catalog.add", json!({"url": "file:///tmp/index.json"})).unwrap_err().to_string();
    assert!(e.contains("automation"), "{e}");
    let e = s.execute("plugin.install", json!({"catalog": "file:///tmp/index.json", "id": "x.y"})).unwrap_err().to_string();
    assert!(e.contains("automation"), "{e}");
    // Automation installs for the session only, never into the person's Plug-ins folder.
    let dir = tempfile::tempdir().unwrap();
    s.plugins.open_store(dir.path());
    let r = s.execute("plugin.install", json!({"data": base64(&bundle("io.example.agent", "read", module(&[], true, "")))})).unwrap();
    assert_eq!(r["stored"], false);
    assert!(!dir.path().join("Plug-ins").exists());
}

#[test]
fn a_repository_install_without_network_says_so() {
    let mut s = Session::new();
    let e = s.execute("plugin.install", json!({"repo": "example/tools"})).unwrap_err().to_string();
    assert!(e.contains("no network access"), "{e}");
    assert!(s.execute("plugin.install", json!({"repo": "not a repo"})).is_err());
}

#[test]
fn bad_params_are_errors_not_panics() {
    let mut s = with_plugin("io.example.params", "write", module(&[], true, ""));
    for (cmd, p) in [
        ("plugin.install", json!({"repo": 3})),
        ("plugin.install", json!({"catalog": "https://github.com/x/y/index.json"})),
        ("plugin.install", json!({"data": base64(b"FCPLUGB1junk")})),
        ("plugin.install", json!({"data": base64(&bundle("io.example.x", "write", module(&[], true, ""))), "sha256": 5})),
        ("plugin.update", json!({"id": 5})),
        ("plugin.update", json!({"id": "no.such.plugin"})),
        ("plugin.catalog.add", json!({})),
        ("plugin.catalog.add", json!({"url": ""})),
        ("plugin.catalog.add", json!({"url": "ftp://x"})),
        ("plugin.catalog.list", json!({"url": 4})),
        ("plugin.run", json!({"id": "io.example.params", "params": 4})),
        ("plugin.run", json!({"id": "io.example.params", "params": {"x": 1}, "command": ["go"]})),
    ] {
        assert!(s.execute(cmd, p.clone()).is_err(), "{cmd} {p}");
    }
    // A catalog that can't be read is a warning, not a failure, in the listing.
    let r = s.execute("plugin.catalog.list", json!({"url": "https://example.com/index.json"})).unwrap();
    assert_eq!(r["warnings"].as_array().map(Vec::len), Some(1));
}
