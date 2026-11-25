//! #12061: the Type options bar's "Toggle text orientation" button. It is greyed, and takes no click, while the
//! bar has no text layer in hand; with one selected or edited it stays live, and its click converts that layer
//! where the layer stands. Both the Horizontal and the Vertical Type tool behave the same way.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::LayerId;
use photocraft_doc::text::Orientation;
use serde_json::json;

use crate::PhotocraftApp;
use crate::app::framework::BuildApp;
use crate::state::Tool;

const TOGGLE: &str = "Toggle text orientation";

/// An 800x400 document, whose only layer is the pixel background, with the Type tool `tool` active.
fn new_app(tool: Tool) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 400})).unwrap();
    app.sync_views();
    app.ui.tool = tool;
    app
}

/// A text layer of `orientation` at 600,100, made the selected layer (selected, not edited).
fn select_text(app: &mut PhotocraftApp, orientation: Orientation) -> LayerId {
    let name = if orientation == Orientation::Vertical { "vertical" } else { "horizontal" };
    let made = app.run("type.create", json!({"text": "AB CD", "x": 600, "y": 100, "orientation": name})).unwrap();
    let id = LayerId(made["layer"].as_u64().unwrap());
    app.run("layer.select", json!({"layer": id.0, "mode": "replace"})).unwrap();
    id
}

/// The options bar on its own.
fn bar(app: Box<PhotocraftApp>) -> Harness<'static, Box<PhotocraftApp>> {
    let ctx = app.ctx().clone();
    let mut h = Harness::builder().with_context(ctx).with_size(vec2(1400.0, 60.0)).build_app(
        |ui, app: &mut PhotocraftApp| {
            ui.horizontal(|ui| super::options_bar(app, ui));
        },
        app,
    );
    h.run_steps(3);
    h
}

fn greyed(h: &Harness<'static, Box<PhotocraftApp>>) -> bool {
    h.get_by_label(TOGGLE).accesskit_node().is_disabled()
}

fn steps(h: &Harness<'static, Box<PhotocraftApp>>) -> usize {
    h.state().session.active().unwrap().history.entries().len()
}

fn text(h: &Harness<'static, Box<PhotocraftApp>>, id: LayerId) -> photocraft_doc::TextLayer {
    let st = h.state().session.active().unwrap();
    super::text_layer(&st.doc, id).expect("the text layer is still there").clone()
}

/// A document with only its pixel background layer: no text layer is being edited or selected, so the button is
/// greyed for both Type tools, and its click leaves the history as it was.
#[test]
fn a_bar_without_a_text_layer_greys_the_toggle_and_its_click_changes_nothing() {
    for tool in [Tool::Type, Tool::VerticalType] {
        let app = new_app(tool);
        let mut h = bar(app);
        assert!(greyed(&h), "{tool:?}: no text layer to convert");
        let before = steps(&h);
        h.get_by_label(TOGGLE).click();
        h.run_steps(2);
        assert_eq!(steps(&h), before, "{tool:?}: the click of the greyed toggle makes no history state");
    }
}

/// A selected text layer keeps the button live for both Type tools.
#[test]
fn a_selected_text_layer_keeps_the_toggle_live_for_both_type_tools() {
    for tool in [Tool::Type, Tool::VerticalType] {
        let mut app = new_app(tool);
        select_text(&mut app, Orientation::Horizontal);
        let h = bar(app);
        assert!(!greyed(&h), "{tool:?}: a selected text layer offers the toggle");
    }
}

/// Not measured by the ticket: the button while characters are being edited keeps today's behaviour and stays
/// live, because the edited layer is the one in hand.
#[test]
fn an_open_text_edit_keeps_the_toggle_live() {
    for tool in [Tool::Type, Tool::VerticalType] {
        let mut app = new_app(tool);
        let id = select_text(&mut app, Orientation::Horizontal);
        app.ui.text_edit = Some(crate::state::TextEdit {
            layer: id.0,
            document: None,
            caret: 2,
            anchor: 0,
            session: "orientation-toggle-test".into(),
            created: false,
            mask: false,
        });
        let h = bar(app);
        assert!(!greyed(&h), "{tool:?}: the edited layer is a text target");
    }
}

/// Not measured by the ticket: with several text layers selected the bar still has a text layer in hand, so the
/// button keeps today's behaviour and stays live.
#[test]
fn several_selected_text_layers_keep_the_toggle_live() {
    let mut app = new_app(Tool::Type);
    let first = select_text(&mut app, Orientation::Horizontal);
    let made = app.run("type.create", json!({"text": "Second", "x": 300, "y": 300})).unwrap();
    let second = LayerId(made["layer"].as_u64().unwrap());
    app.run("layer.select", json!({"layer": second.0, "mode": "add"})).unwrap();
    let h = bar(app);
    assert!(!greyed(&h), "{first:?} and {second:?} are both text layers to convert");
}

/// Not measured by the ticket: a text layer under Lock All keeps today's behaviour and stays clickable, since the
/// command itself answers a locked target.
#[test]
fn a_locked_text_layer_keeps_the_toggle_clickable() {
    let mut app = new_app(Tool::Type);
    let id = select_text(&mut app, Orientation::Horizontal);
    app.run("layer.lockLayers", json!({"all": true})).unwrap();
    let h = bar(app);
    assert!(!greyed(&h), "the locked text layer {id:?} is still a text target");
}

/// A click converts the layer in hand, one history state per click, and leaves its anchor where it was.
#[test]
fn a_click_converts_the_layer_and_keeps_its_anchor() {
    let mut app = new_app(Tool::Type);
    let id = select_text(&mut app, Orientation::Vertical);
    let mut h = bar(app);
    let anchor = text(&h, id).transform.m;
    let before = steps(&h);
    h.get_by_label(TOGGLE).click();
    h.run_steps(2);
    let converted = text(&h, id);
    assert_eq!(converted.orientation, Orientation::Horizontal, "the click lays the text out across");
    assert_eq!(converted.transform.m, anchor, "the anchor stays where it was");
    assert_eq!(steps(&h), before + 1, "one history state for the conversion");
    h.get_by_label(TOGGLE).click();
    h.run_steps(2);
    assert_eq!(text(&h, id).orientation, Orientation::Vertical, "the second click turns it back");
}
