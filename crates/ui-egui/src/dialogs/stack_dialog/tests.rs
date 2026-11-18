use std::sync::Arc;

use craft::host::{Picked, ScriptedFiles};
use craft::ui::patterns::form_dialogs::HostedParamDialog;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::*;

/// `n` small PNGs named `names` in a fresh folder for `tag`: (folder, paths).
fn pngs(tag: &str, names: &[&str]) -> (String, Vec<String>) {
    let dir = std::env::temp_dir().join(format!("photocraft-load-stack-{tag}-{}", std::process::id()));
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

fn harness(answers: Vec<Picked>) -> Harness<'static, PhotocraftApp> {
    let files: Arc<dyn craft::host::Files> = Arc::new(ScriptedFiles::answering(answers));
    Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());

        PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), crate::Services { files: Some(files), ..Default::default() })
    })
}

fn fields<'a>(h: &'a Harness<'_, PhotocraftApp>, id: craft::ui::dialog::DialogId) -> &'a DialogFields {
    h.state().framework.dialogs.get_by_id::<HostedParamDialog>(id).expect("the dialog is open").fields()
}

fn set(h: &mut Harness<'_, PhotocraftApp>, id: craft::ui::dialog::DialogId, field: &str, value: serde_json::Value) {
    let ctx = h.ctx.clone();
    h.state_mut().framework.dialogs.set_json_field(&ctx, id, field, value).unwrap_or_else(|e| panic!("{field}: {e}"));
}

#[test]
fn statistics_browse_and_mode_selection_compute_the_chosen_statistic() {
    let (_, paths) = pngs("statistics-mean", &["first.png", "second.png"]);
    let mut source = photocraft_engine::Session::new();
    source.execute("file.new", json!({"width": 8, "height": 6, "background": "#80a0c0"})).unwrap();
    source.execute("file.saveACopy", json!({"path": paths[1]})).unwrap();
    let mut h = harness(vec![Picked::Paths(paths.iter().map(PathBuf::from).collect())]);
    let id = open_statistics(h.state_mut());
    h.run_steps(3);
    h.get_by_label("Browse…").click();
    h.run_steps(4);
    let listed: Vec<String> = fields(&h, id)
        .list("files")
        .expect("Browse filled the list")
        .iter()
        .filter_map(|v| if let FieldValue::Text(p) = v { Path::new(p).file_name().map(|n| n.to_string_lossy().into_owned()) } else { None })
        .collect();
    assert_eq!(listed, ["first.png", "second.png"]);
    set(&mut h, id, "mode", json!(photocraft_doc::StackMode::Mean.id()));
    h.run_steps(2);
    h.get_by_label("OK").click();
    h.run_steps(4);
    assert!(h.state().framework.dialogs.get_by_id::<HostedParamDialog>(id).is_none(), "OK closes the dialog");
    assert_eq!(h.state().session.documents().len(), 1);
    let doc = &h.state().session.active().unwrap().doc;
    assert_eq!(doc.layers.len(), 1);
    let photocraft_doc::LayerContent::Smart(smart) = &doc.layers[0].content else { panic!("expected a stack smart object") };
    assert_eq!(smart.stack_mode, Some(photocraft_doc::StackMode::Mean));
    let pixel = photocraft_compose::flatten(doc).px[0];
    for (actual, expected) in pixel.iter().zip([96.0, 128.0, 160.0, 255.0]) {
        assert!((actual * 255.0 - expected).abs() < 1.5, "{pixel:?}");
    }
}

#[test]
fn statistics_folder_and_alignment_reach_the_engine() {
    let (dir, paths) = pngs("statistics-folder", &["one.png", "two.png"]);
    // Auto-Align needs identifiable detail, not a uniform tiny colour swatch.
    let mut source = photocraft_engine::Session::new();
    source.execute("file.new", json!({"width": 96, "height": 72, "background": "#606060"})).unwrap();
    source
        .edit("texture", |doc, active| {
            let surface = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            for k in 0..40 {
                let (x, y, v) = ((k * 37) % 88, (k * 23) % 66, (k % 7) as f32 / 6.0);
                surface.fill_rect(photocraft_geom::Rect::new(x, y, x + 3 + k % 5, y + 2 + k % 4), &[v, 1.0 - v, 0.5, 1.0]);
            }
            Ok(())
        })
        .unwrap();
    for path in paths {
        source.execute("file.saveACopy", json!({"path": path})).unwrap();
    }
    let mut h = harness(vec![]);
    let id = open_statistics(h.state_mut());
    h.run_steps(3);
    set(&mut h, id, "files", json!({"files": [], "folder": dir, "useFolder": true}));
    h.run_steps(3);
    h.get_by_label("Attempt to Automatically Align Source Images").click();
    h.run_steps(2);
    assert_eq!(fields(&h, id).bool("align"), Some(true));
    let ctx = h.ctx.clone();
    crate::dialogs::confirm(h.state_mut(), &ctx, id).unwrap();
    let doc = &h.state().session.active().unwrap().doc;
    let photocraft_doc::LayerContent::Smart(smart) = &doc.layers[0].content else { panic!("expected a stack smart object") };
    assert_eq!(smart.stack_mode, Some(photocraft_doc::StackMode::Median));
}

/// Use: Folder loads the images of the folder Browse… chose, listed in the dialog; with Create
/// Smart Object they go into one smart object.
#[test]
fn a_folder_loads_its_images() {
    let (dir, _) = pngs("folder", &["one.png", "two.png"]);
    let mut h = harness(vec![Picked::Paths(vec![PathBuf::from(&dir)])]);
    let id = open(h.state_mut());
    h.run_steps(3);
    set(&mut h, id, "files", json!({"files": [], "folder": "", "useFolder": true}));
    h.run_steps(2);
    h.get_by_label("Browse…").click();
    h.run_steps(4);
    assert_eq!(fields(&h, id).text("files.folder"), Some(dir.as_str()));
    assert!(h.query_by_label("one.png").is_some() && h.query_by_label("two.png").is_some(), "the folder's images are listed");
    // Create Smart Object after Loading Layers: both images inside one smart object.
    h.get_by_label("Create Smart Object after Loading Layers").click();
    h.run_steps(2);
    let ctx = h.ctx.clone();
    crate::dialogs::confirm(h.state_mut(), &ctx, id).unwrap();
    let d = &h.state().session.active().unwrap().doc;
    assert_eq!(d.layers.len(), 1);
    assert!(matches!(d.layers[0].content, photocraft_doc::LayerContent::Smart(_)));
}
