//! Floating panels in their own OS windows. A stand-in for the window system runs each native
//! window's frame on the harness's context with its own input and desktop rect, as eframe does, around
//! the whole app's frame.

use super::*;
use craft::behavior::monitor::Monitor;
use craft::menu::{Backend, Chord, Dispatch, Event as MenuEvent, MenuBar};
use craft::ui::patterns::presets::PresetBrowser;
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, ViewportId, pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;
use std::{cell::RefCell, collections::HashMap, rc::Rc};

/// Where the main window's content sits on the desktop.
const ORIGIN: Pos2 = pos2(200.0, 100.0);

struct Window {
    rect: Rect,
    events: Vec<Event>,
    /// The window asked for the clipboard text.
    paste_requested: bool,
    hidden: bool,
    /// The window's access tree nodes, as of its last pass.
    nodes: HashMap<egui::accesskit::NodeId, egui::accesskit::Node>,
    /// Physical pixels per point of the monitor the window is on.
    scale: f32,
}

#[derive(Default)]
struct Host {
    windows: HashMap<ViewportId, Window>,
    /// The window with the keyboard; `None` is the main window.
    focus: Option<ViewportId>,
    time: f64,
    size: egui::Vec2,
}

impl Host {
    fn infos(&self) -> egui::ViewportIdMap<egui::ViewportInfo> {
        let info = |rect: Rect, focused: bool, scale: f32| egui::ViewportInfo {
            inner_rect: Some(rect),
            outer_rect: Some(rect),
            focused: Some(focused),
            native_pixels_per_point: Some(scale),
            ..Default::default()
        };
        std::iter::once((ViewportId::ROOT, info(Rect::from_min_size(ORIGIN, self.size), self.focus.is_none(), 1.0)))
            .chain(self.windows.iter().map(|(id, window)| (*id, info(window.rect, self.focus == Some(*id), window.scale))))
            .collect()
    }

    fn render(host: &RefCell<Self>, ctx: &egui::Context, mut viewport: egui::ImmediateViewport<'_>) {
        let id = viewport.ids.this;
        let input = {
            let mut host = host.borrow_mut();
            let builder = &viewport.builder;
            let rect = Rect::from_min_size(builder.position.unwrap_or_default(), builder.inner_size.unwrap_or(vec2(300.0, 200.0)));
            host.windows.entry(id).or_insert(Window { rect, events: Vec::new(), paste_requested: false, hidden: false, nodes: HashMap::new(), scale: 1.0 });
            let (viewports, focused, time) = (host.infos(), host.focus == Some(id), host.time);
            let window = host.windows.get_mut(&id).expect("just added");
            egui::RawInput {
                viewport_id: id,
                viewports,
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, window.rect.size())),
                events: std::mem::take(&mut window.events),
                focused,
                time: Some(time),
                ..Default::default()
            }
        };
        let mut full = ctx.run_ui(input, |ui| (viewport.viewport_ui_cb)(ui));
        full.textures_delta.clear();
        let mut host = host.borrow_mut();
        if let (Some(update), Some(window)) = (full.platform_output.accesskit_update.take(), host.windows.get_mut(&id)) {
            window.nodes.extend(update.nodes);
        }
        for command in full.viewport_output.get(&id).map(|output| output.commands.as_slice()).unwrap_or_default() {
            let Some(window) = host.windows.get_mut(&id) else { continue };
            match command {
                egui::ViewportCommand::OuterPosition(pos) => window.rect = Rect::from_min_size(*pos, window.rect.size()),
                egui::ViewportCommand::InnerSize(size) => window.rect = Rect::from_min_size(window.rect.min, *size),
                egui::ViewportCommand::RequestPaste => window.paste_requested = true,
                _ => {}
            }
        }
    }
}

struct Desktop {
    h: Harness<'static, Box<PhotocraftApp>>,
    host: Rc<RefCell<Host>>,
}

impl Desktop {
    /// The whole app in an eframe harness whose floated panels open OS windows on the stand-in host.
    fn new(app: Box<PhotocraftApp>) -> Self {
        Self::with_monitors(app, Vec::new())
    }

    /// [`Desktop::new`] on a desktop whose window system lists `monitors` from the app's first frame,
    /// as winit lists them to the app (`framework::windows::refresh`).
    fn with_monitors(app: Box<PhotocraftApp>, monitors: Vec<Monitor>) -> Self {
        let size = vec2(1400.0, 900.0);
        let host = Rc::new(RefCell::new(Host { size, ..Default::default() }));
        let shared = host.clone();
        egui::Context::set_immediate_viewport_renderer(move |ctx, viewport| Host::render(&shared, ctx, viewport));
        let app = std::cell::Cell::new(Some(app));
        let h = Harness::builder().with_size(size).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            crate::app::framework::windows::of(&cc.egui_ctx).expect("the runtime's window system").set_monitors(monitors.clone());
            cc.egui_ctx.set_embed_viewports(false);
            app.take().expect("one app")
        });
        let mut desktop = Self { h, host };
        desktop.steps(4);
        desktop
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
            // A command sent to a window from the main window's pass is in the main pass's output;
            // one sent from the window's own pass is in that pass's output (`Host::render`).
            let mut host = self.host.borrow_mut();
            for (id, output) in &self.h.output().viewport_output {
                if let Some(window) = host.windows.get_mut(id) {
                    window.paste_requested |= output.commands.contains(&egui::ViewportCommand::RequestPaste);
                    for command in &output.commands {
                        if let egui::ViewportCommand::Visible(visible) = command {
                            window.hidden = !visible;
                        }
                    }
                }
            }
        }
    }

    /// The OS window the dock showed `panel`'s floating window in, as the dock reported it on the last pass.
    fn window(&self, panel: &str) -> Option<ViewportId> {
        probe::native_viewport(&self.h.ctx, panel)
    }

    /// The Close header button of the floating window `viewport`, in the window's coordinates: found by its
    /// label in the window's access tree.
    fn close_button(&self, viewport: ViewportId) -> Pos2 {
        let host = self.host.borrow();
        let nodes = &host.windows[&viewport].nodes;
        let close = nodes
            .values()
            .find(|node| node.role() == egui::accesskit::Role::Button && node.label() == Some("Close"))
            .expect("the floating window has a Close button");
        let bounds = close.bounds().expect("the Close button has bounds");
        pos2(((bounds.x0 + bounds.x1) / 2.0) as f32, ((bounds.y0 + bounds.y1) / 2.0) as f32)
    }

    /// Give `events` to the native window `viewport` and run two frames.
    fn send(&mut self, viewport: ViewportId, events: Vec<Event>) {
        self.host.borrow_mut().windows.get_mut(&viewport).expect("a native window").events.extend(events);
        self.steps(2);
    }

    fn click(&mut self, viewport: ViewportId, at: Pos2) {
        let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        self.send(viewport, vec![Event::PointerMoved(at), button(true)]);
        self.send(viewport, vec![button(false)]);
    }

    fn key(&mut self, viewport: ViewportId, key: Key, modifiers: Modifiers) {
        let event = |pressed| Event::Key { key, physical_key: None, pressed, repeat: false, modifiers };
        self.send(viewport, vec![event(true)]);
        self.send(viewport, vec![event(false)]);
    }

    /// Floats `panel`'s group into its own OS window and gives that window the keyboard.
    fn float_and_focus(&mut self, panel: &str) -> ViewportId {
        self.h.state_mut().run("window.panel.window", json!({"panel": panel})).unwrap();
        self.steps(3);
        let viewport = self.window(panel).unwrap_or_else(|| panic!("{panel} floats in its own OS window"));
        self.host.borrow_mut().focus = Some(viewport);
        viewport
    }
}

fn app_with_document() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app
}

/// A key pressed in a panel's own window reaches PhotoCraft's shortcut dispatch: the runtime routes each
/// window's input, and the dispatch must not read the main window only.
#[test]
fn keys_pressed_in_a_panel_window_run_photocrafts_shortcuts() {
    let mut desktop = Desktop::new(app_with_document());
    let viewport = desktop.float_and_focus("layers");
    desktop.h.state_mut().ui.tool = crate::state::Tool::Move;
    desktop.key(viewport, Key::B, Modifiers::NONE);
    assert_eq!(desktop.h.state().ui.tool, crate::state::Tool::Brush, "B in the Layers window picks the Brush");

    let layers = |desktop: &Desktop| desktop.h.state().session.active().unwrap().doc.layers.len();
    let before = layers(&desktop);
    desktop.h.state_mut().run("layer.new.layer", json!({})).unwrap();
    assert_eq!(layers(&desktop), before + 1);
    desktop.key(viewport, Key::Z, Modifiers::COMMAND);
    assert_eq!(layers(&desktop), before, "⌘Z in the Layers window undoes");
}

/// The Mac menu bar, reporting one key equivalent.
struct MenuKey(Vec<MenuEvent>);

impl Backend for MenuKey {
    fn sync(&mut self, _: &MenuBar) {}
    fn drain(&mut self) -> Vec<MenuEvent> {
        std::mem::take(&mut self.0)
    }
    fn attach(&mut self, _: Dispatch) {}
}

/// A menu key equivalent typed while a panel window's text field has the keyboard goes to the field:
/// the window asks for the clipboard text, and the document is not pasted into.
#[test]
fn a_menu_key_typed_in_a_panel_windows_text_field_goes_to_the_field() {
    let mut desktop = Desktop::new(app_with_document());
    let app = desktop.h.state_mut();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 8, "height": 4})).unwrap();
    app.run("edit.copy", json!({})).unwrap();
    let viewport = desktop.float_and_focus("gradients");
    let browser = crate::panels::preset_browser::browser_id(crate::panels::preset_browser::GRADIENTS);
    PresetBrowser::request_rename(&desktop.h.ctx, browser, &crate::panels::preset_browser::PresetKey::Item("Name".into()));
    desktop.steps(3);

    let layers = |desktop: &Desktop| desktop.h.state().session.active().unwrap().doc.layers.len();
    let before = layers(&desktop);
    let paste = Chord::parse("Cmd+V").unwrap();
    desktop.h.state_mut().services.native_menu = Some(craft::menu::NativeMenu::new(Box::new(MenuKey(vec![MenuEvent::Key(paste)]))));
    desktop.steps(1);
    let main = desktop.h.output().viewport_output.get(&ViewportId::ROOT).map(|output| output.commands.clone()).unwrap_or_default();
    assert!(!main.contains(&egui::ViewportCommand::RequestPaste), "the main window asks for no paste: {main:?}");
    desktop.steps(2);
    assert_eq!(layers(&desktop), before, "nothing is pasted into the document");
    assert!(desktop.host.borrow().windows[&viewport].paste_requested, "the field's window asks for the clipboard text");
}

/// PhotoCraft's dialogs are framework layers, so the runtime's modal blocking reaches the panel windows:
/// under a dialog the window takes no click and no key, and once it is closed the window works again.
#[test]
fn a_dialog_blocks_the_panel_windows() {
    for name in ["About", "Gaussian Blur"] {
        let mut desktop = Desktop::new(app_with_document());
        let viewport = desktop.float_and_focus("layers");
        // Read before the dialog opens: the runtime hides the window's nodes behind a modal.
        let close = desktop.close_button(viewport);
        let id = match name {
            "About" => Ok(crate::dialogs::credits::open_about(desktop.h.state_mut(), Default::default())),
            _ => crate::dialogs::open_command_dialog(desktop.h.state_mut(), "filter.blur.gaussianBlur", "Gaussian Blur"),
        }
        .unwrap_or_else(|e| panic!("{name}: {e}"));
        desktop.steps(2);
        assert!(desktop.h.state().dialog_open(), "{name}");
        // The click arrives in the dialog's first frame.
        desktop.click(viewport, close);
        assert!(is_shown(desktop.h.state(), "layers"), "{name}: the × takes no click under the dialog");
        desktop.h.state_mut().ui.tool = crate::state::Tool::Move;
        desktop.key(viewport, Key::B, Modifiers::NONE);
        assert_eq!(desktop.h.state().ui.tool, crate::state::Tool::Move, "{name}: nor do the shortcuts get its keys");
        desktop.host.borrow_mut().focus = Some(ViewportId::from_hash_of("another app"));
        desktop.steps(30);
        assert!(!desktop.host.borrow().windows[&viewport].hidden, "{name}: the panel window stays shown while the app is in the background");
        desktop.host.borrow_mut().focus = Some(viewport);

        crate::dialogs::testing::cancel(desktop.h.state_mut(), id).unwrap();
        desktop.steps(3);
        assert!(!desktop.h.state().dialog_open(), "{name}");
        desktop.click(viewport, close);
        assert!(!is_shown(desktop.h.state(), "layers"), "{name}: the × closes the panel once the dialog is gone");
    }
}

/// A floated toolbar stays in the main window, where its tooltips, tool flyouts and drawer have room beside the rail;
/// another panel floats in an OS window of its own.
#[test]
fn a_floated_toolbar_stays_in_the_main_window_while_other_panels_get_os_windows() {
    let mut desktop = Desktop::new(app_with_document());
    for panel in [TOOLBAR, "layers"] {
        desktop.h.state_mut().run("window.panel.window", json!({"panel": panel})).unwrap();
    }
    desktop.steps(3);
    let floating = |panel: &str| matches!(desktop.h.state().ui.docking.layout().place(&panel.to_owned()), Some(PanelPlace::Floating { .. }));
    assert!(floating(TOOLBAR) && floating("layers"), "both panels float");
    assert!(desktop.window(TOOLBAR).is_none(), "the toolbar floats in the main window");
    assert!(desktop.window("layers").is_some(), "the Layers panel floats in its own OS window");
}

/// A monitor of the desktop, in physical pixels, as winit reports it.
fn monitor(name: &str, position: [i32; 2], size: [u32; 2], scale: f64) -> Monitor {
    crate::app::framework::windows::monitor(Some(name.into()), position, size, scale).expect("a monitor")
}

/// The 100% monitor the main window (at `ORIGIN`, 1400 × 900) is on.
fn main_monitor() -> Monitor {
    monitor(r"\\.\DISPLAY1", [0, 0], [1920, 1080], 1.0)
}

/// Floats Info into its own OS window on a desktop of `left_on` monitors, leaves that window at `at`
/// (desktop points of a monitor of `scale`), and quits with Remember Workspace Changes on; then
/// starts again with the remembered workspace on a desktop of `connected` monitors. Returns where
/// Info's window opens, and the size it had.
fn reopened_at(left_on: Vec<Monitor>, at: Pos2, scale: f32, connected: Vec<Monitor>) -> (Rect, egui::Vec2) {
    let mut desktop = Desktop::with_monitors(app_with_document(), left_on);
    desktop.h.state_mut().run("window.panel.window", json!({"panel": "info"})).unwrap();
    desktop.steps(3);
    let viewport = desktop.window("info").expect("Info floats in its own OS window");
    // The user drags the window to `at`; the window system reports it there, at its monitor's scale.
    let size = {
        let mut host = desktop.host.borrow_mut();
        let window = host.windows.get_mut(&viewport).expect("Info's window");
        window.rect = Rect::from_min_size(at, window.rect.size());
        window.scale = scale;
        window.rect.size()
    };
    desktop.steps(10);
    let saved = crate::dock::snapshot(desktop.h.state());
    drop(desktop);

    let mut app = app_with_document();
    app.session.prefs.edit(|prefs| {
        prefs.workspace.remember_workspace_changes = true;
        prefs.panel_layout = saved;
    });
    crate::dock::restore(&mut app);
    let mut desktop = Desktop::with_monitors(app, connected);
    desktop.steps(3);
    let viewport = desktop.window("info").expect("Info opens in its own OS window again");
    let reopened = desktop.host.borrow().windows[&viewport].rect;
    (reopened, size)
}

/// #13599: a floating panel left on another monitor reopens there after a restart while that monitor is
/// connected, also on a mixed-DPI desktop with a monitor left of and above the main one.
#[test]
fn a_floating_panel_reopens_on_the_monitor_it_was_left_on() {
    let right = vec![main_monitor(), monitor(r"\\.\DISPLAY2", [1920, 0], [2560, 1440], 1.0)];
    let (reopened, size) = reopened_at(right.clone(), pos2(2400.0, 300.0), 1.0, right);
    assert_eq!(reopened, Rect::from_min_size(pos2(2400.0, 300.0), size), "on the right monitor, where it was left");

    // At 150%: (-2000, -100) points are (-3000, -150) pixels, on the left monitor.
    let left = vec![main_monitor(), monitor(r"\\.\DISPLAY2", [-3840, -300], [3840, 2160], 1.5)];
    let (reopened, size) = reopened_at(left.clone(), pos2(-2000.0, -100.0), 1.5, left);
    assert_eq!(reopened, Rect::from_min_size(pos2(-2000.0, -100.0), size), "on the 150% monitor, where it was left");
}

/// A floating panel left on a monitor that is no longer connected reopens on the main window's monitor,
/// moved the least that puts it wholly there, instead of off every screen.
#[test]
fn a_floating_panel_on_a_monitor_that_is_gone_reopens_on_the_main_one() {
    let main = Rect::from_min_size(Pos2::ZERO, vec2(1920.0, 1080.0));
    let right = vec![main_monitor(), monitor(r"\\.\DISPLAY2", [1920, 0], [2560, 1440], 1.0)];
    let (reopened, size) = reopened_at(right.clone(), pos2(2400.0, 300.0), 1.0, vec![main_monitor()]);
    assert_eq!(reopened, Rect::from_min_size(pos2(1920.0 - size.x, 300.0), size), "against the main monitor's right edge");
    assert!(main.contains_rect(reopened), "{reopened:?}");

    // Left past the gone monitor's bottom-right corner, below the main monitor's bottom: against the main
    // monitor's right edge and up onto it, wholly on it.
    let (reopened, _) = reopened_at(right, pos2(4300.0, 1300.0), 1.0, vec![main_monitor()]);
    assert_eq!(reopened.max.x, main.max.x, "{reopened:?}");
    assert!(main.contains_rect(reopened), "{reopened:?}");
}
