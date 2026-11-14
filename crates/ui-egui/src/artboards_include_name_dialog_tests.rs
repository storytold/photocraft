//! #13637: the Artboards to Files dialog has Include Artboard Name (unticked), kept per file type;
//! ticking it shows the Name Options (Font Arial, Style Regular, Size 12 pt, Canvas Extension
//! White / Black / Other..., Text black), and OK passes them to `file.export.artboardsToFiles`.
//! Layer Comps to Files has none of it.
use crate::PhotocraftApp;
use egui::accesskit::{Role, Toggled};
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

const ARTBOARDS: &str = "file.export.artboardsToFiles";
const INCLUDE: &str = "Include Artboard Name";

fn harness() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(photocraft_engine::Session::new(), Default::default())
    });
    let app = h.state_mut();
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.run("layer.new.artboard", json!({"rect": [0, 0, 64, 64]})).unwrap();
    app.run("layerComp.new", json!({"name": "Comp"})).unwrap();
    app.sync_views();
    h
}

fn open(h: &mut Harness<'_, PhotocraftApp>, id: &str) -> u64 {
    let ctx = h.ctx.clone();
    let dialog = crate::menus::invoke(h.state_mut(), &ctx, id, json!({})).unwrap()["dialog"].as_u64().unwrap();
    h.run_steps(4);
    dialog
}

fn ticked(h: &Harness<'_, PhotocraftApp>) -> bool {
    h.get_by_role_and_label(Role::CheckBox, INCLUDE).accesskit_node().toggled() == Some(Toggled::True)
}

fn params(h: &mut Harness<'_, PhotocraftApp>, dialog: u64) -> Value {
    crate::filter_dialog::params_of(&h.state_mut().ui.dialog_mut(dialog).unwrap().fields)
}

fn set(h: &mut Harness<'_, PhotocraftApp>, dialog: u64, key: &str, v: Value) {
    h.state_mut().ui.dialog_mut(dialog).unwrap().fields.insert(key.into(), v);
    h.run_steps(4);
}

/// Labels of the rows the dialog shows now.
fn has_label(h: &Harness<'_, PhotocraftApp>, label: &str) -> bool {
    h.query_all_by_label(label).next().is_some()
}

#[test]
fn the_checkbox_starts_unticked_and_ticking_it_shows_the_name_options_with_their_defaults() {
    let mut h = harness();
    let dialog = open(&mut h, ARTBOARDS);
    assert!(!ticked(&h));
    assert!(!has_label(&h, "Name Options") && !has_label(&h, "Font:") && !has_label(&h, "Canvas Extension:"));
    assert_eq!(params(&mut h, dialog)["includeName"], false);
    h.get_by_role_and_label(Role::CheckBox, INCLUDE).click();
    h.run_steps(4);
    assert!(ticked(&h));
    for label in ["Name Options", "Font:", "Style", "Size (pt)", "Canvas Extension:", "Text:"] {
        assert!(has_label(&h, label), "{label} is shown");
    }
    // The Other... colour swatch only with Other... chosen.
    assert!(!has_label(&h, "Canvas Extension Color"));
    let p = params(&mut h, dialog);
    assert_eq!(p["includeName"], true);
    assert_eq!((&p["font"], &p["fontStyle"], &p["fontSize"], &p["extension"]), (&json!("Arial"), &json!("Regular"), &json!("12"), &json!("white")));
    assert_eq!(p["textColor"], json!([0.0, 0.0, 0.0]));
    let fields = &h.state_mut().ui.dialog_mut(dialog).unwrap().fields;
    let choices = &fields["__choices"];
    assert_eq!(choices["fontStyle"], json!(["Regular", "Italic", "Bold", "Bold Italic", "Black"]));
    assert_eq!(choices["extension"], json!(["white", "black", "other"]));
    assert!(choices["font"].as_array().unwrap().iter().any(|f| f == "Arial"));
    assert_eq!(fields["__labels"]["other"], "Other...");
    set(&mut h, dialog, "extension", json!("other"));
    assert!(has_label(&h, "Canvas Extension Color"));
}

#[test]
fn the_checkbox_is_kept_per_file_type() {
    let mut h = harness();
    let dialog = open(&mut h, ARTBOARDS);
    h.get_by_role_and_label(Role::CheckBox, INCLUDE).click();
    h.run_steps(4);
    assert!(ticked(&h));
    set(&mut h, dialog, "format", json!("psd"));
    assert!(!ticked(&h), "PSD keeps its own, unticked");
    assert!(!has_label(&h, "Font:"));
    set(&mut h, dialog, "format", json!("tiff"));
    assert!(!ticked(&h));
    set(&mut h, dialog, "format", json!("png"));
    assert!(ticked(&h), "PNG's tick comes back");
    assert_eq!(params(&mut h, dialog)["includeName"], true);
}

#[test]
fn ok_runs_the_export_with_the_name_options() {
    let mut h = harness();
    let dialog = open(&mut h, ARTBOARDS);
    // Removed when dropped, also when an assertion fails.
    let dir = tempfile::tempdir().unwrap();
    set(&mut h, dialog, "dir", json!(dir.path().to_string_lossy()));
    h.get_by_role_and_label(Role::CheckBox, INCLUDE).click();
    h.run_steps(4);
    set(&mut h, dialog, "extension", json!("black"));
    let p = params(&mut h, dialog);
    let r = h.state_mut().run(ARTBOARDS, p).unwrap();
    let file = r["files"][0].as_str().unwrap().to_string();
    let img = photocraft_io::import("x.png", &std::fs::read(&file).unwrap()).unwrap().document;
    // A 64 x 64 board gets a border of (64 + 64) / 20 = 6 px on each side.
    assert_eq!((img.size.width, img.size.height), (76, 76));
    // A size that is not a number fails the run.
    set(&mut h, dialog, "fontSize", json!("abc"));
    let p = params(&mut h, dialog);
    assert!(h.state_mut().run(ARTBOARDS, p).is_err());
}

#[test]
fn layer_comps_to_files_has_no_artboard_name() {
    let mut h = harness();
    let dialog = open(&mut h, "file.export.layerCompsToFiles");
    assert_eq!(h.query_all_by_role_and_label(Role::CheckBox, INCLUDE).count(), 0);
    assert!(h.state_mut().ui.dialog_mut(dialog).unwrap().fields.get("includeName").is_none());
}
