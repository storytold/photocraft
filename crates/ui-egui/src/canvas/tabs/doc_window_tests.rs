//! Document windows in their own OS windows (Window › Arrange › Float in Window, New Window for
//! Document, Open Documents as Tabs off). A stand-in for the window system runs each window's
//! frame on the harness's context with its own input, as eframe does.

use egui::{Event, Key, Modifiers, Pos2, Rect, ViewportId, pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;
use std::{cell::RefCell, collections::HashMap, rc::Rc};

use crate::PhotocraftApp;
use crate::state::Tool;

#[derive(Default)]
struct Host {
    /// Each window's desktop rect and its input for the next frame.
    windows: HashMap<ViewportId, (Rect, Vec<Event>)>,
    /// The window with the keyboard; `None` is the main window.
    focus: Option<ViewportId>,
    time: f64,
    /// The windows the app asked to take the keyboard (`ViewportCommand::Focus`), in order.
    focus_requests: Vec<ViewportId>,
}

impl Host {
    fn infos(&self) -> egui::ViewportIdMap<egui::ViewportInfo> {
        let info = |rect: Rect, focused: bool| egui::ViewportInfo {
            inner_rect: Some(rect),
            outer_rect: Some(rect),
            focused: Some(focused),
            native_pixels_per_point: Some(1.0),
            ..Default::default()
        };
        std::iter::once((ViewportId::ROOT, info(Rect::from_min_size(Pos2::ZERO, vec2(1400.0, 900.0)), self.focus.is_none())))
            .chain(self.windows.iter().map(|(id, (rect, _))| (*id, info(*rect, self.focus == Some(*id)))))
            .collect()
    }

    fn render(host: &RefCell<Self>, ctx: &egui::Context, mut viewport: egui::ImmediateViewport<'_>) {
        let id = viewport.ids.this;
        let input = {
            let mut host = host.borrow_mut();
            let rect = Rect::from_min_size(pos2(100.0, 100.0), viewport.builder.inner_size.unwrap_or(vec2(800.0, 600.0)));
            host.windows.entry(id).or_insert((rect, Vec::new()));
            let (viewports, focused, time) = (host.infos(), host.focus == Some(id), host.time);
            let (rect, events) = host.windows.get_mut(&id).expect("just added");
            egui::RawInput {
                viewport_id: id,
                viewports,
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, rect.size())),
                events: std::mem::take(events),
                focused,
                time: Some(time),
                ..Default::default()
            }
        };
        let mut full = ctx.run_ui(input, |ui| (viewport.viewport_ui_cb)(ui));
        full.textures_delta.clear();
        let mut host = host.borrow_mut();
        for command in full.viewport_output.get(&id).map(|output| output.commands.as_slice()).unwrap_or_default() {
            if let (egui::ViewportCommand::InnerSize(size), Some((rect, _))) = (command, host.windows.get_mut(&id)) {
                *rect = Rect::from_min_size(rect.min, *size);
            }
            // A command sent to this window reaches it in its own pass, as with eframe.
            if *command == egui::ViewportCommand::Focus {
                host.focus_requests.push(id);
            }
        }
    }
}

struct Desktop {
    h: Harness<'static, PhotocraftApp>,
    host: Rc<RefCell<Host>>,
}

impl Desktop {
    /// The app with a 400×300 document floating in its own window, which has the keyboard.
    fn new() -> (Self, ViewportId) {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.sync_views();
        let window = crate::view_cmds::float_window(&mut app, 0, 0, true);
        let host = Rc::new(RefCell::new(Host::default()));
        let shared = host.clone();
        egui::Context::set_immediate_viewport_renderer(move |ctx, viewport| Host::render(&shared, ctx, viewport));
        let mut ready = false;
        let h = Harness::builder().with_size(vec2(1400.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
            move |ui, app: &mut PhotocraftApp| {
                if !ready {
                    PhotocraftApp::setup_context(ui.ctx(), app.ui.theme);
                    ui.ctx().set_embed_viewports(false);
                    ready = true;
                    return;
                }
                crate::canvas::extra_windows(app, ui.ctx());
            },
            app,
        );
        let viewport = ViewportId::from_hash_of(("docwin", window));
        let mut desktop = Self { h, host };
        desktop.host.borrow_mut().focus = Some(viewport);
        desktop.steps(4);
        assert!(desktop.host.borrow().windows.contains_key(&viewport), "the document has its own window");
        (desktop, viewport)
    }

    fn steps(&mut self, n: usize) {
        for _ in 0..n {
            let (infos, main_focused) = {
                let mut host = self.host.borrow_mut();
                host.time += 1.0 / 60.0;
                (host.infos(), host.focus.is_none())
            };
            let input = self.h.input_mut();
            input.viewports = infos;
            input.focused = main_focused;
            self.h.step();
            let asked: Vec<ViewportId> =
                self.h.output().viewport_output.iter().filter(|(_, o)| o.commands.contains(&egui::ViewportCommand::Focus)).map(|(id, _)| *id).collect();
            self.host.borrow_mut().focus_requests.extend(asked);
        }
    }

    fn key(&mut self, viewport: ViewportId, key: Key, modifiers: Modifiers) {
        for pressed in [true, false] {
            let event = Event::Key { key, physical_key: None, pressed, repeat: false, modifiers };
            self.host.borrow_mut().windows.get_mut(&viewport).expect("a document window").1.push(event);
            self.steps(2);
        }
    }

    fn window_zoom(&self) -> f32 {
        self.h.state().ui.windows.first().expect("the document window").view.zoom
    }
}

#[test]
fn keys_pressed_in_a_document_window_run_the_shortcuts() {
    let (mut desktop, viewport) = Desktop::new();
    desktop.h.state_mut().ui.tool = Tool::Move;
    desktop.key(viewport, Key::B, Modifiers::NONE);
    assert_eq!(desktop.h.state().ui.tool, Tool::Brush, "B in the document's window picks the Brush");

    let layers = |desktop: &Desktop| desktop.h.state().session.active().unwrap().doc.layers.len();
    let before = layers(&desktop);
    desktop.h.state_mut().run("layer.new.layer", json!({})).unwrap();
    assert_eq!(layers(&desktop), before + 1);
    desktop.key(viewport, Key::Z, Modifiers::COMMAND);
    assert_eq!(layers(&desktop), before, "⌘Z in the document's window undoes");
}

#[test]
fn zoom_keys_in_a_document_window_zoom_that_window() {
    let (mut desktop, viewport) = Desktop::new();
    desktop.key(viewport, Key::Num1, Modifiers::COMMAND);
    assert_eq!(desktop.window_zoom(), 1.0, "⌘1 shows the window's document at 100%");
    desktop.key(viewport, Key::Equals, Modifiers::COMMAND);
    let zoomed = desktop.window_zoom();
    assert!(zoomed > 1.0, "⌘+ zooms the window in: {zoomed}");
    desktop.key(viewport, Key::Minus, Modifiers::COMMAND);
    assert_eq!(desktop.window_zoom(), 1.0, "⌘− zooms it back out");
}

#[test]
fn zoom_keys_resize_the_window_with_zoom_resizes_windows_on() {
    let (mut desktop, viewport) = Desktop::new();
    desktop.h.state_mut().run("prefs.set", json!({"path": "general.zoomResizesWindows", "value": true})).unwrap();
    desktop.key(viewport, Key::Num1, Modifiers::COMMAND);
    desktop.steps(2);
    let size = |desktop: &Desktop| desktop.host.borrow().windows[&viewport].0.size();
    let at_100 = size(&desktop);
    desktop.key(viewport, Key::Equals, Modifiers::COMMAND);
    desktop.steps(2);
    let zoomed = size(&desktop);
    assert!(zoomed.x > at_100.x && zoomed.y > at_100.y, "⌘+ grows the window around the image: {at_100:?} -> {zoomed:?}");
}

#[test]
fn a_document_window_without_the_keyboard_runs_no_shortcuts() {
    let (mut desktop, viewport) = Desktop::new();
    desktop.host.borrow_mut().focus = None;
    desktop.steps(2);
    desktop.h.state_mut().ui.tool = Tool::Move;
    desktop.key(viewport, Key::B, Modifiers::NONE);
    assert_eq!(desktop.h.state().ui.tool, Tool::Move, "keys a window without the keyboard gets run nothing");
}

/// Ctrl+Tab in a document's window makes the next document active and gives that document's
/// window the keyboard, so the next keys act on it (CodeRabbit on #11663).
#[test]
fn ctrl_tab_in_a_document_window_brings_the_next_document_forward() {
    let (mut desktop, first) = Desktop::new();
    desktop.h.state_mut().run("file.new", json!({"width": 300, "height": 200})).unwrap();
    desktop.h.state_mut().sync_views();
    let window = crate::view_cmds::float_window(desktop.h.state_mut(), 1, 0, true);
    let second = ViewportId::from_hash_of(("docwin", window));
    desktop.h.state_mut().session.set_active(0);
    desktop.steps(4);
    desktop.host.borrow_mut().focus_requests.clear();
    desktop.key(first, Key::Tab, Modifiers::CTRL);
    assert_eq!(desktop.h.state().session.active_index(), Some(1), "Ctrl+Tab makes the next document active");
    assert_eq!(desktop.host.borrow().focus_requests, [second], "its window takes the keyboard");
}

/// When the next document has a tab, the main window takes the keyboard.
#[test]
fn ctrl_tab_to_a_tabbed_document_gives_the_main_window_the_keyboard() {
    let (mut desktop, first) = Desktop::new();
    desktop.h.state_mut().run("file.new", json!({"width": 300, "height": 200})).unwrap();
    desktop.h.state_mut().sync_views();
    desktop.h.state_mut().session.set_active(0);
    desktop.steps(4);
    desktop.host.borrow_mut().focus_requests.clear();
    desktop.key(first, Key::Tab, Modifiers::CTRL);
    assert_eq!(desktop.h.state().session.active_index(), Some(1));
    assert_eq!(desktop.host.borrow().focus_requests, [ViewportId::ROOT], "the main window, with the tab, takes the keyboard");
}
