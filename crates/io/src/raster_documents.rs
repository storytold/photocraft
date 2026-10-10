//! Delivery containers: a single raster image in SVG or a single raster page in PDF.
//! No vector/font/layer preservation is claimed; PDF is export-only.
use crate::{ExportOptions, ExportResult, IoError};
use base64::Engine;
use photocraft_codecs::{ChannelLayout, Format, SampleType};
use photocraft_doc::Document;
use std::io::Write;
fn error(s: impl std::fmt::Display) -> IoError {
    IoError::Unsupported(format!("raster document export: {s}"))
}
pub(crate) fn export(doc: &Document, ext: &str, opts: &ExportOptions) -> Result<ExportResult, IoError> {
    let (w, h) = (doc.size.width, doc.size.height);
    photocraft_codecs::Limits { max_alloc: 256 << 20, ..Default::default() }.check(w, h, ChannelLayout::Rgba, SampleType::U16)?;
    let png = crate::flat::export_flat(doc, Format::Png, opts)?;
    let mut warnings = png.warnings;
    warnings.push(format!("{} output embeds one raster image; editable layers, paths and fonts are not retained", ext.to_ascii_uppercase()));
    let bytes = if ext == "svg" || ext == "svgz" {
        let encoded = base64::engine::general_purpose::STANDARD.encode(&png.bytes);
        let source = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\"><image width=\"{w}\" height=\"{h}\" xlink:href=\"data:image/png;base64,{encoded}\"/></svg>"
        );
        if ext == "svgz" {
            let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            gzip.write_all(source.as_bytes()).map_err(error)?;
            gzip.finish().map_err(error)?
        } else {
            source.into_bytes()
        }
    } else {
        if !doc.resolution_dpi.is_finite() || doc.resolution_dpi <= 0.0 {
            return Err(error("PDF needs a finite, positive document resolution"));
        }
        let image = photocraft_codecs::decode(&png.bytes)?;
        let depth = if image.sample_type() == SampleType::U16 { SampleType::U16 } else { SampleType::U8 };
        let layout = if image.layout().is_gray() { ChannelLayout::GrayA } else { ChannelLayout::Rgba };
        let image = image.convert(layout, depth);
        let points = (f64::from(w) * 72.0 / f64::from(doc.resolution_dpi), f64::from(h) * 72.0 / f64::from(doc.resolution_dpi));
        if points.0 > 14400.0 || points.1 > 14400.0 {
            return Err(error("PDF page exceeds 200 inches; increase document resolution"));
        }
        pdf(&image, points, opts.encode.embed_icc)?
    };
    Ok(ExportResult { bytes, warnings })
}
fn deflate(bytes: &[u8]) -> Result<Vec<u8>, IoError> {
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(bytes).map_err(error)?;
    z.finish().map_err(error)
}
fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut v = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
    v.extend_from_slice(data);
    v.extend_from_slice(b"\nendstream");
    v
}
fn pdf(image: &photocraft_codecs::Image, points: (f64, f64), embed_icc: bool) -> Result<Vec<u8>, IoError> {
    let (w, h) = image.dimensions();
    let ch = image.layout().channels();
    let bps = image.sample_type().bytes();
    let bits = bps * 8;
    let mut colors = Vec::new();
    let mut alpha = Vec::new();
    for px in image.data().chunks_exact(ch * bps) {
        for (c, v) in px.chunks_exact(bps).enumerate() {
            let destination = if c == ch - 1 { &mut alpha } else { &mut colors };
            if bps == 1 {
                destination.extend_from_slice(v);
            } else {
                destination.extend_from_slice(&u16::from_ne_bytes(v.try_into().map_err(error)?).to_be_bytes());
            }
        }
    }
    let color = deflate(&colors)?;
    let mask = deflate(&alpha)?;
    let device = if ch == 2 { "/DeviceGray" } else { "/DeviceRGB" };
    let profile = image.icc.as_ref().filter(|_| embed_icc);
    let space = if profile.is_some() { "[/ICCBased 7 0 R]" } else { device };
    let draw = format!("q {} 0 0 {} 0 0 cm /Image Do Q", points.0, points.1);
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Resources << /XObject << /Image 5 0 R >> >> /Contents 4 0 R >>", points.0, points.1)
            .into_bytes(),
        stream("", draw.as_bytes()),
        stream(
            &format!("/Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace {space} /BitsPerComponent {bits} /Filter /FlateDecode /SMask 6 0 R"),
            &color,
        ),
        stream(&format!("/Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceGray /BitsPerComponent {bits} /Filter /FlateDecode"), &mask),
    ];
    if let Some(profile) = profile {
        objects.push(stream(&format!("/N {} /Alternate {device} /Filter /FlateDecode", ch - 1), &deflate(profile)?));
    }
    let mut out = b"%PDF-1.6\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(object);
        out.extend_from_slice(b"\nendobj\n");
    }
    let start = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{start}\n%%EOF\n", objects.len() + 1).as_bytes());
    Ok(out)
}
