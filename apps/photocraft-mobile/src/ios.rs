//! The C interface the SwiftUI shell calls (declared in `ios/PhotoCraft.h`).
//!
//! Every call comes from the main thread (UIKit's touches, layout and `CADisplayLink`), so the
//! one host lives in a thread-local: no handle crosses the boundary, and only [`pc_start`] takes
//! a pointer. A call before `pc_start` or after `pc_stop` does nothing.

use std::cell::RefCell;
use std::ffi::c_void;

use eframe::wgpu;
use egui::{Pos2, TouchPhase};

use crate::host::{Host, Touch, ViewSize};

thread_local! {
    static HOST: RefCell<Option<Host>> = const { RefCell::new(None) };
}

fn with_host<R>(f: impl FnOnce(&mut Host) -> R) -> Option<R> {
    HOST.with(|h| h.borrow_mut().as_mut().map(f))
}

/// Start PhotoCraft drawing into `layer`, a `CAMetalLayer`. Returns whether it started; on
/// failure the reason is logged. Calling it again restarts the app in the new layer.
///
/// # Safety
/// `layer` must be a valid `CAMetalLayer` that stays alive until [`pc_stop`] is called.
#[expect(unsafe_code, reason = "exported to Swift; takes the view's layer as a raw pointer")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pc_start(layer: *mut c_void, width_px: u32, height_px: u32, scale: f32) -> bool {
    if layer.is_null() || !(scale.is_finite() && scale > 0.0) {
        log::error!("photocraft-mobile: pc_start needs a layer and a positive scale");
        return false;
    }
    let size = ViewSize { width_px, height_px, scale };
    // SAFETY: the caller guarantees the layer outlives the host; pc_stop drops the host first.
    match unsafe { Host::new(wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(layer), size) } {
        Ok(host) => {
            HOST.with(|h| *h.borrow_mut() = Some(host));
            true
        }
        Err(e) => {
            log::error!("photocraft-mobile: {e}");
            false
        }
    }
}

/// Drop the app and its surface. Call before the layer goes away.
#[expect(unsafe_code, reason = "exported to Swift")]
#[unsafe(no_mangle)]
pub extern "C" fn pc_stop() {
    HOST.with(|h| h.borrow_mut().take());
}

/// The view changed size or moved to a screen with another scale.
#[expect(unsafe_code, reason = "exported to Swift")]
#[unsafe(no_mangle)]
pub extern "C" fn pc_resize(width_px: u32, height_px: u32, scale: f32) {
    if scale.is_finite() && scale > 0.0 {
        with_host(|h| h.resize(ViewSize { width_px, height_px, scale }));
    }
}

/// Called on every display refresh. Draws only when something changed; returns whether it drew.
#[expect(unsafe_code, reason = "exported to Swift")]
#[unsafe(no_mangle)]
pub extern "C" fn pc_frame() -> bool {
    with_host(Host::frame).unwrap_or(false)
}

/// A touch, in points from the view's top-left. `phase`: 0 began, 1 moved, 2 ended,
/// 3 cancelled; anything else is ignored. `force` is 0..=1, or 0 where the device has none.
#[expect(unsafe_code, reason = "exported to Swift")]
#[unsafe(no_mangle)]
pub extern "C" fn pc_touch(id: u64, phase: u32, x: f32, y: f32, force: f32) {
    let phase = match phase {
        0 => TouchPhase::Start,
        1 => TouchPhase::Move,
        2 => TouchPhase::End,
        3 => TouchPhase::Cancel,
        _ => return,
    };
    if !(x.is_finite() && y.is_finite()) {
        return;
    }
    let force = if force.is_finite() { force.clamp(0.0, 1.0) } else { 0.0 };
    with_host(|h| h.touch(Touch { id, phase, pos: Pos2::new(x, y), force }));
}
