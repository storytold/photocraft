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

/// A full history of 100 files on a desktop-sized window: most of the cards are below the fold.
fn full_history() -> H {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.background_jobs = false;
    app.run("prefs.set", serde_json::json!({"values": {"fileHandling.recentFileCount": 100}})).unwrap();
    for i in 1..=100 {
        app.push_recent(&format!("/tmp/photocraft-12234/full{i:03}.psd"));
    }
    let mut h = Harness::builder().with_size(egui::vec2(900.0, 700.0)).with_context(app.ctx().clone()).build_ui_state(
        |ui, (app, ready): &mut (Box<PhotocraftApp>, bool)| {
            if *ready {
                egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| crate::canvas::document_area(app, ui));
            }
        },
        (app, false),
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::default());
    h.state_mut().1 = true;
    h
}

fn statted(h: &H) -> Vec<String> {
    let app = &h.state().0;
    let cache = app.thumbnail_cache.lock().unwrap();
    cache.statted().into_iter().map(str::to_owned).collect()
}

/// Home lists the whole history, but a pass stats only a few files on the UI thread, newest (the cards in view)
/// first; the rest follow in later passes.
#[test]
fn home_stats_a_bounded_number_of_recent_files_per_pass() {
    let mut h = full_history();
    h.step();
    let first = statted(&h);
    assert!(!first.is_empty() && first.len() <= 2 * crate::home_recent_ui::STATS_PER_PASS, "statted {} of 100 in the first frame", first.len());
    assert!(first.iter().any(|p| p.ends_with("full100.psd")), "the newest card, in view, is statted first: {first:?}");
    assert!(!first.iter().any(|p| p.ends_with("full001.psd")), "the oldest card, far below the fold, waits");
    h.run_steps(20);
    assert_eq!(statted(&h).len(), 100, "later passes stat the rest");
    assert_eq!((1..=100).filter(|i| h.query_by_label(&format!("full{i:03}.psd")).is_some()).count(), 100, "all 100 are listed");
}
