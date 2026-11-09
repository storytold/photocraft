//! File › Automate › Fit Image, recorded as the script event `3caa3434-cb67-11d1-bc43-0060b0a13dc4`
//! after the Image Size it runs, as `docs/atn.md` ("Recorded with source files") measures it:
//! `Msge` text, `Hght` and `Wdth` in pixels, `limit` (Don't Enlarge), in that order (#10723).

use super::descriptors::{PX, desc, jstr, px, text};
use super::fields::{Mapped, step};
use super::{D, Descriptor, Value, json};

/// The script event's string ID.
pub(super) const FIT_IMAGE_EVENT: &str = "3caa3434-cb67-11d1-bc43-0060b0a13dc4";
/// The native command the event plays as.
pub(super) const FIT_IMAGE: &str = "file.automate.fitImage";
/// The recorded descriptor, kept when it differs from the one [`fit_image_to`] would write
/// (its message, class, key order or ID encoding), so a load and save keeps the step unchanged.
const SOURCE: &str = "_atnFitImageDescriptor";

/// The Fit Image settings the event holds.
#[derive(Debug, PartialEq)]
struct Fit {
    width: f64,
    height: f64,
    dont_enlarge: bool,
}

/// A recorded box side: a pixel unit float the native command reads as set (positive, finite).
fn side(v: &D) -> Option<f64> {
    match v {
        D::UnitFloat { unit, value } if *unit == PX && value.is_finite() && *value > 0.0 => Some(*value),
        _ => None,
    }
}

/// The measured form only: each of the four keys exactly once and nothing else. Another unit,
/// a box side of zero (which the native command reads as "keep that side") or an extra key
/// keeps the whole step as recorded.
fn recorded(d: &Descriptor) -> Option<Fit> {
    if d.items.len() != 4 || ["Msge", "Hght", "Wdth", "limit"].iter().any(|key| d.items.iter().filter(|(k, _)| k.is(key)).count() != 1) {
        return None;
    }
    let D::Text(_) = d.get("Msge")? else { return None };
    let D::Boolean(dont_enlarge) = d.get("limit")? else { return None };
    Some(Fit { width: side(d.get("Wdth")?)?, height: side(d.get("Hght")?)?, dont_enlarge: *dont_enlarge })
}

/// The event's descriptor for `fit` with `message` as its `Msge`.
fn descriptor(fit: &Fit, message: D) -> Descriptor {
    desc("null").with("Msge", message).with("Hght", px(fit.height)).with("Wdth", px(fit.width)).with("limit", D::Boolean(fit.dont_enlarge))
}

/// The descriptor a step recorded in PhotoCraft saves as. The message is empty: it is the
/// script's own text, not a setting, and no text of the reference application is copied.
fn canonical(fit: &Fit) -> Descriptor {
    descriptor(fit, text(""))
}

/// Map the measured form; any other stays a complete kept step.
pub(super) fn fit_image_from(d: &Descriptor) -> Mapped {
    let Some(fit) = recorded(d) else { return Mapped::Unsupported { harmless: false } };
    let mut p = json!({"width": fit.width, "height": fit.height, "dontEnlarge": fit.dont_enlarge});
    if *d != canonical(&fit) {
        p[SOURCE] = photocraft_paint::tile::b64_encode(&d.to_bytes()).into();
    }
    step(FIT_IMAGE, p)
}

/// The settings of a native step, when the event can hold all of them: both box sides set, and
/// the default resampling, since the event has no key for another method. A missing, zero or
/// `null` side plays as the document's own size, which the event cannot carry either.
fn native(p: &Value) -> Option<Fit> {
    let m = p.as_object()?;
    if m.keys().any(|key| !["width", "height", "dontEnlarge", "resample", SOURCE].contains(&key.as_str())) {
        return None;
    }
    if !matches!(m.get("resample"), None | Some(Value::Null)) && jstr(p, "resample") != Some("bicubic") {
        return None;
    }
    let side = |key: &str| m.get(key).and_then(Value::as_f64).filter(|v| v.is_finite() && *v > 0.0);
    let dont_enlarge = match m.get("dontEnlarge") {
        None | Some(Value::Null) => false,
        Some(v) => v.as_bool()?,
    };
    Some(Fit { width: side("width")?, height: side("height")?, dont_enlarge })
}

/// Save the recorded descriptor while the step still holds its settings; once they were
/// changed, write the new settings with the recorded message.
pub(super) fn fit_image_to(_: &str, p: &Value) -> Option<Descriptor> {
    let fit = native(p)?;
    let Some(source) = jstr(p, SOURCE) else { return Some(canonical(&fit)) };
    let source = Descriptor::from_bytes(&photocraft_paint::tile::b64_decode(source)?).ok()?;
    if recorded(&source)? == fit {
        return Some(source);
    }
    Some(descriptor(&fit, source.get("Msge")?.clone()))
}
