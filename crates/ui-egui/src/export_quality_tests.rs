//! #11481: File › Export › Artboards to Files (and Layers / Layer Comps to Files) has a JPEG
//! Quality field behind a Custom checkbox, as Batch has (#3956). The quality reaches the command
//! only for a JPEG with Custom on: left alone, an export writes what the command writes with no
//! quality (the source JPEG's own quality, else the encoder default), a WebP stays lossless and
//! PNG has none.
use crate::PhotocraftApp;
use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

const ARTBOARDS: &str = "file.export.artboardsToFiles";
const EXPORTS: [&str; 3] = [ARTBOARDS, "file.export.layersToFiles", "file.export.layerCompsToFiles"];

fn harness() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(photocraft_engine::Session::new(), Default::default())
    });
    let app = h.state_mut();
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    // A noisy layer inside the board, so the JPEG quality shows in the file.
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#8A42B3"})).unwrap();
    app.run("filter.noise.addNoise", json!({"amount": 60})).unwrap();
    app.run("layer.new.artboardFromLayers", json!({"name": "Board"})).unwrap();
    app.run("layerComp.new", json!({"name": "Comp"})).unwrap();
    app.sync_views();
    h
}

fn open(h: &mut Harness<'_, PhotocraftApp>, id: &str, dir: &str, format: &str) -> u64 {
    let ctx = h.ctx.clone();
    let dialog = crate::menus::invoke(h.state_mut(), &ctx, id, json!({})).unwrap()["dialog"].as_u64().unwrap();
    let fields = &mut h.state_mut().ui.dialog_mut(dialog).unwrap().fields;
    fields.insert("dir".into(), json!(dir));
    fields.insert("prefix".into(), json!("out"));
    fields.insert("format".into(), json!(format));
    h.run_steps(4);
    dialog
}

fn params(h: &mut Harness<'_, PhotocraftApp>, dialog: u64) -> Value {
    crate::filter_dialog::params_of(&h.state_mut().ui.dialog_mut(dialog).unwrap().fields)
}

fn exported(h: &mut Harness<'_, PhotocraftApp>, dialog: u64) -> Vec<u8> {
    let r = crate::dialogs::confirm(h.state_mut(), dialog).unwrap();
    std::fs::read(r["files"][0].as_str().unwrap()).unwrap()
}

/// The same export run straight through the command, into `dir`.
fn direct(h: &mut Harness<'_, PhotocraftApp>, dir: &str, format: &str, quality: Option<i64>) -> Vec<u8> {
    let mut p = json!({"dir": dir, "prefix": "out", "format": format});
    if let Some(q) = quality {
        p["quality"] = json!(q);
    }
    let r = h.state_mut().run(ARTBOARDS, p).unwrap();
    std::fs::read(r["files"][0].as_str().unwrap()).unwrap()
}

/// Whether the Quality number field (not its row label) takes input.
fn quality_enabled(h: &Harness<'_, PhotocraftApp>) -> bool {
    let fields: Vec<bool> =
        h.query_all_by_label("Quality").filter(|n| n.accesskit_node().role() != Role::Label).map(|n| !n.accesskit_node().is_disabled()).collect();
    assert!(!fields.is_empty(), "the dialog shows a Quality field");
    assert!(fields.iter().all(|e| *e == fields[0]), "{fields:?}");
    fields[0]
}

fn tmp() -> (tempfile::TempDir, String) {
    let t = tempfile::tempdir().unwrap();
    let s = t.path().to_string_lossy().into_owned();
    (t, s)
}

#[test]
fn artboards_to_files_sends_a_custom_jpeg_quality() {
    let mut h = harness();
    let (_a, via_dialog) = tmp();
    let (_b, via_command) = tmp();
    let mut sizes = Vec::new();
    for quality in [0, 3, 12] {
        let dialog = open(&mut h, ARTBOARDS, &via_dialog, "jpg");
        assert!(!quality_enabled(&h), "JPEG with Custom off: the field is greyed out");
        h.get_by_label("Custom").click();
        h.run_steps(2);
        assert!(quality_enabled(&h), "JPEG with Custom on: the field takes input");
        assert_eq!(params(&mut h, dialog)["quality"], json!(10), "Custom starts at quality 10");
        h.state_mut().ui.dialog_mut(dialog).unwrap().fields.insert("quality".into(), json!(quality));
        h.run_steps(2);
        assert_eq!(params(&mut h, dialog)["quality"], json!(quality), "the chosen quality reaches the command");
        let bytes = exported(&mut h, dialog);
        assert!(bytes == direct(&mut h, &via_command, "jpg", Some(quality)), "quality {quality}: the file the command writes at that quality");
        sizes.push(bytes.len());
    }
    assert!(sizes[0] < sizes[1] && sizes[1] < sizes[2], "a higher quality writes a bigger JPEG: {sizes:?}");
}

#[test]
fn an_untouched_jpeg_export_writes_what_the_command_writes_with_no_quality() {
    let mut h = harness();
    let (_a, via_dialog) = tmp();
    let (_b, via_command) = tmp();
    let dialog = open(&mut h, ARTBOARDS, &via_dialog, "jpg");
    assert!(params(&mut h, dialog).get("quality").is_none_or(Value::is_null), "no quality is sent");
    let bytes = exported(&mut h, dialog);
    assert!(bytes == direct(&mut h, &via_command, "jpg", None), "the untouched dialog keeps the command's own JPEG quality");
}

#[test]
fn export_to_files_quality_leaves_png_and_webp_as_they_were() {
    let mut h = harness();
    let (_a, dir) = tmp();
    for id in EXPORTS {
        for format in ["png", "webp"] {
            let dialog = open(&mut h, id, &dir, format);
            // Custom on from an earlier JPEG choice still sends nothing for these formats.
            h.state_mut().ui.dialog_mut(dialog).unwrap().fields.insert("__jpegCustom".into(), json!(true));
            h.run_steps(2);
            assert!(!quality_enabled(&h), "{id} {format}: the field is greyed out");
            assert!(params(&mut h, dialog).get("quality").is_none_or(Value::is_null), "{id} {format}: no quality is sent");
            let bytes = exported(&mut h, dialog);
            if format == "webp" {
                // The default WebP stays lossless (a VP8L bitstream, not lossy VP8).
                assert!(bytes.windows(4).any(|w| w == b"VP8L"), "{id}: the WebP stays lossless");
            }
        }
        // Back to JPEG with Custom on, the field sends its quality again.
        let dialog = open(&mut h, id, &dir, "jpg");
        h.state_mut().ui.dialog_mut(dialog).unwrap().fields.insert("__jpegCustom".into(), json!(true));
        h.run_steps(2);
        assert!(quality_enabled(&h), "{id}");
        assert_eq!(params(&mut h, dialog)["quality"], json!(10), "{id}");
        crate::dialogs::cancel(h.state_mut(), dialog).unwrap();
    }
}

/// Alt+Reset puts the Custom checkbox and its quality back to how the dialog opened.
#[test]
fn alt_reset_restores_the_custom_quality() {
    let mut h = harness();
    let (_a, dir) = tmp();
    let dialog = open(&mut h, ARTBOARDS, &dir, "jpg");
    h.get_by_label("Custom").click();
    h.run_steps(2);
    h.state_mut().ui.dialog_mut(dialog).unwrap().fields.insert("quality".into(), json!(4));
    h.run_steps(2);
    let fields = &mut h.state_mut().ui.dialog_mut(dialog).unwrap().fields;
    crate::view_cmds::reset_form(fields);
    assert_eq!((&fields["__jpegCustom"], &fields["__jpegQuality"], &fields["quality"]), (&json!(false), &json!(10), &Value::Null));
}
