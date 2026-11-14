//! #14000: in Load Files into Stack (Files mode), OK and Attempt to Automatically Align Source
//! Images require at least two files in the list. With 0 or 1 file both are greyed; from 2 on both
//! are enabled. The align checkbox keeps its ticked state while greyed and shows again once a second
//! file is added. Create Smart Object stays enabled at every row count. Use: Folder and Statistics
//! keep their existing rules.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Map, Value, json};

use super::*;
use crate::PhotocraftApp;
use crate::file_dialog::FileDialogAnswer;

fn pngs(tag: &str, names: &[&str]) -> (String, Vec<String>) {
    let dir = std::env::temp_dir().join(format!("photocraft-two-files-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 8, "height": 6, "background": "#406080"})).unwrap();
    let paths = names
        .iter()
        .map(|n| {
            let p = dir.join(n).to_string_lossy().into_owned();
            s.execute("file.saveACopy", json!({"path": p})).unwrap();
            p
        })
        .collect();
    (dir.to_string_lossy().into_owned(), paths)
}

fn harness(answers: Vec<Option<FileDialogAnswer>>) -> Harness<'static, PhotocraftApp> {
    let (show, _) = crate::file_dialog::fake(answers);
    Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services { file_dialog: Some(show), ..Default::default() })
    })
}

fn stack_fields(h: &Harness<'_, PhotocraftApp>) -> Option<Map<String, Value>> {
    h.state().ui.dialogs.iter().find(|d| owns(&d.fields)).map(|d| d.fields.clone())
}

#[test]
fn can_confirm_requires_two_files_for_load_stack_in_files_mode() {
    let mut f = Map::new();
    f.insert(MARK.into(), json!(true));
    f.insert("__command".into(), json!(COMMAND));
    f.insert("use".into(), json!("files"));

    // 0 files: cannot confirm
    f.insert("files".into(), json!([]));
    assert!(!can_confirm(&f), "0 files should not confirm");

    // 1 file: cannot confirm (#14000)
    f.insert("files".into(), json!(["/tmp/a.png"]));
    assert!(!can_confirm(&f), "1 file should not confirm");

    // 2 files: can confirm
    f.insert("files".into(), json!(["/tmp/a.png", "/tmp/b.png"]));
    assert!(can_confirm(&f), "2 files should confirm");

    // 3 files: can confirm
    f.insert("files".into(), json!(["/tmp/a.png", "/tmp/b.png", "/tmp/c.png"]));
    assert!(can_confirm(&f), "3 files should confirm");
}

#[test]
fn can_confirm_statistics_and_folder_mode_keep_existing_rules() {
    // Unrelated dialog without MARK: always confirms
    assert!(can_confirm(&Map::new()), "other dialogs unaffected");

    // Folder mode: confirms when folder is non-empty
    let mut f = Map::new();
    f.insert(MARK.into(), json!(true));
    f.insert("__command".into(), json!(COMMAND));
    f.insert("use".into(), json!("folder"));
    f.insert("folder".into(), json!(""));
    assert!(!can_confirm(&f), "empty folder cannot confirm");
    f.insert("folder".into(), json!("/tmp/photos"));
    assert!(can_confirm(&f), "chosen folder confirms");

    // Statistics dialog: keeps 1 file rule in Files mode
    let mut s = Map::new();
    s.insert(MARK.into(), json!(true));
    s.insert("__command".into(), json!(STATISTICS));
    s.insert("use".into(), json!("files"));
    s.insert("files".into(), json!([]));
    assert!(!can_confirm(&s), "Statistics with 0 files cannot confirm");
    s.insert("files".into(), json!(["/tmp/single.png"]));
    assert!(can_confirm(&s), "Statistics with 1 file confirms");
}

#[test]
fn load_files_into_stack_ui_greys_ok_and_align_until_two_files() {
    let (_, paths) = pngs("ui-stack", &["a.png", "b.png"]);
    let mut h = harness(vec![
        Some(FileDialogAnswer::Paths(vec![paths[0].clone()])),
        Some(FileDialogAnswer::Paths(vec![paths[1].clone()])),
    ]);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, COMMAND, json!({})).unwrap();
    h.run_steps(4);

    let align_label = "Attempt to Automatically Align Source Images";
    let smart_label = "Create Smart Object after Loading Layers";

    // 1. Initial state (0 files): OK and Align are disabled, Smart Object is enabled
    assert!(h.get_by_label("OK").accesskit_node().is_disabled(), "OK disabled with 0 files");
    assert!(h.get_by_label(align_label).accesskit_node().is_disabled(), "Align disabled with 0 files");
    assert!(!h.get_by_label(smart_label).accesskit_node().is_disabled(), "Smart Object stays enabled");

    // 2. Add 1 file: OK and Align remain disabled
    h.get_by_label("Browse…").click();
    h.run_steps(4);
    assert!(h.get_by_label("OK").accesskit_node().is_disabled(), "OK disabled with 1 file");
    assert!(h.get_by_label(align_label).accesskit_node().is_disabled(), "Align disabled with 1 file");
    assert!(!h.get_by_label(smart_label).accesskit_node().is_disabled(), "Smart Object enabled");

    // 3. Add 2nd file: OK and Align become enabled
    h.get_by_label("Browse…").click();
    h.run_steps(4);
    assert!(!h.get_by_label("OK").accesskit_node().is_disabled(), "OK enabled with 2 files");
    assert!(!h.get_by_label(align_label).accesskit_node().is_disabled(), "Align enabled with 2 files");

    // 4. Tick Align checkbox while enabled
    h.get_by_label(align_label).click();
    h.run_steps(2);
    assert_eq!(stack_fields(&h).unwrap()["align"], true, "align ticked");

    // 5. Select and Remove 1 file (count drops to 1): OK and Align grey out, but align remains true
    h.get_by_label("b.png").click();
    h.run_steps(2);
    h.get_by_label("Remove").click();
    h.run_steps(2);

    assert!(h.get_by_label("OK").accesskit_node().is_disabled(), "OK disabled after drop to 1 file");
    assert!(h.get_by_label(align_label).accesskit_node().is_disabled(), "Align greyed with 1 file");
    assert_eq!(stack_fields(&h).unwrap()["align"], true, "align ticked state preserved while greyed");

    // 6. Dismiss dialog
    h.get_by_label("Cancel").click();
    h.run_steps(2);
}

#[test]
fn eval_multi_iteration_add_remove_invariants() {
    let (_, paths) = pngs("multi-eval", &["f1.png", "f2.png", "f3.png"]);

    // Test across 5 full cycle iterations of dynamic addition, removal, and checkbox state retention
    for iter in 0..5 {
        let mut h = harness(vec![
            Some(FileDialogAnswer::Paths(vec![paths[0].clone()])),
            Some(FileDialogAnswer::Paths(vec![paths[1].clone()])),
            Some(FileDialogAnswer::Paths(vec![paths[2].clone()])),
        ]);
        let ctx = h.ctx.clone();
        crate::menus::invoke(h.state_mut(), &ctx, COMMAND, json!({})).unwrap();
        h.run_steps(4);

        let align_label = "Attempt to Automatically Align Source Images";

        // Invariant 0: Initial state (0 files)
        assert!(h.get_by_label("OK").accesskit_node().is_disabled(), "iter {iter}: 0 files OK disabled");
        assert!(h.get_by_label(align_label).accesskit_node().is_disabled(), "iter {iter}: 0 files Align disabled");

        // Add 1 file
        h.get_by_label("Browse…").click();
        h.run_steps(4);
        assert!(h.get_by_label("OK").accesskit_node().is_disabled(), "iter {iter}: 1 file OK disabled");
        assert!(h.get_by_label(align_label).accesskit_node().is_disabled(), "iter {iter}: 1 file Align disabled");

        // Add 2nd file -> now 2 files
        h.get_by_label("Browse…").click();
        h.run_steps(4);
        assert!(!h.get_by_label("OK").accesskit_node().is_disabled(), "iter {iter}: 2 files OK enabled");
        assert!(!h.get_by_label(align_label).accesskit_node().is_disabled(), "iter {iter}: 2 files Align enabled");

        // Tick Align
        h.get_by_label(align_label).click();
        h.run_steps(2);
        assert_eq!(stack_fields(&h).unwrap()["align"], true);

        // Add 3rd file -> 3 files
        h.get_by_label("Browse…").click();
        h.run_steps(4);
        assert!(!h.get_by_label("OK").accesskit_node().is_disabled(), "iter {iter}: 3 files OK enabled");
        assert!(!h.get_by_label(align_label).accesskit_node().is_disabled(), "iter {iter}: 3 files Align enabled");
        assert_eq!(stack_fields(&h).unwrap()["align"], true);

        // Remove two files down to 1 file
        h.get_by_label("f3.png").click();
        h.run_steps(2);
        h.get_by_label("Remove").click();
        h.run_steps(2);
        h.get_by_label("f2.png").click();
        h.run_steps(2);
        h.get_by_label("Remove").click();
        h.run_steps(2);

        // Invariant 1: 1 file left -> disabled again, align tick preserved
        assert!(h.get_by_label("OK").accesskit_node().is_disabled(), "iter {iter}: 1 file left OK disabled");
        assert!(h.get_by_label(align_label).accesskit_node().is_disabled(), "iter {iter}: 1 file left Align disabled");
        assert_eq!(stack_fields(&h).unwrap()["align"], true, "iter {iter}: tick retained");

        h.get_by_label("Cancel").click();
        h.run_steps(2);
    }
}
