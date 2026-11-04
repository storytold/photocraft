//! Views follow their document, not their slot: closing a tab must not hand its zoom or centre
//! to the document that shifts into its place.

use serde_json::json;

use crate::PhotocraftApp;

fn app_with_two_documents() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    assert_eq!(app.ui.views.len(), 2);
    // The first document is zoomed in; the second keeps its own default view.
    app.ui.views[0].zoom = 3.2;
    app.ui.views[0].fit_pending = false;
    app
}

/// Regression: views lived next to the documents by index alone, so closing a non-last tab
/// shifted the surviving document into the closed one's slot and it inherited that view — open a
/// 3200 % document, close it, and the next one came up at 3200 %. Views are matched by document
/// id now, so the survivor keeps its own (default) view.
#[test]
fn closing_a_tab_keeps_the_next_documents_own_view() {
    let mut app = app_with_two_documents();
    app.run("file.close", json!({"document": 0})).unwrap();
    assert_eq!(app.session.documents().len(), 1);
    assert_eq!(app.ui.views.len(), 1);
    assert_eq!(app.ui.views[0].zoom, 1.0, "the survivor must not inherit the closed document's zoom");
}

/// Opening and closing documents in any order keeps one view per document, and re-showing a
/// document (undo of a close is out of scope, but reopening by path is not) starts fresh rather
/// than claiming another document's view.
#[test]
fn every_document_keeps_its_own_view_through_syncs() {
    let mut app = app_with_two_documents();
    app.ui.views[1].zoom = 0.5;
    // A third document joins at the end, then the middle one closes: the third keeps its zoom.
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    app.ui.views[2].zoom = 4.0;
    app.run("file.close", json!({"document": 1})).unwrap();
    assert_eq!(app.ui.views.len(), 2);
    assert_eq!(app.ui.views[0].zoom, 3.2, "the first document's view is untouched");
    assert_eq!(app.ui.views[1].zoom, 4.0, "the last document's view follows it to its new index");
}

/// #11325: Zoom In/Out must never use a point right-clicked in another document.
#[test]
fn switching_documents_closes_the_canvas_view_menu() {
    for tool in [crate::Tool::Zoom, crate::Tool::Hand] {
        let mut app = app_with_two_documents();
        app.ui.tool = tool;
        let ctx = egui::Context::default();
        assert!(crate::canvas_tool_menu::open_view(&mut app, &ctx, tool, [10.0, 10.0], [20.0, 20.0]));
        app.sync_views();
        assert!(app.ui.canvas_tool_menu.is_some(), "ordinary sync keeps the menu");
        app.run("layer.new.layer", json!({})).unwrap();
        assert!(app.ui.canvas_tool_menu.is_some(), "an edit in the same document keeps the menu");
        assert!(app.session.set_active(0));
        app.sync_views();
        assert!(app.ui.canvas_tool_menu.is_none(), "a document switch closes the old document's menu");
    }
}

/// A tab closing can put another document at the same index, or move the menu's own document
/// to another index. Follow its identity, not its slot.
#[test]
fn closing_a_tab_keeps_the_canvas_menu_only_for_its_own_document() {
    for close_menu_document in [false, true] {
        let mut app = app_with_two_documents();
        if close_menu_document {
            assert!(app.session.set_active(0));
            app.sync_views();
        }
        app.ui.tool = crate::Tool::Hand;
        let document = app.session.active().unwrap().doc.id;
        assert!(crate::canvas_tool_menu::open_view(&mut app, &egui::Context::default(), crate::Tool::Hand, [10.0, 10.0], [20.0, 20.0]));
        app.run("file.close", json!({"document": 0})).unwrap();
        assert_eq!(app.session.active_index(), Some(0), "the survivor occupies the first slot");
        assert_eq!(app.session.active().unwrap().doc.id == document, !close_menu_document);
        assert_eq!(app.ui.canvas_tool_menu.is_some(), !close_menu_document, "only a menu for the surviving document stays open");
    }
}
