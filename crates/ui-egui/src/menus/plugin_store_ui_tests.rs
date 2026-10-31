use super::*;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_engine::Session;
use photocraft_engine::plugin_store_cmds::FetchResult;
use std::sync::{Arc, Mutex, mpsc};

use crate::Services;

type Requests = Arc<Mutex<Vec<(String, mpsc::Sender<FetchResult>)>>>;

fn entry(id: &str, name: &str) -> Entry {
    Entry {
        id: id.into(),
        name: name.into(),
        version: "1.0.0".into(),
        author: "Tester".into(),
        description: format!("{name} does things"),
        collection: "Test Collection".into(),
        repo: "https://github.com/example/test-plugins".into(),
        license: "MIT".into(),
        deprecated: None,
        url: format!("https://raw.githubusercontent.com/example/test-plugins/0/{id}.wasm"),
        sha256: "0".repeat(64),
        size: 10,
    }
}

fn app() -> (PhotocraftApp, Requests) {
    let requests: Requests = Arc::default();
    let log = requests.clone();
    let fetch: photocraft_engine::plugin_store_cmds::FetchFn = Box::new(move |r: photocraft_engine::plugin_store_cmds::Request| {
        let (tx, rx) = mpsc::channel();
        log.lock().unwrap().push((r.url, tx));
        Ok(rx)
    });
    let mut app = PhotocraftApp::new(Session::new(), Services { fetch_plugin: Some(fetch), ..Default::default() });
    app.session.plugin_store.set_catalog(vec![entry("org.test.ui.halftone", "Halftone Test"), entry("org.test.ui.vignette", "Vignette Test")]);
    (app, requests)
}

fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(900.0, 700.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            show(app, &ctx);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    h
}

#[test]
fn the_plugins_menu_has_browse_and_manage_and_they_open_the_window() {
    let (mut app, _) = app();
    let items = crate::menus::menu_items(&app);
    for id in ["plugins.browse", "plugins.manage"] {
        let item = items.iter().find(|i| i.id == id).unwrap_or_else(|| panic!("{id} is in the menus"));
        assert_eq!(item.path, ["Plugins"]);
        assert!(item.enabled, "{id} works without a document");
    }
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "plugins.browse", json!({})).unwrap();
    assert!(app.ui.plugin_store.open && !app.ui.plugin_store.installed_only);
    crate::menus::invoke(&mut app, &ctx, "plugins.manage", json!({})).unwrap();
    assert!(app.ui.plugin_store.installed_only);
}

#[test]
fn browse_lists_the_catalog_and_install_starts_a_download() {
    let (mut app, requests) = app();
    open(&mut app, false);
    let mut h = harness(app);
    // Each row's checkbox is named after its plug-in.
    h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Halftone Test");
    h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Vignette Test");
    h.get_by_label("Test Collection");
    let install = h.get_all_by_label("Install").next().unwrap();
    install.click();
    h.run_steps(3);
    assert_eq!(requests.lock().unwrap().len(), 1);
    assert!(requests.lock().unwrap()[0].0.ends_with("org.test.ui.halftone.wasm"));
    h.get_by_label("Cancel");

    // A failed download says why and offers Retry.
    let (_, tx) = requests.lock().unwrap().remove(0);
    tx.send(Err("couldn't reach raw.githubusercontent.com".into())).unwrap();
    h.run_steps(3);
    h.state_mut().session.plugin_store.poll();
    h.run_steps(3);
    h.get_by_label("couldn't reach raw.githubusercontent.com");
    h.get_by_label("Retry");
}

#[test]
fn search_filters_and_install_selected_installs_the_ticked_rows() {
    let (mut app, requests) = app();
    open(&mut app, false);
    app.ui.plugin_store.query = "vignette".into();
    let mut h = harness(app);
    assert!(h.query_by_label("Halftone Test").is_none());
    h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Vignette Test");

    h.state_mut().ui.plugin_store.query.clear();
    h.state_mut().ui.plugin_store.selected = ["org.test.ui.halftone".to_string(), "org.test.ui.vignette".to_string()].into();
    h.run_steps(3);
    h.get_by_label("Install Selected (2)").click();
    h.run_steps(3);
    assert_eq!(requests.lock().unwrap().len(), 2);
    assert!(h.state().ui.plugin_store.selected.is_empty());
}

#[test]
fn install_selected_counts_only_the_ticks_the_view_shows() {
    let (mut app, requests) = app();
    open(&mut app, false);
    app.ui.plugin_store.selected = ["org.test.ui.halftone".to_string()].into();
    let mut h = harness(app);
    h.get_by_label("Install Selected (1)");
    // Installed shows neither test plug-in, so the tick made in Browse doesn't count there.
    h.state_mut().ui.plugin_store.installed_only = true;
    h.run_steps(3);
    h.get_by_label("Install Selected (0)").click();
    h.run_steps(3);
    assert!(requests.lock().unwrap().is_empty());
    // Back in Browse it still stands.
    h.state_mut().ui.plugin_store.installed_only = false;
    h.run_steps(3);
    h.get_by_label("Install Selected (1)");
}

#[test]
fn manage_shows_only_installed_and_without_a_transport_install_is_off() {
    let mut app = PhotocraftApp::new(Session::new(), Services::default());
    app.session.plugin_store.set_catalog(vec![entry("org.test.ui.none", "Nothing Installed")]);
    open(&mut app, true);
    let mut h = harness(app);
    h.get_by_label("No catalog plug-ins are installed.");
    h.state_mut().ui.plugin_store.installed_only = false;
    h.run_steps(3);
    h.get_by_label("Downloading plug-ins is not available in this build.");
    h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Nothing Installed");
}

#[test]
fn an_outdated_plug_in_offers_update_and_uninstall() {
    let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../plugins/tests/fixtures/invert.wasm")).unwrap();
    let plugin = photocraft_plugins::registry::install_bytes(&bytes).unwrap();
    let mut listing = entry(plugin.id(), "Invert Test");
    listing.version = format!("{}-newer", plugin.manifest().version);
    let (mut app, _) = app();
    app.session.plugin_store.set_catalog(vec![listing]);
    open(&mut app, true);
    let h = harness(app);
    h.get_by_label("Update");
    h.get_by_label("Uninstall");
}

#[test]
fn a_download_shows_progress_and_cancel_stops_it() {
    let (mut app, requests) = app();
    open(&mut app, false);
    app.run("plugin.store.install", json!({"id": "org.test.ui.vignette"})).unwrap();
    let mut h = harness(app);
    assert!(h.state().session.plugin_store.progress("org.test.ui.vignette").is_some());
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(!h.state().session.plugin_store.busy());
    assert!(h.query_by_label("Cancel").is_none());
    // The row offers Install again, and the late answer installs nothing.
    let (_, tx) = requests.lock().unwrap().remove(0);
    let _ = tx.send(Err("cancelled".into()));
    h.state_mut().session.plugin_store.poll();
    h.run_steps(3);
    assert_eq!(h.get_all_by_label("Install").count(), 2);
}

#[test]
fn a_deprecated_collection_says_so_under_its_heading() {
    let (mut app, _) = app();
    let mut old = entry("org.test.ui.old", "Old Filter");
    old.collection = "Old Collection".into();
    old.deprecated = Some("No longer maintained.".into());
    app.session.plugin_store.set_catalog(vec![entry("org.test.ui.halftone", "Halftone Test"), old]);
    open(&mut app, false);
    let h = harness(app);
    h.get_by_label("No longer maintained.");
    assert_eq!(h.get_all_by_label("No longer maintained.").count(), 1, "said once, under the heading");
}
