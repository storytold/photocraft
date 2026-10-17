//! Windows pointer hook for tablet data dropped by winit (such as the pen barrel button).

#![allow(unsafe_code)]

use std::sync::atomic::{AtomicBool, Ordering};

static BARREL_DOWN: AtomicBool = AtomicBool::new(false);
static HOOK_INSTALLED: AtomicBool = AtomicBool::new(false);

#[repr(C)]
struct POINT {
    x: i32,
    y: i32,
}

#[repr(C)]
struct MSG {
    hwnd: *mut std::ffi::c_void,
    message: u32,
    w_param: usize,
    l_param: isize,
    time: u32,
    pt: POINT,
}

#[repr(C)]
struct POINTER_INFO {
    pointer_type: u32,
    pointer_id: u32,
    frame_id: u32,
    pointer_flags: u32,
    source_device: *mut std::ffi::c_void,
    hwnd_target: *mut std::ffi::c_void,
    pt_pixel_location: POINT,
    pt_himetric_location: POINT,
    pt_pixel_location_raw: POINT,
    pt_himetric_location_raw: POINT,
    dw_time: u32,
    history_count: u32,
    input_data: i32,
    dw_key_states: u32,
    performance_count: u64,
    button_change_type: i32,
}

#[repr(C)]
struct POINTER_PEN_INFO {
    pointer_info: POINTER_INFO,
    pen_flags: u32,
    pen_mask: u32,
    pressure: u32,
    rotation: u32,
    tilt_x: i32,
    tilt_y: i32,
}

#[link(name = "user32")]
unsafe extern "system" {
    fn SetWindowsHookExW(
        id_hook: i32,
        lpfn: unsafe extern "system" fn(i32, usize, isize) -> isize,
        h_mod: *mut std::ffi::c_void,
        dw_thread_id: u32,
    ) -> *mut std::ffi::c_void;
    fn CallNextHookEx(hhk: *mut std::ffi::c_void, n_code: i32, w_param: usize, l_param: isize) -> isize;
    fn GetCurrentThreadId() -> u32;
    fn GetPointerPenInfo(pointer_id: u32, pen_info: *mut POINTER_PEN_INFO) -> i32;
}

const WH_GETMESSAGE: i32 = 3;
const WM_POINTERUPDATE: u32 = 0x0245;
const WM_POINTERDOWN: u32 = 0x0246;
const WM_POINTERUP: u32 = 0x0247;
const WM_POINTERLEAVE: u32 = 0x024A;
const PEN_FLAG_BARREL: u32 = 0x00000001;

unsafe extern "system" fn get_msg_proc(code: i32, w_param: usize, l_param: isize) -> isize {
    if code >= 0 && l_param != 0 {
        let msg = unsafe { &*(l_param as *const MSG) };
        match msg.message {
            WM_POINTERDOWN | WM_POINTERUPDATE => {
                let pointer_id = (msg.w_param & 0xFFFF) as u32;
                let mut pen_info: POINTER_PEN_INFO = unsafe { std::mem::zeroed() };
                let is_barrel = if unsafe { GetPointerPenInfo(pointer_id, &mut pen_info) } != 0 {
                    (pen_info.pen_flags & PEN_FLAG_BARREL) != 0 || ((msg.w_param >> 16) & 0x0024) != 0
                } else {
                    ((msg.w_param >> 16) & 0x0024) != 0
                };
                BARREL_DOWN.store(is_barrel, Ordering::Relaxed);
            }
            WM_POINTERUP | WM_POINTERLEAVE => {
                BARREL_DOWN.store(false, Ordering::Relaxed);
            }
            _ => {}
        }
    }
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, w_param, l_param) }
}

/// Installs the thread-local Windows pointer hook on the UI thread.
pub fn install() {
    if HOOK_INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }
    unsafe {
        let thread_id = GetCurrentThreadId();
        let _ = SetWindowsHookExW(WH_GETMESSAGE, get_msg_proc, std::ptr::null_mut(), thread_id);
    }
}

/// Returns true if the stylus barrel button is physically pressed.
pub fn is_barrel_button_down() -> bool {
    BARREL_DOWN.load(Ordering::Relaxed)
}
