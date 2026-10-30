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

/// The image placed on a print page.
pub struct PrintImage {
    pub width: u32,
    pub height: u32,
    /// 1 (gray), 3 (RGB) or 4 (CMYK, 1 = full ink), 8-bit interleaved.
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
    /// Photoshop's Functions › Bleed, in points: corner crop marks sit this far inside the image
    /// edge, so the trim cuts through the image and leaves no paper-white sliver. 0 = on the edge.
    pub bleed: f64,
}

/// Gap between the image edge and a crop mark, and the mark's length (pt).
pub const MARK_GAP: f64 = 6.0;
pub const MARK_LEN: f64 = 18.0;

/// Corner crop marks around `(x, y, w, h)` as eight `(from, to)` segments, a horizontal then a
/// vertical one per corner. Each starts `MARK_GAP` outside the image (so it never prints on it)
/// and lies on the trim line, `bleed` inside the edge. The inset is symmetric, so this holds for
/// both bottom-up (PDF) and top-down (Windows) coordinates.
pub fn corner_crop_segments(bleed: f64, (x, y, w, h): (f64, f64, f64, f64)) -> [((f64, f64), (f64, f64)); 8] {
    let mut out = [((0.0, 0.0), (0.0, 0.0)); 8];
    for (i, (cx, cy, dx, dy)) in [(x, y, -1.0, -1.0), (x + w, y, 1.0, -1.0), (x, y + h, -1.0, 1.0), (x + w, y + h, 1.0, 1.0)].into_iter().enumerate() {
        let (tx, ty) = (cx - dx * bleed, cy - dy * bleed);
        out[2 * i] = ((cx + dx * MARK_GAP, ty), (cx + dx * (MARK_GAP + MARK_LEN), ty));
        out[2 * i + 1] = ((tx, cy + dy * MARK_GAP), (tx, cy + dy * (MARK_GAP + MARK_LEN)));
    }
    out
}

/// One page: paper (pt), image box (pt, origin bottom-left), marks and texts.
pub struct PrintPage {
    pub paper: (f64, f64),
    pub image: PrintImage,
    pub rect: (f64, f64, f64, f64),
    pub marks: Marks,
    pub description: Option<String>,
    pub label: Option<String>,
    /// Photoshop's Functions › Emulsion Down: the whole page mirrored left to right, for transfer
    /// printing (sublimation, iron-on, DTF, film read from the back).
    pub emulsion_down: bool,
}

// ISO 32000-1, Annex D.2: the standard Helvetica font below uses WinAnsiEncoding,
// not UTF-8. Escape non-ASCII bytes as three octal digits so the content stays ASCII
// and a following digit cannot become part of the escape. Other scripts still need
// an embedded font; retain the existing '?' fallback for unsupported characters.
/// A PDF literal string for `s` in WinAnsi encoding.
fn pdf_string(s: &str) -> String {
    let mut o = String::from("(");
    for c in s.chars() {
        let byte = match c {
            '\u{20}'..='\u{7e}' | '\u{a0}'..='\u{ff}' => c as u8,
            '€' => 0x80,
            '‚' => 0x82,
            'ƒ' => 0x83,
            '„' => 0x84,
            '…' => 0x85,
            '†' => 0x86,
            '‡' => 0x87,
            'ˆ' => 0x88,
            '‰' => 0x89,
            'Š' => 0x8a,
            '‹' => 0x8b,
            'Œ' => 0x8c,
            'Ž' => 0x8e,
            '‘' => 0x91,
            '’' => 0x92,
            '“' => 0x93,
            '”' => 0x94,
            '•' => 0x95,
            '–' => 0x96,
            '—' => 0x97,
            '˜' => 0x98,
            '™' => 0x99,
            'š' => 0x9a,
            '›' => 0x9b,
            'œ' => 0x9c,
            'ž' => 0x9e,
            'Ÿ' => 0x9f,
            _ => b'?',
        };
        match byte {
            b'(' | b')' | b'\\' => {
                o.push('\\');
                o.push(char::from(byte));
            }
            0x80..=0xff => o.push_str(&format!("\\{byte:03o}")),
            _ => o.push(char::from(byte)),
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
    let (gap, len) = (MARK_GAP, MARK_LEN);
    let line = |s: &mut String, x0: f64, y0: f64, x1: f64, y1: f64| s.push_str(&format!("{x0:.2} {y0:.2} m {x1:.2} {y1:.2} l S\n"));
    if m.corner_crop {
        for ((x0, y0), (x1, y1)) in corner_crop_segments(m.bleed, (x, y, w, h)) {
            line(&mut s, x0, y0, x1, y1);
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

/// The page content: paper, image, marks and texts, mirrored for Emulsion Down.
fn content_stream(page: &PrintPage) -> String {
    let (pw, ph) = page.paper;
    let (x, y, w, h) = page.rect;
    // An explicit white page, so every viewer (and rasteriser) shows paper, not transparency.
    let mut content = format!("q 1 g 0 0 {pw:.2} {ph:.2} re f Q\n");
    if page.emulsion_down {
        content.push_str(&format!("q -1 0 0 1 {pw:.2} 0 cm\n"));
    }
    content.push_str(&format!("q {w:.3} 0 0 {h:.3} {x:.3} {y:.3} cm /Im0 Do Q\n"));
    content.push_str(&marks_ops(page.marks, page.rect));
    let any_marks = page.marks.corner_crop || page.marks.center_crop || page.marks.registration;
    let pad = if any_marks { 30.0 } else { 6.0 };
    if let Some(l) = page.label.as_deref().filter(|l| !l.is_empty()) {
        content.push_str(&format!("BT /F1 8 Tf 0 g {:.2} {:.2} Td {} Tj ET\n", x, y + h + pad, pdf_string(l)));
    }
    if let Some(d) = page.description.as_deref().filter(|d| !d.is_empty()) {
        content.push_str(&format!("BT /F1 8 Tf 0 g {:.2} {:.2} Td {} Tj ET\n", x, y - pad - 8.0, pdf_string(d)));
    }
    if page.emulsion_down {
        content.push_str("Q\n");
    }
    content
}

/// A one-page PDF.
pub fn print_pdf(page: &PrintPage) -> Vec<u8> {
    let mut out: Vec<u8> = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets: Vec<usize> = Vec::new();
    let mut obj = |out: &mut Vec<u8>, body: &[u8]| {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", offsets.len()).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };
    let img = &page.image;
    let (pw, ph) = page.paper;
    // 1 catalog, 2 pages, 3 page, 4 contents, 5 image, 6 font, 7 ICC (optional).
    obj(&mut out, b"<< /Type /Catalog /Pages 2 0 R >>");
    obj(&mut out, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    obj(
        &mut out,
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {pw:.2} {ph:.2}] /Resources << /XObject << /Im0 5 0 R >> /Font << /F1 6 0 R >> >> /Contents 4 0 R >>"
        )
        .as_bytes(),
    );
    let c = deflate(content_stream(page).as_bytes());
    let mut body = format!("<< /Length {} /Filter /FlateDecode >>\nstream\n", c.len()).into_bytes();
    body.extend_from_slice(&c);
    body.extend_from_slice(b"\nendstream");
    obj(&mut out, &body);
    let device = match img.channels {
        1 => "/DeviceGray",
        4 => "/DeviceCMYK",
        _ => "/DeviceRGB",
    };
    let cs = if img.icc.is_some() { "[/ICCBased 7 0 R]".to_string() } else { device.to_string() };
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
    obj(&mut out, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>");
    if let Some(icc) = &img.icc {
        let z = deflate(icc);
        let mut body = format!("<< /N {} /Alternate {device} /Filter /FlateDecode /Length {} >>\nstream\n", img.channels, z.len()).into_bytes();
        body.extend_from_slice(&z);
        body.extend_from_slice(b"\nendstream");
        obj(&mut out, &body);
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes());
    for o in &offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", offsets.len() + 1).as_bytes());
    out
}

/// File › Save As / Save a Copy › PDF (#3401): the flattened document, at its print size
/// (pixels ÷ resolution × 72 pt), as a one-page PDF. Layers and transparency are dropped — the
/// warning says so — and the embedded profile travels with the image.
pub fn export(doc: &Document) -> std::result::Result<ExportResult, IoError> {
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
    let to8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    // Straight alpha, composited here over paper: white in RGB/Gray (1 = full light), zero ink
    // in CMYK (0 = paper).
    let paper = if doc.mode == ColorMode::Cmyk { 0.0 } else { 1.0 };
    let mut data = Vec::with_capacity((w * h).saturating_mul(device));
    for px in values.chunks(n) {
        let a = px.last().copied().unwrap_or(1.0).clamp(0.0, 1.0);
        for v in px.iter().take(device) {
            data.push(to8(v * a + paper * (1.0 - a)));
        }
    }
    let image = PrintImage { width: doc.size.width, height: doc.size.height, channels: device, data, icc: doc.icc_profile.as_ref().map(|p| p.to_vec()) };
    let dpi = f64::from(doc.resolution_dpi.max(1.0));
    let (pw, ph) = (f64::from(doc.size.width) * 72.0 / dpi, f64::from(doc.size.height) * 72.0 / dpi);
    let page = PrintPage { paper: (pw, ph), image, rect: (0.0, 0.0, pw, ph), marks: Marks::default(), description: None, label: None, emulsion_down: false };
    let mut warnings = vec!["PDF was saved flattened: layers, masks-as-editable and transparency are not preserved (#3401)".into()];
    if !doc.metadata.text.is_empty() {
        warnings.push("free-form text metadata is not supported by PDF; it will be dropped".into());
    }
    Ok(ExportResult { bytes: print_pdf(&page), warnings })
}

#[cfg(test)]
mod tests;
