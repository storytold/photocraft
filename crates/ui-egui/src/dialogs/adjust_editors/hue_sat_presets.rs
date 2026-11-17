//! Hue/Saturation presets (#9245): starting settings for the editor's Preset menu, named for the
//! looks they give.
//!
//! These settings are PhotoCraft's own, each designed from its name: a saturation lift in three
//! strengths, a lift of one hue range for the two Boosts, a blue or warm brown Colorize for
//! Cyanotype and Sepia, and a faded, slightly darkened image with warm tones kept for Old Style.
//! None of them is taken or measured from another application's presets.

use photocraft_engine::commands::adjustment_from_params;
use serde_json::{Value, json};

/// The menu, Default first: `(id, label)`. The id is also the menu value.
pub const PRESETS: [(&str, &str); 9] = [
    ("default", "Default"),
    ("cyanotype", "Cyanotype"),
    ("increaseSaturation", "Increase Saturation"),
    ("increaseSaturationMore", "Increase Saturation More"),
    ("oldStyle", "Old Style"),
    ("redBoost", "Red Boost"),
    ("sepia", "Sepia"),
    ("strongSaturation", "Strong Saturation"),
    ("yellowBoost", "Yellow Boost"),
];

/// The settings preset `id` sets, replacing the editor's.
pub fn params(id: &str) -> Option<Value> {
    Some(match id {
        "default" => json!({}),
        "cyanotype" => json!({"colorize": true, "hue": 210, "saturation": 35, "lightness": 0}),
        "increaseSaturation" => json!({"saturation": 20}),
        "increaseSaturationMore" => json!({"saturation": 40}),
        "oldStyle" => json!({"saturation": -40, "lightness": -5, "reds": {"saturation": 15}, "yellows": {"saturation": 20}}),
        "redBoost" => json!({"reds": {"saturation": 40}}),
        "sepia" => json!({"colorize": true, "hue": 30, "saturation": 25, "lightness": 0}),
        "strongSaturation" => json!({"saturation": 60}),
        "yellowBoost" => json!({"yellows": {"saturation": 40}}),
        _ => return None,
    })
}

/// The preset `v` amounts to (the same adjustment), if any; otherwise the menu reads Custom.
pub fn matching(v: &Value) -> Option<&'static str> {
    let adj = adjustment_from_params("hueSaturation", v);
    PRESETS.iter().map(|(id, _)| *id).find(|id| params(id).is_some_and(|p| adjustment_from_params("hueSaturation", &p) == adj))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_is_its_own_distinct_adjustment() {
        for (id, _) in PRESETS {
            let p = params(id).unwrap_or_else(|| panic!("{id} has no settings"));
            assert_eq!(matching(&p), Some(id));
        }
        assert_eq!(params("custom"), None);
    }

    #[test]
    fn untouched_settings_are_default_and_an_edit_is_custom() {
        assert_eq!(matching(&json!({})), Some("default"));
        assert_eq!(matching(&json!({"hue": 0, "saturation": 0, "lightness": 0})), Some("default"));
        assert_eq!(matching(&json!({"saturation": 21})), None);
        let mut sepia = params("sepia").unwrap();
        sepia["hue"] = json!(31);
        assert_eq!(matching(&sepia), None);
    }
}
