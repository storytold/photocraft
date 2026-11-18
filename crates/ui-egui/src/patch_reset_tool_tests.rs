//! Reset Tool keeps each tool's kept options (Patch Structure and Color #12127, Content-Aware Move Mode #12183,
//! Zoom In/Out #12066) and resets the rest; Reset All Tools keeps none.

use serde_json::json;

use crate::PhotocraftApp;
use crate::state::{Tool, ToolOptions};

fn app() -> PhotocraftApp {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    let mut app = PhotocraftApp::new(s, crate::Services::default());
    app.ui.tool = Tool::Patch;
    app
}

/// One tool: how its options are customized, and what each reset leaves.
struct Row {
    name: &'static str,
    tool: Tool,
    customize: fn(&mut ToolOptions),
    after_reset_tool: fn(&ToolOptions),
    after_reset_all: fn(&ToolOptions),
}

fn rows() -> Vec<Row> {
    vec![
        Row {
            name: "Patch",
            tool: Tool::Patch,
            customize: |o| {
                o.patch_structure = 7.0;
                o.patch_color = 5.0;
                o.patch_mode = "destination".into();
                o.patch_content_aware = true;
            },
            after_reset_tool: |o| {
                assert_eq!((o.patch_structure, o.patch_color), (7.0, 5.0));
                assert_eq!(o.patch_mode, "source");
                assert!(!o.patch_content_aware);
            },
            after_reset_all: |o| {
                assert_eq!((o.patch_structure, o.patch_color), (4.0, 0.0));
                assert_eq!(o.patch_mode, "source");
            },
        },
        Row {
            name: "Content-Aware Move",
            tool: Tool::ContentAwareMove,
            customize: |o| {
                o.cam_mode = "extend".into();
                o.cam_structure = 7.0;
                o.cam_color = 5.0;
                o.patch_structure = 7.0;
            },
            after_reset_tool: |o| {
                let d = ToolOptions::default();
                assert_eq!(o.cam_mode, "extend", "Reset Tool keeps the Mode");
                assert_eq!((o.cam_structure, o.cam_color), (d.cam_structure, d.cam_color));
                assert_eq!(o.patch_structure, 7.0, "another tool's option stays");
            },
            after_reset_all: |o| {
                assert_eq!(o.cam_mode, "move");
                assert_eq!((o.cam_structure, o.cam_color), (4.0, 0.0));
            },
        },
        Row {
            name: "Zoom",
            tool: Tool::Zoom,
            customize: |o| {
                o.zoom_in = false;
                o.zoom_scrubby = false;
            },
            after_reset_tool: |o| {
                assert!(!o.zoom_in, "Zoom Out stays selected");
                assert!(o.zoom_scrubby, "Scrubby Zoom goes back to on");
            },
            after_reset_all: |o| {
                assert!(o.zoom_in, "Reset All Tools gives Zoom In");
                assert!(o.zoom_scrubby);
            },
        },
    ]
}

#[test]
fn reset_tool_keeps_each_tools_kept_options_and_reset_all_keeps_none() {
    for row in rows() {
        let mut app = app();
        app.ui.tool = row.tool;
        (row.customize)(&mut app.ui.tool_options);
        app.ui.selection_mode = 2;
        let ctx = app.ctx().clone();
        crate::menus::invoke(&mut app, &ctx, crate::tool_reset::RESET, json!({})).unwrap();
        (row.after_reset_tool)(&app.ui.tool_options);
        if row.tool != Tool::Zoom {
            assert_eq!(app.ui.selection_mode, 0, "{}: Reset Tool resets the selection mode", row.name);
        }

        let mut app = app_with(&row);
        crate::menus::invoke(&mut app, &ctx, crate::tool_reset::RESET_ALL, json!({"confirmed": true})).unwrap();
        (row.after_reset_all)(&app.ui.tool_options);
    }
}

/// A fresh app with `row`'s options customized.
fn app_with(row: &Row) -> PhotocraftApp {
    let mut app = app();
    app.ui.tool = row.tool;
    (row.customize)(&mut app.ui.tool_options);
    app
}
