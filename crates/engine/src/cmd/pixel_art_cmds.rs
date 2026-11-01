//! Image › Pixel Art Upscale (`image.pixelArtUpscale`): enlarges the whole document 2, 3 or 4
//! times with Scale2x / Scale3x (`photocraft_algo::pixel_art`), the upscalers made for sprites
//! and low-resolution art. Nearest Neighbor keeps the hard edges but leaves every diagonal a
//! coarse staircase; the interpolating methods smooth the diagonals but blur the edges and invent
//! in-between colours. Scale2x/3x do neither: they only copy existing neighbours, so the palette
//! stays exact (an Indexed Color document keeps its colour table) while diagonals step at the new
//! resolution.
//!
//! Photoshop has no such resample (emulators and pixel-art editors do), so the id is our own; the
//! menu item sits in Image after Image Size. Like Image Size it resizes every layer, mask,
//! channel and the selection, and scales effects, vectors and guides with them, in one history
//! step.

use photocraft_algo::pixel_art::{PixelArtScale, peak_bytes, upscale_surface_in_canvas};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

/// The command id.
pub const ID: &str = "image.pixelArtUpscale";

/// The largest side Image Size allows.
const MAX_SIDE: u64 = 300_000;

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: ID.into(), msg: msg.into() }
}

fn parse_scale(p: &Value) -> Result<PixelArtScale> {
    if !p.is_object() {
        return Err(bad("params must be a JSON object"));
    }
    match p.get("scale") {
        None => Ok(PixelArtScale::Scale2x),
        Some(Value::String(s)) => match s.as_str() {
            "scale2x" => Ok(PixelArtScale::Scale2x),
            "scale3x" => Ok(PixelArtScale::Scale3x),
            "scale4x" => Ok(PixelArtScale::Scale4x),
            other => Err(bad(format!("`scale` must be \"scale2x\", \"scale3x\" or \"scale4x\", not {other:?}"))),
        },
        Some(_) => Err(bad("`scale` must be \"scale2x\", \"scale3x\" or \"scale4x\"")),
    }
}

fn enabled(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

/// Refuses, before anything is allocated, an upscale whose working memory would pass the resample
/// budget: the result alone may fit it while Scale4x's 2x intermediate and the band buffers don't.
fn check_peak(scale: PixelArtScale, ow: u32, oh: u32, bytes_per_pixel: usize, channels: usize) -> Result<()> {
    let peak = peak_bytes(scale, ow, oh, bytes_per_pixel, channels);
    let max = crate::image_cmds::MAX_RESAMPLE_BYTES;
    if peak <= max {
        return Ok(());
    }
    let n = scale.factor();
    Err(bad(format!("upscaling {ow} x {oh} px {n}x needs about {} MB of working memory; the limit is {} MB. Choose a smaller scale", peak >> 20, max >> 20)))
}

/// One surface's preflight: its scaled pixels stay inside the 32-bit coordinate range (the
/// surfaces saturate there), and the working memory for the box around them fits the budget,
/// however far past the canvas they reach.
fn check_surface(scale: PixelArtScale, surf: &Surface) -> Result<()> {
    let r = surf.content_bounds();
    if r.is_empty() {
        return Ok(());
    }
    let n = scale.factor();
    if ![r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.checked_mul(n as i32).is_some()) {
        return Err(bad(format!("upscaling {n}x would push some pixels outside the 32-bit coordinate range")));
    }
    check_peak(scale, r.width(), r.height(), surf.format().bytes_per_pixel(), surf.channels())
}

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let scale = parse_scale(p)?;
    let n = scale.factor();
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let (ow, oh) = (d.doc.size.width, d.doc.size.height);
    let (nw, nh) = (u64::from(ow) * u64::from(n), u64::from(oh) * u64::from(n));
    if nw > MAX_SIDE || nh > MAX_SIDE {
        return Err(bad(format!("{ow} x {oh} px upscaled {n}x is {nw} x {nh} px; a side can be at most {MAX_SIDE} px")));
    }
    let (nw, nh) = (nw as u32, nh as u32);
    crate::image_cmds::check_resample_budget(ID, &d.doc, nw, nh, "")?;
    let f = d.doc.pixel_format();
    check_peak(scale, ow, oh, f.bytes_per_pixel(), f.channels())?;
    let k = f64::from(n);
    s.set_sampler_map(photocraft_geom::Affine { m: [k, 0.0, 0.0, k, 0.0, 0.0] });
    s.edit("Pixel Art Upscale", |doc, _| {
        // Layers, masks and channels may reach far past the canvas, and each surface upscales the
        // box around its own pixels: check every one before changing anything.
        let mut err = None;
        let mut check = |surf: &Surface| {
            if err.is_none() {
                err = check_surface(scale, surf).err();
            }
        };
        crate::image_cmds::for_each_surface(&mut doc.layers, true, &mut |surf, _| check(surf));
        for ch in doc.channels.iter().chain(doc.quick_mask.as_ref()) {
            check(&ch.surface);
        }
        if let Some(sel) = &doc.selection {
            check(sel);
        }
        if let Some(e) = err {
            return Err(e);
        }
        let canvas = Rect::from_xywh(0, 0, ow, oh);
        // Masks, channels and the selection take the same rule, so they keep fitting the pixels.
        crate::image_cmds::rescale_doc(doc, nw, nh, &mut |surf, _| upscale_surface_in_canvas(surf, scale, canvas));
        Ok(())
    })?;
    Ok(json!({ "width": nw, "height": nh }))
}

/// The command spec.
pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: ID,
        label: "Pixel Art Upscale…",
        menu: &["Image"],
        shortcut: None,
        params: r#"{"scale":"scale2x|scale3x|scale4x"="scale2x"} → {"width","height"}"#,
        enabled,
        run,
        journal: true,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{Document, Size};

    fn doc(s: &Session) -> &Document {
        &s.active().unwrap().doc
    }

    /// A 3 × 3 document: a red staircase over a blue sky, on the Background.
    fn stairs(depth: u32) -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 3, "height": 3, "depth": depth})).unwrap();
        s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
        for (x, y, w) in [(0, 0, 1), (0, 1, 2), (0, 2, 3)] {
            s.execute("select.rect", json!({"x": x, "y": y, "width": w, "height": 1})).unwrap();
            s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        }
        s.execute("select.deselect", json!({})).unwrap();
        s
    }

    fn rgb(s: &Session, x: i32, y: i32) -> [f32; 3] {
        let p = doc(s).layers[0].surface().unwrap().rgba(x, y);
        [p[0], p[1], p[2]]
    }

    const RED: [f32; 3] = [1.0, 0.0, 0.0];
    const BLUE: [f32; 3] = [0.0, 0.0, 1.0];

    #[test]
    fn upscales_the_document_and_rounds_diagonals_at_every_depth() {
        for depth in [8, 16, 32] {
            for (scale, n) in [("scale2x", 2), ("scale3x", 3), ("scale4x", 4)] {
                let mut s = stairs(depth);
                let r = s.execute(ID, json!({"scale": scale})).unwrap();
                assert_eq!(r, json!({"width": 3 * n, "height": 3 * n}), "{depth} {scale}");
                assert_eq!(doc(&s).size, Size::new(3 * n, 3 * n));
                // The middle pixel's top-right corner turns to sky; its opposite corner stays red.
                let (x0, y0) = (n as i32, n as i32);
                assert_eq!(rgb(&s, x0 + n as i32 - 1, y0), BLUE, "{depth} {scale}");
                assert_eq!(rgb(&s, x0, y0 + n as i32 - 1), RED, "{depth} {scale}");
                // Every pixel is one of the two colours: nothing in between.
                for y in 0..3 * n as i32 {
                    for x in 0..3 * n as i32 {
                        let p = rgb(&s, x, y);
                        assert!(p == RED || p == BLUE, "{depth} {scale} ({x}, {y}) {p:?}");
                    }
                }
                assert!(s.undo());
                assert_eq!(doc(&s).size, Size::new(3, 3));
            }
        }
    }

    #[test]
    fn defaults_to_scale2x() {
        let mut s = stairs(8);
        s.execute(ID, json!({})).unwrap();
        assert_eq!(doc(&s).size, Size::new(6, 6));
    }

    /// The colour table holds only the source colours, and the result keeps to them.
    #[test]
    fn indexed_color_keeps_its_colour_table() {
        let mut s = stairs(8);
        s.execute("image.mode.indexedColor", json!({})).unwrap();
        s.execute(ID, json!({"scale": "scale3x"})).unwrap();
        let surf = doc(&s).layers[0].surface().unwrap();
        for y in 0..9 {
            for x in 0..9 {
                let p = surf.pixel(x, y);
                assert!(p[..3] == RED || p[..3] == BLUE, "({x}, {y}) {p:?}");
            }
        }
    }

    #[test]
    fn selection_and_masks_follow_the_pixels() {
        let mut s = stairs(8);
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("select.rect", json!({"x": 1, "y": 1, "width": 1, "height": 2})).unwrap();
        s.execute("layer.layerMask.revealSelection", json!({})).unwrap();
        s.execute("select.rect", json!({"x": 1, "y": 1, "width": 1, "height": 2})).unwrap();
        s.execute(ID, json!({"scale": "scale2x"})).unwrap();
        let d = doc(&s);
        assert_eq!(d.selection.as_ref().unwrap().content_bounds(), Rect::new(2, 2, 4, 6));
        let m = d.layers.iter().find_map(|l| l.mask.as_ref()).unwrap();
        for (x, y, v) in [(2, 2, 1.0), (3, 5, 1.0), (1, 3, 0.0), (4, 3, 0.0), (3, 1, 0.0)] {
            assert_eq!(m.surface.pixel(x, y), vec![v], "({x}, {y})");
        }
    }

    #[test]
    fn bad_params_and_sizes_fail_without_changing_the_document() {
        let mut s = Session::new();
        assert!(s.execute(ID, json!({})).is_err(), "no document");
        let mut s = stairs(8);
        for p in [json!([]), json!("scale2x"), json!({"scale": 2}), json!({"scale": "scale5x"}), json!({"scale": ""}), json!({"scale": null})] {
            assert!(s.execute(ID, p.clone()).is_err(), "{p}");
            assert_eq!(doc(&s).size, Size::new(3, 3), "{p}");
        }
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 100_001, "height": 1})).unwrap();
        let e = s.execute(ID, json!({"scale": "scale3x"})).unwrap_err().to_string();
        assert!(e.contains("300000"), "{e}");
        assert_eq!(doc(&s).size, Size::new(100_001, 1));
    }

    /// The result alone fits the budget, but not with Scale4x's 2x intermediate beside it.
    #[test]
    fn scale4x_counts_its_intermediate_against_the_budget() {
        let max = crate::image_cmds::MAX_RESAMPLE_BYTES;
        // 32-bit RGBA: 16 bytes a pixel. A square whose 4x result is just under the budget.
        let side = ((max / 16) as f64).sqrt() as u32 / 4 - 1;
        assert!(u64::from(side * 4).pow(2) * 16 <= max, "the result alone fits");
        let e = check_peak(PixelArtScale::Scale4x, side, side, 16, 4).unwrap_err().to_string();
        assert!(e.contains("working memory"), "{e}");
        // Scale2x of the same source is a quarter of the size: well within.
        assert!(check_peak(PixelArtScale::Scale2x, side, side, 16, 4).is_ok());
        assert!(check_peak(PixelArtScale::Scale4x, u32::MAX, u32::MAX, 16, 5).is_err());
    }

    /// Two pixels far apart on a small canvas: the canvas fits the budget, but the box the layer
    /// would be upscaled over doesn't, so it is refused before anything is allocated.
    #[test]
    fn far_apart_layer_pixels_count_against_the_budget() {
        let mut s = stairs(8);
        s.edit("far", |doc, _| {
            let surf = doc.layers[0].surface_mut().unwrap();
            surf.write_pixel(500_000_000, 0, &[1.0, 1.0, 1.0]);
            Ok(())
        })
        .unwrap();
        let e = s.execute(ID, json!({"scale": "scale2x"})).unwrap_err().to_string();
        assert!(e.contains("working memory"), "{e}");
        assert_eq!(doc(&s).size, Size::new(3, 3));
    }

    /// A layer parked near the edge of the coordinate range can't be scaled past it.
    #[test]
    fn pixels_near_the_coordinate_limit_are_refused() {
        let mut s = stairs(8);
        s.edit("far", |doc, _| {
            let surf = doc.layers[0].surface_mut().unwrap();
            surf.write_pixel(i32::MAX - 70, 0, &[1.0, 1.0, 1.0]);
            Ok(())
        })
        .unwrap();
        assert!(s.execute(ID, json!({"scale": "scale2x"})).is_err());
        assert_eq!(doc(&s).size, Size::new(3, 3));
    }
}
