//! The script host on its own, for tests and headless tools. The app serves the same runs from
//! its own executable (`photocraft --script-host`).

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

fn main() {
    #[cfg(not(target_arch = "wasm32"))]
    {
        if photocraft_script_js::isolate::script_host_requested() {
            std::process::exit(photocraft_script_js::isolate::serve());
        }
        eprintln!("usage: photocraft-script-host {}", photocraft_script_js::isolate::SCRIPT_HOST_FLAG);
        std::process::exit(2);
    }
}
