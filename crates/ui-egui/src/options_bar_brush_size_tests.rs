//! #16340: the brush picker field at the start of the options bar shows the tip over the brush size ("20" for the
//! default brush). Since the shell's options slot became a memoized view (#16118) the bar's line lost the slot's
//! height, so the two-line field was cut under the tip and the size read as a sliver. The whole field, caption
//! included, must lie inside the bar for every tool that shows it.

use egui::accesskit::Role;
use egui::{Rect, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_engine::Session;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

type H = Harness<'static, Box<PhotocraftApp>>;

/// The whole app with a new document and `tool` active, after a few settled frames: the options bar runs in the
/// shell's slot, as it does for a person, not on its own.
fn app_with(tool: Tool) -> H {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), s, crate::Services::default());
        app.ui.tool = tool;
        app
    });
    h.run_steps(8);
    h
}

/// The options bar's rect: the framework `Bar` is the group named Options (the widest one; its scrolling body is a
/// narrower group of the same name).
fn options_bar(h: &H) -> Rect {
    h.query_all_by_role(Role::Group)
        .filter(|node| node.accesskit_node().label().is_some_and(|label| label == "Options"))
        .map(|node| node.rect())
        .max_by(|a, b| a.width().total_cmp(&b.width()))
        .expect("the options bar")
}

/// The closed brush picker field's rect: the tip over its size caption and the chevron, the group named Brush Preset.
fn brush_field(h: &H) -> Rect {
    h.query_all_by_role(Role::Group)
        .find(|node| node.accesskit_node().label().is_some_and(|label| label == "Brush Preset"))
        .expect("the brush picker field")
        .rect()
}

/// The Brush Settings toggle the bar draws right after the field: a one-line, icon-only control.
fn one_line_control(h: &H) -> Rect {
    h.query_all_by_label("Toggle the Brush Settings panel").find(|node| node.accesskit_node().role() != Role::Label).expect("the Brush Settings toggle").rect()
}

/// Every tool that shows the brush picker field keeps the whole field, size caption included, inside the bar.
#[test]
fn the_brush_size_under_the_tip_lies_inside_the_options_bar() {
    for tool in [Tool::Brush, Tool::Pencil, Tool::Eraser, Tool::CloneStamp, Tool::Healing, Tool::Dodge, Tool::QuickSelection] {
        let h = app_with(tool);
        let bar = options_bar(&h);
        let field = brush_field(&h);
        let one_line = one_line_control(&h);
        // A tip of the icon size over a caption line: the field is taller than a one-line control, so this checks
        // the two-line content, not a bar that happens to fit a single line.
        assert!(field.height() > one_line.height(), "{tool:?}: the field {field:?} holds a tip over a caption, taller than a one-line control {one_line:?}");
        assert!(bar.contains_rect(field), "{tool:?}: the brush picker field {field:?} runs out of the options bar {bar:?}, so its size caption is cut");
        // The bar clips its members well inside its outer edge, so a field that only fits the bar's rect can still lose
        // its caption: with the slot's line lost the field sat about 3 points low, inside the bar's rect, and the
        // caption was cut. A field on the bar's middle line keeps the caption in view.
        assert!(
            (field.center().y - bar.center().y).abs() <= 1.0,
            "{tool:?}: the brush picker field {field:?} sits off the options bar's middle {bar:?}, so its size caption is cut"
        );
    }
}
