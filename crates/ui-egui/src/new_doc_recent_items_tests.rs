//! #14038: File › New keeps each created document in Recent. The heading and the four-card grid are the shared
//! dialog's; the list itself is the engine's, so these tests go through the hosted dialog and the preferences.

use craft::behavior::PixelSize;
use photocraft_engine::Session;
use photocraft_engine::prefs::SavedPrefsExt;
use photocraft_engine::presets::PresetsExt;

use crate::new_doc_remember_tests::{AppHarness, app, harness, open, options, press_create, set_options};

/// Opens New Document, sets the size it shows and presses Create.
fn create(h: &mut AppHarness, width: u32, height: u32) {
    let id = open(h);
    let mut shown = options(h.state(), id);
    shown.size = PixelSize::new(width, height);
    set_options(h, id, &shown);
    assert!(press_create(h, id), "Create is enabled");
}

fn sizes(s: &Session) -> Vec<(u32, u32)> {
    s.presets().recent_documents.iter().map(|r| (r.width, r.height)).collect()
}

/// Two documents created alike through the dialog are two Recent items, newest first, also after a restart.
#[test]
fn documents_created_alike_stay_separate_recent_items() {
    let mut h = harness(app());
    for (width, height) in [(300, 200), (640, 480), (300, 200)] {
        create(&mut h, width, height);
    }
    assert_eq!(sizes(&h.state().session), [(300, 200), (640, 480), (300, 200)]);
    let mut restarted = Session::new();
    restarted.load_prefs_json(&h.state().session.prefs_to_json()).unwrap();
    assert_eq!(sizes(&restarted), [(300, 200), (640, 480), (300, 200)]);
    // The restarted dialog is handed both equal entries for its Recent cards.
    let listed: Vec<PixelSize> = restarted.presets().recent_documents.iter().map(|r| crate::new_doc_ui::to_options(r).size).collect();
    assert_eq!(listed, [PixelSize::new(300, 200), PixelSize::new(640, 480), PixelSize::new(300, 200)]);
}
