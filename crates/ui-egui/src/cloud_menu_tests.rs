//! The File menu's cloud items appear only when a backend is connected.
//!
//! The menu is the one surface a person sees before anything else, so the gate is tested through
//! [`crate::menus::base_rows`] itself rather than through the predicate it calls. An item greyed
//! out for ever would be a promise the application cannot keep, and an item missing with a service
//! configured would hide the feature.

use crate::cloud::{self, CloudBackend, Job, Post};
use crate::{PhotocraftApp, Services};
use photocraft_engine::Session;

/// A backend that accepts every job and answers nothing. Enough to make [`cloud::available`] true,
/// which is all the menu reads.
struct Connected;

impl CloudBackend for Connected {
    fn submit(&self, _job: Job, _post: Post) {}
}

fn app() -> PhotocraftApp {
    PhotocraftApp::new(Session::new(), Services::default())
}

fn ids(app: &PhotocraftApp) -> Vec<String> {
    crate::menus::base_rows(app).into_iter().map(|row| row.command.id).collect()
}

#[test]
fn no_service_means_no_cloud_items_at_all() {
    let app = app();
    let ids = ids(&app);
    assert!(ids.iter().any(|id| id == "file.open"), "the ordinary File menu is still there");
    for id in ["cloud.open", "cloud.saveAs"] {
        assert!(!ids.iter().any(|i| i == id), "{id} must not be in the menu with no cloud service");
    }
}

#[test]
fn a_connected_backend_puts_them_beside_open_and_save_as() {
    let mut app = app();
    app.cloud.connect(Connected);
    let ids = ids(&app);
    let at = |want: &str| ids.iter().position(|id| id == want);
    let open = at("file.open").expect("File › Open is in the menu");
    let cloud_open = at("cloud.open").expect("Open from the Cloud is in the menu");
    assert_eq!(cloud_open, open + 1, "Open from the Cloud follows Open…");
    if let Some(save_as) = at("file.saveAs") {
        let cloud_save = at("cloud.saveAs").expect("Save to the Cloud is in the menu");
        assert_eq!(cloud_save, save_as + 1, "Save to the Cloud follows Save As…");
    }
}

/// Each row takes the path of the row it sits after, never a path spelled in the cloud module. A
/// written-out path matches no menu node under any language but English, which is how the plug-in
/// rows once went missing (#2540).
#[test]
fn the_rows_take_the_path_of_the_item_they_follow() {
    let mut app = app();
    app.cloud.connect(Connected);
    let rows = crate::menus::base_rows(&app);
    let path_of = |id: &str| rows.iter().find(|row| row.command.id == id).map(|row| row.path.clone());
    assert_eq!(path_of("cloud.open"), path_of("file.open"), "Open from the Cloud sits in Open's own menu");
    if path_of("file.saveAs").is_some() {
        assert_eq!(path_of("cloud.saveAs"), path_of("file.saveAs"), "Save to the Cloud sits in Save As's own menu");
    }
}

#[test]
fn the_items_are_live_commands_so_the_parity_report_counts_them() {
    for id in ["cloud.open", "cloud.saveAs", "cloud.save", "cloud.status"] {
        assert!(crate::menus::is_live(id), "{id} must be a live command");
    }
    assert!(!crate::menus::is_live("cloud.nonsense"));
}

#[test]
fn save_to_the_cloud_is_greyed_until_a_document_is_open() {
    let mut app = app();
    app.cloud.connect(Connected);
    // Open is available at once; saving needs something to save.
    assert_eq!(cloud::is_enabled(&app, "cloud.open"), Some(true));
    assert_eq!(cloud::is_enabled(&app, "cloud.saveAs"), Some(false));
    let row = crate::menus::base_rows(&app).into_iter().find(|row| row.command.id == "cloud.saveAs").expect("the item is in the menu");
    assert!(!row.enabled, "with no document open it is greyed, not hidden");
}
