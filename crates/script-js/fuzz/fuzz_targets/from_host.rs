//! Whatever bytes a script host writes, the app's frame reader and the checks after it return
//! an answer or an error, never a panic: `cargo +nightly fuzz run from_host` in `crates/script-js`.
#![no_main]

use libfuzzer_sys::fuzz_target;
use photocraft_script_js::isolate::protocol::{FromHost, MAX_FROM_HOST, check_window, read_frame};

fuzz_target!(|data: &[u8]| {
    let mut input = data;
    while let Ok(Some(msg)) = read_frame::<_, FromHost>(&mut input, MAX_FROM_HOST) {
        if let FromHost::Window { window: Some(w), .. } = msg {
            let _ = check_window(&w);
        }
    }
    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(data) {
        let _ = check_window(&v);
    }
});
