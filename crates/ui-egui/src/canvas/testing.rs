//! Headless driver for the canvas tools' tests. Each call queues window input through craft-automation's synthetic
//! input and runs the app's own frame (key route, canvas pass, hosted dialogs) on the app's context (`app.ctx()`, the
//! one its code outside the pass reads) until the window host has delivered it,
//! so a press passes the preflight, the stand-ins, the dialog tool and the canvas controls of the body exactly as a
//! mouse press does. Document points map through the view the canvas last showed (`view`).

use craft::behavior::kurbo::Point;
use craft::behavior::{CanvasView, PointerButton};
use craft::keymap::{Key, Mods};
use craft::ui::canvas::ToolHostResponse;
use craft::ui::runtime::windows::{Released, WindowHost};
use craft::ui::testing::Frames;
use egui::{Modifiers, Pos2};

use super::document_display::CanvasHost;
use super::{active_view, tools};
use crate::PhotocraftApp;
use crate::stylus::PenSample;

/// Frames a queued input may take before the driver gives up: a click with a count of 2 is five steps.
const MAX_FRAMES: usize = 64;

/// One pointer event of a tool test, in document coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ToolEvent {
    /// The primary button goes down.
    Down {
        /// Document x.
        x: f64,
        /// Document y.
        y: f64,
        /// The pen's pressure, 1 for a mouse.
        pressure: f32,
    },
    /// The pointer moves, dragging while a button is down.
    Move {
        /// Document x.
        x: f64,
        /// Document y.
        y: f64,
        /// The pen's pressure, 1 for a mouse.
        pressure: f32,
    },
    /// The primary button comes up.
    Up {
        /// Document x.
        x: f64,
        /// Document y.
        y: f64,
    },
}

/// Delivers `ev` with `mods` held: `Down` is `press`, `Move` is `move_to` and `Up` is `release`, with the pressure
/// of `Down` and `Move` fed through the stylus feed first.
pub(crate) fn tool_event(app: &mut PhotocraftApp, ev: ToolEvent, mods: Modifiers) {
    // The tests that read the response call `tool_event_response`.
    let _ = tool_event_response(app, ev, mods);
}

/// `tool_event`, returning what the tool host reported in the main canvas's pass of the last frame the delivery ran
/// (the default when that frame ran no main canvas pass).
pub(crate) fn tool_event_response(app: &mut PhotocraftApp, ev: ToolEvent, mods: Modifiers) -> ToolHostResponse {
    match ev {
        ToolEvent::Down { x, y, pressure } => {
            pen(app, pressure);
            press(app, [x, y], mods);
        }
        ToolEvent::Move { x, y, pressure } => {
            pen(app, pressure);
            move_to(app, [x, y], mods);
        }
        ToolEvent::Up { x, y } => {
            release(app, [x, y], mods);
            app.stylus.feed.set(None);
        }
    }
    app.canvas_tools.last_response.take().unwrap_or_default()
}

/// Feeds the pen's pressure for the next frame: a reading of `pressure`, unless the feed already holds one of that
/// pressure (a test's own reading, with its tilt and pen end); a mouse (pressure 1) feeds no pen.
fn pen(app: &PhotocraftApp, pressure: f32) {
    let feed = &app.stylus.feed;
    if feed.get().map(|s| s.pressure) == Some(pressure) {
        return;
    }
    feed.set((pressure != 1.0).then(|| PenSample { pressure, ..PenSample::default() }));
}

/// The context the canvas pass runs in: the app's own, which its code outside the pass reads too.
pub(crate) fn ctx(app: &PhotocraftApp) -> egui::Context {
    app.ctx().clone()
}

/// Makes the window the app shows a Mac's: the window host then reports ⌘ as `mac_cmd` and leaves Ctrl its own key, which the
/// ⌘⌥⌃ gestures need (off macOS the host reports Ctrl as ⌘ and never sets both).
pub(crate) fn on_a_mac(app: &PhotocraftApp) {
    craft::ui::testing::windows::set_os(app.ctx(), craft::host::target::Os::Mac);
}

/// `mods` as the window host takes them: the macOS reading of egui's modifiers (⌘ is `mac_cmd`, or `command` on a
/// platform where egui folds Ctrl into it; Ctrl is its own key only where it is not the command key).
fn held(mods: Modifiers) -> Mods {
    Mods { cmd: mods.mac_cmd || mods.command, ctrl: mods.ctrl && (mods.mac_cmd || !mods.command), alt: mods.alt, shift: mods.shift }
}

/// One frame of the app on its own context, as eframe runs its hooks: `raw_input_hook` gives the frame the app's
/// command catalog, states and gate and takes what the last pass fired; each pass runs `logic`'s key route first (the
/// commands the last pass fired, and the window's Escape before the canvas or a dialog can take it), then draws the
/// document area and the hosted dialogs.
fn run_frame(app: &mut PhotocraftApp) {
    // The app's frame reconciles the views with the open documents before anything draws.
    app.sync_views();
    let ctx = ctx(app);
    let mut frames = app.canvas_tools.frames.take().unwrap_or_else(|| Frames::over(ctx.clone()));
    app.canvas_tools.last_response = None;
    app.framework_before_input(&ctx);
    // The platform completes an IME reset the framework asked for in the pass before; without it the engine drops
    // every composition event after an interrupted composition.
    let os_input = ctx.data_mut(|d| d.remove_temp::<Vec<egui::Event>>(egui::Id::unique(OS_IME_RESETS))).unwrap_or_default();
    frames.frame_without_input(os_input, |ui| {
        crate::shortcut_dispatch::record_view_commands(app);
        crate::shortcuts::handle(app, &ui.ctx().clone());
        egui::CentralPanel::default().show(ui, |ui| super::document_area(app, ui));
        crate::dialogs::run(app, ui);
        crate::shortcuts::escape_keys(app, &ui.ctx().clone());
    });
    if let Some(request_id) = frames.ime().and_then(|ime| ime.reset) {
        ctx.data_mut(|d| d.insert_temp(egui::Id::unique(OS_IME_RESETS), vec![egui::Event::ImeResetCompleted { request_id }]));
    }
    app.canvas_tools.frames = Some(frames);
}

const OS_IME_RESETS: &str = "photocraft.test-os-ime-resets";

/// Holds back the platform's answer to the IME reset the last pass asked for, until the next frame has run: events
/// of the interrupted composition that are still on their way reach the framework before the answer, as they do on
/// a platform. The request stays pending, so the answer follows that frame.
pub(crate) fn hold_ime_reset_answer(app: &PhotocraftApp) {
    let _ = ctx(app).data_mut(|d| d.remove_temp::<Vec<egui::Event>>(egui::Id::unique(OS_IME_RESETS)));
}

/// Runs frames until the window host has delivered everything queued.
fn settle(app: &mut PhotocraftApp) {
    let host = WindowHost::of(&ctx(app));
    for _ in 0..MAX_FRAMES {
        run_frame(app);
        if host.injected_idle() {
            return;
        }
    }
    panic!("the injected input was not taken in {MAX_FRAMES} frames");
}

/// The active document's main canvas view as its last pass showed it, rect included.
pub(crate) fn view(app: &PhotocraftApp) -> Option<CanvasView> {
    active_view(app)
}

/// Pans the active view so the document point `at` shows in the strip at the canvas's left edge: a dialog opens centred
/// over the canvas and takes the presses that land on it, so a test whose gesture must reach the canvas starts there.
pub(crate) fn clear_of_dialogs(app: &mut PhotocraftApp, at: [f64; 2]) {
    shown(app);
    let Some(shown_view) = view(app) else { return };
    let (here, goal) = (shown_view.to_screen(Point::new(at[0], at[1])), Pos2::new(shown_view.rect.left() + 40.0, shown_view.rect.center().y));
    let moved = shown_view.to_doc(goal) - shown_view.to_doc(here);
    if let Some(i) = app.session.active_index().filter(|i| *i < app.ui.views.len()) {
        let v = &mut app.ui.views[i];
        v.center = [v.center[0] - moved.x as f32, v.center[1] - moved.y as f32];
        v.clear_pending();
    }
    shown(app);
}

/// The screen point of the document point `at` through the view the canvas last showed.
pub(crate) fn to_screen(app: &PhotocraftApp, at: [f64; 2]) -> Pos2 {
    view(app).expect("the canvas shows a view of the active document").to_screen(Point::new(at[0], at[1]))
}

/// Runs frames until the canvas's view holds still, so an input maps through the layout the next pass shows: the first
/// passes lay the window out (the document area is resized once the chrome has its room), and a zoom eases over several
/// passes (the shown view moves toward the zoom asked for); an input mapped through an earlier pass's view would land
/// on the later layout.
pub(crate) fn shown(app: &mut PhotocraftApp) {
    let mut last = view(app).map(|v| v.rect);
    for _ in 0..MAX_FRAMES {
        run_frame(app);
        let now = view(app).map(|v| v.rect);
        if now.is_some() && now == last && !easing(app) {
            return;
        }
        last = now;
    }
}

/// Whether the active document's main canvas is easing a zoom.
fn easing(app: &PhotocraftApp) -> bool {
    app.session.active_index().is_some_and(|idx| craft::ui::canvas::easing_view(app.ctx(), CanvasHost::main(idx).canvas_id()).is_some())
}

/// Presses the primary button at the document point `at` with `mods` held, and leaves it down.
pub(crate) fn press(app: &mut PhotocraftApp, at: [f64; 2], mods: Modifiers) {
    shown(app);
    let pos = to_screen(app, at);
    WindowHost::of(&ctx(app)).inject_click(egui::ViewportId::ROOT, pos, PointerButton::Primary, 1, held(mods), true);
    settle(app);
}

/// Moves the pointer to `at`; a drag while the button is down.
pub(crate) fn move_to(app: &mut PhotocraftApp, at: [f64; 2], mods: Modifiers) {
    shown(app);
    let pos = to_screen(app, at);
    WindowHost::of(&ctx(app)).inject_move(egui::ViewportId::ROOT, pos, held(mods), 0.0);
    settle(app);
}

/// Lets the primary button go at `at`.
pub(crate) fn release(app: &mut PhotocraftApp, at: [f64; 2], mods: Modifiers) {
    shown(app);
    let pos = to_screen(app, at);
    let host = WindowHost::of(&ctx(app));
    host.inject_move(egui::ViewportId::ROOT, pos, held(mods), 0.0);
    host.inject_release(egui::ViewportId::ROOT, Some(pos), Released::Button(PointerButton::Primary));
    settle(app);
}

/// A press and a release at `at`.
pub(crate) fn click(app: &mut PhotocraftApp, at: [f64; 2], mods: Modifiers) {
    shown(app);
    let pos = to_screen(app, at);
    WindowHost::of(&ctx(app)).inject_click(egui::ViewportId::ROOT, pos, PointerButton::Primary, 1, held(mods), false);
    settle(app);
}

/// A secondary-button press and release at `at`.
pub(crate) fn secondary_click(app: &mut PhotocraftApp, at: [f64; 2], mods: Modifiers) {
    shown(app);
    let pos = to_screen(app, at);
    WindowHost::of(&ctx(app)).inject_click(egui::ViewportId::ROOT, pos, PointerButton::Secondary, 1, held(mods), false);
    settle(app);
}

/// A double click at `at`: one click with a count of 2, so egui counts the presses.
pub(crate) fn double_click(app: &mut PhotocraftApp, at: [f64; 2], mods: Modifiers) {
    shown(app);
    let pos = to_screen(app, at);
    WindowHost::of(&ctx(app)).inject_click(egui::ViewportId::ROOT, pos, PointerButton::Primary, 2, held(mods), false);
    settle(app);
}

/// A key press and release with `mods` held. The app's key route runs first in the frame and reads the claims the tool
/// host made in the last pass, so a Delete or Escape the tool claimed reaches it in the pass after its claim.
pub(crate) fn key(app: &mut PhotocraftApp, key: egui::Key, mods: Modifiers) {
    let Some(key) = Key::from_egui(key) else { panic!("{key:?} has no key of the window host") };
    WindowHost::of(&ctx(app)).inject_key(egui::ViewportId::ROOT, key, held(mods), false);
    settle(app);
    // The command the key fired in the last pass runs in the next frame's key route, which shows what it opened.
    run_frame(app);
}

/// Changes the held modifiers with no pointer event: the pointer stays where it is.
pub(crate) fn modifiers(app: &mut PhotocraftApp, mods: Modifiers) {
    let ctx = ctx(app);
    let at = ctx.input(|i| i.pointer.latest_pos()).or_else(|| view(app).map(|v| v.rect.center())).unwrap_or(Pos2::ZERO);
    WindowHost::of(&ctx).inject_move(egui::ViewportId::ROOT, at, held(mods), 0.0);
    settle(app);
}

/// One pass with no input: what a tool change cancels, a document switch drops.
pub(crate) fn frame(app: &mut PhotocraftApp) {
    run_frame(app);
}

/// Keeps the frames' clock still: a frame leaves the time where it is, so a held pen repeats no point (and no
/// other timed behaviour fires) until the test advances `Frames`' clock.
pub(crate) fn still_clock(app: &mut PhotocraftApp) {
    let ctx = ctx(app);
    app.canvas_tools.frames.get_or_insert_with(|| Frames::over(ctx)).still_clock();
}

/// The window the following frames run in is `size` points (at least 800 × 600); the document area and its canvas
/// fill it as they fill the app's window.
pub(crate) fn window(app: &mut PhotocraftApp, size: egui::Vec2) {
    let ctx = ctx(app);
    app.canvas_tools.frames.get_or_insert_with(|| Frames::over(ctx)).screen(size);
}

/// Sends the control request `method` with `params` and returns its reply, then runs frames until the window host has
/// delivered what the request queued (`ui.pointer` queues its events for the canvas's tool host). A request maps
/// document points through the view the canvas last showed, so the canvas shows one first.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn control(app: &mut PhotocraftApp, method: &str, params: serde_json::Value) -> serde_json::Value {
    shown(app);
    let ctx = ctx(app);
    let (request, _receiver) = crate::control::ControlRequest::new(method, params);
    let crate::control::Reply::Done(reply) = crate::control::handle(app, &ctx, &request) else { panic!("{method}: expected an immediate reply") };
    settle(app);
    reply
}

/// Whether the active canvas has a gesture in progress.
pub(crate) fn in_gesture(app: &PhotocraftApp) -> bool {
    tools::in_gesture(app)
}
