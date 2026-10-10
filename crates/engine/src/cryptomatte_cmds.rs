//! Cryptomatte selection (specification 1.2): click a pixel, and every pixel covered by the
//! same object ID becomes the selection — the Arnold/V-Ray/Redshift workflow on top of the
//! codec's [`photocraft_codecs::decode_cryptomatte`]. Works on the EXR the active document
//! was opened from (its parts came from there); the manifest names the object in the result.

use serde_json::{Value, json};

use crate::commands::{CommandSpec, int_i32};
use crate::{EngineError, Result, Session};

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: "select.cryptomatte".into(), msg: msg.into() }
}

/// `select.cryptomatte`: the ID at `x, y` in the Cryptomatte layer `layer` (default: the
/// first) of the document's source EXR, as a selection whose mask value is the ID's coverage
/// — partial coverage gives a partial selection, so edges land between both objects.
fn select_cryptomatte(s: &mut Session, p: &Value) -> Result<Value> {
    let coord = |k: &str| int_i32("select.cryptomatte", p, k).and_then(|v| v.ok_or_else(|| bad(format!("missing `{k}`"))));
    let (x, y) = (coord("x")?, coord("y")?);
    let mode = p.get("mode").and_then(Value::as_str).unwrap_or("replace").to_string();
    if !matches!(mode.as_str(), "replace" | "add" | "subtract" | "intersect") {
        return Err(bad(format!("mode is \"{mode}\"; it is replace, add, subtract or intersect")));
    }
    let tolerance = p.get("tolerance").and_then(Value::as_f64).unwrap_or(0.0).clamp(0.0, 1.0) as f32;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let Some(path) = st.path.clone() else {
        return Err(EngineError::Other("the document was not opened from a file; its Cryptomatte samples are unavailable".into()));
    };
    if !path.to_ascii_lowercase().ends_with(".exr") {
        return Err(EngineError::Other(format!("{path} is no EXR; Cryptomatte selection needs the source render")));
    }
    let bytes = crate::file_cmds::read_file(&path)?;
    let layers = photocraft_codecs::cryptomatte_layers(&bytes, &Default::default()).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    let layer = match p.get("layer").and_then(Value::as_str) {
        Some(want) => layers.iter().find(|l| l.name.eq_ignore_ascii_case(want)).ok_or_else(|| {
            EngineError::Other(format!("no Cryptomatte layer named {want} (have {})", layers.iter().map(|l| l.name.clone()).collect::<Vec<_>>().join(", ")))
        })?,
        None => layers.first().ok_or_else(|| EngineError::Other(format!("{path} holds no Cryptomatte layer")))?,
    };
    let buffer = photocraft_codecs::decode_cryptomatte(&bytes, &layer.name, &Default::default()).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    if x < 0 || y < 0 || x as u32 >= buffer.width || y as u32 >= buffer.height {
        return Err(bad(format!("({x}, {y}) lies outside the {}×{} Cryptomatte buffer", buffer.width, buffer.height)));
    }
    // The ID to select: the strongest sample at the clicked pixel.
    let clicked = buffer.at(x as u32, y as u32).first().copied().map(|(id, _)| id);
    let Some(want) = clicked else {
        // Nothing at the click: an empty selection (deselects in replace mode, like clicking
        // empty space with the Magic Wand).
        if mode == "replace" {
            s.edit("Cryptomatte", |doc, _| {
                doc.selection = None;
                Ok(())
            })?;
        }
        return Ok(json!({"id": null, "name": null, "selected": 0}));
    };
    // The coverage of `want` at every pixel becomes the selection mask.
    let name = layer.name_of(want).map(str::to_string);
    let doc_bounds = st.doc.bounds();
    let (ox, oy) = (buffer.width.saturating_sub(doc_bounds.width()) / 2, buffer.height.saturating_sub(doc_bounds.height()) / 2);
    let mut shape = photocraft_raster::Surface::new(photocraft_color::PixelFormat::GRAY8);
    let mut selected = 0u64;
    for py in 0..buffer.height {
        for px in 0..buffer.width {
            let cov =
                buffer.at(px, py).iter().find(|(id, _)| (id - want).abs() <= tolerance.max(f32::EPSILON * id.abs().max(1.0))).map(|(_, c)| *c).unwrap_or(0.0);
            if cov > 0.0 {
                shape.write_pixel(px as i32 - ox as i32 + doc_bounds.x0, py as i32 - oy as i32 + doc_bounds.y0, &[cov]);
                selected += 1;
            }
        }
    }
    s.edit("Cryptomatte", |doc, _| {
        let old = doc.selection.take();
        let combined = match (mode.as_str(), old) {
            ("add", Some(o)) => crate::commands::combine(&o, &shape, doc_bounds, |a, b| a.max(b)),
            ("subtract", Some(o)) => crate::commands::combine(&o, &shape, doc_bounds, |a, b| a * (1.0 - b)),
            ("intersect", Some(o)) => crate::commands::combine(&o, &shape, doc_bounds, |a, b| a.min(b)),
            ("subtract" | "intersect", None) => photocraft_raster::Surface::new(photocraft_color::PixelFormat::GRAY8),
            _ => shape.clone(),
        };
        doc.selection = (!combined.content_bounds().is_empty()).then_some(combined);
        Ok(())
    })?;
    Ok(json!({"id": want, "name": name, "selected": selected, "layer": layer.name}))
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
        r##"{"x":i32,"y":i32,"layer":str? (the Cryptomatte layer; default the first),"tolerance":0..1=0 (IDs within this distance of the clicked one),"mode":"replace|add|subtract|intersect"="replace"}"##,
        crate::file_cmds::native,
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
        let (w, h) = (4usize, 4usize);
        let cube = photocraft_codecs::cryptomatte_id("cube");
        let sphere = photocraft_codecs::cryptomatte_id("sphere");
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
        let image = Image::from_layers(ImageAttributes::new(IntegerBounds::new((0, 0), (w, h))), vec![layer]);
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
        // subtract undoes the sphere again.
        s.execute("select.cryptomatte", json!({"x": 3, "y": 0, "mode": "subtract"})).unwrap();
        let sel = s.active().unwrap().doc.selection.as_ref().unwrap();
        // 0.5 coverage minus itself: 0.5 * (1 - 0.5) = 0.25.
        assert!((sel.pixel(3, 0)[0] - 0.25).abs() < 1e-2, "{}", sel.pixel(3, 0)[0]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn hostile_params_error_never_panic() {
        let (mut s, dir) = session_with_crypto();
        for p in [
            json!({"x": -5, "y": 0}),
            json!({"x": 0, "y": 99}),
            json!({"x": 0, "y": 0, "layer": "nope"}),
            json!({"x": 0, "y": 0, "mode": "explode"}),
            json!({}),
            json!({"x": "zero", "y": 0}),
        ] {
            let e = s.execute("select.cryptomatte", p).unwrap_err().to_string();
            assert!(!e.is_empty());
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
