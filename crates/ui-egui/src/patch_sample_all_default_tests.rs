//! The Patch tool's Content-Aware Sample All Layers starts on (#12125): saved settings keep their
//! stored value, and settings saved without it load with it on.

use serde_json::json;

use crate::state::ToolOptions;

#[test]
fn saved_settings_keep_stored_patch_sample_all_layers_value() {
    let saved_off: ToolOptions = serde_json::from_value(json!({
        "sample_all": {
            "patch": false,
            "magic_wand": true
        }
    }))
    .unwrap();
    assert!(!saved_off.sample_all.patch);
    assert!(saved_off.sample_all.magic_wand);

    let missing_patch: ToolOptions = serde_json::from_value(json!({
        "sample_all": {
            "magic_wand": true
        }
    }))
    .unwrap();
    assert!(missing_patch.sample_all.patch);
    assert!(missing_patch.sample_all.magic_wand);
}
