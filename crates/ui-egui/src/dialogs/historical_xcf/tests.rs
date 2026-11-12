use super::*;
#[path = "../../../../engine/src/historical_xcf_open/fixture.rs"]
mod fixture;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(
        photocraft_engine::Session::new(),
        crate::Services {
            import: Some(Box::new(|name, bytes, opts| photocraft_io::import_with_options(name, bytes, opts).map_err(|error| error.to_string()))),
            ..Default::default()
        },
    );
    app.background_jobs = false;
    app
}

fn choose(app: &mut PhotocraftApp, choice: &str) {
    let dialog = app.ui.dialogs.iter_mut().find(|dialog| owns(&dialog.fields)).unwrap();
    dialog.fields.insert("historicalXcfSampleOrder".into(), json!(choice));
    let id = dialog.id;
    crate::dialogs::confirm(app, id).unwrap();
}

fn assert_pixel(app: &PhotocraftApp) {
    let state = app.session.active().unwrap();
    let photocraft_doc::LayerContent::Raster(surface) = &state.doc.layers[0].content else { panic!("native raster") };
    let mut pixel = [0.; 4];
    surface.read_pixel(0, 0, &mut pixel);
    assert_eq!(pixel, [0x1234 as f32 / 65535., 0x4567 as f32 / 65535., 0x89ab as f32 / 65535., 1.]);
    assert!(state.source_read_only);
    assert!(state.path.is_none());
    assert_eq!(state.history.past_len(), 0);
}

#[test]
fn explicit_historical_orders_open_exact_native_samples_and_never_preselect() {
    for (little, order) in [(true, "little-endian"), (false, "big-endian")] {
        let mut app = app();
        app.open_file("/source/renamed.dat", &fixture::historical_pixel(little)).unwrap();
        assert!(app.session.documents().is_empty());
        let dialog = app.ui.dialogs.iter().find(|dialog| owns(&dialog.fields)).unwrap();
        assert_eq!(dialog.fields["historicalXcfSampleOrder"], Value::Null);
        assert!(!can_confirm(&dialog.fields));
        assert!(app.show_if_open("/source/renamed.dat"));
        assert_eq!(app.historical_opens.len(), 1, "the pending source is focused, not opened twice");
        choose(&mut app, order);
        assert_pixel(&app);
        assert!(app.historical_opens.is_empty());
        assert!(app.prompt_order.is_empty());
        assert_eq!(app.ui.recent_files.first().map(String::as_str), Some("/source/renamed.dat"));
    }
}

#[test]
fn missing_or_invalid_choice_keeps_dialog_source_and_document_unchanged_then_cancel_releases_it() {
    let mut app = app();
    app.run("file.new", json!({"width":2,"height":2})).unwrap();
    let control = app.session.active().unwrap().doc.clone();
    app.open_bytes("old.xcf", &fixture::historical_pixel(true)).unwrap();
    let dialog = app.ui.dialogs.iter().find(|dialog| owns(&dialog.fields)).unwrap().id;
    for invalid in [Value::Null, json!("native"), json!(true)] {
        app.ui.dialog_mut(dialog).unwrap().fields.insert("historicalXcfSampleOrder".into(), invalid);
        assert!(crate::dialogs::confirm(&mut app, dialog).is_err());
        assert!(app.ui.dialog_mut(dialog).is_some());
        assert_eq!(app.historical_opens.len(), 1);
        assert_eq!(app.session.documents().len(), 1);
        assert!(std::sync::Arc::ptr_eq(&control, &app.session.active().unwrap().doc));
    }
    let source = app.historical_opens[0].id;
    crate::dialogs::cancel(&mut app, dialog).unwrap();
    assert!(app.historical_opens.is_empty());
    assert!(app.prompt_order.is_empty());
    assert!(!app.session.cancel_historical_xcf_open(source));
    assert!(std::sync::Arc::ptr_eq(&control, &app.session.active().unwrap().doc));
}

fn later_document() -> crate::open_prompt::Held {
    crate::open_prompt::Held {
        doc: photocraft_doc::Document::new("later", photocraft_geom::Size::new(2, 2), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8),
        initial_view: None,
        read_only: false,
        damage_pending: false,
        name: "later".into(),
        path: None,
        warnings: Vec::new(),
        bytes: None,
        slot: None,
        order: None,
        question: None,
        policy_applied: true,
        smart_object: false,
        color_settings: None,
        ignore_exif_profile_tag: None,
    }
}

#[test]
fn historical_resume_keeps_its_position_before_later_file_and_profile_prompt() {
    for background in [false, true] {
        let mut app = app();
        // XCF's implicit nonlinear RGB encoding has a native sRGB profile, so a missing-profile
        // preference would never hold this fixture. Deliberately create a real mismatch.
        app.run("edit.colorSettings", json!({"workingRgb":"adobe-rgb-compat","policyRgb":"preserve","askOnMismatch":true})).unwrap();
        let bytes = fixture::historical_pixel(true);
        let options =
            photocraft_io::ImportOptions { historical_xcf_sample_order: Some(photocraft_io::HistoricalXcfSampleOrder::LittleEndian), ..Default::default() };
        let imported = photocraft_io::import_with_options("first.xcf", &bytes, &options).unwrap();
        let question = app.session.color.open_question(&imported.document).expect("the fixture must require the profile prompt this test orders");
        assert_eq!(question["mismatch"], true);
        app.open_bytes("first.xcf", &bytes).unwrap();
        crate::open_prompt::arrive(&mut app, later_document());
        app.background_jobs = background;
        choose(&mut app, "little-endian");
        if background {
            let ctx = egui::Context::default();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while !app.jobs.opens.is_empty() && std::time::Instant::now() < deadline {
                let mut output = ctx.run_ui(Default::default(), |ui| crate::jobs_ui::tick(&mut app, ui.ctx()));
                output.textures_delta.clear();
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert!(app.jobs.opens.is_empty(), "historical worker finished");
        }
        assert!(app.session.documents().is_empty());
        assert_eq!(app.held_opens.front().unwrap().name, "first.xcf");
        let prompt = app.ui.dialogs.iter().find(|dialog| crate::open_prompt::owns(&dialog.fields)).unwrap().id;
        crate::dialogs::confirm(&mut app, prompt).unwrap();
        assert_eq!(app.session.documents().iter().map(|state| state.doc.name.as_str()).collect::<Vec<_>>(), ["first.xcf", "later"]);
        app.session.set_active(0);
        assert_pixel(&app);
        assert!(app.historical_opens.is_empty());
        assert!(app.prompt_order.is_empty());
    }
}

#[test]
fn cancelled_historical_resume_does_not_steal_later_decoded_entry() {
    let mut app = app();
    app.open_bytes("first.xcf", &fixture::historical_pixel(true)).unwrap();
    crate::open_prompt::arrive(&mut app, later_document());
    app.background_jobs = true;
    choose(&mut app, "little-endian");
    let job = app.jobs.opens[0].job;
    app.session.cancel_job(job);
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(Default::default(), |ui| crate::jobs_ui::tick(&mut app, ui.ctx()));
    output.textures_delta.clear();
    assert_eq!(app.session.documents().len(), 1);
    assert_eq!(app.session.active().unwrap().doc.name, "later");
    assert!(app.historical_opens.is_empty());
    assert!(app.prompt_order.is_empty());
}

#[test]
fn earlier_profile_prompt_stays_ahead_of_historical_choice() {
    let mut app = app();
    let mut first = later_document();
    first.name = "earlier".into();
    first.question = Some(json!({"missing":true,"working":"sRGB"}));
    crate::open_prompt::arrive(&mut app, first);
    let earlier = app.ui.dialogs[0].id;
    app.open_bytes("historical.xcf", &fixture::historical_pixel(false)).unwrap();
    assert_eq!(app.ui.dialogs.len(), 1);
    assert!(!owns(&app.ui.dialogs[0].fields));
    crate::dialogs::confirm(&mut app, earlier).unwrap();
    assert_eq!(app.session.documents().len(), 1);
    assert!(owns(&app.ui.dialogs[0].fields));
    choose(&mut app, "big-endian");
    assert_pixel(&app);
    assert_eq!(app.session.documents().len(), 2);
}

#[test]
fn automation_open_forwards_choice_before_read_and_preserves_source_protection() {
    use std::cell::Cell;
    use std::rc::Rc;
    for (little, order) in [(true, "little-endian"), (false, "big-endian")] {
        let mut app = app();
        let reads = Rc::new(Cell::new(0));
        let count = reads.clone();
        app.services.automation_read = Some(Box::new(move |_| {
            count.set(count.get() + 1);
            Ok(crate::AutomationReadResult { name: "old.xcf".into(), bytes: fixture::historical_pixel(little), save_path: Some("source.xcf".into()) })
        }));
        let ctx = egui::Context::default();
        for choice in [json!("invalid"), json!(order)] {
            let (request, _) = crate::control::ControlRequest::new("app.open", json!({"path":"source.xcf","historicalXcfSampleOrder":choice}));
            let crate::control::Outcome::Done(reply) = crate::control::handle(&mut app, &ctx, &request) else { panic!("immediate") };
            if choice == "invalid" {
                assert_eq!(reply["ok"], false);
                assert_eq!(reads.get(), 0);
                assert!(app.session.documents().is_empty());
            } else {
                assert_eq!(reply["ok"], true, "{reply}");
                assert_eq!(reads.get(), 1);
                assert_pixel(&app);
            }
        }
    }
}
