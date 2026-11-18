//! Save for Web's preview encode: export dimensions, the proxy resample rule, the generated palette
//! and the indexed encode a palette edit recolours without re-quantizing.
use std::sync::Arc;

use photocraft_engine::web_cmds::{self, Optimized, WebSettings};
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// Export dimensions, using the same percent/width/height precedence as web_document.
pub fn dimensions(doc: &photocraft_doc::Document, p: &Value) -> Result<(f64, f64), String> {
    let (w0, h0) = (f64::from(doc.size.width).max(1.0), f64::from(doc.size.height).max(1.0));
    let positive = |k: &str| p.get(k).and_then(Value::as_f64).filter(|v| v.is_finite() && *v > 0.0);
    let (w, h) = match (positive("percent"), positive("width"), positive("height")) {
        (Some(pct), _, _) => (w0 * pct / 100.0, h0 * pct / 100.0),
        (_, Some(w), Some(h)) => (w, h),
        (_, Some(w), None) => (w, h0 * w / w0),
        (_, None, Some(h)) => (w0 * h / h0, h),
        _ => (w0, h0),
    };
    if !w.is_finite() || !h.is_finite() || w > 300_000.0 || h > 300_000.0 {
        return Err("export dimensions exceed the 300,000 px per-side limit".into());
    }
    Ok((w.round().max(1.0), h.round().max(1.0)))
}

/// The method a preview proxy is resampled with: the export's own when the proxy is at full
/// size, so the preview, palette and size estimate show it; bilinear for a reduced proxy.
pub fn proxy_resample(p: &Value, scale: f64) -> &str {
    if scale >= 1.0 { p.get("resample").and_then(Value::as_str).unwrap_or("bicubic") } else { "bilinear" }
}

/// Adds the generated `colorTable` to the export parameters `p` of an indexed format that has none, so the file
/// holds the palette the preview showed. Only palette-affecting options shape it; the table is built from a
/// proxy of at most 2048 px.
pub fn prepare_table(app: &PhotocraftApp, p: &mut Value) -> Result<(), String> {
    if !matches!(p.get("format").and_then(Value::as_str).unwrap_or("jpeg"), "gif" | "png8") || p.get("colorTable").is_some() {
        return Ok(());
    }
    let st = app.session.active().ok_or("no document")?;
    let mut source = p.clone();
    let Some(m) = source.as_object_mut() else { return Err("invalid web settings".into()) };
    m.retain(|k, _| {
        matches!(
            k.as_str(),
            "preset" | "format" | "palette" | "colors" | "transparency" | "matte" | "webSnap" | "convertToSrgb" | "width" | "height" | "percent" | "resample"
        )
    });
    let settings = WebSettings::from_params(&source, "preview").map_err(|e| e.to_string())?;
    let (w, h) = dimensions(&st.doc, &source)?;
    let scale = (2048.0 / w.max(h)).min(1.0);
    let mut proxy = source.clone();
    if let Some(m) = proxy.as_object_mut() {
        m.remove("percent");
        m.insert("width".into(), json!((w * scale).round().max(1.0)));
        m.insert("height".into(), json!((h * scale).round().max(1.0)));
        m.insert("resample".into(), json!(proxy_resample(&source, scale)));
    }
    let (doc, _, _) = web_cmds::web_document(&st.doc, &proxy, &settings).map_err(|e| e.to_string())?;
    let buf = photocraft_compose::flatten(&doc);
    let mut optimized = web_cmds::optimize(&buf.px, doc.size.width as usize, doc.bounds(), &settings, None, None, None, doc.resolution_dpi, false)
        .map_err(|e| e.to_string())?;
    // Tiny transparent features may disappear in the proxy; keep room for their index.
    if settings.transparency {
        optimized.color_table.truncate(255);
    }
    p["colorTable"] = json!(optimized.color_table.iter().map(|c| web_cmds::rgb_hex(*c)).collect::<Vec<_>>());
    Ok(())
}

/// One pane's encode: the recoloured preview pixels (RGBA of the proxy), the file's size, the palette the encode
/// chose (without the user's edits) and the proxy's size.
pub struct EncodedPreview {
    /// The proxy's pixels with the palette edits applied.
    pub rgba: Vec<u8>,
    /// The encoded size of the proxy in bytes.
    pub bytes: usize,
    /// The palette size of an indexed format.
    pub colors: Option<usize>,
    /// The palette the encode chose, as red, green and blue.
    pub palette: Vec<[u8; 3]>,
    /// The proxy's width in pixels.
    pub width: u32,
    /// The proxy's height in pixels.
    pub height: u32,
}

/// Encodes the proxy `source` makes with `p`; `source` gets the settings of `p` without the palette edits. Palette
/// replacements reuse the cached index buffer (kept under `key` while `signature` and the other options are
/// unchanged): picker drags do not re-quantize or re-encode the proxy, and `source` does not run then. The GIF payload length is unchanged by RGB table replacements. The edits
/// (`colorTableEdits`) index the palette the encode chose, unless `p` fixes a `colorTable` of its own.
pub fn optimized(
    ctx: &egui::Context,
    key: egui::Id,
    signature: &str,
    p: &Value,
    source: impl FnOnce(&WebSettings) -> Result<(Vec<[f32; 4]>, u32, u32), String>,
) -> Result<EncodedPreview, String> {
    let mut base = p.clone();
    let edits = base.as_object_mut().and_then(|m| m.remove("colorTableEdits"));
    let signature = format!("{signature}|{base}");
    let key = key.with("indexed-source");
    let cached: Option<(String, Arc<Optimized>)> = ctx.data(|d| d.get_temp(key));
    let encoded = if let Some((_, encoded)) = cached.filter(|(sig, _)| sig == &signature) {
        encoded
    } else {
        let settings = WebSettings::from_params(&base, "preview").map_err(|e| e.to_string())?;
        let (px, w, h) = source(&settings)?;
        let encoded = Arc::new(
            web_cmds::optimize(&px, w as usize, photocraft_geom::Rect::new(0, 0, w as i32, h as i32), &settings, None, None, None, 72.0, true)
                .map_err(|e| e.to_string())?,
        );
        ctx.data_mut(|d| d.insert_temp(key, (signature, encoded.clone())));
        encoded
    };
    let mut full = base;
    // A format without a palette has nothing to edit.
    if let Some(edits) = edits.filter(|_| full.get("colorTable").is_some() || !encoded.color_table.is_empty()) {
        if full.get("colorTable").is_none() {
            full["colorTable"] = json!(encoded.color_table.iter().map(|c| web_cmds::rgb_hex(*c)).collect::<Vec<_>>());
        }
        full["colorTableEdits"] = edits;
    }
    let settings = WebSettings::from_params(&full, "preview").map_err(|e| e.to_string())?;
    let rgba = if settings.color_table_edits.is_empty() || !settings.format.indexed() {
        encoded.preview.clone()
    } else {
        encoded
            .indices
            .iter()
            .flat_map(|index| {
                let index = usize::from(*index);
                let color = settings.color_table_edits.get(&index).copied().or_else(|| encoded.color_table.get(index).copied()).unwrap_or([0; 3]);
                [color[0], color[1], color[2], if Some(index) == encoded.transparent_index { 0 } else { 255 }]
            })
            .collect()
    };
    Ok(EncodedPreview {
        rgba,
        bytes: encoded.bytes.len(),
        colors: encoded.colors,
        palette: encoded.color_table.clone(),
        width: encoded.width as u32,
        height: encoded.height as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_picker_reuses_indices_and_updates_preview_pixels() {
        let ctx = egui::Context::default();
        let key = egui::Id::unique("test-web-preview");
        let px = vec![[1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0]];
        let run = |p: &Value| optimized(&ctx, key, "fixture", p, |_| Ok((px.clone(), 2, 1))).unwrap().rgba;
        let cached = || ctx.data(|d| d.get_temp::<(String, Arc<Optimized>)>(key.with("indexed-source"))).unwrap().1;
        let mut p = json!({"format": "gif", "colorTable": ["#ff0000", "#0000ff"], "dither": "none"});
        assert_eq!(run(&p), [255, 0, 0, 255, 0, 0, 255, 255]);
        let before = cached();
        p["colorTableEdits"] = json!({"0": "#00ff00"});
        assert_eq!(run(&p), [0, 255, 0, 255, 0, 0, 255, 255]);
        assert!(Arc::ptr_eq(&before, &cached()), "picker drag must not re-encode");
    }

    #[test]
    fn absurd_dimensions_are_refused() {
        let doc =
            photocraft_doc::Document::new("fixture", photocraft_geom::Size::new(100, 50), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        assert!(dimensions(&doc, &json!({"percent": 1e308})).is_err());
        assert!(dimensions(&doc, &json!({"width": 1e300})).is_err());
    }
}
