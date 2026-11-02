//! Image › Generative Expand: grow the canvas and let the configured generative provider
//! (`photocraft_ml`, injected by the app as [`Session::generative`]) paint the new area.
//!
//! One command, one background job, one undo step: the canvas grows transparently (exactly as
//! Canvas Size does), the visible composite plus a mask of the new area go to the provider, and
//! the result lands on a new layer above the active one, opaque over the new area and fading
//! out over a `margin` band inside the old edge. The original pixels are never written: Undo or
//! deleting the layer rejects the result. Without a provider the command refuses and the UI
//! shows no menu item (upstream issue #41: off by default, no placeholder menus).

use std::sync::Arc;

use photocraft_color::PixelFormat;
use photocraft_doc::{Document, Layer, LayerId};
use photocraft_geom::Rect;
use photocraft_ml::{GenerateOptions, GenerativeProvider, RgbaImage, expand_coverage, expand_mask};
use photocraft_raster::{from_rgba_into, to_rgba};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

const EXPAND: &str = "image.generativeExpand";
const LABEL: &str = "Generative Expand";
/// Most pixels one expansion sends to the provider (the whole new canvas). A local CPU model
/// answers a 20-30 Mpx canvas in tens of seconds; beyond this the request is refused with a
/// clear message instead of running out of memory.
const MAX_PIXELS: u64 = 40_000_000;
/// Most pixels added on one side.
const MAX_ADD: f64 = 30_000.0;
/// Default and largest blend band (pixels inside the old edge the result fades over).
const DEFAULT_MARGIN: u32 = 32;
const MAX_MARGIN: f64 = 512.0;
/// Rows written to the result layer per step, so a large result never needs one huge buffer.
const BAND_ROWS: u32 = 64;

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn provider(s: &Session, cmd: &str) -> Result<Arc<dyn GenerativeProvider>> {
    s.generative.clone().ok_or_else(|| bad(cmd, "no generative provider is configured (Edit › Preferences › Generative AI)"))
}

fn can_expand(s: &Session) -> std::result::Result<(), String> {
    if s.generative.is_none() {
        return Err("no generative provider is configured".into());
    }
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

fn ml_error(e: photocraft_ml::Error) -> EngineError {
    match e {
        photocraft_ml::Error::Cancelled => EngineError::Cancelled,
        e => EngineError::Other(format!("{LABEL}: {e}")),
    }
}

/// The geometry of one expansion: old canvas `ow` × `oh` placed at `(dx, dy)` in the new
/// `nw` × `nh` canvas, result blended over `margin` pixels inside the old edge.
#[derive(Clone, Copy, Debug)]
struct Expansion {
    ow: u32,
    oh: u32,
    nw: u32,
    nh: u32,
    dx: u32,
    dy: u32,
    margin: u32,
}

impl Expansion {
    fn keep(&self) -> photocraft_ml::Rect {
        photocraft_ml::Rect { x: self.dx, y: self.dy, w: self.ow, h: self.oh }
    }
}

fn side(p: &Value, key: &str) -> Result<u32> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(0),
        Some(v) => v
            .as_f64()
            .filter(|f| f.is_finite() && (0.0..=MAX_ADD).contains(f))
            .map(|f| f.round() as u32)
            .ok_or_else(|| bad(EXPAND, format!("`{key}` must be 0..{MAX_ADD} pixels"))),
    }
}

fn expansion(doc: &Document, p: &Value) -> Result<Expansion> {
    let (ow, oh) = (doc.size.width, doc.size.height);
    let (l, r, t, b) = (side(p, "left")?, side(p, "right")?, side(p, "top")?, side(p, "bottom")?);
    if l + r + t + b == 0 {
        return Err(bad(EXPAND, "nothing to add: give `left`, `right`, `top` or `bottom` in pixels"));
    }
    let margin = match p.get("margin") {
        None | Some(Value::Null) => DEFAULT_MARGIN,
        Some(v) => v
            .as_f64()
            .filter(|f| f.is_finite() && (0.0..=MAX_MARGIN).contains(f))
            .map(|f| f.round() as u32)
            .ok_or_else(|| bad(EXPAND, format!("`margin` must be 0..{MAX_MARGIN} pixels")))?,
    };
    let too_large = || bad(EXPAND, format!("the expanded canvas would be too large for Generative Expand (at most {} Mpx)", MAX_PIXELS / 1_000_000));
    let nw = ow.checked_add(l).and_then(|w| w.checked_add(r)).ok_or_else(too_large)?;
    let nh = oh.checked_add(t).and_then(|h| h.checked_add(b)).ok_or_else(too_large)?;
    if u64::from(nw) * u64::from(nh) > MAX_PIXELS {
        return Err(too_large());
    }
    Ok(Expansion { ow, oh, nw, nh, dx: l, dy: t, margin })
}

/// The visible composite of `doc` as 8-bit RGBA, placed at `(dx, dy)` on a mid-grey canvas of
/// the new size: what the provider sees. Transparent pixels are shown over the same grey.
fn provider_input(doc: &Document, active: Option<LayerId>, e: Expansion, cancelled: &dyn Fn() -> bool) -> Result<RgbaImage> {
    use crate::retouch_cmds::{SampleLayers, composite_region};
    let fmt = doc.pixel_format();
    let region = composite_region(doc, active, SampleLayers::All, doc.bounds(), fmt);
    if cancelled() {
        return Err(EngineError::Cancelled);
    }
    let n = region.ch.max(1);
    let mut img =
        RgbaImage::filled(e.nw, e.nh, [128, 128, 128, 255]).ok_or_else(|| EngineError::Other(format!("{LABEL}: the expanded canvas is too large")))?;
    let to8 = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
    for y in 0..e.oh {
        if y % BAND_ROWS == 0 && cancelled() {
            return Err(EngineError::Cancelled);
        }
        for x in 0..e.ow {
            let i = (y * e.ow + x) as usize * n;
            let Some(px) = region.data.get(i..i + n) else { continue };
            let c = to_rgba(&fmt, px);
            let a = c[3].clamp(0.0, 1.0);
            let o = ((y + e.dy) * e.nw + (x + e.dx)) as usize * 4;
            if let Some(out) = img.data.get_mut(o..o + 4) {
                out[0] = to8(c[0] * a + 0.5 * (1.0 - a));
                out[1] = to8(c[1] * a + 0.5 * (1.0 - a));
                out[2] = to8(c[2] * a + 0.5 * (1.0 - a));
                out[3] = 255;
            }
        }
    }
    Ok(img)
}

/// What the job hands to the apply step.
struct Generated {
    e: Expansion,
    result: RgbaImage,
    coverage: Vec<u8>,
}

fn generative_expand(s: &mut Session, p: &Value) -> Result<Value> {
    let provider = provider(s, EXPAND)?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let e = expansion(&st.doc, p)?;
    let prompt = p.get("prompt").and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty()).map(str::to_owned);
    if prompt.is_some() && !provider.accepts_prompt() {
        return Err(bad(EXPAND, format!("the configured provider ({}) does not take a prompt", provider.info().name)));
    }
    let seed = p.get("seed").and_then(Value::as_u64);
    let (doc, active) = (st.doc.clone(), st.active_layer);
    crate::jobs::run(
        s,
        LABEL,
        true,
        move |ctx| {
            ctx.progress(0.0, "Reading the image");
            let cancelled = || ctx.cancelled();
            let input = provider_input(&doc, active, e, &cancelled)?;
            let mask = expand_mask(e.nw, e.nh, e.keep(), e.margin).ok_or_else(|| EngineError::Other(format!("{LABEL}: the expanded canvas is too large")))?;
            ctx.progress(0.1, "Generating");
            let result = provider.generate(&input, &mask, &GenerateOptions { prompt, seed }, &cancelled).map_err(ml_error)?;
            drop(input);
            ctx.check()?;
            let coverage =
                expand_coverage(e.nw, e.nh, e.keep(), e.margin).ok_or_else(|| EngineError::Other(format!("{LABEL}: the expanded canvas is too large")))?;
            ctx.progress(0.95, "Placing the result");
            Ok(Generated { e, result, coverage })
        },
        apply_expand,
    )
}

/// The apply step: grow the canvas and add the result layer, as one history step.
fn apply_expand(s: &mut Session, g: Generated) -> Result<Value> {
    let Generated { e, result, coverage } = g;
    if result.width != e.nw || result.height != e.nh || !result.is_consistent() {
        return Err(EngineError::Other(format!("{LABEL}: the provider returned an image of the wrong size")));
    }
    let (dx, dy) = (i32::try_from(e.dx).unwrap_or(i32::MAX), i32::try_from(e.dy).unwrap_or(i32::MAX));
    s.set_sampler_map(photocraft_geom::Affine::translate(f64::from(dx), f64::from(dy)));
    let layer = s.edit(LABEL, |doc, active| {
        crate::image_cmds::resize_canvas(doc, EXPAND, e.nw, e.nh, dx, dy, [0.0; 4])?;
        let fmt = PixelFormat { alpha: true, ..doc.pixel_format() };
        let mut layer = Layer::raster(doc.next_layer_name(LABEL), fmt);
        let ln = fmt.channels();
        if let Some(surf) = layer.surface_mut() {
            let mut enc = [0.0f32; 8];
            let w = e.nw as usize;
            let mut px = vec![0.0f32; w * BAND_ROWS as usize * ln];
            for y0 in (0..e.nh).step_by(BAND_ROWS as usize) {
                let rows = BAND_ROWS.min(e.nh - y0);
                let band = &mut px[..w * rows as usize * ln];
                band.fill(0.0);
                for (k, out) in band.chunks_exact_mut(ln).enumerate() {
                    let i = y0 as usize * w + k;
                    let a = coverage.get(i).copied().unwrap_or(0);
                    if a == 0 {
                        continue;
                    }
                    let Some(c) = result.data.get(i * 4..i * 4 + 4) else { continue };
                    let rgba = [f32::from(c[0]) / 255.0, f32::from(c[1]) / 255.0, f32::from(c[2]) / 255.0, f32::from(a) / 255.0];
                    from_rgba_into(&fmt, rgba, &mut enc);
                    out.copy_from_slice(&enc[..ln]);
                }
                surf.write_region(Rect::from_xywh(0, i32::try_from(y0).unwrap_or(i32::MAX), e.nw, rows), band);
            }
            surf.prune();
        }
        let nid = crate::layer_place::insert_new(doc, *active, layer, crate::layer_place::Place::Above);
        *active = Some(nid);
        Ok(nid)
    })?;
    Ok(json!({ "layer": layer.0, "width": e.nw, "height": e.nh, "offset": [dx, dy] }))
}

/// `generative.status`: what is configured, for the UI (menu item, Preferences, privacy text).
fn status(s: &mut Session, _: &Value) -> Result<Value> {
    Ok(match &s.generative {
        None => json!({ "configured": false }),
        Some(p) => {
            let info = p.info();
            json!({ "configured": true, "name": info.name, "destination": info.destination, "acceptsPrompt": p.accepts_prompt() })
        }
    })
}

/// `generative.ping`: ask the provider whether it is reachable (no pixels sent). A job, so the
/// UI thread never waits on the network.
fn ping(s: &mut Session, _: &Value) -> Result<Value> {
    let provider = provider(s, "generative.ping")?;
    crate::jobs::run(
        s,
        "Test generative connection",
        false,
        move |_| Ok(provider.ping().map_err(|e| e.to_string())),
        |_, r| match r {
            Ok(()) => Ok(json!({ "ok": true })),
            Err(detail) => Ok(json!({ "ok": false, "error": detail })),
        },
    )
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        // Not in the Photoshop catalogue: the UI places it after Image › Canvas Size… and drops
        // it while no provider is configured (upstream issue #41: no placeholder menus).
        CommandSpec {
            id: EXPAND,
            label: "Generative Expand…",
            menu: &["Image"],
            shortcut: None,
            params: r##"{"left":px=0,"right":px=0,"top":px=0,"bottom":px=0 (pixels added on each side; at least one),"margin":px=32 (band inside the old edge the result blends over),"prompt":text? (only for providers that take one),"seed":int?} → {layer,width,height,offset:[dx,dy]} (the canvas grows transparently, the result lands on a new layer "Generative Expand N" above the active layer; one undo step)"##,
            enabled: can_expand,
            run: generative_expand,
            journal: true,
        },
        CommandSpec {
            id: "generative.status",
            label: "Generative provider status",
            menu: &[],
            shortcut: None,
            params: r##"{} → {configured:bool, name?, destination?, acceptsPrompt?}"##,
            enabled: crate::cmd::enabled::always,
            run: status,
            journal: false,
        },
        CommandSpec {
            id: "generative.ping",
            label: "Test generative connection",
            menu: &[],
            shortcut: None,
            params: r##"{} → {ok:bool, error?} (a background job; sends no pixels)"##,
            enabled: |s| if s.generative.is_some() { Ok(()) } else { Err("no generative provider is configured".into()) },
            run: ping,
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use photocraft_ml::{Error as MlError, Mask, ProviderInfo};

    use super::*;

    /// A provider that paints the masked area a solid colour and records what it was asked.
    struct Solid {
        rgb: [u8; 3],
        fail: Option<fn() -> MlError>,
        seen: Mutex<Vec<(u32, u32, usize)>>,
    }

    impl Solid {
        fn new(rgb: [u8; 3]) -> Arc<Self> {
            Arc::new(Self { rgb, fail: None, seen: Mutex::new(Vec::new()) })
        }
    }

    impl GenerativeProvider for Solid {
        fn info(&self) -> ProviderInfo {
            ProviderInfo { name: "Solid (test)".into(), destination: "nowhere".into() }
        }
        fn accepts_prompt(&self) -> bool {
            false
        }
        fn ping(&self) -> std::result::Result<(), MlError> {
            Ok(())
        }
        fn generate(&self, image: &RgbaImage, mask: &Mask, _: &GenerateOptions, _: &dyn Fn() -> bool) -> std::result::Result<RgbaImage, MlError> {
            if let Some(f) = self.fail {
                return Err(f());
            }
            let white = mask.data.iter().filter(|&&v| v > 127).count();
            if let Ok(mut s) = self.seen.lock() {
                s.push((image.width, image.height, white));
            }
            let mut out = image.clone();
            for (px, &m) in out.data.as_chunks_mut::<4>().0.iter_mut().zip(&mask.data) {
                if m > 127 {
                    *px = [self.rgb[0], self.rgb[1], self.rgb[2], 255];
                }
            }
            Ok(out)
        }
    }

    fn session(provider: Option<Arc<dyn GenerativeProvider>>) -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 6, "background": "white"})).unwrap();
        s.generative = provider;
        s
    }

    fn pixel(s: &mut Session, x: i32, y: i32) -> [f32; 4] {
        let v = s.execute("document.pixel", json!({"x": x, "y": y})).unwrap();
        let a: Vec<f32> = v.as_array().unwrap().iter().map(|c| c.as_f64().unwrap() as f32).collect();
        [a[0], a[1], a[2], a[3]]
    }

    fn close(a: [f32; 4], b: [f32; 4]) -> bool {
        a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 0.02)
    }

    fn layer_count(s: &Session) -> usize {
        s.active().unwrap().doc.walk().len()
    }

    #[test]
    fn refuses_without_a_provider_or_without_an_amount() {
        let mut s = session(None);
        let err = s.execute(EXPAND, json!({"right": 4})).unwrap_err();
        assert!(err.to_string().contains("provider"), "{err}");
        assert!(can_expand(&s).is_err());
        assert_eq!(s.execute("generative.status", json!({})).unwrap(), json!({"configured": false}));

        let mut s = session(Some(Solid::new([255, 0, 0])));
        assert!(can_expand(&s).is_ok());
        let err = s.execute(EXPAND, json!({})).unwrap_err();
        assert!(matches!(err, EngineError::BadParams { .. }), "{err}");
        let err = s.execute(EXPAND, json!({"left": -3})).unwrap_err();
        assert!(matches!(err, EngineError::BadParams { .. }), "{err}");
        let err = s.execute(EXPAND, json!({"right": 4, "prompt": "a tree"})).unwrap_err();
        assert!(err.to_string().contains("prompt"), "{err}");
        // Nothing changed.
        assert_eq!(s.active().unwrap().doc.size.width, 8);
        assert_eq!(layer_count(&s), 1);
    }

    #[test]
    fn expands_right_on_a_new_layer_and_undoes_as_one_step() {
        let p = Solid::new([255, 0, 0]);
        let mut s = session(Some(p.clone()));
        let r = s.execute(EXPAND, json!({"right": 4, "margin": 0})).unwrap();
        assert_eq!(r["width"], 12);
        assert_eq!(r["height"], 6);
        assert_eq!(r["offset"], json!([0, 0]));
        // The provider saw the whole new canvas with exactly the new strip to fill.
        assert_eq!(p.seen.lock().unwrap().as_slice(), &[(12, 6, 4 * 6)]);
        // New layer above the background, active, named after the command.
        assert_eq!(layer_count(&s), 2);
        let st = s.active().unwrap();
        let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
        assert_eq!(l.name, "Generative Expand 1");
        // Composite: old pixels untouched, new strip red.
        assert!(close(pixel(&mut s, 2, 3), [1.0, 1.0, 1.0, 1.0]));
        assert!(close(pixel(&mut s, 10, 3), [1.0, 0.0, 0.0, 1.0]));
        // The layer itself is transparent over the old image (margin 0 = hard edge).
        let st = s.active().unwrap();
        let surf = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
        assert_eq!(surf.pixel(7, 3)[3], 0.0);
        assert_eq!(surf.pixel(8, 3)[3], 1.0);
        // One undo step restores the old canvas and removes the layer.
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.active().unwrap().doc.size.width, 8);
        assert_eq!(layer_count(&s), 1);
        assert!(close(pixel(&mut s, 2, 3), [1.0, 1.0, 1.0, 1.0]));
    }

    #[test]
    fn expands_left_and_top_with_an_offset_and_a_blend_band() {
        let p = Solid::new([0, 0, 255]);
        let mut s = session(Some(p.clone()));
        let r = s.execute(EXPAND, json!({"left": 3, "top": 2, "margin": 2})).unwrap();
        assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(11), Some(8)));
        assert_eq!(r["offset"], json!([3, 2]));
        // Old content moved to (3, 2); the new area is blue.
        assert!(close(pixel(&mut s, 0, 0), [0.0, 0.0, 1.0, 1.0]));
        assert!(close(pixel(&mut s, 10, 7), [1.0, 1.0, 1.0, 1.0]));
        let st = s.active().unwrap();
        let surf = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
        // Band: opaque at the old edge, fading inwards, gone after `margin` pixels.
        let a = |x: i32, y: i32| surf.pixel(x, y)[3];
        assert!(a(3, 5) > 0.6 && a(3, 5) < 0.7, "edge {}", a(3, 5));
        assert!(a(4, 5) > 0.3 && a(4, 5) < 0.4, "one in {}", a(4, 5));
        assert_eq!(a(5, 5), 0.0, "past the band");
        // Sides that were not extended keep a hard edge: bottom-right corner of the old image.
        assert_eq!(a(10, 7), 0.0);
        // The mask the provider saw covered the new area plus the band (not the untouched sides).
        let (w, h, white) = p.seen.lock().unwrap()[0];
        assert_eq!((w, h), (11, 8));
        let new_area = 11 * 8 - 8 * 6;
        let band = 2 * 6 + 2 * 8 - 2 * 2; // 2 columns inside the left edge + 2 rows inside the top edge, corner once
        assert_eq!(white, new_area + band);
    }

    #[test]
    fn a_cancelled_or_failing_provider_leaves_the_document_alone() {
        let mut p = Solid { rgb: [0; 3], fail: Some(|| MlError::Cancelled), seen: Mutex::new(Vec::new()) };
        let mut s = session(Some(Arc::new(Solid { rgb: p.rgb, fail: p.fail, seen: Mutex::new(Vec::new()) })));
        let err = s.execute(EXPAND, json!({"bottom": 2})).unwrap_err();
        assert!(matches!(err, EngineError::Cancelled), "{err}");
        assert_eq!(s.active().unwrap().doc.size.height, 6);
        assert_eq!(layer_count(&s), 1);

        p.fail = Some(|| MlError::Unreachable { url: "http://127.0.0.1:1".into(), detail: "connection refused".into() });
        let mut s = session(Some(Arc::new(p)));
        let err = s.execute(EXPAND, json!({"bottom": 2})).unwrap_err();
        assert!(err.to_string().contains("connection refused"), "{err}");
        assert_eq!(s.active().unwrap().doc.size.height, 6);
    }

    #[test]
    fn refuses_a_canvas_too_large_before_sending_anything() {
        let p = Solid::new([0; 3]);
        let mut s = session(Some(p.clone()));
        let err = s.execute(EXPAND, json!({"right": 30_000, "bottom": 30_000})).unwrap_err();
        assert!(err.to_string().contains("too large"), "{err}");
        assert!(p.seen.lock().unwrap().is_empty());
    }

    #[test]
    fn status_and_ping_report_the_provider() {
        let mut s = session(Some(Solid::new([0; 3])));
        let st = s.execute("generative.status", json!({})).unwrap();
        assert_eq!(st["configured"], true);
        assert_eq!(st["name"], "Solid (test)");
        assert_eq!(st["acceptsPrompt"], false);
        assert_eq!(s.execute("generative.ping", json!({})).unwrap(), json!({"ok": true}));
    }
}
