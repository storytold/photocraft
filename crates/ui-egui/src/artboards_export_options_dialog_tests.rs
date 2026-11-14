//! #13636: Artboards to Files' Export Options checkbox (unticked by default, below File Type)
//! shows the chosen file type's options with the reference application's defaults, sends them
//! to the command only while ticked, and remembers them after a successful run only.
use crate::PhotocraftApp;
use egui::accesskit::{Role, Toggled};
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

const ARTBOARDS: &str = "file.export.artboardsToFiles";
const OPTION_PARAMS: [&str; 9] =
    ["iccProfile", "bmpBits", "tgaBits", "pngInterlaced", "transparency", "tiffCompression", "maximizeCompatibility", "preserveArtboard", "quality"];

fn harness() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(photocraft_engine::Session::new(), Default::default())
    });
    let app = h.state_mut();
    app.run("file.new", json!({"width": 64, "height": 64, "background": "transparent"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#8A42B3"})).unwrap();
    app.run("layer.new.artboardFromLayers", json!({"name": "Board", "background": "transparent"})).unwrap();
    app.sync_views();
    h
}

fn open(h: &mut Harness<'_, PhotocraftApp>, dir: &str, format: &str) -> u64 {
    let ctx = h.ctx.clone();
    let dialog = crate::menus::invoke(h.state_mut(), &ctx, ARTBOARDS, json!({})).unwrap()["dialog"].as_u64().unwrap();
    set(h, dialog, &[("dir", json!(dir)), ("prefix", json!("out")), ("format", json!(format))]);
    dialog
}

fn set(h: &mut Harness<'_, PhotocraftApp>, dialog: u64, kv: &[(&str, Value)]) {
    let fields = &mut h.state_mut().ui.dialog_mut(dialog).unwrap().fields;
    for (k, v) in kv {
        fields.insert((*k).into(), v.clone());
    }
    h.run_steps(4);
}

fn params(h: &mut Harness<'_, PhotocraftApp>, dialog: u64) -> Value {
    crate::filter_dialog::params_of(&h.state_mut().ui.dialog_mut(dialog).unwrap().fields)
}

/// The option params the dialog sends (the null ones left out).
fn sent(h: &mut Harness<'_, PhotocraftApp>, dialog: u64) -> Value {
    let p = params(h, dialog);
    Value::Object(OPTION_PARAMS.iter().filter_map(|k| p.get(*k).filter(|v| !v.is_null()).map(|v| ((*k).to_string(), v.clone()))).collect())
}

fn ticked(h: &Harness<'_, PhotocraftApp>, label: &str) -> bool {
    h.get_by_role_and_label(Role::CheckBox, label).accesskit_node().toggled() == Some(Toggled::True)
}

fn shown(h: &Harness<'_, PhotocraftApp>, label: &str) -> bool {
    h.query_by_label(label).is_some()
}

fn tick_export_options(h: &mut Harness<'_, PhotocraftApp>) {
    h.get_by_role_and_label(Role::CheckBox, "Export Options").click();
    h.run_steps(3);
}

fn quality_field_enabled(h: &Harness<'_, PhotocraftApp>) -> bool {
    !h.get_by_role_and_label(Role::SpinButton, "Quality").accesskit_node().is_disabled()
}

fn type_quality(h: &mut Harness<'_, PhotocraftApp>, text: &str) {
    h.get_by_role_and_label(Role::SpinButton, "Quality").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    for ch in text.chars() {
        h.event(egui::Event::Text(ch.to_string()));
        h.run_steps(1);
    }
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
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
    let dialog = open(&mut h, &dir, "png");
    assert!(!ticked(&h, "Export Options"));
    assert!(!shown(&h, "Include ICC Profile"), "the Options group is hidden");
    assert_eq!(sent(&mut h, dialog), json!({}), "no option reaches the command");
    assert!(!shown(&h, "Custom"), "no Custom quality row");
    let order = h.state_mut().ui.dialog_mut(dialog).unwrap().fields["__order"].clone();
    assert_eq!(order, json!(["dir", "prefix", "format", "exportOptions"]), "below File Type");
    let below = h.get_by_role_and_label(Role::CheckBox, "Export Options").rect().top();
    let format = h.get_by_role_and_label(Role::ComboBox, "Format").rect().top();
    assert!(below > format, "{below} > {format}");
}

#[test]
fn each_file_type_shows_its_options_with_the_reference_defaults() {
    let mut h = harness();
    let (_t, dir) = tmp();
    let dialog = open(&mut h, &dir, "png");
    tick_export_options(&mut h);
    assert!(shown(&h, "Options:"));
    let cases = [
        ("psd", vec!["Maximize Compatibility", "Preserve Artboard"], json!({"maximizeCompatibility": true, "preserveArtboard": true, "iccProfile": true})),
        ("bmp", vec!["16bit", "24bit", "32bit"], json!({"bmpBits": 24, "iccProfile": true})),
        ("jpg", vec!["Quality"], json!({"quality": 8.0, "iccProfile": true})),
        ("tiff", vec!["Image Compression", "Quality"], json!({"tiffCompression": "none", "iccProfile": true})),
        ("png", vec!["Transparency", "Interlaced"], json!({"transparency": true, "pngInterlaced": false, "iccProfile": true})),
    ];
    for (format, labels, defaults) in cases {
        set(&mut h, dialog, &[("format", json!(format))]);
        for l in labels {
            assert!(shown(&h, l), "{format}: {l}");
        }
        assert!(ticked(&h, "Include ICC Profile"), "{format}");
        assert_eq!(sent(&mut h, dialog), defaults, "{format}");
    }
    assert!(ticked(&h, "Transparency") && !ticked(&h, "Interlaced"));
    set(&mut h, dialog, &[("format", json!("bmp"))]);
    let radio = |h: &Harness<'_, PhotocraftApp>, l: &str| h.get_by_role_and_label(Role::RadioButton, l).accesskit_node().toggled() == Some(Toggled::True);
    assert!(radio(&h, "24bit") && !radio(&h, "16bit") && !radio(&h, "32bit"));
    h.get_by_role_and_label(Role::RadioButton, "32bit").click();
    h.run_steps(3);
    assert_eq!(sent(&mut h, dialog)["bmpBits"], json!(32));
    // Each type keeps its own settings.
    set(&mut h, dialog, &[("format", json!("png"))]);
    assert_eq!(sent(&mut h, dialog), json!({"transparency": true, "pngInterlaced": false, "iccProfile": true}));
    set(&mut h, dialog, &[("format", json!("bmp"))]);
    assert_eq!(sent(&mut h, dialog)["bmpBits"], json!(32));
    // Unticked again: the group is hidden and nothing is sent.
    tick_export_options(&mut h);
    assert!(!shown(&h, "Include ICC Profile") && !shown(&h, "32bit"));
    assert_eq!(sent(&mut h, dialog), json!({}));
}

#[test]
fn tiff_quality_is_greyed_unless_the_compression_is_jpeg() {
    let mut h = harness();
    let (_t, dir) = tmp();
    let dialog = open(&mut h, &dir, "tiff");
    tick_export_options(&mut h);
    assert!(!quality_field_enabled(&h));
    assert!(sent(&mut h, dialog).get("quality").is_none());
    let mut opts = h.state_mut().ui.dialog_mut(dialog).unwrap().fields["__exportOpts"].clone();
    opts["tiff"]["tiffCompression"] = json!("jpeg");
    set(&mut h, dialog, &[("__exportOpts", opts)]);
    assert!(quality_field_enabled(&h));
    assert_eq!(sent(&mut h, dialog), json!({"tiffCompression": "jpeg", "quality": 8.0, "iccProfile": true}));
}

#[test]
fn the_quality_field_clamps_keeps_fractions_and_ignores_text() {
    let mut h = harness();
    let (_t, dir) = tmp();
    let dialog = open(&mut h, &dir, "jpg");
    tick_export_options(&mut h);
    for (typed, expect) in [("99", 12.0), ("-5", 0.0), ("7.6", 7.6), ("abc", 7.6)] {
        type_quality(&mut h, typed);
        let q = sent(&mut h, dialog)["quality"].as_f64().unwrap();
        assert!((q - expect).abs() < 1e-4, "{typed}: {q}");
    }
}

#[test]
fn the_options_reach_the_written_file() {
    let mut h = harness();
    let (_t, dir) = tmp();
    let dialog = open(&mut h, &dir, "png");
    tick_export_options(&mut h);
    h.get_by_role_and_label(Role::CheckBox, "Interlaced").click();
    h.get_by_role_and_label(Role::CheckBox, "Transparency").click();
    h.run_steps(3);
    let r = crate::dialogs::confirm(h.state_mut(), dialog).unwrap();
    let png = std::fs::read(r["files"][0].as_str().unwrap()).unwrap();
    assert_eq!((png[25], png[28]), (2, 1), "RGB without alpha, interlaced");
}

#[test]
fn options_are_remembered_after_a_successful_run_only() {
    let mut h = harness();
    let (_t, dir) = tmp();
    let dialog = open(&mut h, &dir, "png");
    tick_export_options(&mut h);
    h.get_by_role_and_label(Role::CheckBox, "Interlaced").click();
    h.run_steps(3);
    crate::dialogs::confirm(h.state_mut(), dialog).unwrap();
    h.run_steps(3);
    let dialog = open(&mut h, &dir, "png");
    assert!(ticked(&h, "Export Options") && ticked(&h, "Interlaced"), "remembered after the run");
    // A run that fails (16-bit BMP is refused) is not remembered.
    set(&mut h, dialog, &[("format", json!("bmp"))]);
    h.get_by_role_and_label(Role::RadioButton, "16bit").click();
    h.get_by_role_and_label(Role::CheckBox, "Include ICC Profile").click();
    h.run_steps(3);
    assert!(crate::dialogs::confirm(h.state_mut(), dialog).is_err());
    h.state_mut().ui.dialogs.clear();
    h.run_steps(3);
    let dialog = open(&mut h, &dir, "bmp");
    assert_eq!(sent(&mut h, dialog), json!({"bmpBits": 24, "iccProfile": true}), "the last successful run's settings");
    set(&mut h, dialog, &[("format", json!("png"))]);
    assert!(ticked(&h, "Interlaced"));
}
