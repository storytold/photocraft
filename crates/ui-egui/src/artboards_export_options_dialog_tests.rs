//! #13636: Artboards to Files' Export Options checkbox (unticked by default, below File Type)
//! shows the chosen file type's options with the reference application's defaults, sends them
//! to the command only while ticked, and remembers them after a successful run only.
use crate::PhotocraftApp;
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::HostedParamDialog;
use egui::accesskit::{Role, Toggled};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

const ARTBOARDS: &str = "file.export.artboardsToFiles";
const OPTION_PARAMS: [&str; 9] =
    ["iccProfile", "bmpBits", "tgaBits", "pngInterlaced", "transparency", "tiffCompression", "maximizeCompatibility", "preserveArtboard", "quality"];

type H = Harness<'static, Box<PhotocraftApp>>;

fn harness() -> H {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 64, "height": 64, "background": "transparent"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#8A42B3"})).unwrap();
    app.run("layer.new.artboardFromLayers", json!({"name": "Board", "background": "transparent"})).unwrap();
    app.sync_views();
    let h = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .with_max_steps(64)
        .with_context(app.ctx().clone())
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h
}

fn open(h: &mut H, dir: &str, format: &str) -> DialogId {
    let ctx = h.ctx.clone();
    let reply = crate::menus::invoke(h.state_mut(), &ctx, ARTBOARDS, json!({})).unwrap();
    let want = reply["dialog"].as_str().unwrap().to_owned();
    let dialog = h.state().framework.dialogs.open_dialogs().into_iter().map(|d| d.id).find(|d| d.value().to_string() == want).unwrap();
    h.run_steps(4);
    set(h, dialog, "dir", json!(dir));
    set(h, dialog, "prefix", json!("out"));
    set(h, dialog, "format", json!(format));
    dialog
}

/// Sets a field as automation (`ui.dialog.set`) does, then draws.
fn set(h: &mut H, dialog: DialogId, key: &str, v: Value) {
    let ctx = h.ctx.clone();
    h.state_mut().framework.dialogs.set_json_field(&ctx, dialog, key, v).unwrap_or_else(|e| panic!("{key}: {e}"));
    h.run_steps(4);
}

/// The option params OK sends to the command.
fn sent(h: &H, dialog: DialogId) -> Value {
    let form = h.state().framework.dialogs.get_by_id::<HostedParamDialog>(dialog).unwrap();
    let p = crate::filter_dialog::command_params(ARTBOARDS, form.fields());
    assert!(p.as_object().unwrap().keys().all(|k| !k.contains('.') && k != "exportOptions"), "only plain params are sent: {p}");
    Value::Object(OPTION_PARAMS.iter().filter_map(|k| p.get(*k).map(|v| ((*k).to_string(), v.clone()))).collect())
}

fn ticked(h: &H, label: &str) -> bool {
    h.get_by_role_and_label(Role::CheckBox, label).accesskit_node().toggled() == Some(Toggled::True)
}

fn shown(h: &H, label: &str) -> bool {
    h.query_all_by_label(label).next().is_some()
}

fn click(h: &mut H, role: Role, label: &str) {
    h.get_by_role_and_label(role, label).click();
    h.run_steps(4);
}

fn tick_export_options(h: &mut H) {
    click(h, Role::CheckBox, "Export Options");
}

fn tmp() -> (tempfile::TempDir, String) {
    let t = tempfile::tempdir().unwrap();
    let s = t.path().to_string_lossy().into_owned();
    (t, s)
}

#[test]
fn export_options_is_unticked_below_file_type_and_sends_nothing() {
    let mut h = harness();
    let (_t, dir) = tmp();
    let dialog = open(&mut h, &dir, "png24");
    assert!(!ticked(&h, "Export Options"));
    assert!(!shown(&h, "Include ICC Profile") && !shown(&h, "Options"), "the Options section is hidden");
    assert_eq!(sent(&h, dialog), json!({}), "no option reaches the command");
    let below = h.get_by_role_and_label(Role::CheckBox, "Export Options").rect().top();
    let format = h.query_all_by_label("File Type").next().unwrap().rect().top();
    assert!(below > format, "{below} > {format}");
}

#[test]
fn each_file_type_shows_its_options_with_the_reference_defaults() {
    let mut h = harness();
    let (_t, dir) = tmp();
    let dialog = open(&mut h, &dir, "png24");
    tick_export_options(&mut h);
    assert!(shown(&h, "Options"));
    let cases = [
        ("psd", vec!["Maximize Compatibility", "Preserve Artboard"], json!({"maximizeCompatibility": true, "preserveArtboard": true, "iccProfile": true})),
        ("bmp", vec!["16bit", "24bit", "32bit"], json!({"bmpBits": 24, "iccProfile": true})),
        ("jpg", vec!["Quality"], json!({"quality": 8.0, "iccProfile": true})),
        // Targa is on the File Type list (#13635).
        ("tga", vec!["16bit", "24bit", "32bit"], json!({"tgaBits": 24, "iccProfile": true})),
        ("tiff", vec!["Image Compression", "Quality"], json!({"tiffCompression": "none", "iccProfile": true})),
        ("png24", vec!["Transparency", "Interlaced"], json!({"transparency": true, "pngInterlaced": false, "iccProfile": true})),
    ];
    for (format, labels, defaults) in cases {
        set(&mut h, dialog, "format", json!(format));
        for l in labels {
            assert!(shown(&h, l), "{format}: {l}");
        }
        assert_eq!(h.query_all_by_label("Include ICC Profile").count(), 1, "{format}: only the chosen type's rows show");
        assert!(ticked(&h, "Include ICC Profile"), "{format}");
        assert_eq!(sent(&h, dialog), defaults, "{format}");
    }
    assert!(ticked(&h, "Transparency") && !ticked(&h, "Interlaced"));
    set(&mut h, dialog, "format", json!("bmp"));
    let radio = |h: &H, l: &str| h.get_by_role_and_label(Role::RadioButton, l).accesskit_node().toggled() == Some(Toggled::True);
    assert!(radio(&h, "24bit") && !radio(&h, "16bit") && !radio(&h, "32bit"));
    click(&mut h, Role::RadioButton, "32bit");
    assert_eq!(sent(&h, dialog)["bmpBits"], json!(32));
    // Each type keeps its own settings.
    set(&mut h, dialog, "format", json!("png24"));
    assert_eq!(sent(&h, dialog), json!({"transparency": true, "pngInterlaced": false, "iccProfile": true}));
    set(&mut h, dialog, "format", json!("bmp"));
    assert_eq!(sent(&h, dialog)["bmpBits"], json!(32));
    // Unticked again: the section is hidden and nothing is sent.
    tick_export_options(&mut h);
    assert!(!shown(&h, "Include ICC Profile") && !shown(&h, "32bit") && !shown(&h, "Options"));
    assert_eq!(sent(&h, dialog), json!({}));
}

#[test]
fn tiff_quality_is_greyed_unless_the_compression_is_jpeg() {
    let mut h = harness();
    let (_t, dir) = tmp();
    let dialog = open(&mut h, &dir, "tiff");
    tick_export_options(&mut h);
    let ctx = h.ctx.clone();
    assert!(h.state_mut().framework.dialogs.set_json_field(&ctx, dialog, "tiff.quality", json!(5)).is_err(), "the greyed Quality takes no input");
    assert!(sent(&h, dialog).get("quality").is_none());
    set(&mut h, dialog, "tiff.tiffCompression", json!("jpeg"));
    assert_eq!(sent(&h, dialog), json!({"tiffCompression": "jpeg", "quality": 8.0, "iccProfile": true}));
    set(&mut h, dialog, "tiff.quality", json!(5));
    assert_eq!(sent(&h, dialog)["quality"], json!(5.0));
}

#[test]
fn the_quality_keeps_fractions_and_refuses_values_past_its_range() {
    let mut h = harness();
    let (_t, dir) = tmp();
    let dialog = open(&mut h, &dir, "jpg");
    tick_export_options(&mut h);
    set(&mut h, dialog, "jpg.quality", json!(7.6));
    let q = sent(&h, dialog)["quality"].as_f64().unwrap();
    assert!((q - 7.6).abs() < 1e-4, "{q}");
    let ctx = h.ctx.clone();
    for bad in [json!(99), json!(-5), json!("abc")] {
        assert!(h.state_mut().framework.dialogs.set_json_field(&ctx, dialog, "jpg.quality", bad.clone()).is_err(), "{bad}");
    }
    let q = sent(&h, dialog)["quality"].as_f64().unwrap();
    assert!((q - 7.6).abs() < 1e-4, "a refused value leaves {q}");
}

#[test]
fn the_options_reach_the_written_file() {
    let mut h = harness();
    let (t, dir) = tmp();
    let dialog = open(&mut h, &dir, "png24");
    tick_export_options(&mut h);
    click(&mut h, Role::CheckBox, "Interlaced");
    click(&mut h, Role::CheckBox, "Transparency");
    crate::dialogs::testing::confirm(h.state_mut(), dialog).unwrap();
    let file = std::fs::read_dir(t.path()).unwrap().next().expect("a file was written").unwrap().path();
    let png = std::fs::read(file).unwrap();
    assert_eq!((png[25], png[28]), (2, 1), "RGB without alpha, interlaced");
}

#[test]
fn options_are_remembered_after_a_successful_run_only() {
    let mut h = harness();
    let (_t, dir) = tmp();
    let dialog = open(&mut h, &dir, "png24");
    tick_export_options(&mut h);
    click(&mut h, Role::CheckBox, "Interlaced");
    crate::dialogs::testing::confirm(h.state_mut(), dialog).unwrap();
    h.run_steps(3);
    let dialog = open(&mut h, &dir, "png24");
    assert!(ticked(&h, "Export Options") && ticked(&h, "Interlaced"), "remembered after the run");
    // A run that fails (16-bit BMP is refused) is not remembered.
    set(&mut h, dialog, "format", json!("bmp"));
    click(&mut h, Role::RadioButton, "16bit");
    click(&mut h, Role::CheckBox, "Include ICC Profile");
    assert_eq!(sent(&h, dialog), json!({"bmpBits": 16, "iccProfile": false}));
    crate::dialogs::testing::confirm(h.state_mut(), dialog).unwrap();
    h.run_steps(3);
    assert!(h.state().framework.dialogs.open_dialogs().iter().all(|d| d.id != dialog), "OK closed the form");
    let dialog = open(&mut h, &dir, "bmp");
    assert_eq!(sent(&h, dialog), json!({"bmpBits": 24, "iccProfile": true}), "the last successful run's settings");
    set(&mut h, dialog, "format", json!("png24"));
    assert!(ticked(&h, "Interlaced"));
}

/// Opens the editor of a recorded step with `recorded`, as Record Again and a playback with the step's dialog on do.
fn reopen(h: &mut H, recorded: Value) -> DialogId {
    let dialog = crate::panels::actions::open_recorded_editor(h.state_mut(), ARTBOARDS, &recorded).unwrap();
    h.run_steps(4);
    dialog
}

/// The editor of a recorded step has the command's own plain params, the Export Options among them: OK without
/// changes sends what was recorded, the JPEG quality too.
#[test]
fn a_reopened_recording_keeps_its_export_options() {
    let (_t, dir) = tmp();
    let mut h = harness();
    let dialog = reopen(&mut h, json!({"dir": dir, "prefix": "out", "format": "jpg", "quality": 3, "iccProfile": false}));
    let p = sent(&h, dialog);
    assert_eq!(p["quality"].as_f64(), Some(3.0), "{p}");
    assert_eq!(p["iccProfile"], false, "{p}");
    let mut h = harness();
    let dialog = reopen(&mut h, json!({"dir": dir, "format": "tiff", "tiffCompression": "lzw", "pngInterlaced": false}));
    let p = sent(&h, dialog);
    assert_eq!((&p["tiffCompression"], &p["pngInterlaced"]), (&json!("lzw"), &json!(false)), "{p}");
}
