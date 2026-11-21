//! Image › Analysis › Find Sun: where the sun is in a 2:1 equirectangular (latitude/longitude)
//! image, as a direction in a chosen axis convention, and the colour and intensity of a
//! directional light that matches it (#15977). The maths is [`photocraft_algo::sun`].
//!
//! The command reads the composite reduced to at most [`SAMPLE_SIDE`] pixels across (area
//! averaged, so the sun's energy is kept), and changes nothing, so it records no history step.
//! The document keeps that reduced composite until its next revision, so the dialog's drag on the
//! image only measures again.
//! With `at` it measures the sun at that document point instead of finding it, for when the
//! brightest area is not the sun.

use photocraft_algo::sun::{self, Frame, Sky};
use photocraft_color::{ColorMode, SampleType};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

const ID: &str = "image.analysis.findSun";

/// Longest side of the reduced composite the sun is found in. 2048 × 1024 resolves a sun disc
/// (about half a degree) to a few pixels.
pub const SAMPLE_SIDE: u32 = 2048;

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: ID.into(), msg: msg.into() }
}

fn enabled(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    match d.doc.mode {
        ColorMode::Rgb | ColorMode::Grayscale => Ok(()),
        _ => Err("Find Sun needs an RGB or Grayscale document".into()),
    }
}

fn frame_param(p: &Value) -> Result<Frame> {
    match p.get("frame") {
        None | Some(Value::Null) => Ok(Frame::YUpLeftHanded),
        Some(v) => serde_json::from_value(v.clone()).map_err(|_| bad("frame must be \"yUpLeftHanded\", \"yUpRightHanded\" or \"zUpRightHanded\"")),
    }
}

fn number(p: &Value, key: &str, default: f64) -> Result<f64> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v.as_f64().filter(|n| n.is_finite()).ok_or_else(|| bad(format!("{key} must be a number"))),
    }
}

fn find_sun(s: &mut Session, p: &Value) -> Result<Value> {
    let frame = frame_param(p)?;
    let heading = number(p, "heading", 0.0)?;
    let exposure = number(p, "exposure", 1.0)?;
    if exposure < 0.0 {
        return Err(bad("exposure must not be negative"));
    }
    let at = match p.get("at") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let a = v.as_array().filter(|a| a.len() == 2).ok_or_else(|| bad("at must be [x, y] in document pixels"))?;
            let xy = [a[0].as_f64(), a[1].as_f64()];
            match xy {
                [Some(x), Some(y)] if x.is_finite() && y.is_finite() => Some([x, y]),
                _ => return Err(bad("at must be [x, y] in document pixels")),
            }
        }
    };
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    let doc = st.doc.clone();
    let size = doc.bounds();
    let (doc_w, doc_h) = (f64::from(size.width().max(1)), f64::from(size.height().max(1)));
    let reduced = match &st.sun_sample {
        Some((revision, buffer)) if *revision == st.revision => buffer.clone(),
        _ => {
            let buffer = std::sync::Arc::new(photocraft_compose::thumbnail_buffer(&doc, SAMPLE_SIDE));
            st.sun_sample = Some((st.revision, buffer.clone()));
            buffer
        }
    };
    let (w, h) = (reduced.rect.width() as usize, reduced.rect.height() as usize);
    let sky = Sky { px: &reduced.px, width: w, height: h, linear: doc.depth == SampleType::F32 };

    let (direction, found, clear) = match at {
        Some([x, y]) => {
            if !(0.0..=doc_w).contains(&x) || !(0.0..=doc_h).contains(&y) {
                return Err(bad("at is outside the document"));
            }
            (sun::direction(x / doc_w, y / doc_h), true, true)
        }
        None => match sun::find(&sky) {
            Some(f) => (f.direction, true, f.clear),
            // No light at all: report the sky's zenith side so the result is still complete.
            None => (sun::direction(0.5, 0.25), false, false),
        },
    };
    let light = sun::measure(&sky, direction).ok_or_else(|| EngineError::Other("the document is too small to find a sun in".into()))?;
    let [u, v] = sun::position(direction);
    let turned = sun::turn(direction, heading);
    let [azimuth, elevation] = sun::azimuth_elevation(direction);
    let intensity = light.intensity * exposure;
    let hex = format!("#{}", light.colour.iter().map(|&c| format!("{:02X}", (sun::linear_to_srgb(c) * 255.0).round() as u8)).collect::<String>());
    let mut out = json!({
        "found": found,
        "picked": at.is_some(),
        "clear": clear,
        "position": [u * doc_w, v * doc_h],
        "azimuth": azimuth,
        "elevation": elevation,
        "frame": frame,
        "heading": heading,
        "direction": frame.from_sphere(turned),
        "equirectangular": size.width() == size.height().saturating_mul(2),
        "linear": sky.linear,
        "light": {
            "measured": light.measured,
            "clipped": light.clipped,
            "colour": hex,
            "colourLinear": light.colour,
            "kelvin": light.kelvin,
            "intensity": intensity,
            "exposure": exposure,
        },
    });
    if frame == Frame::YUpLeftHanded {
        let (quaternion, euler) = sun::light_rotation(turned);
        out["lightRotation"] = json!({"quaternion": quaternion, "euler": euler});
    }
    Ok(out)
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: ID,
        label: "Find Sun…",
        menu: &["Image", "Analysis"],
        shortcut: None,
        params: r##"{"at":[x,y]? (document px: measure the sun there instead of finding it),"frame":"yUpLeftHanded|yUpRightHanded|zUpRightHanded"="yUpLeftHanded","heading":degrees=0 (the sky turned so its centre faces this far right),"exposure":number=1 (scales intensity)} → found, clear, position, azimuth, elevation, direction (in frame), linear (a 32-bit document), light {measured, clipped, colour "#RRGGBB", colourLinear, kelvin, intensity}, lightRotation {quaternion [x,y,z,w], euler [x,y,z]} (yUpLeftHanded only)"##,
        enabled,
        run: find_sun,
        journal: false,
    }]
}

#[cfg(test)]
#[path = "sun_cmds_tests.rs"]
mod tests;
