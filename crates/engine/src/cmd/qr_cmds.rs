//! QR code recognition (`qr.scan`): composite the document — or a region of it (the current
//! selection or an explicit `region`) — to pixels and decode every QR code in that area with the
//! pure-Rust `rqrr` decoder. A read-only query: it changes nothing and records no history step.
//!
//! Each code reports its content plus the bounding box of its three finder patterns, in document
//! coordinates, so the UI can show where the code is.

use photocraft_geom::Rect;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

/// Decode every QR code inside `r` (document coordinates, already clamped to the canvas).
/// A region entirely outside the canvas simply finds nothing.
fn scan_region(doc: &photocraft_doc::Document, r: Rect) -> Vec<Value> {
    if r.is_empty() {
        return Vec::new();
    }
    let full = photocraft_compose::composite_to_surface(doc, r, doc.pixel_format(), None);
    let (w, h) = (r.width() as usize, r.height() as usize);
    let mut rgba = vec![[0u8; 4]; w * h];
    full.read_rgba8_into(r, &mut rgba);
    // Luminance (ITU-R BT.601): rqrr works on plain greyscale, 0 = black, 255 = white.
    let gray: Vec<u8> = rgba
        .iter()
        .map(|p| ((2126 * u32::from(p[0]) + 7152 * u32::from(p[1]) + 722 * u32::from(p[2])) / 10000) as u8)
        .collect();
    let mut img = rqrr::PreparedImage::prepare_from_greyscale(w, h, |x, y| gray[y * w + x]);
    let mut codes = Vec::new();
    for grid in img.detect_grids() {
        let Ok((_, content)) = grid.decode() else {
            continue;
        };
        // `bounds` lists the three finder corners plus the fourth (any order); the box spans them.
        let xs = [grid.bounds[0].x, grid.bounds[1].x, grid.bounds[2].x, grid.bounds[3].x];
        let ys = [grid.bounds[0].y, grid.bounds[1].y, grid.bounds[2].y, grid.bounds[3].y];
        let x0 = xs.into_iter().min().unwrap_or(0);
        let y0 = ys.into_iter().min().unwrap_or(0);
        let x1 = xs.into_iter().max().unwrap_or(0);
        let y1 = ys.into_iter().max().unwrap_or(0);
        codes.push(json!({
            "x": f64::from(r.x0 + x0),
            "y": f64::from(r.y0 + y0),
            "width": (x1 - x0).max(1),
            "height": (y1 - y0).max(1),
            "content": content,
        }));
    }
    codes
}

fn scan(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "qr.scan";
    let d = s.active().ok_or(EngineError::NoDocument)?.doc.clone();
    let canvas = d.bounds();
    let r: Rect = if let Some(region) = p.get("region") {
        let num = |key: &str| -> Result<f64> {
            region.get(key).and_then(Value::as_f64).ok_or_else(|| bad(CMD, format!("`region` needs `{key}`"))).and_then(|f| {
                if f.is_finite() { Ok(f) } else { Err(bad(CMD, format!("`region.{key}` must be finite"))) }
            })
        };
        let (x, y, w, h) = (num("x")?, num("y")?, num("width")?, num("height")?);
        if w <= 0.0 || h <= 0.0 {
            return Err(bad(CMD, "`region` width and height must be positive"));
        }
        Rect::new(x as i32, y as i32, (x + w).ceil() as i32, (y + h).ceil() as i32).intersect(&canvas)
    } else if p.get("selectionOnly").and_then(Value::as_bool).unwrap_or(false) {
        d.selection.as_ref().map(|m| m.content_bounds().intersect(&canvas)).unwrap_or(Rect::EMPTY)
    } else {
        canvas
    };
    Ok(json!({"codes": scan_region(&d, r)}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "qr.scan",
        label: "Scan QR Code…",
        // No menu entry: the UI reaches it from the "QR" dock panel next to "Revision".
        menu: &[],
        shortcut: None,
        params: r##"{}|{"selectionOnly":true}|{"region":{"x":px,"y":px,"width":px,"height":px}} → codes (decode QR codes in the whole document, the selection, or a region)"##,
        enabled: has_doc,
        run: scan,
        journal: false,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{LayerContent, LayerId};
    use serde_json::json;
    use std::sync::Arc;

    /// Draw a QR code (encoded by the dev-only `qrcode` crate) on a new raster layer, its
    /// top-left module at document (20, 20), module size 8 px, quiet zone 4 modules.
    fn session_with_qr() -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 320, "height": 320})).unwrap();
        let id = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
        let code = qrcode::QrCode::new("https://example.com/qr?x=1").unwrap();
        let size = code.width();
        let dark: Vec<bool> = code.to_colors().into_iter().map(|c| c == qrcode::Color::Dark).collect();
        let (mod_size, off) = (8i32, 20i32);
        let quiet = 4i32;
        let d = s.active_mut().unwrap();
        let mut doc = (*d.doc).clone();
        let LayerContent::Raster(surface) = &mut doc.layer_mut(LayerId(id)).unwrap().content else {
            panic!("new layer is raster");
        };
        for my in 0..size as i32 {
            for mx in 0..size as i32 {
                if dark[(my * size as i32 + mx) as usize] {
                    let x = off + (mx + quiet) * mod_size;
                    let y = off + (my + quiet) * mod_size;
                    surface.fill_rect(Rect::new(x, y, x + mod_size, y + mod_size), &[0.0, 0.0, 0.0, 1.0]);
                }
            }
        }
        d.doc = Arc::new(doc);
        s
    }

    #[test]
    fn scan_finds_the_code() {
        let mut s = session_with_qr();
        let v = s.execute("qr.scan", json!({})).unwrap();
        let codes = v["codes"].as_array().unwrap();
        assert!(!codes.is_empty(), "the drawn QR code must be found: {v}");
        assert_eq!(codes[0]["content"], "https://example.com/qr?x=1");
        // The code's box lies within the canvas (the tile spans 20..316, quiet zone included).
        let x = codes[0]["x"].as_f64().unwrap();
        let y = codes[0]["y"].as_f64().unwrap();
        let w = codes[0]["width"].as_i64().unwrap();
        let h = codes[0]["height"].as_i64().unwrap();
        assert!(x >= 20.0 && y >= 20.0 && x + w as f64 <= 320.0 && y + h as f64 <= 320.0, "{codes:?}");
    }

    #[test]
    fn selection_only_scans_the_selection() {
        let mut s = session_with_qr();
        // Select only the quiet-zone corner of the tile: no finder pattern is inside, so nothing
        // decodes; the full-document scan still finds the code.
        s.execute("select.rect", json!({"x": 20, "y": 20, "width": 20, "height": 20})).unwrap();
        let v = s.execute("qr.scan", json!({"selectionOnly": true})).unwrap();
        assert!(v["codes"].as_array().unwrap().is_empty(), "no code inside the tiny selection");
        let v = s.execute("qr.scan", json!({})).unwrap();
        assert_eq!(v["codes"].as_array().unwrap()[0]["content"], "https://example.com/qr?x=1");
    }

    #[test]
    fn empty_document_finds_nothing() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
        let v = s.execute("qr.scan", json!({})).unwrap();
        assert!(v["codes"].as_array().unwrap().is_empty());
    }

    #[test]
    fn explicit_region_is_clamped() {
        let mut s = session_with_qr();
        // A region entirely outside the canvas is empty, not an error.
        let v = s.execute("qr.scan", json!({"region": {"x": 1000, "y": 1000, "width": 50, "height": 50}})).unwrap();
        assert!(v["codes"].as_array().unwrap().is_empty());
        // A bad region is a params error.
        assert!(s.execute("qr.scan", json!({"region": {"x": 0, "y": 0}})).is_err());
    }
}
