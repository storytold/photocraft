//! Original conformance DLL for the worker's eight legacy handle callbacks.
//! Numeric table positions come from our independent ABI observations, not SDK declarations.
use std::ffi::c_void;

type Handle = *mut *mut u8;
type New = extern "C" fn(i32) -> Handle;
type Dispose = extern "C" fn(Handle);
type Size = extern "C" fn(Handle) -> i32;
type Resize = extern "C" fn(Handle, i32) -> i16;
type Lock = extern "C" fn(Handle, u8) -> *mut u8;
type Unlock = extern "C" fn(Handle);
type Recover = extern "C" fn(i32);

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GlobalAlloc(flags: u32, size: usize) -> *mut c_void;
    fn GlobalFlags(handle: *mut c_void) -> u32;
    fn GlobalFree(handle: *mut c_void) -> *mut c_void;
}

/// # Safety
/// The worker supplies a live record and a writable selector result.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn PluginMain(selector: i16, block: *mut c_void, _data: *mut isize, result: *mut i16) {
    // SAFETY: all memory accesses are confined to the worker's record/table or to allocations
    // requested from that same table. Disposed handle addresses are passed back as opaque IDs.
    unsafe {
        result.write(0);
        if selector == 5 {
            eprintln!("HANDLE_SERVICES_FINISH");
            eprintln!("CONFORMANCE_FINISH");
        }
        if selector != 3 {
            return;
        }
        let handle_at = if cfg!(target_pointer_width = "64") { 0x100 } else { 0xd0 };
        let table = block.cast::<u8>().add(handle_at).cast::<usize>().read_unaligned() as *const u8;
        if table.is_null() {
            result.write(-31901);
            return;
        }
        let version = table.cast::<i16>().read_unaligned();
        let count = table.add(2).cast::<i16>().read_unaligned();
        eprintln!("HANDLE_SERVICES version={version} count={count}");
        // This guard is essential: the old six-slot table must fail without an out-of-bounds
        // read, rather than accidentally calling adjacent stack data as a function pointer.
        if version != 1 || count < 8 {
            result.write(-31902);
            return;
        }
        let buffer_at = if cfg!(target_pointer_width = "64") { 0xe0 } else { 0xc0 };
        let buffers = block.cast::<u8>().add(buffer_at).cast::<usize>().read_unaligned() as *const u8;
        if buffers.is_null() || buffers.cast::<i16>().read_unaligned() != 2 || buffers.add(2).cast::<i16>().read_unaligned() < 6 {
            result.write(-31914);
            return;
        }
        // The legacy extension is nullable. Do not mistake five implemented functions
        // for a five-slot table; callers validate the full compatible table envelope.
        if buffers.add(size_of::<usize>()).cast::<usize>().add(5).read_unaligned() != 0 {
            result.write(-31915);
            return;
        }
        let functions = table.add(size_of::<usize>()).cast::<usize>();
        let new: New = std::mem::transmute(functions.read_unaligned());
        let dispose: Dispose = std::mem::transmute(functions.add(1).read_unaligned());
        let size: Size = std::mem::transmute(functions.add(2).read_unaligned());
        let resize: Resize = std::mem::transmute(functions.add(3).read_unaligned());
        let lock: Lock = std::mem::transmute(functions.add(4).read_unaligned());
        let unlock: Unlock = std::mem::transmute(functions.add(5).read_unaligned());
        let recover: Recover = std::mem::transmute(functions.add(6).read_unaligned());
        let regular_dispose: Dispose = std::mem::transmute(functions.add(7).read_unaligned());

        let handle = new(32);
        if handle.is_null() || size(handle) != 32 {
            result.write(-31903);
            return;
        }
        let bytes = lock(handle, 0);
        if bytes.is_null() || bytes != handle.read() {
            dispose(handle);
            result.write(-31904);
            return;
        }
        for i in 0..32 {
            bytes.add(i).write((i as u8).wrapping_mul(7));
        }
        unlock(handle);
        if resize(handle, 64) != 0 || size(handle) != 64 {
            dispose(handle);
            result.write(-31905);
            return;
        }
        let bytes = lock(handle, 1);
        if bytes.is_null() || bytes != handle.read() || (0..32).any(|i| bytes.add(i).read() != (i as u8).wrapping_mul(7)) {
            dispose(handle);
            result.write(-31906);
            return;
        }
        unlock(handle);
        recover(4096);
        if size(handle) != 64 {
            dispose(handle);
            result.write(-31907);
            return;
        }
        regular_dispose(handle);
        regular_dispose(handle);
        if size(handle) != 0 || !lock(handle, 0).is_null() {
            result.write(-31908);
            return;
        }
        let second = new(8);
        if second.is_null() || size(second) != 8 {
            result.write(-31909);
            return;
        }
        dispose(second);
        dispose(second);
        if size(second) != 0 || !lock(second, 0).is_null() {
            result.write(-31910);
            return;
        }
        let case = std::env::var("PHOTOCRAFT_8BF_HANDLE_CASE").unwrap_or_else(|_| "owned".into());
        if case == "direct" || case == "indirect" {
            let mut global = GlobalAlloc(0x42, 64); // GMEM_MOVEABLE | GMEM_ZEROINIT
            if global.is_null() || GlobalFlags(global) == 0x8000 {
                result.write(-31911);
                return;
            }
            let original = global;
            if case == "direct" {
                regular_dispose(global.cast());
            } else {
                // This wrapper belongs to the fixture's stack. Only its referenced Windows
                // allocation may be freed; the host must neither free nor modify the wrapper.
                regular_dispose((&mut global as *mut *mut c_void).cast());
            }
            if global != original || GlobalFlags(original) != 0x8000 {
                if GlobalFlags(original) != 0x8000 {
                    GlobalFree(original);
                }
                result.write(if case == "direct" { -31912 } else { -31913 });
                return;
            }
        }
        eprintln!("HANDLE_SERVICES_OK case={case} new,size,resize,lock,unlock,recover,dispose,regular_dispose");
    }
}
