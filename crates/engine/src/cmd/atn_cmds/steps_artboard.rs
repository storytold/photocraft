//! Strict Artboard from Group forms whose settings the existing native model can apply.

use super::descriptors::{desc, get_num, get_obj, get_text, target_ref, text};
use super::fields::{Mapped, none, step};
use super::{D, Descriptor, Value, json};

const COMMAND: &str = "layer.new.artboardFromGroup";

/// Import the current-target form or the verified empty-guide settings form, keeping other forms whole.
pub(super) fn artboard_from(d: &Descriptor) -> Mapped {
    let unsupported = Mapped::Unsupported { harmless: false };
    if d.items == desc("null").with("null", target_ref("Lyr ")).items {
        return none(COMMAND);
    }
    let Some(board) = get_obj(d, "artboard") else { return unsupported };
    let Some(color) = get_obj(board, "Clr ") else { return unsupported };
    let Some(preset) = get_text(board, "artboardPresetName") else { return unsupported };
    let background = match board.get("artboardBackgroundType") {
        Some(D::Integer(1)) => "white",
        Some(D::Integer(2)) => "black",
        Some(D::Integer(3)) => "transparent",
        Some(D::Integer(4)) => "custom",
        _ => return unsupported,
    };
    let rgb: Option<Vec<f64>> = ["Rd  ", "Grn ", "Bl  "].iter().map(|key| get_num(color, key).map(|n| n / 255.0)).collect();
    let Some(rgb) = rgb else { return unsupported };
    let p = json!({"background": background, "color": rgb, "preset": preset, "guideIDs": [], "changeBackground": true});
    // Reconstruction checks every ordered key, nested class, value type and the evidenced flag.
    if artboard_to(COMMAND, &p).is_some_and(|expected| expected.items == d.items) { step(COMMAND, p) } else { unsupported }
}

/// Export only current-target steps or complete, representable empty-guide settings without quantizing RGB.
pub(super) fn artboard_to(_: &str, p: &Value) -> Option<Descriptor> {
    let p = p.as_object()?;
    let d = desc("null").with("null", target_ref("Lyr "));
    if p.is_empty() {
        return Some(d);
    }
    if p.len() != 5 || p.get("changeBackground") != Some(&Value::Bool(true)) || !p.get("guideIDs")?.as_array()?.is_empty() {
        return None;
    }
    let preset = p.get("preset")?.as_str()?;
    let background = match p.get("background")?.as_str()? {
        "white" => 1,
        "black" => 2,
        "transparent" => 3,
        "custom" => 4,
        _ => return None,
    };
    let rgb = p.get("color")?.as_array()?;
    if rgb.len() != 3 {
        return None;
    }
    let rgb: Vec<f64> = rgb.iter().map(Value::as_f64).collect::<Option<_>>()?;
    if !rgb.iter().all(|n| n.is_finite() && (0.0..=1.0).contains(n)) {
        return None;
    }
    let color = ["Rd  ", "Grn ", "Bl  "].into_iter().zip(rgb).fold(desc("RGBC"), |d, (key, n)| d.with(key, D::Double(n * 255.0)));
    let board = desc("artboard")
        .with("guideIDs", D::List(vec![]))
        .with("artboardPresetName", text(preset))
        .with("Clr ", D::Descriptor(color))
        .with("artboardBackgroundType", D::Integer(background));
    Some(d.with("artboard", D::Descriptor(board)).with("changeBackground", D::Integer(1)))
}
