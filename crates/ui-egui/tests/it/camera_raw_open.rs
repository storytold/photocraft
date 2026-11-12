//! The open-time Camera Raw dialog in the real shell: raws waiting in it are not documents (no
//! tab, no canvas) until Open, and Cancel leaves the session as it was (#4180).
use egui::vec2;
use egui_kittest::{Harness, kittest::Queryable};
use photocraft_ui_egui::{PhotocraftApp, Services, theme::ThemeKind};
use serde_json::json;

fn dng() -> Vec<u8> {
    use photocraft_raw::testgen::{DngSpec, mosaic, scene};
    let (w, h) = (48, 32);
    let mut spec = DngSpec::cfa(w, h, mosaic(&scene(w, h), w, [0, 1, 1, 2], 0, 30000));
    spec.as_shot_neutral = Some([0.5, 1.0, 0.7]);
    spec.build()
}

/// The app with one document open ("Untitled"), then two raws opened interactively.
fn fixture() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_step_dt(1.0 / 60.0).with_size(vec2(1400.0, 900.0)).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, ThemeKind::Pro);
        let services = Services {
            import: Some(Box::new(|name: &str, b: &[u8], opts: &photocraft_io::ImportOptions| {
                photocraft_io::import_with_options(name, b, opts).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        let dng = dng();
        app.open_bytes("first.dng", &dng).unwrap();
        app.open_bytes("second.dng", &dng).unwrap();
        app
    });
    h.run_steps(4);
    h
}

fn dialog_up(h: &Harness<'_, PhotocraftApp>) -> bool {
    !photocraft_ui_egui::control::inspect(h.state(), &h.ctx)["cameraRaw"].is_null()
}

/// Document tabs of `name` ("name @ 100% (RGB/8)"; a filmstrip tile is the bare name).
fn tabs(h: &Harness<'_, PhotocraftApp>, name: &str) -> usize {
    h.query_all_by_label_contains(&format!("{name} @ ")).count()
}

fn names(h: &Harness<'_, PhotocraftApp>) -> Vec<String> {
    h.state().session.documents().iter().map(|st| st.doc.name.clone()).collect()
}

#[test]
fn raws_in_the_dialog_have_no_tab_until_open() {
    let mut h = fixture();
    assert!(dialog_up(&h), "the dialog is up");
    assert_eq!(names(&h), ["Untitled"], "the raws are not documents");
    assert_eq!(h.state().session.active().unwrap().history.past_len(), 0);
    // Each name shows on its filmstrip tile, not on a tab.
    assert_eq!(h.query_all_by_label("first.dng").count(), 1);
    assert_eq!(h.query_all_by_label("second.dng").count(), 1);
    assert_eq!((tabs(&h, "Untitled"), tabs(&h, "first.dng"), tabs(&h, "second.dng")), (1, 0, 0), "one tab, the document's");
    // Open opens the selected raws: ⌘A selects both.
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(2);
    h.get_by_label("Open").click();
    h.run_steps(4);
    assert!(!dialog_up(&h));
    assert_eq!(names(&h), ["Untitled", "first.dng", "second.dng"], "Open makes exactly these documents");
    assert!(h.state().session.documents().iter().all(|st| st.saved_revision == st.revision));
    assert_eq!((tabs(&h, "first.dng"), tabs(&h, "second.dng")), (1, 1), "a tab each now");
}

#[test]
fn cancel_leaves_the_session_as_it_was() {
    let mut h = fixture();
    let before = h.state().session.documents().iter().map(|st| (st.doc.id, st.revision)).collect::<Vec<_>>();
    h.get_by_label("Cancel").click();
    h.run_steps(4);
    assert!(!dialog_up(&h));
    let after = h.state().session.documents().iter().map(|st| (st.doc.id, st.revision)).collect::<Vec<_>>();
    assert_eq!(after, before);
    assert_eq!(h.state().session.active_index(), Some(0));
    assert_eq!(tabs(&h, "first.dng"), 0, "no tab");
}
