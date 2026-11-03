//! A failed renderer-aware save must never replace the previous file, and closing a document
//! must release its renderer caches.

use std::fs;
use std::sync::{Arc, Mutex};

use photocraft_automation::{Headless, files};
use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::{DocId, Document, Rect, Size};
use photocraft_io::{BandRenderer, ExportOptions, IoError, RenderBand, RenderError};

struct FailingRenderer;

impl BandRenderer for FailingRenderer {
    fn render_bands(
        &mut self,
        _doc: &Document,
        _rect: Rect,
        _band_rows: i32,
        _sink: &mut dyn FnMut(RenderBand) -> Result<(), IoError>,
    ) -> Result<(), RenderError> {
        Err(RenderError::Execution("forced renderer failure".into()))
    }
}

#[test]
fn failed_renderer_save_preserves_prior_file() {
    let dir = std::env::temp_dir().join(format!("photocraft-renderer-save-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let target = dir.join("existing.png");
    let original = b"prior file contents that must survive a failed save";
    fs::write(&target, original).unwrap();

    let mut doc = Document::with_background("test", Size::new(32, 24), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let mut top = doc.layers[0].clone();
    top.name = "top".into();
    doc.layers.push(top);

    let mut renderer = FailingRenderer;
    let res = files::save_with_renderer(&doc, &target, Some("png"), &ExportOptions::default(), None, Some(&mut renderer));
    assert!(res.is_err(), "the renderer failure must fail the save: {res:?}");
    assert_eq!(fs::read(&target).unwrap(), original, "a failed save must leave the previous file intact");
    let _ = fs::remove_dir_all(&dir);
}

struct ForgetRecordingRenderer {
    forgotten: Arc<Mutex<Vec<DocId>>>,
}

impl BandRenderer for ForgetRecordingRenderer {
    fn render_bands(
        &mut self,
        _doc: &Document,
        _rect: Rect,
        _band_rows: i32,
        _sink: &mut dyn FnMut(RenderBand) -> Result<(), IoError>,
    ) -> Result<(), RenderError> {
        Ok(())
    }

    fn forget_document(&mut self, doc: DocId) {
        self.forgotten.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(doc);
    }
}

#[test]
fn closing_a_document_forgets_its_renderer_caches() {
    let forgotten = Arc::new(Mutex::new(Vec::new()));
    let mut h = Headless::trusted_local();
    h.session.open_document(Document::with_background("close", Size::new(16, 16), ColorMode::Rgb, SampleType::U8, Color::WHITE), None);
    let id = h.session.documents()[0].doc.id;
    h.set_renderer(Some(Box::new(ForgetRecordingRenderer { forgotten: forgotten.clone() })));

    h.close(Some(0)).expect("close document");

    let forgotten = forgotten.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
    assert_eq!(forgotten, vec![id], "closing a document must release its renderer caches");
}
