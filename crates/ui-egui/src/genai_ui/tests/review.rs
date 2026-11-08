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

#[test]
fn the_prompt_says_how_many_requests_go_where_and_how_they_are_paid_for() {
    let ready = |local, billing| AiStatus::Ready { model: "Fill One".into(), connection: "OpenAI".into(), local, billing };
    let metered = cost_text(&ready(false, AiBilling::Metered), 3);
    assert_eq!(metered, ["Makes 3 variations: 3 separate requests to OpenAI.", "Each request is billed by the service."]);
    assert_eq!(cost_text(&ready(false, AiBilling::Unknown), 3)[1], "Check the service's own terms for what each request costs.");
    assert_eq!(cost_text(&ready(true, AiBilling::Free), 3), ["Makes 3 variations on this computer, with OpenAI."]);
    assert!(cost_text(&AiStatus::Unavailable { reason: "no".into() }, 3).is_empty());
    // Several areas filled on their own: three for each.
    let mut f = Map::new();
    f.insert(PROMPT_MARK.into(), json!(true));
    f.insert("__areas".into(), json!(4));
    assert_eq!(request_count(&f), 12);
    assert_eq!(ok_label(&f).as_deref(), Some("Generate 12"));
    f.insert("separate".into(), json!(false));
    assert_eq!(request_count(&f), 3, "filled as one");
    f.insert("separate".into(), json!(true));
    f.insert("__areas".into(), json!(MAX_SEPARATE + 1));
    assert_eq!(request_count(&f), 3, "too many to fill one by one: filled as one");
}

/// Two areas of different sizes selected, filled, the consent prompt open: its id.
fn two_areas_waiting(app: &mut PhotocraftApp) -> u64 {
    app.run("select.rect", json!({"x": 2, "y": 2, "width": 8, "height": 8})).unwrap();
    app.run("select.rect", json!({"x": 30, "y": 20, "width": 30, "height": 40, "mode": "add"})).unwrap();
    let id = open_prompt_dialog(app);
    crate::dialogs::confirm(app, id).unwrap();
    dialog_with(app, CONSENT_MARK).expect("the consent prompt opened")
}

#[test]
fn one_consent_prompt_shows_each_waiting_fills_sizes() {
    let (mut app, _) = app_with(Fake { remote: true, ..fake() });
    let consent = two_areas_waiting(&mut app);
    let fields = &app.ui.dialogs.iter().find(|d| d.id == consent).unwrap().fields;
    let source = &fields["items"][0];
    assert_eq!(source["input"], "source");
    let sizes: Vec<u64> = source["sizes"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    assert_eq!(sizes.len(), 2, "one size per fill that waits: {source}");
    let each: Vec<u64> = fields["fills"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| app.session.generative_fill_disclosure(id.as_u64().unwrap()).unwrap().items[0].bytes.unwrap())
        .collect();
    assert_eq!(sizes, each, "each fill's own size, in order");
    assert!(super::super::consent::item_text("source", &sizes).contains(", "));
}

#[test]
fn dont_send_asks_next_about_fills_left_waiting() {
    let (mut app, f) = app_with(Fake { remote: true, ..fake() });
    let consent = two_areas_waiting(&mut app);
    // As if the second fill came to wait after the prompt opened: the prompt is about the first.
    let ids: Vec<u64> = app.session.generative_fills().iter().map(|f| f.id).collect();
    app.ui.dialog_mut(consent).unwrap().fields.insert("fills".into(), json!([ids[0]]));
    crate::dialogs::cancel(&mut app, consent).unwrap();
    let next = dialog_with(&app, CONSENT_MARK).expect("the one left waiting is asked about");
    assert_eq!(app.ui.dialogs.iter().find(|d| d.id == next).unwrap().fields["fills"], json!([ids[1]]));
    assert_eq!(f.sent.load(Ordering::SeqCst), 0);
}

#[test]
fn a_successful_choice_clears_the_setup_message() {
    let (mut app, _) = app_with(fake());
    super::super::setup::changed(&mut app, Err(AiFailure::new(AiErrorKind::InvalidRequest, "no such model")));
    assert!(app.genai.setup.message.is_some());
    super::super::setup::changed(&mut app, Ok(()));
    assert!(app.genai.setup.message.is_none(), "the earlier failure isn't left showing");
}
