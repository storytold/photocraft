//! #14333: a menu with items hidden in Edit › Menus ends with a separator and Show All Menu
//! Items; choosing it reopens the menu with every item shown, and a ⌘-click (Ctrl+click) on the
//! title does the same, for that open only.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::PhotocraftApp;
use crate::menus::{MenuItem, SHOW_ALL_ITEM, menu_items, menu_items_showing};

/// A document, with Edit › Fade…, File › New… and Edit › Transform › Skew hidden.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app.sync_views();
    app.session.edit_prefs(|p| p.menus.hidden = vec!["edit.fade".into(), "file.new".into(), "edit.transform.skew".into()]);
    app
}

/// The rows of the menu at `path` itself (not its submenus).
fn rows<'a>(items: &'a [MenuItem], path: &[&str]) -> Vec<&'a MenuItem> {
    items.iter().filter(|i| i.path.iter().map(String::as_str).eq(path.iter().copied())).collect()
}

/// The ids under the top-level menu `top`, in order, submenus included.
fn under(items: &[MenuItem], top: &str) -> Vec<String> {
    items.iter().filter(|i| i.path.first().is_some_and(|p| p == top)).map(|i| i.id.clone()).collect()
}

fn show_all(path: &str) -> String {
    format!("{SHOW_ALL_ITEM}{path}")
}

#[test]
fn a_menu_with_hidden_items_ends_with_show_all_menu_items() {
    let app = app();
    let items = menu_items(&app);
    for (path, last) in [(&["Edit"][..], "Edit"), (&["File"][..], "File"), (&["Edit", "Transform"][..], "Edit › Transform")] {
        let r = rows(&items, path);
        let n = r.len();
        assert!(n >= 2, "{path:?}");
        assert_eq!((r[n - 2].id.as_str(), r[n - 1].id.as_str()), ("---", show_all(last).as_str()), "{path:?} ends with a separator and the row");
        assert_eq!(r[n - 1].label, "Show All Menu Items");
        assert!(r[n - 1].enabled);
    }
    // Edit's row comes after its submenus, at the very end of the menu.
    assert_eq!(under(&items, "Edit").last().map(String::as_str), Some(show_all("Edit").as_str()));
    // The hidden items are gone; a menu without hidden items has no row.
    for hidden in ["edit.fade", "file.new", "edit.transform.skew"] {
        assert!(!items.iter().any(|i| i.id == hidden), "{hidden}");
    }
    for top in ["Image", "Layer", "Select", "Filter", "View", "Window", "Help"] {
        assert!(!under(&items, top).iter().any(|id| id.starts_with(SHOW_ALL_ITEM)), "{top}");
    }
    // Nothing hidden: no row anywhere.
    let mut plain = app;
    plain.session.edit_prefs(|p| p.menus.hidden.clear());
    assert!(!menu_items(&plain).iter().any(|i| i.id.starts_with(SHOW_ALL_ITEM)));
}

#[test]
fn showing_all_of_a_menu_shows_its_hidden_items_without_the_row() {
    let app = app();
    let all = menu_items_showing(&app, Some("Edit"));
    let edit = under(&all, "Edit");
    assert!(edit.contains(&"edit.fade".to_string()) && edit.contains(&"edit.transform.skew".to_string()));
    assert!(!edit.iter().any(|id| id.starts_with(SHOW_ALL_ITEM)), "no row in Edit");
    // Other menus keep theirs.
    assert!(!under(&all, "File").contains(&"file.new".to_string()));
    assert!(under(&all, "File").contains(&show_all("File")));
    // The saved menu set isn't touched.
    assert_eq!(app.session.prefs().menus.hidden, ["edit.fade", "file.new", "edit.transform.skew"]);
}

#[test]
fn separators_still_collapse_and_hidden_items_keep_their_shortcuts() {
    let app = app();
    let items = menu_items(&app);
    // Separators collapse when drawn, as before: the row's own one follows the menu's last row.
    for path in [&["Edit"][..], &["File"][..], &["Edit", "Transform"][..]] {
        assert_ne!(rows(&items, path).first().map(|i| i.id.as_str()), Some("---"), "{path:?}");
    }
    // Fade's ⇧⌘F still runs it.
    let fade = crate::shortcuts::parse("Cmd+Shift+F").unwrap();
    assert!(crate::shortcut_dispatch::bindings(&app).iter().any(|(id, sc)| id == "edit.fade" && *sc == fade));
}

#[test]
fn agents_see_the_row_and_invoking_it_lists_the_hidden_items() {
    let mut app = app();
    let ctx = egui::Context::default();
    let (req, _rx) = crate::control::ControlRequest::new("ui.menu.list", json!({}));
    let crate::control::Outcome::Done(listed) = crate::control::handle(&mut app, &ctx, &req) else { panic!("ui.menu.list replies at once") };
    assert!(listed.to_string().contains(&show_all("Edit")), "ui.menu.list reports the row");
    let r = crate::menus::invoke(&mut app, &ctx, &show_all("Edit"), json!({})).unwrap();
    assert_eq!(r["menu"], json!(["Edit"]));
    let shown: Vec<&str> = r["items"].as_array().unwrap().iter().filter_map(|i| i["id"].as_str()).collect();
    assert!(shown.contains(&"edit.fade") && shown.contains(&"edit.transform.skew"), "{shown:?}");
    assert!(!shown.iter().any(|id| id.starts_with(SHOW_ALL_ITEM)));
    assert!(crate::menus::invoke(&mut app, &ctx, SHOW_ALL_ITEM, json!({})).is_err(), "no menu named");
}

/// The menu bar in a window tall enough for all of Edit, so its last row is in view.
fn harness() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1280.0, 2400.0)).build_ui_state(
        |ui, app| {
            crate::menus::menu_bar(app, ui);
        },
        app(),
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(3);
    h
}

/// A row whose label holds `label` (a row with a key carries it after the label).
fn shows(h: &Harness<'static, PhotocraftApp>, label: &str) -> bool {
    h.query_all_by_label_contains(label).next().is_some()
}

fn close(h: &mut Harness<'static, PhotocraftApp>) {
    h.key_press(egui::Key::Escape);
    h.run_steps(4);
}

#[test]
fn choosing_the_row_reopens_the_menu_with_every_item_for_that_open() {
    let mut h = harness();
    h.get_by_label("Edit").click();
    h.run_steps(4);
    assert!(shows(&h, "Show All Menu Items") && !shows(&h, "Fade…"));
    h.get_by_label("Show All Menu Items").click();
    h.run_steps(4);
    assert!(shows(&h, "Fade…"), "the menu is open again with Fade…");
    assert!(!shows(&h, "Show All Menu Items"), "and without the row");
    assert_eq!(h.state().session.prefs().menus.hidden.len(), 3, "the saved set is unchanged");
    close(&mut h);
    assert!(!shows(&h, "Fade…"));
    h.get_by_label("Edit").click();
    h.run_steps(4);
    assert!(!shows(&h, "Fade…") && shows(&h, "Show All Menu Items"), "the next plain open hides them again");
}

#[test]
fn a_command_click_on_the_title_shows_every_item_once() {
    let mut h = harness();
    h.get_by_label("Edit").click_modifiers(egui::Modifiers::COMMAND);
    h.run_steps(4);
    assert!(shows(&h, "Fade…"), "every item");
    assert!(!shows(&h, "Show All Menu Items"), "no row");
    close(&mut h);
    h.get_by_label("Edit").click();
    h.run_steps(4);
    assert!(!shows(&h, "Fade…") && shows(&h, "Show All Menu Items"), "for that open only");
}

struct Bar {
    synced: std::rc::Rc<std::cell::RefCell<Vec<crate::native_menu::MenuBar>>>,
    events: std::rc::Rc<std::cell::RefCell<Vec<crate::native_menu::Event>>>,
}

impl crate::native_menu::Backend for Bar {
    fn sync(&mut self, bar: &crate::native_menu::MenuBar) {
        self.synced.borrow_mut().push(bar.clone());
    }
    fn drain(&mut self) -> Vec<crate::native_menu::Event> {
        std::mem::take(&mut *self.events.borrow_mut())
    }
}

#[test]
fn the_macos_menu_bar_shows_the_menu_in_full_until_another_item_runs() {
    let synced = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let mut app = app();
    app.services.native_menu = Some(crate::native_menu::NativeMenu::new(Box::new(Bar { synced: synced.clone(), events: events.clone() })));
    let ctx = egui::Context::default();
    let frame = |app: &mut PhotocraftApp, click: Option<&str>| {
        if let Some(id) = click {
            events.borrow_mut().push(crate::native_menu::Event::Click(id.to_string()));
        }
        let mut raw = egui::RawInput::default();
        if let Some(menu) = app.services.native_menu.as_mut() {
            menu.raw_input(&mut raw);
        }
        crate::native_menu::run(app, &ctx);
        crate::native_menu::sync(app, &ctx);
    };
    let has = |id: &str| synced.borrow().last().is_some_and(|bar| bar.find(id).is_some());
    frame(&mut app, None);
    assert!(has(&show_all("Edit")) && !has("edit.fade"), "the row, not Fade…");
    frame(&mut app, Some(&show_all("Edit")));
    assert!(has("edit.fade") && !has(&show_all("Edit")), "Edit in full, no row");
    assert!(has(&show_all("File")), "File as set");
    frame(&mut app, Some("view.extras"));
    assert!(!has("edit.fade") && has(&show_all("Edit")), "another item chosen: hidden again");
}
