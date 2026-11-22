//! #16415: a document floated into an OS window of its own, with the app-drawn frame, has a title bar: the window
//! can be told apart, moved, and closed from it. A stand-in for the window system runs each OS window's pass on the
//! harness's context with its own input, as eframe does.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use egui::accesskit::{Node, NodeId, Role};
use egui::{Event, Modifiers, PointerButton, Pos2, Rect, ViewportId, pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;

/// Where the floating window opens on the desktop.
fn window_rect() -> Rect {
    Rect::from_min_size(pos2(100.0, 80.0), vec2(500.0, 400.0))
}

/// The OS windows the app opened: each one's input for its next pass and its access tree as of its last pass.
#[derive(Default)]
struct Host {
    events: HashMap<ViewportId, Vec<Event>>,
    nodes: HashMap<ViewportId, HashMap<NodeId, Node>>,
    commands: Vec<(ViewportId, egui::ViewportCommand)>,
    time: f64,
    /// Where the OS has the floating window, once it moved from where it opened.
    moved: Option<Rect>,
    /// The height of a system title bar over the floating window: its outer rect reaches that far above its inner rect.
    decorated: f32,
}

impl Host {
    /// The floating window's outer rect on the desktop, as the OS reports it.
    fn window_rect(&self) -> Rect {
        self.moved.unwrap_or_else(window_rect)
    }

    /// The window facts of the main window and the floating window `id`, the floating one with the keyboard.
    fn infos(&self, id: ViewportId) -> egui::ViewportIdMap<egui::ViewportInfo> {
        let info = |rect: Rect, focused: bool| egui::ViewportInfo {
            inner_rect: Some(rect),
            outer_rect: Some(rect),
            focused: Some(focused),
            native_pixels_per_point: Some(1.0),
            ..Default::default()
        };
        let mut window = info(self.window_rect(), true);
        window.outer_rect = window.inner_rect.map(|inner| Rect::from_min_max(inner.min - vec2(0.0, self.decorated), inner.max));
        [(ViewportId::ROOT, info(Rect::from_min_size(Pos2::ZERO, vec2(900.0, 700.0)), false)), (id, window)].into_iter().collect()
    }

    fn render(host: &RefCell<Self>, ctx: &egui::Context, mut viewport: egui::ImmediateViewport<'_>) {
        let id = viewport.ids.this;
        let size = viewport.builder.inner_size.unwrap_or(window_rect().size());
        let (events, time) = {
            let mut host = host.borrow_mut();
            host.time += 1.0 / 60.0;
            (host.events.remove(&id).unwrap_or_default(), host.time)
        };
        let viewports = host.borrow().infos(id);
        let input = egui::RawInput {
            viewport_id: id,
            viewports,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            events,
            focused: true,
            time: Some(time),
            ..Default::default()
        };
        let mut full = ctx.run_ui(input, |ui| (viewport.viewport_ui_cb)(ui));
        full.textures_delta.clear();
        let mut host = host.borrow_mut();
        if let Some(update) = full.platform_output.accesskit_update.take() {
            host.nodes.entry(id).or_default().extend(update.nodes);
        }
        let commands = full.viewport_output.get(&id).map(|output| output.commands.clone()).unwrap_or_default();
        // The OS follows a move the app asks for.
        for command in &commands {
            if let egui::ViewportCommand::OuterPosition(pos) = command {
                let size = host.window_rect().size();
                host.moved = Some(Rect::from_min_size(*pos, size));
            }
        }
        host.commands.extend(commands.into_iter().map(|command| (id, command)));
    }
}

type App = Box<PhotocraftApp>;

/// The whole app with its one document floated, the frame decided as at launch with the app's own title bar, and
/// OS windows on the stand-in host.
fn desktop() -> (Harness<'static, App>, Rc<RefCell<Host>>) {
    let host = Rc::new(RefCell::new(Host::default()));
    let shared = host.clone();
    egui::Context::set_immediate_viewport_renderer(move |ctx, viewport| Host::render(&shared, ctx, viewport));
    let mut h = Harness::builder().with_size(vec2(900.0, 700.0)).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(|cc| {
        // The app's own frame with Windows' captions, whatever the OS or `PHOTOCRAFT_TITLEBAR`: the bar these tests drive
        // has Minimize, Maximize and Close.
        use craft::window::policy::{CaptionFamily, Captions, Decision, Desktop, DesktopKind, Frame};
        let frame = Frame::Integrated(Captions { family: CaptionFamily::Windows, layout: craft::window::Layout::default() });
        let desktop = Desktop { kind: DesktopKind::Other, ssd: None, gtk_theme: None, icon_theme: None };
        let decision = Decision { frame, overridden: None, desktop };
        PhotocraftApp::setup_window(&cc.egui_ctx, crate::theme::AppTheme::pro(), decision);
        cc.egui_ctx.set_embed_viewports(false);
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), crate::Services::default());
        app.background_jobs = false;
        app.run("file.new", json!({"width": 200, "height": 150, "name": "floated.png"})).unwrap();
        app.sync_views();
        let document = app.session.documents()[0].doc.id;
        assert!(app.ui.document_windows.float(&document, window_rect()).is_some(), "the document floats");
        app
    });
    h.run_steps(4);
    (h, host)
}

/// The one OS window the app opened.
fn window(host: &RefCell<Host>) -> ViewportId {
    let host = host.borrow();
    assert_eq!(host.nodes.len(), 1, "one document window");
    *host.nodes.keys().next().unwrap()
}

/// The bounds of every node in `id`'s window whose role is `role` and whose name contains `name`.
fn nodes_bounds(host: &RefCell<Host>, id: ViewportId, role: Role, name: &str) -> Vec<Rect> {
    let host = host.borrow();
    let Some(nodes) = host.nodes.get(&id) else { return Vec::new() };
    nodes
        .values()
        .filter(|n| n.role() == role && n.label().is_some_and(|l| l.contains(name)))
        .filter_map(|n| n.bounds().map(|b| Rect::from_min_max(pos2(b.x0 as f32, b.y0 as f32), pos2(b.x1 as f32, b.y1 as f32))))
        .collect()
}

/// The window's Close caption button.
fn close_button(host: &RefCell<Host>, id: ViewportId) -> Option<Rect> {
    nodes_bounds(host, id, Role::Button, "Close").into_iter().next()
}

/// The window's title bar: a group named after the document, a bar's height at the window's top. The canvas groups
/// carry the same name but fill the window.
fn title_bar(host: &RefCell<Host>, id: ViewportId) -> Option<Rect> {
    nodes_bounds(host, id, Role::Group, "floated.png").into_iter().find(|r| r.top() <= 1.0 && r.height() < 60.0)
}

#[test]
fn a_floating_document_window_has_a_title_bar_at_its_top() {
    let (_h, host) = desktop();
    let id = window(&host);
    let bar = title_bar(&host, id).expect("the floating window has a title bar named after its document");

    assert!(close_button(&host, id).is_some_and(|c| bar.contains(c.center())), "the bar has a Close button");
}

/// Runs `n` frames with the OS windows listed in the main window's input, the floating one with the keyboard, as an
/// integration reports them.
fn steps(h: &mut Harness<'static, App>, host: &RefCell<Host>, id: ViewportId, n: usize) {
    for _ in 0..n {
        let infos = host.borrow().infos(id);
        let input = h.input_mut();
        input.viewports = infos;
        input.focused = false;
        h.step();
    }
}

/// Clicks the floating window's title-bar Close.
fn click_close(h: &mut Harness<'static, App>, host: &RefCell<Host>, id: ViewportId) {
    let close = close_button(host, id).expect("the bar has a Close button").center();
    let press = |pressed| Event::PointerButton { pos: close, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    for events in [vec![Event::PointerMoved(close)], vec![press(true)], vec![press(false)]] {
        host.borrow_mut().events.insert(id, events);
        steps(h, host, id, 1);
    }
    steps(h, host, id, 2);
}

/// Close in a floating window's title bar closes its document, as in the reference application: a saved one at once,
/// with the window; one with unsaved changes asks first, and the window stays while it asks.
#[test]
fn the_title_bar_close_closes_the_floating_document() {
    let (mut h, host) = desktop();
    let id = window(&host);
    h.state_mut().run("layer.new.layer", json!({})).unwrap();
    steps(&mut h, &host, id, 2);
    click_close(&mut h, &host, id);
    assert!(h.state().dialog_open(), "a document with unsaved changes asks before it closes");
    assert_eq!(h.state().session.documents().len(), 1, "the document stays while the prompt asks");
    assert_eq!(h.state().ui.document_windows.floating.len(), 1, "and so does its window");

    let (mut h, host) = desktop();
    let id = window(&host);
    click_close(&mut h, &host, id);
    assert!(h.state().session.documents().is_empty(), "a saved document closes");
    assert!(h.state().ui.document_windows.floating.is_empty(), "its window goes with it");
}

#[test]
fn dragging_the_title_bar_moves_the_floating_window() {
    let (mut h, host) = desktop();
    let id = window(&host);
    let bar = title_bar(&host, id).expect("the floating window has a title bar");
    // Left of the centred title: the free band, no control.
    let band = bar.left_center() + vec2(40.0, 0.0);
    let press = |pressed, pos| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    host.borrow_mut().commands.clear();
    for events in [
        vec![Event::PointerMoved(band)],
        vec![press(true, band)],
        vec![Event::PointerMoved(band + vec2(40.0, 0.0))],
        vec![press(false, band + vec2(40.0, 0.0))],
    ] {
        host.borrow_mut().events.insert(id, events);
        steps(&mut h, &host, id, 1);
    }
    let started = host.borrow().commands.iter().any(|(viewport, command)| *viewport == id && *command == egui::ViewportCommand::StartDrag)
        || h.output().viewport_output.get(&id).is_some_and(|out| out.commands.contains(&egui::ViewportCommand::StartDrag));
    assert!(started, "a drag on the bar's free band moves the window: {:?}", host.borrow().commands);
}

/// A move of the floating window (the title bar's OS drag, or the OS itself) stays: the app does not put the window
/// back where it opened.
#[test]
fn a_moved_floating_window_stays_where_it_was_moved() {
    let (mut h, host) = desktop();
    let id = window(&host);
    steps(&mut h, &host, id, 2);
    let moved = window_rect().translate(vec2(150.0, 60.0));
    host.borrow_mut().moved = Some(moved);
    steps(&mut h, &host, id, 4);
    assert_eq!(host.borrow().window_rect(), moved, "the window stays where the OS moved it");
    let placed = h.state().ui.document_windows.floating.first().map(|(_, rect)| *rect);
    assert_eq!(placed, Some(moved), "the app keeps the window's new place");
}

/// Presses the floating window's title bar at its middle-left, lets the OS move the window to `to` (outer rect on the
/// desktop) and releases there, as an OS move by the title bar does (the runtime adds the release when it ends).
fn move_by_title_bar(h: &mut Harness<'static, App>, host: &RefCell<Host>, id: ViewportId, to: Option<Rect>) {
    let bar = title_bar(host, id).expect("the floating window has a title bar");
    let at = bar.left_center() + vec2(40.0, 0.0);
    let press = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    host.borrow_mut().events.insert(id, vec![Event::PointerMoved(at), press(true)]);
    steps(h, host, id, 1);
    if let Some(to) = to {
        host.borrow_mut().moved = Some(to);
        steps(h, host, id, 1);
    }
    host.borrow_mut().events.insert(id, vec![press(false)]);
    steps(h, host, id, 3);
}

/// #16415: a floating document window dropped by its title bar on the main window's document tabs goes back into
/// them; dropped elsewhere it stays where it went, and a click on its bar docks nothing.
#[test]
fn a_floating_window_dropped_on_the_main_windows_tabs_docks_again() {
    let (mut h, host) = desktop();
    let id = window(&host);
    steps(&mut h, &host, id, 2);
    // A click, no move: the window stays.
    move_by_title_bar(&mut h, &host, id, None);
    assert_eq!(h.state().ui.document_windows.floating.len(), 1, "a click on the bar docks nothing");
    // Dropped far off the main window: it stays floating there.
    let away = Rect::from_min_size(pos2(2000.0, 1200.0), window_rect().size());
    move_by_title_bar(&mut h, &host, id, Some(away));
    assert_eq!(h.state().ui.document_windows.floating.len(), 1, "dropped off the main window it keeps floating");
    // Dropped with the pointer on the main window's tab area: it docks, its document in front.
    let target = h.state().dock_drop_rect.expect("the main window has a place to dock");
    let bar = title_bar(&host, id).expect("the bar");
    let grab = bar.left_center() + vec2(40.0, 0.0);
    let over = Rect::from_min_size(target.center() - grab.to_vec2(), window_rect().size());
    move_by_title_bar(&mut h, &host, id, Some(over));
    assert!(h.state().ui.document_windows.floating.is_empty(), "dropped on the tabs the window docks");
    assert_eq!(h.state().session.documents().len(), 1, "the document stays open");
    assert_eq!(h.state().session.active_index(), Some(0), "the docked document is in front");
}

/// #16415, Windows: the OS runs a title-bar move of the floating window (its hit test answers the caption), so the app
/// sees no press and no release. The window moving while the physical button is down, then the button going up with the
/// OS pointer on the bar over the main window's tabs, docks it; the same end with the pointer elsewhere keeps it.
#[test]
fn an_os_move_of_the_floating_window_ending_over_the_tabs_docks_it() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    let (mut h, host) = desktop();
    let id = window(&host);
    let down = Arc::new(AtomicBool::new(false));
    let pointer = Rc::new(Cell::new(None::<egui::Pos2>));
    {
        let (down, pointer) = (Arc::clone(&down), Rc::clone(&pointer));
        let app = h.state_mut();
        app.services.primary_down = Some(Box::new(move || down.load(Ordering::SeqCst)));
        app.services.cursor_pos = Some(Box::new(move |_| pointer.get()));
    }
    steps(&mut h, &host, id, 2);
    let bar = title_bar(&host, id).expect("the bar");
    let grab = bar.left_center() + vec2(40.0, 0.0);
    // The OS moves the window with the button held, then the button goes up: off the tabs, nothing docks.
    let os_move = |h: &mut Harness<'static, App>, to: Rect| {
        down.store(true, Ordering::SeqCst);
        host.borrow_mut().moved = Some(to);
        pointer.set(Some(to.min + grab.to_vec2()));
        steps(h, &host, id, 2);
        down.store(false, Ordering::SeqCst);
        steps(h, &host, id, 2);
        h.state().ui.document_windows.floating.len()
    };
    let away = Rect::from_min_size(pos2(500.0, 400.0), window_rect().size());
    assert_eq!(os_move(&mut h, away), 1, "an OS move ending off the tabs keeps it floating");
    let target = h.state().dock_drop_rect.expect("the main window has a place to dock");
    let over = Rect::from_min_size(target.center() - grab.to_vec2(), window_rect().size());
    assert_eq!(os_move(&mut h, over), 0, "an OS move ending on the tabs docks it");
    assert_eq!(h.state().session.documents().len(), 1, "the document stays open");
}

/// #16415: a floating document window has a status bar along its bottom with its own document's readout, as the
/// reference application's floating windows do: the zoom and, by default, the document's dimensions.
#[test]
fn a_floating_document_window_has_a_status_bar_with_its_documents_readout() {
    let (_h, host) = desktop();
    let id = window(&host);
    let host = host.borrow();
    let nodes = host.nodes.get(&id).expect("the window's access tree");
    let labels: Vec<&str> = nodes.values().filter_map(|n| n.label()).collect();
    let dims = labels.iter().find(|l| l.contains("200") && l.contains("150")).copied();
    assert!(dims.is_some(), "the status bar names the document's 200 x 150 size: {labels:?}");
    let zoom = nodes.values().any(|n| n.label().is_some_and(|l| l.contains('%')) || n.value().is_some_and(|v| v.contains('%')));
    assert!(zoom, "the status bar has the zoom: {labels:?}");
}

/// #16415: a floating window with the system's title bar (its outer rect taller than its inner one) keeps its size: the
/// app records the outer origin with the inner size, which the framework applies as the inner size again.
#[test]
fn a_decorated_floating_window_keeps_its_size() {
    let (mut h, host) = desktop();
    let id = window(&host);
    host.borrow_mut().decorated = 30.0;
    steps(&mut h, &host, id, 6);
    let placed = h.state().ui.document_windows.floating.first().map(|(_, rect)| *rect).expect("the window floats");
    assert!((placed.size() - window_rect().size()).length() < 0.5, "the stored size stays the inner size: {placed:?}");
    assert!((placed.min.y - (window_rect().min.y - 30.0)).abs() < 0.5, "at the outer origin: {placed:?}");
}
