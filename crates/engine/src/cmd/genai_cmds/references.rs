//! Reference pictures for Generative Fill (#10940): pictures the person adds to show what the
//! fill should look like (a product, a style, a material). They are a guide only. The document's
//! picture and the selection's shape stay the request's own inputs and are never listed here.
//!
//! Each one arrives as encoded image data (`{"data": base64}` in `edit.generativeFill`). The
//! command checks the count and size at once, so a bad call fails before anything is planned.
//! The fill's job then decodes each picture, scales it to the request's longest side and sends it
//! as a new sRGB PNG. Re-encoding keeps only the pixels, so a photo's location and camera details
//! never leave the computer, and every route gets the one format they all read.
//!
//! Whether the chosen model takes references at all, and how many, is the shared layer's call: a
//! route that cannot take them refuses the plan with its reason, before consent. Nothing is
//! dropped quietly.

use photocraft_geom::Rect;
use serde_json::Value;

use super::pixels;
use crate::{EngineError, Result};

/// Most reference pictures one fill takes: our own bound, so one call cannot queue an unbounded
/// amount of decoding. A model's own, lower limit is enforced by the shared layer.
pub const MAX_REFERENCES: usize = 4;
/// Largest reference picture accepted, in encoded bytes: the shared layer's own bound per picture.
pub const MAX_REFERENCE_BYTES: usize = 10 * 1024 * 1024;

/// The `references` param: encoded pictures in the order given. Missing or `null` is none.
pub(super) fn param(p: &Value, cmd: &str) -> Result<Vec<Vec<u8>>> {
    let bad = |msg: String| EngineError::BadParams { cmd: cmd.into(), msg };
    let list = match p.get("references") {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Array(list)) => list,
        Some(_) => return Err(bad("`references` must be a list of {\"data\": base64}".into())),
    };
    if list.len() > MAX_REFERENCES {
        return Err(bad(format!("Generative Fill takes at most {MAX_REFERENCES} reference pictures")));
    }
    list.iter()
        .enumerate()
        .map(|(i, item)| {
            let n = i + 1;
            let text = item.get("data").and_then(Value::as_str).ok_or_else(|| bad(format!("reference picture {n} must be {{\"data\": base64}}")))?;
            // Base64 is four characters per three bytes: refuse an oversized one before decoding it.
            if text.len() / 4 * 3 > MAX_REFERENCE_BYTES + 3 {
                return Err(bad(too_big(n)));
            }
            let bytes = photocraft_paint::tile::b64_decode(text).ok_or_else(|| bad(format!("reference picture {n} is not valid base64")))?;
            if bytes.is_empty() {
                return Err(bad(format!("reference picture {n} is empty")));
            }
            if bytes.len() > MAX_REFERENCE_BYTES {
                return Err(bad(too_big(n)));
            }
            Ok(bytes)
        })
        .collect()
}

/// Why picture `n` is refused for its size.
fn too_big(n: usize) -> String {
    format!("reference picture {n} is larger than {} MB", MAX_REFERENCE_BYTES / (1024 * 1024))
}

/// Picture `n` (encoded PNG, JPEG or WebP) as it is sent: 8-bit sRGB PNG, its longest side at
/// most `side`, nothing kept but the pixels.
// shortcut: an embedded colour profile is ignored and the pixels are read as sRGB; convert
// through the profile when people report colour shifts on references from wide-gamut cameras.
pub(super) fn prepare(bytes: &[u8], n: usize, side: u32) -> Result<Vec<u8>> {
    let unreadable = format!("PhotoCraft can't read reference picture {n} (use a PNG, JPEG or WebP picture)");
    let (s, r) = pixels::read_picture(bytes, &unreadable, &format!("reference picture {n} is empty"))?;
    let (w, h) = fit(r.width(), r.height(), side);
    let out = Rect::from_xywh(0, 0, w, h);
    let px = pixels::scaled(&s, r, w, h).to_interleaved(out);
    pixels::encode_rgba(px, w, h)
}

/// `w`×`h` scaled down so its longest side is at most `side`, keeping its shape; never scaled up.
pub(super) fn fit(w: u32, h: u32, side: u32) -> (u32, u32) {
    let longest = w.max(h).max(1);
    if longest <= side {
        return (w.max(1), h.max(1));
    }
    let k = f64::from(side) / f64::from(longest);
    (((f64::from(w) * k).round() as u32).max(1), ((f64::from(h) * k).round() as u32).max(1))
}
