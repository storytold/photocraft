//! Cryptomatte selection (specification 1.2): click a pixel, and every pixel covered by the
//! same object ID becomes the selection — the Arnold/V-Ray/Redshift workflow on top of the
//! codec's [`photocraft_codecs::decode_cryptomatte`]. Works on the EXR the active document
//! was opened from (its parts came from there); the manifest names the object in the result.
//!
//! The source is decoded once per document and kept in a process-wide cache (document ids are
//! globally unique). Two pins keep later clicks honest: the file's identity (a re-render at
//! the same path refuses instead of silently selecting from the new file) and a signature of
//! the opened render sampled onto the canvas (a Flip Canvas, a rotation or a crop grown back
//! to the same size moves pixels without moving bounds — refused, never masked wrong).

use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use photocraft_algo::selection::{self as sel, SelectionMode};

use crate::commands::{CommandSpec, int_i32};
use crate::{EngineError, Result, Session};

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: "select.cryptomatte".into(), msg: msg.into() }
}

/// The Cryptomatte source of one open document, decoded once and reused for later clicks.
struct Source {
    /// The file as it was when the samples were decoded ([`photocraft_io::FileIdentity`]:
    /// length and mtime, plus inode change time on Unix).
    identity: photocraft_io::FileIdentity,
    /// The file bytes the buffers decode from, shared so a second layer costs no re-read.
    bytes: Arc<Vec<u8>>,
    layers: Vec<photocraft_codecs::CryptomatteLayer>,
    /// Decoded [`photocraft_codecs::CryptomatteBuffer`]s by layer name.
    buffers: HashMap<String, photocraft_codecs::CryptomatteBuffer>,
    /// The canvas the EXR opens as (the staged decode's size).
    canvas: (u32, u32),
    /// Where the EXR's data window sits on that canvas.
    origin: (i32, i32),
    /// Sample points of the opened render: the document's raster must still show these
    /// values, or the canvas has moved away from the render's grid.
    signature: Vec<((i32, i32), Vec<f32>)>,
}

/// Decoded sources per open document. Document ids are globally unique, so entries cannot
/// collide across sessions; past `CAP` entries the cache clears and the next click re-decodes,
/// bounding what a long-lived process keeps.
static SOURCES: Mutex<Vec<(photocraft_doc::DocId, Source)>> = Mutex::new(Vec::new());

const CAP: usize = 8;

fn sources() -> std::sync::MutexGuard<'static, Vec<(photocraft_doc::DocId, Source)>> {
    SOURCES.lock().unwrap_or_else(PoisonError::into_inner)
}

/// 33 deterministic points spread over the canvas (a 5×5 grid plus offsets): any flip or
/// rotation moves at least one of them in every real render. Uniform synthetic images can be
/// symmetric — the tests use varied fixtures for exactly that.
fn sample_points(w: u32, h: u32) -> Vec<(i32, i32)> {
    let mut pts = Vec::new();
    for gy in 0..5u64 {
        for gx in 0..5u64 {
            pts.push(((gx * (w as u64 - 1) / 4) as i32, (gy * (h as u64 - 1) / 4) as i32));
        }
    }
    for i in 0..8u64 {
        pts.push((((w as i64 * (i as i64 + 1)) / 9) as i32, ((h as i64 * (2 * i as i64 + 1)) / 17) as i32));
    }
    pts
}

/// `select.cryptomatte`: the ID at `x, y` in the Cryptomatte layer `layer` (default: the
/// file's first) of the document's source EXR, as a selection whose mask value is the ID's
/// coverage — partial coverage gives a partial selection, so edges land between both objects.
fn select_cryptomatte(s: &mut Session, p: &Value) -> Result<Value> {
    let coord = |k: &str| int_i32("select.cryptomatte", p, k).and_then(|v| v.ok_or_else(|| bad(format!("missing `{k}`"))));
    let (x, y) = (coord("x")?, coord("y")?);
    let mode = p.get("mode").and_then(Value::as_str).unwrap_or("replace").to_string();
    if !matches!(mode.as_str(), "replace" | "add" | "subtract" | "intersect") {
        return Err(bad(format!("mode is \"{mode}\"; it is replace, add, subtract or intersect")));
    }
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let Some(path) = st.path.clone() else {
        return Err(EngineError::Other("the document was not opened from a file; its Cryptomatte samples are unavailable".into()));
    };
    if !path.to_ascii_lowercase().ends_with(".exr") {
        return Err(EngineError::Other(format!("{path} is no EXR; Cryptomatte selection needs the source render")));
    }
    let doc_id = st.doc.id;
    let doc_bounds = st.doc.bounds();
    // The file as it is now; the decode below only runs when this still matches what the
    // cached samples came from.
    let identity = photocraft_io::FileIdentity::of_path(std::path::Path::new(&path)).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    {
        let mut srcs = sources();
        if let Some((_, src)) = srcs.iter().find(|(id, _)| *id == doc_id)
            && src.identity != identity
        {
            // A re-render at the same path is a different file: refuse rather than
            // select from it behind the open document's back.
            return Err(EngineError::Other(format!(
                "{path} changed since it was opened (length or modification time differ); re-open the EXR to select from the new render"
            )));
        }
        if srcs.iter().all(|(id, _)| *id != doc_id) {
            let bytes = crate::file_cmds::read_file(&path)?;
            let layers = photocraft_codecs::cryptomatte_layers(&bytes, &Default::default()).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
            // The staged decode pins the canvas: its size is what the document opened as,
            // and its sample values are what the document must still show.
            let staged = photocraft_codecs::decode(&bytes).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
            let canvas = (staged.width(), staged.height());
            let origin = staged
                .warnings
                .iter()
                .find_map(|w| if let photocraft_codecs::DecodeWarning::DisplayWindowStage { origin, .. } = w { Some(*origin) } else { None })
                .unwrap_or((0, 0));
            let channels = staged.layout().channels().min(4);
            let signature = sample_points(canvas.0, canvas.1)
                .into_iter()
                .map(|(px, py)| ((px, py), (0..channels).map(|c| staged.get(px as u32, py as u32, c)).collect::<Vec<_>>()))
                .collect();
            if srcs.len() >= CAP {
                srcs.clear();
            }
            srcs.push((doc_id, Source { identity, bytes: Arc::new(bytes), layers, buffers: HashMap::new(), canvas, origin, signature }));
        }
    }
    // The samples only line up with the canvas the EXR opens as. Any Crop, Canvas Size or
    // Image Size afterwards moves or scales the document away from that grid, so the mask
    // would land in the wrong place — refuse instead of selecting something else.
    {
        let st = s.active().ok_or(EngineError::NoDocument)?;
        let srcs = sources();
        let Some((_, src)) = srcs.iter().find(|(id, _)| *id == doc_id) else {
            return Err(EngineError::Other("the Cryptomatte source of this document is unavailable; click again".into()));
        };
        if doc_bounds.width() != src.canvas.0 || doc_bounds.height() != src.canvas.1 {
            return Err(EngineError::Other(format!(
                "the document is {}×{} but {} opens as {}×{}; after Crop, Canvas Size or Image Size the Cryptomatte samples no longer line up with the canvas. Undo back to the opened size, or re-open the EXR",
                doc_bounds.width(),
                doc_bounds.height(),
                path,
                src.canvas.0,
                src.canvas.1
            )));
        }
        // The bounds can match while the pixels moved (Flip Canvas, Rotate, a crop grown back
        // to the same size): the document's raster must still show the opened render at the
        // sample points, or the click refuses rather than select from the wrong grid.
        let Some(surface) = st.doc.layers.iter().find_map(|l| l.surface()) else {
            return Err(EngineError::Other(
                "the document holds no raster layer any more; its pixels no longer line up with the Cryptomatte samples. Re-open the EXR".into(),
            ));
        };
        for ((px, py), expect) in &src.signature {
            let got = surface.pixel(px + doc_bounds.x0, py + doc_bounds.y0);
            if got.len() < expect.len() || got[..expect.len()] != expect[..] {
                return Err(EngineError::Other(
                    "the canvas no longer matches the opened EXR — Flip Canvas, rotation or edits moved its pixels. Undo back to the opened image, or re-open the EXR".into(),
                ));
            }
        }
    }
    // The buffer of the picked layer, decoded once and cached; the mask is built while the
    // cache is pinned so the (large) buffer is never copied.
    let clicked = 'click: {
        let mut srcs = sources();
        let Some((_, src)) = srcs.iter_mut().find(|(id, _)| *id == doc_id) else {
            return Err(EngineError::Other("the Cryptomatte source of this document is unavailable; click again".into()));
        };
        let layer = match p.get("layer") {
            None | Some(Value::Null) => src.layers.first().cloned().ok_or_else(|| EngineError::Other(format!("{path} holds no Cryptomatte layer")))?,
            Some(Value::String(want)) => src.layers.iter().find(|l| l.name.eq_ignore_ascii_case(want)).cloned().ok_or_else(|| {
                EngineError::Other(format!(
                    "no Cryptomatte layer named {want} (have {})",
                    src.layers.iter().map(|l| l.name.clone()).collect::<Vec<_>>().join(", ")
                ))
            })?,
            Some(other) => return Err(bad(format!("`layer` must be a string when given, not {}", json_type(other)))),
        };
        let bytes = Arc::clone(&src.bytes);
        let Some(buffer) = src.buffers.get(&layer.name) else {
            // A layer that fails to decode is not cached: the error reaches the caller, and a
            // retry (or a fixed file) decodes again.
            let decoded =
                photocraft_codecs::decode_cryptomatte(&bytes, &layer.name, &Default::default()).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
            src.buffers.insert(layer.name.clone(), decoded);
            break 'click Clicked::Retry;
        };
        // Where the EXR's data window sits on the canvas: the placement the staged decode
        // recorded, never pixel content — content bounds follow transparency, and a beauty's
        // background usually is transparent.
        let (ox, oy) = src.origin;
        let (bx, by) = (x - ox, y - oy);
        if bx < 0 || by < 0 || bx as u32 >= buffer.width || by as u32 >= buffer.height {
            return Err(bad(format!("({x}, {y}) lies outside the {}×{} Cryptomatte buffer at ({ox}, {oy})", buffer.width, buffer.height)));
        }
        // The ID to select: the strongest sample at the clicked pixel.
        let Some(want) = buffer.at(bx as u32, by as u32).first().copied().map(|(id, _)| id) else {
            break 'click Clicked::Empty(layer.name);
        };
        // The coverage of `want` at every pixel becomes the selection mask.
        let name = layer.name_of(want).map(str::to_string);
        let stride = doc_bounds.width() as usize;
        let mut mask = vec![0.0f32; stride.saturating_mul(doc_bounds.height() as usize)];
        let mut selected = 0u64;
        for py in 0..buffer.height {
            for px in 0..buffer.width {
                // IDs are hashes stored as float bit patterns: a numeric distance between two
                // says nothing about their objects, so only the exact value matches.
                let cov = buffer.at(px, py).iter().find(|(id, _)| *id == want).map(|(_, c)| *c).unwrap_or(0.0);
                if cov > 0.0 {
                    let (dx, dy) = (ox - doc_bounds.x0 + px as i32, oy - doc_bounds.y0 + py as i32);
                    if dx >= 0 && dy >= 0 && (dx as usize) < stride && (dy as usize) < mask.len() / stride.max(1) {
                        mask[dy as usize * stride + dx as usize] = cov;
                        selected += 1;
                    }
                }
            }
        }
        Clicked::Id { want, name, mask, selected, layer: layer.name }
    };
    match clicked {
        // A layer decoded for the first time this click: the buffer is cached now, and the
        // same click runs again against it (one recursion, then the buffer is present).
        Clicked::Retry => select_cryptomatte(s, p),
        // Nothing at the click: clicking empty space. Replace deselects (like the Magic
        // Wand), intersect with nothing empties the selection; add and subtract keep it.
        Clicked::Empty(layer) => {
            if mode == "replace" || mode == "intersect" {
                s.edit("Cryptomatte", |doc, _| {
                    doc.selection = None;
                    Ok(())
                })?;
            }
            Ok(json!({"id": null, "name": null, "selected": 0, "layer": layer}))
        }
        Clicked::Id { want, name, mask, selected, layer } => {
            let mode = match mode.as_str() {
                "add" => SelectionMode::Add,
                "subtract" => SelectionMode::Subtract,
                "intersect" => SelectionMode::Intersect,
                _ => SelectionMode::Replace,
            };
            s.edit("Cryptomatte", |doc, _| {
                doc.selection = sel::combine(doc.selection.as_ref(), &mask, doc_bounds, mode);
                Ok(())
            })?;
            Ok(json!({"id": want, "name": name, "selected": selected, "layer": layer}))
        }
    }
}

/// What a click found: an empty pixel, an ID with its mask, or "the layer's buffer was just
/// cached, run the click again".
enum Clicked {
    Empty(String),
    Retry,
    Id { want: f32, name: Option<String>, mask: Vec<f32>, selected: u64, layer: String },
}

/// The JSON type of `v` for parameter errors ("string", "number", …).
fn json_type(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

pub fn specs() -> Vec<CommandSpec> {
    macro_rules! spec {
        ($id:expr, $label:expr, $menu:expr, $sc:expr, $params:expr, $enabled:expr, $run:expr) => {
            CommandSpec { id: $id, label: $label, menu: $menu, shortcut: $sc, params: $params, enabled: $enabled, journal: true, run: $run }
        };
    }
    vec![spec!(
        "select.cryptomatte",
        "Cryptomatte Selection",
        &["Select"],
        None,
        r##"{"x":i32,"y":i32 (canvas coordinates),"layer":str? (the Cryptomatte layer; default the file's first),"mode":"replace|add|subtract|intersect"="replace"} → {id,name,selected,layer}; refuses after Crop/Canvas Size/Image Size, after Flip Canvas/rotation moved the pixels, or when the source EXR changed since it was opened"##,
        crate::file_cmds::native_doc,
        select_cryptomatte
    )]
}

#[cfg(test)]
mod tests {
    use super::*;
    use exr::prelude::*;
    use serde_json::json;

    /// A 4×4 EXR whose `crypto_asset` Cryptomatte holds two objects: `cube` on the left two
    /// columns (coverage 1) and `sphere` on the right (coverage 0.5), via the codec's hash.
    fn cryptomatte_file(path: &std::path::Path) {
        cryptomatte_file_at(path, (0, 0, 4, 4), (0, 0, 4, 4), (photocraft_codecs::cryptomatte_id("cube"), photocraft_codecs::cryptomatte_id("sphere")));
    }

    /// The fixture at an arbitrary data-window position inside an arbitrary display window,
    /// with explicit object IDs (for tests that need particular float values).
    #[allow(clippy::too_many_arguments)]
    fn cryptomatte_file_at(path: &std::path::Path, display: (i32, i32, usize, usize), data: (i32, i32, usize, usize), ids: (f32, f32)) {
        let (w, h) = (data.2, data.3);
        let cube = ids.0;
        let sphere = ids.1;
        let (s0, s1) = (vec![cube; w * h], (0..w * h).map(|p| if p % w >= 2 { sphere } else { 0.0 }).collect::<Vec<_>>());
        let (c0, c1) = (
            (0..w * h).map(|p| if p % w < 2 { 1.0f32 } else { 0.0 }).collect::<Vec<_>>(),
            (0..w * h).map(|p| if p % w >= 2 { 0.5f32 } else { 0.0 }).collect::<Vec<_>>(),
        );
        let mk = |n: &str, v: Vec<f32>| AnyChannel::new(n, FlatSamples::F32(v));
        // Beauty channels so the plain import opens the file (its colour auto-pick).
        let (r, g, b, a) = (vec![0.5f32; w * h], vec![0.5f32; w * h], vec![0.5f32; w * h], vec![1.0f32; w * h]);
        let list = vec![
            mk("R", r),
            mk("G", g),
            mk("B", b),
            mk("A", a),
            mk("crypto_asset00.red", s0),
            mk("crypto_asset00.green", c0),
            mk("crypto_asset01.red", s1),
            mk("crypto_asset01.green", c1),
        ];
        let key = photocraft_codecs::cryptomatte_key("crypto_asset");
        let manifest = format!("{{\"cube\":\"{:08x}\",\"sphere\":\"{:08x}\"}}", cube.to_bits(), sphere.to_bits());
        let mut attrs = LayerAttributes::named("crypto_asset");
        attrs.other.insert(format!("cryptomatte/{key}/name").as_str().into(), AttributeValue::Text("crypto_asset".into()));
        attrs.other.insert(format!("cryptomatte/{key}/manifest").as_str().into(), AttributeValue::Text(manifest.as_str().into()));
        let layer = Layer::new(
            (w, h),
            attrs,
            Encoding { compression: Compression::Uncompressed, blocks: Blocks::ScanLines, line_order: LineOrder::Increasing },
            AnyChannels::sort(list.into_iter().collect()),
        );
        let image = Image::from_layers(ImageAttributes::new(IntegerBounds::new((display.0, display.1), (display.2, display.3))), vec![layer]);
        image.write().to_buffered(&mut std::io::BufWriter::new(std::fs::File::create(path).expect("fixture"))).expect("write fixture");
    }

    fn session_with_crypto() -> (Session, std::path::PathBuf) {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!("pcraft-cm-select-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("render.exr");
        cryptomatte_file(&path);
        let mut s = Session::new();
        let bytes = std::fs::read(&path).unwrap();
        let r = photocraft_io::import("render.exr", &bytes).unwrap();
        let (i, _) = s.open_document(r.document, Some(path.to_string_lossy().into_owned()));
        s.set_active(i);
        (s, dir)
    }

    #[test]
    fn clicking_an_object_selects_exactly_its_pixels() {
        let (mut s, dir) = session_with_crypto();
        // cube covers the left two columns fully.
        let r = s.execute("select.cryptomatte", json!({"x": 0, "y": 0})).unwrap();
        assert_eq!(r["name"], "cube", "{r}");
        assert_eq!(r["selected"], 8, "{r}");
        let st = s.active().unwrap();
        let sel = st.doc.selection.as_ref().expect("a selection");
        assert_eq!(sel.pixel(0, 0)[0], 1.0);
        assert_eq!(sel.pixel(3, 0)[0], 0.0);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn partial_coverage_gives_a_partial_selection() {
        let (mut s, dir) = session_with_crypto();
        let r = s.execute("select.cryptomatte", json!({"x": 3, "y": 0})).unwrap();
        assert_eq!(r["name"], "sphere");
        let st = s.active().unwrap();
        let sel = st.doc.selection.as_ref().expect("a selection");
        assert!((sel.pixel(3, 0)[0] - 0.5).abs() < 1e-2, "{}", sel.pixel(3, 0)[0]); // GRAY8 quantises
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn empty_click_add_and_layer_by_name() {
        let (mut s, dir) = session_with_crypto();
        s.execute("select.cryptomatte", json!({"x": 0, "y": 0})).unwrap();
        // add: both objects end up selected.
        s.execute("select.cryptomatte", json!({"x": 3, "y": 0, "mode": "add", "layer": "crypto_asset"})).unwrap();
        let st = s.active().unwrap();
        let sel = st.doc.selection.as_ref().unwrap();
        assert!(sel.pixel(0, 0)[0] > 0.99 && sel.pixel(3, 0)[0] > 0.49);
        // subtract removes the sphere again, on the shared selection algebra.
        s.execute("select.cryptomatte", json!({"x": 3, "y": 0, "mode": "subtract"})).unwrap();
        let sel = s.active().unwrap().doc.selection.as_ref().unwrap();
        assert!(sel.pixel(3, 0)[0] < 1e-2, "{}", sel.pixel(3, 0)[0]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn canvas_changes_refuse_instead_of_misplacing() {
        let (mut s, dir) = session_with_crypto();
        // A non-centred crop: the old centring assumption would place the mask wrong.
        s.execute("image.crop", json!({"x": 1, "y": 1, "width": 3, "height": 3})).unwrap();
        let e = s.execute("select.cryptomatte", json!({"x": 0, "y": 0})).unwrap_err().to_string();
        assert!(e.contains("no longer line up"), "{e}");
        assert!(!e.is_empty());
        // A resize scales the document away from the render's grid just the same.
        s.execute("edit.undo", json!({})).unwrap();
        s.execute("image.imageSize", json!({"width": 8, "height": 8})).unwrap();
        let e = s.execute("select.cryptomatte", json!({"x": 0, "y": 0})).unwrap_err().to_string();
        assert!(e.contains("no longer line up"), "{e}");
        // Back on the opened grid, the same click selects again.
        s.execute("edit.undo", json!({})).unwrap();
        let r = s.execute("select.cryptomatte", json!({"x": 0, "y": 0})).unwrap();
        assert_eq!(r["name"], "cube");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn nearby_float_ids_stay_separate_objects() {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!("pcraft-cm-near-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("near.exr");
        // Two IDs a hair apart numerically: a distance-based grouping (the old `tolerance`
        // epsilon) lumped them together, but they are unrelated hashes.
        let a = 0.1f32;
        let b = a + 1e-7;
        cryptomatte_file_at(&path, (0, 0, 4, 4), (0, 0, 4, 4), (a, b));
        let mut s = Session::new();
        let bytes = std::fs::read(&path).unwrap();
        let r = photocraft_io::import("near.exr", &bytes).unwrap();
        let (i, _) = s.open_document(r.document, Some(path.to_string_lossy().into_owned()));
        s.set_active(i);
        let r = s.execute("select.cryptomatte", json!({"x": 0, "y": 0})).unwrap();
        assert_eq!(r["name"], "cube", "{r}");
        assert_eq!(r["selected"], 8, "only the clicked object's pixels: {r}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn hostile_params_error_never_panic() {
        let (mut s, dir) = session_with_crypto();
        for p in [
            json!({"x": -5, "y": 0}),
            json!({"x": 0, "y": 99}),
            json!({"x": 0, "y": 0, "layer": "nope"}),
            json!({"x": 0, "y": 0, "layer": 5}),
            json!({"x": 0, "y": 0, "mode": "explode"}),
            json!({}),
            json!({"x": "zero", "y": 0}),
        ] {
            let e = s.execute("select.cryptomatte", p).unwrap_err().to_string();
            assert!(!e.is_empty());
        }
        // A wrong-typed `layer` is a parameter error naming the field, not a silent default.
        let e = s.execute("select.cryptomatte", json!({"x": 0, "y": 0, "layer": 5})).unwrap_err().to_string();
        assert!(e.contains("`layer` must be a string"), "{e}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A fixture whose beauty varies per pixel (a flip or rotation moves the sample points)
    /// and whose Cryptomatte covers the left two columns (cube), only column 2 (sphere, 0.5)
    /// and nothing in column 3. With `alpha_border` the beauty's first two columns are
    /// transparent, like a CG render's empty frame edges.
    fn varied_crypto_file(path: &std::path::Path, alpha_border: bool) {
        let (w, h) = (4usize, 4usize);
        let cube = photocraft_codecs::cryptomatte_id("cube");
        let sphere = photocraft_codecs::cryptomatte_id("sphere");
        let (mut r, mut g, mut b, mut a) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let (mut s0, mut c0, mut s1, mut c1) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for y in 0..h {
            for x in 0..w {
                r.push(((x + 4 * y) as f32 + 0.0) / 16.0);
                g.push(((x + 4 * y) as f32 + 0.25) / 16.0);
                b.push(((x + 4 * y) as f32 + 0.5) / 16.0);
                a.push(if alpha_border && x < 2 { 0.0 } else { 1.0 });
                let (left, mid) = (x < 2, x == 2);
                s0.push(if left { cube } else { 0.0 });
                c0.push(if left { 1.0 } else { 0.0 });
                s1.push(if mid { sphere } else { 0.0 });
                c1.push(if mid { 0.5 } else { 0.0 });
            }
        }
        let mk = |n: &str, v: Vec<f32>| AnyChannel::new(n, FlatSamples::F32(v));
        let list = vec![
            mk("R", r),
            mk("G", g),
            mk("B", b),
            mk("A", a),
            mk("crypto_asset00.red", s0),
            mk("crypto_asset00.green", c0),
            mk("crypto_asset01.red", s1),
            mk("crypto_asset01.green", c1),
        ];
        let key = photocraft_codecs::cryptomatte_key("crypto_asset");
        let manifest = format!("{{\"cube\":\"{:08x}\",\"sphere\":\"{:08x}\"}}", cube.to_bits(), sphere.to_bits());
        let mut attrs = LayerAttributes::named("crypto_asset");
        attrs.other.insert(format!("cryptomatte/{key}/name").as_str().into(), AttributeValue::Text("crypto_asset".into()));
        attrs.other.insert(format!("cryptomatte/{key}/manifest").as_str().into(), AttributeValue::Text(manifest.as_str().into()));
        let layer = Layer::new(
            (w, h),
            attrs,
            Encoding { compression: Compression::Uncompressed, blocks: Blocks::ScanLines, line_order: LineOrder::Increasing },
            AnyChannels::sort(list.into_iter().collect()),
        );
        let image = Image::from_layers(ImageAttributes::new(IntegerBounds::new((0, 0), (w, h))), vec![layer]);
        image.write().to_buffered(&mut std::io::BufWriter::new(std::fs::File::create(path).expect("fixture"))).expect("write fixture");
    }

    fn session_with_varied_crypto(alpha_border: bool) -> (Session, std::path::PathBuf) {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!("pcraft-cm-varied-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("render.exr");
        varied_crypto_file(&path, alpha_border);
        let mut s = Session::new();
        let bytes = std::fs::read(&path).unwrap();
        let r = photocraft_io::import("render.exr", &bytes).unwrap();
        let (i, _) = s.open_document(r.document, Some(path.to_string_lossy().into_owned()));
        s.set_active(i);
        (s, dir)
    }

    /// The click maps through the import's placement, never pixel content: with a fully
    /// transparent border the old content-bounds origin pointed two columns into the image,
    /// so a click on the sphere returned the cube and a click on the border errored (#2811's
    /// review). Now both click what they hit.
    #[test]
    fn a_transparent_border_does_not_shift_the_click() {
        let (mut s, dir) = session_with_varied_crypto(true);
        let r = s.execute("select.cryptomatte", json!({"x": 2, "y": 0})).unwrap();
        assert_eq!(r["name"], "sphere", "{r}");
        assert_eq!(r["selected"], 4, "column 2's four pixels: {r}");
        let sel = s.active().unwrap().doc.selection.as_ref().expect("a selection");
        assert!((sel.pixel(2, 0)[0] - 0.5).abs() < 1e-2, "{}", sel.pixel(2, 0)[0]);
        assert!(sel.pixel(3, 0)[0] < 1e-2, "the empty column stays unselected");
        // A click inside the transparent border selects its coverage instead of erroring.
        let r = s.execute("select.cryptomatte", json!({"x": 0, "y": 0})).unwrap();
        assert_eq!(r["name"], "cube", "{r}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Flips and rotations keep the bounds and move the pixels: the pinned sample points
    /// catch them and the click refuses instead of masking the wrong grid (#2811's review).
    #[test]
    fn flip_and_rotate_refuse_instead_of_misplacing() {
        let (mut s, dir) = session_with_varied_crypto(false);
        for op in ["image.imageRotation.flipCanvasHorizontal", "image.imageRotation.180"] {
            s.execute(op, json!({})).unwrap();
            let e = s.execute("select.cryptomatte", json!({"x": 0, "y": 0})).unwrap_err().to_string();
            assert!(e.contains("no longer matches"), "{op}: {e}");
            s.execute("edit.undo", json!({})).unwrap();
        }
        // Back on the opened grid, the same click selects again.
        let r = s.execute("select.cryptomatte", json!({"x": 0, "y": 0})).unwrap();
        assert_eq!(r["name"], "cube", "{r}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A re-render written to the same path is a different file: the identity pin refuses
    /// instead of selecting from the new render behind the open document's back (#2811's
    /// review).
    #[test]
    fn a_rerendered_source_refuses() {
        let (mut s, dir) = session_with_varied_crypto(false);
        let path = dir.join("render.exr");
        // A click first, so the source is pinned; then the same path holds new pixels.
        let r = s.execute("select.cryptomatte", json!({"x": 0, "y": 0})).unwrap();
        assert_eq!(r["name"], "cube");
        let mut mirrored = std::fs::read(&path).unwrap();
        // Same length, different content: the beauty mirrored by rewriting the R/G/B f32
        // samples of each scanline in place is invasive; instead write the varied fixture
        // again with the border flag flipped (different alpha column), which is a rewrite
        // of the very same length.
        drop(&mut mirrored);
        varied_crypto_file(&path, true);
        let e = s.execute("select.cryptomatte", json!({"x": 0, "y": 0})).unwrap_err().to_string();
        assert!(e.contains("changed since it was opened") || e.contains("no longer matches"), "{e}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Clicking empty space intersects the selection with nothing: replace and intersect
    /// empty it, add and subtract keep it (#2811's review).
    #[test]
    fn an_empty_click_empties_replace_and_intersect_selections() {
        let (mut s, dir) = session_with_varied_crypto(false);
        s.execute("select.cryptomatte", json!({"x": 0, "y": 0})).unwrap();
        // Column 3 holds no coverage: intersecting with it leaves nothing selected.
        let r = s.execute("select.cryptomatte", json!({"x": 3, "y": 0, "mode": "intersect"})).unwrap();
        assert!(r["id"].is_null(), "{r}");
        assert_eq!(r["layer"], "crypto_asset", "the empty result names its layer: {r}");
        assert!(s.active().unwrap().doc.selection.is_none(), "intersect with nothing deselects");
        // Add of nothing keeps the previous selection.
        s.execute("select.cryptomatte", json!({"x": 0, "y": 0})).unwrap();
        s.execute("select.cryptomatte", json!({"x": 3, "y": 0, "mode": "add"})).unwrap();
        let sel = s.active().unwrap().doc.selection.as_ref().expect("kept by add");
        assert!(sel.pixel(0, 0)[0] > 0.9);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Without a document the command is not enabled (`native_doc`, #2811's review).
    #[test]
    fn the_command_needs_a_document() {
        let spec = super::specs().into_iter().find(|s| s.id == "select.cryptomatte").expect("spec");
        let err = (spec.enabled)(&Session::new()).expect_err("no document, not enabled");
        assert!(!err.is_empty());
    }
}
