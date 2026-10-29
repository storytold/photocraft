#![no_main]
//! Load arbitrary text as `preferences.json` (read from the settings folder at start-up, with
//! the presets it holds): it must fail or load, never panic. What loads is saved and loaded in a
//! fresh session, which must take every value and save the same text: preferences we write have
//! to come back unchanged.

use libfuzzer_sys::fuzz_target;
use photocraft_engine::Session;
use serde_json::Value;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else { return };
    // An additional plug-ins folder would make loading read a directory the input names and
    // install the modules in it into the process-wide registry, so it stays off.
    let text = match serde_json::from_str::<Value>(text) {
        Ok(mut v) => {
            if let Some(p) = v.get_mut("plugIns").and_then(Value::as_object_mut) {
                p.insert("useAdditionalPluginsFolder".into(), Value::Bool(false));
            }
            v.to_string()
        }
        Err(_) => text.to_string(),
    };
    let mut s = Session::new();
    if s.load_prefs_json(&text).is_err() {
        return;
    }
    let saved = s.prefs_to_json();
    let mut fresh = Session::new();
    match fresh.load_prefs_json(&saved) {
        Ok(invalid) => assert!(invalid.is_empty(), "preferences we wrote have invalid values: {:?}", invalid.iter().map(|p| &p.keys).collect::<Vec<_>>()),
        Err(e) => panic!("preferences we wrote do not load: {e}"),
    }
    assert_eq!(fresh.prefs_to_json(), saved, "preferences change across a save and load");
});
