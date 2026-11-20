//! Native Wayland tablet input (`zwp_tablet_v2`) on winit's own `wl_display` connection.
//!
//! Tablet-tool events name the `wl_surface` the pen is over, and that surface belongs to winit's
//! connection, so the protocol has to be bound on that same connection: a second connection
//! would never see this window. This module wraps winit's display in a `wayland-client`
//! connection of its own (the one `unsafe` call below) and binds the tablet manager on a private
//! event queue. libwayland lets several queues share one connection, so winit's own queue is
//! untouched.
//!
//! The queue is dispatched on the UI thread ([`Reader::dispatch`]), the thread winit creates and
//! destroys its surfaces on: decoding an event that names one of winit's surfaces reads winit's
//! data for that surface, which must not race winit destroying it. That is the main thread on
//! Linux, and the reader refuses to start or dispatch on any other. A background thread only
//! reads the socket and wakes the UI when events wait in the queue.
//!
//! Binding the tablet manager makes the compositor stop emulating a pointer for the pen, so the
//! reader turns the tool's motion, tip and barrel buttons into [`PenEvent`]s for the UI to inject
//! (see [`crate::wl`]), each protocol frame together with its pressure/tilt/rotation/eraser
//! [`Sample`](crate::Sample). The pen's cursor is the tool's own (pointer cursors don't apply to
//! it): [`Reader::set_cursor`] sets a `cursor-shape-v1` shape when the compositor offers it, or
//! shows the UI's cursor bitmap (the brush outline) on a cursor surface of the tool's own, as
//! winit does for the mouse.

#![allow(unsafe_code)]

use std::collections::HashMap;
use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use wayland_backend::client::{Backend, ObjectId};
use wayland_client::backend::WaylandError;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_buffer::{self, WlBuffer};
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_shm::{self, WlShm};
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::protocol::{wl_registry, wl_seat::WlSeat};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, event_created_child};
use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::{Shape, WpCursorShapeDeviceV1};
use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_manager_v1::WpCursorShapeManagerV1;
use wayland_protocols::wp::tablet::zv2::client::zwp_tablet_manager_v2::ZwpTabletManagerV2;
use wayland_protocols::wp::tablet::zv2::client::zwp_tablet_pad_group_v2::{self, ZwpTabletPadGroupV2};
use wayland_protocols::wp::tablet::zv2::client::zwp_tablet_pad_ring_v2::ZwpTabletPadRingV2;
use wayland_protocols::wp::tablet::zv2::client::zwp_tablet_pad_strip_v2::ZwpTabletPadStripV2;
use wayland_protocols::wp::tablet::zv2::client::zwp_tablet_pad_v2::{self, ZwpTabletPadV2};
use wayland_protocols::wp::tablet::zv2::client::zwp_tablet_seat_v2::{self, ZwpTabletSeatV2};
use wayland_protocols::wp::tablet::zv2::client::zwp_tablet_tool_v2::{self, ZwpTabletToolV2};
use wayland_protocols::wp::tablet::zv2::client::zwp_tablet_v2::{self, ZwpTabletV2};
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;
use wayland_protocols::wp::viewporter::client::wp_viewporter::WpViewporter;

use crate::Error;
use crate::wl::{self, FrameOut, Raw, Tool};

/// The tablet manager's interface name, as the registry advertises it.
pub const TABLET_MANAGER: &str = "zwp_tablet_manager_v2";

/// The cursor the pen shows over the window.
#[derive(Clone, Debug, PartialEq)]
pub enum Cursor {
    Default,
    Crosshair,
    Text,
    Pointer,
    Grab,
    Grabbing,
    Move,
    NotAllowed,
    ResizeHorizontal,
    ResizeVertical,
    /// Corner to corner: north-east to south-west.
    ResizeNeSw,
    /// Corner to corner: north-west to south-east.
    ResizeNwSe,
    Copy,
    Alias,
    Wait,
    Progress,
    Help,
    ContextMenu,
    ZoomIn,
    ZoomOut,
    /// No cursor (the UI paints its own, e.g. a brush outline).
    Hidden,
    /// A bitmap (the brush outline); a crosshair when the compositor can't show it.
    Image(CursorImage),
}

/// A cursor bitmap: straight-alpha RGBA rows of `width`×`height` physical pixels, shown at that
/// size on a window of `scale` physical pixels per logical pixel.
#[derive(Clone, Debug)]
pub struct CursorImage {
    pub rgba: Arc<[u8]>,
    pub width: u16,
    pub height: u16,
    pub hotspot: [u16; 2],
    pub scale: f32,
}

/// The same bitmap: the UI hands the same `Arc` while the cursor doesn't change, so comparing
/// the pointer avoids comparing pixels every frame.
impl PartialEq for CursorImage {
    /// Equal when the same pixels (by pointer) show at the same size, hotspot and scale.
    fn eq(&self, o: &Self) -> bool {
        Arc::ptr_eq(&self.rgba, &o.rgba)
            && (self.width, self.height, self.hotspot) == (o.width, o.height, o.hotspot)
            && self.scale.to_bits() == o.scale.to_bits()
    }
}

/// Does the session's compositor offer the tablet protocol? Asked on a short-lived connection of
/// its own before any window exists, so the app can stay on native Wayland only when this reader
/// will work. `false` on any error, and when the compositor handed the app a single connection
/// (`WAYLAND_SOCKET`): probing would use up the socket winit needs.
pub fn compositor_has_tablet() -> bool {
    let Some(path) = socket_path(|k| std::env::var_os(k)) else { return false };
    let Ok(stream) = UnixStream::connect(path) else { return false };
    let Ok(conn) = Connection::from_socket(stream) else { return false };
    let Ok((globals, _queue)) = registry_queue_init::<Probe>(&conn) else { return false };
    globals.contents().with_list(|list| list.iter().any(|g| g.interface == TABLET_MANAGER))
}

/// The compositor's socket as libwayland finds it: `$WAYLAND_DISPLAY` (default `wayland-0`),
/// relative to `$XDG_RUNTIME_DIR` unless absolute. `None` with `$WAYLAND_SOCKET` set (an
/// inherited connection, not to be touched) or without a runtime dir for a relative name.
pub fn socket_path(var: impl Fn(&str) -> Option<std::ffi::OsString>) -> Option<PathBuf> {
    if var("WAYLAND_SOCKET").is_some_and(|v| !v.is_empty()) {
        return None;
    }
    let name = PathBuf::from(var("WAYLAND_DISPLAY").filter(|v| !v.is_empty()).unwrap_or_else(|| "wayland-0".into()));
    if name.is_absolute() {
        return Some(name);
    }
    Some(PathBuf::from(var("XDG_RUNTIME_DIR").filter(|v| !v.is_empty())?).join(name))
}

/// The process's main thread (on Linux its thread id is the process id).
fn on_main_thread() -> bool {
    rustix::thread::gettid() == rustix::process::getpid()
}

struct Probe;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Probe {
    /// The probe only lists the globals.
    fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

/// Tablet input for a window. Start it and call [`Self::dispatch`] on the main thread, where
/// winit runs its event loop on Linux, whenever the `wake` callback asked for it (each frame, say).
/// Both refuse to run on any other thread: decoding an event that names one of winit's objects
/// reads that object's data, which only winit's own thread may free.
pub struct Reader {
    queue: EventQueue<State>,
    state: State,
    conn: Connection,
    waker: Waker,
    error: Option<String>,
    error_taken: bool,
    /// The thread that started the reader: the only one that may dispatch it.
    thread: std::thread::ThreadId,
    /// The main window's `wl_surface*` address, as [`wl::PenEvent::Enter`] reports surfaces.
    pub window: u64,
    /// The compositor offers `xdg-decoration`, which winit asks for, so windows normally have
    /// server-side decorations and no decoration subsurfaces of winit's. A compositor may still
    /// refuse server-side decorations for a window (a user rule), which this can't see.
    pub server_decorations: bool,
    /// The window whose display handle this reads: kept until every reference to the connection
    /// above is gone (fields drop in order, this one last).
    _owner: Box<dyn std::any::Any + Send + Sync>,
}

impl Reader {
    /// Start reading tablet input for `window` (its owner: an `Arc` of the window, say), which
    /// the reader keeps until it is dropped. `wake` is called from a background thread when
    /// tablet events wait for [`Self::dispatch`]. Fails with [`Error::NotMainThread`] off the
    /// main thread, and with [`Error::Unsupported`] when the window isn't a Wayland one or the
    /// compositor has no tablet protocol.
    pub fn spawn<W>(window: W, wake: impl Fn() + Send + 'static) -> Result<Reader, Error>
    where
        W: HasDisplayHandle + HasWindowHandle + Send + Sync + 'static,
    {
        // The registry round trip below already decodes events, which may name winit's objects.
        if !on_main_thread() {
            return Err(Error::NotMainThread);
        }
        let (display, surface) = {
            let (Ok(d), Ok(w)) = (window.display_handle(), window.window_handle()) else {
                return Err(Error::Unsupported("no window handle".into()));
            };
            let (RawDisplayHandle::Wayland(d), RawWindowHandle::Wayland(w)) = (d.as_raw(), w.as_raw()) else {
                return Err(Error::Unsupported("not a Wayland window".into()));
            };
            (d.display, w.surface)
        };
        // SAFETY: `display` is the `wl_display*` of `window`'s display handle. raw-window-handle
        // guarantees the handle valid while `window` lives, and the reader keeps `window` until
        // after every reference to this backend is gone: the queue, state, connection and the
        // waker thread (joined on drop) are all fields dropped before `_owner`. The backend wraps
        // the display without taking ownership: a foreign display is never closed or freed by
        // `wayland-backend`, and libwayland allows other event queues and readers on a connection
        // while winit dispatches its own (`wl_display_prepare_read_queue`). Events that name
        // winit's objects are decoded only on the main thread, winit's (checked above and in
        // `dispatch`; the waker thread reads without dispatching), so winit can't destroy such an
        // object while it is being read.
        let backend = unsafe { Backend::from_foreign_display(display.as_ptr().cast()) };
        let conn = Connection::from_backend(backend);
        let (globals, queue) = registry_queue_init::<State>(&conn).map_err(|e| Error::Platform(format!("Wayland registry: {e}")))?;
        let qh = queue.handle();
        let manager: ZwpTabletManagerV2 = globals.bind(&qh, 1..=1, ()).map_err(|_| Error::Unsupported(format!("the compositor has no {TABLET_MANAGER}")))?;
        let shapes: Option<WpCursorShapeManagerV1> = globals.bind(&qh, 1..=1, ()).ok();
        let cursor_globals = match (globals.bind::<WlCompositor, _, _>(&qh, 1..=4, ()), globals.bind::<WlShm, _, _>(&qh, 1..=1, ())) {
            (Ok(compositor), Ok(shm)) => Some(CursorGlobals { compositor, shm, viewporter: globals.bind::<WpViewporter, _, _>(&qh, 1..=1, ()).ok() }),
            _ => None,
        };
        let server_decorations = globals.contents().with_list(|list| list.iter().any(|g| g.interface == "zxdg_decoration_manager_v1"));
        let seats: Vec<WlSeat> = globals
            .contents()
            .with_list(|list| list.iter().filter(|g| g.interface == "wl_seat").map(|g| (g.name, g.version.min(5))).collect::<Vec<_>>())
            .into_iter()
            .map(|(name, version)| globals.registry().bind(name, version, &qh, ()))
            .collect();
        if seats.is_empty() {
            return Err(Error::Unsupported("no Wayland seat".into()));
        }
        let tablet_seats: Vec<ZwpTabletSeatV2> = seats.iter().map(|s| manager.get_tablet_seat(s, &qh, ())).collect();
        let state = State {
            tools: HashMap::new(),
            active: None,
            wanted: Cursor::Default,
            shapes,
            cursor_globals,
            pads: HashMap::new(),
            qh,
            out: Vec::new(),
            _keep: (manager, seats, tablet_seats),
        };
        let _ = conn.flush();
        let waker = Waker::spawn(conn.clone(), wake)?;
        Ok(Reader {
            queue,
            state,
            conn,
            waker,
            error: None,
            error_taken: false,
            thread: std::thread::current().id(),
            window: surface.as_ptr() as u64,
            server_decorations,
            _owner: Box::new(window),
        })
    }

    /// Dispatch the tablet events that arrived: each protocol frame's pointer events and pen
    /// sample, in order. Call on the thread that started the reader; on any other this fails the
    /// reader. Once the reader failed, every pen is released and nothing more arrives
    /// ([`Self::take_error`] says why).
    pub fn dispatch(&mut self) -> Vec<FrameOut> {
        if std::thread::current().id() != self.thread {
            self.fail("tablet events dispatched off the main thread".into());
            return std::mem::take(&mut self.state.out);
        }
        if self.error.is_none()
            && let Err(e) = self.queue.dispatch_pending(&mut self.state)
        {
            self.fail(format!("tablet events: {e}"));
        }
        let _ = self.conn.flush();
        if let Some(e) = self.waker.dispatched() {
            self.fail(e);
        }
        std::mem::take(&mut self.state.out)
    }

    /// Why the reader stopped, once.
    pub fn take_error(&mut self) -> Option<String> {
        if self.error_taken {
            return None;
        }
        self.error_taken = self.error.is_some();
        self.error.clone()
    }

    /// Keep the first connection error and release every tool, so the UI keeps no pressed button.
    /// The waking thread stops too: nothing dispatches this queue any more, so it would wake the
    /// UI for its events every [`REWAKE`] for good.
    fn fail(&mut self, error: String) {
        if self.error.is_none() {
            self.error = Some(error);
            self.state.release_all();
            self.waker.stop();
        }
    }

    /// Show `cursor` while the pen is over one of the app's surfaces (applied now if it is).
    pub fn set_cursor(&mut self, cursor: Cursor) {
        if self.state.wanted != cursor {
            self.state.wanted = cursor;
            self.state.apply_cursor();
            let _ = self.conn.flush();
        }
    }
}

impl Drop for Reader {
    /// Stop the reading thread first: it reads winit's display.
    fn drop(&mut self) {
        // The thread reads winit's display: stop it before the fields (and `_owner`) go.
        self.waker.stop();
    }
}

/// The background thread that reads the socket and wakes the UI for queued tablet events.
struct Waker {
    shared: Arc<WakerShared>,
    /// Closing it wakes the thread's poll to stop.
    stop: Option<UnixStream>,
    thread: Option<JoinHandle<()>>,
}

#[derive(Default)]
struct WakerShared {
    state: Mutex<WakeState>,
    cv: Condvar,
}

#[derive(Default)]
struct WakeState {
    stop: bool,
    /// Bumped by each dispatch, so the thread knows the queue was emptied.
    dispatched: u64,
    /// Why the thread ended, for the UI to report.
    error: Option<String>,
}

/// How long the thread waits for a dispatch before waking the UI again (a minimised window may
/// not repaint).
const REWAKE: Duration = Duration::from_millis(250);

impl Waker {
    /// Start the thread that reads `conn`'s socket and calls `wake` when events wait.
    fn spawn(conn: Connection, wake: impl Fn() + Send + 'static) -> Result<Self, Error> {
        let fail = |e: std::io::Error| Error::Platform(format!("cannot start the tablet thread: {e}"));
        let (stop, stopped) = UnixStream::pair().map_err(fail)?;
        let shared = Arc::new(WakerShared::default());
        let s = shared.clone();
        let thread = std::thread::Builder::new()
            .name("photocraft-tablet-wayland".into())
            .spawn(move || {
                let error = read_until_stopped(&conn, &stopped, &s, &wake);
                if let Some(e) = error {
                    s.lock().error = Some(e);
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(&wake));
                }
            })
            .map_err(fail)?;
        Ok(Self { shared, stop: Some(stop), thread: Some(thread) })
    }

    /// The UI dispatched; the thread's error, if it ended with one.
    fn dispatched(&self) -> Option<String> {
        let mut s = self.shared.lock();
        s.dispatched = s.dispatched.wrapping_add(1);
        self.shared.cv.notify_all();
        s.error.take()
    }

    /// Stop the reading thread and wait for it.
    fn stop(&mut self) {
        self.shared.lock().stop = true;
        self.shared.cv.notify_all();
        drop(self.stop.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl WakerShared {
    /// The wake state (after a panic elsewhere it is still whole: each field is set in one step).
    fn lock(&self) -> std::sync::MutexGuard<'_, WakeState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Read the socket until stopped: when events wait in this reader's queue, wake the UI and wait
/// for its dispatch. `Some(error)` when the connection failed.
fn read_until_stopped(conn: &Connection, stopped: &UnixStream, shared: &WakerShared, wake: &dyn Fn()) -> Option<String> {
    use rustix::event::{PollFd, PollFlags, poll};
    loop {
        if shared.lock().stop {
            return None;
        }
        // `None`: events already wait in this reader's queue.
        let Some(guard) = conn.prepare_read() else {
            let seen = shared.lock().dispatched;
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(wake));
            let s = shared.lock();
            let _ = shared.cv.wait_timeout_while(s, REWAKE, |s| !s.stop && s.dispatched == seen);
            continue;
        };
        let fd = guard.connection_fd();
        let mut fds = [PollFd::new(&fd, PollFlags::IN | PollFlags::ERR), PollFd::new(stopped, PollFlags::IN)];
        loop {
            match poll(&mut fds, None) {
                Ok(_) => break,
                Err(rustix::io::Errno::INTR) => continue,
                Err(e) => return Some(format!("tablet events: {e}")),
            }
        }
        if !fds[1].revents().is_empty() {
            // Dropping the guard cancels the read, leaving the socket to winit.
            return None;
        }
        // Read only: `read()` would also decode the events here, off the UI thread, and move them
        // out of libwayland's queue, so the next `prepare_read` would not see them and not wake.
        match guard.read_without_dispatch() {
            Ok(()) => {}
            Err(WaylandError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Some(format!("tablet events: {e}")),
        }
    }
}

struct CursorGlobals {
    compositor: WlCompositor,
    shm: WlShm,
    viewporter: Option<WpViewporter>,
}

struct State {
    tools: HashMap<ObjectId, ToolState>,
    /// The tool last in proximity: the one the cursor is shown for.
    active: Option<ObjectId>,
    wanted: Cursor,
    shapes: Option<WpCursorShapeManagerV1>,
    cursor_globals: Option<CursorGlobals>,
    pads: HashMap<ObjectId, Pad>,
    qh: QueueHandle<State>,
    /// The frames of the [`Reader::dispatch`] in progress.
    out: Vec<FrameOut>,
    /// Bound objects kept alive for the connection's lifetime.
    _keep: (ZwpTabletManagerV2, Vec<WlSeat>, Vec<ZwpTabletSeatV2>),
}

impl State {
    /// Queue a frame for the [`Reader::dispatch`] that is decoding it (it returns them all, so this
    /// holds one dispatch's frames: while the UI stalls, events wait undecoded in libwayland's queue).
    fn push_frame(&mut self, f: FrameOut) {
        self.out.push(f);
    }

    /// The reader stops: every tool leaves, so the UI never keeps a pressed tip or a pen sample.
    fn release_all(&mut self) {
        let mut frames = Vec::new();
        for t in self.tools.values_mut() {
            t.serial = None;
            frames.extend([Raw::ProximityOut, Raw::Frame].into_iter().filter_map(|r| t.map.handle(r)));
        }
        for f in frames {
            self.push_frame(f);
        }
        self.active = None;
    }

    /// Show the wanted cursor for the tool in proximity, if any.
    fn apply_cursor(&mut self) {
        let Some(t) = self.active.as_ref().and_then(|id| self.tools.get_mut(id)) else { return };
        t.apply(&self.wanted, self.shapes.as_ref(), self.cursor_globals.as_ref(), &self.qh);
    }
}

/// One tablet tool: the mapping, the proximity serial its cursor is set with, and its own cursor
/// objects (a cursor surface belongs to one tool for good, per the protocol).
struct ToolState {
    proxy: ZwpTabletToolV2,
    map: Tool,
    serial: Option<u32>,
    /// The tool left proximity; the frame that carries its `Leave` has not arrived yet.
    leaving: bool,
    shape: Option<WpCursorShapeDeviceV1>,
    images: Option<Images>,
}

/// The fallback crosshair's pixels, made once so the cursor compares equal across frames.
static CROSSHAIR: std::sync::OnceLock<Arc<[u8]>> = std::sync::OnceLock::new();

impl ToolState {
    /// A tool just announced: no proximity serial or cursor objects yet.
    fn new(proxy: ZwpTabletToolV2) -> Self {
        Self { proxy, map: Tool::default(), serial: None, leaving: false, shape: None, images: None }
    }

    /// Show `wanted` for this tool, with its proximity serial: a shape through cursor-shape-v1, a
    /// bitmap on its own cursor surface (a crosshair when it can't), or none. Nothing out of proximity.
    fn apply(&mut self, wanted: &Cursor, shapes: Option<&WpCursorShapeManagerV1>, globals: Option<&CursorGlobals>, qh: &QueueHandle<State>) {
        let Some(serial) = self.serial else { return };
        if self.shape.is_none() {
            self.shape = shapes.map(|m| m.get_tablet_tool_v2(&self.proxy, qh, ()));
        }
        let (proxy, shape, images) = (&self.proxy, &self.shape, &mut self.images);
        let mut show = |image: &CursorImage| {
            let Some(g) = globals else { return false };
            let images = images.get_or_insert_with(|| Images::new(g, qh));
            match images.show(image) {
                Some([x, y]) => {
                    proxy.set_cursor(serial, Some(&images.surface), x, y);
                    true
                }
                None => false,
            }
        };
        match (wanted, shape) {
            (Cursor::Hidden, _) => proxy.set_cursor(serial, None, 0, 0),
            (Cursor::Image(image), _) => {
                if !show(image)
                    && let Some(d) = shape
                {
                    d.set_shape(serial, Shape::Crosshair);
                }
            }
            (c, Some(d)) => {
                if let Some(s) = cursor_shape(c) {
                    d.set_shape(serial, s);
                }
            }
            // Without cursor-shape the tool keeps whatever was set last, a hidden or bitmap
            // cursor included: show the crosshair bitmap rather than nothing.
            (_, None) => {
                let (rgba, width, height, hotspot) = wl::crosshair();
                show(&CursorImage { rgba: CROSSHAIR.get_or_init(|| rgba.into()).clone(), width, height, hotspot, scale: 1.0 });
            }
        }
    }

    /// The tool is gone: destroy it and its cursor objects.
    fn destroy(self) {
        if let Some(d) = self.shape {
            d.destroy();
        }
        if let Some(i) = self.images {
            i.destroy();
        }
        self.proxy.destroy();
    }
}

/// The cursor-shape-v1 shape for `c` (`None` for a bitmap or no cursor).
fn cursor_shape(c: &Cursor) -> Option<Shape> {
    Some(match c {
        Cursor::Default => Shape::Default,
        Cursor::Crosshair => Shape::Crosshair,
        Cursor::Text => Shape::Text,
        Cursor::Pointer => Shape::Pointer,
        Cursor::Grab => Shape::Grab,
        Cursor::Grabbing => Shape::Grabbing,
        Cursor::Move => Shape::Move,
        Cursor::NotAllowed => Shape::NotAllowed,
        Cursor::ResizeHorizontal => Shape::EwResize,
        Cursor::ResizeVertical => Shape::NsResize,
        Cursor::ResizeNeSw => Shape::NeswResize,
        Cursor::ResizeNwSe => Shape::NwseResize,
        Cursor::Copy => Shape::Copy,
        Cursor::Alias => Shape::Alias,
        Cursor::Wait => Shape::Wait,
        Cursor::Progress => Shape::Progress,
        Cursor::Help => Shape::Help,
        Cursor::ContextMenu => Shape::ContextMenu,
        Cursor::ZoomIn => Shape::ZoomIn,
        Cursor::ZoomOut => Shape::ZoomOut,
        Cursor::Hidden | Cursor::Image(_) => return None,
    })
}

/// A tool's bitmap cursor surface and the buffers shown on it.
struct Images {
    surface: WlSurface,
    surface_version: u32,
    shm: WlShm,
    viewport: Option<WpViewport>,
    qh: QueueHandle<State>,
    shown: Option<(CursorImage, WlBuffer)>,
    /// The compositor already released the shown buffer (it copied it), so it can be destroyed
    /// as soon as it is replaced.
    shown_released: bool,
    /// Buffers replaced on the surface, destroyed once the compositor releases them.
    retired: Vec<WlBuffer>,
}

/// Retired buffers kept waiting for a release; past it the oldest is destroyed anyway.
const MAX_RETIRED: usize = 4;

impl Images {
    fn new(g: &CursorGlobals, qh: &QueueHandle<State>) -> Self {
        let surface = g.compositor.create_surface(qh, ());
        let viewport = g.viewporter.as_ref().map(|v| v.get_viewport(&surface, qh, ()));
        Self {
            surface_version: surface.version(),
            surface,
            shm: g.shm.clone(),
            viewport,
            qh: qh.clone(),
            shown: None,
            shown_released: false,
            retired: Vec::new(),
        }
    }

    /// Put `image` on the cursor surface (unless it is there already); its hotspot in surface
    /// coordinates, or `None` when the bitmap is unusable or shared memory fails.
    fn show(&mut self, image: &CursorImage) -> Option<[i32; 2]> {
        let (size, hotspot) = wl::cursor_size(image.width, image.height, image.hotspot, image.scale);
        if self.shown.as_ref().is_some_and(|(shown, _)| shown == image) {
            return Some(self.hotspot(image, hotspot));
        }
        let pixels = wl::argb8888(&image.rgba, image.width, image.height)?;
        let len = i32::try_from(pixels.len()).ok()?;
        let fd = rustix::fs::memfd_create("photocraft-pen-cursor", rustix::fs::MemfdFlags::CLOEXEC).ok()?;
        let mut file = std::fs::File::from(fd);
        file.write_all(&pixels).ok()?;
        let pool: WlShmPool = self.shm.create_pool(file.as_fd(), len, &self.qh, ());
        let (w, h) = (i32::from(image.width), i32::from(image.height));
        let buffer = pool.create_buffer(0, w, h, w * 4, wl_shm::Format::Argb8888, &self.qh, ());
        // The buffer keeps the memory mapped on the compositor's side; the pool and file can go.
        pool.destroy();
        self.surface.attach(Some(&buffer), 0, 0);
        match &self.viewport {
            Some(v) => v.set_destination(size[0], size[1]),
            None if self.surface_version >= 3 => self.surface.set_buffer_scale(i32::from(wl::buffer_scale(image.width, image.height, image.scale))),
            None => {}
        }
        if self.surface_version >= 4 {
            self.surface.damage_buffer(0, 0, w, h);
        } else {
            self.surface.damage(0, 0, size[0], size[1]);
        }
        self.surface.commit();
        let released = std::mem::take(&mut self.shown_released);
        if let Some((_, old)) = self.shown.replace((image.clone(), buffer)) {
            if released {
                old.destroy();
            } else {
                self.retired.push(old);
                if self.retired.len() > MAX_RETIRED {
                    self.retired.remove(0).destroy();
                }
            }
        }
        Some(self.hotspot(image, hotspot))
    }

    /// The hotspot in surface coordinates: scaled by viewporter, else by the buffer scale.
    fn hotspot(&self, image: &CursorImage, scaled: [i32; 2]) -> [i32; 2] {
        if self.viewport.is_some() {
            return scaled;
        }
        let k = if self.surface_version >= 3 { wl::buffer_scale(image.width, image.height, image.scale) } else { 1 };
        image.hotspot.map(|h| i32::from(h / k))
    }

    /// The compositor released `buffer`: destroy it if it was replaced, else remember that the
    /// shown one may go as soon as it is replaced.
    fn released(&mut self, buffer: &WlBuffer) {
        if self.shown.as_ref().is_some_and(|(_, b)| b == buffer) {
            self.shown_released = true;
        } else if let Some(i) = self.retired.iter().position(|b| b == buffer) {
            self.retired.remove(i).destroy();
        }
    }

    /// Destroy the cursor surface and its buffers.
    fn destroy(self) {
        for b in self.retired.into_iter().chain(self.shown.map(|(_, b)| b)) {
            b.destroy();
        }
        if let Some(v) = self.viewport {
            v.destroy();
        }
        self.surface.destroy();
    }
}

/// A pad (express keys) and its groups' rings and strips, destroyed with it.
#[derive(Default)]
struct Pad {
    proxy: Option<ZwpTabletPadV2>,
    groups: Vec<ZwpTabletPadGroupV2>,
    rings: Vec<ZwpTabletPadRingV2>,
    strips: Vec<ZwpTabletPadStripV2>,
}

impl Dispatch<ZwpTabletToolV2, ()> for State {
    /// Map a tool's events to [`Raw`] events and queue each frame for [`Reader::dispatch`]; another
    /// tool's frames go in only as their releases.
    fn event(state: &mut Self, tool: &ZwpTabletToolV2, event: zwp_tablet_tool_v2::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        use zwp_tablet_tool_v2::Event as E;
        let id = tool.id();
        // The UI follows the tool in proximity last; a frame from another goes in only as its
        // releases ([`FrameOut::background`]). The active tool's own leaving goes in whole.
        let was_active = state.active.as_ref() == Some(&id);
        let t = state.tools.entry(id.clone()).or_insert_with(|| ToolState::new(tool.clone()));
        let raw = match event {
            E::Type { tool_type } => {
                t.map.eraser = tool_type == WEnum::Value(zwp_tablet_tool_v2::Type::Eraser);
                return;
            }
            E::Capability { capability } => {
                t.map.pressure_axis |= capability == WEnum::Value(zwp_tablet_tool_v2::Capability::Pressure);
                return;
            }
            E::Removed => {
                if let Some(mut t) = state.tools.remove(&id) {
                    // A tool removed in proximity leaves first.
                    let left = [Raw::ProximityOut, Raw::Frame].into_iter().filter_map(|r| t.map.handle(r)).collect::<Vec<_>>();
                    t.destroy();
                    for f in left.into_iter().filter_map(|f| if was_active { Some(f) } else { f.background() }) {
                        state.push_frame(f);
                    }
                }
                if state.active.as_ref() == Some(&id) {
                    state.active = None;
                }
                return;
            }
            E::ProximityIn { serial, surface, .. } => {
                t.serial = Some(serial);
                t.leaving = false;
                state.active = Some(id.clone());
                Raw::ProximityIn { surface: surface.id().as_ptr() as u64 }
            }
            E::ProximityOut => {
                t.serial = None;
                // Keep the tool active until its departure frame is delivered: the `Leave` and
                // the sample clear belong to the active tool, not to a background one.
                t.leaving = true;
                Raw::ProximityOut
            }
            E::Down { .. } => Raw::Down,
            E::Up => Raw::Up,
            E::Motion { x, y } => Raw::Motion { x, y },
            E::Pressure { pressure } => Raw::Pressure(pressure),
            E::Tilt { tilt_x, tilt_y } => Raw::Tilt { x: tilt_x, y: tilt_y },
            E::Rotation { degrees } => Raw::Rotation(degrees),
            E::Button { button, state: s, .. } => Raw::Button { code: button, pressed: s == WEnum::Value(zwp_tablet_tool_v2::ButtonState::Pressed) },
            E::Frame { .. } => Raw::Frame,
            _ => return,
        };
        let entered = matches!(raw, Raw::ProximityIn { .. });
        let leaving_frame = matches!(raw, Raw::Frame) && state.tools.get(&id).is_some_and(|t| t.leaving);
        let out = state.tools.get_mut(&id).and_then(|t| {
            let out = t.map.handle(raw);
            if leaving_frame {
                t.leaving = false;
            }
            out
        });
        if entered {
            state.apply_cursor();
        }
        let active = was_active || state.active.as_ref() == Some(&id);
        if let Some(out) = out.and_then(|f| if active { Some(f) } else { f.background() }) {
            state.push_frame(out);
        }
        if leaving_frame && state.active.as_ref() == Some(&id) {
            state.active = None;
        }
    }
}

impl Dispatch<ZwpTabletV2, ()> for State {
    /// A removed tablet is destroyed.
    fn event(_: &mut Self, tablet: &ZwpTabletV2, event: zwp_tablet_v2::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let zwp_tablet_v2::Event::Removed = event {
            tablet.destroy();
        }
    }
}

impl Dispatch<ZwpTabletSeatV2, ()> for State {
    /// The seat only announces tablets, tools and pads (created as children).
    fn event(_: &mut Self, _: &ZwpTabletSeatV2, _: zwp_tablet_seat_v2::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}

    event_created_child!(State, ZwpTabletSeatV2, [
        zwp_tablet_seat_v2::EVT_TABLET_ADDED_OPCODE => (ZwpTabletV2, ()),
        zwp_tablet_seat_v2::EVT_TOOL_ADDED_OPCODE => (ZwpTabletToolV2, ()),
        zwp_tablet_seat_v2::EVT_PAD_ADDED_OPCODE => (ZwpTabletPadV2, ()),
    ]);
}

impl Dispatch<ZwpTabletPadV2, ()> for State {
    /// Track a pad's groups, and destroy the pad with its rings and strips when it is removed.
    fn event(state: &mut Self, pad: &ZwpTabletPadV2, event: zwp_tablet_pad_v2::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        let p = state.pads.entry(pad.id()).or_default();
        p.proxy.get_or_insert_with(|| pad.clone());
        match event {
            zwp_tablet_pad_v2::Event::Group { pad_group } => p.groups.push(pad_group),
            zwp_tablet_pad_v2::Event::Removed => {
                if let Some(p) = state.pads.remove(&pad.id()) {
                    p.rings.into_iter().for_each(|r| r.destroy());
                    p.strips.into_iter().for_each(|s| s.destroy());
                    p.groups.into_iter().for_each(|g| g.destroy());
                }
                pad.destroy();
            }
            _ => {}
        }
    }

    event_created_child!(State, ZwpTabletPadV2, [
        zwp_tablet_pad_v2::EVT_GROUP_OPCODE => (ZwpTabletPadGroupV2, ()),
    ]);
}

impl Dispatch<ZwpTabletPadGroupV2, ()> for State {
    /// Keep a pad group's rings and strips, to destroy them with the pad.
    fn event(state: &mut Self, group: &ZwpTabletPadGroupV2, event: zwp_tablet_pad_group_v2::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        let Some(p) = state.pads.values_mut().find(|p| p.groups.contains(group)) else { return };
        match event {
            zwp_tablet_pad_group_v2::Event::Ring { ring } => p.rings.push(ring),
            zwp_tablet_pad_group_v2::Event::Strip { strip } => p.strips.push(strip),
            _ => {}
        }
    }

    event_created_child!(State, ZwpTabletPadGroupV2, [
        zwp_tablet_pad_group_v2::EVT_RING_OPCODE => (ZwpTabletPadRingV2, ()),
        zwp_tablet_pad_group_v2::EVT_STRIP_OPCODE => (ZwpTabletPadStripV2, ()),
    ]);
}

/// Objects whose events this reader ignores (the express-key pad's rings and strips, cursor
/// objects).
macro_rules! ignore_events {
    ($($t:ty),*) => {$(
        impl Dispatch<$t, ()> for State {
            /// Events this reader doesn't use.
            fn event(_: &mut Self, _: &$t, _: <$t as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
        }
    )*};
}

ignore_events!(
    ZwpTabletManagerV2,
    ZwpTabletPadRingV2,
    ZwpTabletPadStripV2,
    WlSeat,
    WpCursorShapeManagerV1,
    WpCursorShapeDeviceV1,
    WlCompositor,
    WlSurface,
    WlShm,
    WlShmPool,
    WpViewporter,
    WpViewport
);

impl Dispatch<WlBuffer, ()> for State {
    /// A released buffer may be destroyed once it is replaced.
    fn event(state: &mut Self, buffer: &WlBuffer, event: wl_buffer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wl_buffer::Event::Release = event {
            for images in state.tools.values_mut().filter_map(|t| t.images.as_mut()) {
                images.released(buffer);
            }
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reader refuses to start off the main thread, before it touches the window.
    #[test]
    fn the_reader_only_runs_on_the_main_thread() {
        assert_eq!(std::thread::spawn(on_main_thread).join().ok(), Some(false));
        let other = std::thread::spawn(|| Reader::spawn(NoWindow, || {}).err()).join().ok().flatten();
        assert_eq!(other, Some(Error::NotMainThread), "refused before touching the window");
    }

    /// A failed reader stops its waking thread (`fail`), and `Drop` stops it again: the thread
    /// ends, wakes the UI no more, and the second stop does nothing.
    #[test]
    fn the_waker_thread_stops_for_good_and_a_second_stop_is_harmless() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let (client, _server) = UnixStream::pair().unwrap();
        let conn = Connection::from_socket(client).unwrap();
        let wakes = Arc::new(AtomicUsize::new(0));
        let w = wakes.clone();
        let mut waker = Waker::spawn(conn, move || {
            w.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();
        waker.stop();
        assert!(waker.thread.is_none() && waker.stop.is_none(), "joined, its stop socket closed");
        let after = wakes.load(Ordering::SeqCst);
        waker.stop();
        std::thread::sleep(REWAKE * 2);
        assert_eq!(wakes.load(Ordering::SeqCst), after, "no wakes once stopped");
    }

    /// Every cursor but a bitmap or none has its cursor-shape-v1 shape, the corners included.
    #[test]
    fn cursors_map_to_their_protocol_shapes() {
        assert_eq!(cursor_shape(&Cursor::ResizeNeSw), Some(Shape::NeswResize));
        assert_eq!(cursor_shape(&Cursor::ResizeNwSe), Some(Shape::NwseResize));
        assert_eq!(cursor_shape(&Cursor::Copy), Some(Shape::Copy));
        assert_eq!(cursor_shape(&Cursor::Wait), Some(Shape::Wait));
        assert_eq!(cursor_shape(&Cursor::Hidden), None);
    }

    /// A window whose handles would fail: the main-thread check must come first.
    struct NoWindow;

    impl HasDisplayHandle for NoWindow {
        /// No display handle.
        fn display_handle(&self) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
            Err(raw_window_handle::HandleError::Unavailable)
        }
    }

    impl HasWindowHandle for NoWindow {
        fn window_handle(&self) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
            Err(raw_window_handle::HandleError::Unavailable)
        }
    }

    #[test]
    fn the_probe_finds_the_socket_like_libwayland_and_leaves_an_inherited_one_alone() {
        let env = |pairs: &'static [(&str, &str)]| move |k: &str| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| std::ffi::OsString::from(v));
        assert_eq!(socket_path(env(&[("XDG_RUNTIME_DIR", "/run/user/1000")])), Some(PathBuf::from("/run/user/1000/wayland-0")));
        assert_eq!(
            socket_path(env(&[("XDG_RUNTIME_DIR", "/run/user/1000"), ("WAYLAND_DISPLAY", "wayland-1")])),
            Some(PathBuf::from("/run/user/1000/wayland-1"))
        );
        assert_eq!(socket_path(env(&[("WAYLAND_DISPLAY", "/tmp/w")])), Some(PathBuf::from("/tmp/w")), "absolute needs no runtime dir");
        assert_eq!(socket_path(env(&[("WAYLAND_DISPLAY", "wayland-1")])), None, "relative without a runtime dir");
        assert_eq!(socket_path(env(&[("XDG_RUNTIME_DIR", "/run/user/1000"), ("WAYLAND_SOCKET", "5")])), None, "never take winit's inherited socket");
    }
}
