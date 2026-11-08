//! Pen tablet pressure, tilt, rotation and eraser end on macOS, Linux X11 and Wayland (issue
//! #79), and barrel buttons on Windows (#5905) and Wayland. Windows also reads the eraser end
//! (#2285).
//!
//! winit 0.30 drops tablet data on both macOS and X11, so `photocraft-tablet` reads it beside winit
//! (an AppKit local event monitor; XInput2 raw events on a second X connection) and this module writes
//! each sample into the UI's [`StylusFeed`], where the canvas reads it exactly like the web runner's
//! Pointer Events and automation's simulated pen. On Windows, winit forwards `WM_POINTER` pressure
//! as touch force, but drops `PEN_FLAG_BARREL` (`0x0001`) and the eraser end (`PEN_FLAG_INVERTED`);
//! `photocraft-tablet` recovers them through thread-local message hooks beside winit. With Windows
//! Ink disabled for the tablet, the optional WinTab mode reads pressure, tilt, rotation and the
//! eraser end from a WinTab 1.4 system context instead ([`install_wintab`]).
//!
//! Wayland compositors give a pen only to clients that bind the tablet protocol
//! (`zwp_tablet_v2`), which winit 0.30 doesn't: there the pen does nothing in the window (#639,
//! #5805). `photocraft-tablet` binds it on winit's connection ([`spawn_wayland`]), and the
//! [`WaylandPen`] plugin feeds the pen's motion, tip and barrel buttons to egui as pointer events
//! besides writing pressure, tilt and the eraser end into the stylus feed (#7707). The pen shows
//! the UI's cursor, the brush outline included. A pen press can't start a native window move or
//! resize (winit asks the compositor with the mouse's serial), so the custom title bar is dragged
//! with the mouse. Only when the compositor has no tablet protocol does the app open its window
//! through Xwayland with a pen attached ([`display_session`]), where the X11 reader applies.
//! On some multi-GPU systems Xwayland can only present through a GPU that drives no display,
//! which shows up as a blurred, very slow window; keep native Wayland with
//! `PHOTOCRAFT_NATIVE_WAYLAND=1` on a compositor with tablet-v2 (or use an X11 session).

#[cfg(any(target_os = "linux", test))]
use crate::linux_libs::DisplaySession;
use photocraft_tablet::Sample;
use photocraft_ui_egui::stylus::{PenSample, StylusFeed};

/// A tablet sample as the UI's pen sample.
#[allow(dead_code)]
pub fn pen_sample(s: Sample) -> PenSample {
    PenSample { pressure: s.pressure, tilt_x: s.tilt_x, tilt_y: s.tilt_y, rotation: s.rotation, eraser: s.eraser }.sanitized()
}

/// Writes tablet samples into `feed` (`None` = a mouse).
#[allow(dead_code)]
pub fn sink(feed: StylusFeed) -> impl Fn(Option<Sample>) + Send + 'static {
    move |s| feed.set(s.map(pen_sample))
}

/// Install the Windows pointer message hooks (main thread; keep the result until the event loop
/// returns). On failure this logs why: the pen barrel button and eraser end are then missed.
#[cfg(target_os = "windows")]
pub fn install_windows() -> Option<photocraft_tablet::windows::Monitor> {
    let monitor = photocraft_tablet::windows::Monitor::install().map_err(|e| log::warn!("{e}")).ok()?;
    if !monitor.tracks_leave() {
        log::warn!("tablet input: the WH_CALLWNDPROC hook did not install, so the pen leaving the window may be missed");
    }
    Some(monitor)
}

/// Installs the WinTab backend on the real PhotoCraft window and returns its per-frame poll.
///
/// `enabled` is Preferences > Tools > Pen Tablet > Tablet input set to WinTab. With Windows Ink disabled there, the vendor
/// driver still moves the cursor; this backend only supplies pressure/tilt/rotation/eraser
/// through the normal stylus feed. On failure it logs why and leaves Windows Ink untouched.
#[cfg(target_os = "windows")]
pub fn install_wintab(cc: &eframe::CreationContext<'_>, feed: &StylusFeed, enabled: bool) -> Option<photocraft_ui_egui::TabletPollFn> {
    if !enabled {
        return None;
    }
    use eframe::wgpu::rwh::{HasWindowHandle, RawWindowHandle};
    let hwnd = match cc.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => handle.hwnd.get(),
        _ => return None,
    };
    let ctx = cc.egui_ctx.clone();
    if let Err(error) = photocraft_tablet::wintab::install(hwnd, Box::new(move || ctx.request_repaint())) {
        log::warn!("WinTab backend: {error}");
        return None;
    }
    let sink = sink(feed.clone());
    log::info!("WinTab backend active (system context)");
    let logged = std::cell::Cell::new(false);
    Some(Box::new(move |focused| {
        photocraft_tablet::wintab::poll(focused, &|sample| {
            if let Some(sample) = sample
                && !logged.replace(true)
            {
                log::info!("WinTab sample: pressure={:.3} tilt=({:.1},{:.1}) eraser={}", sample.pressure, sample.tilt_x, sample.tilt_y, sample.eraser);
            }
            sink(sample);
        });
    }))
}

/// Stylus barrel button down check (Windows). The right mouse button is left to egui, which
/// reports the logical secondary button (swapped buttons included) for the app's own window only.
#[cfg(target_os = "windows")]
pub fn right_button_service() -> Option<photocraft_ui_egui::RightButtonDownFn> {
    Some(Box::new(photocraft_tablet::windows::is_barrel_button_down))
}

#[cfg(not(target_os = "windows"))]
pub fn right_button_service() -> Option<photocraft_ui_egui::RightButtonDownFn> {
    None
}

/// Which end of the pen is in use (Windows): flipping the pen to its eraser end selects the
/// Eraser (#2285). Elsewhere the end arrives with the samples in the stylus feed.
#[cfg(target_os = "windows")]
pub fn pen_end_service() -> Option<photocraft_ui_egui::PenEndFn> {
    Some(Box::new(photocraft_tablet::windows::take_pen_end))
}

#[cfg(not(target_os = "windows"))]
pub fn pen_end_service() -> Option<photocraft_ui_egui::PenEndFn> {
    None
}

/// Install the AppKit tablet monitor (main thread, once winit created the shared application:
/// from eframe's app creation, not before the event loop). Keep the result until the event loop
/// returns. On failure this logs why: pens then paint like a mouse.
#[cfg(target_os = "macos")]
pub fn install_macos(feed: &StylusFeed) -> Option<photocraft_tablet::macos::Monitor> {
    photocraft_tablet::macos::Monitor::install(sink(feed.clone())).map_err(|e| log::warn!("{e}")).ok()
}

/// The display server to open the window on: the session's own, except Xwayland on Wayland when
/// a pen is attached, the compositor has no tablet protocol (see the module docs) and Xwayland
/// can run the window: `$DISPLAY` is set and the X11 libraries are installed.
/// `PHOTOCRAFT_NATIVE_WAYLAND=1` keeps native Wayland and wins when both variables are set;
/// `PHOTOCRAFT_XWAYLAND_PEN=1` takes Xwayland for a pen even when the compositor has the tablet
/// protocol (the behaviour before native Wayland pens).
#[cfg(target_os = "linux")]
pub fn display_session() -> DisplaySession {
    let var = |k: &str| std::env::var(k).ok();
    let session = crate::linux_libs::session_from_env(var);
    let pen = || std::fs::read_to_string("/proc/bus/input/devices").is_ok_and(|d| has_pen(&d));
    let x11_libs = || crate::linux_libs::available(DisplaySession::X11);
    let native_tablet = photocraft_tablet::wayland::compositor_has_tablet;
    if pen_needs_xwayland(session, var, pen, native_tablet, x11_libs) {
        eprintln!("photocraft: pen found; opening the window through Xwayland, where the pen works (PHOTOCRAFT_NATIVE_WAYLAND=1 keeps Wayland)");
        return DisplaySession::X11;
    }
    session
}

/// [`display_session`]'s decision; the system probes run only when the session asks for them.
/// `PHOTOCRAFT_NATIVE_WAYLAND` takes precedence over `PHOTOCRAFT_XWAYLAND_PEN`.
#[cfg(any(target_os = "linux", test))]
pub fn pen_needs_xwayland(
    session: DisplaySession,
    var: impl Fn(&str) -> Option<String>,
    pen: impl FnOnce() -> bool,
    native_tablet: impl FnOnce() -> bool,
    x11_libs: impl FnOnce() -> bool,
) -> bool {
    let set = |k: &str| var(k).is_some_and(|v| !v.is_empty() && v != "0");
    session == DisplaySession::Wayland
        && !set("PHOTOCRAFT_NATIVE_WAYLAND")
        && set("DISPLAY")
        && pen()
        && (set("PHOTOCRAFT_XWAYLAND_PEN") || !native_tablet())
        && x11_libs()
}

/// `BTN_TOOL_PEN` (linux/input-event-codes.h): the key a pen tablet's stylus, a pen display's or
/// a pen-enabled touchscreen's reports (libinput treats such a device as a tablet tool).
#[cfg(any(target_os = "linux", test))]
const BTN_TOOL_PEN: u32 = 0x140;

/// `ABS_X` and `ABS_Y` (linux/input-event-codes.h): the absolute axes every pen reports.
#[cfg(any(target_os = "linux", test))]
const ABS_X: u32 = 0x00;
#[cfg(any(target_os = "linux", test))]
const ABS_Y: u32 = 0x01;

/// Is a pen among the input devices `/proc/bus/input/devices` lists? A pen is a device whose
/// `B: KEY=` bitmap has `BTN_TOOL_PEN` *and* whose `B: ABS=` bitmap has `ABS_X` and `ABS_Y`
/// (libinput asks the same of a tablet tool). The key alone is not enough: a virtual keyboard
/// made with uinput (RustDesk's, for one) claims every key code, `BTN_TOOL_PEN` included, but
/// has no absolute axes. Taking it for a pen sent the window through Xwayland, where a
/// fractionally scaled Wayland desktop shows it upscaled and blurred (#1072, #639).
#[cfg(any(target_os = "linux", test))]
pub fn has_pen(devices: &str) -> bool {
    devices.split("\n\n").any(|device| {
        let bitmap = |name: &str| device.lines().find_map(|l| l.strip_prefix(name));
        bitmap("B: KEY=").is_some_and(|keys| bit_set(keys, BTN_TOOL_PEN)) && bitmap("B: ABS=").is_some_and(|axes| bit_set(axes, ABS_X) && bit_set(axes, ABS_Y))
    })
}

/// Bit `bit` of a `/proc/bus/input/devices` bitmap: hex words of the kernel's `long`, most
/// significant first. Short or malformed bitmaps have no bits set.
#[cfg(any(target_os = "linux", test))]
fn bit_set(bitmap: &str, bit: u32) -> bool {
    let bits = usize::BITS;
    let word = usize::try_from(bit / bits).unwrap_or(usize::MAX);
    bitmap.split_whitespace().rev().nth(word).and_then(|w| u64::from_str_radix(w, 16).ok()).is_some_and(|w| (w >> (bit % bits)) & 1 == 1)
}

/// Start the XInput2 reader when eframe runs on X11 (Xwayland included). On native Wayland,
/// `$DISPLAY` is Xwayland, which sees none of this window's input (and a pen sample from another
/// X app would outlive a mouse stroke here), so nothing starts: [`spawn_wayland`] reads the pen.
#[cfg(target_os = "linux")]
pub fn spawn_x11(feed: &StylusFeed, display: Option<DisplayKind>) {
    if display != Some(DisplayKind::X11) {
        if display != Some(DisplayKind::Wayland) {
            log::info!("tablet pressure: not available on this display server ({display:?})");
        }
        return;
    }
    if let Err(e) = photocraft_tablet::x11::spawn(None, sink(feed.clone())) {
        log::warn!("{e}");
    }
}

/// Start the tablet-protocol reader when eframe runs on native Wayland (see the module docs). On
/// success the [`WaylandPen`] plugin injects the pen into the viewport it is over ([`PenRoute`]).
/// Keep the result until the event loop returns, then drop it (before the process exits: the
/// reader reads winit's display).
#[cfg(target_os = "linux")]
pub fn spawn_wayland(cc: &eframe::CreationContext<'_>, feed: &StylusFeed, display: Option<DisplayKind>) -> Option<WaylandTablet> {
    if display != Some(DisplayKind::Wayland) {
        return None;
    }
    let Some(window) = cc.winit_window().cloned() else {
        log::warn!("tablet: no Wayland window; the pen won't work in the window");
        return None;
    };
    let ctx = cc.egui_ctx.clone();
    match photocraft_tablet::wayland::Reader::spawn(window, move || ctx.request_repaint()) {
        Ok(reader) => {
            log::info!("tablet: reading the pen through the Wayland tablet protocol");
            let route = PenRoute { window: reader.window, server_decorations: reader.server_decorations };
            let reader = std::sync::Arc::new(std::sync::Mutex::new(Some(reader)));
            cc.egui_ctx.add_plugin(WaylandPen { reader: reader.clone(), feed: feed.clone(), route, input: PenInput::default(), passes: Vec::new() });
            Some(WaylandTablet(reader))
        }
        Err(e) => {
            log::warn!("{e}");
            None
        }
    }
}

/// The running Wayland tablet reader; dropping it stops the reader (the plugin then does nothing).
#[cfg(target_os = "linux")]
pub struct WaylandTablet(std::sync::Arc<std::sync::Mutex<Option<photocraft_tablet::wayland::Reader>>>);

#[cfg(target_os = "linux")]
impl Drop for WaylandTablet {
    /// Stop the reader (it reads winit's display) while the window still exists.
    fn drop(&mut self) {
        let reader = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
        drop(reader);
    }
}

#[cfg(any(target_os = "linux", test))]
use photocraft_tablet::wl::{FrameOut, PenButton, PenEvent};

/// Cap on the pen events, and on the samples, waiting for one viewport's pass (a viewport that
/// stopped repainting, a stalled UI): past it both are reduced together (`PenInput::bound`),
/// the events to their net effect ([`reconcile`]) and the samples to the last, so the moves left
/// still pair with samples.
#[cfg(any(target_os = "linux", test))]
const MAX_QUEUED: usize = 4096;

/// Reduce queued pen events to their net effect for the viewport they wait for: each button's
/// first transition (from the state the viewport has) and its last when that ends differently,
/// the final position, and the departure when the pen left last. Taps in between are lost; a
/// press never loses its release.
#[cfg(any(target_os = "linux", test))]
fn reconcile(q: &mut Vec<PenEvent>) {
    let button_of = |e: &PenEvent| match e {
        PenEvent::Button { button, pressed, .. } => Some((*button, *pressed)),
        _ => None,
    };
    let left = q.last() == Some(&PenEvent::Leave);
    let (mut first, mut last, mut moved) = (Vec::new(), Vec::new(), None);
    for e in q.drain(..) {
        match button_of(&e) {
            Some((b, _)) if first.iter().any(|f| button_of(f).is_some_and(|(fb, _)| fb == b)) => {
                last.retain(|l| button_of(l).is_some_and(|(lb, _)| lb != b));
                last.push(e);
            }
            Some(_) => first.push(e),
            None if matches!(e, PenEvent::Move { .. }) => moved = Some(e),
            None => {}
        }
    }
    last.retain(|l| first.iter().any(|f| matches!((button_of(f), button_of(l)), (Some((fb, fp)), Some((lb, lp))) if fb == lb && fp != lp)));
    q.extend(first);
    q.extend(moved);
    q.extend(last);
    if left {
        q.push(PenEvent::Leave);
    }
}

/// The mouse's buttons, from each pass's own (native) input. While one is held, and in a pass
/// that pressed one, the mouse owns the pointer and the stylus feed: a hovering pen must not move
/// the pointer, press or set its pressure 0 into a mouse stroke.
#[cfg(any(target_os = "linux", test))]
#[derive(Default)]
pub struct MouseButtons {
    held: Vec<(egui::ViewportId, egui::PointerButton)>,
}

#[cfg(any(target_os = "linux", test))]
impl MouseButtons {
    /// Track `viewport`'s native `events`; true when one pressed a button (the pass belongs to
    /// the mouse even when the button was released within it).
    pub fn track(&mut self, viewport: egui::ViewportId, events: &[egui::Event]) -> bool {
        let mut pressed = false;
        for e in events {
            match e {
                egui::Event::PointerButton { button, pressed: true, .. } => {
                    pressed = true;
                    if !self.held.contains(&(viewport, *button)) {
                        self.held.push((viewport, *button));
                    }
                }
                egui::Event::PointerButton { button, pressed: false, .. } => self.held.retain(|h| *h != (viewport, *button)),
                // The release goes to whichever window has focus now.
                egui::Event::WindowFocused(false) => self.held.retain(|(v, _)| *v != viewport),
                _ => {}
            }
        }
        pressed
    }

    /// The mouse holds a button in some viewport.
    pub fn held(&self) -> bool {
        !self.held.is_empty()
    }

    /// The mouse holds `button` in `viewport`.
    pub fn holds(&self, viewport: egui::ViewportId, button: egui::PointerButton) -> bool {
        self.held.contains(&(viewport, button))
    }

    /// Forget buttons held in viewports that no longer exist: their release never arrives, and a
    /// stale entry would keep the mouse owning the pointer in every remaining viewport.
    pub fn retain(&mut self, viewports: &[egui::ViewportId]) {
        self.held.retain(|(viewport, _)| viewports.contains(viewport));
    }
}

/// The pen's protocol frames as UI input: each frame's events go to the viewport the pen is over
/// ([`PenRouter`]), its sample into the stylus feed in that viewport's pass ([`Self::flush`]),
/// paired with them. The canvas spreads a frame batch's samples over the pointer moves it strokes
/// with, by count (`Stylus::select`), so while the tip touches a window each of its frames brings
/// exactly one such move and one sample (a frame that only changes pressure or tilt moves in
/// place), and no sample comes without one. A logical button the mouse and the pen both hold is
/// released when the last of them lets go.
#[cfg(any(target_os = "linux", test))]
#[derive(Default)]
pub struct PenInput {
    pub router: PenRouter,
    pub mouse: MouseButtons,
    /// Pen buttons whose press went into the UI and whose release hasn't yet.
    held: Vec<PenButton>,
    /// The viewport where those buttons were pressed: their release arrives there.
    held_in: Option<egui::ViewportId>,
    /// Where the pen last was, in its surface's logical pixels.
    pos: Option<[f64; 2]>,
    /// The tip's last sample while touching: the lift's last move strokes with it.
    contact: Option<PenSample>,
    /// The stylus feed's updates waiting for each viewport's pass, in order (`None` clears it).
    samples: std::collections::HashMap<egui::ViewportId, Vec<Option<PenSample>>>,
}

#[cfg(any(target_os = "linux", test))]
impl PenInput {
    /// Take one protocol frame. `mouse`: the mouse owns the pointer in this pass
    /// ([`MouseButtons`]); the pen's motion, presses and samples are then left out, and only the
    /// releases of presses the UI got go through, unless the mouse holds the same button there.
    pub fn frame(&mut self, f: FrameOut, mouse: bool, route: PenRoute, viewports: &[egui::ViewportId]) {
        self.bound();
        let (held, held_in, pos, buttons) = (&mut self.held, &mut self.held_in, &mut self.pos, &self.mouse);
        let mut strokes = Vec::new();
        let target = self.router.route(f.events, route, viewports, |e, viewport| {
            match *e {
                PenEvent::Move { x, y } if !mouse => {
                    *pos = Some([x, y]);
                    if held.contains(&PenButton::Tip) {
                        strokes.push(viewport);
                    }
                }
                PenEvent::Button { button, pressed: true, x, y } if !mouse => {
                    *pos = Some([x, y]);
                    *held_in = Some(viewport);
                    held.push(button);
                }
                PenEvent::Button { button, pressed: false, .. } if held.contains(&button) => {
                    held.retain(|b| *b != button);
                    if held.is_empty() {
                        *held_in = None;
                    }
                    // The mouse still holds it: its release ends the press.
                    return !buttons.holds(viewport, pointer_button(button));
                }
                PenEvent::Leave => {
                    if held.is_empty() {
                        *held_in = None;
                    }
                }
                _ => return false,
            }
            true
        });
        self.samples.retain(|id, _| viewports.contains(id));
        let sample = match f.sample {
            photocraft_tablet::Update::Keep => return,
            photocraft_tablet::Update::Set(Some(_)) if mouse => return,
            photocraft_tablet::Update::Set(s) => s.zip(target).map(|(s, v)| (pen_sample(s.sanitized()), v)),
        };
        let Some((s, viewport)) = sample else {
            self.contact = None;
            self.queue(target.unwrap_or(egui::ViewportId::ROOT), None);
            return;
        };
        let strokes = strokes.iter().filter(|v| **v == viewport).count();
        if self.held.contains(&PenButton::Tip) {
            // Touching, also at pressure 0 (the stylus takes pressure 0 for no contact).
            let s = PenSample { pressure: s.pressure.max(f32::MIN_POSITIVE), ..s };
            if strokes == 0
                && let Some([x, y]) = self.pos
            {
                self.router.push(viewport, PenEvent::Move { x, y });
            }
            self.contact = Some(s);
            self.queue(viewport, Some(s));
        } else {
            if strokes > 0
                && let Some(c) = self.contact.take()
            {
                self.queue(viewport, Some(c));
            }
            self.queue(viewport, Some(s));
        }
    }

    /// Forget holds and queued input for viewports that have closed: their release can never
    /// arrive, and a stale mouse or pen hold would otherwise suppress input in the remaining
    /// viewports.
    pub fn retain_viewports(&mut self, viewports: &[egui::ViewportId]) {
        self.mouse.retain(viewports);
        if self.held_in.is_some_and(|viewport| !viewports.contains(&viewport)) {
            self.held.clear();
            self.held_in = None;
            self.contact = None;
        }
        self.router.retain(viewports);
        self.samples.retain(|id, _| viewports.contains(id));
    }

    /// Queue a stylus feed update for `viewport`'s pass ([`Self::flush`]).
    fn queue(&mut self, viewport: egui::ViewportId, s: Option<PenSample>) {
        self.samples.entry(viewport).or_default().push(s);
    }

    /// Past [`MAX_QUEUED`], a viewport's events and samples are reduced together: the events to
    /// their net effect (one final move) and the samples to the last.
    fn bound(&mut self) {
        let mut over: Vec<_> = self.router.over(MAX_QUEUED).collect();
        over.extend(self.samples.iter().filter(|(_, q)| q.len() > MAX_QUEUED).map(|(id, _)| *id));
        for id in over {
            self.router.reconcile(id);
            if let Some(q) = self.samples.get_mut(&id)
                && let Some(last) = q.pop()
            {
                q.clear();
                q.push(last);
            }
        }
    }

    /// Write the samples waiting for `viewport` into the feed, at the start of its pass (after
    /// the canvases of the passes before it took theirs). `mouse`: the mouse owns the pointer in
    /// this pass, and only the clears go in.
    pub fn flush(&mut self, viewport: egui::ViewportId, mouse: bool, feed: &StylusFeed) {
        for s in self.samples.remove(&viewport).unwrap_or_default() {
            if s.is_none() || !mouse {
                feed.set(s);
            }
        }
    }

    /// Drop `viewport`'s native button releases of buttons the pen still holds there: the pen's
    /// release ends the press.
    pub fn keep_pen_buttons(&self, viewport: egui::ViewportId, events: &mut Vec<egui::Event>) {
        if self.router.over != Some(viewport) {
            return;
        }
        events.retain(|e| match e {
            egui::Event::PointerButton { button, pressed: false, .. } => !self.held.iter().any(|b| pointer_button(*b) == *button),
            _ => true,
        });
    }
}

/// Which viewport a surface the pen enters is. eframe doesn't say which `wl_surface` an extra
/// viewport (a detached document window, the screen color picker, Camera Raw's window) has, so a
/// surface other than the main window's is taken for an extra viewport only when exactly one is
/// open and the compositor offers server-side decorations (else the surface may be a decoration
/// subsurface of any window).
/// Otherwise the pen does nothing there, and the mouse still works. A compositor that offers
/// server-side decorations but draws none for a window (a user rule forcing client-side ones)
/// defeats the check: there a pen on that window's title bar lands in the extra viewport.
#[cfg(any(target_os = "linux", test))]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PenRoute {
    pub window: u64,
    pub server_decorations: bool,
}

#[cfg(any(target_os = "linux", test))]
impl PenRoute {
    /// The viewport `surface` is, among the open `extra` viewports (`None`: not certain).
    pub fn target(&self, surface: u64, extra: &[egui::ViewportId]) -> Option<egui::ViewportId> {
        match extra {
            _ if surface == self.window => Some(egui::ViewportId::ROOT),
            [only] if self.server_decorations => Some(*only),
            _ => None,
        }
    }
}

/// The pen's events, sorted by the viewport they are for until that viewport's pass.
#[cfg(any(target_os = "linux", test))]
#[derive(Default)]
pub struct PenRouter {
    /// The viewport the pen is over (`None`: none, or a surface no viewport is known for).
    pub over: Option<egui::ViewportId>,
    pending: std::collections::HashMap<egui::ViewportId, Vec<PenEvent>>,
}

#[cfg(any(target_os = "linux", test))]
impl PenRouter {
    /// Sort `events` by viewport, those `keep` accepts. `viewports` are the open ones, the root
    /// included. Returns the viewport the last event went to, else the one the pen is over.
    pub fn route(
        &mut self,
        events: Vec<PenEvent>,
        route: PenRoute,
        viewports: &[egui::ViewportId],
        mut keep: impl FnMut(&PenEvent, egui::ViewportId) -> bool,
    ) -> Option<egui::ViewportId> {
        self.retain(viewports);
        let extra: Vec<_> = viewports.iter().copied().filter(|id| *id != egui::ViewportId::ROOT).collect();
        let mut last = None;
        for e in events {
            if let PenEvent::Enter { surface } = e {
                self.over = route.target(surface, &extra);
                continue;
            }
            let Some(target) = self.over else { continue };
            if e == PenEvent::Leave {
                self.over = None;
            }
            if keep(&e, target) {
                self.push(target, e);
                last = Some(target);
            }
        }
        last.or(self.over)
    }

    /// Forget queued events and the pen's surface for viewports that have closed.
    pub fn retain(&mut self, viewports: &[egui::ViewportId]) {
        self.pending.retain(|id, _| viewports.contains(id));
        if self.over.is_some_and(|id| !viewports.contains(&id)) {
            self.over = None;
        }
    }

    /// Queue `e` for `viewport`'s pass.
    pub fn push(&mut self, viewport: egui::ViewportId, e: PenEvent) {
        self.pending.entry(viewport).or_default().push(e);
    }

    /// The viewports with more than `max` events waiting.
    fn over(&self, max: usize) -> impl Iterator<Item = egui::ViewportId> + '_ {
        self.pending.iter().filter(move |(_, q)| q.len() > max).map(|(id, _)| *id)
    }

    /// Reduce `viewport`'s waiting events to their net effect ([`reconcile`]).
    fn reconcile(&mut self, viewport: egui::ViewportId) {
        if let Some(q) = self.pending.get_mut(&viewport) {
            reconcile(q);
        }
    }

    /// The events waiting for `viewport`'s pass, as egui events with that viewport's `modifiers`.
    pub fn take(&mut self, viewport: egui::ViewportId, zoom: f32, modifiers: egui::Modifiers) -> Vec<egui::Event> {
        self.pending.remove(&viewport).unwrap_or_default().into_iter().map(|e| pen_event(e, zoom, modifiers)).collect()
    }
}

/// The modifiers for pen clicks in a pass: the latest this pass's own input reports (a
/// modifier change, or a key or pointer event's), else the viewport's from its last pass
/// (`last`).
#[cfg(any(target_os = "linux", test))]
pub fn pass_modifiers(events: &[egui::Event], last: egui::Modifiers) -> egui::Modifiers {
    events
        .iter()
        .rev()
        .find_map(|e| match e {
            egui::Event::ModifiersChanged(modifiers) | egui::Event::Key { modifiers, .. } | egui::Event::PointerButton { modifiers, .. } => Some(*modifiers),
            _ => None,
        })
        .unwrap_or(last)
}

/// What a pass's output means for the pen's cursor.
#[cfg(any(target_os = "linux", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PassCursor {
    /// The pen is over this pass's viewport: show its cursor.
    Output,
    /// The pen is over none of the viewports: the default cursor (set once per frame, at the root).
    Default,
    /// Another viewport's pass decides.
    Keep,
}

/// What `pass`'s output means for the pen's cursor, with the pen `over` that viewport (or none).
#[cfg(any(target_os = "linux", test))]
pub fn pass_cursor(over: Option<egui::ViewportId>, pass: egui::ViewportId) -> PassCursor {
    match over {
        Some(v) if v == pass => PassCursor::Output,
        None if pass == egui::ViewportId::ROOT => PassCursor::Default,
        _ => PassCursor::Keep,
    }
}

/// The pen as egui input: positions are the surface's logical pixels, which are points times the
/// UI zoom.
#[cfg(any(target_os = "linux", test))]
pub fn pen_event(e: PenEvent, zoom: f32, modifiers: egui::Modifiers) -> egui::Event {
    let zoom = if zoom.is_finite() && zoom > 0.0 { zoom } else { 1.0 };
    let pos = |x: f64, y: f64| egui::pos2(x as f32 / zoom, y as f32 / zoom);
    match e {
        PenEvent::Move { x, y } => egui::Event::PointerMoved(pos(x, y)),
        PenEvent::Button { button, pressed, x, y } => egui::Event::PointerButton { pos: pos(x, y), button: pointer_button(button), pressed, modifiers },
        // `Enter` only routes ([`PenRouter`]); the motion that follows moves the pointer.
        PenEvent::Leave | PenEvent::Enter { .. } => egui::Event::PointerGone,
    }
}

/// The egui button a pen button presses.
#[cfg(any(target_os = "linux", test))]
pub fn pointer_button(button: PenButton) -> egui::PointerButton {
    match button {
        PenButton::Tip => egui::PointerButton::Primary,
        PenButton::Barrel => egui::PointerButton::Secondary,
        PenButton::Barrel2 => egui::PointerButton::Middle,
    }
}

/// The OS cursor the pen shows for the UI's cursor: its bitmap (the brush outline, physical
/// pixels at `scale` per logical pixel) when there is one, else the matching shape.
#[cfg(target_os = "linux")]
pub fn pen_cursor(icon: egui::CursorIcon, image: Option<&egui::CustomCursorImage>, scale: f32) -> photocraft_tablet::wayland::Cursor {
    use egui::CursorIcon as I;
    use photocraft_tablet::wayland::{Cursor as C, CursorImage};
    if let Some(i) = image {
        // A zero, negative or non-finite window scale would make the brush outline vanish,
        // blow up or re-upload every frame; treat it as 1:1 (the protocol layer sanitizes too).
        let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
        return C::Image(CursorImage { rgba: i.rgba.clone(), width: i.size[0], height: i.size[1], hotspot: i.hotspot, scale });
    }
    match icon {
        I::None => C::Hidden,
        I::Crosshair | I::Cell => C::Crosshair,
        I::Text | I::VerticalText => C::Text,
        I::PointingHand => C::Pointer,
        I::Grab => C::Grab,
        I::Grabbing => C::Grabbing,
        I::Move | I::AllScroll => C::Move,
        I::NotAllowed | I::NoDrop => C::NotAllowed,
        I::ResizeHorizontal | I::ResizeEast | I::ResizeWest | I::ResizeColumn => C::ResizeHorizontal,
        I::ResizeVertical | I::ResizeNorth | I::ResizeSouth | I::ResizeRow => C::ResizeVertical,
        I::ResizeNeSw | I::ResizeNorthEast | I::ResizeSouthWest => C::ResizeNeSw,
        I::ResizeNwSe | I::ResizeNorthWest | I::ResizeSouthEast => C::ResizeNwSe,
        I::Copy => C::Copy,
        I::Alias => C::Alias,
        I::Wait => C::Wait,
        I::Progress => C::Progress,
        I::Help => C::Help,
        I::ContextMenu => C::ContextMenu,
        I::ZoomIn => C::ZoomIn,
        I::ZoomOut => C::ZoomOut,
        _ => C::Default,
    }
}

/// Feeds the Wayland pen into the input of the viewport it is over, and that viewport's cursor
/// back to the reader. The reader's events are dispatched here, on the UI thread, at the start
/// of the root viewport's pass, through [`PenInput`]: each frame's pen sample goes into the
/// stylus feed paired with its pointer events, which go into the input of the same pass.
#[cfg(target_os = "linux")]
pub struct WaylandPen {
    reader: std::sync::Arc<std::sync::Mutex<Option<photocraft_tablet::wayland::Reader>>>,
    feed: StylusFeed,
    route: PenRoute,
    input: PenInput,
    /// The viewport of each pass in progress (immediate viewports nest inside the root's pass).
    passes: Vec<egui::ViewportId>,
}

#[cfg(target_os = "linux")]
impl WaylandPen {
    /// Read the reader's waiting frames into [`PenInput`]; `mouse`: the mouse owns the pointer.
    fn dispatch(&mut self, viewports: &[egui::ViewportId], mouse: bool) {
        let (frames, error) = {
            let mut guard = self.reader.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let Some(reader) = guard.as_mut() else { return };
            (reader.dispatch(), reader.take_error())
        };
        if let Some(e) = error {
            log::warn!("{e}; the pen stops working until PhotoCraft restarts");
        }
        for f in frames {
            self.input.frame(f, mouse, self.route, viewports);
        }
    }
}

#[cfg(target_os = "linux")]
impl egui::Plugin for WaylandPen {
    fn debug_name(&self) -> &'static str {
        "photocraft.wayland_pen"
    }

    /// Before each viewport's pass: track the mouse's buttons, dispatch the reader's frames (in the
    /// root's pass), give this viewport its pen samples and events, and keep a held pen button down
    /// through a mouse release of the same button.
    fn input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        let id = input.viewport_id;
        self.passes.push(id);
        // The native events only (the pen's are added below): a mouse press takes the feed.
        let pressed = self.input.mouse.track(id, &input.events);
        if pressed {
            self.feed.set(None);
        }
        // The root's input lists every open viewport; an extra viewport's pass runs inside the
        // root's, so its events are sorted out by then. A window that closed without delivering
        // its release must not keep the mouse or a pen button held.
        let mut mouse = pressed || self.input.mouse.held();
        if id == egui::ViewportId::ROOT {
            let viewports: Vec<_> = input.viewports.keys().copied().chain([egui::ViewportId::ROOT]).collect();
            self.input.retain_viewports(&viewports);
            mouse = pressed || self.input.mouse.held();
            self.dispatch(&viewports, mouse);
        }
        self.input.flush(id, mouse, &self.feed);
        self.input.keep_pen_buttons(id, &mut input.events);
        let modifiers = pass_modifiers(&input.events, ctx.input_for(id, |i| i.modifiers));
        input.events.extend(self.input.router.take(id, ctx.zoom_factor(), modifiers));
    }

    /// After each pass: show the pass's cursor on the pen when the pen is over that viewport (the
    /// default cursor when it is over none).
    fn output_hook(&mut self, ctx: &egui::Context, output: &mut egui::FullOutput) {
        let Some(pass) = self.passes.pop() else { return };
        let out = &output.platform_output;
        let cursor = match pass_cursor(self.input.router.over, pass) {
            // The pass's own viewport scale: egui has already left the viewport by now.
            PassCursor::Output => {
                pen_cursor(out.cursor_icon, out.cursor_image.as_ref(), ctx.input_for(pass, |i| i.viewport().native_pixels_per_point).unwrap_or(1.0))
            }
            PassCursor::Default => photocraft_tablet::wayland::Cursor::Default,
            PassCursor::Keep => return,
        };
        if let Some(reader) = self.reader.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_mut() {
            reader.set_cursor(cursor);
        }
    }
}

/// The windowing system eframe ended up on.
#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayKind {
    X11,
    Wayland,
    Other,
}

#[cfg(target_os = "linux")]
impl DisplayKind {
    /// From the creation context's display handle.
    pub fn of(cc: &eframe::CreationContext<'_>) -> Option<Self> {
        use eframe::wgpu::rwh::{HasDisplayHandle, RawDisplayHandle};
        Some(match cc.display_handle().ok()?.as_raw() {
            RawDisplayHandle::Xlib(_) | RawDisplayHandle::Xcb(_) => DisplayKind::X11,
            RawDisplayHandle::Wayland(_) => DisplayKind::Wayland,
            _ => DisplayKind::Other,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_tablet::{Update, appkit, xi};
    use photocraft_ui_egui::PhotocraftApp;
    use photocraft_ui_egui::canvas::{ToolEvent, tool_event};
    use photocraft_ui_egui::state::Tool;
    use serde_json::{Value, json};

    #[test]
    fn samples_map_and_sanitize() {
        let p = pen_sample(Sample { pressure: 0.25, tilt_x: 30.0, tilt_y: -10.0, rotation: 90.0, eraser: true });
        assert_eq!(p, PenSample { pressure: 0.25, tilt_x: 30.0, tilt_y: -10.0, rotation: 90.0, eraser: true });
        let p = pen_sample(Sample { pressure: f32::NAN, tilt_x: 500.0, tilt_y: f32::NEG_INFINITY, rotation: -45.0, eraser: false });
        assert_eq!(p, PenSample { pressure: 1.0, tilt_x: 90.0, tilt_y: 0.0, rotation: 315.0, eraser: false });
        let feed = StylusFeed::default();
        let f = sink(feed.clone());
        f(Some(Sample { pressure: 0.5, ..Default::default() }));
        assert_eq!(feed.get().map(|p| p.pressure), Some(0.5));
        f(None);
        assert_eq!(feed.get(), None);
    }

    /// The platform path end to end, minus the OS: raw AppKit / XInput2 values → the crate's
    /// mapping → this module's sink → the stylus feed → the canvas's tool events → `paint.stroke`.
    /// A light-to-heavy pen stroke must paint thin and faint at the light end and wide and opaque
    /// at the heavy end (Size and Opacity on Pen Pressure); a mouse paints full strength.
    fn pressure_stroke(mut next: impl FnMut(f32) -> Option<Sample>) -> (f64, f64, f64, f64) {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let feed = app.stylus.feed.clone();
        let sink = sink(feed);
        app.session.execute("file.new", json!({"width": 240, "height": 80, "background": "white"})).unwrap();
        app.session.execute("tools.setColors", json!({"foreground": [0.0, 0.0, 0.0, 1.0]})).unwrap();
        let brush = json!({"size": 30, "hardness": 1.0, "spacing": 0.05, "pressureSize": true, "pressureOpacity": true, "smoothing": {"amount": 0}});
        app.session.execute("tools.setBrush", json!({ "brush": brush })).unwrap();
        app.ui.tool = Tool::Brush;
        let m = egui::Modifiers::NONE;
        let n = 20;
        for i in 0..=n {
            let t = i as f32 / n as f32;
            sink(next(0.1 + 0.9 * t));
            let (x, y) = (20.0 + 200.0 * f64::from(t), 40.0);
            let pressure = app.stylus.pressure();
            let ev = if i == 0 { ToolEvent::Down { x, y, pressure } } else { ToolEvent::Move { x, y, pressure } };
            tool_event(&mut app, ev, m);
        }
        tool_event(&mut app, ToolEvent::Up { x: 220.0, y: 40.0 }, m);
        sink(None);
        // Darkness (1 - luma) along x = 50 (light end) and x = 200 (heavy end), and the painted
        // half-width there.
        let mut dark = |x: i32, y: i32| -> f64 {
            let px: Value = app.session.execute("document.pixel", json!({"x": x, "y": y})).unwrap();
            1.0 - px[0].as_f64().unwrap()
        };
        let (light, heavy) = (dark(50, 40), dark(200, 40));
        let light_w = (0..40).take_while(|d| dark(50, 40 + d) > 0.02).count() as f64;
        let heavy_w = (0..40).take_while(|d| dark(200, 40 + d) > 0.02).count() as f64;
        (light, light_w, heavy, heavy_w)
    }

    #[test]
    fn appkit_tablet_pressure_drives_stroke_width_and_opacity() {
        let mut st = appkit::State::default();
        let (light_dark, light_w, heavy_dark, heavy_w) = pressure_stroke(|p| {
            let e =
                appkit::RawEvent { kind: appkit::event_type::LEFT_MOUSE_DRAGGED, subtype: appkit::subtype::TABLET_POINT, pressure: p, ..Default::default() };
            match st.handle(&e) {
                Update::Set(s) => s,
                Update::Keep => None,
            }
        });
        assert!(heavy_w > light_w * 2.0, "width responds: {light_w} → {heavy_w}");
        assert!(heavy_dark > light_dark + 0.3, "opacity responds: {light_dark} → {heavy_dark}");
        assert!(heavy_dark > 0.9, "full pressure paints (nearly) opaque: {heavy_dark}");

        // A mouse (non-tablet subtype) through the same path: full size and opacity throughout.
        let mut st = appkit::State::default();
        let (m_dark, m_w, _, _) = pressure_stroke(|p| {
            let e = appkit::RawEvent { kind: appkit::event_type::LEFT_MOUSE_DRAGGED, subtype: appkit::subtype::MOUSE, pressure: p, ..Default::default() };
            match st.handle(&e) {
                Update::Set(s) => s,
                Update::Keep => None,
            }
        });
        assert!(m_dark > 0.9 && m_w >= heavy_w - 1.0, "mouse = pressure 1: {m_dark}, {m_w}");
    }

    #[test]
    fn xinput_tablet_pressure_drives_stroke_width_and_opacity() {
        let mut st = xi::State::default();
        st.set_devices([xi::Device::new(9, "Wacom Intuos Pen stylus", [(2, "Abs Pressure".to_string(), 0.0, 65535.0)])]);
        let (light_dark, light_w, heavy_dark, heavy_w) = pressure_stroke(|p| match st.handle(9, [(2, f64::from(p) * 65535.0)]) {
            Update::Set(s) => s,
            Update::Keep => None,
        });
        assert!(heavy_w > light_w * 2.0, "width responds: {light_w} → {heavy_w}");
        assert!(heavy_dark > light_dark + 0.3, "opacity responds: {light_dark} → {heavy_dark}");
    }

    /// `/proc/bus/input/devices` of a laptop with a Wacom Intuos Pro attached (64-bit longs: the
    /// pen's `KEY=1c03 0 0 0 0 0` has bit 0x140, BTN_TOOL_PEN) and of the same laptop without it.
    const WITH_PEN: &str = "I: Bus=0011 Vendor=0001 Product=0001 Version=ab83\n\
        N: Name=\"AT Translated Set 2 keyboard\"\n\
        H: Handlers=sysrq kbd event3 leds\n\
        B: KEY=402000000 3803078f800d001 feffffdfffefffff fffffffffffffffe\n\
        \n\
        I: Bus=0018 Vendor=06cb Product=ce26 Version=0100\n\
        N: Name=\"SYNA2B33:00 06CB:CE26 Touchpad\"\n\
        H: Handlers=mouse1 event7\n\
        B: KEY=e520 10000 0 0 0 0\n\
        B: ABS=2e0800000000003\n\
        \n\
        I: Bus=0003 Vendor=056a Product=0357 Version=0110\n\
        N: Name=\"Wacom Intuos Pro M Pen\"\n\
        H: Handlers=mouse2 event10\n\
        B: KEY=1c03 0 0 0 0 0\n\
        B: ABS=1000f000003\n";

    #[test]
    fn a_pen_is_found_by_its_btn_tool_pen_key() {
        assert!(has_pen(WITH_PEN));
        let without = WITH_PEN.split("\n\n").take(2).collect::<Vec<_>>().join("\n\n");
        assert!(!has_pen(&without), "a touchpad's BTN_TOOL_FINGER (0x145) is not a pen");
        // Short or malformed bitmaps are no pen, never a panic.
        for keys in ["", "B: KEY=", "B: KEY=1", "B: KEY=zz 0 0 0 0 0", "B: KEY=ffffffffffffffffff 0 0 0 0 0", "B: KEY=0 0 0 0 0 0 0 0"] {
            assert!(!has_pen(keys), "{keys:?}");
        }
        for abs in ["B: ABS=", "B: ABS=zz", "B: ABS=ffffffffffffffffff", "B: ABS=0 0"] {
            assert!(!has_pen(&format!("B: KEY=1c03 0 0 0 0 0\n{abs}")), "{abs:?}");
        }
    }

    /// #1072: RustDesk's uinput keyboard claims every key code, `BTN_TOOL_PEN` included, and has
    /// no absolute axes (`EV=20013`: no `EV_ABS`, no `B: ABS=` line). It is no pen, so the window
    /// stays on native Wayland, where the fractional display scale renders sharp. The bitmaps are
    /// the device's own, from `/proc/bus/input/devices`.
    const VIRTUAL_KEYBOARD: &str = "I: Bus=0003 Vendor=1234 Product=5678 Version=0111\n\
        N: Name=\"RustDesk UInput Keyboard\"\n\
        P: Phys=\n\
        S: Sysfs=/devices/virtual/input/input54\n\
        U: Uniq=\n\
        H: Handlers=sysrq rfkill kbd event256 leds \n\
        B: PROP=0\n\
        B: EV=20013\n\
        B: KEY=ffffffffff 0 7ffffff000301ff 1000f7fffffff 7fe001fffff000f 7ffffffffffffff ffffffff0003fdff 7fff8fff00ff03ff 1ffffffffffff07 ffffffffffffffff ffffffffffefffff fffffffffffffffe\n\
        B: MSC=10\n\
        B: LED=7\n";

    /// A virtual keyboard that claims every key, `BTN_TOOL_PEN` included, is no pen.
    #[test]
    fn a_virtual_keyboard_claiming_btn_tool_pen_is_no_pen() {
        assert!(
            bit_set(
                "ffffffffff 0 7ffffff000301ff 1000f7fffffff 7fe001fffff000f 7ffffffffffffff ffffffff0003fdff 7fff8fff00ff03ff 1ffffffffffff07 ffffffffffffffff ffffffffffefffff fffffffffffffffe",
                BTN_TOOL_PEN
            ),
            "the fixture claims the key"
        );
        assert!(!has_pen(VIRTUAL_KEYBOARD));
        // Beside a real pen, the pen is still found; the keyboard's key bits never leak into the
        // pen's device and the pen's axes never into the keyboard's.
        let both = format!("{VIRTUAL_KEYBOARD}\n{WITH_PEN}");
        assert!(has_pen(&both));
        let pen_without_axes = WITH_PEN.lines().filter(|l| !l.starts_with("B: ABS=1000f")).collect::<Vec<_>>().join("\n");
        assert!(!has_pen(&pen_without_axes), "a pen reports absolute positions");
        // A pen display or pen-enabled touchscreen reports the pen with its own axes.
        let pen_display = "N: Name=\"Wacom One Pen\"\nB: KEY=1c03 0 0 0 0 0\nB: ABS=3\n";
        assert!(has_pen(pen_display));
        // Axes without `ABS_X` or `ABS_Y` don't count.
        assert!(!has_pen("B: KEY=1c03 0 0 0 0 0\nB: ABS=1000f000002\n"));
        assert!(!has_pen("B: KEY=1c03 0 0 0 0 0\nB: ABS=1000f000001\n"));
    }

    /// The window opens through Xwayland only for a pen on a compositor without tablet-v2, unless
    /// the environment says otherwise.
    #[test]
    fn a_pen_on_wayland_without_the_tablet_protocol_opens_the_window_through_xwayland() {
        use std::collections::HashMap;
        let env = |pairs: &[(&str, &str)]| {
            let m: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            move |k: &str| m.get(k).cloned()
        };
        let wayland = [("WAYLAND_DISPLAY", "wayland-0"), ("DISPLAY", ":0")];
        assert!(pen_needs_xwayland(DisplaySession::Wayland, env(&wayland), || true, || false, || true));
        // No pen, no Xwayland, no X11 libraries, or the user keeps Wayland: native Wayland.
        assert!(!pen_needs_xwayland(DisplaySession::Wayland, env(&wayland), || false, || false, || true));
        assert!(!pen_needs_xwayland(DisplaySession::Wayland, env(&[("WAYLAND_DISPLAY", "wayland-0")]), || true, || false, || true));
        assert!(!pen_needs_xwayland(DisplaySession::Wayland, env(&wayland), || true, || false, || false));
        let native = [("WAYLAND_DISPLAY", "wayland-0"), ("DISPLAY", ":0"), ("PHOTOCRAFT_NATIVE_WAYLAND", "1")];
        assert!(!pen_needs_xwayland(DisplaySession::Wayland, env(&native), || true, || false, || true));
        let zero = [("WAYLAND_DISPLAY", "wayland-0"), ("DISPLAY", ":0"), ("PHOTOCRAFT_NATIVE_WAYLAND", "0")];
        assert!(pen_needs_xwayland(DisplaySession::Wayland, env(&zero), || true, || false, || true));
        // A compositor with the tablet protocol keeps native Wayland, unless the user asks for
        // Xwayland; the compositor is only asked once a pen is found.
        assert!(!pen_needs_xwayland(DisplaySession::Wayland, env(&wayland), || true, || true, || true));
        assert!(!pen_needs_xwayland(DisplaySession::Wayland, env(&wayland), || false, || panic!("probed"), || true));
        let forced = [("WAYLAND_DISPLAY", "wayland-0"), ("DISPLAY", ":0"), ("PHOTOCRAFT_XWAYLAND_PEN", "1")];
        assert!(pen_needs_xwayland(DisplaySession::Wayland, env(&forced), || true, || panic!("probed"), || true));
        // X11 already gets the pen; the probes don't even run off Wayland.
        assert!(!pen_needs_xwayland(DisplaySession::X11, env(&[("DISPLAY", ":0")]), || panic!("probed"), || panic!("probed"), || panic!("probed")));
    }

    /// Pen events map to egui pointer events in points, with the window's modifiers.
    #[test]
    fn wayland_pen_events_become_egui_pointer_events_in_points() {
        let m = egui::Modifiers::SHIFT;
        assert_eq!(pen_event(PenEvent::Move { x: 300.0, y: 150.0 }, 1.5, m), egui::Event::PointerMoved(egui::pos2(200.0, 100.0)));
        let press = pen_event(PenEvent::Button { button: PenButton::Tip, pressed: true, x: 10.0, y: 20.0 }, 1.0, m);
        assert_eq!(press, egui::Event::PointerButton { pos: egui::pos2(10.0, 20.0), button: egui::PointerButton::Primary, pressed: true, modifiers: m });
        let barrel = |button| match pen_event(PenEvent::Button { button, pressed: false, x: 0.0, y: 0.0 }, 1.0, m) {
            egui::Event::PointerButton { button, .. } => Some(button),
            _ => None,
        };
        assert_eq!(barrel(PenButton::Barrel), Some(egui::PointerButton::Secondary));
        assert_eq!(barrel(PenButton::Barrel2), Some(egui::PointerButton::Middle));
        assert_eq!(pen_event(PenEvent::Leave, 1.0, m), egui::Event::PointerGone);
        // A broken zoom factor never divides by zero.
        for zoom in [0.0, -1.0, f32::NAN] {
            assert_eq!(pen_event(PenEvent::Move { x: 4.0, y: 8.0 }, zoom, m), egui::Event::PointerMoved(egui::pos2(4.0, 8.0)));
        }
    }

    /// The pen's bitmap cursor keeps its pixels and hotspot, and a broken window scale is 1:1.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_pen_cursor_uses_the_ui_bitmap_and_sanitizes_scale() {
        use photocraft_tablet::wayland::{Cursor, CursorImage};
        let image = egui::CustomCursorImage { rgba: vec![255, 0, 0, 255].into(), size: [1, 1], hotspot: [0, 0] };
        for scale in [1.5, 2.0] {
            let Cursor::Image(got) = pen_cursor(egui::CursorIcon::Default, Some(&image), scale) else { panic!("a bitmap") };
            assert_eq!((got.width, got.height, got.hotspot), (1, 1, [0, 0]));
            assert_eq!(got.scale, scale);
        }
        for scale in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            let Cursor::Image(CursorImage { scale: got, .. }) = pen_cursor(egui::CursorIcon::Default, Some(&image), scale) else { panic!("a bitmap") };
            assert_eq!(got, 1.0, "{scale}");
        }
        assert_eq!(pen_cursor(egui::CursorIcon::Crosshair, None, 2.0), Cursor::Crosshair);
        assert_eq!(pen_cursor(egui::CursorIcon::None, None, 2.0), Cursor::Hidden);
    }

    fn tip(pressed: bool, x: f64) -> PenEvent {
        PenEvent::Button { button: PenButton::Tip, pressed, x, y: 0.0 }
    }

    fn pen_frame(events: Vec<PenEvent>, pressure: Option<f32>) -> FrameOut {
        let sample = photocraft_tablet::Sample { pressure: pressure.unwrap_or(0.0), tilt_x: 0.0, tilt_y: 0.0, rotation: 0.0, eraser: false };
        FrameOut { events, sample: photocraft_tablet::Update::Set(Some(sample)) }
    }

    /// One pass of the root viewport through [`PenInput`], as `WaylandPen::input_hook` runs it:
    /// the native events, then the pen frames, into a real egui context.
    fn pass(ctx: &egui::Context, input: &mut PenInput, feed: &StylusFeed, native: Vec<egui::Event>, frames: Vec<FrameOut>) -> Vec<egui::Event> {
        let root = egui::ViewportId::ROOT;
        let route = PenRoute { window: 1, server_decorations: true };
        let pressed = input.mouse.track(root, &native);
        if pressed {
            feed.set(None);
        }
        let mouse = pressed || input.mouse.held();
        for f in frames {
            input.frame(f, mouse, route, &[root]);
        }
        input.flush(root, mouse, feed);
        let mut events = native;
        input.keep_pen_buttons(root, &mut events);
        events.extend(input.router.take(root, 1.0, egui::Modifiers::NONE));
        let raw = egui::RawInput { events: events.clone(), ..Default::default() };
        ctx.run_ui(raw, |_| {}).textures_delta.clear();
        events
    }

    /// A mouse button event.
    fn mouse(pressed: bool, button: egui::PointerButton) -> egui::Event {
        egui::Event::PointerButton { pos: egui::pos2(5.0, 5.0), button, pressed, modifiers: egui::Modifiers::NONE }
    }

    /// A pen tap while the mouse drags neither presses nor releases the mouse's button, and leaves
    /// the feed to the mouse.
    #[test]
    fn a_pen_tap_during_a_mouse_drag_never_releases_the_mouse() {
        use egui::PointerButton::Primary;
        let (ctx, feed, mut input) = (egui::Context::default(), StylusFeed::default(), PenInput::default());
        let enter = PenEvent::Enter { surface: 1 };
        // The pen's press and release in the pass of the mouse press, then in later passes.
        let events = pass(&ctx, &mut input, &feed, vec![mouse(true, Primary)], vec![pen_frame(vec![enter, tip(true, 1.0), tip(false, 1.0)], Some(0.5))]);
        assert_eq!(events, vec![mouse(true, Primary)], "neither the pen's press nor its release");
        pass(&ctx, &mut input, &feed, vec![], vec![pen_frame(vec![tip(true, 2.0)], Some(0.5))]);
        pass(&ctx, &mut input, &feed, vec![], vec![pen_frame(vec![tip(false, 2.0)], Some(0.0))]);
        assert!(ctx.input(|i| i.pointer.primary_down()), "the mouse still holds its button");
        assert_eq!(feed.get(), None, "and the feed: the mouse paints at full pressure");
        // A pen press the UI got still gets its release while the mouse holds a button.
        pass(&ctx, &mut input, &feed, vec![mouse(false, Primary)], vec![]);
        pass(&ctx, &mut input, &feed, vec![], vec![pen_frame(vec![tip(true, 3.0)], Some(0.5))]);
        let events = pass(&ctx, &mut input, &feed, vec![mouse(true, egui::PointerButton::Secondary)], vec![pen_frame(vec![tip(false, 3.0)], Some(0.0))]);
        assert!(events.contains(&egui::Event::PointerButton { pos: egui::pos2(3.0, 0.0), button: Primary, pressed: false, modifiers: egui::Modifiers::NONE }));
    }

    /// The mouse owns the pointer while one of its buttons is held, or in a pass it clicked in.
    #[test]
    fn the_mouse_owns_the_pointer_while_any_of_its_buttons_is_held_or_in_a_pass_it_clicked() {
        use egui::PointerButton::{Primary, Secondary};
        let root = egui::ViewportId::ROOT;
        let picker = egui::ViewportId::from_hash_of("picker");
        let mut m = MouseButtons::default();
        // Chording: the Secondary click leaves Primary held.
        assert!(m.track(root, &[mouse(true, Primary)]));
        assert!(m.track(root, &[mouse(true, Secondary), mouse(false, Secondary)]));
        assert!(m.held());
        // Another window losing focus doesn't end this one's drag; this one losing it does.
        assert!(!m.track(picker, &[egui::Event::WindowFocused(false)]));
        assert!(m.held());
        m.track(root, &[egui::Event::WindowFocused(false)]);
        assert!(!m.held(), "the release goes to the window with focus");

        // A whole click in one pass: the pen's hover in that pass doesn't undo the full pressure.
        let (ctx, feed, mut input) = (egui::Context::default(), StylusFeed::default(), PenInput::default());
        pass(&ctx, &mut input, &feed, vec![], vec![pen_frame(vec![PenEvent::Enter { surface: 1 }, PenEvent::Move { x: 1.0, y: 1.0 }], None)]);
        assert_eq!(feed.get().map(|s| s.pressure), Some(0.0), "the pen hovers");
        let events =
            pass(&ctx, &mut input, &feed, vec![mouse(true, Primary), mouse(false, Primary)], vec![pen_frame(vec![PenEvent::Move { x: 2.0, y: 2.0 }], None)]);
        assert_eq!(feed.get(), None);
        assert_eq!(events, vec![mouse(true, Primary), mouse(false, Primary)], "no pen motion either");
        // The next pass is the pen's again.
        pass(&ctx, &mut input, &feed, vec![], vec![pen_frame(vec![PenEvent::Move { x: 3.0, y: 3.0 }], None)]);
        assert_eq!(feed.get().map(|s| s.pressure), Some(0.0));
    }

    /// A viewport that closes while the mouse or a pen button is held never delivers its release.
    /// Its stale hold must not keep the mouse owning the pointer in the remaining viewports.
    #[test]
    fn a_closed_viewport_clears_mouse_and_pen_holds() {
        use egui::PointerButton::Primary;
        let (root, doc) = (egui::ViewportId::ROOT, egui::ViewportId::from_hash_of("docwin"));
        let mut m = MouseButtons::default();
        m.track(doc, &[mouse(true, Primary)]);
        assert!(m.held());
        m.retain(&[root]);
        assert!(!m.held(), "the closed window's mouse hold is gone");

        let mut input = PenInput::default();
        let route = PenRoute { window: 1, server_decorations: true };
        input.frame(pen_frame(vec![PenEvent::Enter { surface: 2 }, PenEvent::Move { x: 1.0, y: 1.0 }, tip(true, 1.0)], Some(0.5)), false, route, &[root, doc]);
        assert!(input.held.contains(&PenButton::Tip));
        assert_eq!(input.held_in, Some(doc));
        input.retain_viewports(&[root]);
        assert!(input.held.is_empty(), "the closed window's pen hold is gone");
        assert_eq!(input.held_in, None);
        assert_eq!(input.router.over, None, "the pen is no longer over the closed viewport");
    }

    /// The canvas's stroke moves: `PointerMoved` while the tip is down (canvas `pointer_moves`).
    fn stroke_moves(events: &[egui::Event], mut down: bool) -> Vec<f32> {
        let mut out = Vec::new();
        for e in events {
            match e {
                egui::Event::PointerButton { button: egui::PointerButton::Primary, pressed, .. } => down = *pressed,
                egui::Event::PointerMoved(p) if down => out.push(p.x),
                _ => {}
            }
        }
        out
    }

    /// Each stroke move the pen's frames bring pairs with that frame's own sample, a lift's last
    /// move with the pen's last contact pressure.
    #[test]
    fn each_stroke_move_gets_the_pressure_of_its_own_protocol_frame() {
        let (ctx, feed, mut input) = (egui::Context::default(), StylusFeed::default(), PenInput::default());
        let enter = PenEvent::Enter { surface: 1 };
        // Hover, press (its frame has no stroke move of its own), a move, a pressure change in
        // place, another move and the lift with a last move, all in one UI frame.
        let frames = vec![
            pen_frame(vec![enter, PenEvent::Move { x: 1.0, y: 0.0 }], Some(0.0)),
            pen_frame(vec![PenEvent::Move { x: 2.0, y: 0.0 }, tip(true, 2.0)], Some(0.2)),
            pen_frame(vec![PenEvent::Move { x: 10.0, y: 0.0 }], Some(0.1)),
            pen_frame(vec![], Some(1.0)),
            pen_frame(vec![PenEvent::Move { x: 20.0, y: 0.0 }], Some(0.9)),
            pen_frame(vec![PenEvent::Move { x: 25.0, y: 0.0 }, tip(false, 25.0)], Some(0.0)),
        ];
        let events = pass(&ctx, &mut input, &feed, vec![], frames);
        let samples: Vec<f32> = feed.drain().iter().map(|s| s.pressure).filter(|p| *p > 0.0).collect();
        assert_eq!(stroke_moves(&events, false), vec![2.0, 10.0, 10.0, 20.0, 25.0]);
        assert_eq!(samples, vec![0.2, 0.1, 1.0, 0.9, 0.9], "one sample per stroke move, in order: the lift's move keeps the pen's last pressure");
        assert_eq!(feed.get().map(|s| s.pressure), Some(0.0), "hovering after the lift");
        // A touch the tablet reports at pressure 0 is still a touch, with its own sample.
        let events = pass(
            &ctx,
            &mut input,
            &feed,
            vec![],
            vec![pen_frame(vec![tip(true, 30.0)], Some(0.0)), pen_frame(vec![PenEvent::Move { x: 31.0, y: 0.0 }], Some(0.0))],
        );
        let samples = feed.drain().iter().filter(|s| s.pressure > 0.0).count();
        assert_eq!((stroke_moves(&events, false).len(), samples), (2, 2));
        // Off the root window the samples wait for the other window's pass; on no known window
        // there are none.
        let other = egui::ViewportId::from_hash_of("picker");
        let route = PenRoute { window: 1, server_decorations: true };
        let hover = |surface| pen_frame(vec![PenEvent::Enter { surface }, PenEvent::Move { x: 1.0, y: 1.0 }], Some(0.0));
        feed.set(None);
        input.frame(hover(7), false, route, &[egui::ViewportId::ROOT, other]);
        input.flush(egui::ViewportId::ROOT, false, &feed);
        assert_eq!(feed.get(), None, "unchanged by the root's pass");
        input.flush(other, false, &feed);
        assert!(feed.get().is_some(), "the other window's pass gets its sample");
        input.frame(hover(7), false, route, &[egui::ViewportId::ROOT, other, egui::ViewportId::from_hash_of("tool")]);
        input.flush(egui::ViewportId::ROOT, false, &feed);
        assert_eq!(feed.get(), None);
    }

    /// The pen paints in a detached document window with its pressure and tilt: the window's
    /// samples go into the feed in its own pass, after the root's canvas took the root's, paired
    /// with its stroke moves.
    #[test]
    fn a_detached_document_window_gets_the_pens_pressure_and_tilt() {
        let (root, doc) = (egui::ViewportId::ROOT, egui::ViewportId::from_hash_of(("docwin", 1)));
        let (feed, mut input) = (StylusFeed::default(), PenInput::default());
        let route = PenRoute { window: 1, server_decorations: true };
        let frame = |events, pressure: f32| FrameOut {
            events,
            sample: photocraft_tablet::Update::Set(Some(Sample { pressure, tilt_x: 30.0, tilt_y: -10.0, rotation: 0.0, eraser: false })),
        };
        let frames = [
            frame(vec![PenEvent::Enter { surface: 2 }, PenEvent::Move { x: 1.0, y: 0.0 }], 0.0),
            frame(vec![tip(true, 1.0)], 0.25),
            frame(vec![PenEvent::Move { x: 5.0, y: 0.0 }], 0.5),
        ];
        for f in frames {
            input.frame(f, false, route, &[root, doc]);
        }
        // The root's pass: nothing for it.
        input.flush(root, false, &feed);
        assert!(input.router.take(root, 1.0, egui::Modifiers::NONE).is_empty());
        assert!(feed.drain().is_empty() && feed.get().is_none());
        // The window's pass.
        input.flush(doc, false, &feed);
        let events = input.router.take(doc, 1.0, egui::Modifiers::NONE);
        assert_eq!(stroke_moves(&events, false), vec![1.0, 5.0], "the press's frame moves in place");
        let touching: Vec<_> = feed.drain().into_iter().filter(|s| s.pressure > 0.0).collect();
        assert_eq!(touching.iter().map(|s| s.pressure).collect::<Vec<_>>(), vec![0.25, 0.5], "one sample per stroke move");
        assert!(touching.iter().all(|s| s.tilt_x == 30.0 && s.tilt_y == -10.0));
        assert_eq!(feed.get().map(|s| s.pressure), Some(0.5));

        // A canvas in the window strokes with them (`Stylus::select` over its moves).
        let mut stylus = photocraft_ui_egui::stylus::Stylus::default();
        stylus.feed = feed.clone();
        input.frame(frame(vec![PenEvent::Move { x: 9.0, y: 0.0 }], 0.75), false, route, &[root, doc]);
        input.flush(doc, false, &feed);
        stylus.update_for_frame(doc, 1, &[]);
        assert_eq!(stylus.sample().map(|s| (s.pressure, s.tilt_x)), Some((0.75, 30.0)));
    }

    /// A mouse press in a detached window's own pass comes after the root's pass routed the pen's
    /// frames: the window's waiting samples stay out of the mouse's stroke, and clears still go in.
    #[test]
    fn a_mouse_press_in_a_windows_pass_keeps_that_windows_pen_samples_out() {
        let (root, doc) = (egui::ViewportId::ROOT, egui::ViewportId::from_hash_of(("docwin", 1)));
        let (feed, mut input) = (StylusFeed::default(), PenInput::default());
        let route = PenRoute { window: 1, server_decorations: true };
        input.frame(pen_frame(vec![PenEvent::Enter { surface: 2 }, PenEvent::Move { x: 1.0, y: 0.0 }], Some(0.0)), false, route, &[root, doc]);
        input.flush(doc, true, &feed);
        assert!(feed.drain().is_empty() && feed.get().is_none(), "no pen sample in the mouse's pass");
        input.frame(pen_frame(vec![PenEvent::Move { x: 2.0, y: 0.0 }], Some(0.0)), false, route, &[root, doc]);
        input.flush(doc, false, &feed);
        assert!(feed.get().is_some());
        input.frame(FrameOut { events: vec![PenEvent::Leave], sample: photocraft_tablet::Update::Set(None) }, false, route, &[root, doc]);
        input.flush(doc, true, &feed);
        assert_eq!(feed.get(), None, "the pen's departure clears the feed even so");
    }

    /// A logical button the mouse and the pen both hold stays down until the last of them lets
    /// go, whichever pressed first: the tip and the left button, the barrel and the right.
    #[test]
    fn a_button_the_mouse_and_the_pen_both_hold_is_released_by_the_last() {
        use egui::PointerButton::{Primary, Secondary};
        for (pen, button) in [(PenButton::Tip, Primary), (PenButton::Barrel, Secondary)] {
            let pen_button = |pressed, x| PenEvent::Button { button: pen, pressed, x, y: 0.0 };
            let down = |ctx: &egui::Context| ctx.input(|i| i.pointer.button_down(button));
            // The pen first: the mouse's release leaves it to the pen's.
            let (ctx, feed, mut input) = (egui::Context::default(), StylusFeed::default(), PenInput::default());
            pass(&ctx, &mut input, &feed, vec![], vec![pen_frame(vec![PenEvent::Enter { surface: 1 }, pen_button(true, 1.0)], Some(0.5))]);
            pass(&ctx, &mut input, &feed, vec![mouse(true, button)], vec![]);
            let events = pass(&ctx, &mut input, &feed, vec![mouse(false, button)], vec![]);
            assert!(!events.contains(&mouse(false, button)) && down(&ctx), "{button:?}: the pen still holds it");
            pass(&ctx, &mut input, &feed, vec![], vec![pen_frame(vec![pen_button(false, 1.0)], Some(0.0))]);
            assert!(!down(&ctx), "{button:?}: released by the pen");

            // The mouse holds it when the pen lets go: the mouse's drag goes on.
            let (ctx, feed, mut input) = (egui::Context::default(), StylusFeed::default(), PenInput::default());
            pass(&ctx, &mut input, &feed, vec![], vec![pen_frame(vec![PenEvent::Enter { surface: 1 }, pen_button(true, 1.0)], Some(0.5))]);
            pass(&ctx, &mut input, &feed, vec![mouse(true, button)], vec![]);
            pass(&ctx, &mut input, &feed, vec![], vec![pen_frame(vec![pen_button(false, 1.0)], Some(0.0))]);
            assert!(input.mouse.held() && down(&ctx), "{button:?}: the mouse still holds it");
            pass(&ctx, &mut input, &feed, vec![mouse(false, button)], vec![]);
            assert!(!down(&ctx), "{button:?}: released by the mouse");
        }
    }

    /// A reduced backlog keeps each button's net transitions, the final position and a departure.
    #[test]
    fn a_backlog_is_bounded_and_keeps_its_net_effect() {
        let mut q = Vec::new();
        for i in 0..20_000 {
            q.push(tip(i % 2 == 0, i as f64));
            q.push(PenEvent::Move { x: i as f64, y: 1.0 });
        }
        reconcile(&mut q);
        // Pressed first, released last (19,999 is odd), ending where the pen stopped.
        assert_eq!(q, vec![tip(true, 0.0), PenEvent::Move { x: 19_999.0, y: 1.0 }, tip(false, 19_999.0)]);
        // A press without its release keeps the pen down; a departure stays last.
        let mut q = vec![tip(true, 0.0), tip(false, 1.0), tip(true, 2.0), PenEvent::Leave];
        reconcile(&mut q);
        assert_eq!(q, vec![tip(true, 0.0), PenEvent::Leave]);
        let barrel = |pressed| PenEvent::Button { button: PenButton::Barrel, pressed, x: 0.0, y: 0.0 };
        let mut q = vec![barrel(false), tip(true, 0.0), barrel(true), tip(false, 1.0), barrel(false)];
        reconcile(&mut q);
        assert_eq!(q, vec![barrel(false), tip(true, 0.0), tip(false, 1.0)]);
    }

    /// A stroke whose window stops taking its input: the events and the samples waiting for it
    /// stay bounded, and are reduced together, so each stroke move still has its own sample.
    #[test]
    fn a_stalled_windows_backlog_keeps_moves_and_samples_paired() {
        let root = egui::ViewportId::ROOT;
        let (feed, mut input) = (StylusFeed::default(), PenInput::default());
        let route = PenRoute { window: 1, server_decorations: true };
        input.frame(pen_frame(vec![PenEvent::Enter { surface: 1 }, PenEvent::Move { x: 0.0, y: 0.0 }, tip(true, 0.0)], Some(0.5)), false, route, &[root]);
        for i in 1..10_000 {
            input.frame(pen_frame(vec![PenEvent::Move { x: f64::from(i), y: 0.0 }], Some(0.5)), false, route, &[root]);
            assert!(input.samples.get(&root).map_or(0, Vec::len) <= MAX_QUEUED + 2);
            assert!(input.router.pending.get(&root).map_or(0, Vec::len) <= MAX_QUEUED + 2);
        }
        input.flush(root, false, &feed);
        let events = input.router.take(root, 1.0, egui::Modifiers::NONE);
        let samples = feed.drain().iter().filter(|s| s.pressure > 0.0).count();
        let moves = stroke_moves(&events, false);
        assert_eq!(moves.len(), samples, "one sample per stroke move");
        assert_eq!(moves.last(), Some(&9_999.0), "ending where the pen is");
    }

    /// The pen goes to the main window, to the only extra viewport, or to none when unsure.
    #[test]
    fn the_pen_goes_to_the_viewport_it_is_over_only_when_that_is_certain() {
        use egui::ViewportId as V;
        let doc = V::from_hash_of("docwin");
        let other = V::from_hash_of("picker");
        let route = PenRoute { window: 100, server_decorations: true };
        assert_eq!(route.target(100, &[doc, other]), Some(V::ROOT));
        assert_eq!(route.target(200, &[doc]), Some(doc), "the only extra viewport");
        assert_eq!(route.target(200, &[doc, other]), None, "which one is unknown");
        assert_eq!(route.target(200, &[]), None, "no extra viewport: a subsurface of the main window");
        let csd = PenRoute { server_decorations: false, ..route };
        assert_eq!(csd.target(200, &[doc]), None, "could be a decoration subsurface");

        let m = egui::Modifiers::NONE;
        let mut r = PenRouter::default();
        let stroke = |surface| vec![PenEvent::Enter { surface }, PenEvent::Move { x: 2.0, y: 4.0 }, PenEvent::Leave];
        r.route([stroke(200), stroke(100)].concat(), route, &[V::ROOT, doc], |_, _| true);
        assert_eq!(r.take(doc, 2.0, m), vec![egui::Event::PointerMoved(egui::pos2(1.0, 2.0)), egui::Event::PointerGone]);
        assert_eq!(r.take(V::ROOT, 2.0, m), vec![egui::Event::PointerMoved(egui::pos2(1.0, 2.0)), egui::Event::PointerGone]);
        assert_eq!(r.over, None);
        // Unroutable surfaces get nothing; events before any Enter go nowhere.
        r.route([vec![PenEvent::Move { x: 1.0, y: 1.0 }], stroke(300)].concat(), route, &[V::ROOT, doc, other], |_, _| true);
        assert!(r.take(V::ROOT, 1.0, m).is_empty() && r.take(doc, 1.0, m).is_empty() && r.take(other, 1.0, m).is_empty());
        // A viewport that closed while the pen was over it drops its events and the pen.
        r.route(vec![PenEvent::Enter { surface: 200 }, PenEvent::Move { x: 1.0, y: 1.0 }], route, &[V::ROOT, doc], |_, _| true);
        r.route(vec![PenEvent::Move { x: 2.0, y: 2.0 }], route, &[V::ROOT], |_, _| true);
        assert_eq!(r.over, None);
        assert!(r.take(doc, 1.0, m).is_empty());
    }

    /// A pen click carries the modifiers of the window it lands in.
    #[test]
    fn pen_clicks_carry_their_own_windows_modifiers() {
        let mut r = PenRouter::default();
        let route = PenRoute { window: 1, server_decorations: true };
        r.route(
            vec![PenEvent::Enter { surface: 1 }, PenEvent::Button { button: PenButton::Tip, pressed: true, x: 0.0, y: 0.0 }],
            route,
            &[egui::ViewportId::ROOT],
            |_, _| true,
        );
        let shift = egui::Modifiers::SHIFT;
        // egui-winit reports Shift's own key event with the modifiers from before it, then the
        // change: the change wins.
        let key = egui::Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        let modifiers = pass_modifiers(&[key.clone(), egui::Event::ModifiersChanged(shift)], egui::Modifiers::NONE);
        assert_eq!(modifiers, shift, "this pass's own modifier change wins over the last pass");
        assert_eq!(
            pass_modifiers(&[egui::Event::ModifiersChanged(shift), egui::Event::ModifiersChanged(egui::Modifiers::NONE)], shift),
            egui::Modifiers::NONE,
            "a release counts too"
        );
        assert_eq!(pass_modifiers(&[], egui::Modifiers::ALT), egui::Modifiers::ALT);
        let taken = r.take(egui::ViewportId::ROOT, 1.0, modifiers);
        assert!(matches!(taken.as_slice(), [egui::Event::PointerButton { modifiers, .. }] if *modifiers == shift));
    }

    #[test]
    fn only_the_viewport_under_the_pen_sets_its_cursor() {
        use egui::ViewportId as V;
        let doc = V::from_hash_of("docwin");
        assert_eq!(pass_cursor(Some(doc), doc), PassCursor::Output);
        assert_eq!(pass_cursor(Some(doc), V::ROOT), PassCursor::Keep, "the root's pass ends after the nested one");
        assert_eq!(pass_cursor(Some(V::ROOT), V::ROOT), PassCursor::Output);
        assert_eq!(pass_cursor(None, V::ROOT), PassCursor::Default);
        assert_eq!(pass_cursor(None, doc), PassCursor::Keep);
    }

    /// The pen shows the UI's cursor: its bitmap, or the matching shape.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_pen_cursor_follows_the_ui_cursor() {
        use photocraft_tablet::wayland::Cursor;
        let image = egui::CustomCursorImage { rgba: std::sync::Arc::from(vec![0u8; 16]), size: [2, 2], hotspot: [1, 1] };
        let Cursor::Image(i) = pen_cursor(egui::CursorIcon::Default, Some(&image), 2.0) else { panic!("a brush outline bitmap") };
        assert!(std::sync::Arc::ptr_eq(&i.rgba, &image.rgba) && i.width == 2 && i.hotspot == [1, 1] && i.scale == 2.0);
        assert_eq!(pen_cursor(egui::CursorIcon::None, None, 1.0), Cursor::Hidden);
        assert_eq!(pen_cursor(egui::CursorIcon::PointingHand, None, 1.0), Cursor::Pointer);
        assert_eq!(pen_cursor(egui::CursorIcon::ResizeColumn, None, 1.0), Cursor::ResizeHorizontal);
        assert_eq!(pen_cursor(egui::CursorIcon::ZoomIn, None, 1.0), Cursor::ZoomIn);
        assert_eq!(pen_cursor(egui::CursorIcon::ZoomOut, None, 1.0), Cursor::ZoomOut);
        assert_eq!(pen_cursor(egui::CursorIcon::Help, None, 1.0), Cursor::Help);
        for (icon, want) in [
            (egui::CursorIcon::ResizeNeSw, Cursor::ResizeNeSw),
            (egui::CursorIcon::ResizeNorthEast, Cursor::ResizeNeSw),
            (egui::CursorIcon::ResizeSouthWest, Cursor::ResizeNeSw),
            (egui::CursorIcon::ResizeNwSe, Cursor::ResizeNwSe),
            (egui::CursorIcon::ResizeNorthWest, Cursor::ResizeNwSe),
            (egui::CursorIcon::ResizeSouthEast, Cursor::ResizeNwSe),
            (egui::CursorIcon::Copy, Cursor::Copy),
            (egui::CursorIcon::Alias, Cursor::Alias),
            (egui::CursorIcon::Wait, Cursor::Wait),
            (egui::CursorIcon::Progress, Cursor::Progress),
            (egui::CursorIcon::ContextMenu, Cursor::ContextMenu),
        ] {
            assert_eq!(pen_cursor(icon, None, 1.0), want, "{icon:?}");
        }
        assert_eq!(pen_cursor(egui::CursorIcon::Default, None, 1.0), Cursor::Default);
    }

    #[test]
    fn use_tablet_pressure_off_paints_like_a_mouse() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.session.execute("prefs.set", json!({"path": "tools.useTabletPressure", "value": false})).unwrap();
        // The canvas copies the preference each frame; do what it does.
        app.stylus.use_pressure = app.session.prefs().tools.use_tablet_pressure;
        sink(app.stylus.feed.clone())(Some(Sample { pressure: 0.2, tilt_x: 40.0, ..Default::default() }));
        assert_eq!(app.stylus.pressure(), 1.0);
        assert_eq!(app.stylus.sample(), None);
    }

    #[test]
    fn eraser_end_switches_to_the_eraser_and_back() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let sink = sink(app.stylus.feed.clone());
        let mut st = appkit::State::default();
        let mut send = |e: appkit::RawEvent| {
            if let Update::Set(s) = st.handle(&e) {
                sink(s);
            }
        };
        let prox = |device, entering| appkit::RawEvent { kind: appkit::event_type::TABLET_PROXIMITY, device, entering, ..Default::default() };
        app.ui.tool = Tool::Brush;
        send(prox(appkit::device::PEN, true));
        assert!(!photocraft_ui_egui::stylus::Stylus::sync_eraser_tool(&mut app));
        assert_eq!(app.ui.tool, Tool::Brush, "the tip changes nothing");
        send(prox(appkit::device::PEN, false));
        send(prox(appkit::device::ERASER, true));
        assert!(photocraft_ui_egui::stylus::Stylus::sync_eraser_tool(&mut app));
        assert_eq!(app.ui.tool, Tool::Eraser);
        // Mouse moves in between change nothing; the tip coming back restores the Brush.
        send(appkit::RawEvent { kind: appkit::event_type::MOUSE_MOVED, ..Default::default() });
        assert!(!photocraft_ui_egui::stylus::Stylus::sync_eraser_tool(&mut app));
        send(prox(appkit::device::PEN, true));
        assert!(photocraft_ui_egui::stylus::Stylus::sync_eraser_tool(&mut app));
        assert_eq!(app.ui.tool, Tool::Brush);
    }

    /// Windows: what the pointer hooks record reaches the Eraser switch through the service the
    /// app installs (#2285).
    #[cfg(target_os = "windows")]
    #[test]
    fn windows_eraser_end_switches_to_the_eraser_and_back() {
        use photocraft_tablet::windows::{
            PEN_FLAG_ERASER, PEN_FLAG_INVERTED, WM_POINTERDOWN, WM_POINTERLEAVE, WM_POINTERUP, WM_POINTERUPDATE, handle_pointer_message,
        };
        use photocraft_ui_egui::stylus::Stylus;
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.services.pen_end = pen_end_service();
        app.ui.tool = Tool::Brush;
        let hover = |pen_flags: u32| handle_pointer_message(WM_POINTERUPDATE, 1, move |_| Some(pen_flags));
        hover(0);
        assert!(!Stylus::sync_eraser_tool(&mut app), "the tip changes nothing");
        hover(PEN_FLAG_ERASER);
        assert!(!Stylus::sync_eraser_tool(&mut app), "an eraser button on the barrel is not the eraser end");
        handle_pointer_message(WM_POINTERLEAVE, 1, |_| None);
        hover(PEN_FLAG_INVERTED);
        assert!(Stylus::sync_eraser_tool(&mut app));
        assert_eq!(app.ui.tool, Tool::Eraser);
        handle_pointer_message(WM_POINTERLEAVE, 1, |_| None);
        assert!(!Stylus::sync_eraser_tool(&mut app), "out of range");
        hover(0);
        assert!(Stylus::sync_eraser_tool(&mut app));
        assert_eq!(app.ui.tool, Tool::Brush, "the tip restores the Brush");
        // An eraser-end tap and the pen leaving, all before the next frame: the frame that
        // handles the queued click still selects the Eraser.
        handle_pointer_message(WM_POINTERDOWN, 1, |_| Some(PEN_FLAG_INVERTED | PEN_FLAG_ERASER));
        handle_pointer_message(WM_POINTERUP, 1, |_| None);
        handle_pointer_message(WM_POINTERLEAVE, 1, |_| None);
        assert!(Stylus::sync_eraser_tool(&mut app));
        assert_eq!(app.ui.tool, Tool::Eraser);
        assert!(!Stylus::sync_eraser_tool(&mut app), "out of range");
    }
}
