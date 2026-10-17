use photocraft_io::ExportOptions;
use serde_json::Value;
use std::io::Write;

use crate::{R, args::Args};

pub(crate) fn export_opts(a: &Args) -> Result<ExportOptions, String> {
    // TIFF output is flat unless asked: scripted conversions keep predictable sizes.
    let mut o = ExportOptions { tiff_layers: a.has("--tiff-layers"), ..Default::default() };
    if let Some(q) = a.get("--quality") {
        let q = q.parse().ok().filter(|q| (1..=100).contains(q)).ok_or_else(|| format!("bad --quality `{q}`: expected a whole number from 1 to 100"))?;
        o.encode.jpeg_quality = q;
        // Asking for a quality asks for a lossy WebP; the default WebP stays lossless.
        o.encode.webp_quality = q;
        o.encode.webp_lossless = false;
    }
    if let Some(b) = a.get("--tga-bits") {
        o.tga_bits = Some(b.parse().ok().and_then(photocraft_io::TgaBits::from_bits).ok_or_else(|| format!("bad --tga-bits `{b}`: expected 24 or 32"))?);
    }
    if let Some(b) = a.get("--dpx-bits") {
        o.encode.dpx_bits =
            Some(b.parse().ok().and_then(photocraft_io::DpxBits::from_bits).ok_or_else(|| format!("bad --dpx-bits `{b}`: expected 8, 10, 12 or 16"))?);
    }
    Ok(o)
}

pub(crate) fn warn_all(err: &mut dyn Write, ws: &[String]) {
    for w in ws {
        let _ = writeln!(err, "warning: {w}");
    }
}

/// The engine knows which type families aren't installed. Warn before an export
/// uses a fallback face rather than silently changing the appearance of the text.
pub(crate) fn missing_font_warnings(fonts: Vec<String>) -> Vec<String> {
    fonts.into_iter().map(|font| format!("font '{font}' is not installed; text may render using a fallback face")).collect()
}

pub(crate) fn print_json(out: &mut dyn Write, v: &Value, compact: bool) -> R {
    let s = if compact { serde_json::to_string(v) } else { serde_json::to_string_pretty(v) }.map_err(|e| e.to_string())?;
    writeln!(out, "{s}").map_err(|e| e.to_string())
}
