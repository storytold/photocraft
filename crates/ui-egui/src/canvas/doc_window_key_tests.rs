//! Shortcuts typed in a document's own OS window (Window › Arrange › New Window for Document, a
//! floating document). A stand-in for the window system runs each window's frame on the
//! harness's context with its own input, as eframe does: keys go only to the focused window.

use super::extra_windows;
use crate::PhotocraftApp;
use crate::state::{DocWindow, Tool};
use egui::{Event, Key, Modifiers, Pos2, Rect, ViewportId, vec2};
use egui_kittest::Harness;
use serde_json::json;
use std::{cell::RefCell, collections::HashMap, rc::Rc};

#[derive(Default)]
struct Host {
    /// Events waiting for each document window's next frame.
    pending: HashMap<ViewportId, Vec<Event>>,
    /// The window with the keyboard; `None` is the main window.
    focus: Option<ViewportId>,
    /// Frames each document window ran.
    frames: HashMap<ViewportId, usize>,
    time: f64,
}

impl Host {
    fn infos(&self, size: egui::Vec2) -> egui::ViewportIdMap<egui::ViewportInfo> {
        let info = |focused: bool| egui::ViewportInfo {
            inner_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            focused: Some(focused),
            native_pixels_per_point: Some(1.0),
            ..Default::default()
        };
        std::iter::once((ViewportId::ROOT, info(self.focus.is_none()))).chain(self.frames.keys().map(|id| (*id, info(self.focus == Some(*id))))).collect()
    }

    fn render(host: &RefCell<Self>, ctx: &egui::Context, mut viewport: egui::ImmediateViewport<'_>) {
        let id = viewport.ids.this;
        let input = {
            let mut host = host.borrow_mut();
            *host.frames.entry(id).or_default() += 1;
            let events = host.pending.remove(&id).unwrap_or_default();
            egui::RawInput {
                viewport_id: id,
                viewports: host.infos(vec2(800.0, 600.0)),
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
                events,
                focused: host.focus == Some(id),
                time: Some(host.time),
                ..Default::default()
            }
        };
        ctx.run_ui(input, |ui| (viewport.viewport_ui_cb)(ui)).textures_delta.clear();
    }
}

struct Desktop {
    h: Harness<'static, PhotocraftApp>,
    host: Rc<RefCell<Host>>,
    window: ViewportId,
}

impl Desktop {
    /// One document with a second window of it. `native`: the window is its own OS window;
    /// otherwise it is embedded in the main window, as in the web build.
    fn new(native: bool) -> Self {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        app.fonts_ready = true;
        let id = app.ui.alloc_id();
        let view = app.ui.views[0].clone();
        app.ui.windows.push(DocWindow { id, document: 0, view, open: true, floating: false, focused: false, position: None });
        let host = Rc::new(RefCell::new(Host::default()));
        let shared = host.clone();
        egui::Context::set_immediate_viewport_renderer(move |ctx, viewport| Host::render(&shared, ctx, viewport));
        let mut ready = false;
        let h = Harness::builder().with_size(vec2(1200.0, 800.0)).build_ui_state(
            move |ui, app: &mut PhotocraftApp| {
                let ctx = ui.ctx().clone();
                if !ready {
                    PhotocraftApp::setup_context(&ctx, app.ui.theme);
                    crate::type_tool::install_reset_hooks(&ctx);
                    ctx.set_embed_viewports(!native);
                    ready = true;
                    return;
                }
                // The main window's shortcuts, as `logic_frame` runs them.
                if crate::type_tool::in_ui_pass(&ctx) {
                    crate::shortcuts::handle(app, &ctx);
                }
                egui::CentralPanel::default().show(ui, |_| {});
                extra_windows(app, &ctx);
            },
            app,
        );
        let mut desktop = Self { h, host, window: ViewportId::from_hash_of(("docwin", id)) };
        desktop.steps(3);
        desktop
    }

    fn steps(&mut self, n: usize) {
        for _ in 0..n {
            let (infos, main_focused, time) = {
                let mut host = self.host.borrow_mut();
                host.time += 1.0 / 60.0;
                (host.infos(vec2(1200.0, 800.0)), host.focus.is_none(), host.time)
            };
            let input = self.h.input_mut();
            input.viewports = infos;
            input.focused = main_focused;
            input.time = Some(time);
            self.h.step();
        }
    }

    /// Press and release `key` in the window with the keyboard.
    fn press(&mut self, key: Key, modifiers: Modifiers) {
        let mut events = vec![Event::ModifiersChanged(modifiers)];
        events.extend([true, false].into_iter().map(|pressed| Event::Key { key, physical_key: None, pressed, repeat: false, modifiers }));
        events.push(Event::ModifiersChanged(Modifiers::NONE));
        self.send(events);
    }

    /// Give `events` to the window with the keyboard and run two frames.
    fn send(&mut self, events: Vec<Event>) {
        let focus = self.host.borrow().focus;
        match focus {
            Some(window) => self.host.borrow_mut().pending.entry(window).or_default().extend(events),
            None => self.h.input_mut().events.extend(events),
        }
        self.steps(2);
    }

    fn focus(&mut self, window: Option<ViewportId>) {
        self.host.borrow_mut().focus = window;
        self.steps(2);
    }
}

/// ⌘R, tool keys and ⌘D typed while a document's own window has the keyboard run as they do in
/// the main window, once each. Before, its input was never read and nothing happened.
#[test]
fn keys_typed_in_a_document_window_run_the_shortcuts() {
    let mut d = Desktop::new(true);
    assert!(d.host.borrow().frames.contains_key(&d.window), "the document has its own OS window");
    let window = d.window;
    d.focus(Some(window));

    let rulers = d.h.state().ui.extras.rulers;
    d.press(Key::R, Modifiers::COMMAND);
    assert_ne!(d.h.state().ui.extras.rulers, rulers, "⌘R in the window toggles the rulers (once: twice would undo it)");
    d.press(Key::R, Modifiers::COMMAND);
    assert_eq!(d.h.state().ui.extras.rulers, rulers, "and again turns them back");

    d.h.state_mut().ui.tool = Tool::Move;
    d.press(Key::B, Modifiers::NONE);
    assert_eq!(d.h.state().ui.tool, Tool::Brush, "B picks the Brush");
    d.press(Key::V, Modifiers::NONE);
    assert_eq!(d.h.state().ui.tool, Tool::Move, "V picks Move");

    d.h.state_mut().run("select.all", json!({})).unwrap();
    d.press(Key::D, Modifiers::COMMAND);
    assert!(d.h.state().session.active().unwrap().doc.selection.is_none(), "⌘D deselects the window's document");
}

/// With the keyboard back in the main window, its keys run once, and the document window's
/// unfocused frame doesn't run them again.
#[test]
fn main_window_keys_run_once_with_a_document_window_open() {
    let mut d = Desktop::new(true);
    let window = d.window;
    d.focus(Some(window));
    d.focus(None);
    let rulers = d.h.state().ui.extras.rulers;
    d.press(Key::R, Modifiers::COMMAND);
    assert_ne!(d.h.state().ui.extras.rulers, rulers, "⌘R in the main window toggles the rulers once");
}

/// Embedded in the main window (the web build, where viewports aren't separate windows), a
/// document window shares the main window's input, whose shortcuts already ran: a key still runs
/// once. (The shortcuts consume the keys they act on, so this holds even without the window's
/// `EmbeddedWindow` check; the check keeps the rest of the pass, such as held keys, single.)
#[test]
fn an_embedded_document_window_does_not_run_the_main_windows_keys_again() {
    let mut d = Desktop::new(false);
    assert!(d.host.borrow().frames.is_empty(), "no OS window: the document window is drawn in the main one");
    let rulers = d.h.state().ui.extras.rulers;
    d.press(Key::R, Modifiers::COMMAND);
    assert_ne!(d.h.state().ui.extras.rulers, rulers, "⌘R toggles the rulers once");
    d.h.state_mut().ui.tool = Tool::Move;
    d.press(Key::B, Modifiers::NONE);
    assert_eq!(d.h.state().ui.tool, Tool::Brush, "B picks the Brush, not the Pencil after it");
}

/// A tool key held down in a document window springs the tool for as long as it is held, then
/// gives the previous tool back (#2672). The main window's shortcuts, which run every frame, see
/// the key held there too, and don't end the temporary tool early.
#[test]
fn a_tool_key_held_in_a_document_window_springs_the_tool_until_released() {
    let mut d = Desktop::new(true);
    let window = d.window;
    d.focus(Some(window));
    d.h.state_mut().ui.tool = Tool::Move;
    let b = |pressed| Event::Key { key: Key::B, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
    d.send(vec![b(true)]);
    assert_eq!(d.h.state().ui.tool, Tool::Brush, "B picks the Brush");
    d.steps(60);
    assert_eq!(d.h.state().ui.tool, Tool::Brush, "held for a second, B keeps the Brush");
    d.send(vec![b(false)]);
    assert_eq!(d.h.state().ui.tool, Tool::Move, "released, the Move tool comes back");
}
