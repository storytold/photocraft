//! File › Import › Scan… puts a scanned page into a new document.
//!
//! The desktop app acquires pixels from a scanner and calls [`open_scanned`]. The command itself
//! takes those pixels (base64), so agents and tests can open a scan without hardware.

use base64::Engine;
use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::{Document, Size};
use photocraft_geom::Rect;
use serde_json::{Value, json};

use crate::{EngineError, Result, Session};

/// 128 megapixels: 512MB of RGBA, the same cap as the scanner crate.
const MAX_PIXELS: u64 = 128 * 1024 * 1024;
const MAX_DIMENSION: u32 = 300_000;
const ROWS: u32 = 64;

/// Pixels of one scanned page.
pub enum ScannedPixels {
    /// 8-bit RGBA, row by row, `width * height * 4` bytes.
    Rgba(Vec<u8>),
    /// 8-bit gray, row by row, `width * height` bytes. Alpha is filled opaque.
    Gray(Vec<u8>),
}

/// A page to open as its own document.
pub struct ScannedImage {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub resolution_dpi: f32,
    pub pixels: ScannedPixels,
}

/// Open `image` as a new RGB or grayscale document and make it active. Returns its index.
pub fn open_scanned(session: &mut Session, image: ScannedImage) -> Result<usize> {
    let ScannedImage { name, width, height, resolution_dpi, pixels } = image;
    check_size(width, height)?;
    let (mode, channels) = match &pixels {
        ScannedPixels::Rgba(bytes) => {
            expect_len(bytes.len(), width, height, 4)?;
            (ColorMode::Rgb, 4)
        }
        ScannedPixels::Gray(bytes) => {
            expect_len(bytes.len(), width, height, 1)?;
            (ColorMode::Grayscale, 2)
        }
    };
    let mut doc = Document::with_background(name, Size::new(width, height), mode, SampleType::U8, Color::WHITE);
    doc.resolution_dpi = resolution_dpi.clamp(1.0, 30_000.0);
    {
        let layer = doc.layers.first_mut().ok_or_else(|| bad("the new document has no background"))?;
        let surface = crate::pixels_mut(layer)?;
        write_rows(surface, width, height, channels, &pixels)?;
    }
    Ok(session.add_document(doc, None))
}

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: "file.import.scan".into(), msg: msg.into() }
}

fn check_size(width: u32, height: u32) -> Result<()> {
    if !(1..=MAX_DIMENSION).contains(&width) || !(1..=MAX_DIMENSION).contains(&height) {
        return Err(bad(format!("width and height must be within 1..={MAX_DIMENSION}")));
    }
    let pixels = u64::from(width).checked_mul(u64::from(height)).ok_or_else(|| bad("the scan is too large"))?;
    if pixels > MAX_PIXELS {
        return Err(bad("the scan is too large"));
    }
    Ok(())
}

fn expect_len(got: usize, width: u32, height: u32, channels: u32) -> Result<()> {
    let expected =
        u64::from(width).checked_mul(u64::from(height)).and_then(|n| n.checked_mul(u64::from(channels))).ok_or_else(|| bad("the scan is too large"))?;
    if expected != got as u64 {
        return Err(bad(format!("expected {expected} pixel bytes, got {got}")));
    }
    Ok(())
}

fn write_rows(surface: &mut photocraft_raster::Surface, width: u32, height: u32, channels: usize, pixels: &ScannedPixels) -> Result<()> {
    let mut row = 0;
    let mut buf = Vec::new();
    while row < height {
        let rows = (height - row).min(ROWS);
        let count = (rows as usize).checked_mul(width as usize).ok_or_else(|| bad("the scan is too large"))?;
        buf.clear();
        match pixels {
            ScannedPixels::Rgba(bytes) => {
                let start = (row as usize).checked_mul(width as usize).and_then(|n| n.checked_mul(4)).ok_or_else(|| bad("the scan is too large"))?;
                let end = start.checked_add(count.saturating_mul(4)).ok_or_else(|| bad("the scan is too large"))?;
                let src = bytes.get(start..end).ok_or_else(|| bad("the scan is too large"))?;
                buf.reserve(count.saturating_mul(4));
                for px in src.as_chunks::<4>().0 {
                    buf.extend([u8_to_f(px[0]), u8_to_f(px[1]), u8_to_f(px[2]), u8_to_f(px[3])]);
                }
            }
            ScannedPixels::Gray(bytes) => {
                let start = (row as usize).checked_mul(width as usize).ok_or_else(|| bad("the scan is too large"))?;
                let end = start.checked_add(count).ok_or_else(|| bad("the scan is too large"))?;
                let src = bytes.get(start..end).ok_or_else(|| bad("the scan is too large"))?;
                buf.reserve(count.saturating_mul(2));
                for sample in src {
                    buf.extend([u8_to_f(*sample), 1.0]);
                }
            }
        }
        if buf.len() != count * channels {
            return Err(bad("the scan is too large"));
        }
        let rect = Rect::from_xywh(0, row as i32, width, rows);
        surface.try_write_region(rect, &buf).map_err(|e| EngineError::Other(format!("not enough memory for the scan ({e})")))?;
        row += rows;
    }
    Ok(())
}

fn u8_to_f(v: u8) -> f32 {
    f32::from(v) / 255.0
}

fn positive(p: &Value, key: &str, max: u32) -> Result<u32> {
    let Some(value) = p.get(key) else { return Err(bad(format!("`{key}` is required"))) };
    let n = if let Some(n) = value.as_u64() {
        n
    } else if let Some(f) = value.as_f64() {
        if !f.is_finite() || f < 1.0 || f > f64::from(max) {
            return Err(bad(format!("`{key}` must be within 1..={max}")));
        }
        f.round() as u64
    } else {
        return Err(bad(format!("`{key}` must be a number")));
    };
    if n < 1 || n > u64::from(max) {
        return Err(bad(format!("`{key}` must be within 1..={max}")));
    }
    Ok(n as u32)
}

fn resolution(p: &Value) -> Result<f32> {
    let Some(value) = p.get("resolution") else { return Ok(300.0) };
    let n = value.as_f64().or_else(|| value.as_u64().map(|n| n as f64)).filter(|n| n.is_finite()).ok_or_else(|| bad("`resolution` must be a number"))?;
    if !(1.0..=30_000.0).contains(&n) {
        return Err(bad("`resolution` must be within 1..=30000"));
    }
    Ok(n as f32)
}

fn decode_bytes(p: &Value, key: &str) -> Result<Vec<u8>> {
    let text = p.get(key).and_then(Value::as_str).ok_or_else(|| bad(format!("`{key}` must be base64")))?;
    base64::engine::general_purpose::STANDARD.decode(text.trim()).map_err(|_| bad(format!("`{key}` is not valid base64")))
}

fn run(session: &mut Session, p: &Value) -> Result<Value> {
    let width = positive(p, "width", MAX_DIMENSION)?;
    let height = positive(p, "height", MAX_DIMENSION)?;
    let resolution_dpi = resolution(p)?;
    let name = p.get("name").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or("Scan").to_string();
    let mode = p.get("mode").and_then(Value::as_str).unwrap_or("rgb");
    let pixels = match mode {
        "rgb" => ScannedPixels::Rgba(decode_bytes(p, "rgba")?),
        "gray" | "grayscale" => ScannedPixels::Gray(decode_bytes(p, "gray")?),
        _ => return Err(bad("`mode` must be rgb or gray")),
    };
    let index = open_scanned(session, ScannedImage { name, width, height, resolution_dpi, pixels })?;
    Ok(json!({ "document": index }))
}

pub fn specs() -> Vec<crate::commands::CommandSpec> {
    vec![crate::commands::CommandSpec {
        id: "file.import.scan",
        label: "Scan…",
        menu: &["File", "Import"],
        shortcut: None,
        params: r#"{"name":str="Scan","width":1..300000,"height":1..300000,"resolution":1..30000=300,"mode":"rgb|gray"="rgb","rgba":base64 (mode=rgb, width*height*4 bytes),"gray":base64 (mode=gray, width*height bytes)} → {document}: open scanned pixels as a new document"#,
        enabled: |_s| Ok(()),
        journal: false,
        run,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    fn b64(bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    #[test]
    fn rgba_pixels_open_a_document() {
        let mut session = Session::new();
        let r =
            session.execute("file.import.scan", json!({"width": 2, "height": 1, "resolution": 300, "rgba": b64(&[255, 0, 0, 255, 0, 0, 255, 255])})).unwrap();
        assert_eq!(r["document"], 0);
        let doc = &session.active().unwrap().doc;
        assert_eq!(doc.size, Size::new(2, 1));
        assert_eq!(doc.mode, ColorMode::Rgb);
        assert!((doc.resolution_dpi - 300.0).abs() < f32::EPSILON);
        let px = doc.layers[0].surface().unwrap().pixel(0, 0);
        assert_eq!(px, vec![1.0, 0.0, 0.0, 1.0]);
        let px = doc.layers[0].surface().unwrap().pixel(1, 0);
        assert_eq!(px, vec![0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn gray_pixels_open_a_grayscale_document() {
        let mut session = Session::new();
        session.execute("file.import.scan", json!({"width": 1, "height": 1, "mode": "gray", "gray": b64(&[0])})).unwrap();
        let doc = &session.active().unwrap().doc;
        assert_eq!(doc.mode, ColorMode::Grayscale);
        let px = doc.layers[0].surface().unwrap().pixel(0, 0);
        assert_eq!(px, vec![0.0, 1.0]);
    }

    #[test]
    fn bad_params_do_not_open_a_document() {
        let mut session = Session::new();
        assert!(session.execute("file.import.scan", json!({})).is_err());
        assert!(session.execute("file.import.scan", json!({"width": 0, "height": 1, "rgba": ""})).is_err());
        assert!(session.execute("file.import.scan", json!({"width": 1, "height": 1, "mode": "cmyk", "rgba": "AAAA"})).is_err());
        assert!(session.execute("file.import.scan", json!({"width": 1, "height": 1, "rgba": "not base64!!"})).is_err());
        assert!(session.execute("file.import.scan", json!({"width": 2, "height": 2, "rgba": b64(&[0, 0, 0, 255])})).is_err());
        assert!(session.execute("file.import.scan", json!({"width": 20000, "height": 20000, "rgba": ""})).is_err());
        assert!(session.execute("file.import.scan", json!("nope")).is_err());
        assert!(session.documents().is_empty());
    }
}
