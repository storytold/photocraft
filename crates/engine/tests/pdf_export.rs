use photocraft_doc::{Color, ColorMode, Document, SampleType, Size};
use photocraft_engine::{Session, pdf_export_cmds::Snapshot};
use serde_json::json;

fn session() -> Session {
    let mut s = Session::new();
    for (i, depth) in [SampleType::U8, SampleType::U16, SampleType::F32].into_iter().enumerate() {
        let mut d = Document::with_background(format!("Page {i}"), Size::new(30 + i as u32 * 10, 20), ColorMode::Rgb, depth, Color::WHITE);
        d.resolution_dpi = 72.0;
        s.add_document(d, None);
    }
    s.edit("Current edit", |doc, _| {
        doc.layers.clear();
        Ok(())
    })
    .unwrap();
    assert!(s.active().unwrap().is_dirty());
    s
}

#[test]
fn all_tabs_export_keeps_order_sizes_and_closes_only_after_write() {
    let mut s = session();
    let before = s.documents().to_vec();
    let mut output = Vec::new();
    let result = Snapshot::capture(&s)
        .unwrap()
        .write(&mut s, "combined.pdf", false, |_, bytes| {
            output = bytes.to_vec();
            Ok(())
        })
        .unwrap();
    assert_eq!(result["tabs"], 3);
    assert_eq!(result["closed"], 0);
    assert_eq!(photocraft_io::pdf::page_sizes(&output).unwrap(), [(30.0, 20.0), (40.0, 20.0), (50.0, 20.0)]);
    for (a, b) in s.documents().iter().zip(&before) {
        assert_eq!(a.doc, b.doc);
        assert_eq!(a.revision, b.revision);
        assert_eq!(a.saved_revision, b.saved_revision);
    }
    Snapshot::capture(&s)
        .unwrap()
        .write(&mut s, "combined.pdf", true, |_, bytes| {
            assert!(bytes.ends_with(b"%%EOF\n"));
            Ok(())
        })
        .unwrap();
    assert!(s.documents().is_empty());
}

#[test]
fn failed_export_retains_every_document_and_dirty_state() {
    let mut s = session();
    let before = s.documents().to_vec();
    let err = Snapshot::capture(&s).unwrap().write(&mut s, "combined.pdf", true, |_, _| Err("disk full".into())).unwrap_err();
    assert!(err.to_string().contains("disk full"));
    assert_eq!(s.documents().len(), 3);
    for (a, b) in s.documents().iter().zip(&before) {
        assert_eq!(a.doc, b.doc);
        assert_eq!(a.is_dirty(), b.is_dirty());
    }
    assert!(Snapshot::capture(&s).unwrap().write(&mut s, "wrong.png", true, |_, _| panic!("must not write")).is_err());
    assert_eq!(s.documents().len(), 3);
}

#[test]
fn new_or_changed_tabs_are_never_closed_by_an_older_snapshot() {
    let mut s = session();
    let snap = Snapshot::capture(&s).unwrap();
    s.active_mut().unwrap().revision += 1;
    s.add_document(Document::new("new", Size::new(10, 10), ColorMode::Rgb, SampleType::U8), None);
    let result = snap.write(&mut s, "combined.pdf", true, |_, _| Ok(())).unwrap();
    assert_eq!(result["closed"], 2);
    assert_eq!(s.documents().len(), 2);
    assert_eq!(s.documents()[1].doc.name, "new");
}

#[test]
fn command_rejects_bad_params_without_losing_tabs() {
    let mut s = session();
    for p in [json!({}), json!({"path":false}), json!({"path":"x.pdf","closeAfter":"yes"}), json!({"path":"x.png"})] {
        assert!(s.execute("file.export.allTabsPdf", p).is_err());
        assert_eq!(s.documents().len(), 3);
    }
    assert!(Snapshot::capture(&Session::new()).is_err());
}

#[test]
fn cropped_pdf_page_artboard_and_binder_use_the_cropped_sheet() {
    use photocraft_io::{ExportOptions, export, pdf};
    for delete in [false, true] {
        for angle in [0.0, 17.0] {
            let source = Document::with_background("source", Size::new(100, 80), ColorMode::Rgb, SampleType::U8, Color::WHITE);
            let bytes = export(&source, "pdf", &ExportOptions::default()).unwrap().bytes;
            let imported = photocraft_io::import("source.pdf", &bytes).unwrap().document;
            let page = photocraft_engine::jobs::pdf_page_documents(&imported).unwrap().remove(0);
            let mut s = Session::new();
            s.add_document(page, None);
            let original = s.active().unwrap().doc.clone();
            s.execute("image.crop", json!({"x": 30, "y": 20, "width": 60, "height": 40, "angle": angle, "deleteCroppedPixels": delete})).unwrap();
            let cropped = &s.active().unwrap().doc;
            assert_eq!(cropped.size, Size::new(60, 40));
            assert_eq!(cropped.artboards()[0].2.rect, cropped.bounds(), "the visible page boundary must follow Crop");
            let single = export(cropped, "pdf", &ExportOptions::default()).unwrap();
            assert_eq!(pdf::page_sizes(&single.bytes).unwrap(), [(30.0, 20.0)], "144 ppi pixels must export at the cropped physical size");
            let mut binder = Vec::new();
            Snapshot::capture(&s)
                .unwrap()
                .write(&mut s, "binder.pdf", false, |_, bytes| {
                    binder = bytes.to_vec();
                    Ok(())
                })
                .unwrap();
            assert_eq!(pdf::page_sizes(&binder).unwrap(), [(30.0, 20.0)]);
            assert!(s.undo());
            assert_eq!(s.active().unwrap().doc, original, "Undo must restore both the image and the page boundary");
        }
    }
}
