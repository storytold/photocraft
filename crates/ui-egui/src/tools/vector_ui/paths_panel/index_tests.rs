//! Framework events preserve indices across duplicate names and batch operations.
use super::*;
use crate::state::PathSelection;
use craft::keymap::Mods;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width":32,"height":32})).unwrap();
    for (name, x) in [("A", 1), ("B", 8), ("C", 16), ("work", 24)] {
        app.run("path.set", json!({"name":name,"path":{"subpaths":[{"knots":[[x,1],[x+4,1],[x+4,5]],"closed":true}]}})).unwrap();
    }
    for p in &mut std::sync::Arc::make_mut(&mut app.session.active_mut().unwrap().doc).paths {
        p.name = "Same".into();
    }
    app.sync_views();
    app
}

#[test]
fn framework_row_events_keep_duplicate_selection_rename_and_delete_independent() {
    let mut app = app();
    let before = app.session.active().unwrap().doc.paths.clone();
    apply(&mut app, PathsEvent::Select(Some(PathRef::Saved(1))));
    assert_eq!(app.ui.selected_path, Some(PathSelection::Saved { index: 1 }));
    apply(&mut app, PathsEvent::rename(PathRef::Saved(1), "Second"));
    assert_eq!(app.session.active().unwrap().doc.paths[0], before[0]);
    assert_eq!(app.session.active().unwrap().doc.paths[1].name, "Second");
    assert_eq!(app.ui.selected_path, Some(PathSelection::Saved { index: 1 }));
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc.paths, before);
    app.run("edit.redo", json!({})).unwrap();
    apply(&mut app, PathsEvent::command(PathsCommands::DeletePath, Some(PathRef::Saved(1))));
    assert_eq!(app.session.active().unwrap().doc.paths, vec![before[0].clone(), before[2].clone()]);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc.paths[1].name, "Second");
}

#[test]
fn framework_batch_delete_sorts_and_deduplicates_saved_indices() {
    for drop in [false, true] {
        let mut app = app();
        let before = app.session.active().unwrap().doc.clone();
        let rows = vec![PathRef::Saved(0), PathRef::Saved(1), PathRef::Saved(0)];
        let event =
            if drop { PathsEvent::footer_drop(PathsFooterButton::Delete, rows) } else { PathsEvent::footer(PathsFooterButton::Delete, rows, Mods::NONE) };
        apply(&mut app, event);
        assert_eq!(app.session.active().unwrap().doc.paths, vec![before.paths[2].clone()]);
        assert_eq!(app.session.active().unwrap().doc.work_path, before.work_path);
        for _ in 0..2 {
            app.run("history.undo", json!({})).unwrap();
        }
        assert_eq!(app.session.active().unwrap().doc.paths, before.paths);
        for _ in 0..2 {
            app.run("history.redo", json!({})).unwrap();
        }
        assert_eq!(app.session.active().unwrap().doc.paths, vec![before.paths[2].clone()]);
    }
}

#[test]
fn framework_footer_never_falls_back_from_invalid_saved_row_to_work() {
    for button in [PathsFooterButton::Fill, PathsFooterButton::Stroke, PathsFooterButton::Load, PathsFooterButton::VectorMask, PathsFooterButton::Delete] {
        let mut app = app();
        let before = app.session.active().unwrap().doc.clone();
        let n = app.session.active().unwrap().history.past_len();
        apply(&mut app, PathsEvent::footer(button, vec![PathRef::Saved(99)], Mods::NONE));
        assert_eq!(app.session.active().unwrap().doc.as_ref(), before.as_ref());
        assert_eq!(app.session.active().unwrap().history.past_len(), n);
        app.ui.selected_path = Some(PathSelection::Saved { index: 99 });
        apply(&mut app, PathsEvent::footer(button, vec![], Mods::NONE));
        assert_eq!(app.session.active().unwrap().doc.as_ref(), before.as_ref());
        assert_eq!(app.session.active().unwrap().history.past_len(), n);
    }
}

#[test]
fn framework_saving_work_path_selects_the_new_saved_index() {
    for menu in [false, true] {
        let mut app = app();
        let before = app.session.active().unwrap().doc.clone();
        let index = before.paths.len();
        let event = if menu { PathsEvent::command(PathsCommands::SavePath, Some(PathRef::Work)) } else { PathsEvent::rename(PathRef::Work, "Saved work") };
        apply(&mut app, event);
        assert_eq!(app.ui.selected_path, Some(PathSelection::Saved { index }));
        let doc = &app.session.active().unwrap().doc;
        assert!(doc.work_path.is_none());
        assert_eq!(Some(&doc.paths[index].path), before.work_path.as_ref());
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.work_path, before.work_path);
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.paths.len(), index + 1);
    }
}
