//! Image menu remnants: Image Rotation › Arbitrary, and the palette / ink modes — Mode › Indexed
//! Color, Color Table, Bitmap and Duotone.
//!
//! Indexed, Bitmap and Duotone documents keep expanded pixels (RGB 8-bit / Gray 8-bit / Gray, see
//! `Document::pixel_format`): Indexed and Bitmap quantise the flattened image in place and record
//! the palette (`Document::color_table`) or the 1-bit look; Duotone keeps the gray pixels and the
//! inks (`Document::duotone`), which display and flat export render through. Like Photoshop the
//! conversions flatten (Indexed, Bitmap) and require a grayscale source (Bitmap, Duotone).

use craft_cms::{Builtin, ColorSpace, TransformOptions};
use photocraft_algo::palette_map::{AlphaMode, Palette};
use photocraft_algo::quantize::{self, BitmapMethod, Dither, Forced, HalftoneShape, PaletteKind};
use photocraft_algo::transform::{Homography, Interp};

use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::adjust::CurvePoint;
use photocraft_doc::{ColorIndices, ColorTable, Document, Duotone, DuotoneInk, Layer, LayerContent, LayerId, Size};
use photocraft_geom::{Affine, Rect};
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

/// A finite number param, `default` when absent or null; anything else is a bad-params error.
fn finite_num(p: &Value, cmd: &str, key: &str, default: f64) -> Result<f64> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v.as_f64().filter(|f| f.is_finite()).ok_or_else(|| bad(cmd, format!("`{key}` must be a number"))),
    }
}

fn str_or<'a>(p: &'a Value, key: &str, default: &'a str) -> &'a str {
    p.get(key).and_then(Value::as_str).unwrap_or(default)
}

type Enabled = std::result::Result<(), String>;

fn mode_is(s: &Session, modes: &[ColorMode], what: &str) -> Enabled {
    let d = s.active().ok_or("no document open")?;
    if modes.contains(&d.doc.mode) { Ok(()) } else { Err(format!("{what} (the document is {:?})", d.doc.mode)) }
}

/// #14509, measured: in a Bitmap document only Arbitrary is greyed in the Image Rotation
/// submenu - 180, the two 90 turns and both flips stay available - and a scripted call answers
/// this sentence with the document unchanged. The Crop tool's straighten shares
/// [`rotate_arbitrary_with`] but was not measured in Bitmap, so only this spec's gate names it.
const BITMAP_ROTATE_REFUSAL: &str = "Could not complete the command because it does not work with Bitmap images (convert image to Grayscale mode to edit).";

fn rotate_arbitrary_enabled(s: &Session) -> Enabled {
    let d = s.active().ok_or("no document open")?;
    if d.doc.mode == ColorMode::Bitmap {
        return Err(BITMAP_ROTATE_REFUSAL.into());
    }
    Ok(())
}

// ---------- Image Rotation › Arbitrary ----------

/// Canvas size after rotating `size` by `deg` (the canvas grows to fit, as in Photoshop).
pub fn rotated_size(size: Size, deg: f64) -> Size {
    let (s, c) = deg.to_radians().sin_cos();
    let (w, h) = (f64::from(size.width), f64::from(size.height));
    let nw = (w * c.abs() + h * s.abs() - 1e-6).ceil().max(1.0);
    let nh = (w * s.abs() + h * c.abs() - 1e-6).ceil().max(1.0);
    Size::new(nw as u32, nh as u32)
}

pub(crate) fn rotate_arbitrary(s: &mut Session, p: &Value) -> Result<Value> {
    // #14465: with no `interpolation` param the resampling comes from Preferences ›
    // General › Image Interpolation; the preference's own names feed `Interp::parse`, so
    // Nearest Neighbor and Bilinear act as the `nearest`/`bilinear` params do and the
    // *not measured* variants (Bicubic Automatic/Sharper/Smoother, Preserve Details)
    // resample as Bicubic, as today. The param stays for scripts and actions (#7630).
    // The direct callers keep the old default through `rotate_arbitrary_with`: the Crop
    // tool's rotated crop and Straighten pass no param and still rotate Bicubic.
    let mut p = p.as_object().cloned().unwrap_or_default();
    if p.get("interpolation").and_then(Value::as_str).is_none() {
        p.insert("interpolation".into(), json!(s.prefs().general.image_interpolation.name()));
    }
    rotate_arbitrary_with(s, &Value::Object(p), true, true)
}

/// [`rotate_arbitrary`]; `image_rotation` is true on the Image Rotation path and decides two
/// things there: count points stay where they were (#11762) and the selection is dropped, in the
/// same history step, so one undo brings it back (#14507). The Crop tool's straighten turn and the
/// Ruler's Straighten pass false: they move count points with the image and warp the selection.
/// `keep_guides` turns the guides with the canvas (#14508, measured for Image Rotation > Arbitrary)
/// instead of dropping them, as the turned Crop and the Straighten still do (not measured there).
pub(crate) fn rotate_arbitrary_with(s: &mut Session, p: &Value, image_rotation: bool, keep_guides: bool) -> Result<Value> {
    const CMD: &str = "image.rotation.arbitrary";
    // At full precision: an f32 angle can grow the canvas 1 px past what `rotated_size` gives callers.
    let angle = p.get("angle").and_then(Value::as_f64).unwrap_or(0.0);
    if !angle.is_finite() || angle.abs() > 3600.0 {
        return Err(bad(CMD, "angle out of range"));
    }
    let deg = match str_or(p, "direction", "cw") {
        "cw" => angle,
        "ccw" => -angle,
        d => return Err(bad(CMD, format!("direction `{d}` (cw|ccw)"))),
    };
    if deg.rem_euclid(360.0).abs() < 1e-9 {
        return Ok(json!({"width": s.active().map(|d| d.doc.size.width), "height": s.active().map(|d| d.doc.size.height)}));
    }
    use crate::canvas_geom::{self, Turn};
    let turn = match deg.rem_euclid(360.0) {
        90.0 => Some(Turn::Cw90),
        180.0 => Some(Turn::Rotate180),
        270.0 => Some(Turn::Ccw90),
        _ => None,
    };
    if image_rotation
        && let Some(turn) = turn
        && let Some(st) = s.active()
        && !st.doc.has_artboards()
    {
        let a = turn.affine(f64::from(st.doc.size.width), f64::from(st.doc.size.height));
        let half = Affine::translate(0.5, 0.5);
        s.set_canvas_map(a, half.then(&a).then(&Affine::translate(-0.5, -0.5)));
        let size = s.edit("Rotate Canvas", |doc, _| {
            // Clear the selection before the exact turn so it is dropped in the same history step.
            doc.selection = None;
            canvas_geom::keeping_count_points(doc, |doc| canvas_geom::turn_canvas(doc, turn));
            if !keep_guides {
                doc.guides = Default::default();
            }
            Ok(doc.size)
        })?;
        return Ok(json!({"width": size.width, "height": size.height}));
    }
    // The Background stays opaque, whatever the background colour's alpha.
    let bg = [s.tools.background[0], s.tools.background[1], s.tools.background[2], 1.0];
    // Color Sampler points turn with the canvas (the same map the edit applies to the geometry).
    // They are pixel indices, so map the pixel's centre, as the warp samples the pixels.
    {
        let old = s.active().ok_or(EngineError::NoDocument)?.doc.size;
        let new = rotated_size(old, deg);
        let (sn, cs) = deg.to_radians().sin_cos();
        let (cx, cy) = (f64::from(old.width) / 2.0, f64::from(old.height) / 2.0);
        let (nx, ny) = (f64::from(new.width) / 2.0, f64::from(new.height) / 2.0);
        let a = Affine { m: [cs, sn, -sn, cs, nx - (cs * cx - sn * cy), ny - (sn * cx + cs * cy)] };
        s.set_canvas_map(a, Affine::translate(0.5, 0.5).then(&a).then(&Affine::translate(-0.5, -0.5)));
    }
    let size = s.edit("Rotate Canvas", |doc, _| {
        let old = doc.size;
        let new = rotated_size(old, deg);
        let (sn, cs) = deg.to_radians().sin_cos();
        let (cx, cy) = (f64::from(old.width) / 2.0, f64::from(old.height) / 2.0);
        let (nx, ny) = (f64::from(new.width) / 2.0, f64::from(new.height) / 2.0);
        // Clockwise on screen (y down): x' = c·x − s·y, y' = s·x + c·y about the centres.
        let a = Affine { m: [cs, sn, -sn, cs, nx - (cs * cx - sn * cy), ny - (sn * cx + cs * cy)] };
        let h = Homography([a.m[0], a.m[2], a.m[4], a.m[1], a.m[3], a.m[5], 0.0, 0.0, 1.0]);
        let interp = Interp::parse(str_or(p, "interpolation", "bicubic"));
        doc.size = new;
        let canvas = doc.bounds();
        let fmt = doc.pixel_format();
        let dpi = doc.resolution_dpi;
        let background_id = doc.background().map(|l| l.id);
        // Type in a font that isn't installed turns its own pixels, as it can't re-render (#8860).
        crate::canvas_geom::map_missing_font_type(&mut doc.layers, &mut |c| photocraft_algo::transform::warp_surface(c, c.content_bounds(), &h, interp))?;
        for l in doc.layers.iter_mut() {
            let background = Some(l.id) == background_id;
            if background && let Some(surf) = l.surface_mut() {
                // The Background stays a Background: rotated pixels over the background colour.
                let src = surf.content_bounds();
                let rotated = photocraft_algo::transform::warp_surface(surf, src, &h, interp)?;
                let mut base = Surface::new(fmt);
                let fill = photocraft_raster::from_rgba(&fmt, bg);
                // One shared tile per full tile, not a canvas-sized float buffer.
                base.fill_rect(canvas, &fill);
                crate::transform_cmds::composite_over(&mut base, &rotated);
                base.prune();
                *surf = base;
                if let Some(m) = l.mask.as_mut() {
                    m.surface = crate::transform_cmds::warp_gray(&m.surface, &h, interp)?;
                }
            } else {
                // Rotating the whole image moves locked layers too, including inside groups.
                with_released_movement_locks(l, |l| crate::transform_cmds::transform_layer(None, photocraft_doc::Locks::default(), l, &h, Some(a), interp))?;
                // Free Transform moves only linked masks; the whole image turns them all (#9201).
                warp_unlinked_masks(l, &h, interp)?;
            }
            // What Free Transform leaves alone: unlinked vector masks, gradient angles, artboards…
            crate::canvas_geom::transform_layer_geometry(l, &a, false, dpi);
        }
        let guides_before = doc.guides.clone();
        if image_rotation {
            crate::canvas_geom::keeping_count_points(doc, |doc| crate::canvas_geom::transform_doc_marks(doc, &a));
        } else {
            crate::canvas_geom::transform_doc_marks(doc, &a);
        }
        for ch in doc.channels.iter_mut().chain(doc.quick_mask.as_mut()) {
            ch.surface = crate::transform_cmds::warp_gray(&ch.surface, &h, interp)?;
        }
        if image_rotation {
            doc.selection = None;
        } else if let Some(sel) = &doc.selection {
            doc.selection = Some(crate::transform_cmds::warp_gray(sel, &h, Interp::Bilinear)?).filter(|s| !s.content_bounds().is_empty());
        }
        // Type, shapes and smart objects re-render from their new geometry; type in a missing
        // font keeps its turned pixels.
        crate::canvas_geom::refresh(doc, crate::canvas_geom::Refresh::All);
        // #14508: the turn keeps its guides. A right angle goes through transform_doc_marks
        // above, which swaps orientation and position exactly as the reference measures
        // (90° clockwise: a vertical guide at x=100 becomes a horizontal one at y=100). Any
        // other angle follows the measured rule: each guide's edge point (the vertical guide's
        // point on the top edge, the horizontal one's on the left) turns about the old centre
        // while the UNROUNDED turned bounding box sits flush at the new canvas's origin - on a
        // 400x300 canvas turned 30° that lands x=100 at 236.60 and y=50 at 43.30, both within
        // 0.15 px of the reference's 236.49 and 43.19 (the exact rule is not settled, so no
        // further refinement is guessed). The content itself is centred in the ROUNDED canvas,
        // so on a non-right angle the guides hug the unrounded geometry, up to a pixel off the
        // painted edge - as measured. Guide COLOURS for a turned canvas were not measured and
        // stay cleared as today; guides that end up off the new canvas follow the same rule
        // continued, they are kept. The turned Crop and Straighten pass keep_guides=false: not
        // measured there, so they drop guides exactly as they do today.
        if !keep_guides {
            doc.guides = Default::default();
        } else if sn.abs() > 1e-9 && cs.abs() > 1e-9 {
            let (ow, oh) = (f64::from(old.width), f64::from(old.height));
            let gw = ow * cs.abs() + oh * sn.abs();
            let gh = ow * sn.abs() + oh * cs.abs();
            let txf = gw / 2.0 - (cs * cx - sn * cy);
            let tyf = gh / 2.0 - (sn * cx + cs * cy);
            doc.guides.vertical = guides_before.vertical.iter().map(|x| cs * *x + txf).collect();
            doc.guides.horizontal = guides_before.horizontal.iter().map(|y| cs * *y + tyf).collect();
        }
        Ok(new)
    })?;
    Ok(json!({"width": size.width, "height": size.height}))
}

/// Warps the unlinked pixel masks of `l` and its descendants, which [`transform_layer`] leaves in
/// place.
///
/// [`transform_layer`]: crate::transform_cmds::transform_layer
fn warp_unlinked_masks(l: &mut Layer, h: &Homography, interp: Interp) -> Result<()> {
    if let Some(m) = l.mask.as_mut()
        && !m.linked
    {
        m.surface = crate::transform_cmds::warp_gray(&m.surface, h, interp)?;
    }
    if let LayerContent::Group(g) = &mut l.content {
        for c in &mut g.children {
            warp_unlinked_masks(c, h, interp)?;
        }
    }
    Ok(())
}

/// Runs `f` with movement locks (`position` and `all`) temporarily released on `l` and all its
/// descendants, so canvas-wide rotation moves every layer, then restores each lock.
fn with_released_movement_locks<R>(l: &mut Layer, f: impl FnOnce(&mut Layer) -> Result<R>) -> Result<R> {
    let mut saved = Vec::new();
    crate::image_cmds::for_each_layer(std::slice::from_mut(l), &mut |x| {
        saved.push(x.locks);
        x.locks.position = false;
        x.locks.all = false;
    });
    let r = f(l);
    let mut saved = saved.into_iter();
    crate::image_cmds::for_each_layer(std::slice::from_mut(l), &mut |x| {
        if let Some(locks) = saved.next() {
            x.locks = locks;
        }
    });
    r
}

// ---------- flatten helpers ----------

/// The flattened image as straight RGBA (over white when `opaque`).
fn composite(doc: &Document, opaque: bool) -> Vec<[f32; 4]> {
    let buf = photocraft_compose::flatten(doc);
    if opaque { buf.over_background([1.0, 1.0, 1.0]).into_px() } else { buf.into_px() }
}

/// Replace the document's layers by one 8-bit layer holding `px` in `mode`'s storage format.
fn single_layer(doc: &mut Document, active: &mut Option<LayerId>, mode: ColorMode, px: &[[f32; 4]], name: &str, background: bool) {
    let fmt = storage_format(mode);
    // Reuse one format-sized pixel buffer instead of allocating a Vec for every pixel.
    let mut pixel = vec![0.0; fmt.channels()];
    let mut data = Vec::new();
    for q in px {
        photocraft_raster::from_rgba_into(&fmt, *q, &mut pixel);
        data.extend_from_slice(&pixel);
    }
    let mut surf = Surface::new(fmt);
    surf.write_region(doc.bounds(), &data);
    single_layer_surface(doc, active, mode, surf, name, background);
}

/// The pixel format of an 8-bit document in `mode`.
fn storage_format(mode: ColorMode) -> PixelFormat {
    Document::pixel_format_for(mode, SampleType::U8)
}

/// [`single_layer`] with the layer's pixels already in a surface of [`storage_format`]`(mode)`.
fn single_layer_surface(doc: &mut Document, active: &mut Option<LayerId>, mode: ColorMode, pixels: Surface, name: &str, background: bool) {
    doc.mode = mode;
    doc.depth = SampleType::U8;
    let mut l = Layer::raster(name, doc.pixel_format());
    if background {
        l.locks.transparency = true;
        l.locks.position = true;
    }
    // A new raster layer always has pixels.
    if let Some(surf) = l.surface_mut() {
        *surf = pixels;
    }
    *active = Some(l.id);
    doc.layers = vec![l].into();
    for ch in doc.channels.iter_mut() {
        let f = PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false);
        ch.surface = ch.surface.convert(PixelFormat { alpha: ch.surface.format().alpha, ..f });
    }
    doc.quick_mask = None;
    doc.color_table = None;
    doc.color_indices = None;
    doc.duotone = None;
}

// ---------- Indexed Color / Color Table ----------

fn parse_forced(s: &str) -> Option<Forced> {
    Some(match s {
        "none" => Forced::None,
        "blackWhite" | "blackAndWhite" => Forced::BlackWhite,
        "primaries" => Forced::Primaries,
        "web" => Forced::Web,
        _ => return None,
    })
}

fn parse_dither(s: &str) -> Option<quantize::Dither> {
    Some(match s {
        "none" => quantize::Dither::None,
        "diffusion" => quantize::Dither::Diffusion,
        "pattern" => quantize::Dither::Pattern,
        "noise" => quantize::Dither::Noise,
        _ => return None,
    })
}

/// Convert a tool colour from the working RGB space into the samples used by the
/// flattened document. RGB and Indexed documents keep their own document profile;
/// grayscale output is a scalar sample replicated into the flattened RGB view.
fn working_matte_color(s: &Session, doc: &Document, color: [f32; 4]) -> Result<[f32; 3]> {
    let destination = match crate::color_cmds::mode_space(doc.mode) {
        ColorSpace::Rgb | ColorSpace::Gray => crate::color_cmds::document_profile(doc),
        // CMYK and Lab flatten through the compositor's sRGB view on the pinned main path.
        _ => crate::color_cmds::composite_profile(doc),
    };
    let source = s.color.working(ColorMode::Rgb);
    let options = TransformOptions { intent: s.color.settings.intent(), bpc: s.color.settings.bpc, ..Default::default() };
    let transform = craft_cms::transform::cached(&source, &destination, options)
        .map_err(|e| bad("image.mode.indexedColor", format!("cannot convert matte colour: {e}")))?;
    let mut output = [0.0; 16];
    transform.eval(&color[..3], &mut output).map_err(|e| bad("image.mode.indexedColor", format!("cannot convert matte colour: {e}")))?;
    let matte = if destination.color_space == ColorSpace::Gray { [output[0]; 3] } else { [output[0], output[1], output[2]] };
    if matte.iter().any(|v| !v.is_finite()) {
        return Err(bad("image.mode.indexedColor", "cannot convert matte colour to finite document samples"));
    }
    Ok(matte.map(|v| v.clamp(0.0, 1.0)))
}

fn indexed_matte(s: &Session, p: &Value) -> Result<Option<[f32; 3]>> {
    const CMD: &str = "image.mode.indexedColor";
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    let matte = match p.get("matte") {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::String(value)) => match value.as_str() {
            "none" => return Ok(None),
            "white" => [1.0; 3],
            "black" => [0.0; 3],
            "gray" => [128.0 / 255.0; 3],
            "netscapeGray" => [192.0 / 255.0; 3],
            "foreground" => working_matte_color(s, doc, s.tools.foreground)?,
            "background" => working_matte_color(s, doc, s.tools.background)?,
            value if value.len() == 7 && value.starts_with('#') => p
                .get("matte")
                .and_then(parse_color)
                .ok_or_else(|| bad(CMD, "matte must be none, foreground, background, white, black, gray, netscapeGray or #rrggbb"))?
                .map(|v| f32::from(v) / 255.0),
            _ => return Err(bad(CMD, "matte must be none, foreground, background, white, black, gray, netscapeGray or #rrggbb")),
        },
        _ => return Err(bad(CMD, "matte must be a colour string")),
    };
    if matte.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
        return Err(bad(CMD, "matte channels must be finite and in 0..=1"));
    }
    Ok(Some(matte))
}

/// Stock Indexed Color accepts the legacy percentage amount. An explicit malformed value must
/// remain an error instead of silently selecting the stock path.
fn stock_amount(p: &Value, cmd: &str) -> Result<f32> {
    for key in ["amount", "ditherAmount"] {
        if let Some(value) = p.get(key).filter(|value| !value.is_null()) {
            let amount = value.as_f64().filter(|value| value.is_finite()).ok_or_else(|| bad(cmd, format!("{key} must be a finite number from 0 to 100")))?;
            if !(0.0..=100.0).contains(&amount) {
                return Err(bad(cmd, format!("{key} must be a finite number from 0 to 100")));
            }
        }
    }
    let value = p.get("amount").filter(|value| !value.is_null()).or_else(|| p.get("ditherAmount").filter(|value| !value.is_null()));
    let Some(value) = value else {
        return Ok(0.75);
    };
    let amount = value.as_f64().ok_or_else(|| bad(cmd, "amount must be a finite number from 0 to 100"))?;
    Ok((amount as f32 / 100.0).clamp(0.0, 1.0))
}

/// Advanced mapping is opt-in: a serialized mapping, a palette snapshot/catalog entry, or a
/// non-legacy palette-map method. Legacy dither values stay on the stock quantizer.
fn custom_mapping_requested(p: &Value, cmd: &str) -> Result<bool> {
    let object = p.as_object().ok_or_else(|| bad(cmd, "params must be a JSON object"))?;
    let dither = match object.get("dither").filter(|value| !value.is_null()) {
        None => None,
        Some(value) => {
            let dither = value.as_str().ok_or_else(|| bad(cmd, "dither must be none|diffusion|pattern|noise or a palette-map method"))?;
            if parse_dither(dither).is_none() && photocraft_algo::palette_map::Method::from_id(dither).is_none() {
                return Err(bad(cmd, "dither must be none|diffusion|pattern|noise or a palette-map method"));
            }
            Some(dither)
        }
    };
    if ["mapping", "paletteId", "paletteSnapshot"].into_iter().any(|key| object.get(key).is_some_and(|value| !value.is_null())) {
        return Ok(true);
    }
    Ok(dither.is_some_and(|value| parse_dither(value).is_none()))
}

fn indexed_color(s: &mut Session, raw: &Value) -> Result<Value> {
    let params = if raw.is_null() { json!({}) } else { raw.clone() };
    let p = &params;
    const CMD: &str = "image.mode.indexedColor";
    let transparent_hint = match p.get("transparentIndex") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or_else(|| bad(CMD, "transparentIndex must be a non-negative integer"))?),
    };
    // An explicit reserved slot must use the exclusion-aware renderer; the stock nearest-colour
    // lookup considers every table entry, including that slot.
    let advanced = custom_mapping_requested(p, CMD)? || transparent_hint.is_some();
    let custom_palette = crate::palette_cmds::palette_from_snapshot_for_indexed(p)?;
    let stock_amount_value = stock_amount(p, CMD)?;
    let stock_dither =
        if advanced { None } else { Some(parse_dither(str_or(p, "dither", "diffusion")).ok_or_else(|| bad(CMD, "dither: none|diffusion|pattern|noise"))?) };
    let stock_amount = (!advanced).then_some(stock_amount_value);
    let settings = if advanced {
        Some(crate::palette_cmds::mapping_settings(p, "amount").map_err(|e| match e {
            EngineError::BadParams { msg, .. } => bad(CMD, msg),
            other => other,
        })?)
    } else {
        None
    };
    let reduce = match p.get("reducePalette") {
        None => false,
        Some(Value::Bool(v)) => *v,
        Some(_) => return Err(bad(CMD, "reducePalette must be a boolean")),
    };
    let transparency = match p.get("transparency") {
        None => true,
        Some(Value::Bool(v)) => *v,
        Some(_) => return Err(bad(CMD, "transparency must be a boolean")),
    };
    if !transparency && transparent_hint.is_some() {
        return Err(bad(CMD, "transparentIndex requires transparency=true"));
    }
    let colors = match p.get("colors") {
        None => 256,
        Some(value) => {
            let value = value.as_f64().ok_or_else(|| bad(CMD, "colors must be an integer from 2 to 256"))?;
            if !value.is_finite() || value.fract() != 0.0 || !(2.0..=256.0).contains(&value) {
                return Err(bad(CMD, "colors must be an integer from 2 to 256"));
            }
            value as usize
        }
    };
    let matte = indexed_matte(s, p)?;
    let (kind, forced) = if custom_palette.is_none() {
        let kind = PaletteKind::from_id(str_or(p, "palette", "selective"))
            .ok_or_else(|| bad(CMD, "palette: blackWhite|exact|systemMac|systemWindows|web|uniform|perceptual|selective|adaptive"))?;
        let forced = parse_forced(str_or(p, "forced", "blackWhite")).ok_or_else(|| bad(CMD, "forced: none|blackWhite|primaries|web"))?;
        (Some(kind), Some(forced))
    } else {
        // A fixed palette is authoritative: generated-palette and forced-colour controls do not
        // alter its order or entries.
        (None, None)
    };
    let generated_black_white = custom_palette.is_none() && kind == Some(PaletteKind::BlackWhite);
    if let Some(custom) = &custom_palette {
        if let Some(index) = transparent_hint
            && index >= custom.colors.len()
        {
            return Err(bad(CMD, format!("transparentIndex {index} is outside the {}-entry palette", custom.colors.len())));
        }
        if custom.colors.len() > 256 && !reduce {
            return Err(bad(
                CMD,
                format!(
                    "fixed palette has {} entries; Indexed Color supports at most 256 (including transparency). Enable reducePalette for deliberate reduction",
                    custom.colors.len()
                ),
            ));
        }
    }
    // Advanced palette-map comparisons use encoded sRGB, but stock Indexed Color stays in the
    // document's encoded RGB coordinates and keeps its original profile.
    let mapping_options = TransformOptions { intent: s.color.settings.intent(), bpc: s.color.settings.bpc, ..TransformOptions::default() };
    let (pal_len, transparent, palette_snapshot) = s.edit("Indexed Color", |doc, active| {
        let mut px = composite(doc, !transparency && matte.is_none());
        if let Some(matte) = matte {
            for pixel in &mut px {
                let alpha = pixel[3].clamp(0.0, 1.0);
                if alpha == 1.0 {
                    continue;
                }
                if !transparency || alpha > 0.0 {
                    for (channel, background) in pixel[..3].iter_mut().zip(matte) {
                        *channel = *channel * alpha + background * (1.0 - alpha);
                    }
                    pixel[3] = 1.0;
                }
            }
        }
        let source_profile = crate::color_cmds::document_profile(doc);
        let srgb = Builtin::Srgb.profile();
        // Advanced palette-map comparisons are encoded sRGB. A grayscale document's pixels are
        // gray-profile samples, so they reach that comparison through the compositor's RGB view
        // of the profile (`composite_profile`), not as if gray samples were sRGB.
        let comparison_profile = crate::color_cmds::composite_profile(doc);
        let advanced_cms = advanced && !comparison_profile.same_bytes(srgb);
        if advanced_cms {
            let transform = craft_cms::transform::cached(&comparison_profile, srgb, mapping_options)
                .map_err(|e| bad(CMD, format!("could not convert advanced Indexed mapping to sRGB: {e}")))?;
            let mut converted = [0.0f32; 16];
            for pixel in &mut px {
                transform
                    .eval(&pixel[..3], &mut converted)
                    .map_err(|e| bad(CMD, format!("could not convert advanced Indexed mapping to sRGB: {e}")))?;
                pixel[..3].copy_from_slice(&converted[..3]);
            }
        }
        let mut mapping = settings.clone().unwrap_or_default();
        let custom = custom_palette.clone();
        let mut palette = if let Some(custom) = custom {
            custom
        } else {
            let has_clear = transparency
                && px.iter().any(|q| match mapping.alpha {
                    AlphaMode::Dither => q[3] < 1.0,
                    _ => q[3] < 0.5,
                });
            let requested = if generated_black_white {
                // BlackWhite is a fixed two-opaque-entry preset. Its palette count is intentionally
                // independent of the Indexed Color total-count field so transparency can add a
                // third slot without reducing either required entry.
                2
            } else if has_clear {
                colors.saturating_sub(1).max(2)
            } else {
                colors
            };
            let kind = kind.ok_or_else(|| bad(CMD, "generated palette kind is missing"))?;
            let forced = forced.ok_or_else(|| bad(CMD, "generated forced-colour mode is missing"))?;
            let pal = quantize::build_palette(&px, kind, requested, forced).map_err(|e| bad(CMD, e))?;
            // Exact must reserve one palette slot for clear pixels: a built 256-entry opaque
            // palette plus a transparent entry is one too many. Reject it here, before any
            // reduction below can silently drop a colour and commit what Exact cannot represent.
            // The rule is stock-only; advanced callers supply their own mapping.
            if !advanced && kind == PaletteKind::Exact && has_clear && pal.len() >= 256 {
                return Err(bad(CMD, "Exact with transparency needs 255 or fewer opaque palette colours plus one transparent entry"));
            }
            let mut generated = Palette { name: str_or(p, "palette", "selective").to_string(), colors: pal.into_iter().map(|c| c.map(|v| f32::from(v) / 255.0)).collect() };
            // The quantizer requires two samples, but a two-entry Indexed image with a
            // transparency slot has capacity for exactly one opaque colour. Reduce that
            // generated result algorithmically instead of silently exceeding `colors`; the
            // explicit BlackWhite preset is the intentional two-opaque-entry exception.
            if !generated_black_white && (advanced || kind != PaletteKind::Exact) && has_clear && colors == 2 {
                generated = crate::palette_cmds::reduce_palette(&generated, 1).map_err(|e| match e {
                    EngineError::BadParams { msg, .. } => bad(CMD, msg),
                    other => other,
                })?;
            }
            generated
        };
        let mut transparent = transparent_hint;
        if custom_palette.is_some() {
            let (mapped, slot) = crate::palette_cmds::indexed_palette(palette, reduce, transparent).map_err(|e| match e {
                EngineError::BadParams { msg, .. } => bad(CMD, msg),
                other => other,
            })?;
            palette = mapped;
            transparent = slot;
        } else if let Some(index) = transparent
            && index >= palette.colors.len()
        {
            return Err(bad(CMD, format!("transparentIndex {index} is outside the {}-entry palette", palette.colors.len())));
        }
        let needs_transparency = transparency
            && transparent.is_none()
            && px.iter().any(|q| match mapping.alpha {
                AlphaMode::Dither => q[3] < 1.0,
                _ => q[3] < 0.5,
            });
        if needs_transparency {
            let room = if custom_palette.is_some() { 255 } else { colors.saturating_sub(1).max(1) };
            // Stock Exact never reduces: it either fits beside the transparency entry or the
            // 256-entry rule above rejected it. Fixed palettes keep their explicit reduction.
            if !generated_black_white && (advanced || kind != Some(PaletteKind::Exact)) && palette.colors.len() > room {
                if custom_palette.is_some() && !reduce {
                    return Err(bad(CMD, format!("the fixed palette has {} opaque entries but Indexed Color allows {room} with a transparency slot; enable reducePalette for deliberate reduction", palette.colors.len())));
                }
                palette = crate::palette_cmds::reduce_palette(&palette, room).map_err(|e| match e {
                    EngineError::BadParams { msg, .. } => bad(CMD, msg),
                    other => other,
                })?;
            }
        }
        // Replayable snapshots contain the final opaque palette, never our reserved alpha slot.
        let mut snapshot = palette.clone();
        // Snapshots are a palette-map input, so they are encoded sRGB. Stock palettes are built
        // in the document's encoded RGB coordinates; convert only this copy, leaving the document
        // colour table and indexed pixel output in the document profile. Untagged and sRGB
        // documents are already sRGB.
        if !advanced && doc.icc_profile.is_some() && source_profile.color_space == ColorSpace::Rgb && !source_profile.same_bytes(srgb) {
            let transform = craft_cms::transform::cached(&source_profile, srgb, mapping_options)
                .map_err(|e| bad(CMD, format!("could not convert Indexed palette snapshot to sRGB: {e}")))?;
            let mut converted = [0.0f32; 16];
            for color in &mut snapshot.colors {
                transform
                    .eval(&color[..], &mut converted)
                    .map_err(|e| bad(CMD, format!("could not convert Indexed palette snapshot to sRGB: {e}")))?;
                // A snapshot is a palette-map input: keep entries in the 0..=1 range that replay
                // validation requires.
                color.copy_from_slice(&[converted[0].clamp(0.0, 1.0), converted[1].clamp(0.0, 1.0), converted[2].clamp(0.0, 1.0)]);
            }
        }
        if needs_transparency {
            transparent = if advanced {
                crate::palette_cmds::append_transparency(&mut palette)
            } else if palette.colors.len() < 256 {
                palette.colors.push([1.0; 3]);
                Some(palette.colors.len() - 1)
            } else {
                None
            };
        }
        let indices: Vec<u8> = if advanced {
            crate::palette_cmds::alpha_for_indexed(&mut mapping, transparent);
            let w = doc.size.width as usize;
            crate::palette_cmds::indexed_render(&mut px, w, &palette, &mapping, transparent)?.into_iter().map(|index| index as u8).collect()
        } else {
            let dither = stock_dither.ok_or_else(|| bad(CMD, "stock dither mode is missing"))?;
            let amount = stock_amount.unwrap_or(0.75);
            let pal: Vec<[u8; 3]> = palette.colors.iter().map(|c| c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)).collect();
            // The appended transparency entry is a reserved slot, not an opaque colour: an opaque
            // pixel must match the opaque entries only. Clear pixels still take `transparent`.
            let opaque = transparent.map_or(pal.len(), |index| index.min(pal.len()));
            quantize::quantize(&mut px, doc.size.width as usize, &pal[..opaque], dither, amount, transparent)
        };
        if advanced_cms {
            // The Indexed output returns to the document's own profile. A gray profile has one
            // output channel; the expanded Indexed pixels replicate that gray into RGB.
            let gray_output = source_profile.color_space == ColorSpace::Gray;
            let transform = craft_cms::transform::cached(srgb, &source_profile, mapping_options)
                .map_err(|e| bad(CMD, format!("could not convert advanced Indexed mapping to document RGB: {e}")))?;
            let mut converted = [0.0f32; 16];
            for pixel in &mut px {
                transform
                    .eval(&pixel[..3], &mut converted)
                    .map_err(|e| bad(CMD, format!("could not convert advanced Indexed mapping to document RGB: {e}")))?;
                if gray_output {
                    let gray = converted[0];
                    pixel[..3].copy_from_slice(&[gray; 3]);
                } else {
                    pixel[..3].copy_from_slice(&converted[..3]);
                }
            }
            for color in &mut palette.colors {
                transform
                    .eval(&color[..], &mut converted)
                    .map_err(|e| bad(CMD, format!("could not convert advanced Indexed palette to document RGB: {e}")))?;
                if gray_output {
                    let gray = converted[0];
                    color.copy_from_slice(&[gray; 3]);
                } else {
                    color.copy_from_slice(&converted[..3]);
                }
            }
        }
        single_layer(doc, active, ColorMode::Indexed, &px, if transparent.is_some() { "Index" } else { "Background" }, transparent.is_none());

        let mut table = crate::palette_cmds::palette_to_color_table(&palette);
        table.transparent = transparent.map(|index| index as u8);
        doc.color_table = Some(table);
        let bounds = doc.bounds();
        doc.color_indices = doc.layers.first().and_then(|l| ColorIndices::new(l.id, l.surface()?, bounds, &indices));
        Ok((palette.colors.len(), transparent, snapshot))
    })?;
    Ok(json!({"colors": pal_len, "transparentIndex": transparent, "paletteSnapshot": palette_snapshot}))
}

fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

fn parse_color(v: &Value) -> Option<[u8; 3]> {
    match v {
        Value::String(s) => {
            let s = s.trim_start_matches('#');
            let b = |i: usize| u8::from_str_radix(s.get(i..i + 2)?, 16).ok();
            Some([b(0)?, b(2)?, b(4)?])
        }
        Value::Array(a) if a.len() >= 3 => {
            let c = |i: usize| a[i].as_f64().map(|x| x.round().clamp(0.0, 255.0) as u8);
            Some([c(0)?, c(1)?, c(2)?])
        }
        _ => None,
    }
}

/// Built-in Color Table presets.
pub fn preset_table(name: &str) -> Option<Vec<[u8; 3]>> {
    let ramp = |f: &dyn Fn(f32) -> [f32; 3]| (0..256).map(|i| f(i as f32 / 255.0).map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)).collect();
    Some(match name {
        "grayscale" => ramp(&|t| [t; 3]),
        // Black → red → yellow → white, the incandescence scale.
        "blackBody" => ramp(&|t| [(t * 3.0).min(1.0), (t * 3.0 - 1.0).clamp(0.0, 1.0), (t * 3.0 - 2.0).clamp(0.0, 1.0)]),
        "spectrum" => ramp(&|t| {
            let h = t * 300.0 / 60.0;
            let x = 1.0 - (h % 2.0 - 1.0).abs();
            match h as i32 {
                0 => [1.0, x, 0.0],
                1 => [x, 1.0, 0.0],
                2 => [0.0, 1.0, x],
                3 => [0.0, x, 1.0],
                _ => [x, 0.0, 1.0],
            }
        }),
        "systemMac" | "system" => quantize::mac_palette(),
        "systemWindows" => quantize::windows_palette(),
        "web" => quantize::web_palette(),
        _ => return None,
    })
}

/// Image › Mode › Color Table: replace entries (pixels keep their index, so their colour follows
/// the table, as in Photoshop) and set the transparent entry. No changes: report the table.
fn color_table(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "image.mode.colorTable";
    let cur = s.active().ok_or(EngineError::NoDocument)?.doc.color_table.clone().ok_or_else(|| EngineError::Other("the document has no color table".into()))?;
    let mut table = cur.clone();
    match str_or(p, "table", "custom") {
        "custom" => {}
        name => table.colors = preset_table(name).ok_or_else(|| bad(CMD, format!("unknown table `{name}`")))?,
    }
    if let Some(a) = p.get("colors").and_then(Value::as_array) {
        let c: Option<Vec<[u8; 3]>> = a.iter().map(parse_color).collect();
        let c = c.ok_or_else(|| bad(CMD, "colors: [\"#rrggbb\" | [r, g, b], …]"))?;
        if c.is_empty() || c.len() > 256 {
            return Err(bad(CMD, "a color table has 1 to 256 entries"));
        }
        table.colors = c;
    }
    if let Some(e) = p.get("entries").and_then(Value::as_object) {
        for (k, v) in e {
            let i: usize = k.parse().map_err(|_| bad(CMD, format!("entry index `{k}`")))?;
            let c = parse_color(v).ok_or_else(|| bad(CMD, format!("entry {i}: bad colour")))?;
            if i >= table.colors.len() {
                return Err(bad(CMD, format!("entry {i} is past the table ({} entries)", table.colors.len())));
            }
            table.colors[i] = c;
        }
    }
    match p.get("transparent") {
        Some(Value::Null) => table.transparent = None,
        Some(v) => {
            let len = table.colors.len();
            let i =
                v.as_u64().filter(|&i| i < len as u64).ok_or_else(|| bad(CMD, format!("transparent: an index from 0 to {} or null", len.saturating_sub(1))))?;
            table.transparent = u8::try_from(i).ok();
        }
        // A shrunk table can't keep a transparent entry past its end (#4186).
        None => table.transparent = table.transparent.filter(|&i| usize::from(i) < table.colors.len()),
    }
    let report = |t: &ColorTable| json!({"colors": t.colors.iter().map(|c| hex(*c)).collect::<Vec<_>>(), "transparent": t.transparent});
    if table == cur {
        return Ok(report(&cur));
    }
    let out = report(&table);
    s.edit("Color Table", |doc, _| {
        let old = cur.clone();
        // A pixel keeps its index where it is known, so entries of one colour keep their own
        // pixels; elsewhere it takes the nearest entry (#14342). The indices written here are
        // known afterwards, for the recorded layer (Indexed documents have one).
        let known = doc.color_indices.take();
        let has_pixels = |id: LayerId| doc.layers.iter().any(|l| l.id == id && l.surface().is_some());
        let record = known.as_ref().map(|k| k.layer).filter(|&id| has_pixels(id)).or_else(|| doc.layers.iter().find(|l| l.surface().is_some()).map(|l| l.id));
        for l in doc.layers.iter_mut() {
            let id = l.id;
            let Some(surf) = l.surface_mut() else { continue };
            let r = surf.content_bounds();
            if r.is_empty() {
                continue;
            }
            let fmt = surf.format();
            let n = fmt.channels();
            let mut data = surf.read_region(r);
            let indices = known.as_ref().and_then(|k| k.known(id, surf, r)).unwrap_or_default();
            let mut written = vec![0u8; if record == Some(id) { data.len() / n } else { 0 }];
            for (k, q) in data.chunks_exact_mut(n).enumerate() {
                let c = photocraft_raster::to_rgba(&fmt, q);
                let rgb = [c[0], c[1], c[2]];
                let i = if c[3] <= 0.0 {
                    let Some(i) = old.transparent.map(usize::from) else { continue };
                    if i >= old.colors.len() || old.rgb(i) != rgb {
                        continue;
                    }
                    i
                } else {
                    // A known index past the table stays: its pixel shows black until the table
                    // grows back over it.
                    indices.get(k).map_or_else(|| old.nearest(rgb), |&i| usize::from(i))
                };
                if let Some(w) = written.get_mut(k) {
                    *w = i as u8;
                }
                let alpha = if table.transparent == Some(i as u8) {
                    0.0
                } else if c[3] <= 0.0 {
                    1.0
                } else {
                    c[3]
                };
                // An index past a shrunk table has no colour: black, as in Photoshop (#14342).
                let nc = table.rgb(i);
                let mut enc = [0.0f32; 8];
                let m = photocraft_raster::from_rgba_into(&fmt, [nc[0], nc[1], nc[2], alpha], &mut enc);
                q.copy_from_slice(&enc[..m]);
            }
            surf.write_region(r, &data);
            surf.prune();
            if record == Some(id) {
                doc.color_indices = ColorIndices::new(id, surf, r, &written);
            }
        }
        doc.color_table = Some(table.clone());
        Ok(())
    })?;
    Ok(out)
}

/// The colour table painting on `p`'s target must stay inside: an Indexed Color document's,
/// when the target is layer pixels (masks and channels are gray, not indexed).
pub(crate) fn paint_color_table(doc: &Document, p: &Value) -> Option<ColorTable> {
    let pixels = crate::channel_cmds::target_of(p) == crate::channel_cmds::Target::Pixels;
    doc.color_table.clone().filter(|_| pixels && doc.mode == ColorMode::Indexed)
}

/// The colour `c` paints as in a document with `table` (see [`keep_in_color_table`]).
pub(crate) fn table_color(table: &ColorTable, c: [f32; 4]) -> [f32; 4] {
    let mut px = [c];
    if !table.colors.is_empty() {
        quantize::quantize(&mut px, 1, &table.colors, Dither::None, 0.0, table.transparent.map(usize::from));
    }
    let [c] = px;
    c
}

/// Map the pixels of `r` that the stroke changed (they differ from `pre`) to their nearest
/// colour-table entry, as the Indexed Color conversion does with no dither, so painting never
/// leaves the table (#8669). One tile at a time: a long stroke's bounds can span the canvas.
pub(crate) fn keep_in_color_table(table: &ColorTable, pre: &Surface, surf: &mut Surface, r: Rect) {
    if table.colors.is_empty() {
        return;
    }
    let fmt = surf.format();
    let n = fmt.channels();
    let mut enc = [0.0f32; 8];
    for tc in r.tiles() {
        let t = tc.rect().intersect(&r);
        if t.is_empty() || !surf.has_tiles_in(t) {
            continue;
        }
        let (before, mut data) = (pre.read_region(t), surf.read_region(t));
        let (mut changed, mut px) = (Vec::new(), Vec::new());
        for (i, (q, b)) in data.chunks_exact(n).zip(before.chunks_exact(n)).enumerate() {
            if q != b {
                changed.push(i);
                px.push(photocraft_raster::to_rgba(&fmt, q));
            }
        }
        if px.is_empty() {
            continue;
        }
        let w = px.len();
        quantize::quantize(&mut px, w, &table.colors, Dither::None, 0.0, table.transparent.map(usize::from));
        let mut mapped = changed.into_iter().zip(px).peekable();
        for (i, q) in data.chunks_exact_mut(n).enumerate() {
            if let Some((_, c)) = mapped.next_if(|(j, _)| *j == i) {
                photocraft_raster::from_rgba_into(&fmt, c, &mut enc);
                q.iter_mut().zip(enc).for_each(|(d, v)| *d = v);
            }
        }
        surf.write_region(t, &data);
    }
}

// ---------- Bitmap ----------

/// Bitmap's Output resolution range (ppi): what a new document may have.
const BITMAP_RESOLUTION: std::ops::RangeInclusive<f64> = 1.0..=crate::document_preset_cmds::MAX_RESOLUTION;

/// Bytes per pixel the Bitmap conversion holds besides the document: its 8-bit gray + alpha
/// result. The composite is converted in bands of [`photocraft_compose::BAND_PIXELS`] (#8477).
const BITMAP_WORK_BYTES_PER_PX: u64 = 2;

/// Bitmap's Custom Pattern: a pattern's threshold map (see
/// [`quantize::custom_pattern_thresholds`]) with its width and height.
fn custom_pattern_map(s: &Session, cmd: &str, p: &Value) -> Result<(Vec<f32>, usize, usize)> {
    if p.get("pattern").is_some_and(|v| !v.is_string() && !v.is_null()) {
        return Err(bad(cmd, "`pattern` must be a pattern id or name"));
    }
    let pat = crate::pattern_cmds::resolve_param(s, cmd, p)?;
    // A pattern keeps its own colours: its lightness is what it paints in a Grayscale document,
    // over white where it is transparent.
    if !pat.is_paintable() {
        return Err(bad(cmd, format!("the pattern \"{}\" is empty or too large", pat.display_name())));
    }
    let count = usize::try_from(pat.pixel_count()).map_err(|_| bad(cmd, "the pattern is too large"))?;
    quantize::custom_pattern_working_bytes(count).map_err(|_| bad(cmd, "the pattern's threshold ranking exceeds the 256 MiB workspace limit"))?;
    let mut lightness = Vec::new();
    lightness.try_reserve_exact(count).map_err(|_| bad(cmd, "not enough memory for the pattern's lightness map"))?;
    let mut band = Vec::new();
    for top in (0..pat.height).step_by(photocraft_doc::pattern::BAND_ROWS as usize) {
        photocraft_compose::pattern::pattern_rgba_rows(&pat, ColorMode::Grayscale, top, top + photocraft_doc::pattern::BAND_ROWS, &mut band)
            .ok_or_else(|| bad(cmd, "not enough memory for the pattern's pixel rows"))?;
        lightness.extend(band.iter().map(|q| photocraft_color::convert::rgb_to_gray([q[0], q[1], q[2]]) * q[3] + (1.0 - q[3])));
    }
    // Rank the complete pattern once: band-local ranks would change its thresholds.
    drop(band);
    let map = quantize::custom_pattern_thresholds(&lightness).map_err(|error| bad(cmd, format!("could not rank the pattern: {error}")))?;
    Ok((map, pat.width as usize, pat.height as usize))
}

fn bitmap(s: &mut Session, p: &Value) -> Result<Value> {
    bitmap_in_bands(s, p, 0)
}

/// Convert with the requested compositor band height (zero chooses its normal budget).
fn bitmap_in_bands(s: &mut Session, p: &Value, band_rows: i32) -> Result<Value> {
    const CMD: &str = "image.mode.bitmap";
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let input = f64::from(d.doc.resolution_dpi.max(1.0));
    let (ow, oh) = (d.doc.size.width, d.doc.size.height);
    // Output resolution (ppi): Photoshop resamples the image to it, keeping the print size,
    // before converting, and the document takes it as its resolution.
    let output = finite_num(p, CMD, "resolution", input)?;
    if !BITMAP_RESOLUTION.contains(&output) {
        return Err(bad(CMD, format!("resolution {output} is outside {}..{} pixels/inch", BITMAP_RESOLUTION.start(), BITMAP_RESOLUTION.end())));
    }
    let (nw, nh) = if (output - input).abs() < 1e-6 {
        (ow, oh)
    } else {
        // Keeping the print size can ask for more pixels than a document may have: refuse that
        // rather than clamp, which would change the print size.
        let max = crate::document_preset_cmds::MAX_DIMENSION;
        let side = |v: u32| {
            let n = (f64::from(v) * output / input).round().max(1.0);
            if n > f64::from(max) {
                return Err(bad(CMD, format!("{n} px across at {output} pixels/inch is more than {max} px; choose a lower Output resolution")));
            }
            Ok(n as u32)
        };
        (side(ow)?, side(oh)?)
    };
    // Besides the (resampled) document, the conversion holds its result at the new size: budget
    // both before allocating.
    crate::image_cmds::check_working_budget(CMD, &d.doc, nw, nh, BITMAP_WORK_BYTES_PER_PX, ", or a lower Output resolution")?;
    let dpi = output as f32;
    let mut custom = None;
    let method = match str_or(p, "method", "diffusion") {
        "threshold" | "threshold50" => BitmapMethod::Threshold,
        "pattern" => BitmapMethod::Pattern,
        "diffusion" => BitmapMethod::Diffusion,
        "halftone" => {
            // Photoshop's Halftone Screen: frequency 1..999 lines/inch, angle -180..180 degrees.
            let freq = finite_num(p, CMD, "frequency", 53.0)?.clamp(1.0, 999.0) as f32;
            let angle = finite_num(p, CMD, "angle", 45.0)?;
            if !(-180.0..=180.0).contains(&angle) {
                return Err(bad(CMD, format!("angle {angle} is outside -180..180")));
            }
            let shape = HalftoneShape::from_id(str_or(p, "shape", "round")).ok_or_else(|| bad(CMD, "shape: round|ellipse|line|square|diamond|cross"))?;
            // The screen is laid out at the output resolution.
            BitmapMethod::Halftone { cell: (dpi / freq).max(2.0), angle: angle as f32, shape }
        }
        "customPattern" => {
            custom = Some(custom_pattern_map(s, CMD, p)?);
            BitmapMethod::Threshold
        }
        m => {
            return Err(bad(CMD, format!("method `{m}` (threshold|pattern|diffusion|halftone|customPattern)")));
        }
    };
    if (nw, nh) != (ow, oh) {
        // Color Sampler points scale with the pixels, as in Image Size.
        let (sx, sy) = (f64::from(nw) / f64::from(ow.max(1)), f64::from(nh) / f64::from(oh.max(1)));
        s.set_sampler_map(photocraft_geom::Affine { m: [sx, 0.0, 0.0, sy, 0.0, 0.0] });
    }
    s.edit("Bitmap", |doc, active| {
        if (nw, nh) != (doc.size.width, doc.size.height) {
            crate::image_cmds::resample_doc(doc, CMD, nw, nh, photocraft_algo::resample::Resample::Bicubic)?;
        }
        doc.resolution_dpi = dpi;
        let w = doc.size.width as usize;
        let mut strips = match &custom {
            Some((map, mw, mh)) => quantize::BitmapStrips::custom(w, map, *mw, *mh),
            None => quantize::BitmapStrips::new(w, method),
        };
        // Band by band, top to bottom: a full-canvas f32 composite of a 30000 px square image is
        // 14 GB, and allocating it (and its copies) aborted the app (#8477).
        let fmt = storage_format(ColorMode::Bitmap);
        let n = fmt.channels();
        let mut out = Surface::new(fmt);
        let (mut gray, mut data) = (Vec::new(), Vec::new());
        let _ = photocraft_compose::render_bands(doc, doc.bounds(), band_rows, |band| -> std::result::Result<(), std::convert::Infallible> {
            gray.clear();
            gray.extend(band.px.iter().map(|q| {
                // Over white, as the flattened image.
                let a = q[3];
                photocraft_color::convert::rgb_to_gray([q[0] * a + (1.0 - a), q[1] * a + (1.0 - a), q[2] * a + (1.0 - a)])
            }));
            strips.convert(&mut gray);
            data.resize(gray.len() * n, 0.0);
            for (g, px) in gray.iter().zip(data.chunks_exact_mut(n)) {
                photocraft_raster::from_rgba_into(&fmt, [*g, *g, *g, 1.0], px);
            }
            out.write_region(band.rect, &data);
            Ok(())
        });
        single_layer_surface(doc, active, ColorMode::Bitmap, out, "Background", true);
        Ok(())
    })?;
    let size = s.active().ok_or(EngineError::NoDocument)?.doc.size;
    Ok(json!({"width": size.width, "height": size.height, "resolution": dpi}))
}

// ---------- Duotone ----------

/// Default inks per type: black first, then warm brown, gold and slate blue (generic names;
/// Photoshop ships Pantone presets, which we don't reproduce).
fn default_inks(n: usize) -> Vec<DuotoneInk> {
    let all = [("Black", [0.0, 0.0, 0.0]), ("Warm Brown", [0.62, 0.38, 0.18]), ("Gold", [0.86, 0.68, 0.18]), ("Slate Blue", [0.27, 0.38, 0.58])];
    all.iter().take(n).map(|(name, c)| DuotoneInk::new(*name, *c)).collect()
}

fn parse_inks(cmd: &str, v: &Value, n: usize) -> Result<Vec<DuotoneInk>> {
    let a = v.as_array().ok_or_else(|| bad(cmd, "inks: [{\"name\",\"color\":\"#rrggbb\",\"curve\":[[in,out],…] (0..100)}, …]"))?;
    let defaults = default_inks(n.max(a.len()).min(4));
    let mut out = Vec::new();
    for (i, ink) in a.iter().take(4).enumerate() {
        let mut d = defaults.get(i).cloned().unwrap_or_else(|| DuotoneInk::new(format!("Ink {}", i + 1), [0.0; 3]));
        if let Some(name) = ink.get("name").and_then(Value::as_str) {
            d.name = name.to_string();
        }
        if let Some(c) = ink.get("color").and_then(parse_color) {
            d.color = c.map(|v| f32::from(v) / 255.0);
        }
        if let Some(pts) = ink.get("curve").and_then(Value::as_array) {
            let mut curve: Vec<CurvePoint> =
                pts.iter().filter_map(|p| Some(CurvePoint { input: p.get(0)?.as_f64()? as f32 / 100.0, output: p.get(1)?.as_f64()? as f32 / 100.0 })).collect();
            // 1e39 is a valid JSON number but inf as f32, and a .pcraft with an inf curve can't be saved.
            if curve.iter().any(|p| !p.input.is_finite() || !p.output.is_finite()) {
                return Err(bad(cmd, "curve points must be finite numbers 0..100"));
            }
            curve.sort_by(|a, b| a.input.total_cmp(&b.input));
            if curve.len() >= 2 {
                d.curve = curve;
            }
        }
        out.push(d);
    }
    Ok(out)
}

fn duotone(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "image.mode.duotone";
    let n = match str_or(p, "type", "duotone") {
        "monotone" => 1,
        "duotone" => 2,
        "tritone" => 3,
        "quadtone" => 4,
        t => {
            return Err(bad(CMD, format!("type `{t}` (monotone|duotone|tritone|quadtone)")));
        }
    };
    let mut inks = match p.get("inks") {
        Some(v) => parse_inks(CMD, v, n)?,
        None => default_inks(n),
    };
    if inks.len() < n {
        inks.extend(default_inks(n).into_iter().skip(inks.len()));
    }
    inks.truncate(n);
    let names: Vec<String> = inks.iter().map(|i| i.name.clone()).collect();
    s.edit("Duotone", |doc, _| {
        doc.mode = ColorMode::Duotone;
        doc.duotone = Some(Duotone { inks, psd_raw: None });
        doc.color_table = None;
        Ok(())
    })?;
    Ok(json!({"inks": names}))
}

macro_rules! spec {
    ($id:literal, $label:literal, [$($m:literal),*], $params:literal, $en:expr, $run:expr) => {
        CommandSpec { id: $id, label: $label, menu: &[$($m),*], shortcut: None, params: $params, enabled: $en, run: $run, journal: true }
    };
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "image.rotation.arbitrary",
            "Arbitrary…",
            ["Image", "Image Rotation"],
            r##"{"angle":-359.99..359.99=0,"direction":"cw|ccw"="cw","interpolation":"bicubic|bilinear|nearest" (absent: Preferences › General › Image Interpolation; its not-measured variants resample as bicubic)}"##,
            rotate_arbitrary_enabled,
            rotate_arbitrary
        ),
        spec!(
            "image.mode.indexedColor",
            "Indexed Color…",
            ["Image", "Mode"],
            r##"{"palette":"selective|perceptual|adaptive|exact|systemMac|systemWindows|web|uniform"="selective","paletteId":string?,"paletteSnapshot":json?,"colors":2..256=256,"forced":"blackWhite|none|primaries|web"="blackWhite","transparency":bool=true,"matte":color,"transparentIndex":integer?,"reducePalette":bool=false,"mapping":json?,"dither":"diffusion|none|pattern|noise"="diffusion","amount":0..100=75} (flattens; matte: none|foreground|background|white|black|gray|netscapeGray|#rrggbb; gray=50% Gray (#808080), netscapeGray=Netscape Gray (#c0c0c0); fixed palette controls override generated palette; automation also accepts palette=blackWhite, always two opaque entries with an optional transparency slot)"##,
            |s| mode_is(
                s,
                &[ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Indexed, ColorMode::Duotone, ColorMode::Cmyk, ColorMode::Lab],
                "Indexed Color needs an RGB or Grayscale image"
            ),
            indexed_color
        ),
        spec!(
            "image.mode.colorTable",
            "Color Table…",
            ["Image", "Mode"],
            r##"{"table":"custom|blackBody|grayscale|spectrum|systemMac|systemWindows|web"="custom","colors":json,"entries":json,"transparent":json} (colors: ["#rrggbb", …]; entries: {"index": "#rrggbb"}; transparent: index|null)"##,
            |s| mode_is(s, &[ColorMode::Indexed], "Color Table needs an Indexed Color image"),
            color_table
        ),
        spec!(
            "image.mode.bitmap",
            "Bitmap…",
            ["Image", "Mode"],
            r##"{"resolution":1..30000=document,"method":"diffusion|threshold|pattern|halftone|customPattern"="diffusion","frequency":1..999=53,"angle":-180..180=45,"shape":"round|ellipse|line|square|diamond|cross"="round","pattern":id|name=Patterns panel selection} (resolution: Output in pixels/inch, resamples first; frequency/angle/shape: Halftone Screen, frequency in lines/inch; pattern: Custom Pattern; flattens)"##,
            |s| mode_is(s, &[ColorMode::Grayscale], "Bitmap needs a Grayscale image (Image › Mode › Grayscale first)"),
            bitmap
        ),
        spec!(
            "image.mode.duotone",
            "Duotone…",
            ["Image", "Mode"],
            r##"{"type":"duotone|monotone|tritone|quadtone"="duotone","inks":json} (inks: [{"name","color":"#rrggbb","curve":[[in,out],…] in 0..100}, …])"##,
            |s| mode_is(s, &[ColorMode::Grayscale, ColorMode::Duotone], "Duotone needs a Grayscale image (Image › Mode › Grayscale first)"),
            duotone
        ),
    ]
}

/// The gradient-map layer that renders Duotone inks over the gray composite.
pub fn duotone_display_layer(d: &Duotone) -> Layer {
    Layer::new("Duotone", LayerContent::Adjustment(d.display_adjustment()))
}

/// A copy of `doc` that displays like the printed result (Duotone inks), or None when the
/// document displays as stored.
pub fn display_document(doc: &Document) -> Option<Document> {
    let d = doc.duotone.as_ref().filter(|_| doc.mode == ColorMode::Duotone)?;
    let mut out = doc.clone();
    out.layers.push(duotone_display_layer(d));
    Some(out)
}

#[cfg(test)]
#[path = "rotate_canvas_right_angle_tests.rs"]
mod rotate_canvas_right_angle_tests;
#[cfg(test)]
#[path = "rotate_canvas_selection_tests.rs"]
mod rotate_canvas_selection_tests;
#[cfg(test)]
mod tests;

#[cfg(test)]
mod color_table_index_tests;
#[cfg(test)]
#[path = "rotate_canvas_interpolation_pref_tests.rs"]
mod rotate_canvas_interpolation_pref_tests;

/// Stock Indexed Color regressions kept beside the conversion they guard.
#[cfg(test)]
mod stock_indexed_regressions {
    use super::*;

    fn paint(s: &mut Session, f: impl Fn(i32) -> [f32; 4]) {
        s.edit("setup", |doc, active| {
            let b = doc.bounds();
            let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            let fmt = surf.format();
            let mut data = Vec::new();
            for y in b.y0..b.y1 {
                for x in b.x0..b.x1 {
                    data.extend(photocraft_raster::from_rgba(&fmt, f(x)));
                }
            }
            surf.write_region(b, &data);
            Ok(())
        })
        .unwrap();
    }

    fn doc(s: &Session) -> &Document {
        &s.active().unwrap().doc
    }

    #[test]
    fn stock_black_white_keeps_opaque_white_out_of_the_transparency_slot() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 3, "height": 1, "background": "transparent"})).unwrap();
        paint(&mut s, |x| match x {
            0 => [0.0, 0.0, 0.0, 1.0],
            1 => [1.0, 1.0, 1.0, 1.0],
            _ => [0.0; 4],
        });
        let result = s.execute("image.mode.indexedColor", json!({"palette": "blackWhite", "colors": 2, "forced": "blackWhite", "dither": "none"})).unwrap();
        assert_eq!(result["colors"], 3);
        assert_eq!(result["transparentIndex"], 2);
        let d = doc(&s);
        let table = d.color_table.as_ref().unwrap();
        assert_eq!(table.transparent, Some(2));
        assert_eq!(d.layers[0].surface().unwrap().rgba(0, 0), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(d.layers[0].surface().unwrap().rgba(1, 0), [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(d.layers[0].surface().unwrap().rgba(2, 0)[3], 0.0);
        let indices = d.color_indices.as_ref().unwrap().indices.to_interleaved(d.bounds());
        assert_eq!(indices, vec![0, 1, 2], "opaque white must use the opaque white entry, not the transparency slot");
    }

    #[test]
    fn stock_opaque_near_white_does_not_take_the_synthetic_transparency_slot() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 4, "height": 1, "background": "transparent"})).unwrap();
        paint(&mut s, |x| match x {
            0 => [0.0, 0.0, 0.0, 1.0],
            1 => [0.5, 0.5, 0.5, 1.0],
            2 => [250.0 / 255.0, 250.0 / 255.0, 250.0 / 255.0, 1.0],
            _ => [0.0; 4],
        });
        s.execute("image.mode.indexedColor", json!({"palette": "uniform", "colors": 3, "forced": "none", "dither": "none"})).unwrap();
        let d = doc(&s);
        let transparent = d.color_table.as_ref().unwrap().transparent.expect("transparency slot") as u8;
        let indices = d.color_indices.as_ref().unwrap().indices.to_interleaved(d.bounds());
        assert_ne!(indices[2], transparent, "an opaque near-white must match an opaque palette entry");
        assert_eq!(indices[3], transparent, "a clear pixel still takes the transparency slot");
    }

    #[test]
    fn stock_tagged_rgb_palette_snapshot_is_encoded_srgb() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 1, "height": 1, "depth": 8, "mode": "rgb", "profile": "display-p3", "background": "transparent"})).unwrap();
        let p3 = craft_cms::Builtin::DisplayP3.profile();
        let mut encoded = [0.0f32; 16];
        craft_cms::Transform::new(craft_cms::Builtin::Srgb.profile(), p3, craft_cms::Intent::RelativeColorimetric, true)
            .unwrap()
            .eval(&[1.0, 0.0, 0.0], &mut encoded)
            .unwrap();
        paint(&mut s, |_| [encoded[0], encoded[1], encoded[2], 1.0]);
        let result = s.execute("image.mode.indexedColor", json!({"palette": "exact", "forced": "none", "dither": "none", "transparency": false})).unwrap();
        // The document colour table stays in Display P3 (the pixels' own encoding)…
        assert_eq!(doc(&s).color_table.as_ref().unwrap().colors.as_slice(), &[[234, 51, 35]]);
        // …while the replayable snapshot, a palette-map input, is encoded sRGB.
        let snapshot = result["paletteSnapshot"]["colors"][0].as_array().unwrap();
        let rgb = [snapshot[0].as_f64().unwrap(), snapshot[1].as_f64().unwrap(), snapshot[2].as_f64().unwrap()];
        assert!((rgb[0] - 1.0).abs() < 0.01 && rgb[1].abs() < 0.01 && rgb[2].abs() < 0.01, "snapshot must be encoded sRGB: {rgb:?}");
    }

    #[test]
    fn advanced_mapping_on_grayscale_stays_gray() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 2, "height": 1, "mode": "grayscale", "background": "transparent"})).unwrap();
        paint(&mut s, |x| if x == 0 { [0.5, 0.5, 0.5, 1.0] } else { [0.0; 4] });
        // A deliberately non-gray palette: the Indexed output transform must return it to the
        // grayscale document's profile, so neither the table nor the pixels stay coloured.
        s.execute(
            "image.mode.indexedColor",
            json!({"paletteSnapshot": {"name": "rgb", "colors": [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0]]}, "mapping": {"method": "none"}, "transparency": true}),
        )
        .unwrap();
        let d = doc(&s);
        for color in &d.color_table.as_ref().unwrap().colors {
            assert!(color[0] == color[1] && color[1] == color[2], "Indexed table must stay gray: {color:?}");
        }
        let pixel = d.layers[0].surface().unwrap().rgba(0, 0);
        assert!(pixel[0] == pixel[1] && pixel[1] == pixel[2], "Indexed pixels must stay gray: {pixel:?}");
    }
}
