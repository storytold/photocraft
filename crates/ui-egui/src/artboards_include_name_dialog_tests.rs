//! #13637: the Artboards to Files dialog has Include Artboard Name (unticked), kept per file type;
//! ticking it shows the Name Options (Font Arial, Style Regular, Size 12 pt, Canvas Extension
//! White / Black / Other..., Text black), and OK passes them to `file.export.artboardsToFiles`.
//! Layer Comps to Files has none of it.
use crate::PhotocraftApp;
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::HostedParamDialog;
use egui::accesskit::{Role, Toggled};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

const ARTBOARDS: &str = "file.export.artboardsToFiles";
const INCLUDE: &str = "Include Artboard Name";

type H = Harness<'static, Box<PhotocraftApp>>;

fn harness() -> H {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.run("layer.new.artboard", json!({"rect": [0, 0, 64, 64]})).unwrap();
    app.run("layerComp.new", json!({"name": "Comp"})).unwrap();
    app.sync_views();
    let h = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .with_max_steps(64)
        .with_context(app.ctx().clone())
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h
}

fn open(h: &mut H, command: &str) -> DialogId {
    let ctx = h.ctx.clone();
    let reply = crate::menus::invoke(h.state_mut(), &ctx, command, json!({})).unwrap();
    let want = reply["dialog"].as_str().unwrap().to_owned();
    let dialog = h.state().framework.dialogs.open_dialogs().into_iter().map(|d| d.id).find(|d| d.value().to_string() == want).unwrap();
    h.run_steps(4);
    dialog
}

fn ticked(h: &H) -> bool {
    h.get_by_role_and_label(Role::CheckBox, INCLUDE).accesskit_node().toggled() == Some(Toggled::True)
}

fn click(h: &mut H) {
    h.get_by_role_and_label(Role::CheckBox, INCLUDE).click();
    h.run_steps(4);
}

/// What OK sends to the command.
fn params(h: &H, dialog: DialogId) -> Value {
    let form = h.state().framework.dialogs.get_by_id::<HostedParamDialog>(dialog).unwrap();
    crate::filter_dialog::command_params(ARTBOARDS, form.fields())
}

/// Sets a field as automation (`ui.dialog.set`) does, then draws.
fn set(h: &mut H, dialog: DialogId, key: &str, v: Value) {
    let ctx = h.ctx.clone();
    h.state_mut().framework.dialogs.set_json_field(&ctx, dialog, key, v).unwrap_or_else(|e| panic!("{key}: {e}"));
    h.run_steps(4);
}

fn has_label(h: &H, label: &str) -> bool {
    h.query_all_by_label(label).next().is_some()
}

/// The size of the first file the export wrote into `dir`.
fn written_size(dir: &std::path::Path) -> (u32, u32) {
    let file = std::fs::read_dir(dir).unwrap().next().expect("a file was written").unwrap().path();
    let name = file.to_string_lossy().into_owned();
    let img = photocraft_io::import(&name, &std::fs::read(&file).unwrap()).unwrap().document;
    (img.size.width, img.size.height)
}

#[test]
fn the_checkbox_starts_unticked_and_ticking_it_shows_the_name_options_with_their_defaults() {
    let mut h = harness();
    let dialog = open(&mut h, ARTBOARDS);
    assert!(!ticked(&h));
    assert!(!has_label(&h, "Name Options") && !has_label(&h, "Font") && !has_label(&h, "Canvas Extension"));
    let p = params(&h, dialog);
    assert_eq!(p["includeName"], false);
    assert!(p.get("font").is_none() && p.get("fontSize").is_none(), "no Name Options are sent while unticked: {p}");
    click(&mut h);
    assert!(ticked(&h));
    for label in ["Name Options", "Font", "Style", "Size (pt)", "Canvas Extension", "Text"] {
        assert!(has_label(&h, label), "{label} is shown");
    }
    // The Other... colour swatch only with Other... chosen.
    assert!(!has_label(&h, "Canvas Extension Color"));
    let p = params(&h, dialog);
    assert_eq!(p["includeName"], true);
    assert_eq!((&p["font"], &p["fontStyle"], &p["fontSize"], &p["extension"]), (&json!("Arial"), &json!("Regular"), &json!("12"), &json!("white")));
    assert_eq!(p["textColor"], json!("#000000"));
    assert!(p.get("extensionColor").is_none());
    // The choices the rows offer: automation takes each, and refuses what is not offered.
    for style in ["Regular", "Italic", "Bold", "Bold Italic", "Black"] {
        set(&mut h, dialog, "fontStyle", json!(style));
    }
    let ctx = h.ctx.clone();
    assert!(h.state_mut().framework.dialogs.set_json_field(&ctx, dialog, "fontStyle", json!("Light")).is_err());
    set(&mut h, dialog, "font", json!("Arial"));
    set(&mut h, dialog, "extension", json!("other"));
    assert!(has_label(&h, "Canvas Extension Color"));
    assert!(params(&h, dialog).get("extensionColor").is_some());
    // Unticked again, the swatch goes with the rest.
    click(&mut h);
    assert!(!has_label(&h, "Canvas Extension Color") && !has_label(&h, "Font"));
}

#[test]
fn the_checkbox_is_kept_per_file_type() {
    let mut h = harness();
    let dialog = open(&mut h, ARTBOARDS);
    set(&mut h, dialog, "format", json!("png24"));
    click(&mut h);
    assert!(ticked(&h));
    set(&mut h, dialog, "format", json!("psd"));
    assert!(!ticked(&h), "PSD keeps its own, unticked");
    assert!(!has_label(&h, "Font"));
    set(&mut h, dialog, "format", json!("tiff"));
    assert!(!ticked(&h));
    set(&mut h, dialog, "format", json!("png24"));
    assert!(ticked(&h), "PNG-24's tick comes back");
    assert_eq!(params(&h, dialog)["includeName"], true);
}

#[test]
fn ok_runs_the_export_with_the_name_options() {
    let mut h = harness();
    let dialog = open(&mut h, ARTBOARDS);
    // Removed when dropped, also when an assertion fails.
    let dir = tempfile::tempdir().unwrap();
    set(&mut h, dialog, "dir", json!(dir.path().to_string_lossy()));
    set(&mut h, dialog, "format", json!("png24"));
    click(&mut h);
    set(&mut h, dialog, "extension", json!("black"));
    crate::dialogs::testing::confirm(h.state_mut(), dialog).unwrap();
    h.run_steps(2);
    // A 64 x 64 board gets a border of (64 + 64) / 20 = 6 px on each side.
    assert_eq!(written_size(dir.path()), (76, 76));
    // A size that is not a number fails the run.
    let failed = tempfile::tempdir().unwrap();
    let p = json!({"dir": failed.path(), "format": "png", "includeName": true, "fontSize": "abc"});
    assert!(h.state_mut().run(ARTBOARDS, p).is_err());
}

#[test]
fn layer_comps_to_files_has_no_artboard_name() {
    let mut h = harness();
    let dialog = open(&mut h, "file.export.layerCompsToFiles");
    assert_eq!(h.query_all_by_role_and_label(Role::CheckBox, INCLUDE).count(), 0);
    let form = h.state().framework.dialogs.get_by_id::<HostedParamDialog>(dialog).unwrap();
    assert!(form.fields().get("includeName").is_none());
}

/// A control client that picks another file type and confirms before the dialog is drawn again
/// exports with that type's own Include Artboard Name, not the tick of the type it left.
#[test]
fn a_file_type_set_by_a_control_client_brings_its_own_tick_at_once() {
    let mut h = harness();
    let dialog = open(&mut h, ARTBOARDS);
    let dir = tempfile::tempdir().unwrap();
    set(&mut h, dialog, "dir", json!(dir.path().to_string_lossy()));
    set(&mut h, dialog, "format", json!("png24"));
    click(&mut h);
    assert!(ticked(&h), "PNG-24 ticked");
    let ctx = h.ctx.clone();
    h.state_mut().framework.dialogs.set_json_field(&ctx, dialog, "format", json!("jpg")).unwrap();
    // OK in the same frame: no border around the board.
    crate::dialogs::testing::confirm(h.state_mut(), dialog).unwrap();
    assert_eq!(written_size(dir.path()), (64, 64));
}
