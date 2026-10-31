use super::*;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

/// A minimal plug-in with its own `id` and `version` (leaves pixels alone).
fn module(id: &str, version: &str) -> Vec<u8> {
    let manifest = format!(r#"{{"id":"{id}","name":"Store {id}","version":"{version}","kind":"filter"}}"#);
    let src = format!(
        r#"(module
  (memory (export "memory") 1)
  (data (i32.const 16) "{data}")
  (func (export "pc_abi_version") (result i32) (i32.const 1))
  (func (export "pc_manifest") (result i64) (i64.or (i64.shl (i64.const {len}) (i64.const 32)) (i64.const 16)))
  (func (export "pc_alloc") (param $n i32) (result i32) (i32.const 0))
  (func (export "pc_filter") (param i32 i32 i32 i32 i32 i32 i32 i32) (result i32) (i32.const 0)))"#,
        data = manifest.replace('"', "\\\""),
        len = manifest.len()
    );
    wat::parse_str(src).unwrap()
}

fn entry(id: &str, version: &str, bytes: &[u8]) -> Entry {
    Entry {
        id: id.into(),
        name: format!("Store {id}"),
        version: version.into(),
        author: "Test".into(),
        description: "A test listing".into(),
        collection: "Tests".into(),
        repo: "https://github.com/example/tests".into(),
        license: "MIT".into(),
        deprecated: None,
        url: format!("https://raw.githubusercontent.com/example/tests/0/{id}.wasm"),
        sha256: hex(&Sha256::digest(bytes)),
        size: bytes.len(),
    }
}

type Served = Arc<Mutex<Vec<(String, mpsc::Sender<FetchResult>, Arc<Progress>)>>>;

/// A session whose catalog is `entries` and whose transport records requests for the test to answer.
fn session(entries: Vec<Entry>) -> (Session, Served) {
    let mut s = Session::new();
    s.plugin_store.catalog = entries;
    let served: Served = Arc::default();
    let log = served.clone();
    s.plugin_store.fetch = Some(Box::new(move |r: Request| {
        let (tx, rx) = mpsc::channel();
        log.lock().unwrap().push((r.url, tx, r.progress));
        Ok(rx)
    }));
    (s, served)
}

fn answer(served: &Served, result: FetchResult) {
    let (_, tx, _) = served.lock().unwrap().remove(0);
    tx.send(result).unwrap();
}

fn listing(s: &mut Session, id: &str) -> Value {
    let v = s.execute("plugin.store.list", json!({})).unwrap();
    v["plugins"].as_array().unwrap().iter().find(|p| p["id"] == id).cloned().unwrap()
}

#[test]
fn the_bundled_catalog_parses_and_every_listing_is_valid() {
    let raw: Catalog = serde_json::from_str(include_str!("catalog.json")).unwrap();
    assert!(raw.plugins.len() >= 40, "{} listings", raw.plugins.len());
    for e in &raw.plugins {
        check_entry(e).unwrap_or_else(|why| panic!("{}: {why}", e.id));
        assert!(photocraft_plugins::Manifest::parse(format!(r#"{{"id":"{}","name":"x","kind":"filter"}}"#, e.id).as_bytes()).is_ok(), "{}", e.id);
    }
    assert_eq!(bundled().len(), raw.plugins.len());
    let mut ids: Vec<&str> = raw.plugins.iter().map(|e| e.id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), raw.plugins.len(), "ids are unique");
}

#[test]
fn listings_with_bad_urls_or_checksums_are_refused() {
    let good = entry("org.test.store.check", "1", b"abc");
    assert!(check_entry(&good).is_ok());
    for (url, sha) in [
        ("http://raw.githubusercontent.com/x.wasm", good.sha256.as_str()),
        ("https://evil.example/x.wasm", good.sha256.as_str()),
        ("https://github.com.evil.example/x.wasm", good.sha256.as_str()),
        (good.url.as_str(), "zz"),
    ] {
        let e = Entry { url: url.into(), sha256: sha.into(), ..good.clone() };
        assert!(check_entry(&e).is_err(), "{url} {sha}");
    }
    assert!(check_entry(&Entry { size: 0, ..good.clone() }).is_err());
    assert!(check_entry(&Entry { size: usize::MAX, ..good }).is_err());
}

#[test]
fn install_downloads_verifies_and_registers_then_uninstall_removes() {
    let id = "org.test.store.flow";
    let bytes = module(id, "1.0.0");
    let (mut s, served) = session(vec![entry(id, "1.0.0", &bytes)]);
    assert_eq!(listing(&mut s, id)["installed"], false);

    let r = s.execute("plugin.store.install", json!({"id": id})).unwrap();
    assert_eq!(r["plugins"][0]["job"]["state"], "downloading");
    assert_eq!(served.lock().unwrap()[0].0, format!("https://raw.githubusercontent.com/example/tests/0/{id}.wasm"));
    assert!(s.plugin_store.busy());
    // A second request while it downloads doesn't start another.
    s.execute("plugin.store.install", json!({"id": id})).unwrap();
    assert_eq!(served.lock().unwrap().len(), 1);

    answer(&served, Ok(bytes));
    let l = listing(&mut s, id);
    assert_eq!(l["installed"], true, "{l}");
    assert_eq!(l["installedVersion"], "1.0.0");
    assert!(l.get("job").is_none());
    assert!(registry::get(id).is_some());
    // Up to date: installing again does nothing.
    s.execute("plugin.store.install", json!({"id": id})).unwrap();
    assert!(served.lock().unwrap().is_empty());

    let r = s.execute("plugin.store.uninstall", json!({"id": id})).unwrap();
    assert_eq!(r["removed"], json!([id]));
    assert!(registry::get(id).is_none());
}

#[test]
fn a_tampered_download_is_refused_and_not_installed() {
    let id = "org.test.store.tampered";
    let bytes = module(id, "1");
    let (mut s, served) = session(vec![entry(id, "1", &bytes)]);
    s.execute("plugin.store.install", json!({"id": id})).unwrap();
    let mut evil = bytes.clone();
    if let Some(b) = evil.last_mut() {
        *b ^= 1;
    }
    answer(&served, Ok(evil));
    let l = listing(&mut s, id);
    assert_eq!(l["installed"], false);
    assert_eq!(l["job"]["state"], "failed");
    assert!(l["job"]["message"].as_str().unwrap().contains("checksum"), "{l}");
    assert!(registry::get(id).is_none());
}

#[test]
fn a_module_with_another_id_or_a_transport_error_fails_cleanly() {
    let id = "org.test.store.wrongid";
    let other = module("org.test.store.impostor", "1");
    let (mut s, served) = session(vec![entry(id, "1", &other)]);
    s.execute("plugin.store.install", json!({"id": id})).unwrap();
    answer(&served, Ok(other));
    assert!(listing(&mut s, id)["job"]["message"].as_str().unwrap().contains("impostor"));
    assert!(registry::get("org.test.store.impostor").is_none());

    s.execute("plugin.store.install", json!({"id": id})).unwrap();
    answer(&served, Err("couldn't reach raw.githubusercontent.com".into()));
    assert_eq!(listing(&mut s, id)["job"]["message"], "couldn't reach raw.githubusercontent.com");

    // A worker that goes away without answering.
    s.execute("plugin.store.install", json!({"id": id})).unwrap();
    served.lock().unwrap().clear();
    assert!(listing(&mut s, id)["job"]["message"].as_str().unwrap().contains("stopped"));
}

#[test]
fn uninstall_cancels_a_download_in_flight() {
    let id = "org.test.store.cancel";
    let bytes = module(id, "1");
    let (mut s, served) = session(vec![entry(id, "1", &bytes)]);
    s.execute("plugin.store.install", json!({"id": id})).unwrap();
    assert!(s.plugin_store.busy());
    s.execute("plugin.store.uninstall", json!({"id": id})).unwrap();
    assert!(!s.plugin_store.busy());
    // The download still arrives, but nobody is waiting for it.
    let (_, tx, _) = served.lock().unwrap().remove(0);
    let _ = tx.send(Ok(bytes));
    let l = listing(&mut s, id);
    assert_eq!(l["installed"], false, "{l}");
    assert!(l.get("job").is_none());
    assert!(registry::get(id).is_none());
}

#[test]
fn a_cancelled_download_keeps_its_slot_until_it_answers() {
    let entries: Vec<Entry> = (0..MAX_RUNNING + 1).map(|k| entry(&format!("org.test.store.slot{k}"), "1", &module(&format!("org.test.store.slot{k}"), "1"))).collect();
    let ids: Vec<String> = entries.iter().map(|e| e.id.clone()).collect();
    let (mut s, served) = session(entries);
    s.execute("plugin.store.install", json!({"ids": ids[..MAX_RUNNING]})).unwrap();
    assert_eq!(served.lock().unwrap().len(), MAX_RUNNING);
    // Uninstalling one doesn't free its slot while its download is still out.
    s.execute("plugin.store.uninstall", json!({"id": ids[0]})).unwrap();
    s.execute("plugin.store.install", json!({"id": ids[MAX_RUNNING]})).unwrap();
    assert_eq!(served.lock().unwrap().len(), MAX_RUNNING, "no extra download starts");
    // Once it answers, the queued one starts.
    let (_, tx, _) = served.lock().unwrap().remove(0);
    tx.send(Err("late".into())).unwrap();
    s.plugin_store.poll();
    assert_eq!(served.lock().unwrap().len(), MAX_RUNNING);
    assert!(served.lock().unwrap().last().unwrap().0.contains(&ids[MAX_RUNNING]));
    assert!(listing(&mut s, &ids[0]).get("job").is_none(), "the cancelled one reports nothing");
}

#[test]
fn a_module_whose_version_isnt_the_listed_one_is_refused() {
    let id = "org.test.store.version";
    let bytes = module(id, "2.0.0");
    let (mut s, served) = session(vec![entry(id, "1.0.0", &bytes)]);
    s.execute("plugin.store.install", json!({"id": id})).unwrap();
    answer(&served, Ok(bytes));
    let l = listing(&mut s, id);
    assert_eq!(l["installed"], false, "{l}");
    assert!(l["job"]["message"].as_str().unwrap().contains("version"), "{l}");
}

#[test]
fn a_new_catalog_version_is_offered_as_an_update() {
    let id = "org.test.store.update";
    let old = module(id, "1.0.0");
    let new = module(id, "2.0.0");
    let (mut s, served) = session(vec![entry(id, "1.0.0", &old)]);
    s.execute("plugin.store.install", json!({"id": id})).unwrap();
    answer(&served, Ok(old));
    assert_eq!(listing(&mut s, id)["updateAvailable"], false);
    s.plugin_store.catalog = vec![entry(id, "2.0.0", &new)];
    assert_eq!(listing(&mut s, id)["updateAvailable"], true);
    s.execute("plugin.store.install", json!({"id": id})).unwrap();
    answer(&served, Ok(new));
    assert_eq!(listing(&mut s, id)["installedVersion"], "2.0.0");
    registry::remove(id);
}

#[test]
fn downloads_beyond_the_limit_wait_their_turn() {
    let entries: Vec<Entry> = (0..MAX_RUNNING + 2).map(|k| entry(&format!("org.test.store.queue{k}"), "1", &module(&format!("org.test.store.queue{k}"), "1"))).collect();
    let ids: Vec<String> = entries.iter().map(|e| e.id.clone()).collect();
    let (mut s, served) = session(entries);
    s.execute("plugin.store.install", json!({"ids": ids})).unwrap();
    assert_eq!(served.lock().unwrap().len(), MAX_RUNNING);
    answer(&served, Err("x".into()));
    s.plugin_store.poll();
    assert_eq!(served.lock().unwrap().len(), MAX_RUNNING, "a finished download makes room for a queued one");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn installs_are_saved_to_the_folder_and_load_from_it_again() {
    let id = "org.test.store.saved";
    let bytes = module(id, "1");
    let dir = tempfile::tempdir().unwrap();
    let (mut s, served) = session(vec![entry(id, "1", &bytes)]);
    s.plugin_store.set_dir(dir.path().to_path_buf());
    s.execute("plugin.store.install", json!({"id": id})).unwrap();
    answer(&served, Ok(bytes));
    s.plugin_store.poll();
    let file = dir.path().join(format!("{id}.wasm"));
    assert!(file.is_file());
    assert_eq!(registry::get(id).unwrap().source(), Some(file.display().to_string().as_str()));

    registry::remove(id);
    let report = Session::new().plugin_store.set_dir(dir.path().to_path_buf());
    assert_eq!(report["loaded"], json!([id]));

    s.execute("plugin.store.uninstall", json!({"id": id})).unwrap();
    assert!(!file.exists());
    assert!(registry::get(id).is_none());
}

#[test]
fn bad_params_and_a_missing_transport_are_errors() {
    let id = "org.test.store.params";
    let (mut s, _) = session(vec![entry(id, "1", &module(id, "1"))]);
    for (cmd, p) in [
        ("plugin.store.install", json!({})),
        ("plugin.store.install", json!({"id": 3})),
        ("plugin.store.install", json!({"ids": "x"})),
        ("plugin.store.install", json!({"ids": []})),
        ("plugin.store.install", json!({"ids": [1]})),
        ("plugin.store.install", json!({"id": id, "ids": [id]})),
        ("plugin.store.install", json!({"id": "org.unknown"})),
        ("plugin.store.uninstall", json!({"id": "org.unknown"})),
        ("plugin.store.uninstall", json!(null)),
        ("plugin.store.list", json!({"query": 5})),
    ] {
        assert!(s.execute(cmd, p.clone()).is_err(), "{cmd} {p}");
    }
    let mut bare = Session::new();
    bare.plugin_store.catalog = s.plugin_store.catalog.clone();
    assert_eq!(bare.execute("plugin.store.list", json!({})).unwrap()["available"], false);
    let e = bare.execute("plugin.store.install", json!({"id": id})).unwrap_err().to_string();
    assert!(e.contains("not available"), "{e}");
}

#[test]
fn list_filters_by_query_and_installed() {
    let (mut s, _) = session(vec![entry("org.test.store.alpha", "1", b"a"), Entry { description: "Makes halftone dots".into(), ..entry("org.test.store.beta", "1", b"b") }]);
    let names = |v: Value| v["plugins"].as_array().unwrap().iter().map(|p| p["id"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert_eq!(names(s.execute("plugin.store.list", json!({"query": "HALFTONE"})).unwrap()), ["org.test.store.beta"]);
    assert_eq!(names(s.execute("plugin.store.list", json!({"query": "alpha"})).unwrap()), ["org.test.store.alpha"]);
    assert!(names(s.execute("plugin.store.list", json!({"installed": true})).unwrap()).is_empty());
}

#[test]
fn cancel_stops_a_download_and_the_transport_is_told() {
    let id = "org.test.store.stop";
    let bytes = module(id, "1");
    let (mut s, served) = session(vec![entry(id, "1", &bytes)]);
    s.execute("plugin.store.install", json!({"id": id})).unwrap();
    let progress = served.lock().unwrap()[0].2.clone();
    let v = s.execute("plugin.store.cancel", json!({"id": id})).unwrap();
    assert_eq!(v["cancelled"], json!([id]));
    assert!(progress.cancelled(), "the transport sees the cancel");
    assert!(!s.plugin_store.busy());
    assert!(listing(&mut s, id).get("job").is_none());
    // Its answer is dropped, and cancelling again finds nothing.
    answer(&served, Ok(bytes));
    assert_eq!(listing(&mut s, id)["installed"], false);
    assert_eq!(s.execute("plugin.store.cancel", json!({"id": id})).unwrap()["cancelled"], json!([]));
    assert!(s.execute("plugin.store.cancel", json!({"id": "org.test.store.unknown"})).is_err());
    assert!(s.execute("plugin.store.cancel", json!({})).is_err());
}

#[test]
fn reinstalling_waits_for_the_cancelled_download_to_answer() {
    let id = "org.test.store.again";
    let bytes = module(id, "1");
    let (mut s, served) = session(vec![entry(id, "1", &bytes)]);
    s.execute("plugin.store.install", json!({"id": id})).unwrap();
    s.execute("plugin.store.cancel", json!({"id": id})).unwrap();
    s.execute("plugin.store.install", json!({"id": id})).unwrap();
    assert_eq!(served.lock().unwrap().len(), 1, "no second download while the first is out");
    assert_eq!(s.plugin_store.job(id), Some(&Job::Queued));
    answer(&served, Err("cancelled".into()));
    s.plugin_store.poll();
    assert_eq!(served.lock().unwrap().len(), 1, "the new one starts once the old one answered");
    answer(&served, Ok(bytes));
    assert_eq!(listing(&mut s, id)["installed"], true);
    registry::remove(id);
}

#[test]
fn a_queued_download_can_be_cancelled_before_it_starts() {
    let entries: Vec<Entry> = (0..MAX_RUNNING + 1).map(|k| entry(&format!("org.test.store.wait{k}"), "1", &module(&format!("org.test.store.wait{k}"), "1"))).collect();
    let ids: Vec<String> = entries.iter().map(|e| e.id.clone()).collect();
    let (mut s, served) = session(entries);
    s.execute("plugin.store.install", json!({"ids": ids})).unwrap();
    let last = &ids[MAX_RUNNING];
    assert_eq!(s.plugin_store.job(last), Some(&Job::Queued));
    s.execute("plugin.store.cancel", json!({"id": last})).unwrap();
    answer(&served, Err("x".into()));
    s.plugin_store.poll();
    assert_eq!(served.lock().unwrap().len(), MAX_RUNNING - 1, "the cancelled one never starts");
    assert!(s.plugin_store.job(last).is_none());
}

#[test]
fn list_reports_download_progress() {
    let id = "org.test.store.progress";
    let bytes = module(id, "1");
    let (mut s, served) = session(vec![entry(id, "1", &bytes)]);
    s.execute("plugin.store.install", json!({"id": id})).unwrap();
    served.lock().unwrap()[0].2.set_received(10);
    let job = listing(&mut s, id)["job"].clone();
    assert_eq!(job["state"], "downloading");
    assert_eq!(job["received"], 10);
    assert_eq!(job["total"], bytes.len());
    // A transport that overshoots is shown as complete, not past it.
    served.lock().unwrap()[0].2.set_received(u64::MAX);
    assert_eq!(s.plugin_store.progress(id), Some((bytes.len() as u64, bytes.len() as u64)));
}

#[test]
fn deprecated_collections_say_so_on_every_listing() {
    let raw: Catalog = serde_json::from_str(include_str!("catalog.json")).unwrap();
    let artcraft: Vec<&Entry> = raw.plugins.iter().filter(|e| e.collection == "ArtCraft Store").collect();
    assert!(!artcraft.is_empty());
    assert!(artcraft.iter().all(|e| e.deprecated.as_deref().is_some_and(|d| !d.is_empty())), "every ArtCraft Store listing is marked");
    // A collection is one project: its listings agree on where they come from and whether it's kept up.
    let mut by_collection: BTreeMap<&str, (&str, &str, Option<&str>)> = BTreeMap::new();
    for e in &raw.plugins {
        let these = (e.repo.as_str(), e.license.as_str(), e.deprecated.as_deref());
        assert_eq!(*by_collection.entry(&e.collection).or_insert(these), these, "{}", e.id);
    }
    let (s, _) = session(raw.plugins.clone());
    let mut s = s;
    let l = listing(&mut s, &artcraft[0].id);
    assert!(l["deprecated"].as_str().is_some_and(|d| d.contains("no updates")), "{l}");
}

/// The catalog file on disk is the one compiled in, and none of its listings is silently dropped.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn the_catalog_on_disk_is_the_one_compiled_in() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/cmd/plugin_store_cmds/catalog.json");
    let text = std::fs::read_to_string(&path).unwrap();
    let raw: Catalog = serde_json::from_str(&text).unwrap();
    assert_eq!(raw.plugins.as_slice(), bundled(), "every listing in {} is offered, unchanged", path.display());
}
