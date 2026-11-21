//! #11403: each row of the document tab's right-click menu runs a real command on the clicked tab.

use photocraft_doc::DocId;
use serde_json::json;

use crate::canvas::tabs::tab_context_items;
use crate::{PhotocraftApp, Services};

/// The rows name the clicked tab's document: the file commands by its index in the session's list (they take an index),
/// Move to New Window by its id.
#[test]
fn each_row_names_its_command_and_the_clicked_tabs_document() {
    let items = tab_context_items(DocId(7), 1, 3, true);
    let menu = crate::menus::menu_rows(&PhotocraftApp::new(photocraft_engine::Session::new(), Services::default()));
    let want = [
        ("Consolidate All to Here", "window.arrange.consolidateAllToTabs", json!({})),
        ("Close", "file.close", json!({"document": 1})),
        ("Close All", "file.closeAll", json!({})),
        ("Close Others", "file.closeOthers", json!({"document": 1})),
        ("Move to New Window", "window.arrange.floatInWindow", json!({"document": 7})),
        ("New Document…", "file.new", json!({})),
        ("Open Document…", "file.open", json!({})),
        (crate::menus::reveal_label(), "file.revealInFinder", json!({"document": 1})),
    ];
    for ((label, id, params, _), (wl, wi, wp)) in items.iter().zip(want) {
        assert_eq!((*label, *id, params), (wl, wi, &wp));
        assert!(menu.iter().any(|row| row.command.id == *id) || photocraft_engine::commands::find(id).is_some(), "{id} is a command");
    }
}

/// A document's id is not its place in the list: ids come from a counter shared with layers and other objects, so the
/// menu's Close, Close Others and Reveal in Finder named a document that did not exist and nothing happened.
#[test]
fn the_tab_menu_closes_the_clicked_document() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    for name in ["A", "B", "C"] {
        app.run("file.new", json!({"width": 8, "height": 8, "name": name})).unwrap();
    }
    let ctx = app.ctx().clone();
    let names = |app: &PhotocraftApp| app.session.documents().iter().map(|st| st.doc.name.clone()).collect::<Vec<_>>();
    assert!(app.session.documents().iter().enumerate().any(|(i, st)| st.doc.id.0 != i as u64), "ids are not indices");
    let run = |app: &mut PhotocraftApp, name: &str, label: &str| {
        let index = app.session.documents().iter().position(|st| st.doc.name == name).unwrap();
        let id = app.session.documents()[index].doc.id;
        let count = app.session.documents().len();
        let (_, command, params, _) = tab_context_items(id, index, count, false).into_iter().find(|(l, ..)| *l == label).unwrap();
        crate::menus::invoke(app, &ctx, command, params).unwrap();
    };
    // Close on a tab that is not the active one (C is).
    run(&mut app, "B", "Close");
    assert_eq!(names(&app), ["A", "C"]);
    run(&mut app, "A", "Close Others");
    assert_eq!(names(&app), ["A"]);
    run(&mut app, "A", "Close");
    assert!(names(&app).is_empty());
}
