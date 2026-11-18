//! #12076: ⌘. (Ctrl+. off the Mac) ends a type editing session the way the options bar's Cancel
//! does, with any of the four type tools: the document, the selection and the history are as
//! before the session. ⇧⌘. stays Increase Type Size.
//! Keys go through a full app frame, so the canvas editor takes them as on the canvas.

use egui::Modifiers;
use photocraft_doc::{LayerContent, LayerId};
use photocraft_raster::Surface;
use serde_json::json;

use super::commit_tests::{edit_active, type_new};
use crate::PhotocraftApp;
use crate::state::Tool;

fn new_app(tool: Tool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    // A selection away from the text, which a Type Mask session must leave as it is.
    app.run("select.rect", json!({"x": 300, "y": 150, "width": 60, "height": 30})).unwrap();
    app.sync_views();
    app.ui.tool = tool;
    app
}

fn press(mut app: PhotocraftApp, k: egui::Key, modifiers: Modifiers) -> PhotocraftApp {
    crate::canvas::testing::key(&mut app, k, modifiers);
    app
}

fn command_period(app: PhotocraftApp) -> PhotocraftApp {
    press(app, egui::Key::Period, Modifiers::COMMAND)
}

/// The selection (compared pixel for pixel, shown by its bounds), the history (oldest first) and
/// the layers' ids.
#[derive(PartialEq)]
struct Snapshot {
    selection: Option<Surface>,
    history: Vec<String>,
    layers: Vec<LayerId>,
}

impl std::fmt::Debug for Snapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let bounds = self.selection.as_ref().map(Surface::content_bounds);
        write!(f, "selection {bounds:?}, history {:?}, layers {:?}", self.history, self.layers)
    }
}

fn snapshot(app: &PhotocraftApp) -> Snapshot {
    let st = app.session.active().unwrap();
    Snapshot { selection: st.doc.selection.clone(), history: st.history.entries(), layers: st.doc.layers.iter().map(|l| l.id).collect() }
}

fn text_of(app: &PhotocraftApp, id: LayerId) -> String {
    match &app.session.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Text(t) => t.text.clone(),
        other => panic!("{}", other.kind_name()),
    }
}

fn size_of(app: &PhotocraftApp, id: LayerId) -> f32 {
    match &app.session.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Text(t) => t.runs[0].style.size_pt,
        other => panic!("{}", other.kind_name()),
    }
}

#[test]
fn command_period_cancels_new_text_with_every_type_tool() {
    for tool in [Tool::Type, Tool::VerticalType, Tool::HorizontalTypeMask, Tool::VerticalTypeMask] {
        let mut app = new_app(tool);
        let before = snapshot(&app);
        type_new(&mut app, "AB");
        assert!(app.ui.text_edit.is_some(), "{tool:?}: editing");
        let app = command_period(app);
        assert!(app.ui.text_edit.is_none(), "{tool:?}: the session ends");
        assert_eq!(snapshot(&app), before, "{tool:?}: as before the session");
    }
}

#[test]
fn command_period_restores_edited_text() {
    for tool in [Tool::Type, Tool::VerticalType] {
        let mut app = new_app(tool);
        let orientation = if tool == Tool::VerticalType { "vertical" } else { "horizontal" };
        let made = app.run("type.create", json!({"text": "Hello", "x": 50, "y": 100, "orientation": orientation})).unwrap();
        let id = LayerId(made["layer"].as_u64().unwrap());
        let before = snapshot(&app);
        edit_active(&mut app);
        super::commit_tests::type_keys(&mut app, "CD");
        assert_ne!(text_of(&app, id), "Hello", "{tool:?}: typed");
        let app = command_period(app);
        assert!(app.ui.text_edit.is_none(), "{tool:?}: the session ends");
        assert_eq!(text_of(&app, id), "Hello", "{tool:?}: the text is as before");
        assert_eq!(snapshot(&app), before, "{tool:?}: as before the session");
    }
}

/// ⇧⌘. is not the editor's Cancel: it falls through to the app's Increase Type Size shortcut.
#[test]
fn shift_command_period_still_increases_the_type_size() {
    let mut app = new_app(Tool::Type);
    type_new(&mut app, "AB");
    let id = LayerId(app.ui.text_edit.as_ref().unwrap().layer);
    let before = size_of(&app, id);
    let app = press(app, egui::Key::Period, Modifiers::COMMAND | Modifiers::SHIFT);
    assert!(app.ui.text_edit.is_some(), "still editing");
    assert!(size_of(&app, id) > before, "Increase Type Size ran");
}
