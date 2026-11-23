//! #13644: the Artboards to PDF dialog has a Reverse Page Order checkbox. It is unticked on first
//! open, reaches the command, and reopens as the last successful run left it.
use crate::PhotocraftApp;
use craft::ui::dialog::DialogId;
use egui::accesskit::{Role, Toggled};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

const COMMAND: &str = "file.export.artboardsToPdf";

type H = Harness<'static, Box<PhotocraftApp>>;

/// A document with two artboards of different sizes, so a page's MediaBox says which board it is.
fn harness() -> H {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    app.run("layer.new.artboard", json!({"rect": [0, 0, 200, 150], "name": "Low"})).unwrap();
    app.run("layer.new.artboard", json!({"rect": [250, 0, 100, 100], "name": "High"})).unwrap();
    app.sync_views();
    let h = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .with_max_steps(64)
        .with_context(app.ctx().clone())
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h
}

/// Opens the dialog from its menu entry with File set to `path`.
fn open(h: &mut H, path: &str) -> DialogId {
    let ctx = h.ctx.clone();
    let reply = crate::menus::invoke(h.state_mut(), &ctx, COMMAND, json!({})).unwrap();
    let want = reply["dialog"].as_str().unwrap().to_owned();
    let dialog = h.state().framework.dialogs.open_dialogs().into_iter().map(|d| d.id).find(|d| d.value().to_string() == want).unwrap();
    h.run_steps(4);
    h.state_mut().framework.dialogs.set_json_field(&ctx, dialog, "path", json!(path)).unwrap();
    // Every board, not only the selected one: Export Selected Artboards opens ticked while a board is selected (#13647).
    h.state_mut().framework.dialogs.set_json_field(&ctx, dialog, "artboards", serde_json::Value::Null).unwrap();
    h.run_steps(4);
    dialog
}

fn confirm(h: &mut H, dialog: DialogId) -> Result<(), String> {
    let ctx = h.ctx.clone();
    let r = crate::dialogs::confirm(h.state_mut(), &ctx, dialog);
    h.run_steps(4);
    r
}

fn ticked(h: &H) -> bool {
    h.get_by_role_and_label(Role::CheckBox, "Reverse Page Order").accesskit_node().toggled() == Some(Toggled::True)
}

fn tick(h: &mut H) {
    h.get_by_role_and_label(Role::CheckBox, "Reverse Page Order").click();
    h.run_steps(2);
}

/// Each page's width in points, in page order.
fn page_widths(path: &str) -> Vec<u32> {
    let text = String::from_utf8_lossy(&std::fs::read(path).unwrap()).into_owned();
    text.split("/MediaBox [0 0 ").skip(1).map(|rest| rest.split(' ').next().unwrap().parse::<f64>().unwrap().round() as u32).collect()
}

#[test]
fn the_checkbox_starts_unticked_and_reverses_the_pages() {
    let t = tempfile::tempdir().unwrap();
    let path = t.path().join("plain.pdf").to_string_lossy().into_owned();
    let mut h = harness();
    let d = open(&mut h, &path);
    assert!(!ticked(&h), "unticked on first open");
    confirm(&mut h, d).unwrap();
    let plain = page_widths(&path);
    assert_eq!(plain.len(), 2);

    let path = t.path().join("reversed.pdf").to_string_lossy().into_owned();
    let d = open(&mut h, &path);
    tick(&mut h);
    assert!(ticked(&h));
    confirm(&mut h, d).unwrap();
    let mut want = plain;
    want.reverse();
    assert_eq!(page_widths(&path), want);
}

#[test]
fn a_successful_run_is_remembered_a_failed_one_is_not() {
    let t = tempfile::tempdir().unwrap();
    let mut h = harness();
    let path = t.path().join("out.pdf").to_string_lossy().into_owned();
    let d = open(&mut h, &path);
    tick(&mut h);
    confirm(&mut h, d).unwrap();

    // Reopens ticked; untick it and fail the run (the destination is a file, not a folder).
    let blocked = t.path().join("file");
    std::fs::write(&blocked, b"x").unwrap();
    let bad = blocked.join("out.pdf").to_string_lossy().into_owned();
    let d = open(&mut h, &bad);
    assert!(ticked(&h), "remembered after a successful run");
    tick(&mut h);
    assert!(!ticked(&h));
    assert!(!h.state().ui.status_error, "the first run succeeded: {}", h.state().ui.status);
    let r = confirm(&mut h, d);
    // The run fails, so it must not be what the next open remembers.
    assert!(r.is_err() || h.state().ui.status_error, "the blocked run fails: {r:?}, status {}", h.state().ui.status);
    assert!(!std::path::Path::new(&bad).exists(), "the blocked run writes nothing");
    if h.state().framework.dialogs.get_dyn(d).is_some() {
        let ctx = h.ctx.clone();
        crate::dialogs::cancel(h.state_mut(), &ctx, d).unwrap();
        h.run_steps(4);
    }

    let d = open(&mut h, &path);
    assert!(ticked(&h), "a failed run leaves the remembered value alone");
    let ctx = h.ctx.clone();
    crate::dialogs::cancel(h.state_mut(), &ctx, d).unwrap();
}
