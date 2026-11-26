//! A script host built with this crate, for its own process tests: the executable started once
//! per run with `--script-host` (`isolate`). The app ships its own, `photocraft-script-host`, the
//! same few lines (`apps/photocraft/src/bin`); this one has another name, so a workspace build
//! makes two executables, not one written twice. Only it counts its allocations, to cap its heap at the run's
//! memory limit.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

/// The heap the run's memory limit caps (`isolate::limits`).
#[cfg(not(target_arch = "wasm32"))]
#[global_allocator]
static HEAP: photocraft_script_js::isolate::HostAllocator = photocraft_script_js::isolate::host_allocator();

fn main() {
    #[cfg(not(target_arch = "wasm32"))]
    {
        if photocraft_script_js::isolate::script_host_requested() {
            std::process::exit(photocraft_script_js::isolate::serve(&HEAP));
        }
        eprintln!("usage: script-js-test-host {}", photocraft_script_js::isolate::SCRIPT_HOST_FLAG);
        std::process::exit(2);
    }
}
