//! #14035: New Document shows Background Contents right after Color Mode and depth, then an
//! Advanced Options disclosure holding Color Profile; the depth entries read `8 bit`, `16 bit`,
//! `32 bit`.

use egui_kittest::{Harness, kittest::Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::{DialogKind, UiState};

fn harness() -> Harness<'static, PhotocraftApp> {
    let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(
        |ui, app| {
            crate::prefs_ui::tick(app, ui.ctx());
            crate::dialogs::show(app, ui.ctx());
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.state_mut().ui.open_dialog(DialogKind::NewDocument, UiState::new_document_fields());
    h.run_steps(3);
    h
}

/// The top of the first widget carrying `label`.
fn top(h: &Harness<'static, PhotocraftApp>, label: &str) -> f32 {
    h.query_all_by_label(label).map(|n| n.rect().min.y).fold(f32::INFINITY, f32::min)
}

fn count(h: &Harness<'static, PhotocraftApp>, label: &str) -> usize {
    h.query_all_by_label(label).count()
}

#[test]
fn background_contents_comes_before_advanced_options_and_color_profile() {
    let h = harness();
    let order = ["Color Mode", "Background Contents", "Advanced Options", "Color Profile"];
    let tops: Vec<f32> = order.iter().map(|l| top(&h, l)).collect();
    assert!(tops.iter().all(|y| y.is_finite()), "every label is shown: {tops:?}");
    assert!(tops.windows(2).all(|w| w[0] < w[1]), "{order:?} are in this order from the top: {tops:?}");
}

#[test]
fn collapsing_advanced_options_hides_color_profile_and_keeps_the_choice() {
    let mut h = harness();
    let nd = h.state().ui.dialogs[0].id;
    let profile = photocraft_cms::Builtin::AdobeRgbCompat.id().to_string();
    h.state_mut().ui.dialog_mut(nd).unwrap().fields.insert("profile".into(), json!(profile));
    h.run_steps(3);
    assert!(count(&h, "Color Profile") > 0, "expanded by default");

    h.get_by_label("Advanced Options").click();
    h.run_steps(3);
    assert_eq!(count(&h, "Color Profile"), 0, "collapsed: Color Profile is hidden");
    assert_eq!(h.state().ui.dialogs[0].fields.get("profile"), Some(&json!(profile)), "the profile is kept while collapsed");

    h.get_by_label("Advanced Options").click();
    h.run_steps(3);
    assert!(count(&h, "Color Profile") > 0, "expanded again: Color Profile is back");
    assert_eq!(h.state().ui.dialogs[0].fields.get("profile"), Some(&json!(profile)), "the same profile");
}

#[test]
fn an_unavailable_profile_is_dropped_while_advanced_options_is_collapsed() {
    // A saved preset can bring back a profile that is no longer installed: Create must not send
    // it, also when the disclosure holding Color Profile is collapsed (egui skips its body then).
    let mut h = harness();
    let nd = h.state().ui.dialogs[0].id;
    h.get_by_label("Advanced Options").click();
    h.run_steps(3);
    assert_eq!(count(&h, "Color Profile"), 0, "collapsed");
    h.state_mut().ui.dialog_mut(nd).unwrap().fields.insert("profile".into(), json!("no-such-installed-profile"));
    h.run_steps(3);
    assert_eq!(h.state().ui.dialogs[0].fields.get("profile"), None, "the unavailable profile is dropped");
}

#[test]
fn depth_labels_have_no_float_suffix() {
    let labels: Vec<&str> = crate::dialogs::new_doc_ui::DEPTH_OPTIONS.iter().map(|o| o.1).collect();
    assert_eq!(labels, ["8 bit", "16 bit", "32 bit"]);
    let values: Vec<u64> = crate::dialogs::new_doc_ui::DEPTH_OPTIONS.iter().map(|o| o.0).collect();
    assert_eq!(values, [8, 16, 32]);
}
