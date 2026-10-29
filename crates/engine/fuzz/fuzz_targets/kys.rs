#![no_main]
//! Import arbitrary text as a Photoshop keyboard shortcut set (.kys) through Edit › Keyboard
//! Shortcuts, as the app does: it must fail or apply, never panic. What applies is saved as a
//! .kys again, and that file must import in a fresh session: a set we write has to open.

use libfuzzer_sys::fuzz_target;
use photocraft_engine::Session;
use serde_json::json;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else { return };
    let mut s = Session::new();
    let Ok(reply) = s.execute("edit.keyboardShortcuts", json!({ "importKys": text, "exportKys": true })) else { return };
    let Some(saved) = reply.get("kys").and_then(|v| v.as_str()) else {
        panic!("Keyboard Shortcuts replied without the exported set: {reply}");
    };
    let mut fresh = Session::new();
    if let Err(e) = fresh.execute("edit.keyboardShortcuts", json!({ "importKys": saved })) {
        panic!("a .kys we wrote does not import: {e}");
    }
});
