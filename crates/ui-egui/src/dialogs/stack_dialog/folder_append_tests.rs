//! #14002: Load Files into Stack keeps one list for Use: Files and Folder. Browse… in Folder mode
//! appends the folder's images, sorted by name, skipping ones already listed; switching Use keeps
//! the list; Remove, Add Open Files and Sort By Name work in both modes; OK loads the list.
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

use super::*;
use crate::file_dialog::FileDialogAnswer;

/// Small PNGs named `names` in `dir`: their paths.
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

fn harness(answers: Vec<Option<FileDialogAnswer>>) -> Harness<'static, PhotocraftApp> {
    let (show, _) = crate::file_dialog::fake(answers);
    Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services { file_dialog: Some(show), ..Default::default() })
    })
}

fn fields<'a>(h: &'a mut Harness<'_, PhotocraftApp>) -> &'a mut Map<String, Value> {
    &mut h.state_mut().ui.dialogs.iter_mut().find(|d| owns(&d.fields)).unwrap().fields
}

fn listed(h: &mut Harness<'_, PhotocraftApp>) -> Vec<String> {
    files(fields(h)).iter().map(|p| file_name(p).to_string()).collect()
}

fn set_use(h: &mut Harness<'_, PhotocraftApp>, mode: &str) {
    fields(h).insert("use".into(), json!(mode));
    h.run_steps(2);
}

fn click(h: &mut Harness<'_, PhotocraftApp>, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(3);
}

/// A folder with c.png, a.png, b.png, a text file and a subfolder holding d.png; and loose.png
/// elsewhere.
fn setup() -> (tempfile::TempDir, String, String) {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("shots");
    std::fs::create_dir_all(folder.join("sub")).unwrap();
    pngs(&folder, &["c.png", "a.png", "b.png"]);
    pngs(&folder.join("sub"), &["d.png"]);
    std::fs::write(folder.join("notes.txt"), "not an image").unwrap();
    let loose = pngs(root.path(), &["loose.png"]).remove(0);
    let folder = folder.to_string_lossy().into_owned();
    (root, folder, loose)
}

fn open(h: &mut Harness<'_, PhotocraftApp>) {
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, COMMAND, json!({})).unwrap();
    h.run_steps(4);
}

#[test]
fn folder_browse_appends_to_the_one_list() {
    let (_root, folder, loose) = setup();
    let mut h = harness(vec![
        Some(FileDialogAnswer::Paths(vec![loose.clone()])),
        Some(FileDialogAnswer::FolderTo(folder.clone())),
        Some(FileDialogAnswer::FolderTo(folder)),
    ]);
    open(&mut h);
    click(&mut h, "Browse…");
    assert_eq!(listed(&mut h), ["loose.png"]);
    set_use(&mut h, "folder");
    assert_eq!(listed(&mut h), ["loose.png"], "switching Use keeps the list");
    click(&mut h, "Browse…");
    assert_eq!(listed(&mut h), ["loose.png", "a.png", "b.png", "c.png"], "the folder's images, by name; no subfolder or text file");
    click(&mut h, "Browse…");
    assert_eq!(listed(&mut h), ["loose.png", "a.png", "b.png", "c.png"], "nothing listed twice");
    set_use(&mut h, "files");
    assert_eq!(listed(&mut h), ["loose.png", "a.png", "b.png", "c.png"]);
}

#[test]
fn the_buttons_work_in_folder_mode_and_ok_loads_the_list() {
    let (_root, folder, loose) = setup();
    let mut h = harness(vec![Some(FileDialogAnswer::FolderTo(folder)), Some(FileDialogAnswer::Paths(vec![loose]))]);
    open(&mut h);
    set_use(&mut h, "folder");
    click(&mut h, "Browse…");
    assert_eq!(listed(&mut h), ["a.png", "b.png", "c.png"]);
    for label in ["Remove", "Add Open Files", "Sort By Name"] {
        assert!(h.query_by_label(label).is_some(), "{label}");
    }
    click(&mut h, "b.png");
    assert!(!h.get_by_label("Remove").accesskit_node().is_disabled(), "Remove works in Folder mode");
    click(&mut h, "Remove");
    assert_eq!(listed(&mut h), ["a.png", "c.png"]);
    set_use(&mut h, "files");
    click(&mut h, "Browse…");
    assert_eq!(listed(&mut h), ["a.png", "c.png", "loose.png"], "one list with both");
    set_use(&mut h, "folder");
    let docs = h.state().session.documents().len();
    click(&mut h, "OK");
    let st = h.state().session.active().unwrap();
    assert_eq!(h.state().session.documents().len(), docs + 1);
    let names: Vec<&str> = st.doc.layers.iter().rev().map(|l| l.name.as_str()).collect();
    assert_eq!(names.len(), 3, "exactly the listed files: {names:?}");
    for n in ["a.png", "c.png", "loose.png"] {
        assert!(names.contains(&n), "{n} in {names:?}");
    }
}

#[test]
fn a_folder_string_still_loads_through_the_engine() {
    let (_root, folder, _) = setup();
    let mut s = photocraft_engine::Session::new();
    let r = s.execute(COMMAND, json!({"paths": folder})).unwrap();
    assert_eq!(r["layers"], 3);
}
