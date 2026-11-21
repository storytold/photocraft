use photocraft_codecs::TiffCompression;
use photocraft_io::ExportOptions;
use serde_json::Value;
use std::io::Write;

use crate::{Result, args::ExportArgs};

/// The `--format` family's options. A bad value never gets here: clap refused it.
impl From<&ExportArgs> for ExportOptions {
    fn from(a: &ExportArgs) -> Self {
        // TIFF output is flat unless asked: scripted conversions keep predictable sizes.
        let mut o = ExportOptions {
            tiff_layers: a.tiff_layers,
            tiff_transparency: a.tiff_transparency,
            tga_bits: a.tga_bits,
            pcraft_full_composite: a.pcraft_full_composite,
            ..Default::default()
        };
        if let Some(q) = a.quality {
            o.encode.jpeg_quality = q;
            // Asking for a quality asks for a lossy WebP; the default WebP stays lossless.
            o.encode.webp_quality = q;
            o.encode.webp_lossless = false;
        }
        o.encode.dpx_bits = a.dpx_bits;
        match a.tiff_compression {
            Some(crate::args::CliTiffCompression::None) => o.encode.tiff_compression = TiffCompression::None,
            Some(crate::args::CliTiffCompression::Lzw) => o.encode.tiff_compression = TiffCompression::Lzw,
            Some(crate::args::CliTiffCompression::Zip) => o.encode.tiff_compression = TiffCompression::Deflate,
            Some(crate::args::CliTiffCompression::G4) => o.tiff_ccitt_g4 = true,
            None => {}
        }
        o.encode.tiff_pyramid = a.tiff_pyramid;
        if let Some(f) = a.jpeg_format {
            o.encode.jpeg_format = f;
        }
        if let Some(c) = a.dds_compression {
            o.encode.dds.compression = c;
        }
        o.encode.dds.mipmaps = a.dds_mipmaps;
        o.psd_maximize_compatibility = !a.no_maximize_compatibility;
        // Conversions keep the whole packet unless asked, as Save As does (#733, #9466).
        if let Some(m) = a.metadata {
            o.xmp = m;
        }
        o
    }
}

pub(crate) fn warn_all(err: &mut dyn Write, ws: &[String]) {
    for w in ws {
        let _ = writeln!(err, "warning: {w}");
    }
}

pub(crate) fn warn_dedup(err: &mut dyn Write, ws: &[String], seen: &mut std::collections::HashSet<String>) {
    for w in ws {
        if seen.insert(w.clone()) {
            let _ = writeln!(err, "warning: {w}");
        }
    }
}

/// How [`print_json`] lays the JSON out.
pub(crate) enum JsonStyle {
    Pretty,
    Compact,
}

pub(crate) fn print_json(out: &mut dyn Write, v: &Value, style: JsonStyle) -> Result {
    let s = match style {
        JsonStyle::Pretty => serde_json::to_string_pretty(v),
        JsonStyle::Compact => serde_json::to_string(v),
    }?;
    Ok(writeln!(out, "{s}")?)
}
