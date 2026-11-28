//! #14803: Layer > Export As takes its selected layers in Layers panel order, top to bottom. The
//! dialog opens on the top row (its trimmed size and its preview), the batch writes the rows in
//! that order while #14808's duplicate naming (the upper layer keeps the plain name) holds, and the
//! dialog is titled `Export As` for one layer and for several.

use super::*;
use craft::host::{Picked, ScriptedFiles};
use egui_kittest::Harness;
use std::{cell::RefCell, path::Path, rc::Rc};

type Written = Rc<RefCell<Vec<String>>>;

/// A 40x30 document with one painted pixel layer per entry of `layers` (name, x, y, width, height),
/// bottom to top. Returns the app, its layers in Layers panel order (top row first), the paths its
/// writer was asked for, and its file dialogs, which stay open until the test answers them.
fn painted_layers(layers: &[(&str, i32, i32, u32, u32)]) -> (Box<PhotocraftApp>, Vec<LayerId>, Written, Arc<ScriptedFiles>) {
    let files = Arc::new(ScriptedFiles::held());
    let written = Written::default();
    let capture = written.clone();
    let services = crate::Services {
        files: Some(files.clone()),
        export: Some(Box::new(|_, _, _| Ok((Vec::new(), Vec::new())))),
        write: Some(Box::new(move |path, _| {
            capture.borrow_mut().push(path.to_string());
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.run("file.new", json!({"width": 40, "height": 30})).unwrap();
    let mut created = Vec::new();
    for (name, x, y, width, height) in layers {
        app.run("layer.new.layer", json!({"name": name})).unwrap();
        app.run("select.rect", json!({"x": x, "y": y, "width": width, "height": height})).unwrap();
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        created.push(app.session.active().unwrap().active_layer.unwrap());
    }
    (app, created.into_iter().rev().collect(), written, files)
}

/// `Bottom` (30x5), `Middle` (20x20) and `Square` (10x6), bottom to top.
fn layered_app() -> (Box<PhotocraftApp>, Vec<LayerId>, Written, Arc<ScriptedFiles>) {
    painted_layers(&[("Bottom", 0, 0, 30, 5), ("Middle", 0, 0, 20, 20), ("Square", 4, 2, 10, 6)])
}

/// `Dup`, `Dup` and `Top`, each 4x4, bottom to top.
fn duplicated_layers_app() -> (Box<PhotocraftApp>, Vec<LayerId>, Written, Arc<ScriptedFiles>) {
    painted_layers(&[("Dup", 0, 0, 4, 4), ("Dup", 4, 0, 4, 4), ("Top", 8, 0, 4, 4)])
}

/// The dialog host, so a frame runs the hosted dialog and draws its title.
fn harness(app: Box<PhotocraftApp>) -> Harness<'static, Box<PhotocraftApp>> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(900.0, 700.0))
        .with_step_dt(1.0 / 60.0)
        .with_context(app.ctx().clone())
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app);
    h.run_steps(2);
    h
}

/// Every string a frame draws, so a dialog's title can be read.
fn drawn_text(h: &Harness<'_, Box<PhotocraftApp>>) -> Vec<String> {
    fn collect(shape: &egui::Shape, text: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(shape) => text.push(shape.galley.job.text.clone()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, text)),
            _ => {}
        }
    }
    let mut text = Vec::new();
    for shape in &h.output().shapes {
        collect(&shape.shape, &mut text);
    }
    text
}

/// Layer > Export As on `ids` as PNG, writing into the folder `dir`: the first OK asks for the
/// folder, the test answers it, and the next OK writes the files.
fn export_folder(app: &mut PhotocraftApp, files: &ScriptedFiles, ids: &[LayerId], dir: &Path) {
    let ctx = app.ctx().clone();
    let id = open_layers(app, ids).expect("the dialog opens for several layers");
    app.framework.dialogs.set_json_field(&ctx, id, "format", json!("png")).expect("the format row");
    crate::dialogs::confirm(app, &ctx, id).expect("Export is enabled");
    files.answer(Picked::Paths(vec![dir.to_path_buf()])).expect("the folder dialog is open");
    app.poll_file_dialog();
    // The dialog's next pass takes the answered folder and exports.
    crate::dialogs::confirm(app, &ctx, id).expect("Export is enabled a second time");
}

/// The file names the writer was asked for, in the order it was asked for them.
fn written_names(written: &Written) -> Vec<String> {
    written.borrow().iter().map(|path| Path::new(path).file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()).collect()
}

/// Layer > Export As with several layers selected lists one row per selected layer, in Layers panel
/// order top to bottom (the reference application's order, not the document's bottom to top).
#[test]
fn the_menu_lists_one_row_per_selected_layer_in_panel_order() {
    let (mut app, rows, _, _) = layered_app();
    let selected: Vec<u64> = rows.iter().map(|layer| layer.0).collect();
    app.run("layer.select", json!({"layers": selected})).expect("three layers selected");
    let ctx = app.ctx().clone();
    crate::menus::invoke(&mut app, &ctx, "layer.exportAs", json!({})).expect("Layer > Export As opens");
    let open = app.export_open.as_ref().expect("the dialog is open");
    assert_eq!(open.layers, rows, "one row per selected layer, the Layers panel's top row first");
}

/// The dialog opens on the first (top) row: its size fields fit that row's trimmed size, and the
/// preview it asks for renders that layer.
#[test]
fn the_dialog_opens_on_the_top_row() {
    let (mut app, rows, _, _) = layered_app();
    let selected: Vec<u64> = rows.iter().map(|layer| layer.0).collect();
    app.run("layer.select", json!({"layers": selected})).expect("three layers selected");
    let ctx = app.ctx().clone();
    crate::menus::invoke(&mut app, &ctx, "layer.exportAs", json!({})).expect("Layer > Export As opens");
    let open = app.export_open.as_ref().expect("the dialog is open");
    assert_eq!(open.source, PixelSize::new(10, 6), "the size fields follow the top row, the 10x6 `Square`");
    let work = start_preview(&app, open, PreviewRequest::new(1, ExportOptions::default())).expect("a preview request");
    assert_eq!(work.settings.get("__layer"), Some(&json!(rows[0].0)), "the preview renders the top row");
}

/// The rows are written in Layers panel order, and #14808's naming still gives the plain name to
/// the upper of two layers that share one.
#[test]
fn the_rows_are_written_in_panel_order_and_the_upper_one_keeps_a_duplicate_name() {
    let (mut app, rows, written, files) = duplicated_layers_app();
    let dir = tempfile::tempdir().expect("a temporary folder");
    export_folder(&mut app, &files, &rows, dir.path());
    assert_eq!(written_names(&written), ["Top.png", "Dup.png", "Dup-1.png"]);
}

/// The dialog is titled `Export As`, not after the layers it exports, for several layers.
#[test]
fn several_layers_open_titled_export_as() {
    let (app, rows, _, _) = layered_app();
    let mut h = harness(app);
    open_layers(h.state_mut(), &rows).expect("the dialog opens for several layers");
    h.run_steps(3);
    let text = drawn_text(&h);
    assert!(!text.iter().any(|drawn| drawn == "Export As: 3 layers"), "the title names no layer count: {text:?}");
    assert_eq!(text.iter().filter(|drawn| drawn.as_str() == "Export As").count(), 1, "the dialog is titled `Export As`: {text:?}");
}

/// And the same for one selected layer: no `Export As: <layer name>` either.
#[test]
fn one_layer_opens_titled_export_as() {
    let (app, rows, _, _) = layered_app();
    let mut h = harness(app);
    open_layer(h.state_mut(), rows[0]).expect("the dialog opens for one layer");
    h.run_steps(3);
    let text = drawn_text(&h);
    assert!(!text.iter().any(|drawn| drawn == "Export As: Square"), "the title names no layer: {text:?}");
    assert_eq!(text.iter().filter(|drawn| drawn.as_str() == "Export As").count(), 1, "the dialog is titled `Export As`: {text:?}");
}
