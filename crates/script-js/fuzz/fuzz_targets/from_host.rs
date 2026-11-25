//! Whatever bytes a script host writes, the app's frame reader and the checks after it return
//! an answer or an error, never a panic: `cargo +nightly fuzz run from_host` in `crates/script-js`.
#![no_main]

use libfuzzer_sys::fuzz_target;
use photocraft_script_js::isolate::protocol::{DialogKind, FromHost, MAX_FROM_HOST, check_command, check_dialog_args, read_frame};

fuzz_target!(|data: &[u8]| {
    let mut input = data;
    while let Ok(Some(msg)) = read_frame::<_, FromHost>(&mut input, MAX_FROM_HOST) {
        match msg {
            FromHost::Dialog { kind, args } => drop(check_dialog_args(kind, &args)),
            FromHost::Execute { id, params } => drop(check_command(&id, &params)),
            _ => {}
        }
    }
    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(data) {
        let _ = check_dialog_args(DialogKind::Alert, &v);
    }
});
