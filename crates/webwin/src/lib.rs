//! The preview window's WebView2 child, and the OS handle of the window it lives in.
//!
//! `ui-egui` forbids unsafe code (`#![forbid(unsafe_code)]`), so the two places that need it live
//! here: borrowing the preview window's raw handle for `wry`, and walking this process's windows to
//! find that handle — the window is an egui viewport, which only exposes its handle through the OS.
//!
//! Windows only. On other platforms every call reports an error and the caller falls back to the
//! system browser.

/// Where the page sits inside the preview window: no tool strip, so a full client area.
#[cfg(windows)]
fn bounds(x: f64, y: f64, w: f64, h: f64) -> wry::Rect {
    wry::Rect {
        position: wry::dpi::Position::Physical(wry::dpi::PhysicalPosition::<i32>::new(x.round() as i32, y.round() as i32)),
        size: wry::dpi::Size::Physical(wry::dpi::PhysicalSize::<u32>::new(w.round().max(1.0) as u32, h.round().max(1.0) as u32)),
    }
}

/// The preview window's top-level handle, if a window with `title` in it exists.
pub fn find_window(title: &str) -> Option<isize> {
    #[cfg(windows)]
    {
        win::find_by_title(title)
    }
    #[cfg(not(windows))]
    {
        let _ = title;
        None
    }
}

/// Whether `hwnd` still exists (the user can close the preview window at any time).
pub fn is_window(hwnd: isize) -> bool {
    #[cfg(windows)]
    {
        win::is_window(hwnd)
    }
    #[cfg(not(windows))]
    {
        let _ = hwnd;
        false
    }
}

/// The window's client area in device pixels: `(x, y)` of its top-left corner on the desktop, then
/// `(width, height)`. This is what the page is sized to and what a screenshot captures.
pub fn client_rect(hwnd: isize) -> Option<(i32, i32, u32, u32)> {
    #[cfg(windows)]
    {
        win::client_rect(hwnd)
    }
    #[cfg(not(windows))]
    {
        let _ = hwnd;
        None
    }
}

/// Raise the window above its siblings, so nothing covers it while the page is being captured.
pub fn bring_to_front(hwnd: isize) {
    #[cfg(windows)]
    {
        win::bring_to_front(hwnd);
    }
    #[cfg(not(windows))]
    {
        let _ = hwnd;
    }
}

/// A page live inside the preview window.
pub struct Page {
    #[cfg(windows)]
    webview: wry::WebView,
    /// Client size (device pixels) the page was last sized to.
    sized: (u32, u32),
}

/// Put `url` into the window `hwnd`, covering its client area below `strip_px` device pixels.
/// `size` is the client area in device pixels.
pub fn attach(hwnd: isize, url: &str, strip_px: f64, size: (f64, f64), zoom: f64) -> Result<Page, String> {
    let (w, h) = size;
    #[cfg(windows)]
    {
        let webview = wry::WebViewBuilder::new()
            .with_url(url)
            .with_bounds(bounds(0.0, strip_px, w, h - strip_px))
            .with_focused(false)
            .build_as_child(&HwndRef(hwnd))
            .map_err(|e| e.to_string())?;
        let _ = webview.zoom(zoom);
        Ok(Page { webview, sized: (w.round().max(1.0) as u32, h.round().max(1.0) as u32) })
    }
    #[cfg(not(windows))]
    {
        let _ = (hwnd, url, strip_px, zoom);
        Err("the preview window is only supported on Windows".into())
    }
}

impl Page {
    /// The client size the page was last sized to.
    pub fn size(&self) -> (u32, u32) {
        self.sized
    }

    /// Follow the window: size the page to its client area below `strip_px`. True when it changed.
    pub fn fit(&mut self, strip_px: f64, size: (f64, f64)) -> bool {
        let (w, h) = size;
        let want = (w.round().max(1.0) as u32, h.round().max(1.0) as u32);
        if want == self.sized {
            return false;
        }
        self.sized = want;
        #[cfg(windows)]
        {
            let _ = self.webview.set_bounds(bounds(0.0, strip_px, w, h - strip_px));
        }
        true
    }

    pub fn zoom(&self, zoom: f64) {
        #[cfg(windows)]
        {
            let _ = self.webview.zoom(zoom);
        }
        #[cfg(not(windows))]
        {
            let _ = zoom;
        }
    }

    pub fn load(&self, url: &str) {
        #[cfg(windows)]
        {
            let _ = self.webview.load_url(url);
        }
        #[cfg(not(windows))]
        {
            let _ = url;
        }
    }
}

/// A window handle handed to `wry`, which asks for `HasWindowHandle`.
#[cfg(windows)]
struct HwndRef(isize);

#[cfg(windows)]
impl raw_window_handle::HasWindowHandle for HwndRef {
    fn window_handle(&self) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        let hwnd = std::num::NonZeroIsize::new(self.0).ok_or(raw_window_handle::HandleError::Unavailable)?;
        let mut handle = raw_window_handle::Win32WindowHandle::new(hwnd);
        handle.hinstance = None;
        let raw = raw_window_handle::RawWindowHandle::Win32(handle);
        Ok(unsafe { raw_window_handle::WindowHandle::borrow_raw(raw) })
    }
}

#[cfg(windows)]
mod win {
    //! Walking this process's top-level windows to find the preview window by its title.

    use windows_sys::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
    use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
    use windows_sys::Win32::UI::WindowsAndMessaging::{BringWindowToTop, EnumWindows, GetClientRect, GetWindowTextW, IsWindow, IsWindowVisible, SetForegroundWindow};

    struct Search {
        needle: Vec<u16>,
        found: isize,
    }

    fn contains(hay: &[u16], needle: &[u16]) -> bool {
        !needle.is_empty() && hay.windows(needle.len()).any(|w| w == needle)
    }

    unsafe extern "system" fn visit(hwnd: HWND, lparam: LPARAM) -> windows_sys::core::BOOL {
        let search = unsafe { &mut *(lparam as *mut Search) };
        if unsafe { IsWindowVisible(hwnd) } == 0 {
            return 1;
        }
        let mut text = [0u16; 512];
        let n = unsafe { GetWindowTextW(hwnd, text.as_mut_ptr(), text.len() as i32) };
        if n > 0 && contains(&text[..n as usize], &search.needle) {
            search.found = hwnd as isize;
            return 0;
        }
        1
    }

    /// This process's top-level window whose title contains `title`.
    pub fn find_by_title(title: &str) -> Option<isize> {
        let mut search = Search { needle: title.encode_utf16().collect(), found: 0 };
        unsafe { EnumWindows(Some(visit), &mut search as *mut Search as LPARAM) };
        (search.found != 0).then_some(search.found)
    }

    pub fn is_window(hwnd: isize) -> bool {
        unsafe { IsWindow(hwnd as HWND) != 0 }
    }

    /// The window's client area: its top-left corner on the desktop, then its size, both in device
    /// pixels. `ClientToScreen` is where the OS reports that corner, so this needs no scaling of
    /// egui's point coordinates — which was the previous, wrong, approach.
    pub fn client_rect(hwnd: isize) -> Option<(i32, i32, u32, u32)> {
        let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        let mut at = POINT { x: 0, y: 0 };
        unsafe {
            if GetClientRect(hwnd as HWND, &mut rect) == 0 {
                return None;
            }
            if ClientToScreen(hwnd as HWND, &mut at) == 0 {
                return None;
            }
        }
        let w = (rect.right - rect.left).max(0) as u32;
        let h = (rect.bottom - rect.top).max(0) as u32;
        (w > 0 && h > 0).then_some((at.x, at.y, w, h))
    }

    /// Put the window above everything it overlaps, and give it the foreground so its page is what
    /// a screen capture sees.
    pub fn bring_to_front(hwnd: isize) {
        unsafe {
            BringWindowToTop(hwnd as HWND);
            SetForegroundWindow(hwnd as HWND);
        }
    }
}
