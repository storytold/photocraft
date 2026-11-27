//! The script Worker of PhotoCraft in the browser: one per run, started by the page
//! (`scripts.rs`). It holds the script engine and runs one script; the page checks every request
//! it makes and stops it at the time limit (`photocraft_script_js::isolate::worker`).

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

/// The run's memory limit caps this heap.
#[cfg(target_arch = "wasm32")]
#[global_allocator]
static HEAP: photocraft_script_js::isolate::HostAllocator = photocraft_script_js::isolate::host_allocator();

#[cfg(target_arch = "wasm32")]
fn main() {
    photocraft_script_js::isolate::serve_worker(&HEAP);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!("photocraft-script-worker only runs in the browser, started by photocraft-web");
}
