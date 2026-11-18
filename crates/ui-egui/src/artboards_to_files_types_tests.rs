//! #13635: File › Export › Artboards to Files has the reference application's File Type list:
//! BMP, JPEG, PDF, PSD, Targa, TIFF, PNG-8, PNG-24 in that order, PSD chosen when it opens.
//! Layer Comps to Files keeps its own list.
use crate::PhotocraftApp;
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::HostedParamDialog;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

const ARTBOARDS: &str = "file.export.artboardsToFiles";
const NAMES: [&str; 8] = ["BMP", "JPEG", "PDF", "PSD", "Targa", "TIFF", "PNG-8", "PNG-24"];

type H = Harness<'static, Box<PhotocraftApp>>;

fn harness() -> H {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 200, "background": "transparent"})).unwrap();
    app.run("layer.new.artboard", json!({"rect": [0, 0, 100, 50], "name": "Red Board", "background": "custom", "color": "#ff0000"})).unwrap();
    app.run("layer.new.artboard", json!({"rect": [200, 0, 80, 80], "name": "Blue Board", "background": "custom", "color": "#0000ff"})).unwrap();
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

fn field(h: &H, dialog: DialogId, key: &str) -> Option<Value> {
    let form = h.state().framework.dialogs.get_by_id::<HostedParamDialog>(dialog)?;
    serde_json::to_value(form.fields()).ok()?.get(key).cloned()
}

fn set(h: &mut H, dialog: DialogId, key: &str, value: Value) {
    let ctx = h.ctx.clone();
    h.state_mut().framework.dialogs.set_json_field(&ctx, dialog, key, value).unwrap_or_else(|e| panic!("{key}: {e}"));
    h.run_steps(2);
}

#[test]
fn file_type_list_is_the_reference_list_with_psd_chosen() {
    let mut h = harness();
    let dialog = open(&mut h, ARTBOARDS);
    assert_eq!(field(&h, dialog, "format"), Some(json!("psd")), "PSD on first open");
    for id in ["bmp", "jpg", "pdf", "psd", "tga", "tiff", "png8", "png24"] {
        set(&mut h, dialog, "format", json!(id));
    }
    let ctx = h.ctx.clone();
    for gone in ["png", "webp"] {
        assert!(h.state_mut().framework.dialogs.set_json_field(&ctx, dialog, "format", json!(gone)).is_err(), "{gone} is not in the list");
    }
    set(&mut h, dialog, "format", json!("psd"));
    // The row reads "File Type:" and the closed dropdown shows PSD.
    let combo = h
        .query_all_by_role(Role::ComboBox)
        .find(|n| n.accesskit_node().label().is_some_and(|l| l.trim_end_matches(':') == "File Type"))
        .expect("a File Type dropdown");
    assert_eq!(combo.value().as_deref(), Some("PSD"));
    combo.click();
    h.run_steps(3);
    // Opened, the list shows the eight names top to bottom in the reference order.
    let ys: Vec<f32> = NAMES
        .iter()
        .map(|name| {
            let tops: Vec<f32> = h.query_all_by_label(name).map(|n| n.rect().min.y).collect();
            assert!(!tops.is_empty(), "the list shows {name}");
            tops.into_iter().fold(f32::MIN, f32::max)
        })
        .collect();
    assert!(ys.windows(2).all(|w| w[0] < w[1]), "{NAMES:?} at {ys:?}");
}

#[test]
fn ok_with_png8_writes_indexed_pngs_and_pdf_one_file() {
    let mut h = harness();
    for (format, files) in [("png8", 2), ("pdf", 1), ("psd", 2)] {
        let t = tempfile::tempdir().unwrap();
        let dialog = open(&mut h, ARTBOARDS);
        set(&mut h, dialog, "dir", json!(t.path()));
        set(&mut h, dialog, "prefix", json!(format));
        set(&mut h, dialog, "format", json!(format));
        let ctx = h.ctx.clone();
        crate::dialogs::confirm(h.state_mut(), &ctx, dialog).unwrap();
        let mut written: Vec<std::path::PathBuf> = std::fs::read_dir(t.path()).unwrap().map(|e| e.unwrap().path()).collect();
        written.sort();
        assert_eq!(written.len(), files, "{format}: {written:?}");
        for f in &written {
            let bytes = std::fs::read(f).unwrap();
            let name = f.to_string_lossy();
            match format {
                "png8" => assert_eq!(bytes[25], 3, "{name} is an indexed PNG"),
                "pdf" => assert!(name.ends_with("pdf.pdf") && bytes.starts_with(b"%PDF-"), "{name}"),
                _ => assert!(name.ends_with(".psd") && bytes.starts_with(b"8BPS"), "{name}"),
            }
        }
    }
}

#[test]
fn layer_comps_to_files_keeps_its_list() {
    let mut h = harness();
    let dialog = open(&mut h, "file.export.layerCompsToFiles");
    assert_eq!(field(&h, dialog, "format"), Some(json!("png")));
    for id in ["png", "jpg", "psd", "tiff", "webp", "bmp"] {
        set(&mut h, dialog, "format", json!(id));
    }
}
