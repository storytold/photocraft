//! X11 window discovery for the AppMenu registrar: this process's top-level windows,
//! scanned with a second pure-Rust X connection (x11rb's `RustConnection`, no `unsafe`).
//!
//! D-Bus menu registration is per top-level window (`RegisterWindow(windowId, menuPath)`),
//! so the exporter needs the X window ids. winit stamps every map it creates with
//! `_NET_WM_PID`, so a background scan of the root tree by process id finds them safely —
//! and every window is registered, in case the app opens a second document window.

use x11rb::connection::Connection as _;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _};
use x11rb::rust_connection::RustConnection;

/// Window-type atoms that mean "not a menu bar host": docks, notifications, tooltips,
/// pop-ups, splash screens, panels. (winit sets none of these on real windows; the check
/// keeps us from registering other clients' odd windows should the pid trick misfire.)
const POPUP_TYPES: &[&[u8]] = &[
    b"_NET_WM_WINDOW_TYPE_DOCK",
    b"_NET_WM_WINDOW_TYPE_DESKTOP",
    b"_NET_WM_WINDOW_TYPE_TOOLBAR",
    b"_NET_WM_WINDOW_TYPE_MENU",
    b"_NET_WM_WINDOW_TYPE_SPLASH",
    b"_NET_WM_WINDOW_TYPE_UTILITY",
    b"_NET_WM_WINDOW_TYPE_NOTIFICATION",
    b"_NET_WM_WINDOW_TYPE_TOOLTIP",
    b"_NET_WM_WINDOW_TYPE_DROPDOWN_MENU",
    b"_NET_WM_WINDOW_TYPE_POPUP_MENU",
    b"_NET_WM_WINDOW_TYPE_COMBO",
    b"_NET_WM_WINDOW_TYPE_DND",
];

/// Cap on how many windows a scan may return (a root with thousands of children would still
/// finish quickly, but registration is bounded work too).
const MAX_WINDOWS: usize = 32;

/// The top-level X windows of process `pid` (empty on any error — scans are best effort:
/// the registrar simply sees no window and the in-window menu bar stays).
///
/// The managed clients come from `_NET_CLIENT_LIST` (one EWMH property — every reparenting
/// window manager keeps it, and it is exactly the set a menu host asks for, because focus
/// lands on those client ids, not on a WM's frame). A window manager without EWMH gets the
/// shallow root-children fallback instead. Note the reparenting trap the naive scan falls
/// into: a WM frame (and on Gershwin even a *copy* of the client's `_NET_WM_PID`) is not the
/// window a shell focuses, so root children are a fallback, never the primary source.
pub fn top_level_windows(pid: u32) -> Vec<u32> {
    let Ok((conn, screen)) = RustConnection::connect(None) else {
        return Vec::new();
    };
    let Some(root) = conn.setup().roots.get(screen).map(|r| r.root) else {
        return Vec::new();
    };
    if let Some(clients) = window_list(&conn, root, b"_NET_CLIENT_LIST") {
        return ours_only(&conn, clients, pid);
    }
    ours_only(&conn, children(&conn, root), pid)
}

/// The windows of the list this process owns and doesn't look like a pop-up.
fn ours_only(conn: &RustConnection, windows: Vec<u32>, pid: u32) -> Vec<u32> {
    let Some(pid_atom) = intern(conn, b"_NET_WM_PID") else { return Vec::new() };
    let excluded: Vec<u32> = POPUP_TYPES.iter().filter_map(|n| intern(conn, n)).collect();
    let type_atom = intern(conn, b"_NET_WM_WINDOW_TYPE");
    let mut out = Vec::new();
    for window in windows {
        if out.len() >= MAX_WINDOWS {
            break;
        }
        if cardinal(conn, window, pid_atom) != Some(pid) {
            continue;
        }
        if let Some(type_atom) = type_atom
            && property_atoms(conn, window, type_atom).iter().any(|a| excluded.contains(a))
        {
            continue;
        }
        out.push(window);
    }
    out
}

/// One array-of-WINDOW property (`_NET_CLIENT_LIST` and friends); None on any error.
fn window_list(conn: &RustConnection, window: u32, name: &[u8]) -> Option<Vec<u32>> {
    let atom = intern(conn, name)?;
    let cookie = conn.get_property(false, window, atom, AtomEnum::WINDOW, 0, 256).ok()?;
    let reply = cookie.reply().ok()?;
    Some(reply.value.as_chunks::<4>().0.iter().map(|b| u32::from_le_bytes(*b)).collect())
}

/// The immediate children of a window (empty on any error — the caller keeps walking).
fn children(conn: &RustConnection, window: u32) -> Vec<u32> {
    conn.query_tree(window).ok().and_then(|c| c.reply().ok()).map(|tree| tree.children).unwrap_or_default()
}

/// Never returns a window whose child also carries the same pid: with a reparenting window
/// manager the frame window often carries a copy of the client's `_NET_WM_PID`, and the
/// depth-first pick must keep the client (which is what a menu host asks for by focused
/// window id), not its frame. Verified against whatever real layout is on the display; any
/// other process's pid-stamped shell windows serve as the test object.
#[test]
fn scans_pick_the_client_under_any_frame() {
    if std::env::var_os("DISPLAY").is_none() {
        eprintln!("skipping (no X display)");
        return;
    }
    let Ok((conn, screen)) = RustConnection::connect(None) else {
        eprintln!("skipping (X connection failed)");
        return;
    };
    let Some(root) = conn.setup().roots.get(screen).map(|r| r.root) else { return };
    let Some(pid_atom) = intern(&conn, b"_NET_WM_PID") else { return };
    // Find any process id stamped on a root child (some other running X client).
    let mut target = None;
    let mut attempts = 0;
    for window in children(&conn, root) {
        attempts += 1;
        if let Some(pid) = cardinal(&conn, window, pid_atom) {
            target = Some(pid);
            break;
        }
    }
    let Some(target) = target else {
        eprintln!("skipping (no pid-stamped root windows; tried {attempts})");
        return;
    };
    let found = top_level_windows(target);
    assert!(found.len() <= MAX_WINDOWS, "the scan cap holds");
    // The invariant every other test below relies on.
    for &window in &found {
        for child in children(&conn, window) {
            assert_ne!(cardinal(&conn, child, pid_atom), Some(target), "window {window:#x} is a frame: its child {child:#x} carries the same pid");
        }
    }
}

fn intern(conn: &RustConnection, name: &[u8]) -> Option<u32> {
    conn.intern_atom(false, name).ok()?.reply().ok().map(|r| r.atom)
}

/// The first CARDINAL value of a window property (0-length on any error).
fn cardinal(conn: &RustConnection, window: u32, atom: u32) -> Option<u32> {
    let cookie = conn.get_property(false, window, atom, AtomEnum::CARDINAL, 0, 1).ok()?;
    let reply = cookie.reply().ok()?;
    let bytes: [u8; 4] = reply.value.get(0..4)?.try_into().ok()?;
    Some(u32::from_le_bytes(bytes))
}

/// The ATOM values of a window property (empty on any error).
fn property_atoms(conn: &RustConnection, window: u32, atom: u32) -> Vec<u32> {
    let Ok(cookie) = conn.get_property(false, window, atom, AtomEnum::ATOM, 0, 32) else {
        return Vec::new();
    };
    let Ok(reply) = cookie.reply() else { return Vec::new() };
    reply.value.as_chunks::<4>().0.iter().map(|b| u32::from_le_bytes(*b)).collect()
}
