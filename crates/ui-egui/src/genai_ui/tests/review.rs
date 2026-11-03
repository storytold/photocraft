//! The shell after review (#10903): placement notes, the count and cost before starting, consent
//! and setup fixes, and Save Fill Diagnostics.

use super::*;

#[test]
fn skipped_colour_matching_and_another_framing_are_said() {
    let note = |colour, framing| VariationNote { colour, framing };
    let plain = note(ColourMatch::Matched { drift: 0.01, changed: 0.0, shadows: 0.0, midtones: 0.0, highlights: 0.0 }, Framing::Same);
    assert!(note_lines(&plain).is_empty(), "nothing to say about a plain one");
    let skipped = note_lines(&note(ColourMatch::TooDifferent { drift: 0.4, limit: 0.25 }, Framing::Centre { returned: [1536, 1024], sent: [1024, 1024] }));
    assert_eq!(skipped.len(), 2);
    assert!(skipped[0].contains("Colour matching was skipped"), "{skipped:?}");
    assert!(skipped[1].contains("1536×1024") && skipped[1].contains("1024×1024") && skipped[1].contains("centre"), "{skipped:?}");
    assert!(note_lines(&note(ColourMatch::TooLittleOutside, Framing::Same))[0].contains("too little"));
    // The status line after a fill lands counts the skipped ones.
    let matching = json!([{"colour": {"state": "matched"}}, {"colour": {"state": "tooDifferent"}}, {"colour": {"state": "tooLittleOutside"}}]);
    assert_eq!(skipped_note(Some(&matching)).as_deref(), Some("Colour matching was skipped for 2 variations."));
    assert_eq!(skipped_note(Some(&json!([{"colour": {"state": "matched"}}]))), None);
    assert_eq!(skipped_note(None), None);
}

type Open = std::rc::Rc<std::cell::RefCell<Vec<(crate::file_dialog::FileDialogRequest, crate::file_dialog::FileDialogReply)>>>;

#[test]
fn save_fill_diagnostics_asks_for_a_folder_and_writes_there() {
    let open: Open = Default::default();
    let held = open.clone();
    let ai: photocraft_engine::genai::SharedAi = Arc::new(Ai(Arc::new(Fake { remote: false, ..fake() })));
    let services =
        Services { genai: Some(ai), file_dialog: Some(Box::new(move |request, _, reply| held.borrow_mut().push((request, reply)))), ..Default::default() };
    let mut app = PhotocraftApp::new(Session::new(), services);
    app.run("file.new", json!({"width": 64, "height": 64, "background": "#ffffff"})).unwrap();
    app.run("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    let r = app.run(FILL, json!({"prompt": "x", "variations": 1})).unwrap();
    let group = LayerId(r["layer"].as_u64().unwrap());
    assert!(offers_diagnostics(&app, group));
    let ctx = egui::Context::default();
    assert_eq!(crate::menus::invoke(&mut app, &ctx, SAVE, json!({})).unwrap(), json!({"fileDialog": "folder"}));
    app.poll_file_dialog(&ctx, None);
    let dir = tempfile::tempdir().unwrap();
    let (_, reply) = open.borrow_mut().pop().expect("a folder is asked for");
    let answer = Some(crate::file_dialog::FileDialogAnswer::FolderTo(dir.path().to_string_lossy().into_owned()));
    std::thread::spawn(move || reply.send(answer)).join().unwrap();
    app.poll_file_dialog(&ctx, None);
    let written: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
    assert_eq!(written.len(), 1, "one new folder");
    assert!(app.ui.status.starts_with("Saved Generative Fill diagnostics to"), "{}", app.ui.status);
    // The web build has no generative AI: nothing is offered.
    let mut web = PhotocraftApp::new(Session::new(), Services::default());
    web.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    assert!(!offers_diagnostics(&web, group));
    assert!(crate::menus::invoke(&mut web, &ctx, SAVE, json!({})).is_err());
}
