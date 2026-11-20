//! #9642: Save As and `app.save` take `pdfDownsamplePpi` for a PDF's image resolution, record
//! it in the step and hand it to the PDF writer; unset, the image keeps every pixel. A value the
//! engine refuses writes nothing.

use super::tests::call;
use super::*;
use std::cell::RefCell;
use std::rc::Rc;

#[test]
fn saves_pass_pdf_downsample_ppi_to_the_writer() {
    let asked: Rc<RefCell<Vec<Option<f32>>>> = Rc::default();
    let written: Rc<RefCell<Vec<String>>> = Rc::default();
    let (a, w, w2) = (asked.clone(), written.clone(), written.clone());
    let services = crate::Services {
        export: Some(Box::new(move |_d: &photocraft_doc::Document, _p: &str, s: &crate::ExportSettings| {
            assert_eq!(s.io_options().pdf_downsample_ppi, s.pdf_downsample_ppi, "the writer gets what was chosen");
            a.borrow_mut().push(s.pdf_downsample_ppi);
            Ok((b"out".to_vec(), Vec::new()))
        })),
        write: Some(Box::new(move |path: &str, _b: &[u8]| {
            w.borrow_mut().push(path.to_string());
            Ok(())
        })),
        automation_write: Some(Box::new(move |path: &str, _b: &[u8]| {
            w2.borrow_mut().push(path.to_string());
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    for params in [json!({"path": "a.pdf"}), json!({"path": "b.pdf", "pdfDownsamplePpi": 150}), json!({"path": "c.pdf", "pdfDownsamplePpi": null})] {
        let r = call(&mut app, &ctx, "app.save", params.clone());
        assert_eq!(r["result"]["path"], params["path"], "{r}");
    }
    crate::menus::invoke(&mut app, &ctx, "file.saveAs", json!({"path": "d.pdf", "pdfDownsamplePpi": 72})).unwrap();
    assert_eq!(*asked.borrow(), [None, Some(150.0), None, Some(72.0)]);
    assert_eq!(*written.borrow(), ["a.pdf", "b.pdf", "c.pdf", "d.pdf"]);
    let (id, step) = app.session.journal.last().unwrap();
    assert_eq!((id.as_str(), &step["pdfDownsamplePpi"]), ("file.saveAs", &json!(72.0)), "the step replays the same PDF: {step}");
    let r = call(&mut app, &ctx, "app.save", json!({"path": "e.pdf", "pdfDownsamplePpi": 0}));
    assert!(r["error"].as_str().unwrap().contains("pdfDownsamplePpi"), "{r}");
    assert_eq!(written.borrow().len(), 4, "a bad pdfDownsamplePpi writes nothing");
}
