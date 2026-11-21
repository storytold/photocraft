//! #12084: the right-click menu while a Type Mask is edited has no type layer rows: no Rasterize Type, Create Work
//! Path, Convert to Shape, Horizontal or Vertical. The Type tool's edit menu keeps them.

use egui::Modifiers;
use serde_json::json;

use super::tests::{labels, open_here};
use super::{MenuKind, rows};
use crate::PhotocraftApp;
use crate::canvas::testing::{ToolEvent, tool_event};
use crate::state::Tool;

const LAYER_ROWS: [&str; 5] = ["Rasterize Type", "Create Work Path", "Convert to Shape", "Horizontal", "Vertical"];

/// An edit started by a click with `tool` on an 800 × 400 document, and the menu a right-click there opens.
fn edit_menu(tool: Tool) -> (Box<PhotocraftApp>, Vec<String>) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 400})).unwrap();
    app.sync_views();
    app.ui.tool_options.type_size = 48.0;
    app.ui.tool = tool;
    tool_event(&mut app, ToolEvent::Down { x: 100.0, y: 200.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 100.0, y: 200.0 }, Modifiers::NONE);
    assert!(app.ui.text_edit.is_some(), "{tool:?} is editing");
    assert!(open_here(&mut app, tool, [100.0, 200.0]), "{tool:?}: the menu opens");
    let menu = app.ui.canvas_tool_menu.clone().unwrap();
    assert!(matches!(menu.kind, MenuKind::TypeEdit { .. }), "{tool:?}: the edit menu");
    let shown = labels(rows(&menu)).into_iter().flatten().map(str::to_owned).collect();
    (app, shown)
}

#[test]
fn a_type_mask_edit_has_no_type_layer_rows() {
    for tool in [Tool::HorizontalTypeMask, Tool::VerticalTypeMask] {
        let (app, shown) = edit_menu(tool);
        assert!(app.ui.text_edit.as_ref().is_some_and(|ed| ed.mask), "{tool:?} edits a mask");
        for row in LAYER_ROWS {
            assert!(!shown.iter().any(|l| l == row), "{tool:?}: no {row}: {shown:?}");
        }
        for row in ["Select All", "Faux Bold", "Warp Text…"] {
            assert!(shown.iter().any(|l| l == row), "{tool:?}: {row} stays: {shown:?}");
        }
    }
}

#[test]
fn the_type_tools_edit_menu_keeps_them() {
    let (_, shown) = edit_menu(Tool::Type);
    for row in LAYER_ROWS {
        assert!(shown.iter().any(|l| l == row), "the Type tool keeps {row}: {shown:?}");
    }
}
