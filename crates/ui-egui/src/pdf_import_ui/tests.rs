use super::*;
use egui_kittest::{
    Harness,
    kittest::Queryable,
};
use std::time::{Duration, Instant};

const PDF: &[u8] = include_bytes!("../../../io/tests/fixtures/pdf-three-pages.pdf");

fn harness() -> Harness<'static, PhotocraftApp> {
    harness_at_size(vec2(1280.0, 800.0))
}

fn harness_at_size(size: egui::Vec2) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(size).with_max_steps(32).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.background_jobs = true;
        app.open_file("colors.pdf", PDF).unwrap();
        app
    });
    wait(&mut h, |a| a.pdf_import.first().is_some_and(|st| !st.loading));
    let id = h.state().pdf_import[0].dialog;
    let f = &mut h.state_mut().ui.dialog_mut(id).unwrap().fields;
    f.insert("resolution".into(), json!(72));
    f.insert("crop".into(), json!("mediaBox"));
    h.run_steps(4);
    h
}

#[test]
fn pdf_dialog_modes_and_confirmation_fit_a_short_window() {
    let h = harness_at_size(vec2(960.0, 485.0));
    let screen = h.ctx.content_rect();
    for label in ["Separate tabs", "Layers in one document", "OK", "Cancel"] {
        let r = h.get_by_label(label).rect();
        assert!(screen.contains_rect(r), "{label}: {r:?} outside {screen:?}");
    }
}

fn wait(h: &mut Harness<'_, PhotocraftApp>, ready: impl Fn(&PhotocraftApp) -> bool) {
    let until = Instant::now() + Duration::from_secs(30);
    while !ready(h.state()) {
        assert!(Instant::now() < until, "PDF import timed out: {}", h.state().ui.status);
        h.step();
        std::thread::sleep(Duration::from_millis(1));
    }
    h.run_steps(2);
}

fn fields(h: &Harness<'_, PhotocraftApp>) -> Map<String, Value> {
    h.state().ui.dialogs.iter().find(|d| owns(&d.fields)).unwrap().fields.clone()
}

fn finish(h: &mut Harness<'_, PhotocraftApp>) {
    h.get_by_label("OK").click();
    h.run_steps(2);
    wait(h, |a| a.jobs.opens.is_empty());
    assert!(h.state().ui.dialogs.iter().all(|d| !owns(&d.fields)), "PDF import dialog still open; status: {}; error: {}; fields: {:?}", h.state().ui.status, h.state().ui.status_error, h.state().ui.dialogs.iter().filter(|d| owns(&d.fields)).map(|d| d.fields.clone()).collect::<Vec<_>>());
    assert!(!h.state().ui.status_error, "{}", h.state().ui.status);
}

#[test]
fn pdf_dialog_two_modes_keep_selection_and_all_pages_become_layers() {
    let mut h = harness();
    assert_eq!(fields(&h)["pagesAsLayers"], false);
    h.get_by_label("Select All").click();
    h.run_steps(2);
    assert_eq!(picks(&fields(&h), "pages"), [1, 2, 3]);
    h.get_by_label("Layers in one document").click();
    h.run_steps(2);
    assert_eq!(fields(&h)["pagesAsLayers"], true);
    h.get_by_label("Separate tabs").click();
    h.run_steps(2);
    assert_eq!(fields(&h)["pagesAsLayers"], false);
    assert_eq!(picks(&fields(&h), "pages"), [1, 2, 3]);
    h.get_by_label("Layers in one document").click();
    h.run_steps(2);
    h.get_by_label("Images").click();
    h.run_steps(2);
    assert!(h.query_by_label("Layers in one document").is_none());
    assert!(h.query_by_label("One selected image per tab.").is_some());
    let pages_hit = h.get_by_label("Pages").rect();
    h.get_by_label("Pages").click();
    h.run_steps(2);
    assert_eq!(fields(&h)["select"], "pages", "Pages click at {pages_hit:?} did not change the selection");
    assert_eq!(fields(&h)["pagesAsLayers"], true);
    assert_eq!(picks(&fields(&h), "pages"), [1, 2, 3]);
    finish(&mut h);
    assert_eq!(h.state().session.documents().len(), 1);
    let doc = &h.state().session.active().unwrap().doc;
    assert_eq!(doc.layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["Page 1", "Page 2", "Page 3"]);
    for (i, layer) in doc.layers.iter().enumerate() {
        assert_eq!(layer.surface().unwrap().pixel(48, 30)[i], 1.0);
    }
}

#[test]
fn pdf_dialog_separate_tabs_keep_page_order_and_reverse_order() {
    for (reverse, expected) in [(false, vec!["colors-1.pdf", "colors-2.pdf", "colors-3.pdf"]), (true, vec!["colors-3.pdf", "colors-2.pdf", "colors-1.pdf"])] {
        let mut h = harness();
        h.get_by_label("Select All").click();
        h.run_steps(2);
        let id = h.state().pdf_import[0].dialog;
        h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("reverseOrder".into(), json!(reverse));
        finish(&mut h);
        let docs = h.state().session.documents();
        assert_eq!(docs.iter().map(|d| d.doc.name.as_str()).collect::<Vec<_>>(), expected);
        assert!(docs.iter().all(|d| d.doc.layers.len() == 1));
    }
}

#[test]
fn pdf_dialog_subset_and_cancel_preserve_the_requested_page_set() {
    for as_layers in [true, false] {
        let mut h = harness();
        let id = h.state().pdf_import[0].dialog;
        let f = &mut h.state_mut().ui.dialog_mut(id).unwrap().fields;
        f.insert("pages".into(), json!([1, 3]));
        f.insert("pagesAsLayers".into(), json!(as_layers));
        f.insert("reverseOrder".into(), json!(false));
        finish(&mut h);
        let docs = h.state().session.documents();
        if as_layers {
            assert_eq!(docs.len(), 1);
            assert_eq!(docs[0].doc.layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["Page 1", "Page 3"]);
        } else {
            assert_eq!(docs.iter().map(|d| d.doc.name.as_str()).collect::<Vec<_>>(), ["colors-1.pdf", "colors-3.pdf"]);
        }
    }
    let mut h = harness();
    h.get_by_label("Select All").click();
    h.run_steps(2);
    h.get_by_label("None").click();
    h.run_steps(2);
    assert!(picks(&fields(&h), "pages").is_empty());
    h.get_by_label("Cancel").click();
    h.run_steps(2);
    assert!(h.state().session.documents().is_empty());
    assert!(h.state().pdf_import.is_empty());
    assert!(h.state().jobs.opens.is_empty());
}


#[test]
fn pdf_dialog_artboard_mode_recognizes_pages_without_confusing_them_with_layers() {
    let mut h = harness();
    h.get_by_label("Select All").click();
    h.run_steps(2);
    h.get_by_label("PDF pages as artboards (editable)").click();
    h.run_steps(2);
    assert_eq!(fields(&h)["pagesAsArtboards"], true);
    assert_eq!(fields(&h)["pagesAsLayers"], false);
    assert_eq!(fields(&h)["crop"], "mediaBox", "editable PDF pages must default to the complete Media Box, not a content bounding box");
    finish(&mut h);
    let doc = &h.state().session.active().unwrap().doc;
    assert_eq!(doc.artboards().len(), 3);
    for (id, name, page) in doc.artboards() {
        assert!(name.starts_with("PDF Page "));
        assert!(page.preset.starts_with("photocraft.pdf.page.v1:"));
        let children = doc.layer(id).unwrap().children().unwrap();
        assert_eq!(children.len(), 1, "a PDF source is a child of the page, not the page itself");
    }
}
