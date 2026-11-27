//! Edit › Fill… (`edit.fill`): Photoshop's Fill dialog as one command.
//!
//! Contents: Foreground / Background / Color / Content-Aware / Pattern / History / Black /
//! 50% Gray / White, blended with `mode` at `opacity`, optionally preserving the target's
//! transparency. The selection (or the whole canvas) is filled, as one history step ("Fill").
//! Every parameter is validated: a bad one is an error, never a panic.

use photocraft_algo::content_aware::FillOptions;
use photocraft_color::BlendMode;
use photocraft_geom::Rect;
#[cfg(test)]
use photocraft_geom::TILE_SIZE;
#[cfg(test)]
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::commands::{cmyk_param, color_param, layer_param, parse_blend};
use crate::content_sampling::{content_aware, content_aware_options};
pub use crate::pixel_fill::greyed_modes;
use crate::pixel_fill::{Blend, Source, blend_into, check_mode_available, read_rgba};
use crate::{EngineError, Result, Session};

const CMD: &str = "edit.fill";

/// Every `contents` value, in the dialog's order.
pub const CONTENTS: [&str; 9] = ["foreground", "background", "color", "contentAware", "pattern", "history", "black", "gray", "white"];

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: CMD.into(), msg: msg.into() }
}

/// The `contents` param; without one, `color` (the old `{"color": …}` form, foreground by default).
fn contents(p: &Value) -> Result<&str> {
    match p.get("contents") {
        None | Some(Value::Null) => Ok("color"),
        Some(Value::String(c)) => {
            CONTENTS.iter().find(|k| k.eq_ignore_ascii_case(c)).copied().ok_or_else(|| bad(format!("unknown contents `{c}` ({})", CONTENTS.join("|"))))
        }
        Some(v) => Err(bad(format!("`contents` must be a string, not {v}"))),
    }
}

fn mode(p: &Value) -> Result<BlendMode> {
    match p.get("mode") {
        None | Some(Value::Null) => Ok(BlendMode::Normal),
        Some(Value::String(m)) => parse_blend(m).filter(|m| *m != BlendMode::PassThrough).ok_or_else(|| bad(format!("unknown blend mode `{m}`"))),
        Some(v) => Err(bad(format!("`mode` must be a blend mode name, not {v}"))),
    }
}

/// Opacity in percent (0–100) → 0..1.
fn opacity(p: &Value) -> Result<f32> {
    match p.get("opacity") {
        None | Some(Value::Null) => Ok(1.0),
        Some(v) => {
            let o = v.as_f64().filter(|o| o.is_finite()).ok_or_else(|| bad(format!("`opacity` must be a number 0..100, not {v}")))?;
            Ok((o.clamp(0.0, 100.0) / 100.0) as f32)
        }
    }
}

fn flag(p: &Value, k: &str, d: bool) -> Result<bool> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(d),
        Some(Value::Bool(b)) => Ok(*b),
        Some(v) => Err(bad(format!("`{k}` must be true or false, not {v}"))),
    }
}

/// Edit › Fill.
pub fn fill(s: &mut Session, p: &Value) -> Result<Value> {
    let contents = contents(p)?;
    let mode = mode(p)?;
    let opacity = opacity(p)?;
    let preserve = flag(p, "preserveTransparency", false)?;
    let channel = crate::channel_cmds::is_channel_target(p);
    let id = if channel { None } else { Some(layer_param(s, p)?) };
    let st = s.active().ok_or(EngineError::NoDocument)?;
    check_mode_available(CMD, &st.doc, mode)?;
    let doc_bounds = st.doc.bounds();
    let sel = st.doc.selection.clone();
    let area = sel.as_ref().map_or(doc_bounds, |m| m.content_bounds().intersect(&doc_bounds));
    let mut out = json!({ "contents": contents });
    let restore = contents == "history";
    let source = match contents {
        "foreground" => color_source(s, s.tools.foreground, 0),
        // As the document takes the swatch: its gray profile's value in a Grayscale document (#11940).
        "background" => color_source(s, crate::color_cmds::swatch_in_doc(s, s.tools.background), 1),
        "color" => {
            let rgba = color_param(CMD, p, "color", s.tools.foreground)?;
            match cmyk_param(CMD, p, "colorCmyk")? {
                Some(cmyk) if rgba[3] >= 1.0 => Source::Cmyk { rgba, cmyk },
                Some(_) => return Err(bad("`colorCmyk` requires an opaque color")),
                None if p.get("color").is_none_or(Value::is_null) => color_source(s, rgba, 0),
                None => match s.tools.matching_cmyk([rgba[0], rgba[1], rgba[2]]) {
                    Some(cmyk) if rgba[3] >= 1.0 => Source::Cmyk { rgba, cmyk },
                    _ => Source::Color(rgba),
                },
            }
        }
        "black" => Source::Color([0.0, 0.0, 0.0, 1.0]),
        "gray" => Source::Color([128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0, 1.0]),
        "white" => Source::Color([1.0; 4]),
        "pattern" => {
            let pat = crate::pattern_cmds::resolve_param(s, CMD, p)?;
            let (scale, angle, _, phase) = crate::pattern_cmds::placement(CMD, p)?;
            // Where a pattern fill layer and a Pattern Overlay paint the pattern through the grey
            // profile (an 8- or 16-bit Grayscale document in the default grey space), so does
            // Edit › Fill (#14975).
            let space = photocraft_compose::pattern::PatternSpace::of_pixels(&st.doc);
            let tile = photocraft_compose::pattern::Tile::new_in(&pat, space).ok_or_else(|| bad("the pattern is empty or too large to paint"))?;
            // Photoshop's Fill uses the canvas origin.
            let place = photocraft_compose::pattern::Placement::new(Rect::EMPTY, false, phase, scale, angle);
            out["pattern"] = json!(pat.id);
            Source::Pixels(area, photocraft_compose::pattern::render(&tile, &place, area))
        }
        "history" => {
            // A History panel row `state` or a `snapshot`, else the History Brush's source.
            let source_doc = crate::history_source::paint_source(st, p, CMD)?;
            let src = crate::channel_cmds::read_target_surface(source_doc.as_ref(), id, p)
                .map_err(|_| EngineError::Other("the target did not exist (as pixels, a mask or a channel) in that history state".into()))?;
            Source::Pixels(area, read_rgba(src, area, "reading Fill's history source")?)
        }
        "contentAware" => {
            let sel = sel.as_ref().ok_or_else(|| EngineError::Other("Content-Aware fill needs a selection".into()))?;
            if channel || crate::commands::is_mask_target(p) {
                return Err(EngineError::Other("Content-Aware fill works on layer pixels only".into()));
            }
            let adapt = flag(p, "colorAdaptation", true)?;
            let scale = flag(p, "scaleAdaptation", false)?;
            let mirror = flag(p, "mirror", false)?;
            let rotate = flag(p, "rotationAdaptation", false)?;
            let lid = id.ok_or_else(|| EngineError::Other("no active layer".into()))?;
            let surf = st.doc.layer(lid).and_then(|l| l.surface()).ok_or_else(|| EngineError::Other("Content-Aware fill needs a pixel layer".into()))?;
            let seed = p.get("seed").and_then(Value::as_u64).unwrap_or(1);
            // The native Boolean enables quarter-turn candidates, within the existing mosaic cap.
            let rotations = if rotate { photocraft_algo::content_aware::rotation_level("full") } else { Vec::new() };
            let options = FillOptions { rotations, scale, mirror, ..content_aware_options(adapt, seed) };
            content_aware(surf, sel, doc_bounds, &options)?
        }
        _ => return Err(bad(format!("unknown contents `{contents}`"))),
    };
    if area.is_empty() {
        return Ok(out);
    }
    // The document's own CMYK profile converts the fill colour, like the compositor and brush.
    let cmyk = crate::brush_cmds::doc_cmyk_space(s);
    photocraft_color::convert::with_cmyk_space(cmyk.as_ref(), || {
        s.edit("Fill", |doc, _| {
            let sel = doc.selection.clone();
            let (surf, lock) = crate::channel_cmds::target_surface(doc, id, p)?;
            blend_into(surf, area, &source, sel.as_ref(), Blend { mode, opacity, keep_alpha: preserve || lock, restore })?;
            Ok(())
        })
    })?;
    Ok(out)
}

/// A fill colour: the inks it was picked as when it was picked as CMYK (#4196), else plain sRGB.
fn color_source(s: &Session, c: [f32; 4], role: usize) -> Source {
    match s.tools.picked_role_cmyk(role) {
        Some(cmyk) if c[3] >= 1.0 => Source::Cmyk { rgba: c, cmyk },
        _ => Source::Color(c),
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod mode_by_document_tests;
