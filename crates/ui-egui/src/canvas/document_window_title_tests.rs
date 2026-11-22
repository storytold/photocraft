//! #16415: a document floated into an OS window of its own, with the app-drawn frame, has a title bar: the window
//! can be told apart, moved, and closed from it. A stand-in for the window system runs each OS window's pass on the
//! harness's context with its own input, as eframe does.

use std::cell::RefCell;
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
}

impl Host {
    fn render(host: &RefCell<Self>, ctx: &egui::Context, mut viewport: egui::ImmediateViewport<'_>) {
        let id = viewport.ids.this;
        let size = viewport.builder.inner_size.unwrap_or(window_rect().size());
        let (events, time) = {
            let mut host = host.borrow_mut();
            host.time += 1.0 / 60.0;
            (host.events.remove(&id).unwrap_or_default(), host.time)
        };
        let info = |rect: Rect, focused: bool| egui::ViewportInfo {
            inner_rect: Some(rect),
            outer_rect: Some(rect),
            focused: Some(focused),
            native_pixels_per_point: Some(1.0),
            ..Default::default()
        };
        let viewports =
            [(ViewportId::ROOT, info(Rect::from_min_size(Pos2::ZERO, vec2(900.0, 700.0)), false)), (id, info(window_rect(), true))].into_iter().collect();
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
        let decision = crate::window_policy::decide_for(crate::Prefix::new("PHOTOCRAFT"), crate::Backend::from_env(), crate::TitleBarPref::Integrated);
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
fn steps(h: &mut Harness<'static, App>, id: ViewportId, n: usize) {
    for _ in 0..n {
        let info = |rect: Rect, focused: bool| egui::ViewportInfo {
            inner_rect: Some(rect),
            outer_rect: Some(rect),
            focused: Some(focused),
            native_pixels_per_point: Some(1.0),
            ..Default::default()
        };
        let input = h.input_mut();
        input.viewports =
            [(ViewportId::ROOT, info(Rect::from_min_size(Pos2::ZERO, vec2(900.0, 700.0)), false)), (id, info(window_rect(), true))].into_iter().collect();
        input.focused = false;
        h.step();
    }
}

#[test]
fn the_title_bar_close_closes_the_floating_window() {
    let (mut h, host) = desktop();
    let id = window(&host);
    let close = close_button(&host, id).expect("the bar has a Close button").center();
    let press = |pressed| Event::PointerButton { pos: close, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    for events in [vec![Event::PointerMoved(close)], vec![press(true)], vec![press(false)]] {
        host.borrow_mut().events.insert(id, events);
        steps(&mut h, id, 1);
    }
    steps(&mut h, id, 2);
    assert!(h.state().ui.document_windows.floating.is_empty(), "Close in the bar closes the window");
    assert_eq!(h.state().session.documents().len(), 1, "the document is kept, back in the tabs");
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
        steps(&mut h, id, 1);
    }
    let started = host.borrow().commands.iter().any(|(viewport, command)| *viewport == id && *command == egui::ViewportCommand::StartDrag)
        || h.output().viewport_output.get(&id).is_some_and(|out| out.commands.contains(&egui::ViewportCommand::StartDrag));
    assert!(started, "a drag on the bar's free band moves the window: {:?}", host.borrow().commands);
}
