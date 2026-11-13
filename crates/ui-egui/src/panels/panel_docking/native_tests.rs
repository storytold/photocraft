//! Floating panels in their own OS windows. A stand-in for the window system runs each native
//! window's frame on the harness's context with its own input and desktop rect, as eframe does.

use super::*;
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, ViewportId, pos2, vec2};
use egui_kittest::Harness;
use std::{cell::RefCell, collections::HashMap, rc::Rc};

/// Where the main window's content sits on the desktop.
const ORIGIN: Pos2 = pos2(200.0, 100.0);

struct Window {
    rect: Rect,
    events: Vec<Event>,
    dropped: Vec<egui::DroppedFileHandle>,
    /// The window asked for the clipboard text.
    paste_requested: bool,
    hidden: bool,
}

#[derive(Default)]
struct Host {
    windows: HashMap<ViewportId, Window>,
    /// The window with the keyboard; `None` is the main window.
    focus: Option<ViewportId>,
    time: f64,
    size: egui::Vec2,
    /// The size of the monitor each window is on, as egui-winit reports it; `None` reports none.
    monitor_size: Option<egui::Vec2>,
}

impl Host {
    fn infos(&self) -> egui::ViewportIdMap<egui::ViewportInfo> {
        let info = |rect: Rect, focused: bool| egui::ViewportInfo {
            inner_rect: Some(rect),
            outer_rect: Some(rect),
            focused: Some(focused),
            native_pixels_per_point: Some(1.0),
            monitor_size: self.monitor_size,
            ..Default::default()
        };
        std::iter::once((ViewportId::ROOT, info(Rect::from_min_size(ORIGIN, self.size), self.focus.is_none())))
            .chain(self.windows.iter().map(|(id, window)| (*id, info(window.rect, self.focus == Some(*id)))))
            .collect()
    }

    fn render(host: &RefCell<Self>, ctx: &egui::Context, mut viewport: egui::ImmediateViewport<'_>) {
        let id = viewport.ids.this;
        let input = {
            let mut host = host.borrow_mut();
            let builder = &viewport.builder;
            let rect = Rect::from_min_size(builder.position.unwrap_or_default(), builder.inner_size.unwrap_or(vec2(300.0, 200.0)));
            host.windows.entry(id).or_insert(Window { rect, events: Vec::new(), dropped: Vec::new(), paste_requested: false, hidden: false });
            let (viewports, focused, time) = (host.infos(), host.focus == Some(id), host.time);
            let window = host.windows.get_mut(&id).expect("just added");
            egui::RawInput {
                viewport_id: id,
                viewports,
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, window.rect.size())),
                events: std::mem::take(&mut window.events),
                dropped_files: std::mem::take(&mut window.dropped),
                focused,
                time: Some(time),
                ..Default::default()
            }
        };
        let mut full = ctx.run_ui(input, |ui| (viewport.viewport_ui_cb)(ui));
        full.textures_delta.clear();
        let mut host = host.borrow_mut();
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
    h: Harness<'static, PhotocraftApp>,
    host: Rc<RefCell<Host>>,
}

impl Desktop {
    fn new(app: PhotocraftApp) -> Self {
        Self::on_monitor(app, None)
    }

    /// A desktop whose windows report `monitor_size` as the size of their monitor.
    fn on_monitor(app: PhotocraftApp, monitor_size: Option<egui::Vec2>) -> Self {
        let size = vec2(1400.0, 900.0);
        let host = Rc::new(RefCell::new(Host { size, monitor_size, ..Default::default() }));
        let shared = host.clone();
        egui::Context::set_immediate_viewport_renderer(move |ctx, viewport| Host::render(&shared, ctx, viewport));
        let mut ready = false;
        let h = Harness::builder().with_size(size).with_step_dt(1.0 / 60.0).build_ui_state(
            move |ui, app: &mut PhotocraftApp| {
                if !ready {
                    PhotocraftApp::setup_context(ui.ctx(), app.ui.theme);
                    ui.ctx().set_embed_viewports(false);
                    ready = true;
                    return;
                }
                crate::panels::dock_area(app, ui);
                egui::CentralPanel::default().show(ui, |_| {});
            },
            app,
        );
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

    /// The OS window of the floating window holding `panel`, once the dock area opened one.
    fn window_of(&self, panel: &str) -> Option<ViewportId> {
        let window = lay(self.h.state()).floating.iter().find(|window| window.contains(&panel.to_owned()))?;
        let viewport = craft_ui::docking::DockArea::<String>::window_viewport(area_id(), window.id);
        self.host.borrow().windows.contains_key(&viewport).then_some(viewport)
    }

    fn window_rect(&self, viewport: ViewportId) -> Rect {
        self.host.borrow().windows[&viewport].rect
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
}

fn lay(app: &PhotocraftApp) -> &Layout<String> {
    &app.ui.docking.as_ref().expect("a customized workspace").layout
}

fn app_with_document() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app
}

/// The × of a native window's bar, in its coordinates: an 18 pt square 7 pt from the trailing
/// edge (craft-ui's `OTHER_CLOSE`; this host is not macOS).
fn close_button(rect: Rect) -> Pos2 {
    pos2(rect.width() - 16.0, 9.0)
}

#[test]
fn undocking_a_panel_opens_its_own_window_and_the_studio_card_stays_in_the_main_window() {
    let mut desktop = Desktop::new(app_with_document());
    desktop.h.state_mut().run("window.panel.float", json!({"panel": "info"})).unwrap();
    desktop.steps(3);
    let viewport = desktop.window_of("info").expect("Info floats in its own OS window");
    let window = lay(desktop.h.state()).floating.iter().find(|window| window.contains(&"info".to_owned())).unwrap().clone();
    assert!(
        craft_ui::docking::DockArea::<String>::floating_layer(area_id(), &window).is_none_or(|layer| desktop.h.ctx.memory(|m| m.area_rect(layer.id)).is_none()),
        "and not in the main window"
    );
    // Undock Panel's window opens over its place in the main window.
    let [x, y, ..] = window.rect;
    assert_eq!(desktop.window_rect(viewport).min, ORIGIN + vec2(x, y));
    assert!(native_viewports(desktop.h.state(), &desktop.h.ctx).contains(&viewport));

    // window.panel.window floats a docked panel's group into its own window.
    desktop.h.state_mut().run("window.panel.window", json!({"panel": "history"})).unwrap();
    desktop.steps(3);
    assert!(desktop.window_of("history").is_some(), "History has its own window");

    let mut studio = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    studio.ui.theme = crate::theme::ThemeKind::Studio;
    studio.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    let mut desktop = Desktop::new(studio);
    desktop.steps(3);
    let card = legacy(desktop.h.state()).floating.into_iter().find(|window| window.id == PROPERTIES_WINDOW).expect("the Studio Properties card");
    assert!(desktop.host.borrow().windows.is_empty(), "the card opens no OS window");
    let layer = craft_ui::docking::DockArea::<String>::floating_layer(area_id(), &card).unwrap();
    assert!(desktop.h.ctx.memory(|m| m.area_rect(layer.id)).is_some(), "the card is drawn in the main window");
    let refused = desktop.h.state_mut().run("window.panel.window", json!({"panel": "properties"}));
    assert!(refused.is_err(), "{refused:?}");
}

fn studio_app() -> PhotocraftApp {
    let mut app = app_with_document();
    app.ui.theme = crate::theme::ThemeKind::Studio;
    app
}

#[test]
fn the_studio_card_stays_in_the_main_window_after_a_reload() {
    let mut desktop = Desktop::new(studio_app());
    desktop.h.state_mut().run("window.panel.close", json!({"panel": "layers"})).unwrap();
    desktop.steps(3);
    assert!(desktop.host.borrow().windows.is_empty());
    let saved: Value = serde_json::from_str(&crate::dock::snapshot(desktop.h.state()).to_string()).unwrap();
    drop(desktop);
    let mut reloaded = studio_app();
    crate::dock::apply(&mut reloaded, &saved);
    let mut desktop = Desktop::new(reloaded);
    desktop.steps(3);
    let card = lay(desktop.h.state()).floating.iter().find(|window| window.contains(&"properties".to_owned())).expect("the card").clone();
    assert_eq!(card.id, PROPERTIES_WINDOW);
    assert!(desktop.host.borrow().windows.is_empty(), "the card opens no OS window");
    let layer = craft_ui::docking::DockArea::<String>::floating_layer(area_id(), &card).unwrap();
    assert!(desktop.h.ctx.memory(|m| m.area_rect(layer.id)).is_some(), "the card is drawn in the main window");
    let refused = desktop.h.state_mut().run("window.panel.window", json!({"panel": "properties"}));
    assert!(refused.is_err(), "{refused:?}");
}

#[test]
fn keys_pressed_in_a_panel_window_run_photocrafts_shortcuts() {
    let mut desktop = Desktop::new(app_with_document());
    desktop.h.state_mut().run("window.panel.float", json!({"panel": "info"})).unwrap();
    desktop.steps(3);
    let viewport = desktop.window_of("info").unwrap();
    desktop.host.borrow_mut().focus = Some(viewport);
    desktop.h.state_mut().ui.tool = crate::state::Tool::Move;
    desktop.key(viewport, Key::B, Modifiers::NONE);
    assert_eq!(desktop.h.state().ui.tool, crate::state::Tool::Brush, "B in the Info window picks the Brush");

    let layers = |desktop: &Desktop| desktop.h.state().session.active().unwrap().doc.layers.len();
    let before = layers(&desktop);
    desktop.h.state_mut().run("layer.new.layer", json!({})).unwrap();
    assert_eq!(layers(&desktop), before + 1);
    desktop.key(viewport, Key::Z, Modifiers::COMMAND);
    assert_eq!(layers(&desktop), before, "⌘Z in the Info window undoes");
}

/// The Mac menu bar, reporting one key equivalent.
struct MenuKey(Vec<crate::native_menu::Event>);

impl crate::native_menu::Backend for MenuKey {
    fn sync(&mut self, _: &crate::native_menu::MenuBar) {}
    fn drain(&mut self) -> Vec<crate::native_menu::Event> {
        std::mem::take(&mut self.0)
    }
}

#[test]
fn a_menu_key_typed_in_a_panel_windows_text_field_goes_to_the_field() {
    let mut desktop = Desktop::new(app_with_document());
    let app = desktop.h.state_mut();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 8, "height": 4})).unwrap();
    app.run("edit.copy", json!({})).unwrap();
    app.run("window.panel.float", json!({"panel": "gradients"})).unwrap();
    desktop.steps(3);
    let viewport = desktop.window_of("gradients").unwrap();
    desktop.host.borrow_mut().focus = Some(viewport);
    desktop.h.state_mut().ui.presets_ui.renaming = Some(("gradients".into(), "key".into(), "Name".into()));
    desktop.steps(3);

    let layers = |desktop: &Desktop| desktop.h.state().session.active().unwrap().doc.layers.len();
    let before = layers(&desktop);
    let paste = crate::native_menu::Chord::parse("Cmd+V").unwrap();
    desktop.h.state_mut().services.native_menu = Some(crate::native_menu::NativeMenu::new(Box::new(MenuKey(vec![crate::native_menu::Event::Key(paste)]))));
    let mut raw = egui::RawInput { viewports: desktop.host.borrow().infos(), ..Default::default() };
    let ctx = desktop.h.ctx.clone();
    eframe::App::raw_input_hook(desktop.h.state_mut(), &ctx, &mut raw);
    desktop.h.input_mut().events.extend(raw.events);
    desktop.steps(1);
    let main = desktop.h.output().viewport_output.get(&ViewportId::ROOT).map(|output| output.commands.clone()).unwrap_or_default();
    assert!(!main.contains(&egui::ViewportCommand::RequestPaste), "the main window asks for no paste: {main:?}");
    desktop.steps(2);
    assert_eq!(layers(&desktop), before, "nothing is pasted into the document");
    assert!(desktop.host.borrow().windows[&viewport].paste_requested, "the field's window asks for the clipboard text");
}

#[test]
fn keys_held_in_a_panel_window_count_as_released_once_it_loses_the_keyboard() {
    let mut desktop = Desktop::new(app_with_document());
    desktop.h.state_mut().run("window.panel.float", json!({"panel": "gradients"})).unwrap();
    desktop.steps(3);
    let viewport = desktop.window_of("gradients").unwrap();
    desktop.host.borrow_mut().focus = Some(viewport);
    let space = |pressed| Event::Key { key: Key::Space, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
    // The main window's shortcuts, in a pass with the host's focus.
    let held = |desktop: &mut Desktop| {
        let ctx = desktop.h.ctx.clone();
        let raw = {
            let host = desktop.host.borrow();
            egui::RawInput { viewports: host.infos(), focused: host.focus.is_none(), ..Default::default() }
        };
        ctx.run_ui(raw, |ui| crate::shortcuts::handle(desktop.h.state_mut(), ui.ctx())).textures_delta.clear();
        crate::shortcuts::forwarded_keys_down(&ctx)
    };
    desktop.send(viewport, vec![space(true)]);
    assert_eq!(held(&mut desktop), [Key::Space], "held while the window has the keyboard");
    desktop.host.borrow_mut().focus = Some(ViewportId::from_hash_of("another app"));
    assert_eq!(held(&mut desktop), [], "released once no window of the app has the keyboard");

    desktop.host.borrow_mut().focus = Some(viewport);
    desktop.send(viewport, vec![space(true)]);
    assert_eq!(held(&mut desktop), [Key::Space]);
    desktop.h.state_mut().ui.presets_ui.renaming = Some(("gradients".into(), "key".into(), "Name".into()));
    desktop.steps(3);
    assert_eq!(held(&mut desktop), [], "released once a field of the window takes keyboard focus");
}

#[test]
fn a_dialog_blocks_the_panel_windows() {
    let about = |app: &mut PhotocraftApp| _ = app.ui.open_dialog(crate::state::DialogKind::About, Default::default());
    let blur = |app: &mut PhotocraftApp| _ = crate::dialogs::open_command_dialog(app, "filter.blur.gaussianBlur", "Gaussian Blur");
    for (name, open) in [("About", &about as &dyn Fn(&mut PhotocraftApp)), ("Gaussian Blur", &blur)] {
        let mut desktop = Desktop::new(app_with_document());
        desktop.h.state_mut().run("window.panel.float", json!({"panel": "info"})).unwrap();
        desktop.steps(3);
        let viewport = desktop.window_of("info").unwrap();
        desktop.host.borrow_mut().focus = Some(viewport);
        open(desktop.h.state_mut());
        assert!(modal_open(desktop.h.state(), &desktop.h.ctx), "{name}");
        // The click arrives in the dialog's first frame.
        let close = close_button(desktop.window_rect(viewport));
        desktop.click(viewport, close);
        assert!(lay(desktop.h.state()).contains(&"info".to_owned()), "{name}: the × takes no click under the dialog");
        desktop.h.state_mut().ui.tool = crate::state::Tool::Move;
        desktop.key(viewport, Key::B, Modifiers::NONE);
        assert_eq!(desktop.h.state().ui.tool, crate::state::Tool::Move, "{name}: nor do the shortcuts get its keys");
        desktop.host.borrow_mut().focus = Some(ViewportId::from_hash_of("another app"));
        desktop.steps(30);
        assert!(!desktop.host.borrow().windows[&viewport].hidden, "{name}: the panel window stays shown while the app is in the background");
        desktop.host.borrow_mut().focus = Some(viewport);

        desktop.h.state_mut().ui.dialogs.clear();
        desktop.steps(3);
        assert!(!modal_open(desktop.h.state(), &desktop.h.ctx), "{name}");
        desktop.click(viewport, close);
        assert!(!lay(desktop.h.state()).contains(&"info".to_owned()), "{name}: the × closes the panel once the dialog is gone");
    }
}

#[derive(Debug)]
struct Dropped(std::path::PathBuf, Vec<u8>);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        Ok(self.1.clone())
    }
}

#[test]
fn a_file_dropped_on_a_panel_window_opens_as_a_document() {
    // An importer that names the document after the file, as photocraft-io does.
    let import = |name: &str, _: &[u8], _: &photocraft_io::ImportOptions| {
        let size = photocraft_geom::Size::new(4, 4);
        Ok((photocraft_doc::Document::new(name, size, photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8), Vec::new()))
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services { import: Some(Box::new(import)), ..Default::default() });
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    let mut desktop = Desktop::new(app);
    desktop.h.state_mut().run("window.panel.float", json!({"panel": "info"})).unwrap();
    desktop.steps(3);
    let viewport = desktop.window_of("info").unwrap();
    let documents = desktop.h.state().session.documents().len();
    let file: egui::DroppedFileHandle = std::sync::Arc::new(Dropped("dropped.psd".into(), b"x".to_vec()));
    desktop.host.borrow_mut().windows.get_mut(&viewport).unwrap().dropped.push(file);
    desktop.steps(2);
    let names: Vec<String> = desktop.h.state().session.documents().iter().map(|d| d.doc.name.clone()).collect();
    assert_eq!(names.len(), documents + 1, "{names:?}");
    assert_eq!(names.last().map(String::as_str), Some("dropped.psd"));
}

/// The monitors of a desktop, from what winit reports for each: name, physical origin and size,
/// and scale. The main window (at `ORIGIN`, 1400 × 900) is on the 100% "main" one.
fn desktop_monitors(extra: &[(&str, [i32; 2], [u32; 2], f64)]) -> Vec<craft_ui::docking::Monitor> {
    let display = |&(name, position, size, scale): &(&str, [i32; 2], [u32; 2], f64)| monitors::Display { name: Some(name.into()), position, size, scale };
    let main = ("main", [0, 0], [1920, 1080], 1.0);
    monitors::dock_monitors(std::iter::once(&main).chain(extra).map(display), 1.0)
}

/// Float Info, leave its OS window at `at` on a desktop of `saved` monitors, and quit; start
/// again on a desktop of `connected` monitors. Returns where Info's window opens.
fn reopened_at(saved: Vec<craft_ui::docking::Monitor>, at: Pos2, connected: Vec<craft_ui::docking::Monitor>) -> Rect {
    // egui-winit reports the size of the monitor each window is on; the main window's is 1920 × 1080.
    let monitor_size = Some(vec2(1920.0, 1080.0));
    let mut app = app_with_document();
    app.ui.docking_monitors = saved;
    let mut desktop = Desktop::on_monitor(app, monitor_size);
    desktop.h.state_mut().run("window.panel.float", json!({"panel": "info"})).unwrap();
    // Past the dock's wait for the window system to place a new window.
    desktop.steps(40);
    let viewport = desktop.window_of("info").expect("Info's OS window");
    // The user drags it to `at`: the window system reports it there.
    let size = desktop.window_rect(viewport).size();
    desktop.host.borrow_mut().windows.get_mut(&viewport).unwrap().rect = Rect::from_min_size(at, size);
    desktop.steps(40);
    let saved: Value = serde_json::from_str(&crate::dock::snapshot(desktop.h.state()).to_string()).unwrap();
    drop(desktop);

    let mut app = app_with_document();
    crate::dock::apply(&mut app, &saved);
    app.ui.docking_monitors = connected;
    let mut desktop = Desktop::on_monitor(app, monitor_size);
    desktop.steps(3);
    let reopened = desktop.window_rect(desktop.window_of("info").expect("Info opens in its own OS window again"));
    assert_eq!(reopened.size(), size, "at the size it had");
    reopened
}

#[test]
fn a_floating_panel_reopens_on_the_monitor_it_was_left_on() {
    // A 100% monitor right of the main one (#13599).
    let right = desktop_monitors(&[("right", [1920, 0], [2560, 1440], 1.0)]);
    assert_eq!(reopened_at(right.clone(), pos2(2400.0, 300.0), right).min, pos2(2400.0, 300.0));
    // A 150% monitor left of and above the main one: negative origins, in its own points.
    let left = desktop_monitors(&[("left", [-3840, -300], [3840, 2160], 1.5)]);
    assert_eq!(left[1].rect, [-2560.0, -200.0, 2560.0, 1440.0]);
    assert_eq!(reopened_at(left.clone(), pos2(-2000.0, -100.0), left).min, pos2(-2000.0, -100.0));
}

#[test]
fn a_floating_panel_on_a_monitor_that_is_gone_reopens_on_the_main_one() {
    let right = desktop_monitors(&[("right", [1920, 0], [2560, 1440], 1.0)]);
    let only_main = desktop_monitors(&[]);
    let reopened = reopened_at(right, pos2(2400.0, 300.0), only_main);
    // At the same place on the main monitor as it had on its own, wholly on it.
    assert_eq!(reopened.min, pos2(480.0, 300.0));
    assert!(Rect::from_min_size(Pos2::ZERO, vec2(1920.0, 1080.0)).contains_rect(reopened), "{reopened:?}");
    // Left past the bottom right corner of a monitor that is gone: pulled wholly onto the main one.
    let reopened = reopened_at(desktop_monitors(&[("right", [1920, 0], [2560, 1440], 1.0)]), pos2(4300.0, 1300.0), desktop_monitors(&[]));
    assert_eq!(reopened.max, pos2(1920.0, 1080.0), "{reopened:?}");
}
