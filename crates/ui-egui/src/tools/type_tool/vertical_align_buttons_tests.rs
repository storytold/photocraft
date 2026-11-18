//! #12059: for vertical text the options bar's alignment buttons are Top / Center / Bottom, after
//! the targeted text layer's orientation, or with no text targeted the Vertical Type tool's. The
//! Horizontal Type Mask tool keeps the horizontal buttons (not measured); the Vertical Type Mask
//! tool's are in `vertical_mask_align_buttons_tests` (#12088).

use crate::app::framework::BuildApp;

use egui::accesskit::Toggled;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::{LayerContent, LayerId, text::TextAlign};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

pub(super) const HORIZONTAL: [&str; 3] = ["Left align text", "Center text", "Right align text"];
pub(super) const VERTICAL: [&str; 3] = ["Align text to the top", "Center text vertically", "Align text to the bottom"];

pub(super) fn new_app(tool: Tool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 400})).unwrap();
    app.sync_views();
    app.ui.tool = tool;
    app
}

/// A text layer with `orientation`, made the active layer (selected, not edited).
fn text_layer(app: &mut PhotocraftApp, orientation: &str) -> LayerId {
    let made = app.run("type.create", json!({"text": "AB CD", "x": 400, "y": 100, "orientation": orientation})).unwrap();
    LayerId(made["layer"].as_u64().unwrap())
}

pub(super) fn bar(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let ctx = app.ctx().clone();
    let mut h = Harness::builder().with_context(ctx).with_size(egui::vec2(1200.0, 60.0)).build_app(
        |ui, app: &mut PhotocraftApp| {
            ui.horizontal(|ui| super::options_bar(app, ui));
        },
        app,
    );
    h.run_steps(3);
    h
}

/// The bar drawn without the app's frame logic: that logic ends a text edit whose document is not the
/// active one, and the bar's own decision for such an edit is what the caller tests.
fn bar_without_frame_logic(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let ctx = app.ctx().clone();
    let mut h = Harness::builder().with_context(ctx).with_size(egui::vec2(1200.0, 60.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            ui.horizontal(|ui| super::options_bar(app, ui));
        },
        app,
    );
    h.run_steps(3);
    h
}

/// Which of the two button sets the bar shows; panics on a mix or neither.
pub(super) fn shown(h: &Harness<'static, PhotocraftApp>) -> [&'static str; 3] {
    let has = |label: &str| h.query_by_label(label).is_some();
    match (has(VERTICAL[0]) && has(VERTICAL[2]), has(HORIZONTAL[0]) && has(HORIZONTAL[2])) {
        (true, false) => VERTICAL,
        (false, true) => HORIZONTAL,
        (true, true) => panic!("both the vertical and the horizontal buttons"),
        (false, false) => panic!("no alignment buttons"),
    }
}

pub(super) fn lit(h: &Harness<'static, PhotocraftApp>, label: &str) -> bool {
    h.get_by_label(label).accesskit_node().toggled() == Some(Toggled::True)
}

#[allow(dead_code)] // a fixture for tests of the bar's alignment buttons
pub(super) fn align_of(app: &PhotocraftApp, id: LayerId) -> TextAlign {
    match &app.session.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Text(t) => t.paragraphs[0].style.align,
        other => panic!("{}", other.kind_name()),
    }
}

/// The target's orientation decides the buttons, else the tool.
#[test]
fn with_no_text_targeted_the_tool_decides() {
    for (tool, buttons) in [(Tool::VerticalType, VERTICAL), (Tool::Type, HORIZONTAL)] {
        let h = bar(new_app(tool));
        assert_eq!(shown(&h), buttons, "{tool:?}");
        assert!(lit(&h, buttons[0]), "{tool:?}: the default, left, lights the first button");
    }
    for tool in [Tool::Type, Tool::VerticalType] {
        for (orientation, buttons) in [("vertical", VERTICAL), ("horizontal", HORIZONTAL)] {
            let mut app = new_app(tool);
            text_layer(&mut app, orientation);
            assert_eq!(shown(&bar(app)), buttons, "{tool:?}, {orientation} layer selected");
        }
    }
    for (orientation, buttons) in [("vertical", VERTICAL), ("horizontal", HORIZONTAL)] {
        let mut app = new_app(Tool::Type);
        text_layer(&mut app, orientation);
        let ctx = app.ctx().clone();
        super::edit_active(&mut app, &ctx).unwrap();
        assert!(app.ui.text_edit.is_some());
        assert_eq!(shown(&bar(app)), buttons, "editing {orientation} text");
    }
}

#[test]
fn the_horizontal_type_mask_tool_keeps_the_horizontal_buttons() {
    assert_eq!(shown(&bar(new_app(Tool::HorizontalTypeMask))), HORIZONTAL, "idle");
    let mut app = new_app(Tool::HorizontalTypeMask);
    crate::canvas::testing::tool_event(&mut app, crate::canvas::testing::ToolEvent::Down { x: 400.0, y: 100.0, pressure: 1.0 }, egui::Modifiers::NONE);
    crate::canvas::testing::tool_event(&mut app, crate::canvas::testing::ToolEvent::Up { x: 400.0, y: 100.0 }, egui::Modifiers::NONE);
    assert!(app.ui.text_edit.as_ref().is_some_and(|ed| ed.mask), "typing a mask");
    assert_eq!(shown(&bar(app)), HORIZONTAL, "editing");
}

/// A text edit left in a background document decides by its own layer, not by a same-id layer of
/// the active document (layer ids repeat across documents; as `styles_at`, #9179).
#[test]
fn a_background_text_edit_decides_by_its_own_document() {
    let mut app = new_app(Tool::Type);
    let a = app.session.active().unwrap().doc.id.0;
    let id = text_layer(&mut app, "vertical");
    app.ui.text_edit = Some(crate::state::TextEdit { layer: id.0, document: Some(a), caret: 0, anchor: 0, session: "s".into(), created: false, mask: false });
    // The same file opened twice, its copy of the layer turned horizontal.
    let twin = app.session.active().unwrap().doc.as_ref().clone();
    app.session.open_document(twin, None);
    app.run("type.orientation.horizontal", json!({"layer": id.0})).unwrap();
    assert_ne!(app.session.active().unwrap().doc.id.0, a, "the other document is active");
    assert_eq!(shown(&bar_without_frame_logic(app)), VERTICAL, "the edited layer is vertical");
}
