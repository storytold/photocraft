//! Color Sampler points (#1046): numbered colour readings pinned to canvas pixels and listed in
//! the Info panel, so a fixed position can be compared instead of chasing the live pointer.
//!
//! Points are per-document view state ([`crate::DocState::color_samplers`]): they belong to their
//! document, are discarded when it closes, and are not saved to `.pcraft` or PSD. Placing, moving
//! and removing them changes no pixels and adds no History step, so they are not journaled. Each
//! point reads one composite pixel (no averaging); its values match the Info panel's pointer
//! readout: every colour readout mode ([`Readouts`]) plus the extra channels' ink
//! ([`channel_readouts`]).

use photocraft_doc::Document;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{DocState, EngineError, Result, Session};

/// Photoshop's Color Sampler limit: at most 10 points per document.
pub const MAX_SAMPLERS: usize = 10;

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

/// Colour-managed CMYK, in percent, as the Info panel shows it: the same conversion the Color
/// Picker uses (`photocraft_color::convert::rgb_to_cmyk`: the CMYK profile active on this thread,
/// else the built-in coated CMYK), so both panels agree (#8101). 32-bit samples can lie outside
/// 0..1, so each channel is clamped first (NaN reads as 0), like the RGB readout.
pub fn device_cmyk(r: f32, g: f32, b: f32) -> [i32; 4] {
    let unit = |v: f32| if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
    photocraft_color::convert::rgb_to_cmyk([unit(r), unit(g), unit(b)]).map(|v| (unit(v) * 100.0).round() as i32)
}

/// Every colour readout of one composite sample, as the Info panel's readout modes show it
/// (Photoshop's Panel Options › Color Readout): RGB 0–255, colour-managed CMYK percentages,
/// Grayscale, HSB, HSL, Lab, Web Color, Total Ink and Opacity.
///
/// Verified against Photoshop 2027's Info panel (2026-10-28, flat swatches): Grayscale is
/// `100 − Rec.601 luma` of the encoded RGB (pure red reads K 70 %, gray 128 reads 50 %), whatever
/// the document's profile (sRGB and Adobe RGB read alike) or the gray working space; Opacity is
/// the composite alpha under the pointer (a 50 % layer over an opaque layer reads 100 %).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Readouts {
    pub rgb: [i32; 3],
    pub cmyk: [i32; 4],
    /// Grayscale: K %.
    pub gray: i32,
    /// Hue 0–359°, saturation %, brightness %.
    pub hsb: [i32; 3],
    /// Hue 0–359°, saturation %, lightness %.
    pub hsl: [i32; 3],
    /// L 0–100, a and b about −128..127.
    pub lab: [i32; 3],
    /// Six lowercase hex digits, `rrggbb`.
    pub web: String,
    /// The CMYK percentages added up.
    pub total_ink: i32,
    /// Composite alpha %.
    pub opacity: i32,
}

/// The readouts of straight-alpha `rgba` (0..1; 32-bit samples outside that are clamped, NaN
/// reads as 0) in `doc`, whose CMYK profile the CMYK and Total Ink readouts use.
/// Lab is derived from this RGB composite, not the original Lab channels: native out-of-gamut
/// Lab values already clipped during RGB compositing cannot be recovered here.
pub fn readouts(doc: &Document, rgba: [f32; 4]) -> Readouts {
    let unit = |v: f32| if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
    let rgb = [unit(rgba[0]), unit(rgba[1]), unit(rgba[2])];
    let q = |v: f32| (v * 255.0).round() as i32;
    let pct = |v: f32| (v * 100.0).round() as i32;
    let cmyk = photocraft_color::convert::with_cmyk_space(photocraft_compose::cmyk_space(doc).as_ref(), || device_cmyk(rgb[0], rgb[1], rgb[2]));
    let [h, s, b] = photocraft_color::convert::rgb_to_hsb_sextant(rgb);
    let hue = ((h * 60.0).round() as i32).rem_euclid(360);
    let (mx, mn) = (rgb[0].max(rgb[1]).max(rgb[2]), rgb[0].min(rgb[1]).min(rgb[2]));
    let (l, d) = ((mx + mn) / 2.0, mx - mn);
    let sl = if d <= 0.0 { 0.0 } else { d / (1.0 - (2.0 * l - 1.0).abs()).max(f32::EPSILON) };
    Readouts {
        rgb: rgb.map(q),
        cmyk,
        gray: pct(1.0 - photocraft_color::convert::rgb_to_gray(rgb)),
        hsb: [hue, pct(s), pct(b)],
        hsl: [hue, pct(sl), pct(l)],
        lab: lab(doc, rgb).map(|v| v.round() as i32),
        web: format!("{:02x}{:02x}{:02x}", q(rgb[0]), q(rgb[1]), q(rgb[2])),
        total_ink: cmyk.iter().sum(),
        opacity: pct(unit(rgba[3])),
    }
}

/// CIE Lab (D50) of composite `rgb` through the document's profile (the compositor's output
/// profile, so an Adobe RGB document's red is not read as sRGB red), the same transform Color
/// Range uses; the sRGB maths only when no transform can be built.
fn lab(doc: &Document, rgb: [f32; 3]) -> [f32; 3] {
    let src = crate::color_cmds::composite_profile(doc);
    match photocraft_cms::cached(&src, photocraft_cms::Builtin::LabD50.profile(), photocraft_cms::TransformOptions::default()) {
        Ok(t) => {
            let mut out = [0.0f32; 3];
            t.eval(&rgb, &mut out);
            // The Lab profile's normalised encoding (ICC v4) → L 0–100, a and b −128–127.
            [out[0] * 100.0, out[1] * 255.0 - 128.0, out[2] * 255.0 - 128.0]
        }
        Err(_) => photocraft_color::convert::srgb_to_lab(rgb),
    }
}

/// One extra channel's reading at a pixel, as Photoshop shows a channel targeted on its own:
/// ink % for a spot channel (its stored value, 1 = solid ink), masked % for an alpha channel
/// (`1 − value`: white, selected, reads 0 %).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChannelReadout {
    pub name: String,
    pub spot: bool,
    pub percent: i32,
}

/// The reading of every extra (alpha and spot) channel of `doc` at pixel `(x, y)`, in
/// Channels-panel order: one pixel, no averaging. Empty off the canvas.
pub fn channel_readouts(doc: &Document, x: i32, y: i32) -> Vec<ChannelReadout> {
    if !doc.bounds().contains(x, y) {
        return Vec::new();
    }
    doc.channels
        .iter()
        .map(|ch| {
            let v = ch.surface.pixel(x, y).first().copied().unwrap_or(0.0);
            let v = if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
            let ink = if ch.spot.is_some() { v } else { 1.0 - v };
            ChannelReadout { name: ch.name.clone(), spot: ch.spot.is_some(), percent: (ink * 100.0).round() as i32 }
        })
        .collect()
}

/// Total Spot Ink: every spot channel's ink % at the pixel added up (alpha channels left out). A
/// separations total, where Photoshop's Total Ink adds up the CMYK plates only.
pub fn total_spot_ink(channels: &[ChannelReadout]) -> i32 {
    channels.iter().filter(|c| c.spot).fold(0_i32, |total, c| total.saturating_add(c.percent))
}

/// The Info-panel values for a sample point at document pixel `pos`: composite RGB 0–255 and
/// colour-managed CMYK percentages (the document's CMYK profile when it has one). One composite
/// pixel, no averaging.
pub fn readout(doc: &Document, pos: [f64; 2]) -> ([i32; 3], [i32; 4]) {
    let r = readouts(doc, crate::sample_cmds::sample_color(doc, None, pos[0], pos[1], 1, crate::sample_cmds::SampleLayers::All));
    (r.rgb, r.cmyk)
}

/// One point as the list command reports it: its number and position, every [`Readouts`] field,
/// `totalSpotInk` ([`total_spot_ink`]) and `channels` ([`channel_readouts`]).
pub fn point_json(doc: &Document, index: usize, pos: [f64; 2]) -> Value {
    let rgba = crate::sample_cmds::sample_color(doc, None, pos[0], pos[1], 1, crate::sample_cmds::SampleLayers::All);
    let mut v = json!({"index": index, "number": index + 1, "position": [pos[0], pos[1]]});
    if let (Some(o), Ok(Value::Object(r))) = (v.as_object_mut(), serde_json::to_value(readouts(doc, rgba))) {
        o.extend(r);
        // Points sit on whole pixels inside the canvas (`pixel`), so the casts are exact.
        let channels = channel_readouts(doc, pos[0] as i32, pos[1] as i32);
        o.insert("totalSpotInk".into(), json!(total_spot_ink(&channels)));
        o.insert("channels".into(), json!(channels));
    }
    v
}

fn sample_channels(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "document.sampleChannels";
    let c = |k: &str| -> Result<i32> {
        p.get(k)
            .and_then(Value::as_f64)
            .map(f64::floor)
            .filter(|v| v.is_finite() && *v >= f64::from(i32::MIN) && *v <= f64::from(i32::MAX))
            .map(|v| v as i32)
            .ok_or_else(|| bad(CMD, format!("`{k}` must be a finite number (document pixels)")))
    };
    let (x, y) = (c("x")?, c("y")?);
    let st = s.active().ok_or(EngineError::NoDocument)?;
    Ok(json!(channel_readouts(&st.doc, x, y)))
}

fn list_json(doc: &Document, points: &[[f64; 2]]) -> Value {
    json!({
        "max": MAX_SAMPLERS,
        "samplers": points.iter().enumerate().map(|(i, p)| point_json(doc, i, *p)).collect::<Vec<_>>(),
    })
}

fn coord(cmd: &str, p: &Value, key: &str) -> Result<f64> {
    p.get(key).and_then(Value::as_f64).filter(|v| v.is_finite()).ok_or_else(|| bad(cmd, format!("`{key}` must be a finite number (document pixels)")))
}

/// The pixel a point at document coordinate `(x, y)` sits on, if it is inside the canvas. Whole
/// pixels only: the click's fractional position floors to the pixel it lands on.
fn pixel(doc: &Document, x: f64, y: f64) -> Option<[f64; 2]> {
    let (px, py) = (x.floor(), y.floor());
    if px < f64::from(i32::MIN) || px > f64::from(i32::MAX) || py < f64::from(i32::MIN) || py > f64::from(i32::MAX) {
        return None;
    }
    doc.bounds().contains(px as i32, py as i32).then_some([px, py])
}

fn index_param(cmd: &str, p: &Value) -> Result<usize> {
    p.get("index").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).ok_or_else(|| bad(cmd, "pass `index` (0-based)"))
}

/// Apply a Color Sampler change to the active document: view state, so no history step and no
/// pixels. Bumps the revision (keeping a clean document clean) so UIs refresh.
fn change<R>(s: &mut Session, f: impl FnOnce(&mut DocState) -> Result<R>) -> Result<R> {
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    let r = f(st)?;
    let clean = st.saved_revision == st.revision;
    st.revision += 1;
    if clean {
        st.saved_revision = st.revision;
    }
    st.last_damage = Some(photocraft_geom::Rect::EMPTY);
    Ok(r)
}

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "view.colorSamplers.add";
    let x = coord(CMD, p, "x")?;
    let y = coord(CMD, p, "y")?;
    change(s, |st| {
        let pos = pixel(&st.doc, x, y).ok_or_else(|| EngineError::Other("the sample point is outside the canvas".into()))?;
        if st.color_samplers.len() >= MAX_SAMPLERS {
            return Err(EngineError::Other(format!("Color Sampler limit reached: at most {MAX_SAMPLERS} points per document")));
        }
        st.color_samplers.push(pos);
        let i = st.color_samplers.len() - 1;
        Ok(json!({"index": i, "count": st.color_samplers.len(), "point": point_json(&st.doc, i, pos)}))
    })
}

fn move_point(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "view.colorSamplers.move";
    let i = index_param(CMD, p)?;
    let x = coord(CMD, p, "x")?;
    let y = coord(CMD, p, "y")?;
    change(s, |st| {
        let n = st.color_samplers.len();
        if i >= n {
            return Err(bad(CMD, format!("no sample point {i} (the document has {n})")));
        }
        let pos = pixel(&st.doc, x, y).ok_or_else(|| EngineError::Other("the sample point is outside the canvas".into()))?;
        st.color_samplers[i] = pos;
        Ok(json!({"index": i, "point": point_json(&st.doc, i, pos)}))
    })
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "view.colorSamplers.delete";
    let i = index_param(CMD, p)?;
    change(s, |st| {
        let n = st.color_samplers.len();
        if i >= n {
            return Err(bad(CMD, format!("no sample point {i} (the document has {n})")));
        }
        st.color_samplers.remove(i);
        Ok(json!({"deleted": 1, "remaining": st.color_samplers.len()}))
    })
}

fn clear(s: &mut Session, _p: &Value) -> Result<Value> {
    change(s, |st| {
        let n = st.color_samplers.len();
        st.color_samplers.clear();
        Ok(json!({"deleted": n, "remaining": 0}))
    })
}

fn list(s: &mut Session, _p: &Value) -> Result<Value> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    Ok(list_json(&st.doc, &st.color_samplers))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "view.colorSamplers.add",
            label: "Add Color Sampler",
            menu: &[],
            shortcut: None,
            params: r##"{"x":f64,"y":f64} → the new point {index, number, position, rgb, cmyk, gray, hsb, hsl, lab, web, totalInk, opacity, totalSpotInk, channels:[{name, spot, percent}]}; errors at the 10-point limit or off the canvas"##,
            enabled: has_doc,
            run: add,
            journal: false,
        },
        CommandSpec {
            id: "view.colorSamplers.move",
            label: "Move Color Sampler",
            menu: &[],
            shortcut: None,
            params: r##"{"index":n,"x":f64,"y":f64}"##,
            enabled: has_doc,
            run: move_point,
            journal: false,
        },
        CommandSpec {
            id: "view.colorSamplers.delete",
            label: "Delete Color Sampler",
            menu: &[],
            shortcut: None,
            params: r##"{"index":n}"##,
            enabled: has_doc,
            run: delete,
            journal: false,
        },
        CommandSpec {
            id: "view.colorSamplers.clear",
            label: "Clear Color Samplers",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: has_doc,
            run: clear,
            journal: false,
        },
        CommandSpec {
            id: "view.colorSamplers.list",
            label: "Color Samplers",
            menu: &[],
            shortcut: None,
            params: r##"{} → {max, samplers:[{index, number, position, rgb, cmyk, gray, hsb, hsl, lab, web, totalInk, opacity, totalSpotInk, channels:[{name, spot, percent}]}]} (every Info panel readout of each point: RGB 0–255, CMYK %, Grayscale K %, HSB, HSL, Lab, web hex, Total Ink % (the CMYK plates), composite Opacity %, Total Spot Ink % (every spot channel added up) and each alpha/spot channel's % as a channel targeted alone reads it)"##,
            enabled: has_doc,
            run: list,
            journal: false,
        },
        CommandSpec {
            id: "document.sampleChannels",
            label: "Sample Channels",
            menu: &[],
            shortcut: None,
            params: r##"{"x":i32,"y":i32} → [{name, spot, percent}] (every alpha and spot channel's reading at that pixel, in Channels-panel order: ink % for a spot channel, masked % for an alpha channel with white = 0 %; one pixel, no averaging; empty off the canvas)"##,
            enabled: has_doc,
            run: sample_channels,
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests;
