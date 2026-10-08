//! #1276: a document tab strip with more tabs than fit keeps the active document's tab on the
//! strip, shrinks and elides the others, and moves the rest into a » menu (whole app, real tree).

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::PhotocraftApp;
use crate::theme::ThemeKind;

use super::{TabStrip, pro_tab_title};

/// A real app window with `docs` documents named "ch1-pNN.png".
fn harness(docs: usize, theme: ThemeKind) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1000.0, 700.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, theme);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        // The theme is an interface preference, applied on the first frame: set the preference.
        app.run("prefs.set", json!({"values": {"interface.theme": theme.id()}})).unwrap();
        for n in 1..=docs {
            app.run("file.new", json!({"width": 400, "height": 300, "name": format!("ch1-p{n:02}.png")})).unwrap();
        }
        app
    });
    h.run_steps(4);
    h
}

fn strip(h: &Harness<'_, PhotocraftApp>) -> TabStrip {
    h.state().tab_strip.clone().expect("the strip is drawn")
}

fn has_menu(h: &Harness<'_, PhotocraftApp>) -> bool {
    h.query_by_label("More documents").is_some()
}

#[test]
fn overflowing_tabs_stay_on_the_strip_and_the_rest_move_to_the_menu() {
    let h = harness(14, ThemeKind::Pro);
    assert_eq!(h.state().session.documents().len(), 14);
    let s = strip(&h);
    assert!(s.tabs.len() < 14, "some tabs must overflow into the menu: {} shown", s.tabs.len());
    for r in &s.tabs {
        assert!(s.rect.contains_rect(*r), "a tab runs past the strip: {r:?} in {:?}", s.rect);
    }
    assert!(has_menu(&h), "the strip carries a » menu while tabs are hidden");
    // The active document is the first thing the fit keeps: its tab is on the strip, not behind
    // the menu, however many tabs are open (#1276).
    let title = pro_tab_title(h.state(), h.state().session.active_index().unwrap());
    assert!(h.query_by_label(&title).is_some(), "the active tab is on the strip: {title}");
}

#[test]
fn the_active_document_brings_its_tab_onto_the_strip() {
    let mut h = harness(14, ThemeKind::Pro);
    for pick in [0, 13, 7] {
        h.state_mut().session.set_active(pick);
        h.run_steps(2);
        let title = pro_tab_title(h.state(), pick);
        assert!(h.query_by_label(&title).is_some(), "tab {pick} is focused on the strip: {title}");
    }
}

#[test]
fn tabs_that_fit_keep_their_natural_widths_and_no_menu() {
    let h = harness(3, ThemeKind::Pro);
    let s = strip(&h);
    assert_eq!(s.tabs.len(), 3, "every tab stays on the strip when they fit");
    assert!(h.query_by_label("More documents").is_none(), "no overflow menu when the tabs fit");
    for i in 0..3 {
        assert!(h.query_by_label(&pro_tab_title(h.state(), i)).is_some(), "tab {i} is on the strip");
    }
}

#[test]
fn studio_tabs_overflow_into_the_same_menu() {
    let h = harness(14, ThemeKind::Studio);
    let s = strip(&h);
    assert!(s.tabs.len() < 14, "{} shown", s.tabs.len());
    for r in &s.tabs {
        assert!(s.rect.contains_rect(*r), "a tab runs past the strip: {r:?} in {:?}", s.rect);
    }
    assert!(has_menu(&h), "the Studio strip carries a » menu too");
    let active = h.state().session.active_index().unwrap();
    let name = h.state().session.documents()[active].doc.name.clone();
    assert!(h.query_by_label(&name).is_some(), "the active tab is on the strip: {name}");
}
