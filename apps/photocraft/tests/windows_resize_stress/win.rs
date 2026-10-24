//! The Win32 side of the resize stress test: real mouse input, window geometry, and the app's memory.
//! Safe `winsafe` wrappers only (the workspace forbids `unsafe`).

use std::sync::Once;

use winsafe::{self as w, co};

/// Make this process DPI aware, so window rectangles, cursor positions and `SendInput` all speak physical
/// pixels. (The app under test sets its own awareness; this only concerns the test process.)
pub fn init_dpi() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = w::SetProcessDPIAware();
    });
}

/// The primary display's size in px.
pub fn screen_size() -> (i32, i32) {
    (w::GetSystemMetrics(co::SM::CXSCREEN), w::GetSystemMetrics(co::SM::CYSCREEN))
}

/// The virtual desktop's origin and size, which `SendInput` coordinates are normalised to.
fn virtual_screen() -> (i32, i32, i32, i32) {
    (
        w::GetSystemMetrics(co::SM::XVIRTUALSCREEN),
        w::GetSystemMetrics(co::SM::YVIRTUALSCREEN),
        w::GetSystemMetrics(co::SM::CXVIRTUALSCREEN),
        w::GetSystemMetrics(co::SM::CYVIRTUALSCREEN),
    )
}

/// Inject one mouse event. Whether the OS took it: it does not when a window of a more privileged process has
/// the foreground (input cannot be injected into that).
fn send_mouse(flags: co::MOUSEEVENTF, dx: i32, dy: i32) -> bool {
    let input = w::HwKbMouse::Mouse(w::MOUSEINPUT { dx, dy, mouseData: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 });
    w::SendInput(&[input]).is_ok_and(|sent| sent == 1)
}

/// Move the real cursor to `(x, y)` (screen px) the way a mouse does, through the input queue, so that the
/// OS's modal sizing loop sees it. `SetCursorPos` would not drive that loop the same way.
pub fn move_cursor(x: i32, y: i32) -> bool {
    let (ox, oy, width, height) = virtual_screen();
    let norm = |v: i32, origin: i32, size: i32| i32::try_from(i64::from(v - origin) * 65535 / i64::from((size - 1).max(1))).unwrap_or(0);
    send_mouse(co::MOUSEEVENTF::MOVE | co::MOUSEEVENTF::ABSOLUTE | co::MOUSEEVENTF::VIRTUALDESK, norm(x, ox, width), norm(y, oy, height))
}

/// Where the cursor is, in screen px; `None` when the OS will not say (the secure desktop has input, say).
pub fn cursor() -> Option<(i32, i32)> {
    w::GetCursorPos().ok().map(|p| (p.x, p.y))
}

/// The left mouse button, held down. Released when dropped, so a panic mid-drag cannot leave it stuck.
pub struct HeldButton(());

impl HeldButton {
    /// Press the button where the cursor is now; `None` if the OS refused the press.
    pub fn press() -> Option<HeldButton> {
        send_mouse(co::MOUSEEVENTF::LEFTDOWN, 0, 0).then_some(HeldButton(()))
    }
}

impl Drop for HeldButton {
    fn drop(&mut self) {
        send_mouse(co::MOUSEEVENTF::LEFTUP, 0, 0);
    }
}

/// The app's main window.
pub struct Window {
    hwnd: w::HWND,
    thread: u32,
}

/// The visible top-level window of process `pid` that is big enough to be the main window. Matching on the
/// process keeps other running PhotoCraft instances out of the test.
pub fn find_window(pid: u32) -> Option<Window> {
    let mut found = None;
    let _ = w::EnumWindows(|hwnd| {
        let (thread, owner) = hwnd.GetWindowThreadProcessId();
        let big = hwnd.GetWindowRect().is_ok_and(|r| r.right - r.left > 300 && r.bottom - r.top > 200);
        if owner == pid && hwnd.IsWindowVisible() && big {
            found = Some(Window { hwnd, thread });
            return false;
        }
        true
    });
    found
}

impl Window {
    /// Left, top, right, bottom of the outer frame, in screen px.
    pub fn rect(&self) -> Option<(i32, i32, i32, i32)> {
        self.hwnd.GetWindowRect().ok().map(|r| (r.left, r.top, r.right, r.bottom))
    }

    /// The client area's size, which is what the app draws into.
    pub fn client(&self) -> Option<(i32, i32)> {
        self.hwnd.GetClientRect().ok().map(|r| (r.right - r.left, r.bottom - r.top))
    }

    /// Restore the window and put it at `pos` with outer size `size`, above every other window while the
    /// test runs: a click can then only land on it, never on whatever was behind.
    pub fn park(&self, pos: (i32, i32), size: (i32, i32)) -> bool {
        let _ = self.hwnd.ShowWindow(co::SW::RESTORE);
        let placed = self.hwnd.SetWindowPos(
            w::HwndPlace::Place(co::HWND_PLACE::TOPMOST),
            w::POINT { x: pos.0, y: pos.1 },
            w::SIZE { cx: size.0, cy: size.1 },
            co::SWP::SHOWWINDOW,
        );
        let _ = self.hwnd.SetForegroundWindow();
        placed.is_ok()
    }

    /// Change only the outer size, as a program (not a person) resizing the window.
    pub fn resize(&self, size: (i32, i32)) -> bool {
        let at = w::POINT { x: 0, y: 0 };
        let flags = co::SWP::NOMOVE | co::SWP::NOZORDER | co::SWP::NOACTIVATE;
        self.hwnd.SetWindowPos(w::HwndPlace::None, at, w::SIZE { cx: size.0, cy: size.1 }, flags).is_ok()
    }

    /// Maximize the window. Like every `ShowWindow` call this returns only once the app has handled the resulting
    /// `WM_SIZE`, which for PhotoCraft includes painting a frame, so it is a resize the app must keep up with.
    pub fn maximize(&self) {
        let _ = self.hwnd.ShowWindow(co::SW::SHOWMAXIMIZED);
    }

    /// Restore the window from maximized, with the same blocking as [`Window::maximize`].
    pub fn restore(&self) {
        let _ = self.hwnd.ShowWindow(co::SW::RESTORE);
    }

    /// Whether the OS is in its modal move/size loop on this window's thread: proof that a drag really
    /// started an OS resize, the path a person's drag takes, and not just a window-size change.
    pub fn in_modal_loop(&self) -> bool {
        w::GetGUIThreadInfo(self.thread).is_ok_and(|info| info.flags.has(co::GUI::INMOVESIZE))
    }

    /// Whether this window is what a click at `(x, y)` would hit.
    pub fn is_under(&self, x: i32, y: i32) -> bool {
        w::HWND::WindowFromPoint(w::POINT { x, y }).and_then(|hit| hit.GetAncestor(co::GA::ROOT)).is_some_and(|root| root.ptr() == self.hwnd.ptr())
    }
}

/// The private memory of process `pid` in MB.
pub fn private_mb(pid: u32) -> Option<f64> {
    let process = w::HPROCESS::OpenProcess(co::PROCESS::QUERY_LIMITED_INFORMATION | co::PROCESS::VM_READ, false, pid).ok()?;
    let counters = process.GetProcessMemoryInfo().ok()?;
    Some(counters.PrivateUsage as f64 / 1_048_576.0)
}
