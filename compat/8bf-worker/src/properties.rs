//! The independently observed watch-suspension property used around native previews.
//! Unverified property identifiers are deliberately not advertised.
use std::sync::atomic::{AtomicIsize, Ordering};

static WATCH: AtomicIsize = AtomicIsize::new(0);
const SIGNATURE: u32 = u32::from_be_bytes(*b"8BIM");
const WATCH_KEY: u32 = u32::from_be_bytes(*b"wtch");

/// # Safety
/// Non-null outputs must point to writable pointer-sized slots for this call.
pub unsafe extern "C" fn get(signature: u32, key: u32, index: i32, simple: *mut isize, complex: *mut usize) -> i32 {
    crate::native::trace(format_args!("property get {signature:08x}/{key:08x} index {index}, simple {simple:p}, complex {complex:p}"));
    // SAFETY: initialize the caller's optional output slots even on failure.
    unsafe {
        if !simple.is_null() {
            simple.write_unaligned(0);
        }
        if !complex.is_null() {
            complex.write_unaligned(0);
        }
    }
    if signature != SIGNATURE || key != WATCH_KEY || index != 0 || simple.is_null() {
        return -1;
    }
    // SAFETY: validated non-null output slot under the callback contract.
    unsafe {
        simple.write_unaligned(WATCH.load(Ordering::Relaxed));
    }
    0
}

pub extern "C" fn set(signature: u32, key: u32, index: i32, simple: isize, complex: usize) -> i32 {
    crate::native::trace(format_args!("property set {signature:08x}/{key:08x} index {index}, simple {simple}, complex {complex:x}"));
    if signature != SIGNATURE || key != WATCH_KEY || index != 0 || complex != 0 {
        return -1;
    }
    WATCH.store(simple, Ordering::Relaxed);
    0
}
