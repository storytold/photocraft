//! File › Open's Image Sequence option and Frame Rate dialog (#14057).

use super::*;

/// A fresh folder holding the measured sequence: `frame001.png` .. `frame003.png` (40 x 30) and an
/// unnumbered `flat.png`.
fn sequence_folder(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("pc-ui-imgseq-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30, "background": "#ff0000"})).unwrap();
    let png = photocraft_io::export(&s.active().unwrap().doc, "png", &photocraft_io::ExportOptions::default()).unwrap().bytes;
    for file in ["frame001.png", "frame002.png", "frame003.png", "flat.png"] {
        std::fs::write(d.join(file), &png).unwrap();
    }
    d
}

/// The app after File › Open answered with `file` of the folder picked and Image Sequence ticked:
/// the Frame Rate dialog is up.
fn picked_with_image_sequence(folder: &std::path::Path, file: &str) -> PhotocraftApp {
    let (mut app, open, _) = app();
    let ctx = egui::Context::default();
    menus::invoke(&mut app, &ctx, "file.open", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    answer(&open, Some(FileDialogAnswer::ImageSequence(folder.join(file).to_string_lossy().into_owned())));
    app.poll_file_dialog(&ctx, None);
    app
}

/// The names of the open documents.
fn documents(app: &PhotocraftApp) -> Vec<String> {
    app.session.documents().iter().map(|d| d.doc.name.clone()).collect()
}

/// File › Open offers Image Sequence; the other open dialogs (Open as Smart Object, Place) don't.
#[test]
fn only_file_open_offers_image_sequence() {
    let (mut app, open, _) = app();
    let ctx = egui::Context::default();
    for (command, offered) in [("file.open", true), ("file.openAsSmartObject", false), ("file.placeEmbedded", false)] {
        menus::invoke(&mut app, &ctx, command, json!({})).unwrap();
        app.poll_file_dialog(&ctx, None);
        assert!(
            matches!(open.borrow().as_slice(), [(FileDialogRequest::Open { image_sequence, .. }, _)] if *image_sequence == offered),
            "{command}: {:?}",
            open.borrow().first().map(|(r, _)| r.clone())
        );
        answer(&open, None);
        app.poll_file_dialog(&ctx, None);
    }
}

/// Image Sequence ticked: nothing opens before the Frame Rate dialog is answered, which starts at
/// 30 fps; Cancel opens nothing.
#[test]
fn the_frame_rate_dialog_comes_first_and_cancel_opens_nothing() {
    let d = sequence_folder("cancel");
    let mut app = picked_with_image_sequence(&d, "frame001.png");
    let prompt = app.frame_rate.as_ref().expect("the Frame Rate dialog is open");
    assert_eq!((prompt.fps, prompt.preset()), (30.0, Some(30.0)));
    assert_eq!(documents(&app), ["Untitled"], "nothing opened yet");
    assert_eq!(app.answer_frame_rate(false), Err(CANCELLED.to_string()));
    assert!(app.frame_rate.is_none());
    assert_eq!(documents(&app), ["Untitled"]);
    assert!(!app.ui.status_error);
}

/// OK opens the run as the next Untitled-N (the app's own first document is "Untitled") with
/// Video Group 1 > Layer 1 at the rate picked, and shows the timeline.
#[test]
fn ok_opens_the_sequence_at_the_rate_picked() {
    let d = sequence_folder("ok");
    let mut app = picked_with_image_sequence(&d, "frame002.png");
    app.frame_rate.as_mut().unwrap().pick(Some(23.976));
    app.answer_frame_rate(true).unwrap();
    assert_eq!(documents(&app), ["Untitled", "Untitled-1"]);
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.layers[0].name, "Video Group 1");
    let layer = &st.doc.layers[0].children().unwrap()[0];
    assert_eq!(layer.name, "Layer 1");
    let video = layer.video.as_ref().unwrap();
    assert_eq!((video.frames.len(), video.fps), (3, 23.976));
    assert!(app.ui.timeline.open);
    assert!(app.frame_rate.is_none());
}

/// OK with a file whose name has no number gives Photoshop's alert and opens nothing.
#[test]
fn an_unnumbered_file_gives_the_alert() {
    let d = sequence_folder("flat");
    let mut app = picked_with_image_sequence(&d, "flat.png");
    let alert = "Could not complete the Open command because the file is not part of a numeric sequence.";
    assert_eq!(app.answer_frame_rate(true), Err(alert.to_string()));
    assert_eq!(app.ui.status, alert);
    assert!(app.ui.status_error);
    assert_eq!(documents(&app), ["Untitled"]);
}

/// The dialog drawn: Enter is OK, Escape is Cancel.
#[test]
fn enter_and_escape_answer_the_drawn_dialog() {
    let d = sequence_folder("keys");
    let key = |k: egui::Key| egui::RawInput {
        events: vec![egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }],
        ..Default::default()
    };
    let ctx = egui::Context::default();

    let mut app = picked_with_image_sequence(&d, "frame001.png");
    ctx.run_ui(egui::RawInput::default(), |ui| crate::file_dialog::show_frame_rate(&mut app, ui.ctx())).drop_without_applying_deltas();
    assert!(app.frame_rate.is_some(), "the dialog stays up until answered");
    ctx.run_ui(key(egui::Key::Escape), |ui| crate::file_dialog::show_frame_rate(&mut app, ui.ctx())).drop_without_applying_deltas();
    assert!(app.frame_rate.is_none());
    assert_eq!(documents(&app), ["Untitled"]);

    let mut app = picked_with_image_sequence(&d, "frame001.png");
    ctx.run_ui(key(egui::Key::Enter), |ui| crate::file_dialog::show_frame_rate(&mut app, ui.ctx())).drop_without_applying_deltas();
    assert!(app.frame_rate.is_none());
    assert_eq!(documents(&app), ["Untitled", "Untitled-1"]);
}

/// The combo box lists the measured rates in order, then Custom; a preset fills the field, a rate
/// typed in the field that isn't a preset shows as Custom, and Custom keeps the field's rate.
#[test]
fn frame_rate_presets_and_the_field_follow_each_other() {
    let labels: Vec<String> = FRAME_RATES.iter().map(|r| frame_rate_label(*r)).collect();
    assert_eq!(labels, ["10", "12", "12.5", "15", "23.976", "24", "25", "29.97", "30", "50", "59.94", "60", "Custom"]);
    let mut p = FrameRatePrompt::new("/f/frame001.png".into());
    p.pick(Some(59.94));
    assert_eq!((p.fps, p.preset()), (59.94, Some(59.94)));
    p.pick(None);
    assert_eq!((p.fps, p.preset()), (59.94, None), "Custom keeps the rate");
    p.set_fps(25.0);
    assert_eq!((p.fps, p.preset()), (25.0, Some(25.0)), "a preset typed in the field selects it");
    p.set_fps(17.5);
    assert_eq!((p.fps, p.preset()), (17.5, None), "any other rate is Custom");
    for bad in [0.0, -3.0, f32::NAN, f32::INFINITY] {
        p.set_fps(bad);
        assert_eq!(p.fps, 17.5, "{bad} is ignored");
    }
    p.set_fps(5000.0);
    assert_eq!(p.fps, 1000.0, "at most a video layer's 1000 fps");
}
