//! #11481: File › Export › Layers / Layer Comps to Files have a JPEG Quality row behind a Custom
//! check box, as Batch has (#3956). Artboards to Files has its quality in its Export Options
//! instead (#13636, `artboards_export_options_dialog_tests`). The quality reaches the command
//! only for a JPEG with Custom on: left alone, an export writes what the command writes with no
//! quality (the source JPEG's own quality, else the encoder default), a WebP stays lossless and
//! PNG has none. The forms are the hosted command forms; the tests drive them as automation does.
use crate::PhotocraftApp;
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::HostedParamDialog;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const COMPS: &str = "file.export.layerCompsToFiles";
const EXPORTS: [&str; 2] = [COMPS, "file.export.layersToFiles"];

fn app() -> (Box<PhotocraftApp>, egui::Context) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    // A noisy layer inside the board, so the JPEG quality shows in the file.
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#8A42B3"})).unwrap();
    app.run("filter.noise.addNoise", json!({"amount": 60})).unwrap();
    app.run("layer.new.artboardFromLayers", json!({"name": "Board"})).unwrap();
    app.run("layerComp.new", json!({"name": "Comp"})).unwrap();
    app.sync_views();
    let ctx = app.ctx().clone();
    (app, ctx)
}

fn open(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, dir: &Path, format: &str) -> DialogId {
    let reply = crate::menus::invoke(app, ctx, id, json!({})).unwrap();
    let want = reply["dialog"].as_str().unwrap().to_owned();
    // PLAN: assumed `app.framework.dialogs: DialogHost` and `OpenDialog { id, .. }`
    let dialog = app.framework.dialogs.open_dialogs().into_iter().map(|d| d.id).find(|d| d.value().to_string() == want).unwrap();
    set(app, ctx, dialog, "dir", json!(dir));
    set(app, ctx, dialog, "prefix", json!("out"));
    set(app, ctx, dialog, "format", json!(format));
    dialog
}

fn set(app: &mut PhotocraftApp, ctx: &egui::Context, dialog: DialogId, key: &str, value: Value) {
    app.framework.dialogs.set_json_field(ctx, dialog, key, value).unwrap_or_else(|e| panic!("{key}: {e}"));
}

fn fields(app: &PhotocraftApp, dialog: DialogId) -> Value {
    serde_json::to_value(app.framework.dialogs.get_by_id::<HostedParamDialog>(dialog).unwrap().fields()).unwrap()
}

/// The first file an export wrote under `dir`.
fn written(dir: &Path) -> Vec<u8> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, out);
            } else {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    walk(dir, &mut files);
    files.sort();
    std::fs::read(files.first().expect("the export wrote a file")).unwrap()
}

fn exported(app: &mut PhotocraftApp, ctx: &egui::Context, dialog: DialogId, dir: &Path) -> Vec<u8> {
    crate::dialogs::confirm(app, ctx, dialog).unwrap();
    written(dir)
}

/// The same export run straight through the command, into `dir`.
fn direct(app: &mut PhotocraftApp, dir: &Path, format: &str, quality: Option<i64>) -> Vec<u8> {
    let mut p = json!({"dir": dir, "prefix": "out", "format": format});
    if let Some(q) = quality {
        p["quality"] = json!(q);
    }
    app.run(COMPS, p).unwrap();
    written(dir)
}

fn tmp() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

#[test]
fn layer_comps_to_files_sends_a_custom_jpeg_quality() {
    let (mut app, ctx) = app();
    let via_dialog = tmp();
    let via_command = tmp();
    // Custom off: the quality row is greyed out for any format but JPEG, and an untouched JPEG export writes what
    // the command writes with no quality.
    let dialog = open(&mut app, &ctx, COMPS, via_dialog.path(), "png");
    assert!(app.framework.dialogs.set_json_field(&ctx, dialog, "quality", json!(5)).is_err(), "PNG: the quality row takes no input");
    set(&mut app, &ctx, dialog, "format", json!("jpg"));
    assert!(fields(&app, dialog).get("quality").is_none_or(Value::is_null), "no quality is sent while Custom is off");
    let bytes = exported(&mut app, &ctx, dialog, via_dialog.path());
    assert!(bytes == direct(&mut app, via_command.path(), "jpg", None), "the untouched dialog keeps the command's own JPEG quality");

    // Custom on: the typed quality reaches the command.
    let mut sizes = Vec::new();
    for quality in [0, 3, 12] {
        let (a, b) = (tmp(), tmp());
        let dialog = open(&mut app, &ctx, COMPS, a.path(), "jpg");
        set(&mut app, &ctx, dialog, "quality", json!(quality));
        assert_eq!(fields(&app, dialog)["quality"].as_f64(), Some(quality as f64), "the typed quality is the field's value");
        let bytes = exported(&mut app, &ctx, dialog, a.path());
        assert!(bytes == direct(&mut app, b.path(), "jpg", Some(quality)), "quality {quality}: the file the command writes at that quality");
        sizes.push(bytes.len());
    }
    assert!(sizes[0] < sizes[1] && sizes[1] < sizes[2], "a higher quality writes a bigger JPEG: {sizes:?}");
}

#[test]
fn export_to_files_quality_leaves_png_and_webp_as_they_were() {
    let (mut app, ctx) = app();
    for id in EXPORTS {
        // A quality typed for JPEG, then another format: nothing is sent, so a WebP stays lossless (a VP8L
        // bitstream, not lossy VP8).
        let dir = tmp();
        let dialog = open(&mut app, &ctx, id, dir.path(), "jpg");
        set(&mut app, &ctx, dialog, "quality", json!(4));
        set(&mut app, &ctx, dialog, "format", json!("webp"));
        let bytes = exported(&mut app, &ctx, dialog, dir.path());
        assert!(bytes.windows(4).any(|w| w == b"VP8L"), "{id}: the WebP stays lossless");

        let dir = tmp();
        let dialog = open(&mut app, &ctx, id, dir.path(), "png");
        assert!(app.framework.dialogs.set_json_field(&ctx, dialog, "quality", json!(4)).is_err(), "{id}: PNG takes no quality");
        crate::dialogs::cancel(&mut app, &ctx, dialog).unwrap();

        // Back to JPEG, the quality typed before is still there.
        let dialog = open(&mut app, &ctx, id, dir.path(), "jpg");
        set(&mut app, &ctx, dialog, "quality", json!(4));
        for format in ["png", "jpg"] {
            set(&mut app, &ctx, dialog, "format", json!(format));
        }
        assert_eq!(fields(&app, dialog)["quality"].as_f64(), Some(4.0), "{id}: the round trip keeps the quality");
        crate::dialogs::cancel(&mut app, &ctx, dialog).unwrap();
    }
}
