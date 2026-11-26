//! The script host PhotoCraft ships: the app starts it, once per run, with `--script-host`, from
//! next to its own executable. Only this process counts its allocations (to cap its heap at the
//! run's memory limit); the app keeps the system allocator as it is.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
// A console window is never shown for it: the app talks to it through its standard streams.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

/// The heap the run's memory limit caps.
#[global_allocator]
static HEAP: photocraft_script_js::isolate::HostAllocator = photocraft_script_js::isolate::host_allocator();

fn main() {
    if photocraft_script_js::isolate::script_host_requested() {
        std::process::exit(photocraft_script_js::isolate::serve(&HEAP));
    }
    eprintln!("usage: photocraft-script-host {}", photocraft_script_js::isolate::SCRIPT_HOST_FLAG);
    std::process::exit(2);
}
