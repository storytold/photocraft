//! `WM_POINTER` pen monitor for Windows: a window subclass that takes the pen over from winit.
//!
//! winit 0.30 reports a pen as `WindowEvent::Touch` and lets egui emulate the mouse from it: the
//! frame's buttons and its cancel flag are dropped, so a barrel/right click painted as a left tap
//! and a press-and-hold left the emulated button stuck down forever. The subclass swallows the
//! pen's `WM_POINTER*` frames before winit's wndproc sees them and synthesizes the standard mouse
//! messages instead (hover and tip-drag moves, the tip as the left button, the barrel as a real
//! right button); winit and egui handle those natively. Finger touches pass through unchanged.
//!
//! Windows Ink's right clicks are *system gestures*: press-and-hold cancels the tip contact
//! (`POINTER_FLAG_CANCELED`, reported as [`Signal::Cancel`]) and posts `WM_CONTEXTMENU`; a right
//! tap posts it after the tap. A pen `WM_CONTEXTMENU` becomes a right-button click at the pointer
//! — unless the frames already synthesized that click from the barrel, when it is their echo.
//!
//! Swallowing the pen's frames also swallows the system's promotion of them to legacy mouse
//! messages, which is what moves the real cursor: without help the pen would never move it and
//! the mouse would resume from its stale spot. So every pen frame parks the cursor on the pen
//! (`SetCursorPos`), making pen and mouse one pointer again. The parking echoes back as a
//! `WM_MOUSEMOVE` on the *input* queue, which Win32 dispatches after the posted messages — it
//! reports a position several posted moves old, and letting it through makes the stroke walk
//! backward for a sample (the jitter while drawing straight lines). Each parked position is
//! therefore remembered and its echo swallowed.
//!
//! Like `macos.rs` this module allows `unsafe` (the workspace's one isolated helper crate); every
//! block has a `SAFETY:` comment and everything it reads goes through the pure, tested mapping in
//! [`crate::pointer`].

#![allow(unsafe_code)]

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::fs;
use std::time::{Duration, Instant, SystemTime};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{ClientToScreen, ScreenToClient};
use windows::Win32::UI::Input::Pointer::{
    GetPointerPenInfo, GetPointerType, POINTER_FLAG_CANCELED, POINTER_FLAG_INCONTACT, POINTER_FLAG_SECONDBUTTON, POINTER_PEN_INFO,
};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    PEN_FLAG_BARREL, PEN_FLAG_ERASER, PEN_FLAG_INVERTED, POINTER_INPUT_TYPE, PT_PEN, PostMessageW, SetCursorPos, WM_CONTEXTMENU, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WM_POINTERDOWN, WM_POINTERUP, WM_POINTERUPDATE, WM_RBUTTONDOWN, WM_RBUTTONUP,
};

use crate::pointer::{Button, Mouse, RawFrame, State, advance, gesture};
use crate::wacom::{self, PenFeel};
use crate::{Error, Signal, deliver_signal};

/// The subclass id (`'pcpe'`); identifies this subclass for removal.
const SUBCLASS_ID: usize = 0x7063_7065;

/// How long after a pen contact a `WM_CONTEXTMENU` still counts as its gesture. A hold takes about
/// a second; a mouse right click long after any pen use must not become the pen's.
const GESTURE_WINDOW: Duration = Duration::from_secs(2);

/// `MK_LBUTTON` / `MK_RBUTTON`: the down-button flag of the synthesized mouse messages.
const MK_LBUTTON: usize = 0x0001;
const MK_RBUTTON: usize = 0x0002;

/// How far (physical px) a held-right pen may drift from where its contact began while still
/// counting as one right click. A fast pen tap always jitters a few pixels; the UI (egui) reads a
/// press-release that moved more than about 6 points as a drag, not a click, and a drag never
/// opens the right-click UI. Frames within the slop are pinned to the contact's start point; once
/// the pen clearly moves away it is a drag and follows freely.
const CLICK_SLOP_PX: i32 = 12;

/// How many `SetCursorPos` echo positions may be awaited at once. The echo of one parking arrives
/// within a frame or two; this bounds the list if echoes never come (a clipped or blocked cursor),
/// so one stale entry can never silently eat mouse moves forever.
const ECHO_CAP: usize = 64;

/// What the subclass proc reads: the user callback, the synthesized mouse's state and when the pen
/// was last seen (to attribute a `WM_CONTEXTMENU`).
struct Data {
    callback: Box<dyn Fn(Signal)>,
    /// The pure state machine: which button the synthesized mouse holds.
    state: Cell<State>,
    /// Where the pen was last seen, in screen coordinates (an unreadable frame releases there).
    last: Cell<POINT>,
    /// The screen position the cursor was last parked at: skips redundant `SetCursorPos` calls.
    cursor: Cell<POINT>,
    /// Screen positions recently given to `SetCursorPos`, whose echo `WM_MOUSEMOVE`s are still
    /// awaited and must be swallowed (they report old positions after newer posted ones).
    cursor_echoes: RefCell<VecDeque<POINT>>,
    /// When the pen was last in contact: the gesture window of a `WM_CONTEXTMENU`.
    contacted: Cell<Option<Instant>>,
    /// Where the current right-button contact began (the click-slop origin) and whether the pen
    /// has stayed inside the slop so far.
    right_press: Cell<POINT>,
    right_pinned: Cell<bool>,
    /// The Wacom driver's Tip Feel (see [`crate::wacom`]): the Windows Ink path delivers the
    /// pen's raw pressure — the driver keeps its curve for Wintab — so it is played onto each
    /// frame here. Reloaded when the driver's settings file changes.
    feel: Cell<PenFeel>,
    /// The mtime of the settings file the feel was read from (a change reloads it).
    feel_stamp: Cell<Option<SystemTime>>,
}

/// Keeps the subclass installed; dropping it removes the subclass and frees the callback (keep it
/// until the window is gone). The proc runs on the window's thread, so install on that thread.
pub struct Monitor {
    hwnd: HWND,
    /// The `Box<Data>` the proc reads; freed in `Drop` after the subclass is removed.
    data: *mut Data,
}

impl Monitor {
    /// Subclass the window (the eframe/winit window's `HWND`) and report pen samples and cancels
    /// through `callback`, synthesizing the pen's mouse messages for the window. Call on the
    /// window's thread.
    pub fn install(hwnd: *mut std::ffi::c_void, callback: impl Fn(Signal) + 'static) -> Result<Self, Error> {
        let hwnd = HWND(hwnd);
        if hwnd.is_invalid() {
            return Err(Error::Unsupported("no window handle".into()));
        }
        let data = Box::into_raw(Box::new(Data {
            callback: Box::new(callback),
            state: Cell::new(State::default()),
            last: Cell::new(POINT::default()),
            cursor: Cell::new(POINT { x: i32::MIN, y: i32::MIN }),
            cursor_echoes: RefCell::new(VecDeque::new()),
            contacted: Cell::new(None),
            right_press: Cell::new(POINT::default()),
            right_pinned: Cell::new(false),
            feel: Cell::new(PenFeel::default()),
            feel_stamp: Cell::new(None),
        }));
        // SAFETY: `hwnd` is a live window (the caller keeps it alive); `data` is a `Box<Data>` the
        // proc reads until `Drop` removes the subclass; `subclass_proc` is a plain
        // `extern "system"` fn; `SUBCLASS_ID` identifies this subclass for removal.
        if !unsafe { SetWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID, data as usize) }.as_bool() {
            // SAFETY: the subclass was not installed, so the proc will never read `data`.
            drop(unsafe { Box::from_raw(data) });
            return Err(Error::Platform("SetWindowSubclass failed".into()));
        }
        // The first load (later ones follow the settings file's mtime, on each pen contact).
        // SAFETY: the subclass was just installed and reads `data` until `Drop` removes it.
        load_feel(unsafe { &*data });
        Ok(Monitor { hwnd, data })
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        // SAFETY: the subclass with `SUBCLASS_ID` was installed on this window's thread, and
        // `Drop` runs there too. Removing it ends the callbacks, so the proc cannot read `data`
        // while (and after) it is freed below. If the window is already destroyed this fails and
        // the callback box stays alive, which is harmless (nothing reads it).
        unsafe {
            let _ = RemoveWindowSubclass(self.hwnd, Some(subclass_proc), SUBCLASS_ID);
            drop(Box::from_raw(self.data));
        }
    }
}

/// The subclass proc: swallows pen frames (synthesizing their mouse messages), turns the pen's
/// `WM_CONTEXTMENU` gestures into right clicks and passes every other message on to winit.
unsafe extern "system" fn subclass_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, dwref: usize) -> LRESULT {
    if dwref != 0 {
        // SAFETY: `dwref` is the `Box<Data>` `install` leaked; it stays alive until `Drop` removes
        // this subclass (or the window is destroyed, which ends the callbacks first).
        let data = unsafe { &*(dwref as *const Data) };
        match msg {
            // The echo of a `SetCursorPos` parking (see the module docs): it reports a position
            // this module already synthesized newer moves past, and letting it through makes the
            // stroke walk backward for a sample — the jitter while drawing straight lines.
            WM_MOUSEMOVE if swallow_cursor_echo(hwnd, data, coords(lparam)) => {
                return LRESULT(0);
            }
            WM_POINTERDOWN | WM_POINTERUPDATE | WM_POINTERUP => {
                // Tip Feel can change mid-session (Wacom's properties rewrite the file); each
                // contact's first frame re-reads it if the file changed since the last load.
                if msg == WM_POINTERDOWN {
                    load_feel(data);
                }
                let pointer_id = (wparam.0 & 0xFFFF) as u32; // LOWORD
                match read(pointer_id) {
                    Reading::Pen { mut raw, at } => {
                        // The Windows Ink path delivers the pen's raw pressure (the driver keeps
                        // its Tip Feel curve for Wintab), so play the curve onto the frame here.
                        let feel = data.feel.get();
                        raw.pressure = if raw.eraser { feel.eraser } else { feel.tip }.apply(raw.pressure);
                        frame(hwnd, data, &raw, at);
                        return LRESULT(0);
                    }
                    // A pen whose info could not be read (a device that went away mid-message):
                    // treat it as the contact ending, so a synthesized button cannot get stuck.
                    Reading::Lost => {
                        frame(hwnd, data, &RawFrame::default(), data.last.get());
                        return LRESULT(0);
                    }
                    // Not a pen (a finger, a mouse in pointer mode): winit's to handle.
                    Reading::Other => {}
                }
            }
            // lParam == -1 is the keyboard's context menu (Shift+F10 / the Menu key): no pen there.
            // A pen gesture is the message's own (swallowed: the app wants its right-click UI).
            WM_CONTEXTMENU if lparam.0 != -1 && context_menu(hwnd, data, coords(lparam)) => {
                return LRESULT(0);
            }
            _ => {}
        }
    }
    // SAFETY: plain pass-through; winit's wndproc must see every message that isn't ours.
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

/// Handle one pen frame: advance the state machine, synthesize its mouse messages at `at` (the
/// frame's screen position) and report its signal.
fn frame(hwnd: HWND, data: &Data, raw: &RawFrame, at: POINT) {
    // The subclass swallows the pen's `WM_POINTER*` frames before the system can promote them to
    // legacy mouse messages, so the pen would never move the real cursor (it would stay parked
    // wherever the mouse left it, and the mouse would resume from that stale spot): keep the
    // cursor on the pen, making both devices one pointer (as native Windows Ink does).
    if data.cursor.get() != at {
        // SAFETY: moves the cursor to the pen's frame position; the pen owns the pointer while
        // its frames arrive. It queues a `WM_MOUSEMOVE` echo on the input queue (dispatched after
        // the posted messages), which [`swallow_cursor_echo`] intercepts; a failed call queues
        // nothing, so only a success awaits an echo.
        if unsafe { SetCursorPos(at.x, at.y) }.is_ok() {
            let mut echoes = data.cursor_echoes.borrow_mut();
            if echoes.len() >= ECHO_CAP {
                echoes.pop_front();
            }
            echoes.push_back(at);
        }
        data.cursor.set(at);
    }
    data.last.set(at);
    let at = client_point(hwnd, at);
    let mut state = data.state.get();
    let step = advance(&mut state, raw);
    data.state.set(state);
    // A touch or a cancelled contact: a `WM_CONTEXTMENU` soon after is the pen's gesture. A pen
    // merely leaving hover range holds nothing and must not claim a later mouse context menu.
    if raw.in_contact || step.signal == Signal::Cancel {
        data.contacted.set(Some(Instant::now()));
    }
    // Click slop (`CLICK_SLOP_PX`): keep the synthesized right button where its contact began
    // until the pen clearly drags away, so a jittery tap stays a click for the UI.
    let mut at = at;
    match step.mouse {
        Mouse::Press(Button::Right) => {
            data.right_press.set(at);
            data.right_pinned.set(true);
        }
        Mouse::Move(Button::Right) | Mouse::Release(Button::Right) if data.right_pinned.get() => {
            let origin = data.right_press.get();
            let (dx, dy) = (at.x - origin.x, at.y - origin.y);
            if dx * dx + dy * dy > CLICK_SLOP_PX * CLICK_SLOP_PX {
                data.right_pinned.set(false);
            } else {
                at = origin;
            }
        }
        // A left press ends any stale pin (its release was lost).
        Mouse::Press(Button::Left) => data.right_pinned.set(false),
        _ => {}
    }
    post_mouse(hwnd, at, step.mouse);
    deliver_signal(&data.callback, step.signal);
}

/// Handle a pen `WM_CONTEXTMENU` at `at` (screen coordinates): Windows Ink's system right click.
/// Returns whether the message was the pen's (then it is swallowed: the app wants its own
/// right-click UI, not a native menu).
fn context_menu(hwnd: HWND, data: &Data, at: POINT) -> bool {
    if !data.contacted.get().is_some_and(|t| t.elapsed() < GESTURE_WINDOW) {
        return false;
    }
    let at = client_point(hwnd, at);
    let mut state = data.state.get();
    // `gesture` returns `None` when the frames already synthesized that click (the message is
    // their echo): swallow it, or the gesture would re-fire forever.
    let Some(g) = gesture(&mut state) else { return true };
    data.state.set(state);
    if let Some(b) = g.release {
        // The gesture's tip contact is still held (the system did not cancel it): release it. A
        // left one had begun a stroke, and the UI must drop that.
        post_mouse(hwnd, at, Mouse::Release(b));
        if b == Button::Left {
            deliver_signal(&data.callback, Signal::Cancel);
        }
    }
    // The gesture is the right click: press and release the right button at the pointer.
    post(hwnd, WM_MOUSEMOVE, WPARAM(0), at);
    post(hwnd, WM_RBUTTONDOWN, WPARAM(MK_RBUTTON), at);
    post(hwnd, WM_RBUTTONUP, WPARAM(0), at);
    true
}

/// What one `WM_POINTER*` message is (read inside the message, where its pointer id is valid).
enum Reading {
    /// A pen frame.
    Pen { raw: RawFrame, at: POINT },
    /// A pen frame whose `POINTER_PEN_INFO` could not be read.
    Lost,
    /// Not a pen (another pointer device).
    Other,
}

/// Load the Wacom driver's Tip Feel into `data` (see [`crate::wacom`]) if its settings file
/// changed since the last load — a no-op on a machine without the driver or the file. Runs at
/// install and on each pen contact's first frame; the file is rewritten by the driver whenever
/// its properties change, and the mtime guard keeps the per-contact re-read cheap.
fn load_feel(data: &Data) {
    let Some(path) = wacom::settings_path() else { return };
    let stamp = fs::metadata(&path).and_then(|m| m.modified()).ok();
    if stamp.is_some() && stamp == data.feel_stamp.get() {
        return;
    }
    let exe = std::env::current_exe().ok().map(|p| p.to_string_lossy().into_owned());
    match wacom::load(exe.as_deref()) {
        Some(feel) => {
            data.feel.set(feel);
            data.feel_stamp.set(stamp);
        }
        // Unreadable (mid-rewrite or an unrecognized file): keep what we had, and no stamp, so
        // the next contact retries.
        None => data.feel_stamp.set(None),
    }
}

/// Read the pen fields of the message's pointer as plain values.
fn read(pointer_id: u32) -> Reading {
    let mut ty = POINTER_INPUT_TYPE(0);
    // SAFETY: `pointer_id` comes from the message's `WPARAM` and is valid inside this
    // `WM_POINTER*` message; the out pointer lives for the call.
    if unsafe { GetPointerType(pointer_id, &mut ty) }.is_err() || ty != PT_PEN {
        return Reading::Other;
    }
    let mut info = POINTER_PEN_INFO::default();
    // SAFETY: same message scope as `GetPointerType` above.
    if unsafe { GetPointerPenInfo(pointer_id, &mut info) }.is_err() {
        return Reading::Lost;
    }
    let p = &info.pointerInfo;
    Reading::Pen {
        raw: RawFrame {
            in_contact: (p.pointerFlags & POINTER_FLAG_INCONTACT).0 != 0,
            cancelled: (p.pointerFlags & POINTER_FLAG_CANCELED).0 != 0,
            barrel: (p.pointerFlags & POINTER_FLAG_SECONDBUTTON).0 != 0 || info.penFlags & PEN_FLAG_BARREL != 0,
            eraser: info.penFlags & (PEN_FLAG_ERASER | PEN_FLAG_INVERTED) != 0,
            pressure: info.pressure as f32 / 1024.0,
            tilt_x: info.tiltX as f32,
            tilt_y: info.tiltY as f32,
            rotation: info.rotation as f32,
        },
        at: p.ptPixelLocation,
    }
}

/// Synthesize the mouse messages of one frame at `at` (a client-area point), in order.
fn post_mouse(hwnd: HWND, at: POINT, mouse: Mouse) {
    match mouse {
        Mouse::Hover => post(hwnd, WM_MOUSEMOVE, WPARAM(0), at),
        Mouse::Move(b) => post(hwnd, WM_MOUSEMOVE, WPARAM(flag(b)), at),
        Mouse::Press(b) => {
            post(hwnd, WM_MOUSEMOVE, WPARAM(0), at);
            post(hwnd, down(b), WPARAM(flag(b)), at);
        }
        Mouse::Release(b) => {
            post(hwnd, WM_MOUSEMOVE, WPARAM(flag(b)), at);
            post(hwnd, up(b), WPARAM(0), at);
        }
    }
}

/// Post one synthesized mouse message; `lparam` packs the client coordinates.
fn post(hwnd: HWND, msg: u32, wparam: WPARAM, at: POINT) {
    let packed = ((at.y as u32 & 0xFFFF) << 16) | (at.x as u32 & 0xFFFF);
    // SAFETY: `hwnd` is the window being subclassed (alive while the subclass is installed);
    // `PostMessageW` only queues the message and does not read any pointers.
    let _ = unsafe { PostMessageW(Some(hwnd), msg, wparam, LPARAM(packed as i32 as isize)) };
}

/// A screen point as a client point (the coordinate space of the mouse messages).
fn client_point(hwnd: HWND, mut p: POINT) -> POINT {
    // SAFETY: `hwnd` is the window being subclassed; `ScreenToClient` only reads `hwnd` and
    // writes `p`.
    let _ = unsafe { ScreenToClient(hwnd, &mut p) };
    p
}

/// Whether this `WM_MOUSEMOVE` is the echo of a `SetCursorPos` parking (then it is swallowed and
/// its entry forgotten). The echo reports the parked point itself, so its client coordinates,
/// converted back to screen, match a remembered target exactly. A real mouse move at that exact
/// point is dropped too — harmless: the pen owns the pointer while echoes are pending, and the
/// mouse's next move differs by more than 0 and corrects it. An entry that never finds its echo
/// (a clipped or blocked cursor) is dropped once [`ECHO_CAP`] newer ones arrived.
fn swallow_cursor_echo(hwnd: HWND, data: &Data, at: POINT) -> bool {
    let mut p = at;
    // SAFETY: `hwnd` is the window being subclassed; `ClientToScreen` only reads `hwnd` and
    // writes `p`.
    let _ = unsafe { ClientToScreen(hwnd, &mut p) };
    let mut echoes = data.cursor_echoes.borrow_mut();
    match echoes.iter().position(|&e| e.x == p.x && e.y == p.y) {
        Some(i) => {
            echoes.remove(i);
            true
        }
        None => false,
    }
}

/// The signed halves of a message's `LPARAM` (GET_X_LPARAM / GET_Y_LPARAM).
fn coords(lparam: LPARAM) -> POINT {
    let l = lparam.0 as u32;
    POINT { x: (l & 0xFFFF) as u16 as i16 as i32, y: (l >> 16) as u16 as i16 as i32 }
}

fn down(b: Button) -> u32 {
    if b == Button::Left { WM_LBUTTONDOWN } else { WM_RBUTTONDOWN }
}

fn up(b: Button) -> u32 {
    if b == Button::Left { WM_LBUTTONUP } else { WM_RBUTTONUP }
}

fn flag(b: Button) -> usize {
    if b == Button::Left { MK_LBUTTON } else { MK_RBUTTON }
}
