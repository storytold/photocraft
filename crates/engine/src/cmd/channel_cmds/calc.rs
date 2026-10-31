use super::manage::gray_document;
use super::planes::{channel_format, edit_mode, next_alpha_name, plane_surface, read_plane, tone};
use super::refs::{Blending, bad, doc_index, mask_plane, source, source_spec};
use super::targeting::{Target, is_channel_target, is_spot_target, target_of, target_surface};
use super::{AlphaChannel, ColorMode, EngineError, Result, SelectionMode, Session, Value, json, sel};

// ---------------------------------------------------------------------------------------------
// Apply Image / Calculations

/// Apply Image's settings from its params: the blending, the opacity (a fraction) and Preserve
/// Transparency. Shared with the GPU preview.
pub fn apply_image_settings(p: &Value) -> Result<(Blending, f32, bool)> {
    let blending = Blending::parse(p).map_err(|e| bad("image.applyImage", e))?;
    let opacity = p.get("opacity").and_then(Value::as_f64).unwrap_or(100.0).clamp(0.0, 100.0) as f32 / 100.0;
    let preserve = p.get("preserveTransparency").and_then(Value::as_bool).unwrap_or(false);
    Ok((blending, opacity, preserve))
}

/// Apply Image's source planes and weight ([`apply_image_planes`]).
pub type ApplyPlanes = (Vec<Vec<f32>>, Option<Vec<f32>>);

/// The mode of Apply Image's source composite: its own document's, which may not be the target's.
fn apply_source_mode(s: &Session, p: &Value) -> Result<ColorMode> {
    let src_spec = source_spec(p, "source", "source").unwrap_or_else(|| json!({}));
    s.documents().get(doc_index(s, &src_spec, "document")?).map(|d| edit_mode(&d.doc)).ok_or(EngineError::NoDocument)
}

/// Apply Image's inputs over the active document's canvas (row-major): the source planes (one per
/// colour channel of a composite, else one), and the weight the result is applied with besides
/// the opacity, the mask times the selection (`None` for neither). Shared with the GPU preview.
pub fn apply_image_planes(s: &Session, p: &Value) -> Result<ApplyPlanes> {
    let cmd = "image.applyImage";
    let src_spec = source_spec(p, "source", "source").unwrap_or_else(|| json!({}));
    // A CMYK composite reaches a channel or a layer mask as its luminosity, as Calculations reads
    // it: its planes are ink, and an average of their tones would leave Black alone light gray.
    let src_mode = apply_source_mode(s, p)?;
    let src = source(s, &src_spec, cmd, src_mode == ColorMode::Cmyk && (is_channel_target(p) || target_of(p) == Target::Mask))?;
    let mask = mask_plane(s, p, cmd)?;
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    let selection = doc.selection.as_ref().map(|m| sel::mask_from_surface(Some(m), doc.bounds()));
    let weight = match (mask, selection) {
        (Some(m), Some(sl)) => Some(m.iter().zip(&sl).map(|(a, b)| a * b).collect()),
        (m, sl) => m.or(sl),
    };
    Ok((src, weight))
}

/// Apply Image on one pixel `px` of a layer of `mode` (`colors` colour channels, then alpha when
/// `alpha`): `top(c)` is the source's tone for colour channel `c`, `w` the opacity times the
/// weight. Painted over the layer like a brush: the blend only acts where the layer has pixels
/// (where it is transparent the source shows as is), and the alpha grows by source-over, so
/// applying to an empty layer copies the source in any mode. Blended as tone, as Photoshop does:
/// a CMYK layer's ink turns into tone and back (Multiply darkens, so it adds ink); RGB and
/// Grayscale are tones already. Shared with the GPU preview, which ports it for RGB and Grayscale.
#[allow(clippy::too_many_arguments)]
pub fn apply_image_pixel(blending: Blending, preserve: bool, px: &mut [f32], mode: ColorMode, colors: usize, alpha: bool, top: impl Fn(usize) -> f32, w: f32) {
    let base_a = if alpha { px.get(colors).copied().unwrap_or(1.0).clamp(0.0, 1.0) } else { 1.0 };
    let w = if preserve { w * base_a } else { w };
    if w <= 0.0 {
        return;
    }
    let out_a = if preserve { base_a } else { w + base_a * (1.0 - w) };
    for (c, b) in px.iter_mut().take(colors).enumerate() {
        let top = top(c);
        let base = tone(mode, *b);
        let blended = top + (blending.apply(base, top) - top) * base_a;
        *b = tone(mode, if preserve { base + (blended - base) * w } else { (w * blended + base_a * (1.0 - w) * base) / out_a });
    }
    if alpha && let Some(a) = px.get_mut(colors) {
        *a = out_a;
    }
}

pub(super) fn apply_image(s: &mut Session, p: &Value) -> Result<Value> {
    let (blending, opacity, preserve) = apply_image_settings(p)?;
    let src_mode = apply_source_mode(s, p)?;
    let (src, weight) = apply_image_planes(s, p)?;
    let layer = s.active().and_then(|d| d.active_layer);
    s.edit("Apply Image", |doc, _| {
        let area = doc.bounds();
        let weight = |i: usize| opacity * weight.as_ref().map_or(1.0, |m| m[i]);
        let mask_target = target_of(p) == Target::Mask;
        if is_channel_target(p) || mask_target {
            // A grayscale target gets one plane: a CMYK composite's luminosity (above), the average of
            // another composite's planes.
            let srcp =
                if src.len() == 1 { src[0].clone() } else { (0..src[0].len()).map(|i| src.iter().map(|pl| pl[i]).sum::<f32>() / src.len() as f32).collect() };
            let spot = is_spot_target(doc, p);
            let ink = |v: f32| if spot { 1.0 - v } else { v };
            let (surf, _) = target_surface(doc, layer, p)?;
            let base = read_plane(surf, area);
            let out: Vec<f32> = base
                .iter()
                .enumerate()
                .map(|(i, &b)| {
                    let b = ink(b);
                    ink(b + (blending.apply(b, srcp[i]) - b) * weight(i))
                })
                .collect();
            if mask_target {
                // A layer mask keeps its default (reveal or hide) outside the canvas.
                if surf.channels() != 1 {
                    return Err(EngineError::Other("the layer mask is not a grayscale surface".into()));
                }
                surf.write_region(area, &out);
                surf.prune();
            } else {
                *surf = plane_surface(&out, area, surf.format());
            }
            return Ok(());
        }
        let id = layer.ok_or_else(|| EngineError::Other("no active layer".into()))?;
        let fmt = doc.pixel_format();
        let surf = crate::commands::paint_surface(doc, id, &Value::Null)?;
        let sf = surf.format();
        if sf != fmt {
            return Err(EngineError::Other("the target layer's format doesn't match the document".into()));
        }
        let n = sf.channels();
        let colors = sf.mode.color_channels();
        let mut v = surf.read_region(area);
        for (i, px) in v.chunks_exact_mut(n).enumerate() {
            // A CMYK composite onto another mode darkens each tone by its Black, so Black alone
            // stays black. One plane (a channel, a mask) is a tone already.
            let k = if src_mode == ColorMode::Cmyk && sf.mode != ColorMode::Cmyk { src.get(3).map_or(1.0, |pl| tone(src_mode, pl[i])) } else { 1.0 };
            let top = |c: usize| if src.len() > 1 { tone(src_mode, src.get(c).unwrap_or(&src[0])[i]) * k } else { src[0][i] };
            apply_image_pixel(blending, preserve, px, sf.mode, colors, sf.alpha, top, weight(i));
        }
        surf.write_region(area, &v);
        surf.prune();
        Ok(())
    })?;
    Ok(Value::Null)
}

pub(super) fn calculations(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "image.calculations";
    let blending = Blending::parse(p).map_err(|e| bad(cmd, e))?;
    let opacity = p.get("opacity").and_then(Value::as_f64).unwrap_or(100.0).clamp(0.0, 100.0) as f32 / 100.0;
    let s1 = source(s, &source_spec(p, "source1", "source1").unwrap_or_else(|| json!({})), cmd, true)?.remove(0);
    let s2 = source(s, &source_spec(p, "source2", "source2").unwrap_or_else(|| json!({})), cmd, true)?.remove(0);
    let mask = mask_plane(s, p, cmd)?;
    // Source 1 is blended onto Source 2.
    let out: Vec<f32> =
        s2.iter().zip(&s1).enumerate().map(|(i, (&b, &t))| b + (blending.apply(b, t) - b) * opacity * mask.as_ref().map_or(1.0, |m| m[i])).collect();
    let name = p.get("name").and_then(Value::as_str).map(str::to_string);
    match p.get("result").and_then(Value::as_str).unwrap_or("newChannel") {
        "newChannel" => {
            let i = s.edit("Calculations", |doc, _| {
                let name = name.clone().unwrap_or_else(|| next_alpha_name(doc, "Alpha"));
                let area = doc.bounds();
                let fmt = channel_format(doc);
                doc.channels.push(AlphaChannel::new(name, plane_surface(&out, area, fmt)));
                Ok(doc.channels.len() - 1)
            })?;
            Ok(json!({ "channel": i }))
        }
        "newDocument" => {
            let like = s.active().ok_or(EngineError::NoDocument)?.doc.clone();
            let d = gray_document(name.as_deref().unwrap_or("Untitled"), &like, &out);
            let i = s.add_document(d, None);
            Ok(json!({ "document": i }))
        }
        "selection" => {
            let selected = s.edit("Calculations", |doc, _| {
                doc.selection = sel::combine(None, &out, doc.bounds(), SelectionMode::Replace);
                Ok(doc.selection.is_some())
            })?;
            Ok(json!({ "selected": selected }))
        }
        other => Err(bad(cmd, format!("unknown result `{other}` (newChannel|newDocument|selection)"))),
    }
}
