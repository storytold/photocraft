//! The PDF writer (ISO 32000-1 basics): one raster page per file.
//!
//! A small, self-contained writer used by two paths: File › Save a Copy / Save As › PDF (#3401)
//! through [`export`], and File › Print through [`print_pdf`] (called by the engine, which adds
//! paper geometry, printer marks and labels around the image). One page holds a
//! Flate-compressed image XObject, optional printer marks as vector operators and optional
//! Helvetica text; an embedded ICC profile rides in an ICCBased colour space. Multi-page PDF,
//! layer-preserving PDF and PDF import stay open (FILE-215-10).

use std::io::Write as _;

use photocraft_color::{ColorMode, PixelFormat};
use photocraft_doc::Document;

use crate::{ExportResult, IoError};

/// The image placed on a page: 8-bit interleaved samples, 1 (gray), 3 (RGB) or 4 (CMYK,
/// 1 = full ink), with the profile it was authored in if any.
pub struct PrintImage {
    pub width: u32,
    pub height: u32,
    pub channels: usize,
    pub data: Vec<u8>,
    pub icc: Option<Vec<u8>>,
}

/// Printer marks around the image.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Marks {
    pub corner_crop: bool,
    pub center_crop: bool,
    pub registration: bool,
}

/// One page: paper (pt), image box (pt, origin bottom-left), marks and texts.
pub struct PrintPage {
    pub paper: (f64, f64),
    pub image: PrintImage,
    pub rect: (f64, f64, f64, f64),
    pub marks: Marks,
    pub description: Option<String>,
    pub label: Option<String>,
}

fn pdf_string(s: &str) -> String {
    let mut o = String::from("(");
    for c in s.chars() {
        match c {
            '(' | ')' | '\\' => {
                o.push('\\');
                o.push(c);
            }
            c if c.is_ascii() && !c.is_ascii_control() => o.push(c),
            _ => o.push('?'),
        }
    }
    o.push(')');
    o
}

fn deflate(data: &[u8]) -> Vec<u8> {
    // Fast: print PDFs are transient and large. Level 3, not 1: zlib-rs level 1 writes static
    // Huffman blocks only (13-30% bigger on photos); level 3 is the lowest that is never bigger
    // than miniz's level 1 was, at about twice its time.
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(3));
    let _ = e.write_all(data);
    e.finish().unwrap_or_default()
}

/// Vector marks around `(x, y, w, h)`.
fn marks_ops(m: Marks, (x, y, w, h): (f64, f64, f64, f64)) -> String {
    let mut s = String::from("q 0 G 0.3 w\n");
    let (gap, len) = (6.0, 18.0);
    let line = |s: &mut String, x0: f64, y0: f64, x1: f64, y1: f64| s.push_str(&format!("{x0:.2} {y0:.2} m {x1:.2} {y1:.2} l S\n"));
    if m.corner_crop {
        for (cx, cy, dx, dy) in [(x, y, -1.0, -1.0), (x + w, y, 1.0, -1.0), (x, y + h, -1.0, 1.0), (x + w, y + h, 1.0, 1.0)] {
            line(&mut s, cx + dx * gap, cy, cx + dx * (gap + len), cy);
            line(&mut s, cx, cy + dy * gap, cx, cy + dy * (gap + len));
        }
    }
    if m.center_crop {
        let (mx, my) = (x + w / 2.0, y + h / 2.0);
        line(&mut s, mx, y - gap, mx, y - gap - len);
        line(&mut s, mx, y + h + gap, mx, y + h + gap + len);
        line(&mut s, x - gap, my, x - gap - len, my);
        line(&mut s, x + w + gap, my, x + w + gap + len, my);
    }
    if m.registration {
        let r = 5.0;
        let off = gap + len / 2.0;
        for (cx, cy) in [(x + w / 2.0, y - off), (x + w / 2.0, y + h + off), (x - off, y + h / 2.0), (x + w + off, y + h / 2.0)] {
            // Circle (four Béziers) plus crosshair.
            let k = 0.5523 * r;
            s.push_str(&format!(
                "{:.2} {cy:.2} m {:.2} {:.2} {:.2} {:.2} {cx:.2} {:.2} c {:.2} {:.2} {:.2} {:.2} {:.2} {cy:.2} c {:.2} {:.2} {:.2} {:.2} {cx:.2} {:.2} c {:.2} {:.2} {:.2} {:.2} {:.2} {cy:.2} c S\n",
                cx + r, cx + r, cy + k, cx + k, cy + r, cy + r, cx - k, cy + r, cx - r, cy + k, cx - r, cx - r, cy - k, cx - k, cy - r, cy - r, cx + k, cy - r, cx + r, cy - k, cx + r
            ));
            line(&mut s, cx - r * 1.6, cy, cx + r * 1.6, cy);
            line(&mut s, cx, cy - r * 1.6, cx, cy + r * 1.6);
        }
    }
    s.push_str("Q\n");
    s
}

/// A one-page PDF.
pub fn print_pdf(page: &PrintPage) -> Vec<u8> {
    write_pdf(page, None)
}

/// What a PDF/X-1a:2001 file adds to the page (ISO 15930-1): the output intent, the trim and
/// bleed boxes and the identification the standard requires.
struct XPage<'a> {
    /// The output profile (ICC v2, CMYK), embedded as the intent's `DestOutputProfile`.
    profile: &'a [u8],
    /// The registered characterization the profile describes (`FOGRA39`…), or `Custom`.
    condition_id: &'a str,
    /// The profile's description.
    condition: &'a str,
    title: &'a str,
    /// The trim box (pt, origin bottom-left); the media box is the bleed box.
    trim: (f64, f64, f64, f64),
    /// `D:YYYYMMDDHHmmSS+00'00'`.
    date: String,
    /// 32 hex digits for the trailer's `/ID`.
    id: String,
}

fn write_pdf(page: &PrintPage, x: Option<&XPage>) -> Vec<u8> {
    // PDF/X-1a:2001 is PDF 1.3.
    let mut out: Vec<u8> = if x.is_some() { b"%PDF-1.3\n%\xe2\xe3\xcf\xd3\n".to_vec() } else { b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec() };
    let mut offsets: Vec<usize> = Vec::new();
    let mut obj = |out: &mut Vec<u8>, body: &[u8]| {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", offsets.len()).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };
    let img = &page.image;
    let (pw, ph) = page.paper;
    let (x0, y0, w, h) = page.rect;
    // 1 catalog, 2 pages, 3 page, 4 contents, 5 image, 6 font, 7 ICC (optional). PDF/X has no
    // text (an unembedded font isn't allowed): 6 is the output profile and 7 the info dictionary.
    match x {
        Some(x) => obj(
            &mut out,
            format!(
                "<< /Type /Catalog /Pages 2 0 R /OutputIntents [<< /Type /OutputIntent /S /GTS_PDFX /OutputConditionIdentifier {} /OutputCondition {} /RegistryName (http://www.color.org) /Info {} /DestOutputProfile 6 0 R >>] >>",
                pdf_string(x.condition_id),
                pdf_string(x.condition),
                pdf_string(x.condition)
            )
            .as_bytes(),
        ),
        None => obj(&mut out, b"<< /Type /Catalog /Pages 2 0 R >>"),
    }
    obj(&mut out, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    match x {
        Some(x) => {
            let (tx0, ty0, tx1, ty1) = x.trim;
            obj(
                &mut out,
                format!(
                    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {pw:.2} {ph:.2}] /BleedBox [0 0 {pw:.2} {ph:.2}] /TrimBox [{tx0:.2} {ty0:.2} {tx1:.2} {ty1:.2}] /Resources << /XObject << /Im0 5 0 R >> >> /Contents 4 0 R >>"
                )
                .as_bytes(),
            );
        }
        None => obj(
            &mut out,
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {pw:.2} {ph:.2}] /Resources << /XObject << /Im0 5 0 R >> /Font << /F1 6 0 R >> >> /Contents 4 0 R >>"
            )
            .as_bytes(),
        ),
    }
    // An explicit white page, so every viewer (and rasteriser) shows paper, not transparency.
    let mut content = format!("q 1 g 0 0 {pw:.2} {ph:.2} re f Q\nq {w:.3} 0 0 {h:.3} {x0:.3} {y0:.3} cm /Im0 Do Q\n");
    content.push_str(&marks_ops(page.marks, page.rect));
    let any_marks = page.marks.corner_crop || page.marks.center_crop || page.marks.registration;
    let pad = if any_marks { 30.0 } else { 6.0 };
    if x.is_none() {
        if let Some(l) = page.label.as_deref().filter(|l| !l.is_empty()) {
            content.push_str(&format!("BT /F1 8 Tf 0 g {:.2} {:.2} Td {} Tj ET\n", x0, y0 + h + pad, pdf_string(l)));
        }
        if let Some(d) = page.description.as_deref().filter(|d| !d.is_empty()) {
            content.push_str(&format!("BT /F1 8 Tf 0 g {:.2} {:.2} Td {} Tj ET\n", x0, y0 - pad - 8.0, pdf_string(d)));
        }
    }
    let c = deflate(content.as_bytes());
    let mut body = format!("<< /Length {} /Filter /FlateDecode >>\nstream\n", c.len()).into_bytes();
    body.extend_from_slice(&c);
    body.extend_from_slice(b"\nendstream");
    obj(&mut out, &body);
    let device = match img.channels {
        1 => "/DeviceGray",
        4 => "/DeviceCMYK",
        _ => "/DeviceRGB",
    };
    // PDF/X-1a colour is device CMYK or gray, characterized by the output intent.
    let icc = img.icc.as_ref().filter(|_| x.is_none());
    let cs = if icc.is_some() { "[/ICCBased 7 0 R]".to_string() } else { device.to_string() };
    let z = deflate(&img.data);
    let mut body = format!(
        "<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace {cs} /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream\n",
        img.width,
        img.height,
        z.len()
    )
    .into_bytes();
    body.extend_from_slice(&z);
    body.extend_from_slice(b"\nendstream");
    obj(&mut out, &body);
    match x {
        Some(x) => {
            let z = deflate(x.profile);
            let mut body = format!("<< /N 4 /Filter /FlateDecode /Length {} >>\nstream\n", z.len()).into_bytes();
            body.extend_from_slice(&z);
            body.extend_from_slice(b"\nendstream");
            obj(&mut out, &body);
            let producer = format!("PhotoCraft {}", env!("CARGO_PKG_VERSION"));
            obj(
                &mut out,
                format!(
                    "<< /Title {} /Creator (PhotoCraft) /Producer {} /CreationDate {} /ModDate {} /Trapped /False /GTS_PDFXVersion (PDF/X-1:2001) /GTS_PDFXConformance (PDF/X-1a:2001) >>",
                    pdf_string(x.title),
                    pdf_string(&producer),
                    pdf_string(&x.date),
                    pdf_string(&x.date)
                )
                .as_bytes(),
            );
        }
        None => {
            obj(&mut out, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>");
            if let Some(icc) = icc {
                let z = deflate(icc);
                let mut body = format!("<< /N {} /Alternate {device} /Filter /FlateDecode /Length {} >>\nstream\n", img.channels, z.len()).into_bytes();
                body.extend_from_slice(&z);
                body.extend_from_slice(b"\nendstream");
                obj(&mut out, &body);
            }
        }
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes());
    for o in &offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    let extra = x.map_or(String::new(), |x| format!(" /Info 7 0 R /ID [<{}> <{}>]", x.id, x.id));
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R{extra} >>\nstartxref\n{xref}\n%%EOF\n", offsets.len() + 1).as_bytes());
    out
}

/// The document flattened over paper for a PDF page: `(channels, samples)`, one row after
/// another, 1 (gray), 3 (RGB) or 4 (CMYK, 1 = full ink) values per pixel in `0..=1`.
fn page_samples(doc: &Document) -> std::result::Result<(usize, Vec<f32>), IoError> {
    let (w, h) = (usize::try_from(doc.size.width).unwrap_or(0), usize::try_from(doc.size.height).unwrap_or(0));
    if w == 0 || h == 0 {
        return Err(IoError::Unsupported("PDF needs a document with non-zero width and height; nothing was written".into()));
    }
    let device = match doc.mode {
        ColorMode::Rgb => 3,
        ColorMode::Grayscale => 1,
        ColorMode::Cmyk => 4,
        m => return Err(IoError::Unsupported(format!("PDF saves RGB, Grayscale and CMYK documents; {m:?} needs converting to RGB first (Image > Mode)"))),
    };
    let fmt = doc.pixel_format();
    let n = fmt.channels().max(1);
    let canvas = photocraft_geom::Rect::new(0, 0, w as i32, h as i32);
    let surface = photocraft_compose::flatten_to_surface(doc, PixelFormat::new(fmt.mode, fmt.sample, true), None);
    let values = surface.read_region(canvas);
    // Straight alpha, composited here over paper: white in RGB/Gray (1 = full light), zero ink
    // in CMYK (0 = paper).
    let paper = if doc.mode == ColorMode::Cmyk { 0.0 } else { 1.0 };
    let mut data = Vec::with_capacity((w * h).saturating_mul(device));
    for px in values.chunks(n) {
        let a = px.last().copied().unwrap_or(1.0).clamp(0.0, 1.0);
        for v in px.iter().take(device) {
            data.push(v * a + paper * (1.0 - a));
        }
    }
    Ok((device, data))
}

fn to8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// The page size in points: pixels ÷ resolution × 72.
fn page_size(doc: &Document) -> (f64, f64) {
    let dpi = f64::from(doc.resolution_dpi.max(1.0));
    (f64::from(doc.size.width) * 72.0 / dpi, f64::from(doc.size.height) * 72.0 / dpi)
}

const FLATTENED: &str = "PDF was saved flattened: layers, masks-as-editable and transparency are not preserved (#3401)";

/// File › Save As / Save a Copy › PDF (#3401): the flattened document, at its print size
/// (pixels ÷ resolution × 72 pt), as a one-page PDF. Layers and transparency are dropped — the
/// warning says so — and the embedded profile travels with the image.
pub fn export(doc: &Document) -> std::result::Result<ExportResult, IoError> {
    let (device, samples) = page_samples(doc)?;
    let data = samples.into_iter().map(to8).collect();
    let image = PrintImage { width: doc.size.width, height: doc.size.height, channels: device, data, icc: doc.icc_profile.as_ref().map(|p| p.to_vec()) };
    let (pw, ph) = page_size(doc);
    let page = PrintPage { paper: (pw, ph), image, rect: (0.0, 0.0, pw, ph), marks: Marks::default(), description: None, label: None };
    let mut warnings = vec![FLATTENED.into()];
    if !doc.metadata.text.is_empty() {
        warnings.push("free-form text metadata is not supported by PDF; it will be dropped".into());
    }
    Ok(ExportResult { bytes: print_pdf(&page), warnings })
}

/// The PDF Options a save asks for (File › Save As › PDF): a plain PDF, or PDF/X-1a:2001 for
/// print.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PdfOptions {
    /// `Some` writes PDF/X-1a:2001 (ISO 15930-1), as Photoshop's "[PDF/X-1a:2001]" preset.
    pub x1a: Option<PdfX1a>,
}

/// PDF/X-1a:2001: CMYK (or gray) device colour for one output condition, described by an
/// embedded ICC v2 output profile, with the trim and bleed boxes a printer imposes by. Colour
/// that isn't already in that condition is converted to it, as Photoshop's Output › Convert to
/// Destination does; a CMYK document already in it keeps its numbers.
#[derive(Clone, Debug, PartialEq)]
pub struct PdfX1a {
    /// The CMYK output profile (ICC version 2: PDF/X-1a:2001 is PDF 1.3, which predates v4).
    pub profile: std::sync::Arc<Vec<u8>>,
    /// The profile the document's colour is in, when it is converted to [`Self::profile`]. `None`
    /// keeps a CMYK or gray document's numbers; an RGB document is then read as sRGB.
    pub source: Option<std::sync::Arc<Vec<u8>>>,
    pub intent: photocraft_cms::Intent,
    pub bpc: bool,
    /// The bleed already inside the document, in points: the trim box sits this far inside the
    /// page on every side.
    pub bleed_pt: f64,
}

/// The registered characterization a CMYK profile's description names, for the output intent's
/// `OutputConditionIdentifier` (the ICC registry's names); `Custom` otherwise, which the
/// embedded profile then defines.
pub fn output_condition_id(description: &str) -> &'static str {
    let d = description.to_ascii_lowercase();
    let known: [(&[&str], &str); 7] = [
        (&["fogra51", "pso coated v3"], "FOGRA51"),
        (&["fogra52", "pso uncoated v3"], "FOGRA52"),
        (&["fogra39", "iso coated v2"], "FOGRA39"),
        (&["fogra47", "pso uncoated iso12647"], "FOGRA47"),
        (&["fogra29", "iso uncoated"], "FOGRA29"),
        (&["gracol2006", "gracol 2006", "coated gracol"], "CGATS TR 006"),
        (&["swop"], "CGATS TR 001"),
    ];
    known.iter().find(|(keys, _)| keys.iter().any(|k| d.contains(k))).map_or("Custom", |(_, id)| id)
}

/// `D:YYYYMMDDHHmmSS+00'00'` for `secs` after the Unix epoch (UTC).
fn pdf_date(secs: u64) -> String {
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // Howard Hinnant's civil_from_days.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("D:{year:04}{month:02}{day:02}{:02}{:02}{:02}+00'00'", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// 32 hex digits from `data` and `salt` (two FNV-1a passes), for the trailer's file `/ID`.
fn file_id(data: &[u8], salt: u64) -> String {
    let fnv = |seed: u64| data.iter().fold(seed ^ salt, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3));
    format!("{:016x}{:016x}", fnv(0xcbf2_9ce4_8422_2325), fnv(0x8422_2325_cbf2_9ce4))
}

/// File › Save As › PDF with [`PdfOptions`].
pub fn export_with_options(doc: &Document, opts: &PdfOptions) -> std::result::Result<ExportResult, IoError> {
    let Some(x) = &opts.x1a else { return export(doc) };
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    export_x1a(doc, x, secs)
}

fn export_x1a(doc: &Document, x: &PdfX1a, secs: u64) -> std::result::Result<ExportResult, IoError> {
    use photocraft_cms::{Builtin, ColorSpace, Profile, Transform};
    let bad = |m: String| IoError::Unsupported(m);
    let dst = Profile::parse(&x.profile).map_err(|e| bad(format!("PDF/X-1a: the output profile can't be read ({e})")))?;
    if dst.color_space != ColorSpace::Cmyk {
        return Err(bad(format!("PDF/X-1a needs a CMYK output profile; \"{}\" is {:?}", dst.description, dst.color_space)));
    }
    if dst.version.0 != 2 {
        return Err(bad(format!(
            "PDF/X-1a:2001 needs an ICC version 2 output profile and \"{}\" is version {}; choose the printer's profile (for example Coated FOGRA39 or ISO Coated v2)",
            dst.description, dst.version.0
        )));
    }
    let (pw, ph) = page_size(doc);
    let b = x.bleed_pt.max(0.0);
    if !b.is_finite() || 2.0 * b >= pw.min(ph) {
        return Err(bad(format!("PDF/X-1a: a {b:.2} pt bleed leaves no trim on a {pw:.2} × {ph:.2} pt page")));
    }
    let (device, samples) = page_samples(doc)?;
    let mut warnings = vec![FLATTENED.to_string()];
    let space = |m: ColorMode| match m {
        ColorMode::Grayscale => ColorSpace::Gray,
        ColorMode::Cmyk => ColorSpace::Cmyk,
        _ => ColorSpace::Rgb,
    };
    let (channels, samples) = match (doc.mode, &x.source) {
        (ColorMode::Cmyk | ColorMode::Grayscale, None) => (device, samples),
        (mode, source) => {
            let src = match source {
                Some(bytes) => Profile::parse(bytes).map_err(|e| bad(format!("PDF/X-1a: the document's profile can't be read ({e})")))?,
                None if mode == ColorMode::Grayscale => Builtin::SGray.profile().clone(),
                None => Builtin::Srgb.profile().clone(),
            };
            if src.color_space != space(mode) {
                return Err(bad(format!("PDF/X-1a: \"{}\" is a {:?} profile, not the document's {mode:?}", src.description, src.color_space)));
            }
            let t = Transform::new(&src, &dst, x.intent, x.bpc).map_err(|e| bad(format!("PDF/X-1a: can't convert to \"{}\" ({e})", dst.description)))?;
            let mut out = vec![0.0f32; samples.len() / device * 4];
            t.convert_f32(&samples, device, &mut out, 4, false);
            // Paper stays paper: the transform lands white a fraction of a percent off zero
            // ink, which would print a tint over every blank area.
            if mode != ColorMode::Cmyk {
                for (src, ink) in samples.chunks_exact(device).zip(out.as_chunks_mut::<4>().0) {
                    if src.iter().all(|&v| v >= 1.0) {
                        ink.fill(0.0);
                    }
                }
            }
            warnings.push(format!("PDF/X-1a: colour converted from \"{}\" to \"{}\"", src.description, dst.description));
            (4, out)
        }
    };
    if !doc.metadata.text.is_empty() {
        warnings.push("free-form text metadata is not supported by PDF; it will be dropped".into());
    }
    let data: Vec<u8> = samples.into_iter().map(to8).collect();
    let id = file_id(&data, secs);
    let image = PrintImage { width: doc.size.width, height: doc.size.height, channels, data, icc: None };
    let page = PrintPage { paper: (pw, ph), image, rect: (0.0, 0.0, pw, ph), marks: Marks::default(), description: None, label: None };
    let title = doc.name.rsplit_once('.').map_or(doc.name.as_str(), |(stem, _)| stem);
    let xp = XPage {
        profile: &x.profile,
        condition_id: output_condition_id(&dst.description),
        condition: &dst.description,
        title: if title.is_empty() { "Untitled" } else { title },
        trim: (b, b, pw - b, ph - b),
        date: pdf_date(secs),
        id,
    };
    Ok(ExportResult { bytes: write_pdf(&page, Some(&xp)), warnings })
}

#[cfg(test)]
mod tests;
