//! #11934: with Normal Brush Tip cursors the Color Replacement cursor always draws the centre +,
//! its sampling hotspot, whatever Show Crosshair in Brush Tip says. Other brush tools follow the
//! preference, and Clone Stamp / Healing Brush keep their Alt behaviour.

use super::*;

#[test]
fn color_replacement_always_shows_its_centre() {
    for show_crosshair in [false, true] {
        for option in [false, true] {
            assert!(brush_tip_centre(Tool::ColorReplacement, option, show_crosshair), "crosshair pref {show_crosshair}, Alt {option}");
        }
    }
}

#[test]
fn other_brush_tools_follow_the_preference() {
    for tool in [Tool::Brush, Tool::Eraser, Tool::PatternStamp, Tool::Dodge, Tool::Pencil, Tool::Smudge] {
        assert!(!brush_tip_centre(tool, false, false), "{tool:?} off");
        assert!(brush_tip_centre(tool, false, true), "{tool:?} on");
    }
    // The Background Eraser keeps its hotspot, as before.
    assert!(brush_tip_centre(Tool::BackgroundEraser, false, false));
}

#[test]
fn clone_stamp_and_healing_keep_their_alt_behaviour() {
    for tool in [Tool::CloneStamp, Tool::Healing] {
        assert!(!brush_tip_centre(tool, false, false), "{tool:?}");
        assert!(brush_tip_centre(tool, true, false), "{tool:?} with Alt");
        assert!(brush_tip_centre(tool, false, true), "{tool:?} with the preference");
    }
}
