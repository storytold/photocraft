#![no_main]
//! Parse arbitrary bytes as a plug-in manifest (the JSON a module's `pc_manifest` returns): it
//! must fail or validate, never panic. A manifest that validates must survive a JSON round
//! trip unchanged, and resolving default parameters against it must not panic.

use libfuzzer_sys::fuzz_target;
use photocraft_plugins::manifest::Manifest;
use serde_json::json;

fuzz_target!(|data: &[u8]| {
    let Ok(man) = Manifest::parse(data) else { return };
    let _ = man.params_notation();
    if let Ok(mut params) = man.resolve_params(&json!({})) {
        man.scale_for_preview(&mut params, 4);
        let _ = man.preview_factor(&params, 4);
    }
    let text = serde_json::to_vec(&man).expect("a manifest serialises");
    let back = Manifest::parse(&text).expect("a manifest we serialise parses again");
    assert_eq!(back, man, "a manifest changes across a JSON round trip");
});
