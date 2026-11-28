//! #16891: Window › Arrange keeps the document that was active when the command ran. A document window the app
//! opened itself takes the keyboard as it opens, and that keyboard is not the user's choice of document: the active
//! document stays active, and the window standing for it is the one the app brings forward. A window the user
//! brings forward, a tab torn off the strip and the layouts that only move panes keep the old rule.
//!
//! A stand-in for the window system runs each OS window's pass on the harness's context, as eframe does, and keeps
//! the keyboard where a window system would: with the newest window it opened, and with the window the app asks to
//! focus.

use std::cell::RefCell;
use std::rc::Rc;

use egui::{Pos2, Rect, Vec2, ViewportBuilder, ViewportCommand, ViewportId, ViewportIdMap, ViewportInfo, vec2};
use egui_kittest::Harness;
use photocraft_doc::DocId;
use serde_json::json;

use crate::PhotocraftApp;

/// The main window on the stand-in's desktop.
const MAIN: Vec2 = vec2(1000.0, 760.0);

/// The documents of the measured start state, tabbed in one strip: four sizes of their own, the first of them
/// active unless the test says otherwise.
const SIZES: [(u32, u32); 4] = [(400, 300), (300, 400), (250, 250), (500, 200)];

/// The app's own id of a document window, out of the title it opens with (`<name> - window <id>`).
fn app_window_of(builder: &ViewportBuilder) -> Option<u64> {
    builder.title.as_deref().and_then(|title| title.rsplit_once("window ")?.1.parse::<u64>().ok())
}

/// The OS windows the app opened and what the stand-in window system does with them.
#[derive(Default)]
struct Host {
    /// One entry per document window the system has seen: its viewport, the app's id for it, and where the system
    /// stands it, which is the place the app asked for.
    windows: Vec<(ViewportId, u64, Rect)>,
    /// The window the system holds the keyboard in. A window takes it as it opens, as it does when a layout opens a
    /// run of windows one after the other; the app's own focus request moves it, and so does the user's click.
    keyboard: Option<ViewportId>,
    /// The app's ids of the windows it asked the system to focus, in the order it asked.
    raised: Vec<u64>,
    time: f64,
}

impl Host {
    /// The window facts of the main window and of every document window, the keyboard in one of them. Each window
    /// stands where the app asked, so a window the app did not move stays where it opened.
    fn infos(&self) -> ViewportIdMap<ViewportInfo> {
        let info = |rect: Rect, focused: bool| ViewportInfo {
            inner_rect: Some(rect),
            outer_rect: Some(rect),
            focused: Some(focused),
            native_pixels_per_point: Some(1.0),
            ..Default::default()
        };
        let mut viewports: ViewportIdMap<ViewportInfo> =
            [(ViewportId::ROOT, info(Rect::from_min_size(Pos2::ZERO, MAIN), self.keyboard.is_none()))].into_iter().collect();
        for (viewport, _, place) in &self.windows {
            viewports.insert(*viewport, info(*place, self.keyboard == Some(*viewport)));
        }
        viewports
    }

    /// The system follows the app's request to focus a window it knows: the keyboard moves there.
    fn follow(&mut self, full: &egui::FullOutput) {
        for (viewport, output) in &full.viewport_output {
            if output.commands.contains(&ViewportCommand::Focus) {
                let asked = self.windows.iter().find(|(seen, _, _)| seen == viewport).map(|(_, window, _)| *window);
                self.raised.extend(asked);
                self.keyboard = Some(*viewport);
            }
        }
    }

    fn render(host: &RefCell<Self>, ctx: &egui::Context, mut viewport: egui::ImmediateViewport<'_>) {
        let id = viewport.ids.this;
        let size = viewport.builder.inner_size.unwrap_or(vec2(420.0, 340.0));
        let time = {
            let mut host = host.borrow_mut();
            host.time += 1.0 / 60.0;
            if !host.windows.iter().any(|(seen, _, _)| *seen == id) {
                let app_window = app_window_of(&viewport.builder).unwrap_or(host.windows.len() as u64);
                // The system opens the window where the app asked for it.
                let place = Rect::from_min_size(viewport.builder.position.unwrap_or(Pos2::new(30.0, 120.0)), size);
                host.windows.push((id, app_window, place));
                host.keyboard = Some(id);
            }
            host.time
        };
        let viewports = host.borrow().infos();
        let input = egui::RawInput {
            viewport_id: id,
            viewports,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            focused: true,
            time: Some(time),
            ..Default::default()
        };
        let mut full = ctx.run_ui(input, |ui| (viewport.viewport_ui_cb)(ui));
        full.textures_delta.clear();
        host.borrow_mut().follow(&full);
    }

    /// The viewport of the app's document window `window`, or `None` when the system never saw one.
    fn viewport_of(&self, window: u64) -> Option<ViewportId> {
        self.windows.iter().find(|(_, app_window, _)| *app_window == window).map(|(viewport, _, _)| *viewport)
    }

    /// The app's id of the window the system holds the keyboard in.
    fn keyboard_window(&self) -> Option<u64> {
        self.keyboard.and_then(|viewport| self.windows.iter().find(|(seen, _, _)| *seen == viewport).map(|(_, window, _)| *window))
    }
}

type App = Box<PhotocraftApp>;

/// The whole app with `documents` documents tabbed in one strip, `active` of them active, its document windows run
/// by the stand-in window system.
fn desktop(documents: usize, active: usize) -> (Harness<'static, App>, Rc<RefCell<Host>>) {
    let host = Rc::new(RefCell::new(Host::default()));
    let shared = host.clone();
    egui::Context::set_immediate_viewport_renderer(move |ctx, viewport| Host::render(&shared, ctx, viewport));
    let mut h = Harness::builder().with_size(MAIN).with_step_dt(1.0 / 60.0).with_max_steps(128).build_eframe(|cc| {
        use craft::window::policy::{CaptionFamily, Captions, Decision, Desktop, DesktopKind, Frame};
        let frame = Frame::Integrated(Captions { family: CaptionFamily::Windows, layout: craft::window::Layout::default() });
        let desktop = Desktop { kind: DesktopKind::Other, ssd: None, gtk_theme: None, icon_theme: None };
        let decision = Decision { frame, overridden: None, desktop };
        PhotocraftApp::setup_window(&cc.egui_ctx, crate::theme::AppTheme::pro(), decision, None);
        cc.egui_ctx.set_embed_viewports(false);
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), crate::Services::default());
        app.background_jobs = false;
        for (i, (width, height)) in SIZES.iter().take(documents).enumerate() {
            app.run("file.new", json!({"width": width, "height": height, "name": format!("doc {i}")})).unwrap();
        }
        app.sync_views();
        assert!(app.session.set_active(active.min(documents - 1)), "document {active} is open and becomes the active one");
        app
    });
    h.run_steps(4);
    (h, host)
}

/// Runs `n` frames with the main window's input listing every document window the system knows.
fn steps(h: &mut Harness<'static, App>, host: &RefCell<Host>, n: usize) {
    for _ in 0..n {
        let infos = host.borrow().infos();
        {
            let input = h.input_mut();
            input.viewports = infos;
            input.focused = host.borrow().keyboard.is_none();
        }
        h.step();
        let output = h.output();
        host.borrow_mut().follow(output);
    }
}

/// Runs one Window › Arrange command and lets the frames settle.
fn arrange(h: &mut Harness<'static, App>, host: &RefCell<Host>, command: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
    let ctx = h.ctx.clone();
    let out = crate::menus::invoke(h.state_mut(), &ctx, &format!("window.arrange.{command}"), params);
    steps(h, host, 4);
    out
}

/// The document a floating window shows.
fn shown(h: &Harness<'static, App>, window: u64) -> Option<DocId> {
    h.state()
        .ui
        .document_windows
        .floating
        .iter()
        .find(|(group, _)| group.id == window)
        .and_then(|(group, _)| group.shown.or_else(|| group.documents.first().copied()))
}

/// The document of the window the system holds the keyboard in.
fn keyboard_document(h: &Harness<'static, App>, host: &RefCell<Host>) -> Option<DocId> {
    shown(h, host.borrow().keyboard_window()?)
}

/// Float All in Windows opens a window for every document, and a window system takes the keyboard for the last one it
/// opened. The document that was active when the command ran is still active afterwards, and its own window is the
/// one the app brings forward, so the keyboard ends up there and stays there in the frames that follow.
#[test]
fn float_all_in_windows_keeps_the_active_document_and_puts_its_window_in_front() {
    let (mut h, host) = desktop(SIZES.len(), 0);
    let before = h.state().session.active_index().expect("a document is active");
    let active_id = h.state().session.documents()[before].doc.id;
    assert_eq!(h.state().ui.document_windows.docked.len(), 1, "the documents start tabbed in one strip");

    arrange(&mut h, &host, "floatAllInWindows", json!({})).unwrap();
    assert_eq!(h.state().ui.document_windows.floating.len(), SIZES.len(), "every document has a window of its own");
    assert_eq!(h.state().session.active_index(), Some(before), "the document that was active when the command ran is still active");
    assert_eq!(shown(&h, h.state().ui.document_windows.floating[0].0.id), Some(active_id), "the app lists the active document's window first");

    steps(&mut h, &host, 6);
    assert_eq!(h.state().session.active_index(), Some(before), "and it is still active after the frames that follow");
    assert_eq!(keyboard_document(&h, &host), Some(active_id), "and the window system holds the keyboard in that document's window");
}

/// The windows of the documents that were not active are not ones the app asks the system to focus: each such
/// request would pull the keyboard away from the active document again.
#[test]
fn float_all_in_windows_asks_the_system_to_focus_only_the_active_documents_window() {
    let (mut h, host) = desktop(SIZES.len(), 0);
    let active_id = h.state().session.documents()[0].doc.id;
    arrange(&mut h, &host, "floatAllInWindows", json!({})).unwrap();
    let windows = &h.state().ui.document_windows.floating;
    let want = windows.iter().find(|(group, _)| group.shown == Some(active_id)).map(|(group, _)| group.id).expect("the active document has a window");
    let asked = host.borrow().raised.clone();
    assert!(!asked.is_empty(), "the app asked the system to focus the active document's window");
    assert!(asked.iter().all(|window| *window == want), "every request was for that window: {asked:?}, and it is {want}");
}

/// A window the user brings forward is the user's choice: its document becomes the active one, as it did before,
/// whatever opened that window.
#[test]
fn a_document_window_the_user_brings_forward_makes_its_document_active() {
    let (mut h, host) = desktop(SIZES.len(), 0);
    arrange(&mut h, &host, "floatAllInWindows", json!({})).unwrap();
    let (first, second) = {
        let windows = &h.state().ui.document_windows.floating;
        (windows[0].0.id, windows[1].0.id)
    };
    let second_id = shown(&h, second).expect("the second window shows a document");
    let second_doc = h.state().session.documents().iter().position(|st| st.doc.id == second_id).expect("it is an open document");

    // The system moves the keyboard, as it does when the user clicks the window.
    let moved = host.borrow().viewport_of(second);
    host.borrow_mut().keyboard = moved;
    steps(&mut h, &host, 4);
    assert_eq!(h.state().session.active_index(), Some(second_doc), "the document of the window the user brought forward is the active one");

    let back = host.borrow().viewport_of(first);
    host.borrow_mut().keyboard = back;
    steps(&mut h, &host, 4);
    assert_eq!(h.state().session.active_index(), Some(0), "and the document of the window the user went back to");
}

/// Tile All Vertically and Horizontally, the n-up layouts and Consolidate All to Tabs move panes and tabs, not
/// windows: the active document is the same document afterwards.
#[test]
fn the_layouts_that_open_no_window_keep_the_active_document() {
    for layout in ["tileAllVertically", "tileAllHorizontally", "twoUpVertical", "twoUpHorizontal", "fourUp", "consolidateAllToTabs"] {
        let (mut h, host) = desktop(SIZES.len(), 0);
        let before = h.state().session.active_index();
        arrange(&mut h, &host, layout, json!({})).unwrap();
        assert_eq!(h.state().session.active_index(), before, "{layout} keeps the active document");
        assert!(h.state().ui.document_windows.floating.is_empty(), "{layout} opens no document window");
    }
}

/// A layout that floats every document, then Consolidate All to Tabs: the document that was active stays active, and
/// the one strip that takes the documents back shows it.
#[test]
fn consolidate_after_float_all_keeps_the_active_document_shown() {
    let (mut h, host) = desktop(SIZES.len(), 2);
    let before = h.state().session.active_index().expect("a document is active");
    let active_id = h.state().session.documents()[before].doc.id;
    arrange(&mut h, &host, "floatAllInWindows", json!({})).unwrap();
    assert_eq!(h.state().session.active_index(), Some(before), "Float All in Windows keeps it active");

    arrange(&mut h, &host, "consolidateAllToTabs", json!({})).unwrap();
    assert_eq!(h.state().session.active_index(), Some(before), "and Consolidate All to Tabs keeps it active");
    let shown = h.state().ui.document_windows.docked.first().and_then(|group| group.shown);
    assert_eq!(shown, Some(active_id), "and the one strip shows that document");
}

/// Float in Window with a document and a place, the path a tab torn off the strip runs: that document is the user's
/// choice and becomes the active one, and stays active while its new window opens and takes the keyboard.
#[test]
fn a_tab_torn_off_the_strip_still_makes_its_document_active() {
    let (mut h, host) = desktop(SIZES.len(), 0);
    let doc = h.state().session.documents()[2].doc.id;
    arrange(&mut h, &host, "floatInWindow", json!({"document": doc.0, "position": [640.0, 420.0]})).unwrap();
    assert_eq!(h.state().session.active_index(), Some(2), "the torn-off document is the active one");
    assert_eq!(shown(&h, window_holding(&h, doc)), Some(doc), "and it stands in a window of its own");

    steps(&mut h, &host, 6);
    assert_eq!(h.state().session.active_index(), Some(2), "and it is still the active one after the frames that follow");
    assert_eq!(keyboard_document(&h, &host), Some(doc), "with the keyboard in its own window");
}

/// The floating window that holds document `doc`.
fn window_holding(h: &Harness<'static, App>, doc: DocId) -> u64 {
    h.state()
        .ui
        .document_windows
        .floating
        .iter()
        .find(|(group, _)| group.documents.contains(&doc))
        .map(|(group, _)| group.id)
        .expect("the document has a window of its own")
}

/// A window the app opened for a document that was not active does not make that document active. A document the app
/// opens is another thing and still becomes the active one, beside the windows that already stand.
#[test]
fn a_document_opened_while_windows_stand_is_the_active_one_as_before() {
    let (mut h, host) = desktop(2, 0);
    arrange(&mut h, &host, "floatAllInWindows", json!({})).unwrap();
    h.state_mut().run("file.new", json!({"width": 200, "height": 200, "name": "later.png"})).unwrap();
    steps(&mut h, &host, 4);
    let docs = h.state().session.documents().len();
    assert_eq!(h.state().session.active_index(), Some(docs - 1), "the new document is the active one");
    assert_eq!(h.state().ui.document_windows.floating.len(), 2, "the two windows that stood keep standing");
    let new_id = h.state().session.documents()[docs - 1].doc.id;
    assert!(!crate::view_cmds::document_is_floating(h.state(), new_id), "the new document joins the strip, not a window of its own");
}
