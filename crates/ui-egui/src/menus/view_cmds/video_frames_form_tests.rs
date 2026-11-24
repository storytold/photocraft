//! #9040: File › Import › Video Frames to Layers for a movie asks which frames: all of them, or a
//! range, optionally only every Nth.
// Movie files are read only by the desktop build.
#![cfg(not(target_arch = "wasm32"))]

use craft::ui::dialog::DialogId;
use photocraft_io::movie::testgen::{Container, h264, level};
use serde_json::json;

use crate::PhotocraftApp;

fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 40, "height": 24})).unwrap();
    app
}

fn movie(name: &str, frames: usize) -> String {
    let p = std::env::temp_dir().join(format!("pc-ui-frames-{}-{name}.mp4", std::process::id()));
    std::fs::write(&p, h264(Container::Mp4, 40, 24, frames, 25).unwrap()).unwrap();
    p.to_string_lossy().into_owned()
}

/// Opens the form for `path`, after the frame that shows it.
fn open(app: &mut PhotocraftApp, path: &str) -> DialogId {
    let ctx = app.ctx().clone();
    let reply = super::dialogs::open_video_frames_form(app, path).unwrap();
    let want = reply["dialog"].as_str().expect("the reply names a dialog").to_owned();
    let id = app.framework.dialogs.open_dialogs().into_iter().map(|d| d.id).find(|id| id.value().to_string() == want).expect("the dialog is open");
    ctx.run_ui(Default::default(), |ui| crate::dialogs::run(app, ui)).textures_delta.clear();
    id
}

fn layers(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.layers.len()
}

#[test]
fn all_frames_by_default() {
    let path = movie("all", 6);
    let mut app = app();
    let ctx = app.ctx().clone();
    let before = layers(&app);
    let id = open(&mut app, &path);
    let fields = app.framework.dialogs.open_dialogs().into_iter().find(|d| d.id == id).and_then(|d| d.fields).unwrap();
    assert_eq!(fields["range"], "all");
    assert_eq!(fields["to"], 5, "the range ends at the movie's last frame");
    crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
    assert_eq!(layers(&app), before + 6, "status: {}", app.ui.status);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_range_and_a_step_pick_those_frames() {
    let path = movie("range", 20);
    let mut app = app();
    let ctx = app.ctx().clone();
    let before = layers(&app);
    let id = open(&mut app, &path);
    // Each choice opens the rows that follow it, as a click would before the next one.
    for (key, value) in [("range", json!("selected")), ("from", json!(3)), ("to", json!(12)), ("limit", json!(true)), ("every", json!(3))] {
        app.framework.dialogs.set_json_field(&ctx, id, key, value).unwrap();
        ctx.run_ui(Default::default(), |ui| crate::dialogs::run(&mut app, ui)).textures_delta.clear();
    }
    crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
    let doc = &app.session.active().unwrap().doc;
    assert_eq!(doc.layers.len(), before + 4, "frames 3, 6, 9 and 12");
    match &doc.layers.last().unwrap().content {
        photocraft_doc::LayerContent::Raster(r) => assert!((r.pixel(5, 5)[0] - level(12)).abs() < 4.0 / 255.0, "the top layer is frame 12"),
        _ => panic!("frames become pixel layers"),
    }
    let _ = std::fs::remove_file(&path);
}
