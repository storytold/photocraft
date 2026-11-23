//! #16588: files the OS drops on the main window outside the tab strip open again, as before the framework move: on
//! the Home screen they open, on the canvas they are placed, and on the tab strip they open once, at its slot.

use std::sync::Arc;

use egui::vec2;
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;

type H = Harness<'static, Box<PhotocraftApp>>;

/// A dropped file that exists only as a path, as the desktop gives it.
#[derive(Debug)]
struct DroppedPath(std::path::PathBuf);

impl egui::DroppedFile for DroppedPath {
    fn path(&self) -> &std::path::Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        Err("a desktop drop is read from its path".into())
    }
}

/// A real app window with `docs` documents, the pointer at a position the test sets.
fn harness(docs: usize) -> H {
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let services = crate::Services {
            import: Some(Box::new(|name: &str, bytes: &[u8], opts: &photocraft_io::ImportOptions| {
                photocraft_io::import_with_options(name, bytes, opts).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), services);
        for n in 1..=docs {
            app.run("file.new", json!({"width": 320, "height": 240, "name": format!("doc-{n}.png")})).unwrap();
        }
        app
    });
    h.run_steps(4);
    h
}

/// A PNG of a white `w`×`h` image written to a fresh folder, and the folder.
fn png_file(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("photocraft-window-drop-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30, "background": "white"})).unwrap();
    let bytes = photocraft_io::export(&s.active().unwrap().doc, "p.png", &Default::default()).unwrap().bytes;
    let file = dir.join("photo.png");
    std::fs::write(&file, bytes).unwrap();
    (dir, file)
}

/// Holds `file` over the window at `at` for a few passes, as the OS does while dragging, then drops it there.
fn drop_at(h: &mut H, file: &std::path::Path, at: egui::Pos2) {
    h.event(egui::Event::PointerMoved(at));
    for _ in 0..3 {
        h.input_mut().hovered_files = vec![egui::HoveredFile { path: Some(file.to_path_buf()), mime: "image/png".into() }];
        h.step();
    }
    h.input_mut().hovered_files.clear();
    h.input_mut().dropped_files.push(Arc::new(DroppedPath(file.to_path_buf())));
    h.run_steps(3);
}

fn doc_names(h: &H) -> Vec<String> {
    h.state().session.documents().iter().map(|d| d.doc.name.clone()).collect()
}

#[test]
fn a_file_dropped_on_the_home_screen_opens() {
    let (dir, file) = png_file("home");
    let mut h = harness(0);
    assert!(h.state_mut().home_screen_up(), "no document open: the Home screen shows");
    let middle = h.ctx.content_rect().center();
    drop_at(&mut h, &file, middle);
    assert_eq!(doc_names(&h), vec!["photo.png".to_string()], "the dropped file opens as a document");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_file_dropped_on_the_canvas_is_placed_in_the_document() {
    let (dir, file) = png_file("canvas");
    let mut h = harness(1);
    let canvas = h.state().drop_canvas_rect.expect("the canvas is shown");
    let layers = h.state().session.active().unwrap().doc.layers.len();
    drop_at(&mut h, &file, canvas.center());
    // Placement decodes on a worker: frame count does not imply completion.
    let t = std::time::Instant::now();
    while h.state().session.active().unwrap().doc.layers.len() == layers && t.elapsed() < std::time::Duration::from_secs(30) {
        h.step();
    }
    assert_eq!(doc_names(&h), vec!["doc-1.png".to_string()], "a canvas drop opens no document");
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), layers + 1, "the file is placed as a layer");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_file_dropped_on_a_tab_opens_once_at_its_slot() {
    let (dir, file) = png_file("tab");
    let mut h = harness(2);
    let tab = egui_kittest::kittest::Queryable::query_all_by_role(&h, egui::accesskit::Role::Tab)
        .find(|n| egui_kittest::kittest::NodeT::accesskit_node(n).label().is_some_and(|l| l.contains("doc-1.png")))
        .expect("the first document's tab shows")
        .rect();
    drop_at(&mut h, &file, tab.center());
    let names = doc_names(&h);
    assert_eq!(names.len(), 3, "the tab strip takes the drop alone, so the file opens once: {names:?}");
    // The window's own target would open it after every document; the strip opens it at the gap by the tab.
    assert_eq!(names.last().map(String::as_str), Some("doc-2.png"), "it opens at the strip's slot, not at the end: {names:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
