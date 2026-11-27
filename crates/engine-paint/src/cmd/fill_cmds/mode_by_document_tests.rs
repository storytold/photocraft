//! #14647: Edit › Fill and Edit › Stroke refuse the blend modes the reference application greys in their Mode lists
//! for the document's colour mode and depth (Grayscale 8-bit, Lab 8-bit, RGB 32-bit), and keep every mode elsewhere.

use super::*;
use serde_json::json;

/// A white 16×12 document of `mode` at `depth` bits.
fn session(mode: &str, depth: u32) -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 16, "height": 12, "mode": mode, "depth": depth, "background": "white"})).unwrap();
    s
}

fn labels(modes: &[BlendMode]) -> Vec<&'static str> {
    modes.iter().map(|m| m.label()).collect()
}

/// The measured cases: (colour mode, depth, the greyed modes).
const MEASURED: [(&str, u32, &[&str]); 5] = [
    ("grayscale", 8, &["Darker Color", "Lighter Color", "Hue", "Saturation", "Color", "Luminosity"]),
    ("lab", 8, &["Darken", "Color Burn", "Lighten", "Color Dodge", "Difference", "Exclusion", "Subtract", "Divide"]),
    (
        "rgb",
        32,
        &[
            "Color Burn",
            "Linear Burn",
            "Screen",
            "Color Dodge",
            "Overlay",
            "Soft Light",
            "Hard Light",
            "Vivid Light",
            "Linear Light",
            "Pin Light",
            "Hard Mix",
            "Exclusion",
        ],
    ),
    ("rgb", 16, &[]),
    ("cmyk", 8, &[]),
];

/// Cases not measured: nothing greyed, as before.
const NOT_MEASURED: [(&str, u32); 4] = [("grayscale", 16), ("lab", 16), ("cmyk", 16), ("rgb", 8)];

#[test]
fn the_greyed_modes_follow_the_measured_colour_modes_and_depths() {
    for (mode, depth, want) in MEASURED {
        let s = session(mode, depth);
        let doc = &s.active().unwrap().doc;
        assert_eq!(labels(greyed_modes(doc.mode, doc.depth)), want, "{mode} {depth}");
    }
    for (mode, depth) in NOT_MEASURED {
        let s = session(mode, depth);
        let doc = &s.active().unwrap().doc;
        assert!(greyed_modes(doc.mode, doc.depth).is_empty(), "{mode} {depth}");
    }
}

/// Runs `cmd` with `mode` and reports whether it ran; a refusal leaves no history step and no pixel change.
fn runs(s: &mut Session, cmd: &str, mode: BlendMode) -> bool {
    let params = match cmd {
        "edit.fill" => json!({"contents": "color", "color": "#336699", "mode": mode.key(), "opacity": 50}),
        _ => json!({"width": 2, "color": "#336699", "location": "inside", "mode": mode.key(), "opacity": 50}),
    };
    let before = s.active().unwrap().history.past_len();
    match s.execute(cmd, params) {
        Ok(_) => true,
        Err(e) => {
            assert!(matches!(&e, EngineError::BadParams { cmd: c, msg } if c == cmd && msg.contains(mode.label())), "{cmd} {}: {e}", mode.label());
            assert_eq!(s.active().unwrap().history.past_len(), before, "{cmd} {}: a refusal records no step", mode.label());
            false
        }
    }
}

#[test]
fn fill_and_stroke_refuse_a_greyed_mode_and_take_every_other() {
    for cmd in ["edit.fill", "edit.stroke"] {
        for (mode, depth, greyed) in MEASURED {
            for blend in BlendMode::LAYER_MODES {
                let mut s = session(mode, depth);
                s.execute("select.all", json!({})).unwrap();
                assert_eq!(runs(&mut s, cmd, blend), !greyed.contains(&blend.label()), "{cmd} in {mode} {depth}: {}", blend.label());
            }
        }
    }
}

#[test]
fn the_cases_not_measured_keep_every_mode() {
    for cmd in ["edit.fill", "edit.stroke"] {
        for (mode, depth) in NOT_MEASURED {
            for blend in BlendMode::LAYER_MODES {
                let mut s = session(mode, depth);
                s.execute("select.all", json!({})).unwrap();
                assert!(runs(&mut s, cmd, blend), "{cmd} in {mode} {depth}: {}", blend.label());
            }
        }
    }
}

/// The default mode, Normal, is never greyed, so a Fill without `mode` still runs everywhere.
#[test]
fn a_fill_without_a_mode_runs_in_every_measured_document() {
    for (mode, depth, _) in MEASURED {
        let mut s = session(mode, depth);
        s.execute("edit.fill", json!({"contents": "black"})).unwrap();
    }
}
