//! Original conformance DLL, built by validate-workers.py for both architectures.
#![allow(dead_code)]
#[path = "../src/pixels.rs"]
mod pixels;
#[path = "../src/record.rs"]
mod record;
use pixels::Rect;
use record::{O, Record};
use std::ffi::c_void;

#[cfg(target_pointer_width = "32")]
const DEPTH_AT: usize = 0x17c;
#[cfg(target_pointer_width = "64")]
const DEPTH_AT: usize = 0x1e0;

fn image_layout(record: &Record) -> Option<(usize, usize, usize)> {
    let planes = usize::try_from(record.i16(O.planes)).ok()?;
    let depth = i32::from_le_bytes(record.get(DEPTH_AT));
    let mode = record.i16(O.mode);
    let expected = match mode {
        1 => (1, 8),
        3 => (3, 8),
        10 => (1, 16),
        11 => (3, 16),
        _ => return None,
    };
    if (planes, depth) != expected {
        return None;
    }
    let sample_bytes = usize::try_from(depth / 8).ok()?;
    Some((planes, sample_bytes, depth as usize))
}

/// # Safety
/// The caller must supply a live, correctly sized Record and writable result/data slots;
/// all callback tables and pixel pointers must remain valid through this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn PluginMain(selector: i16, block: *mut c_void, _data: *mut isize, result: *mut i16) {
    // SAFETY: the conformance host passes live records and result slots of our shared layout.
    let r = unsafe { &mut *block.cast::<Record>() };
    unsafe {
        *result = 0;
    }
    let mode = std::env::var("PHOTOCRAFT_8BF_FIXTURE_MODE").unwrap_or_default();
    if selector == 5 {
        eprintln!("CONFORMANCE_FINISH");
        if mode == "ignored_callback" {
            // The original host callback failure must win over a later cleanup error.
            unsafe { *result = -32124 };
        }
        return;
    }
    if selector == 3 {
        if mode == "start_error" {
            unsafe { *result = -32125 };
            return;
        }
        if mode == "crash" {
            // Intentionally fault inside the disposable worker to exercise native crash
            // handling. This is test-only invalid memory access, never editor code.
            unsafe { std::ptr::write_volatile(std::ptr::dangling_mut::<u8>(), 0) };
        }
        if mode == "exit_error" {
            std::process::exit(7);
        }
        if mode == "noise" {
            println!("A plug-in wrote diagnostic output");
        }
        if mode == "cancel" {
            unsafe {
                *result = -128;
            }
            return;
        }
        if mode == "mask_request" || mode == "ignored_mask_callback" {
            r.set_rect(O.mask, r.rect(O.filter));
            if mode == "ignored_mask_callback" {
                let at = if cfg!(target_pointer_width = "64") { 0x118 } else { 0xe0 };
                // SAFETY: the worker supplies a live, entry-thread callback.
                let advance: extern "C" fn() -> i16 = unsafe { std::mem::transmute(usize::from_le_bytes(r.get(at))) };
                if advance() != 0 {
                    unsafe { *result = -32126 };
                }
                r.set_rect(O.mask, Rect::default());
            }
            return;
        }
        if mode == "threaded_alloc" {
            let buffer_at = if cfg!(target_pointer_width = "64") { 0xe0 } else { 0xc0 };
            let handle_at = if cfg!(target_pointer_width = "64") { 0x100 } else { 0xd0 };
            let buffers = usize::from_le_bytes(r.get(buffer_at));
            let handles = usize::from_le_bytes(r.get(handle_at));
            let passed = std::thread::spawn(move || {
                let functions_offset = if cfg!(target_pointer_width = "64") { 8 } else { 4 };
                // SAFETY: both callback tables live through the joined processing thread.
                unsafe {
                    let b = (buffers + functions_offset) as *const usize;
                    let h = (handles + functions_offset) as *const usize;
                    let allocate: extern "C" fn(i32, *mut usize) -> i16 = std::mem::transmute(b.read());
                    let lock: extern "C" fn(usize, u8) -> *mut u8 = std::mem::transmute(b.add(1).read());
                    let free: extern "C" fn(usize) = std::mem::transmute(b.add(3).read());
                    let new: extern "C" fn(i32) -> *mut *mut u8 = std::mem::transmute(h.read());
                    let dispose: extern "C" fn(*mut *mut u8) = std::mem::transmute(h.add(1).read());
                    let size: extern "C" fn(*mut *mut u8) -> i32 = std::mem::transmute(h.add(2).read());
                    let resize: extern "C" fn(*mut *mut u8, i32) -> i16 = std::mem::transmute(h.add(3).read());
                    let mut id = 0;
                    if allocate(32, &mut id) != 0 {
                        return false;
                    }
                    let ptr = lock(id, 0);
                    if ptr.is_null() {
                        return false;
                    }
                    ptr.write(93);
                    free(id);
                    let handle = new(0);
                    if handle.is_null() || size(handle) != 0 {
                        return false;
                    }
                    if resize(handle, 32) != 0 || size(handle) != 32 || (*handle).is_null() {
                        return false;
                    }
                    (*handle).write(73);
                    dispose(handle);
                    true
                }
            })
            .join()
            .unwrap_or(false);
            if !passed {
                unsafe {
                    *result = -1;
                }
                return;
            }
        }
        let Some((planes, _, _)) = image_layout(r) else {
            unsafe { *result = -1 };
            return;
        };
        let mut rect = r.rect(O.filter);
        if mode == "bad_rect" || mode == "ignored_callback" {
            rect.left = -1;
        }
        r.set_rect(O.input, rect);
        r.set_rect(O.output, rect);
        let first_plane = if mode == "single_plane" && planes > 1 { 1 } else { 0 };
        let last_plane = if mode == "single_plane" { first_plane } else { planes - 1 };
        r.set16(O.input_low, first_plane as i16);
        r.set16(O.input_low + 2, last_plane as i16);
        if mode == "mask_with_pixels" {
            // A filter can populate this even when the host advertises no native mask.
            r.set_rect(O.mask, r.rect(O.filter));
        }
        r.set16(O.output_low, first_plane as i16);
        r.set16(O.output_low + 2, last_plane as i16);
        if mode == "ignored_callback" {
            let at = if cfg!(target_pointer_width = "64") { 0x118 } else { 0xe0 };
            let f = usize::from_le_bytes(r.get(at));
            // SAFETY: the worker supplies this callback with the independently established ABI.
            let f: extern "C" fn() -> i16 = unsafe { std::mem::transmute(f) };
            let _ = f();
            r.set_rect(O.input, Rect::default());
            r.set_rect(O.output, Rect::default());
        }
    } else if selector == 4 {
        if mode == "mask_request" {
            // With no native mask, a mask-only rectangle must not drive Continue.
            unsafe { *result = -32123 };
            return;
        }
        let rect = r.rect(O.output);
        let Some((_, sample_bytes, _)) = image_layout(r) else {
            unsafe { *result = -1 };
            return;
        };
        let first_plane = usize::try_from(r.i16(O.input_low)).ok();
        let last_plane = usize::try_from(r.i16(O.input_low + 2)).ok();
        let Some(planes) = first_plane.zip(last_plane).and_then(|(first, last)| last.checked_sub(first)).and_then(|count| count.checked_add(1)) else {
            unsafe { *result = -1 };
            return;
        };
        let Some(len) = usize::try_from((rect.right - rect.left).max(0))
            .ok()
            .and_then(|width| usize::try_from((rect.bottom - rect.top).max(0)).ok().and_then(|height| width.checked_mul(height)))
            .and_then(|pixels| pixels.checked_mul(planes))
            .and_then(|samples| samples.checked_mul(sample_bytes))
        else {
            unsafe { *result = -1 };
            return;
        };
        let source = usize::from_le_bytes(r.get(O.input_data)) as *const u8;
        let dest = usize::from_le_bytes(r.get(O.output_data)) as *mut u8;
        // SAFETY: worker callback has supplied len bytes of input and output for this rectangle.
        unsafe {
            let input = std::slice::from_raw_parts(source, len);
            let output = std::slice::from_raw_parts_mut(dest, len);
            if mode == "identity" {
                output.copy_from_slice(input);
            } else if sample_bytes == 1 {
                for (a, b) in input.iter().zip(output.iter_mut()) {
                    *b = 255 - *a;
                }
            } else {
                for (a, b) in input.as_chunks::<2>().0.iter().zip(output.as_chunks_mut::<2>().0.iter_mut()) {
                    let value = u16::from_le_bytes(*a);
                    let inverted = 32768u16.saturating_sub(value);
                    b.copy_from_slice(&inverted.to_le_bytes());
                }
            }
        }
        r.set_rect(O.input, Rect::default());
        r.set_rect(O.output, Rect::default());
    }
}
