//! #14806: Layer › Export As and Quick Export as PNG with several layers skip empty, adjustment
//! and off-canvas layers instead of failing. The dialog opens on the first layer with pixels;
//! with nothing left to export they say why.

use super::*;
use craft::host::{Picked, ScriptedFiles};
use photocraft_geom::Rect;
use std::{cell::RefCell, rc::Rc};

type Written = Rc<RefCell<Vec<String>>>;

struct Layers {
    empty: LayerId,
    adjustment: LayerId,
    gone: LayerId,
    red: LayerId,
    blue: LayerId,
}

/// 100 x 80 with, bottom to top: Background, `Empty` (no pixels), an Invert adjustment layer,
/// `Gone` (pixels only past the right edge), `Red` and `Blue` (filled). Its file dialogs stay open
/// until the test answers them.
fn app() -> (Box<PhotocraftApp>, Layers, Written, Arc<ScriptedFiles>) {
    let written = Written::default();
    let capture = written.clone();
    let files = Arc::new(ScriptedFiles::held());
    let services = crate::Services {
        files: Some(files.clone()),
        export: Some(Box::new(|doc, path, settings| {
            photocraft_io::export(doc, path, &settings.io_options()).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string())
        })),
        write: Some(Box::new(move |path, _| {
            capture.borrow_mut().push(path.to_string());
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    let active = |app: &PhotocraftApp| app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.new.layer", json!({"name": "Empty"})).unwrap();
    let empty = active(&app);
    app.run("layer.newAdjustmentLayer.invert", json!({})).unwrap();
    let adjustment = active(&app);
    app.run("layer.new.layer", json!({"name": "Gone"})).unwrap();
    let gone = active(&app);
    {
        let st = app.session.active_mut().unwrap();
        let doc = Arc::make_mut(&mut st.doc);
        let surface = doc.layer_mut(gone).unwrap().surface_mut().unwrap();
        let red = photocraft_raster::from_rgba(&surface.format(), [1.0, 0.0, 0.0, 1.0]);
        surface.fill_rect(Rect::new(150, 0, 190, 40), &red);
    }
    app.run("layer.new.layer", json!({"name": "Red"})).unwrap();
    app.run("paint.bucket", json!({"x": 0, "y": 0, "color": "#ff0000"})).unwrap();
    let red = active(&app);
    app.run("layer.new.layer", json!({"name": "Blue"})).unwrap();
    app.run("paint.bucket", json!({"x": 0, "y": 0, "color": "#0000ff"})).unwrap();
    let blue = active(&app);
    (app, Layers { empty, adjustment, gone, red, blue }, written, files)
}

/// The user answers the open file dialog with `path`, and the app takes the answer.
fn answer(app: &mut PhotocraftApp, files: &ScriptedFiles, path: &std::path::Path) {
    files.answer(Picked::Paths(vec![path.to_path_buf()])).expect("a file dialog is open");
    app.poll_file_dialog();
}

#[test]
fn quick_export_skips_the_same_layers() {
    let (mut app, l, written, files) = app();
    app.session.active_mut().unwrap().selected_layers = vec![l.empty, l.red, l.adjustment, l.blue, l.gone];
    quick_export_selected_layers_png(&mut app).unwrap();
    answer(&mut app, &files, std::path::Path::new("quick.png"));
    assert_eq!(*written.borrow(), ["quick.png", "quick-2-Blue.png"]);
}

#[test]
fn nothing_left_to_export_writes_nothing_and_says_why() {
    let (mut app, l, written, files) = app();
    let empties = [l.empty, l.adjustment, l.gone];
    let no_asset = |e: String| assert!(e.contains("No valid asset to export"), "{e}");
    no_asset(open_layers(&mut app, &empties).unwrap_err());
    let f = json!({"format": "png", "scale": 100}).as_object().unwrap().clone();
    no_asset(export_layers(&mut app, &f, &empties, "/out").unwrap_err());
    let st = app.session.active_mut().unwrap();
    st.selected_layers = empties.to_vec();
    st.active_layer = Some(l.gone);
    no_asset(quick_export_selected_layers_png(&mut app).unwrap_err());
    assert!(files.requests().is_empty(), "no file dialog was asked for");
    assert!(written.borrow().is_empty());
}

/// A real failure (a layer that isn't in the document) is still an error, not skipped.
#[test]
fn other_failures_are_still_errors() {
    let (app, l, _, _) = app();
    let doc = app.session.active().unwrap().doc.clone();
    let err = exportable_layers(&doc, &[l.red, LayerId(987_654)]).unwrap_err();
    assert!(!err.contains("No valid asset"), "{err}");
    assert_eq!(exportable_layers(&doc, &[l.gone, l.red]).unwrap().len(), 1);
}

/// Opening the dialog skips only empty layers (#15034 review). An unknown id is reported, before or after a valid
/// layer, and the empty-layer message is kept for a selection that is all empty.
#[test]
fn opening_with_an_unknown_layer_reports_it_instead_of_skipping_it() {
    let (mut app, l, _, _) = app();
    let unknown = LayerId(987_654);
    for selection in [vec![unknown, l.empty], vec![l.empty, unknown, l.red], vec![l.red, unknown]] {
        let err = open_layers(&mut app, &selection).unwrap_err();
        assert!(!err.contains("No valid asset"), "{selection:?}: the unknown id is the error, not the empty message: {err}");
        assert!(app.export_open.is_none(), "no dialog opened");
    }
    let e = open_layers(&mut app, &[l.empty, l.adjustment]).unwrap_err();
    assert!(e.contains("No valid asset to export"), "{e}");
}
