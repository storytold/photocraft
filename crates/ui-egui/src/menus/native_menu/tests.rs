use std::cell::RefCell;
use std::rc::Rc;

use craft::menu::{Backend, CommandRef, Dispatch, Event, ItemRole, MenuBar, MenuRole, Node};
use egui_kittest::Harness;
use serde_json::json;

use super::*;

type Synced = Rc<RefCell<Vec<MenuBar>>>;
type Events = Rc<RefCell<Vec<Event>>>;

struct Fake {
    synced: Synced,
    events: Events,
}

impl Backend for Fake {
    fn sync(&mut self, bar: &MenuBar) {
        self.synced.borrow_mut().push(bar.clone());
    }
    fn drain(&mut self) -> Vec<Event> {
        std::mem::take(&mut *self.events.borrow_mut())
    }
    fn attach(&mut self, _dispatch: Dispatch) {}
}

fn app(doc: bool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    if doc {
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        app.sync_views();
    }
    app
}

/// The whole app on its own context, with a Mac menu over a fake backend.
fn harness(doc: bool) -> (Harness<'static, PhotocraftApp>, Synced, Events) {
    let (synced, events): (Synced, Events) = Default::default();
    let backend = Fake { synced: synced.clone(), events: events.clone() };
    let mut h = Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), crate::Services::default());
        if doc {
            app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
            app.sync_views();
        }
        app.services.native_menu = Some(NativeMenu::new(Box::new(backend)));
        app
    });
    h.run_steps(2);
    (h, synced, events)
}

/// A menu click: the event the backend reports, polled before the frame like the app's input hook does.
fn click(h: &mut Harness<'static, PhotocraftApp>, events: &Events, id: &str) {
    events.borrow_mut().push(Event::Click(CommandRef::new(id)));
    let ctx = h.ctx.clone();
    poll(h.state_mut(), &ctx);
    h.run_steps(3);
}

fn laid(app: &mut PhotocraftApp) -> Layout {
    let ctx = app.ctx().clone();
    laid_out(app, &ctx)
}

/// The ThemeMenu submenu lands in the app menu's Appearance, the current theme checked.
#[test]
fn themes_move_to_the_app_menus_appearance() {
    let mut a = app(false);
    // The menu's checked row is the saved choice, which picking a theme sets.
    let ctx = a.ctx().clone();
    let classic = crate::theme::AppTheme::classic().command();
    crate::menus::invoke(&mut a, &ctx, &classic.id, classic.params.clone()).unwrap();
    let layout = laid(&mut a);
    let app_menu = layout.bar.menu(MenuRole::App).unwrap();
    let appearance = app_menu.children.iter().find(|n| matches!(n, Node::Submenu { role: Some(ItemRole::Appearance), .. }));
    assert!(appearance.is_some(), "an Appearance submenu in the app menu");
    let checked = |c: CommandRef| layout.bar.find(&c).and_then(|it| it.kind.checked());
    assert_eq!(checked(crate::theme::AppTheme::classic().command()), Some(true));
    assert_eq!(checked(crate::theme::AppTheme::pro().command()), Some(false));
    let window = layout.bar.menu(MenuRole::Window).unwrap();
    assert!(window.children.iter().all(|n| !matches!(n, Node::Submenu { role: Some(ItemRole::Appearance), .. })), "Theme left the Window menu");
}

/// A Language row fires `app.language {lang}`: the preference is stored, the old `sr` id still
/// switches, and an unknown code is an error.
#[test]
fn the_app_menu_switches_the_ui_language() {
    // The offered languages are the embedded catalogs'.
    if !crate::i18n::needs_catalogs() {
        return;
    }
    let mut a = app(false);
    let ctx = a.ctx().clone();
    // PLAN: assumed the fired command is `app.language` with `{"lang": code}` (top-w12's `LanguageChoice` handling in `invoke`).
    let set = |a: &mut PhotocraftApp, code: &str| crate::menus::invoke(a, &ctx, "app.language", json!({"lang": code}));
    set(&mut a, "fr").unwrap();
    assert_eq!(a.session.prefs().interface.language, "fr");
    set(&mut a, "sr").unwrap();
    assert_eq!(a.session.prefs().interface.language, "sr-latn", "the old Serbian id still switches");
    set(&mut a, "auto").unwrap();
    assert_eq!(a.session.prefs().interface.language, "auto");
    assert!(set(&mut a, "xx-nope").is_err(), "an unknown code is an error");
}

/// #1354: a dialog is modal for the menu bar too. Its rows are greyed (view navigation aside), and
/// a click that arrives anyway doesn't run under the dialog.
#[test]
fn an_open_dialog_blocks_menu_clicks_except_view_navigation() {
    let (mut h, synced, events) = harness(true);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "image.imageSize", json!({})).unwrap();
    h.run_steps(2);
    assert!(h.state().dialog_open(), "Image Size opened its dialog");
    let size = |app: &PhotocraftApp| app.session.active().map(|d| (d.doc.size.width, d.doc.size.height));
    let zoom = h.state().ui.views[0].zoom;
    click(&mut h, &events, "image.imageRotation.90cw");
    click(&mut h, &events, "view.zoomIn");
    assert_eq!(size(h.state()), Some((64, 48)), "the rotation didn't run under the dialog");
    assert!(h.state().ui.views[0].zoom > zoom, "Zoom In still works under a dialog");
    let find = |id: &str| synced.borrow().last().and_then(|b| b.find(&CommandRef::new(id)).map(|it| it.enabled));
    assert_eq!(find("image.imageRotation.90cw"), Some(false));
    assert_eq!(find("view.zoomIn"), Some(true));
    // With the dialog closed the rows come back.
    let id = h.state().framework.dialogs.stack().top().unwrap();
    h.state_mut().framework.dialogs.cancel_by_id(&ctx, id);
    h.run_steps(3);
    assert_eq!(find("image.imageRotation.90cw"), Some(true));
}

fn type_layer(app: &mut PhotocraftApp, ctx: &egui::Context) -> photocraft_doc::LayerId {
    let id = photocraft_doc::LayerId(app.run("type.create", json!({"text": "Hello", "size": 12, "x": 4, "y": 24})).unwrap()["layer"].as_u64().unwrap());
    crate::menus::invoke(app, ctx, "type.editText", json!({})).unwrap();
    id
}

fn type_text(app: &PhotocraftApp, id: photocraft_doc::LayerId) -> String {
    match &app.session.active().unwrap().doc.layer(id).unwrap().content {
        photocraft_doc::LayerContent::Text(t) => t.text.clone(),
        _ => panic!("type layer changed kind"),
    }
}

/// Edit › Cut while editing type cuts the editor's selection (#1355): the click is fired `edit.cut`
/// and the focused editor takes it.
#[test]
fn cut_selected_type_from_native_menu_matches_keyboard_cut() {
    let (mut h, synced, events) = harness(true);
    let ctx = h.ctx.clone();
    let id = type_layer(h.state_mut(), &ctx);
    h.run_steps(2);
    let edit = h.state().ui.text_edit.as_ref().unwrap();
    assert_eq!((edit.anchor, edit.caret), (0, 5));
    assert_eq!(synced.borrow().last().unwrap().find(&CommandRef::new("edit.cut")).map(|it| it.enabled), Some(true), "Cut is enabled for selected characters");
    click(&mut h, &events, "edit.cut");
    assert!(type_text(h.state(), id).is_empty());
    let edit = h.state().ui.text_edit.as_ref().unwrap();
    assert_eq!((edit.anchor, edit.caret), (0, 0));
    assert!(h.state().session.clipboard.is_none(), "text Cut must not copy rendered pixels");
    assert_eq!(synced.borrow().last().unwrap().find(&CommandRef::new("edit.cut")).map(|it| it.enabled), Some(false), "Cut is disabled at an empty caret");
}

/// Select › All, Edit › Copy and Edit › Paste work on the text being typed, not on pixels (#2893).
#[test]
fn copy_and_paste_type_from_native_menu() {
    let (mut h, synced, events) = harness(true);
    let ctx = h.ctx.clone();
    let id = type_layer(h.state_mut(), &ctx);
    if let Some(e) = h.state_mut().ui.text_edit.as_mut() {
        (e.anchor, e.caret) = (5, 5);
    }
    let layers = h.state().session.active().unwrap().doc.layers.len();
    click(&mut h, &events, "select.all");
    let edit = h.state().ui.text_edit.as_ref().unwrap();
    assert_eq!((edit.anchor, edit.caret), (0, 5), "Select › All selects the characters");
    let enabled = |c: &str| synced.borrow().last().unwrap().find(&CommandRef::new(c)).map(|it| it.enabled);
    assert_eq!((enabled("edit.copy"), enabled("edit.paste")), (Some(true), Some(true)));
    click(&mut h, &events, "edit.copy");
    assert!(h.state().session.clipboard.is_none(), "text Copy must not copy rendered pixels");
    // The window answers Paste's request with the clipboard text; it replaces the selection.
    h.event(egui::Event::Paste("World".into()));
    h.run_steps(3);
    assert_eq!(type_text(h.state(), id), "World");
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), layers, "no pixel layer was pasted");
    assert!(!h.state().ui.status_error, "{}", h.state().ui.status);
}

/// Without a type edit, Cut is the pixel clipboard command.
#[test]
fn native_cut_without_type_edit_keeps_pixel_clipboard_behavior() {
    let (mut h, synced, events) = harness(false);
    h.state_mut().run("file.new", json!({"width": 4, "height": 4, "background": "transparent"})).unwrap();
    h.state_mut().run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    h.run_steps(2);
    assert_eq!(synced.borrow().last().unwrap().find(&CommandRef::new("edit.cut")).map(|it| it.enabled), Some(true));
    click(&mut h, &events, "edit.cut");
    assert_eq!(h.state_mut().run("document.pixel", json!({"x": 1, "y": 1})).unwrap(), json!([0.0, 0.0, 0.0, 0.0]));
    let clip = h.state().session.clipboard.as_ref().unwrap();
    assert_eq!((clip.bounds.width(), clip.bounds.height()), (4, 4));
    let mut px = vec![[0u8; 4]; 16];
    clip.surface.read_rgba8_into(clip.bounds, &mut px);
    assert_eq!(px, vec![[255u8, 0, 0, 255]; 16]);
    assert!(h.state().ui.text_edit.is_none());
}

#[test]
fn opening_or_closing_a_workspace_without_input_rebuilds_the_menus() {
    let mut app = app(true);
    let ctx = app.ctx().clone();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#808080"})).unwrap();
    let closed = state_hash(&app, None);
    crate::gallery_ui::open(&mut app).unwrap();
    let open = state_hash(&app, None);
    assert_ne!(open, closed, "the menus grey out when an agent opens Filter Gallery");
    let id = app.framework.dialogs.stack().top().unwrap();
    app.framework.dialogs.cancel_by_id(&ctx, id);
    assert_eq!(state_hash(&app, None), closed, "and come back when it is cancelled");
}
