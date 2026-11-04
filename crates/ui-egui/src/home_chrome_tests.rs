//! The Home screen is only the menu bar and the Home content: the tool strip, options bar, status
//! bar and docks are not drawn under it, and come back when Home is dismissed. The saved panel
//! preferences are never touched by showing Home.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::PhotocraftApp;
use crate::theme::ThemeKind;

/// The real app (every panel on) in `theme`, with a document open when `document`.
fn app(theme: ThemeKind, document: bool, auto_show: bool) -> Harness<'static, PhotocraftApp> {
    let mut s = photocraft_engine::Session::new();
    if document {
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    }
    s.edit_prefs(|p| p.general.auto_show_home_screen = auto_show);
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, theme);
        let mut app = PhotocraftApp::new(s, crate::Services::default());
        app.ui.theme = theme;
        app
    });
    h.run_steps(6);
    h
}

fn chrome_shown(h: &Harness<'static, PhotocraftApp>) -> bool {
    let toolbar = h.query_by_label("Brush Tool (B)").is_some();
    let dock = h.query_by_label("Create a new layer").is_some();
    assert_eq!(toolbar, dock, "the tool strip and the dock come and go together");
    toolbar
}

/// Click the Home button nearest the top: the header's (or the options bar's), not the rail's row.
fn click_home(h: &mut Harness<'static, PhotocraftApp>) {
    let top = h.query_all_by_label("Home").min_by(|a, b| a.rect().min.y.total_cmp(&b.rect().min.y)).expect("a Home button");
    top.click();
    h.run_steps(6);
}

#[test]
fn no_documents_with_auto_show_draws_only_the_home_content() {
    let h = app(ThemeKind::Pro, false, true);
    assert!(h.query_by_label("New file").is_some(), "Home content");
    assert!(!chrome_shown(&h), "no tool strip or dock under Home");
    assert!(h.query_by_label("No document").is_none(), "no status bar");
    assert_eq!(h.query_all_by_label("Home").count(), 1, "only the rail's entry: nothing to go back to");
}

#[test]
fn toggling_home_swaps_the_chrome_and_keeps_the_panel_prefs() {
    for theme in ThemeKind::ALL {
        let mut h = app(theme, true, true);
        let panels = h.state().ui.panels.clone();
        assert!(chrome_shown(&h), "{theme:?}: chrome with a document");
        click_home(&mut h);
        assert!(h.query_by_label("New file").is_some(), "{theme:?}: Home content");
        assert!(!chrome_shown(&h), "{theme:?}: no chrome on Home");
        assert!(h.query_all_by_label("Home").count() >= 2, "{theme:?}: the header's Home toggle and the rail's entry");
        assert_eq!(h.state().ui.panels, panels, "{theme:?}: showing Home leaves the saved panels alone");
        click_home(&mut h);
        assert!(h.query_by_label("New file").is_none(), "{theme:?}: Home dismissed");
        assert!(chrome_shown(&h), "{theme:?}: chrome back");
        assert_eq!(h.state().ui.panels, panels);
    }
}
