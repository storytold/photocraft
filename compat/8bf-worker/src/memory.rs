//! Process-wide memory services: plug-in processing threads may allocate concurrently.
//! The table lock protects ownership and resizing; callers must not dispose/resize a handle
//! while another plug-in thread uses its locked bytes, as required by the native ABI.
use std::{
    collections::HashMap,
    ptr,
    sync::{Mutex, OnceLock},
};

struct Handle {
    bytes: Vec<u8>,
    // Pointer-sized, aligned slot with a stable address. A usize keeps the Rust ownership table
    // Send; its contents are exposed as a native pointer only at the FFI boundary.
    pointer: Box<usize>,
    logical_size: i32,
}
#[derive(Default)]
struct Memory {
    handles: HashMap<usize, Handle>,
    // Retain tiny pointer slots, but release their pixels, until this one-job worker exits.
    // This makes duplicate disposal harmless without confusing a reused foreign address.
    retired_handles: HashMap<usize, Box<usize>>,
    buffers: HashMap<usize, Vec<u8>>,
}
static MEMORY: OnceLock<Mutex<Memory>> = OnceLock::new();
fn with_memory<R>(_fallback: R, f: impl FnOnce(&mut Memory) -> R) -> R {
    let mut memory = MEMORY.get_or_init(|| Mutex::new(Memory::default())).lock().unwrap_or_else(|e| e.into_inner());
    f(&mut memory)
}

pub extern "C" fn new_handle(size: i32) -> *mut *mut u8 {
    crate::native::trace(format_args!("new_handle size={size}"));
    if size < 0 || size as usize > crate::wire::MAX_PIXELS {
        return ptr::null_mut();
    }
    with_memory(ptr::null_mut(), |h| {
        let mut bytes = Vec::new();
        if bytes.try_reserve_exact((size as usize).max(1)).is_err() {
            return ptr::null_mut();
        }
        bytes.resize((size as usize).max(1), 0);
        let mut pointer = Box::new(bytes.as_mut_ptr() as usize);
        let key = (&mut *pointer as *mut usize).cast::<*mut u8>();
        h.retired_handles.remove(&(key as usize));
        h.handles.insert(key as usize, Handle { bytes, pointer, logical_size: size });
        key
    })
}
pub extern "C" fn dispose_handle(handle: *mut *mut u8) {
    with_memory((), |h| {
        if let Some(mut owned) = h.handles.remove(&(handle as usize)) {
            *owned.pointer = 0;
            h.retired_handles.insert(handle as usize, owned.pointer);
        }
    });
}
pub extern "C" fn handle_size(handle: *mut *mut u8) -> i32 {
    with_memory(0, |h| h.handles.get(&(handle as usize)).map_or(0, |a| a.logical_size))
}
pub extern "C" fn resize_handle(handle: *mut *mut u8, size: i32) -> i16 {
    if size < 0 || size as usize > crate::wire::MAX_PIXELS {
        return -1;
    }
    with_memory(-1, |h| {
        let Some(a) = h.handles.get_mut(&(handle as usize)) else {
            return -1;
        };
        let n = (size as usize).max(1);
        if n > a.bytes.len() && a.bytes.try_reserve_exact(n - a.bytes.len()).is_err() {
            return -1;
        }
        a.bytes.resize(n, 0);
        a.logical_size = size;
        *a.pointer = a.bytes.as_mut_ptr() as usize;
        0
    })
}
pub extern "C" fn lock_handle(handle: *mut *mut u8, _move_high: u8) -> *mut u8 {
    crate::native::trace(format_args!("lock_handle handle={handle:p}"));
    with_memory(ptr::null_mut(), |h| h.handles.get_mut(&(handle as usize)).map_or(ptr::null_mut(), |a| a.bytes.as_mut_ptr()))
}
pub extern "C" fn unlock_handle(_handle: *mut *mut u8) {}
/// # Safety
/// `out` must be null or a writable native pointer-sized output slot.
pub unsafe extern "C" fn allocate_buffer(size: i32, out: *mut usize) -> i16 {
    crate::native::trace(format_args!("allocate_buffer size={size}"));
    if out.is_null() || size < 0 || size as usize > crate::wire::MAX_PIXELS {
        return -1;
    }
    with_memory(-1, |h| {
        let mut b = Vec::new();
        if b.try_reserve_exact((size as usize).max(1)).is_err() {
            return -1;
        }
        b.resize((size as usize).max(1), 0);
        let key = b.as_mut_ptr() as usize;
        h.buffers.insert(key, b);
        // SAFETY: the DLL supplies a writable output slot under the callback contract.
        unsafe { out.write_unaligned(key) };
        0
    })
}
pub extern "C" fn lock_buffer(key: usize, _move_high: u8) -> *mut u8 {
    with_memory(ptr::null_mut(), |h| h.buffers.get_mut(&key).map_or(ptr::null_mut(), |b| b.as_mut_ptr()))
}
pub extern "C" fn unlock_buffer(_key: usize) {}
pub extern "C" fn free_buffer(key: usize) {
    with_memory((), |h| {
        h.buffers.remove(&key);
    });
}
pub extern "C" fn buffer_space() -> i32 {
    256 * 1024 * 1024
}

/// This worker retains no unlocked handle cache to purge. Live allocations remain owned
/// by their callers; a recovery hint must never invalidate them.
pub extern "C" fn recover_space(_size: i32) {}

/// Release a legacy handle, including a Windows global handle supplied by the DLL.
/// Foreign wrappers remain owned by the caller; only their underlying allocation is freed.
pub extern "C" fn dispose_regular_handle(handle: *mut *mut u8) {
    if handle.is_null()
        || with_memory(false, |h| {
            if let Some(mut owned) = h.handles.remove(&(handle as usize)) {
                *owned.pointer = 0;
                h.retired_handles.insert(handle as usize, owned.pointer);
                true
            } else {
                h.retired_handles.contains_key(&(handle as usize))
            }
        })
    {
        return;
    }
    use windows_sys::Win32::{
        Foundation::GlobalFree,
        System::{
            Diagnostics::Debug::ReadProcessMemory,
            Memory::{GlobalFlags, GlobalSize},
            Threading::GetCurrentProcess,
        },
    };
    const INVALID_GLOBAL: u32 = 0x8000; // Win32 GMEM_INVALID_HANDLE, not an 8BF constant.
    // SAFETY: Windows validates the opaque allocation through GlobalFlags and GlobalSize
    // before freeing it. Flags alone can accept non-heap pointers under Wine. Only this
    // disposable worker receives the callback; the DLL relinquishes the allocation.
    unsafe {
        let direct = handle.cast();
        if GlobalFlags(direct) != INVALID_GLOBAL && GlobalSize(direct) > 0 {
            GlobalFree(direct);
            return;
        }
        // Use the OS read API instead of dereferencing an untrusted foreign wrapper.
        let mut indirect = 0usize;
        let mut read = 0;
        if ReadProcessMemory(GetCurrentProcess(), handle.cast(), (&mut indirect as *mut usize).cast(), size_of::<usize>(), &mut read) != 0
            && read == size_of::<usize>()
            && indirect != 0
            && GlobalFlags(indirect as _) != INVALID_GLOBAL
            && GlobalSize(indirect as _) > 0
        {
            GlobalFree(indirect as _);
        }
    }
}
