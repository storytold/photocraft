use std::time::Duration;

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::Document;
use photocraft_engine::prefs::{HomeRecentSort, HomeRecentView};
use photocraft_geom::Size;
use serde_json::json;

use crate::{PhotocraftApp, Services};

type State = (PhotocraftApp, bool);

fn harness(app: PhotocraftApp, size: egui::Vec2) -> Harness<'static, State> {
    let mut h = Harness::builder().with_size(size).build_ui_state(
        |ui, (app, ready): &mut State| {
            if *ready {
                egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| crate::canvas::document_area(app, ui));
                crate::dialogs::show(app, ui.ctx());
            }
        },
        (app, false),
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::default());
    h.state_mut().1 = true;
    h.run_steps(3);
    h
}

/// An app whose importer names the document after the file, as the shell's does.
fn app() -> PhotocraftApp {
    let services = Services {
        import: Some(Box::new(|name: &str, _bytes: &[u8], _opts: &photocraft_io::ImportOptions| {
            Ok((Document::new(name, Size::new(4, 4), ColorMode::Rgb, SampleType::U8), Vec::new()))
        })),
        ..Default::default()
    };
    PhotocraftApp::new(photocraft_engine::Session::new(), services)
}

/// Files `(name, length, opened)` in a fresh folder, listed in Recent with their times.
fn with_files(files: &[(&str, usize, u64)]) -> (PhotocraftApp, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app();
    for (name, len, opened) in files {
        let path = dir.path().join(name);
        std::fs::write(&path, vec![b'x'; *len]).unwrap();
        let path = path.to_string_lossy().into_owned();
        app.push_recent(&path);
        app.session.prefs.edit(|p| p.file_handling.recent_opened.insert(path, *opened));
    }
    (app, dir)
}

/// Frames until the worker has looked at every listed file.
fn settle(h: &mut Harness<'static, State>) {
    for _ in 0..500 {
        h.run_steps(1);
        let app = &h.state().0;
        if app.ui.recent_files.iter().all(|p| app.recent_view.status(p).is_some()) {
            h.run_steps(3);
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("the recent files were not looked at");
}

/// The names that are on screen, in reading order (top to bottom, then left to right).
fn order(h: &Harness<'static, State>, names: &[&str]) -> Vec<String> {
    let mut found: Vec<(i32, i32, String)> = names
        .iter()
        .filter_map(|n| h.query_by_label(n).map(|node| (node.rect().top().round() as i32, node.rect().left().round() as i32, n.to_string())))
        .collect();
    found.sort();
    found.into_iter().map(|(_, _, n)| n).collect()
}

fn prefs_of(h: &Harness<'static, State>) -> (HomeRecentView, HomeRecentSort, bool) {
    let i = &h.state().0.session.prefs().interface;
    (i.home_recent_view, i.home_recent_sort, i.home_recent_reverse)
}

const FILES: [(&str, usize, u64); 3] = [("a.txt", 300, 100), ("b.txt", 100, 300), ("c.txt", 200, 200)];

#[test]
fn the_view_toggles_switch_between_grid_and_list_and_are_remembered() {
    let (app, _dir) = with_files(&FILES);
    let mut h = harness(app, vec2(1360.0, 800.0));
    settle(&mut h);
    assert_eq!(prefs_of(&h).0, HomeRecentView::Grid, "a grid by default");
    assert!(h.query_by_label("Last Opened").is_none(), "no column headers in a grid");
    h.get_by_label("List view").click();
    h.run_steps(3);
    assert_eq!(prefs_of(&h).0, HomeRecentView::List);
    for header in ["Name", "Last Opened", "Size", "Kind"] {
        assert!(h.query_by_label(header).is_some(), "`{header}` header in the list");
    }
    assert!(h.query_by_label("b.txt").is_some());
    // The choice is in the preferences, so it survives a restart and `prefs.get` reports it.
    let saved = h.state().0.session.prefs_to_json();
    let mut restored = photocraft_engine::Session::new();
    restored.load_prefs_json(&saved).unwrap();
    assert_eq!(restored.prefs().interface.home_recent_view, HomeRecentView::List);
    h.get_by_label("Grid view").click();
    h.run_steps(3);
    assert_eq!(prefs_of(&h).0, HomeRecentView::Grid);
    assert!(h.query_by_label("Last Opened").is_none());
    // An agent can drive it the same way.
    h.state_mut().0.run("prefs.set", json!({"values": {"interface.homeRecentView": "list"}})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label("Last Opened").is_some());
}

#[test]
fn sorting_by_a_header_or_the_arrow_reorders_and_reverses() {
    let (app, _dir) = with_files(&FILES);
    let mut h = harness(app, vec2(1360.0, 800.0));
    h.state_mut().0.session.prefs.edit(|p| p.interface.home_recent_view = HomeRecentView::List);
    settle(&mut h);
    let names = ["a.txt", "b.txt", "c.txt"];
    // Most recently opened first by default.
    assert_eq!(order(&h, &names), ["b.txt", "c.txt", "a.txt"]);
    assert_eq!(prefs_of(&h), (HomeRecentView::List, HomeRecentSort::Recent, false));
    // A header sorts by it; again, it reverses.
    h.get_by_label("Name").click();
    h.run_steps(3);
    assert_eq!(order(&h, &names), ["a.txt", "b.txt", "c.txt"]);
    assert_eq!(prefs_of(&h), (HomeRecentView::List, HomeRecentSort::Name, false));
    h.get_by_label("Name").click();
    h.run_steps(3);
    assert_eq!(order(&h, &names), ["c.txt", "b.txt", "a.txt"]);
    assert_eq!(prefs_of(&h), (HomeRecentView::List, HomeRecentSort::Name, true));
    // Another header starts in its own natural order.
    h.get_by_label("Size").click();
    h.run_steps(3);
    assert_eq!(order(&h, &names), ["a.txt", "c.txt", "b.txt"], "largest first");
    assert_eq!(prefs_of(&h), (HomeRecentView::List, HomeRecentSort::Size, false));
    h.get_by_label("Last Opened").click();
    h.run_steps(3);
    assert_eq!(order(&h, &names), ["b.txt", "c.txt", "a.txt"]);
    // The arrow beside Sort flips the order, and the same sort stays.
    h.get_by_label("Reverse sort order").click();
    h.run_steps(3);
    assert_eq!(order(&h, &names), ["a.txt", "c.txt", "b.txt"]);
    assert_eq!(prefs_of(&h), (HomeRecentView::List, HomeRecentSort::Recent, true));
    // The Sort menu is set through the preferences too: Kind falls back to the name.
    h.state_mut().0.run("prefs.set", json!({"values": {"interface.homeRecentSort": "kind", "interface.homeRecentReverse": false}})).unwrap();
    h.run_steps(3);
    assert_eq!(order(&h, &names), ["a.txt", "b.txt", "c.txt"]);
}

#[test]
fn the_filter_matches_names_and_says_when_nothing_does() {
    let (app, _dir) = with_files(&FILES);
    let mut h = harness(app, vec2(1360.0, 800.0));
    settle(&mut h);
    h.state_mut().0.ui.home_recent_filter = "B".into();
    h.run_steps(3);
    assert!(h.query_by_label("b.txt").is_some(), "case-insensitive");
    assert!(h.query_by_label("a.txt").is_none() && h.query_by_label("c.txt").is_none());
    assert!(h.query_by_label_contains("No recent files match").is_none());
    h.state_mut().0.ui.home_recent_filter = "zzz".into();
    h.run_steps(3);
    assert!(h.query_by_label("b.txt").is_none());
    assert!(h.query_by_label("No recent files match “zzz”").is_some());
    assert_eq!(h.state().0.ui.recent_files.len(), 3, "the filter hides, it does not remove");
    // The filter is not a preference.
    assert!(!h.state().0.session.prefs_to_json().contains("zzz"));
    // Typing in the field sets it.
    h.state_mut().0.ui.home_recent_filter.clear();
    h.run_steps(3);
    h.get_by_label("Filter Recent Files").click();
    h.run_steps(2);
    h.get_by_label("Filter Recent Files").type_text("c.t");
    h.run_steps(3);
    assert_eq!(h.state().0.ui.home_recent_filter, "c.t");
    assert_eq!(order(&h, &["a.txt", "b.txt", "c.txt"]), ["c.txt"]);
}

#[test]
fn clicking_a_card_opens_the_file_and_leaves_the_home_screen() {
    let (app, _dir) = with_files(&FILES);
    let mut h = harness(app, vec2(1360.0, 800.0));
    settle(&mut h);
    h.get_by_label("b.txt").click();
    h.run_steps(4);
    let app = &h.state().0;
    assert_eq!(app.session.documents().len(), 1);
    assert_eq!(app.session.active().map(|s| s.doc.name.clone()).as_deref(), Some("b.txt"));
    assert_eq!(app.ui.recent_files.first().map(|p| crate::file_open::display_name(p)).as_deref(), Some("b.txt"));
    assert!(h.query_by_label("Welcome to PhotoCraft").is_none(), "the document replaced the Home screen");
}

#[test]
fn the_x_removes_a_card_in_the_grid_and_a_row_in_the_list() {
    let (app, _dir) = with_files(&FILES);
    let mut h = harness(app, vec2(1360.0, 800.0));
    settle(&mut h);
    assert!(h.query_by_label("Remove a.txt from Recent").is_none(), "only the hovered card has one");
    h.get_by_label("a.txt").hover();
    h.run_steps(2);
    h.get_by_label("Remove a.txt from Recent").click();
    h.run_steps(3);
    assert_eq!(h.state().0.ui.recent_files.len(), 2);
    assert!(h.query_by_label("a.txt").is_none());
    assert!(h.state().0.session.documents().is_empty(), "removing does not open");
    h.state_mut().0.session.prefs.edit(|p| p.interface.home_recent_view = HomeRecentView::List);
    h.run_steps(3);
    h.get_by_label("b.txt").hover();
    h.run_steps(2);
    h.get_by_label("Remove b.txt from Recent").click();
    h.run_steps(3);
    assert_eq!(h.state().0.ui.recent_files.len(), 1);
    assert!(h.state().0.session.prefs().file_handling.recent_opened.keys().all(|k| k.ends_with("c.txt")), "its time goes with it");
}

#[test]
fn the_x_is_reachable_from_the_keyboard() {
    let (app, _dir) = with_files(&FILES);
    let mut h = harness(app, vec2(1360.0, 800.0));
    settle(&mut h);
    assert!(h.query_by_label("Remove a.txt from Recent").is_none());
    h.get_by_label("a.txt").focus();
    h.run_steps(3);
    let x = h.get_by_label("Remove a.txt from Recent");
    x.focus();
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(h.state().0.ui.recent_files.len(), 2, "Enter on the focused × removes the file");
    assert!(h.state().0.session.documents().is_empty(), "and does not open it");
}

#[test]
fn the_context_menu_offers_open_show_remove_and_clear() {
    let (app, _dir) = with_files(&FILES);
    let mut h = harness(app, vec2(1360.0, 800.0));
    settle(&mut h);
    assert_eq!(h.query_all_by_label("Open").count(), 1, "just the rail's button");
    h.get_by_label("c.txt").click_secondary();
    h.run_steps(3);
    assert_eq!(h.query_all_by_label("Open").count(), 2, "and the menu's");
    assert!(h.query_by_label(crate::menus::reveal_label()).is_some());
    assert!(h.query_by_label("Remove from Recent").is_some());
    h.get_by_label("Clear Recent Files").click();
    h.run_steps(3);
    assert!(h.state().0.ui.recent_files.is_empty());
    assert!(h.state().0.session.prefs().file_handling.recent_files.is_empty());
    assert!(h.state().0.session.prefs().file_handling.recent_opened.is_empty(), "the times go too");
    assert!(h.query_by_label("Your recent files will appear here").is_some());
    assert!(h.query_by_label("Open or create a file to get started.").is_some());
}

#[test]
fn a_recent_file_list_set_to_zero_files_says_so_and_points_to_file_handling() {
    let (app, _dir) = with_files(&FILES);
    let mut h = harness(app, vec2(1360.0, 800.0));
    h.state_mut().0.run("prefs.set", json!({"values": {"fileHandling.recentFileCount": 0}})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label("The recent file list is set to 0 files").is_some());
    assert!(h.query_by_label("a.txt").is_none());
    assert_eq!(h.state().0.session.prefs().file_handling.recent_files.len(), 3, "the stored list is kept");
    h.get_by_label("Open Preferences").click();
    h.run_steps(3);
    let section = h.state().0.ui.dialogs.iter().find(|d| crate::prefs_ui::is_preferences(&d.fields)).and_then(|d| d.fields.get("section")).cloned();
    assert_eq!(section, Some(json!("fileHandling")));
    // With nothing stored, it is the plain empty list.
    let mut empty = harness(PhotocraftApp::new(photocraft_engine::Session::new(), Services::default()), vec2(1360.0, 800.0));
    empty.state_mut().0.run("prefs.set", json!({"values": {"fileHandling.recentFileCount": 0}})).unwrap();
    empty.run_steps(3);
    assert!(empty.query_by_label("The recent file list is set to 0 files").is_none());
    assert!(empty.query_by_label("Your recent files will appear here").is_some());
}

#[test]
fn switching_the_recent_files_off_says_so_and_points_to_interface() {
    let (app, _dir) = with_files(&FILES);
    let mut h = harness(app, vec2(1360.0, 800.0));
    h.state_mut().0.run("prefs.set", json!({"values": {"interface.showHomeRecentFiles": false}})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label("Recent files are hidden").is_some());
    assert!(h.query_by_label("Your list is kept. Turn it back on in Preferences › Interface.").is_some());
    assert!(h.query_by_label("a.txt").is_none() && h.query_by_label("Recent").is_none());
    assert_eq!(h.state().0.ui.recent_files.len(), 3, "the list is kept");
    h.get_by_label("Open Preferences").click();
    h.run_steps(3);
    let section = h.state().0.ui.dialogs.iter().find(|d| crate::prefs_ui::is_preferences(&d.fields)).and_then(|d| d.fields.get("section")).cloned();
    assert_eq!(section, Some(json!("interface")));
}

#[test]
fn a_missing_file_is_flagged_and_asks_before_it_is_dropped() {
    let (mut app, dir) = with_files(&[("here.txt", 10, 200)]);
    let gone = dir.path().join("gone").join("ghost.psd").to_string_lossy().into_owned();
    app.push_recent(&gone);
    let mut h = harness(app, vec2(1360.0, 800.0));
    settle(&mut h);
    assert_eq!(h.state().0.recent_view.status(&gone), Some(crate::recent_thumbs::FileStat::Missing));
    assert!(matches!(h.state().0.recent_view.status(&h.state().0.ui.recent_files[1]), Some(crate::recent_thumbs::FileStat::Present { .. })));
    // Clicking it asks, and opens nothing.
    h.get_by_label("ghost.psd").click();
    h.run_steps(4);
    assert!(h.state().0.session.documents().is_empty());
    assert!(h.query_by_label("Could not find “ghost.psd”").is_some());
    let folder = dir.path().join("gone").to_string_lossy().into_owned();
    assert!(h.query_by_label(&format!("It may have been moved, renamed or deleted. It was last at {folder}.")).is_some());
    // Cancel keeps it.
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(h.state().0.ui.dialogs.is_empty());
    assert!(h.state().0.ui.recent_files.contains(&gone));
    // Remove from Recent drops it, without touching anything else.
    h.get_by_label("ghost.psd").click();
    h.run_steps(4);
    h.get_by_label("Remove from Recent").click();
    h.run_steps(3);
    assert!(!h.state().0.ui.recent_files.contains(&gone));
    assert!(!h.state().0.session.prefs().file_handling.recent_files.contains(&gone));
    assert_eq!(h.state().0.ui.recent_files.len(), 1);
}

#[test]
fn recent_times_follow_the_list() {
    let mut app = app();
    let before = crate::recent_thumbs::unix_now();
    app.push_recent("/a/one.psd");
    app.push_recent("/a/two.psd");
    let opened = |app: &PhotocraftApp| app.session.prefs().file_handling.recent_opened.clone();
    let times = opened(&app);
    assert_eq!(times.keys().map(String::as_str).collect::<Vec<_>>(), ["/a/one.psd", "/a/two.psd"]);
    assert!(times.values().all(|&t| t >= before && t <= crate::recent_thumbs::unix_now()));
    // Opening a listed file again updates its time only.
    app.session.prefs.edit(|p| p.file_handling.recent_opened.insert("/a/one.psd".into(), 5));
    app.push_recent("/a/two.psd");
    assert_eq!(opened(&app).get("/a/one.psd"), Some(&5));
    // Remove, clear.
    app.remove_recent("/a/one.psd");
    assert_eq!(opened(&app).keys().collect::<Vec<_>>(), ["/a/two.psd"]);
    app.push_recent("/a/three.psd");
    app.clear_recent();
    assert!(opened(&app).is_empty());
    assert!(app.session.prefs().file_handling.recent_files.is_empty());
    // A shorter list drops the times of what falls off it, and a list of none keeps none.
    app.session.prefs.edit(|p| p.file_handling.recent_file_count = 2);
    for name in ["x", "y", "z"] {
        app.push_recent(&format!("/a/{name}.psd"));
    }
    assert_eq!(opened(&app).keys().map(String::as_str).collect::<Vec<_>>(), ["/a/y.psd", "/a/z.psd"]);
    app.session.prefs.edit(|p| p.file_handling.recent_file_count = 0);
    app.push_recent("/a/w.psd");
    assert!(opened(&app).is_empty());
    // A time that was never recorded is no time: the list falls back to the file's modified time.
    assert!(!app.session.prefs().file_handling.recent_opened.contains_key("/a/never.psd"));
}

#[test]
fn the_rail_stacks_on_top_in_a_narrow_window_and_stands_beside_the_page_in_a_wide_one() {
    let wide = harness(app(), vec2(1200.0, 800.0));
    let new_file = wide.get_by_label("New file").rect();
    let welcome = wide.get_by_label_contains("Welcome to PhotoCraft").rect();
    assert!(new_file.right() < welcome.left(), "beside the page: {new_file:?} {welcome:?}");
    assert!((new_file.width() - 158.0).abs() < 1.0, "the button fills the rail: {new_file:?}");
    let narrow = harness(app(), vec2(400.0, 800.0));
    let new_file = narrow.get_by_label("New file").rect();
    let welcome = narrow.get_by_label_contains("Welcome to PhotoCraft").rect();
    assert!(new_file.bottom() <= welcome.top(), "on top: {new_file:?} {welcome:?}");
    assert!(new_file.left() >= 0.0 && new_file.right() <= 400.0);
    for r in [narrow.get_by_label("Open").rect(), narrow.get_by_label("Join us on Discord").rect()] {
        assert!(r.left() >= 0.0 && r.right() <= 400.0, "clipped: {r:?}");
    }
}

#[test]
fn the_drop_hint_stays_on_the_home_screen() {
    let h = harness(app(), vec2(1200.0, 800.0));
    assert!(h.query_by_label_contains("Drop an image or PSD anywhere").is_some());
    let wayland = harness(PhotocraftApp::new(photocraft_engine::Session::new(), Services { is_wayland: true, ..Default::default() }), vec2(1200.0, 800.0));
    assert!(wayland.query_by_label_contains("Use File › Open").is_some());
}

#[test]
fn cards_fill_the_row_with_as_many_columns_as_fit() {
    let files: Vec<(String, usize, u64)> = (0..7).map(|i| (format!("f{i}.txt"), 10, 100 + i as u64)).collect();
    let refs: Vec<(&str, usize, u64)> = files.iter().map(|(n, l, t)| (n.as_str(), *l, *t)).collect();
    let (app, _dir) = with_files(&refs);
    let mut h = harness(app, vec2(1360.0, 900.0));
    settle(&mut h);
    let names: Vec<&str> = refs.iter().map(|f| f.0).collect();
    let tops: Vec<i32> = names.iter().filter_map(|n| h.query_by_label(n)).map(|n| n.rect().top().round() as i32).collect();
    let first_row = tops.iter().filter(|&&y| y == *tops.iter().min().unwrap()).count();
    assert_eq!(tops.len(), 7);
    // 1360 − the 190 rail − 72 of padding = 1098 points of cards at least 190 wide with a 22 gap.
    assert_eq!(first_row, 5);
    let narrow_page = harness(with_files(&refs).0, vec2(700.0, 900.0));
    let tops: Vec<i32> = names.iter().filter_map(|n| narrow_page.query_by_label(n)).map(|n| n.rect().top().round() as i32).collect();
    assert!(tops.iter().filter(|&&y| y == tops[0]).count() <= 2, "{tops:?}");
}

/// A path a preference change takes off the list loses its allowed thumbnail seed; one that stays
/// keeps it.
#[test]
fn a_path_dropped_by_a_preference_change_loses_its_thumbnail_seed() {
    let (mut app, dir) = with_files(&[("a.txt", 1, 1), ("b.txt", 1, 2)]);
    app.services.thumbnail_cache_dir = Some(dir.path().join("cache"));
    app.sync_recent();
    let listed = app.ui.recent_files.clone();
    assert_eq!(listed.len(), 2);
    let doc = std::sync::Arc::new(Document::new("d", Size::new(4, 4), ColorMode::Rgb, SampleType::U8));
    for p in &listed {
        app.recent_view.seed(p, doc.clone(), app.services.thumbnail_cache_dir.clone());
        assert!(app.recent_view.is_allowed(p));
    }
    let kept = listed[0].clone();
    let dropped = listed[1].clone();
    app.session.prefs.edit(|p| p.file_handling.recent_files.retain(|r| *r != dropped));
    app.sync_recent();
    assert_eq!(app.ui.recent_files, vec![kept.clone()]);
    assert!(!app.recent_view.is_allowed(&dropped), "the dropped path's seed is revoked");
    assert!(app.recent_view.is_allowed(&kept));
    app.run("prefs.set", json!({"values": {"fileHandling.recentFileCount": 0}})).unwrap();
    app.sync_recent();
    assert!(app.ui.recent_files.is_empty());
    assert!(!app.recent_view.is_allowed(&kept));
}
