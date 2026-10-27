//! The process-local native FFI boundary. Never linked into PhotoCraft.
use crate::{
    memory::*,
    pixels::{self, Rect},
    record::{O, Record},
    wire::Request,
};
use std::{
    cell::Cell,
    ffi::{CString, c_void},
    ptr,
};
use windows_sys::Win32::{
    Foundation::{FreeLibrary, GetLastError},
    System::{
        Diagnostics::Debug::{GetErrorMode, SEM_FAILCRITICALERRORS, SEM_NOGPFAULTERRORBOX, SEM_NOOPENFILEERRORBOX, SetErrorMode},
        LibraryLoader::{GetProcAddress, LOAD_WITH_ALTERED_SEARCH_PATH, LoadLibraryExW},
    },
    UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WS_OVERLAPPEDWINDOW},
};

thread_local! { static ACTIVE:Cell<*mut Host>=const{Cell::new(ptr::null_mut())}; }
pub(crate) fn trace(message: std::fmt::Arguments<'_>) {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *ENABLED.get_or_init(|| std::env::var_os("PHOTOCRAFT_8BF_TRACE").is_some()) {
        eprintln!("PhotoCraft 8BF: {message}");
    }
}
fn with_host<R>(fallback: R, f: impl FnOnce(&mut Host) -> R) -> R {
    ACTIVE.with(|slot| {
        let p = slot.get();
        if p.is_null() {
            fallback
        } else {
            // SAFETY: run installs one boxed Host for this thread until all DLL callbacks return.
            // No reference to that Host is held across calls into the plug-in.
            unsafe { f(&mut *p) }
        }
    })
}
struct Host {
    record: Record,
    image: pixels::Image,
    input: Vec<u8>,
    output: Vec<u8>,
    input_tile: Vec<u8>,
    output_tile: Vec<u8>,
    pending: Option<(Rect, i16, i16)>,
    error: Option<String>,
}
impl Host {
    fn advance(&mut self) -> Result<(), String> {
        // The worker advertises no native selection mask: the safe host applies selection
        // coverage after filtering. Some filters still populate maskRect unconditionally.
        // Like independent legacy hosts, ignore that unused rectangle; rejecting it leaves
        // input/output pointers unavailable to filters that continue after the callback.
        self.flush()?;
        let ir = self.record.rect(O.input);
        let or = self.record.rect(O.output);
        self.record.set_ptr(O.input_data, 0);
        self.record.set_ptr(O.output_data, 0);
        if !ir.empty() {
            let lo = self.record.i16(O.input_low);
            let hi = self.record.i16(O.input_low + 2);
            self.input_tile = pixels::pack(&self.input, self.image, ir, lo, hi)?;
            self.record.set32(O.input_stride, (ir.right - ir.left) * i32::from(hi - lo + 1) * i32::from(self.image.depth / 8));
            self.record.set_ptr(O.input_data, self.input_tile.as_mut_ptr() as usize);
        }
        if !or.empty() {
            let lo = self.record.i16(O.output_low);
            let hi = self.record.i16(O.output_low + 2);
            self.output_tile = pixels::pack(&self.output, self.image, or, lo, hi)?;
            self.record.set32(O.output_stride, (or.right - or.left) * i32::from(hi - lo + 1) * i32::from(self.image.depth / 8));
            self.record.set_ptr(O.output_data, self.output_tile.as_mut_ptr() as usize);
            self.pending = Some((or, lo, hi));
        }
        Ok(())
    }
    fn flush(&mut self) -> Result<(), String> {
        if let Some((r, lo, hi)) = self.pending.take() {
            pixels::unpack(&mut self.output, self.image, r, lo, hi, &self.output_tile)?;
        }
        Ok(())
    }
}
extern "C" fn abort() -> u8 {
    0
}
extern "C" fn progress(_done: i32, _total: i32) {}
extern "C" fn advance() -> i16 {
    with_host(-1, |h| match h.advance() {
        Ok(()) => 0,
        Err(e) => {
            h.error = Some(e);
            -1
        }
    })
}
#[repr(C)]
struct Suite<const N: usize> {
    version: i16,
    count: i16,
    functions: [usize; N],
}

#[cfg(target_pointer_width = "32")]
const EXTRA: (usize, usize, usize, usize, usize) = (0xc0, 0xd0, 0xe0, 0x17c, 0xcc);
#[cfg(target_pointer_width = "64")]
const EXTRA: (usize, usize, usize, usize, usize) = (0xe0, 0x100, 0x118, 0x1e0, 0xf8);

pub fn run(request: &Request, pixels: Vec<u8>) -> Result<Vec<u8>, String> {
    // SAFETY: this changes only the disposable worker's error policy, before any plug-in
    // code or processing threads start. Preserve inherited flags. An unhandled native
    // fault must terminate the worker, not open Windows/Wine debugger UI and stall IPC.
    unsafe { SetErrorMode(GetErrorMode() | SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX | SEM_NOOPENFILEERRORBOX) };
    let expected = request.pixel_len().map_err(|e| e.to_string())?;
    if expected != pixels.len() {
        return Err("8BF input buffer does not match its declared format and dimensions".into());
    }
    let path: Vec<u16> = request.path.encode_utf16().chain(Some(0)).collect();
    if request.path.contains('\0') {
        return Err("invalid plug-in path".into());
    }
    let symbol = CString::new(request.entry.as_str()).map_err(|_| "invalid entry point")?;
    // SAFETY: Windows owns the module; path is NUL-terminated and remains alive for the call.
    trace(format_args!("loading {}", request.path));
    let module = unsafe { LoadLibraryExW(path.as_ptr(), ptr::null_mut(), LOAD_WITH_ALTERED_SEARCH_PATH) };
    if module.is_null() {
        return Err(format!("Cannot load 8BF (Windows error {}). Check architecture and companion DLLs.", unsafe { GetLastError() }));
    }
    // SAFETY: handle is loaded and symbol is NUL-terminated.
    let proc = unsafe { GetProcAddress(module, symbol.as_ptr().cast()) };
    let Some(proc) = proc else {
        unsafe { FreeLibrary(module) };
        return Err(format!("8BF entry point {} is missing", request.entry));
    };
    trace(format_args!("loaded; entry {} resolved", request.entry));
    let class: Vec<u16> = "STATIC".encode_utf16().chain(Some(0)).collect();
    let title: Vec<u16> = "PhotoCraft 8BF Filter".encode_utf16().chain(Some(0)).collect();
    // SAFETY: stock STATIC class needs no custom window procedure; window lives until filter returns.
    let hwnd = unsafe {
        CreateWindowExW(0, class.as_ptr(), title.as_ptr(), WS_OVERLAPPEDWINDOW, 0, 0, 640, 480, ptr::null_mut(), ptr::null_mut(), ptr::null_mut(), ptr::null())
    };
    let mut platform = [hwnd as usize, 0, 0, 0];
    let buffers = Suite {
        version: 2,
        count: 6,
        functions: [
            allocate_buffer as *const () as usize,
            lock_buffer as *const () as usize,
            unlock_buffer as *const () as usize,
            free_buffer as *const () as usize,
            buffer_space as *const () as usize,
            // Nullable legacy extension slot. No unsupported callback is advertised.
            0,
        ],
    };
    let handles = Suite {
        version: 1,
        count: 8,
        functions: [
            new_handle as *const () as usize,
            dispose_handle as *const () as usize,
            handle_size as *const () as usize,
            resize_handle as *const () as usize,
            lock_handle as *const () as usize,
            unlock_handle as *const () as usize,
            recover_space as *const () as usize,
            dispose_regular_handle as *const () as usize,
        ],
    };
    let properties = Suite { version: 1, count: 2, functions: [crate::properties::get as *const () as usize, crate::properties::set as *const () as usize] };
    let mut h = Box::new(Host {
        record: Record::new(),
        image: pixels::Image { width: request.width, height: request.height, channels: request.channels, depth: request.depth },
        output: pixels.clone(),
        input: pixels,
        input_tile: Vec::new(),
        output_tile: Vec::new(),
        pending: None,
        error: None,
    });
    h.record.set_ptr(4, abort as *const () as usize);
    h.record.set_ptr(4 + size_of::<usize>(), progress as *const () as usize);
    h.record.set16(O.size, request.height as i16);
    h.record.set16(O.size + 2, request.width as i16);
    h.record.set16(O.planes, i16::from(request.channels));
    h.record.set_rect(O.filter, Rect { top: 0, left: 0, bottom: request.height as i32, right: request.width as i32 });
    h.record.set32(O.input - 8, 256 * 1024 * 1024);
    h.record.set32(O.signature, i32::from_be_bytes(*b"PhCr"));
    let mode = match (request.channels, request.depth) {
        (1, 8) => 1,
        (3, 8) => 3,
        (1, 16) => 10,
        _ => 11,
    };
    h.record.set16(O.mode, mode);
    h.record.set_ptr(O.platform, platform.as_mut_ptr() as usize);
    h.record.set_ptr(EXTRA.0, &buffers as *const _ as usize);
    h.record.set_ptr(EXTRA.1, &handles as *const _ as usize);
    h.record.set_ptr(EXTRA.2, advance as *const () as usize);
    h.record.set32(EXTRA.3, i32::from(request.depth));
    h.record.set_ptr(EXTRA.4, crate::preview::display as *const () as usize);
    h.record.set_ptr(O.properties, &properties as *const _ as usize);
    h.record.set_ptr(O.parameters, 0);
    h.record.set_rect(O.mask, Rect::default());
    ACTIVE.with(|slot| slot.set(&mut *h));
    type Entry = unsafe extern "C" fn(i16, *mut c_void, *mut isize, *mut i16);
    // SAFETY: metadata declares the filter entrypoint; its cdecl ABI was independently verified.
    let entry: Entry = unsafe { std::mem::transmute(proc) };
    let record = h.record.ptr();
    let mut data = 0isize;
    let start_returned_success = Cell::new(false);
    let mut invoke = |selector: i16| -> Result<(), String> {
        let mut result = 0i16;
        trace(format_args!("calling selector {selector}"));
        // SAFETY: record, callback tables, platform and data slots outlive this DLL call.
        unsafe { entry(selector, record, &mut data, &mut result) };
        trace(format_args!(
            "selector {selector} returned {result}; data={data:#x}, parameters={:#x}",
            with_host(0, |h| usize::from_le_bytes(h.record.get(O.parameters)))
        ));
        if selector == 3 {
            start_returned_success.set(result == 0);
        }
        if let Some(e) = with_host(None, |h| h.error.take()) {
            return Err(e);
        }
        if result == 0 {
            Ok(())
        } else if result == -128 {
            Err("cancelled".into())
        } else {
            Err(format!("8BF selector {selector} returned {result}"))
        }
    };
    let result: Result<(), String> = (|| {
        if request.show_ui {
            invoke(1)?;
        }
        invoke(2)?;
        let start = invoke(3);
        if !start_returned_success.get() {
            return start;
        }
        // A successful DLL Start still requires Finish when a host callback failed.
        // Keep the callback error as the primary result, even if cleanup also fails.
        let processing: Result<(), String> = start.and_then(|()| {
            for _ in 0..1_000_000 {
                if h.record.rect(O.input).empty() && h.record.rect(O.output).empty() {
                    h.flush()?;
                    return Ok(());
                }
                h.advance()?;
                invoke(4)?;
                if let Some(e) = h.error.take() {
                    return Err(e);
                }
            }
            Err("8BF exceeded its iteration budget".into())
        });
        let finish = invoke(5);
        processing?;
        finish?;
        h.flush()?;
        Ok(())
    })();
    // SAFETY: no callbacks execute after the selector sequence, all DLL-owned code has returned.
    unsafe {
        if !hwnd.is_null() {
            DestroyWindow(hwnd);
        }
        FreeLibrary(module);
    }
    ACTIVE.with(|slot| slot.set(ptr::null_mut()));
    result?;
    Ok(std::mem::take(&mut h.output))
}
