use crate::PhotocraftApp;
use crate::file_dialog::{FileDialogAnswer, FileDialogRequest};
use crate::state::DialogKind;
use egui::accesskit::Role;
use egui_kittest::{Harness, kittest::Queryable};
use serde_json::{Value, json};

const EXPORT: &str = "file.export.artboardsToFiles";

fn app(cc: &eframe::CreationContext<'_>, services: crate::Services) -> PhotocraftApp {
    PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.run("file.new", json!({"name": "Storyboard.v2.psd", "width": 100, "height": 80})).unwrap();
    app.run("layer.new.artboard", json!({"name": "Red Board", "rect": [0, 0, 100, 80], "background": "custom", "color": "#cc3333"})).unwrap();
    app
}

fn open(h: &mut Harness<'_, PhotocraftApp>, command: &str) -> u64 {
    let ctx = h.ctx.clone();
    let result = crate::menus::invoke(h.state_mut(), &ctx, command, json!({})).unwrap();
    h.run_steps(4);
    result["dialog"].as_u64().unwrap()
}

fn set(h: &mut Harness<'_, PhotocraftApp>, id: u64, key: &str, value: Value) {
    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert(key.into(), value);
    h.run_steps(3);
}

fn click(h: &mut Harness<'_, PhotocraftApp>, label: &str) {
    h.get_by_role_and_label(Role::Button, label).click();
    h.run_steps(4);
}

#[test]
fn artboard_destination_uses_the_native_picker_and_resets_on_open() {
    let (show, requests) = crate::file_dialog::fake(vec![Some(FileDialogAnswer::FolderTo("/tmp/boards".into())), None]);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1100.0, 760.0))
        .build_eframe(move |cc| app(cc, crate::Services { file_dialog: Some(show), ..Default::default() }));
    let id = open(&mut h, EXPORT);
    h.get_by_role_and_label(Role::TextInput, "Destination:");
    h.get_by_role_and_label(Role::TextInput, "File Name Prefix:");
    h.get_by_role_and_label(Role::Button, "Run");
    assert_eq!(h.state().ui.dialogs[0].fields["dir"], "");
    assert_eq!(h.state().ui.dialogs[0].fields["prefix"], "Storyboard.v2");

    click(&mut h, "Browse…");
    assert_eq!(h.state().ui.dialogs[0].fields["dir"], "/tmp/boards");
    click(&mut h, "Browse…");
    assert!(
        matches!(requests.borrow().as_slice(), [FileDialogRequest::Folder { title: Some(first), .. }, FileDialogRequest::Folder { title: Some(second), .. }]
        if first == "Select Destination" && second == "Select Destination")
    );
    assert_eq!(h.state().ui.dialogs[0].fields["dir"], "/tmp/boards");

    set(&mut h, id, "prefix", json!("custom"));
    click(&mut h, "Cancel");
    h.state_mut().session.active_mut().unwrap().path = Some("/tmp/Storyboard.v2.psd".into());
    open(&mut h, EXPORT);
    assert_eq!(h.state().ui.dialogs[0].fields["dir"], "");
    assert_eq!(h.state().ui.dialogs[0].fields["prefix"], "Storyboard.v2");
    click(&mut h, "Cancel");

    h.state_mut().run("layerComp.new", json!({"name": "Comp"})).unwrap();
    open(&mut h, "file.export.layerCompsToFiles");
    h.get_by_role_and_label(Role::TextInput, "Dir");
    h.get_by_role_and_label(Role::TextInput, "Prefix");
    h.get_by_role_and_label(Role::Button, "OK");
    assert!(h.query_by_role_and_label(Role::Button, "Run").is_none());
    assert_eq!(h.state().ui.dialogs[0].fields["dir"], "/tmp");
}

fn snapshot(h: &mut Harness<'_, PhotocraftApp>, dir: Option<&std::path::Path>, name: &str) {
    if let Some(dir) = dir {
        h.render().unwrap().save(dir.join(format!("{name}.png"))).unwrap();
    }
}

fn export_workflow(snapshots: Option<&std::path::Path>) {
    let builder = Harness::builder().with_size(egui::vec2(1100.0, 760.0));
    let builder = if snapshots.is_some() { builder.wgpu() } else { builder };
    let mut h = builder.build_eframe(|cc| app(cc, Default::default()));
    let id = open(&mut h, EXPORT);
    snapshot(&mut h, snapshots, "dialog");

    h.key_press(egui::Key::Enter);
    h.run_steps(4);
    assert_eq!(h.state().ui.dialogs.last().unwrap().fields["message"], "Choose a destination folder.");
    click(&mut h, "OK");

    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing");
    set(&mut h, id, "dir", json!(missing));
    set(&mut h, id, "prefix", json!("Custom"));
    set(&mut h, id, "format", json!("jpg"));
    let fields = h.state().ui.dialogs[0].fields.clone();
    click(&mut h, "Run");
    let error = format!("Destination does not exist: {}", missing.display());
    assert_eq!(h.state().ui.dialogs.last().unwrap().fields["message"], error);
    assert!(!missing.exists());
    snapshot(&mut h, snapshots, "missing-destination");

    click(&mut h, "OK");
    assert_eq!(h.state().ui.dialogs.len(), 1);
    assert_eq!(h.state().ui.dialogs[0].id, id);
    assert_eq!(h.state().ui.dialogs[0].fields, fields);
    snapshot(&mut h, snapshots, "recovered-dialog");

    set(&mut h, id, "dir", json!(dir.path()));
    click(&mut h, "Run");
    let dialogs = &h.state().ui.dialogs;
    assert_eq!(dialogs.len(), 1);
    assert_eq!(dialogs[0].kind, DialogKind::Error);
    assert_eq!(crate::dialogs::title(&dialogs[0]), "Artboards to Files");
    let message = format!("Artboards To Files was successful.\n{}", dir.path().display());
    h.get_by_label(&message);
    assert!(dir.path().join("Custom_0000_Red Board.jpg").is_file());
    snapshot(&mut h, snapshots, "success");

    click(&mut h, "OK");
    open(&mut h, EXPORT);
    assert_eq!(h.state().ui.dialogs[0].fields["dir"], "");
    assert_eq!(h.state().ui.dialogs[0].fields["prefix"], "Storyboard.v2");
}

#[test]
fn run_recovers_from_destination_errors_and_confirms_success() {
    export_workflow(None);
}

#[test]
fn artboards_to_files_gpu_snapshots() {
    let Some(dir) = std::env::var_os("PHOTOCRAFT_ARTBOARD_EXPORT_SNAPSHOTS").map(std::path::PathBuf::from) else { return };
    let _gpu = photocraft_testkit::gpu_lock();
    std::fs::create_dir_all(&dir).unwrap();
    export_workflow(Some(&dir));
}
