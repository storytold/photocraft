//! #14001: Load Files into Stack's Add Open Files adds only the open documents that are saved and
//! unchanged, in tab order; an unsaved one (never saved, or changed since) is left out with one
//! "Documents must be saved before they can be merged." alert per click.
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::*;

const COMMAND: &str = "file.scripts.loadFilesIntoStack";

/// Small PNGs named `names` in a fresh folder: their paths.
fn pngs(dir: &std::path::Path, names: &[&str]) -> Vec<String> {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 8, "height": 6, "background": "#406080"})).unwrap();
    names
        .iter()
        .map(|n| {
            let p = dir.join(n).to_string_lossy().into_owned();
            s.execute("file.saveACopy", json!({"path": p})).unwrap();
            p
        })
        .collect()
}

fn harness() -> Harness<'static, PhotocraftApp> {
    Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(photocraft_engine::Session::new(), Default::default())
    })
}

/// A document standing for `path` opened from disk: saved there, unchanged since.
fn open_saved(h: &mut Harness<'_, PhotocraftApp>, path: &str) {
    h.state_mut().run("file.new", json!({"width": 8, "height": 6})).unwrap();
    let st = h.state_mut().session.active_mut().unwrap();
    st.path = Some(path.to_string());
    st.saved_revision = st.revision;
    h.run_steps(3);
    assert!(!h.state().session.active().unwrap().is_dirty(), "{path} is clean");
}

fn open_dialog(h: &mut Harness<'_, PhotocraftApp>) {
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, COMMAND, json!({})).unwrap();
    h.run_steps(4);
}

fn listed(h: &Harness<'_, PhotocraftApp>) -> Vec<String> {
    let f = &h.state().ui.dialogs.iter().find(|d| owns(&d.fields)).unwrap().fields;
    files(f).iter().map(|p| file_name(p).to_string()).collect()
}

/// The messages of the open alerts.
fn alerts(h: &Harness<'_, PhotocraftApp>) -> Vec<String> {
    h.state().ui.dialogs.iter().filter(|d| d.kind == DialogKind::Error).filter_map(|d| d.fields.get("message")?.as_str().map(str::to_string)).collect()
}

fn click_add(h: &mut Harness<'_, PhotocraftApp>) {
    h.get_by_label("Add Open Files").click();
    h.run_steps(3);
}

#[test]
fn unsaved_documents_are_left_out_with_one_alert() {
    let dir = tempfile::tempdir().unwrap();
    let paths = pngs(dir.path(), &["a.png", "b.png", "c.png"]);
    let mut h = harness();
    open_saved(&mut h, &paths[0]);
    // Never saved.
    h.state_mut().run("file.new", json!({"width": 4, "height": 4})).unwrap();
    open_saved(&mut h, &paths[1]);
    // Opened from disk, then painted on: changed since its save.
    open_saved(&mut h, &paths[2]);
    h.state_mut().run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    assert!(h.state().session.active().unwrap().is_dirty());
    open_dialog(&mut h);
    click_add(&mut h);
    assert_eq!(listed(&h), ["a.png", "b.png"], "the saved, unchanged ones, in tab order");
    assert_eq!(alerts(&h), [UNSAVED_ALERT], "one alert for two unsaved documents");
    // Again: no duplicates, and the alert again.
    h.state_mut().ui.dialogs.retain(|d| d.kind != DialogKind::Error);
    h.run_steps(2);
    click_add(&mut h);
    assert_eq!(listed(&h), ["a.png", "b.png"]);
    assert_eq!(alerts(&h), [UNSAVED_ALERT]);
}

#[test]
fn only_saved_documents_add_without_an_alert() {
    let dir = tempfile::tempdir().unwrap();
    let paths = pngs(dir.path(), &["a.png", "b.png"]);
    let mut h = harness();
    open_saved(&mut h, &paths[1]);
    open_saved(&mut h, &paths[0]);
    open_dialog(&mut h);
    click_add(&mut h);
    assert_eq!(listed(&h), ["b.png", "a.png"], "tab order");
    assert!(alerts(&h).is_empty());
}
