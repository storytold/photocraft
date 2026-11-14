//! #13644: the Artboards to PDF dialog has a Reverse Page Order checkbox. It is unticked on first
//! open, reaches the command, and reopens as the last successful run left it.
use crate::PhotocraftApp;
use egui::accesskit::{Role, Toggled};
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

const COMMAND: &str = "file.export.artboardsToPdf";

/// A document with two artboards of different sizes, so a page's MediaBox says which board it is.
fn harness() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(photocraft_engine::Session::new(), Default::default())
    });
    let app = h.state_mut();
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    app.run("layer.new.artboard", json!({"rect": [0, 0, 200, 150], "name": "Low"})).unwrap();
    app.run("layer.new.artboard", json!({"rect": [250, 0, 100, 100], "name": "High"})).unwrap();
    app.sync_views();
    h
}

fn open(h: &mut Harness<'_, PhotocraftApp>, path: &str) -> u64 {
    let ctx = h.ctx.clone();
    let dialog = crate::menus::invoke(h.state_mut(), &ctx, COMMAND, json!({})).unwrap()["dialog"].as_u64().unwrap();
    h.state_mut().ui.dialog_mut(dialog).unwrap().fields.insert("path".into(), json!(path));
    h.run_steps(4);
    dialog
}

fn ticked(h: &Harness<'_, PhotocraftApp>) -> bool {
    h.get_by_role_and_label(Role::CheckBox, "Reverse Page Order").accesskit_node().toggled() == Some(Toggled::True)
}

fn tick(h: &mut Harness<'_, PhotocraftApp>) {
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
    crate::dialogs::confirm(h.state_mut(), d).unwrap();
    let plain = page_widths(&path);
    assert_eq!(plain.len(), 2);

    let path = t.path().join("reversed.pdf").to_string_lossy().into_owned();
    let d = open(&mut h, &path);
    tick(&mut h);
    assert!(ticked(&h));
    crate::dialogs::confirm(h.state_mut(), d).unwrap();
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
    crate::dialogs::confirm(h.state_mut(), d).unwrap();

    // Reopens ticked; untick it and fail the run (the destination is a file, not a folder).
    let blocked = t.path().join("file");
    std::fs::write(&blocked, b"x").unwrap();
    let bad = blocked.join("out.pdf").to_string_lossy().into_owned();
    let d = open(&mut h, &bad);
    assert!(ticked(&h), "remembered after a successful run");
    tick(&mut h);
    assert!(!ticked(&h));
    assert!(crate::dialogs::confirm(h.state_mut(), d).is_err());

    let d = open(&mut h, &path);
    assert!(ticked(&h), "a failed run leaves the remembered value alone");
    h.state_mut().ui.close_dialog(d);
}
