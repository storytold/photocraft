//! File › Open's Image Sequence option and Frame Rate dialog (#14057).

use super::*;
use craft::ui::dialog::DialogId;

/// A fresh folder holding the measured sequence: `frame001.png` .. `frame003.png` (40 x 30) and an
/// unnumbered `flat.png`. It is removed when the guard drops, a failed test included.
fn sequence_folder() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30, "background": "#ff0000"})).unwrap();
    let png = photocraft_io::export(&s.active().unwrap().doc, "png", &photocraft_io::ExportOptions::default()).unwrap().bytes;
    for file in ["frame001.png", "frame002.png", "frame003.png", "flat.png"] {
        std::fs::write(d.join(file), &png).unwrap();
    }
    dir
}

/// File › Open answered with `files` of `folder`, Image Sequence ticked or not.
fn opened(folder: &Path, files: &[&str], ticked: bool) -> Box<PhotocraftApp> {
    let (mut app, dialogs, _) = app();
    let ctx = egui::Context::default();
    menus::invoke(&mut app, &ctx, "file.open", json!({})).unwrap();
    let picked = Picked::Paths(files.iter().map(|f| folder.join(f)).collect());
    if ticked {
        dialogs.answer_checked(picked).expect("a dialog is open");
    } else {
        answer(&dialogs, picked);
    }
    app.poll_file_dialog();
    crate::dialogs::testing::show(&mut app);
    app
}

/// The open Frame Rate dialog.
fn frame_rate_dialog(app: &PhotocraftApp) -> Option<DialogId> {
    let id = dialog_id(FRAME_RATE_KEY);
    app.framework.dialogs.get_by_id::<HostedParamDialog>(id).map(|_| id)
}

/// Sets `field` of the Frame Rate dialog as `ui.dialog.set` does.
fn set(app: &mut PhotocraftApp, field: &str, value: Value) {
    let ctx = app.ctx().clone();
    let id = frame_rate_dialog(app).expect("the Frame Rate dialog is open");
    app.framework.dialogs.set_json_field(&ctx, id, field, value).unwrap();
}

/// The names of the open documents.
fn documents(app: &PhotocraftApp) -> Vec<String> {
    app.session.documents().iter().map(|d| d.doc.name.clone()).collect()
}

/// File › Open's picker offers Image Sequence; the other open pickers (Open as Smart Object, Place) don't.
#[test]
fn only_file_open_offers_image_sequence() {
    let (mut app, files, _) = app();
    let ctx = egui::Context::default();
    for (command, offered) in [("file.open", Some("Image Sequence")), ("file.openAsSmartObject", None), ("file.placeEmbedded", None)] {
        menus::invoke(&mut app, &ctx, command, json!({})).unwrap();
        assert_eq!(last_request(&files).check_box.as_deref(), offered, "{command}");
        cancel(&files);
        app.poll_file_dialog();
    }
}

/// Image Sequence ticked with one file: nothing opens before the Frame Rate dialog is answered,
/// which starts at 30 fps; Cancel opens nothing.
#[test]
fn the_frame_rate_dialog_comes_first_and_cancel_opens_nothing() {
    let d = sequence_folder();
    let mut app = opened(d.path(), &["frame001.png"], true);
    let id = frame_rate_dialog(&app).expect("the Frame Rate dialog is open");
    let fields = app.framework.dialogs.get_by_id::<HostedParamDialog>(id).unwrap().fields().clone();
    assert_eq!((fields.text("rate"), frame_rate(&fields)), (Some("30"), Some(30.0)));
    assert_eq!(documents(&app), ["Untitled"], "nothing opened yet");
    crate::dialogs::testing::cancel(&mut app, id).unwrap();
    assert!(frame_rate_dialog(&app).is_none());
    assert_eq!(documents(&app), ["Untitled"]);
    assert!(!app.ui.status_error);
}

/// OK opens the run as the next Untitled-N (the app's own first document is "Untitled") with
/// Video Group 1 > Layer 1 at the rate picked, and shows the timeline.
#[test]
fn ok_opens_the_sequence_at_the_rate_picked() {
    let d = sequence_folder();
    let mut app = opened(d.path(), &["frame002.png"], true);
    set(&mut app, "rate", json!("23.976"));
    crate::dialogs::testing::confirm(&mut app, dialog_id(FRAME_RATE_KEY)).unwrap();
    assert_eq!(documents(&app), ["Untitled", "Untitled-1"]);
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.layers[0].name, "Video Group 1");
    let layer = &st.doc.layers[0].children().unwrap()[0];
    assert_eq!(layer.name, "Layer 1");
    let video = layer.video.as_ref().unwrap();
    assert_eq!((video.frames.len(), f64::from(video.fps)), (3, f64::from(23.976_f32)));
    assert!(crate::panel_docking::is_shown(&app, "timeline"));
    assert!(frame_rate_dialog(&app).is_none());
}

/// Custom takes the rate typed in the field.
#[test]
fn custom_takes_the_typed_rate() {
    let d = sequence_folder();
    let mut app = opened(d.path(), &["frame001.png"], true);
    set(&mut app, "rate", json!("custom"));
    set(&mut app, "fps", json!(17.5));
    crate::dialogs::testing::confirm(&mut app, dialog_id(FRAME_RATE_KEY)).unwrap();
    let st = app.session.active().unwrap();
    let video = st.doc.layers[0].children().unwrap()[0].video.as_ref().unwrap();
    assert_eq!(video.fps, 17.5);
}

/// Unticked, or ticked with several files picked, no Frame Rate dialog shows: the files open on
/// their own, as File › Open does without the option.
#[test]
fn without_the_tick_or_with_several_files_no_frame_rate_dialog_shows() {
    let d = sequence_folder();
    for (files, ticked) in [(&["frame001.png"][..], false), (&["frame001.png", "flat.png"][..], true)] {
        let app = opened(d.path(), files, ticked);
        assert!(frame_rate_dialog(&app).is_none(), "{files:?} ticked {ticked}");
        assert!(!app.file_dialog_open());
    }
}

/// OK with a file whose name has no number gives Photoshop's alert and opens nothing.
#[test]
fn an_unnumbered_file_gives_the_alert() {
    let d = sequence_folder();
    let mut app = opened(d.path(), &["flat.png"], true);
    crate::dialogs::testing::confirm(&mut app, dialog_id(FRAME_RATE_KEY)).unwrap();
    let alert = "Could not complete the Open command because the file is not part of a numeric sequence.";
    assert_eq!(app.ui.status, alert);
    assert!(app.ui.status_error);
    assert_eq!(documents(&app), ["Untitled"]);
}

/// The list holds the measured rates in order, then Custom; a preset is its rate, Custom the typed
/// one within what a video layer plays, and anything else no rate.
#[test]
fn the_frame_rate_list_and_custom() {
    let labels: Vec<String> = FRAME_RATES.iter().map(|r| rate_id(*r)).collect();
    assert_eq!(labels, ["10", "12", "12.5", "15", "23.976", "24", "25", "29.97", "30", "50", "59.94", "60"]);
    let rate = |id: &str, fps: f64| {
        let mut fields = DialogFields::default();
        fields.set("rate", id);
        fields.set("fps", fps);
        frame_rate(&fields)
    };
    assert_eq!(rate("59.94", 17.5), Some(59.94), "a preset ignores the field");
    assert_eq!(rate("custom", 17.5), Some(17.5));
    assert_eq!(rate("custom", 5000.0), Some(1000.0), "at most a video layer's 1000 fps");
    for bad in [0.0, -3.0, f64::NAN, f64::INFINITY] {
        assert_eq!(rate("custom", bad), None, "{bad}");
    }
    assert_eq!(rate("31", 17.5), None, "not a listed rate");
}

/// While the Frame Rate dialog waits, File › Open (from a shortcut behind it) is refused and the
/// sequence picked stays.
#[test]
fn file_open_waits_for_the_frame_rate_dialog() {
    let d = sequence_folder();
    let mut app = opened(d.path(), &["frame002.png"], true);
    let ctx = egui::Context::default();
    assert_eq!(menus::invoke(&mut app, &ctx, "file.open", json!({})), Err(FRAME_RATE_OPEN.to_string()));
    assert!(!app.file_dialog_open());
    assert_eq!(crate::dialogs::params::captured(&app, FRAME_RATE_KEY), json!(d.path().join("frame002.png").to_str().unwrap()));
}
