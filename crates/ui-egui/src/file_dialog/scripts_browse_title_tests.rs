//! #13846: File › Scripts › Browse… asks for a picker titled "Load" whose one filter is named
//! "Script Files" and keeps today's extensions; other pickers keep their titles and filter names.

use std::{cell::RefCell, rc::Rc};

use serde_json::json;

use super::{FileDialogReply, FileDialogRequest};
use crate::{PhotocraftApp, Services, menus};

type Asked = Rc<RefCell<Vec<FileDialogRequest>>>;

/// An app whose file dialogs are recorded (and left unanswered) instead of shown.
fn app() -> (PhotocraftApp, Asked) {
    let asked: Asked = Default::default();
    let held = asked.clone();
    let services =
        Services { file_dialog: Some(Box::new(move |request, _parent, _reply: FileDialogReply| held.borrow_mut().push(request))), ..Default::default() };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
    (app, asked)
}

fn ask(command: &str) -> FileDialogRequest {
    let (mut app, asked) = app();
    let ctx = egui::Context::default();
    let _ = menus::invoke(&mut app, &ctx, command, json!({}));
    app.poll_file_dialog(&ctx, None);
    asked.borrow().last().cloned().unwrap_or_else(|| panic!("{command} asks for a file"))
}

#[test]
fn browse_asks_for_a_load_picker_with_one_script_filter() {
    let FileDialogRequest::Open { multiple, extensions, title, filter_name, .. } = ask("file.scripts.browse") else { panic!("an Open picker") };
    assert!(!multiple, "one script");
    assert_eq!(title.as_deref(), Some("Load"));
    assert_eq!(filter_name.as_deref(), Some("Script Files"));
    let exts = extensions.expect("one filter");
    assert_eq!(exts, ["psjs", "ccjs", "jsx", "js", "jsxbin", "json", "txt", "pcdroplet"], "today's extensions");
}

#[test]
fn other_pickers_keep_the_system_title_and_filter_name() {
    let open = ask("file.open");
    assert!(matches!(open, FileDialogRequest::Open { title: None, filter_name: None, .. }), "{open:?}");
    let (mut app, asked) = app();
    let _ = app.pick_file_bytes_filtered(&["cube"], |_, _, _| Ok(json!(null)));
    app.poll_file_dialog(&egui::Context::default(), None);
    let lut = asked.borrow().last().cloned().unwrap();
    assert!(matches!(&lut, FileDialogRequest::Open { title: None, filter_name: None, extensions: Some(e), .. } if e == &["cube"]), "{lut:?}");
}
