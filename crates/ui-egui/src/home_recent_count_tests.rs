//! #12234: the Home screen lists as many recent files as Preferences › File Handling › Recent File
//! List Contains keeps, like File › Open Recent, not only the first 11.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use crate::PhotocraftApp;

type H = Harness<'static, (Box<PhotocraftApp>, bool)>;

/// The Home screen with 15 recent files, newest (`file15.psd`) first. The framework start screen
/// is drawn through `canvas::document_area`, as in the app.
fn home() -> H {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.background_jobs = false;
    for i in 1..=15 {
        app.push_recent(&format!("/tmp/photocraft-12234/file{i:02}.psd"));
    }
    let mut h = Harness::builder().with_size(egui::vec2(900.0, 1400.0)).with_context(app.ctx().clone()).build_ui_state(
        |ui, (app, ready): &mut (Box<PhotocraftApp>, bool)| {
            if *ready {
                egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| crate::canvas::document_area(app, ui));
            }
        },
        (app, false),
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::default());
    h.state_mut().1 = true;
    h.run_steps(3);
    h
}

/// How many of the 15 files have a row; a row is named by its file name.
fn listed(h: &H) -> usize {
    (1..=15).filter(|i| h.query_by_label(&format!("file{i:02}.psd")).is_some()).count()
}

#[test]
fn home_recent_list_follows_the_recent_file_count() {
    let mut h = home();
    assert_eq!(listed(&h), 15, "all 15 under the default of 20");
    assert!(h.query_by_label("file01.psd").is_some(), "the oldest is listed too");

    h.state_mut().0.run("prefs.set", serde_json::json!({"values": {"fileHandling.recentFileCount": 12}})).unwrap();
    // The app's frame mirrors the stored list into the UI state; this harness draws only the
    // document area.
    h.state_mut().0.sync_recent();
    h.run_steps(3);
    assert_eq!(listed(&h), 12, "the preference caps the list");
    assert!(h.query_by_label("file03.psd").is_none(), "the oldest drop off first");
}
