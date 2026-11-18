//! #11403: each row of the document tab's right-click menu runs a real command on the clicked tab.

use photocraft_doc::DocId;
use serde_json::json;

use crate::canvas::tabs::tab_context_items;
use crate::{PhotocraftApp, Services};

/// The rows name the clicked tab's document by its id, which stays the same when another document closes.
#[test]
fn each_row_names_its_command_and_the_clicked_tabs_document() {
    let items = tab_context_items(DocId(7), 3, true);
    let menu = crate::menus::menu_rows(&PhotocraftApp::new(photocraft_engine::Session::new(), Services::default()));
    let want = [
        ("Consolidate All to Here", "window.arrange.consolidateAllToTabs", json!({})),
        ("Close", "file.close", json!({"document": 7})),
        ("Close All", "file.closeAll", json!({})),
        ("Close Others", "file.closeOthers", json!({"document": 7})),
        ("Move to New Window", "window.arrange.floatInWindow", json!({"document": 7})),
        ("New Document…", "file.new", json!({})),
        ("Open Document…", "file.open", json!({})),
        (crate::menus::reveal_label(), "file.revealInFinder", json!({"document": 7})),
    ];
    for ((label, id, params, _), (wl, wi, wp)) in items.iter().zip(want) {
        assert_eq!((*label, *id, params), (wl, wi, &wp));
        assert!(menu.iter().any(|row| row.command.id == *id) || photocraft_engine::commands::find(id).is_some(), "{id} is a command");
    }
}
