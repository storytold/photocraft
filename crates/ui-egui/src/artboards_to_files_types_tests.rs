//! #13635: File › Export › Artboards to Files has the reference application's File Type list:
//! BMP, JPEG, PDF, PSD, Targa, TIFF, PNG-8, PNG-24 in that order, PSD chosen when it opens.
//! Layer Comps to Files keeps its own list.
use crate::PhotocraftApp;
use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

const ARTBOARDS: &str = "file.export.artboardsToFiles";
const NAMES: [&str; 8] = ["BMP", "JPEG", "PDF", "PSD", "Targa", "TIFF", "PNG-8", "PNG-24"];

fn harness() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(photocraft_engine::Session::new(), Default::default())
    });
    let app = h.state_mut();
    app.run("file.new", json!({"width": 400, "height": 200, "background": "transparent"})).unwrap();
    app.run("layer.new.artboard", json!({"rect": [0, 0, 100, 50], "name": "Red Board", "background": "custom", "color": "#ff0000"})).unwrap();
    app.run("layer.new.artboard", json!({"rect": [200, 0, 80, 80], "name": "Blue Board", "background": "custom", "color": "#0000ff"})).unwrap();
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

#[test]
fn file_type_list_is_the_reference_list_with_psd_chosen() {
    let mut h = harness();
    let dialog = open(&mut h, ARTBOARDS);
    let fields = h.state_mut().ui.dialog_mut(dialog).unwrap().fields.clone();
    assert_eq!(fields["format"], json!("psd"), "PSD on first open");
    assert_eq!(fields["__choices"]["format"], json!(["bmp", "jpg", "pdf", "psd", "tga", "tiff", "png8", "png24"]));
    // The row reads "File Type:" and the closed dropdown shows PSD.
    let combo = h.query_all_by_role(Role::ComboBox).find(|n| n.accesskit_node().label().as_deref() == Some("File Type:")).expect("a File Type: dropdown");
    assert_eq!(combo.accesskit_node().value().as_deref(), Some("PSD"));
    let at = combo.rect().center();
    h.hover_at(at);
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
        h.run_steps(2);
    }
    h.run_steps(2);
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
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().to_string_lossy().into_owned();
    for (format, files) in [("png8", 2), ("pdf", 1), ("psd", 2)] {
        let dialog = open(&mut h, ARTBOARDS);
        let fields = &mut h.state_mut().ui.dialog_mut(dialog).unwrap().fields;
        fields.insert("dir".into(), json!(dir));
        fields.insert("prefix".into(), json!(format));
        fields.insert("format".into(), json!(format));
        h.run_steps(2);
        let r = crate::dialogs::confirm(h.state_mut(), dialog).unwrap();
        let written: Vec<String> = r["files"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
        assert_eq!(written.len(), files, "{format}: {written:?}");
        for f in &written {
            let bytes = std::fs::read(f).unwrap();
            match format {
                "png8" => assert_eq!(bytes[25], 3, "{f} is an indexed PNG"),
                "pdf" => assert!(f.ends_with("pdf.pdf") && bytes.starts_with(b"%PDF-"), "{f}"),
                _ => assert!(f.ends_with(".psd") && bytes.starts_with(b"8BPS"), "{f}"),
            }
        }
    }
}

#[test]
fn layer_comps_to_files_keeps_its_list() {
    let mut h = harness();
    let dialog = open(&mut h, "file.export.layerCompsToFiles");
    let fields = h.state_mut().ui.dialog_mut(dialog).unwrap().fields.clone();
    assert_eq!(fields["format"], json!("png"));
    assert_eq!(fields["__choices"]["format"], json!(["png", "jpg", "psd", "tiff", "webp", "bmp"]));
}
