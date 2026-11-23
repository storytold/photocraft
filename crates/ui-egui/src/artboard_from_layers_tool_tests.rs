//! OK in the New Artboard from Layers dialog makes the Artboard tool current, and its options bar shows the new
//! board (#14579). Cancel and runs that don't come from the dialog keep the current tool.

use craft::ui::dialog::DialogId;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

const ID: &str = "layer.new.artboardFromLayers";

/// A white 800 x 600 document with one red pixel layer at 100,100 to 300,250, the Move tool current.
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    app.run("layer.new.layer", json!({"name": "Red"})).unwrap();
    app.run("select.rect", json!({"x": 100, "y": 100, "width": 200, "height": 150})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.sync_views();
    crate::tool_memory::select(&mut app, Tool::Move);
    app
}

fn passes(app: &mut PhotocraftApp, n: usize) {
    let ctx = app.ctx().clone();
    for _ in 0..n {
        crate::testing::run_ui(&ctx, Default::default(), |ui| crate::dialogs::run(app, ui));
    }
}

fn open(app: &mut PhotocraftApp) -> DialogId {
    let id = crate::filter_dialog::open(app, ID).expect("the dialog opens");
    passes(app, 2);
    id
}

fn boards(app: &PhotocraftApp) -> Vec<photocraft_geom::Rect> {
    app.session.active().unwrap().doc.artboards().iter().map(|b| b.2.rect).collect()
}

#[test]
fn ok_makes_the_artboard_tool_current_with_the_new_board_in_its_bar() {
    let mut app = app();
    let id = open(&mut app);
    let ctx = app.ctx().clone();
    crate::dialogs::confirm(&mut app, &ctx, id).expect("OK runs the command");
    passes(&mut app, 3);
    let made = boards(&app);
    assert_eq!(made.len(), 1, "one board from the layer");
    assert_eq!(app.ui.tool, Tool::Artboard);
    // The bar edits the selected board: the new one, at its own size.
    let shown = crate::artboard_tool::items(&app);
    let at = *shown.selected.first().expect("the new board is the bar's board");
    let r = shown.items[at].rect;
    assert_eq!([r.width(), r.height()], [f64::from(made[0].width()), f64::from(made[0].height())]);
}

#[test]
fn cancel_keeps_the_current_tool() {
    let mut app = app();
    let id = open(&mut app);
    let ctx = app.ctx().clone();
    app.framework.dialogs.cancel_by_id(&ctx, id);
    passes(&mut app, 2);
    assert!(boards(&app).is_empty());
    assert_eq!(app.ui.tool, Tool::Move);
}

/// A script, an action or the control channel runs the command without the dialog: the tool stays (not measured in
/// the reference application, so today's behaviour is kept).
#[test]
fn a_run_without_the_dialog_keeps_the_current_tool() {
    let mut app = app();
    app.run(ID, json!({})).expect("the command runs");
    assert_eq!(boards(&app).len(), 1);
    assert_eq!(app.ui.tool, Tool::Move);
}
