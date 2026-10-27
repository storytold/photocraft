//! Original DLL checking independently observed property service calls on both ABIs.
#![allow(dead_code)]
#[path = "../src/pixels.rs"]
mod pixels;
#[path = "../src/record.rs"]
mod record;
use std::ffi::c_void;

type Get = unsafe extern "C" fn(u32, u32, i32, *mut isize, *mut usize) -> i32;
type Set = unsafe extern "C" fn(u32, u32, i32, isize, usize) -> i32;

/// # Safety
/// The worker owns the live record, callback table and writable result slot.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn PluginMain(selector: i16, block: *mut c_void, _data: *mut isize, result: *mut i16) {
    // SAFETY: all record and table accesses remain inside this test invocation.
    unsafe {
        *result = 0;
        if selector == 5 {
            eprintln!("CONFORMANCE_FINISH");
        }
        if selector != 3 {
            return;
        }
        let record = &mut *block.cast::<record::Record>();
        let at = if cfg!(target_pointer_width = "64") { 0x188 } else { 0x140 };
        let table = usize::from_le_bytes(record.get(at)) as *const u8;
        if table.is_null() {
            *result = -32001;
            return;
        }
        let offset = size_of::<usize>();
        let get: Get = std::mem::transmute(table.add(offset).cast::<usize>().read_unaligned());
        let set: Set = std::mem::transmute(table.add(offset * 2).cast::<usize>().read_unaligned());
        let sig = u32::from_be_bytes(*b"8BIM");
        let key = u32::from_be_bytes(*b"wtch");
        let mut simple = -99isize;
        let mut complex = usize::MAX;
        if get(sig, key, 0, &mut simple, &mut complex) != 0 || simple != 0 || complex != 0 {
            *result = -32002;
            return;
        }
        let value = isize::MAX / 2;
        if set(sig, key, 0, value, 0) != 0 || get(sig, key, 0, &mut simple, std::ptr::null_mut()) != 0 || simple != value {
            *result = -32003;
            return;
        }
        for (signature, property, index) in [(0, key, 0), (sig, 0, 0), (sig, key, 1)] {
            simple = -99;
            complex = usize::MAX;
            if get(signature, property, index, &mut simple, &mut complex) == 0 || simple != 0 || complex != 0 {
                *result = -32004;
                return;
            }
            if set(signature, property, index, 17, 0) == 0 {
                *result = -32005;
                return;
            }
        }
        if set(sig, key, 0, 3, 1) == 0 || get(sig, key, 0, std::ptr::null_mut(), &mut complex) == 0 {
            *result = -32006;
            return;
        }
        if get(sig, key, 0, &mut simple, std::ptr::null_mut()) != 0 || simple != value {
            *result = -32007;
        }
    }
}
