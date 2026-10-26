//! The PDF writer (ISO 32000-1 basics): raster pages, one or many per file.
//!
//! A small, self-contained writer. [`PdfWriter`] takes pages one at a time, compressing each
//! image as it arrives so a long document never holds more than one page's samples; [`export`]
//! (File › Save a Copy / Save As › PDF, #3401) and [`print_pdf`] are its one-page uses, and the
//! engine's File › Export › Artboards to PDF writes one page per board through it. A page holds
//! a Flate- or JPEG-compressed image XObject, optional printer marks as vector operators and
//! optional Helvetica text; an embedded ICC profile rides in an ICCBased colour space.
//! Layer-preserving PDF, vector/text output and PDF import stay open (FILE-215-10).

use std::io::Write as _;

use photocraft_color::{ColorMode, PixelFormat};
use photocraft_doc::Document;

use crate::{ExportResult, IoError};

/// How [`PrintImage::data`] is stored.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ImageEncoding {
    /// Raw interleaved samples; the writer Flate-compresses them (lossless, Photoshop's ZIP).
    #[default]
    Flate,
    /// An already encoded baseline JPEG, embedded as-is (DCTDecode).
    Jpeg,
}

/// The image placed on a page: 8-bit interleaved samples, 1 (gray), 3 (RGB) or 4 (CMYK,
/// 1 = full ink), with the profile it was authored in if any.
pub struct PrintImage {
    pub width: u32,
    pub height: u32,
    pub channels: usize,
    pub data: Vec<u8>,
    pub icc: Option<Vec<u8>>,
    pub encoding: ImageEncoding,
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
    // Fast: print PDFs are transient and large; level 1 is ~4× quicker than the default.
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
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

/// Upper bound on pages in one file: past it a request is a runaway loop, not a document.
pub const MAX_PAGES: usize = 10_000;

/// Longest Advance Every, in seconds (an hour per page).
pub const MAX_ADVANCE: f64 = 3600.0;

/// How one page replaces the previous one in full screen (ISO 32000-1 §12.4.4.1): PDF
/// Presentation's Transition list. A viewer that doesn't do transitions just shows the page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Transition {
    #[default]
    None,
    BlindsHorizontal,
    BlindsVertical,
    BoxIn,
    BoxOut,
    Dissolve,
    GlitterDown,
    GlitterRight,
    GlitterRightDown,
    /// A different one of the others on each page (picked from the page number, so the same
    /// list always makes the same file).
    Random,
    SplitHorizontalIn,
    SplitHorizontalOut,
    SplitVerticalIn,
    SplitVerticalOut,
    WipeDown,
    WipeLeft,
    WipeRight,
    WipeUp,
}

impl Transition {
    pub const ALL: [Self; 18] = [
        Self::None,
        Self::BlindsHorizontal,
        Self::BlindsVertical,
        Self::BoxIn,
        Self::BoxOut,
        Self::Dissolve,
        Self::GlitterDown,
        Self::GlitterRight,
        Self::GlitterRightDown,
        Self::Random,
        Self::SplitHorizontalIn,
        Self::SplitHorizontalOut,
        Self::SplitVerticalIn,
        Self::SplitVerticalOut,
        Self::WipeDown,
        Self::WipeLeft,
        Self::WipeRight,
        Self::WipeUp,
    ];

    /// The id commands use.
    pub fn id(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::BlindsHorizontal => "blindsHorizontal",
            Self::BlindsVertical => "blindsVertical",
            Self::BoxIn => "boxIn",
            Self::BoxOut => "boxOut",
            Self::Dissolve => "dissolve",
            Self::GlitterDown => "glitterDown",
            Self::GlitterRight => "glitterRight",
            Self::GlitterRightDown => "glitterRightDown",
            Self::Random => "random",
            Self::SplitHorizontalIn => "splitHorizontalIn",
            Self::SplitHorizontalOut => "splitHorizontalOut",
            Self::SplitVerticalIn => "splitVerticalIn",
            Self::SplitVerticalOut => "splitVerticalOut",
            Self::WipeDown => "wipeDown",
            Self::WipeLeft => "wipeLeft",
            Self::WipeRight => "wipeRight",
            Self::WipeUp => "wipeUp",
        }
    }

    /// The English label, as the dialog lists it.
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::BlindsHorizontal => "Blinds Horizontal",
            Self::BlindsVertical => "Blinds Vertical",
            Self::BoxIn => "Box In",
            Self::BoxOut => "Box Out",
            Self::Dissolve => "Dissolve",
            Self::GlitterDown => "Glitter Down",
            Self::GlitterRight => "Glitter Right",
            Self::GlitterRightDown => "Glitter Right-Down",
            Self::Random => "Random",
            Self::SplitHorizontalIn => "Split Horizontal In",
            Self::SplitHorizontalOut => "Split Horizontal Out",
            Self::SplitVerticalIn => "Split Vertical In",
            Self::SplitVerticalOut => "Split Vertical Out",
            Self::WipeDown => "Wipe Down",
            Self::WipeLeft => "Wipe Left",
            Self::WipeRight => "Wipe Right",
            Self::WipeUp => "Wipe Up",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.id() == id)
    }

    /// The `/Trans` dictionary for page `page` (0-based), or none for an instant cut.
    fn dict(self, page: usize) -> Option<String> {
        // Style and its direction (/Dm), motion (/M) or angle (/Di), over one second.
        let body = match self {
            Self::None => return None,
            Self::Random => {
                let concrete: Vec<Self> = Self::ALL.into_iter().filter(|t| !matches!(t, Self::None | Self::Random)).collect();
                let pick = page.wrapping_mul(7).wrapping_add(3) % concrete.len().max(1);
                return concrete.get(pick).and_then(|t| t.dict(page));
            }
            Self::BlindsHorizontal => "/S /Blinds /Dm /H",
            Self::BlindsVertical => "/S /Blinds /Dm /V",
            Self::BoxIn => "/S /Box /M /I",
            Self::BoxOut => "/S /Box /M /O",
            Self::Dissolve => "/S /Dissolve",
            Self::GlitterDown => "/S /Glitter /Di 270",
            Self::GlitterRight => "/S /Glitter /Di 0",
            Self::GlitterRightDown => "/S /Glitter /Di 315",
            Self::SplitHorizontalIn => "/S /Split /Dm /H /M /I",
            Self::SplitHorizontalOut => "/S /Split /Dm /H /M /O",
            Self::SplitVerticalIn => "/S /Split /Dm /V /M /I",
            Self::SplitVerticalOut => "/S /Split /Dm /V /M /O",
            Self::WipeDown => "/S /Wipe /Di 270",
            Self::WipeLeft => "/S /Wipe /Di 180",
            Self::WipeRight => "/S /Wipe /Di 0",
            Self::WipeUp => "/S /Wipe /Di 90",
        };
        Some(format!("<< /Type /Trans {body} /D 1 >>"))
    }
}

/// PDF Presentation's Save As Presentation: the file opens full screen, each page shown for
/// `advance` seconds if set (otherwise until the viewer is told to move on) and entered with
/// `transition`. `loop_after_last` goes back to the first page after the last; PDF has no key
/// for it, so it is a one-line document script (`app.fs.loop`) that only Acrobat and Adobe
/// Reader run — other viewers stop on the last page.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Presentation {
    pub advance: Option<f64>,
    pub loop_after_last: bool,
    pub transition: Transition,
}

/// A PDF built page by page. Objects 1–3 are the catalog and the page tree (both written by
/// [`PdfWriter::finish`], once every page is known) and the shared Helvetica; each page then
/// adds its page, contents, image and optional ICC profile objects.
pub struct PdfWriter {
    out: Vec<u8>,
    /// Byte offset of object `i + 1`.
    offsets: Vec<usize>,
    kids: Vec<usize>,
    presentation: Option<Presentation>,
}

impl Default for PdfWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl PdfWriter {
    pub fn new() -> Self {
        let mut w = Self { out: b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec(), offsets: vec![0, 0], kids: Vec::new(), presentation: None };
        // 1 and 2 (catalog, page tree) are patched in by `finish`.
        w.obj(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>");
        w
    }

    /// A writer whose file is a full-screen presentation. An `advance` that isn't a positive
    /// number of seconds up to [`MAX_ADVANCE`] is refused.
    pub fn presentation(p: Presentation) -> std::result::Result<Self, IoError> {
        if let Some(s) = p.advance
            && !(s.is_finite() && s > 0.0 && s <= MAX_ADVANCE)
        {
            return Err(IoError::Unsupported(format!("Advance Every takes more than 0 and at most {MAX_ADVANCE} seconds, not {s}")));
        }
        let mut w = Self::new();
        w.presentation = Some(p);
        Ok(w)
    }

    /// Pages added so far.
    pub fn pages(&self) -> usize {
        self.kids.len()
    }

    /// Write `body` as the next object; returns its number.
    fn obj(&mut self, body: &[u8]) -> usize {
        self.offsets.push(self.out.len());
        let n = self.offsets.len();
        self.write_obj(n, body);
        n
    }

    /// Write object `n`, whose number was reserved earlier, here.
    fn write_at(&mut self, n: usize, body: &[u8]) {
        if let Some(slot) = n.checked_sub(1).and_then(|i| self.offsets.get_mut(i)) {
            *slot = self.out.len();
        }
        self.write_obj(n, body);
    }

    fn write_obj(&mut self, n: usize, body: &[u8]) {
        self.out.extend_from_slice(format!("{n} 0 obj\n").as_bytes());
        self.out.extend_from_slice(body);
        self.out.extend_from_slice(b"\nendobj\n");
    }

    fn stream(&mut self, dict: &str, data: &[u8]) -> usize {
        let mut body = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
        body.extend_from_slice(data);
        body.extend_from_slice(b"\nendstream");
        self.obj(&body)
    }

    /// Append `page`. Its samples are compressed here, so the caller can drop them right after.
    pub fn add_page(&mut self, page: &PrintPage) -> std::result::Result<(), IoError> {
        let img = &page.image;
        if self.kids.len() >= MAX_PAGES {
            return Err(IoError::Unsupported(format!("a PDF holds at most {MAX_PAGES} pages")));
        }
        if img.width == 0 || img.height == 0 || !matches!(img.channels, 1 | 3 | 4) {
            return Err(IoError::Unsupported("a PDF page needs a non-empty gray, RGB or CMYK image".into()));
        }
        let samples = (img.width as usize).saturating_mul(img.height as usize).saturating_mul(img.channels);
        if img.encoding == ImageEncoding::Flate && img.data.len() != samples {
            return Err(IoError::Unsupported(format!("page image has {} bytes, expected {samples}", img.data.len())));
        }
        let (pw, ph) = page.paper;
        let (x, y, w, h) = page.rect;
        // Numbers are known before writing: page, contents, image, then the profile if any.
        let n = self.offsets.len() + 1;
        let (contents, image, icc) = (n + 1, n + 2, n + 3);
        let mut show = String::new();
        if let Some(p) = self.presentation {
            if let Some(s) = p.advance {
                show.push_str(&format!(" /Dur {s}"));
            }
            if let Some(t) = p.transition.dict(self.kids.len()) {
                show.push_str(&format!(" /Trans {t}"));
            }
        }
        self.obj(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {pw:.2} {ph:.2}] /Resources << /XObject << /Im0 {image} 0 R >> /Font << /F1 3 0 R >> >> /Contents {contents} 0 R{show} >>"
            )
            .as_bytes(),
        );
        self.kids.push(n);
        // An explicit white page, so every viewer (and rasteriser) shows paper, not transparency.
        let mut content = format!("q 1 g 0 0 {pw:.2} {ph:.2} re f Q\nq {w:.3} 0 0 {h:.3} {x:.3} {y:.3} cm /Im0 Do Q\n");
        content.push_str(&marks_ops(page.marks, page.rect));
        let any_marks = page.marks.corner_crop || page.marks.center_crop || page.marks.registration;
        let pad = if any_marks { 30.0 } else { 6.0 };
        if let Some(l) = page.label.as_deref().filter(|l| !l.is_empty()) {
            content.push_str(&format!("BT /F1 8 Tf 0 g {:.2} {:.2} Td {} Tj ET\n", x, y + h + pad, pdf_string(l)));
        }
        if let Some(d) = page.description.as_deref().filter(|d| !d.is_empty()) {
            content.push_str(&format!("BT /F1 8 Tf 0 g {:.2} {:.2} Td {} Tj ET\n", x, y - pad - 8.0, pdf_string(d)));
        }
        self.stream("/Filter /FlateDecode", &deflate(content.as_bytes()));
        let device = match img.channels {
            1 => "/DeviceGray",
            4 => "/DeviceCMYK",
            _ => "/DeviceRGB",
        };
        let cs = if img.icc.is_some() { format!("[/ICCBased {icc} 0 R]") } else { device.to_string() };
        let dict = format!("/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace {cs} /BitsPerComponent 8", img.width, img.height);
        match img.encoding {
            ImageEncoding::Flate => self.stream(&format!("{dict} /Filter /FlateDecode"), &deflate(&img.data)),
            ImageEncoding::Jpeg => self.stream(&format!("{dict} /Filter /DCTDecode"), &img.data),
        };
        if let Some(profile) = &img.icc {
            self.stream(&format!("/N {} /Alternate {device} /Filter /FlateDecode", img.channels), &deflate(profile));
        }
        Ok(())
    }

    /// The finished file: the page tree, the cross-reference table and the trailer.
    pub fn finish(mut self) -> Vec<u8> {
        let kids: Vec<String> = self.kids.iter().map(|k| format!("{k} 0 R")).collect();
        let tree = format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), self.kids.len());
        self.write_at(2, tree.as_bytes());
        let mut catalog = String::from("<< /Type /Catalog /Pages 2 0 R");
        if let Some(p) = self.presentation {
            catalog.push_str(" /PageMode /FullScreen");
            if p.loop_after_last {
                let script = self.obj(b"<< /S /JavaScript /JS (app.fs.loop = true;) >>");
                let names = self.obj(format!("<< /Names [(PhotoCraft Presentation) {script} 0 R] >>").as_bytes());
                catalog.push_str(&format!(" /Names << /JavaScript {names} 0 R >>"));
            }
        }
        catalog.push_str(" >>");
        self.write_at(1, catalog.as_bytes());
        let mut out = self.out;
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", self.offsets.len() + 1).as_bytes());
        for o in &self.offsets {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", self.offsets.len() + 1).as_bytes());
        out
    }
}

/// A one-page PDF.
pub fn print_pdf(page: &PrintPage) -> std::result::Result<Vec<u8>, IoError> {
    let mut w = PdfWriter::new();
    w.add_page(page)?;
    Ok(w.finish())
}

/// What [`flat_page`] does with a mode PDF has no device space for (Lab, Indexed, Duotone,
/// Multichannel…).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OtherModes {
    /// Refuse, naming Image › Mode (File › Save As, where the user chose the format).
    Refuse,
    /// Composite to RGB and drop the profile (pages of a larger export: artboards, presentations).
    AsRgb,
}

/// `doc` flattened over white paper as one page at its print size (pixels ÷ resolution × 72 pt).
/// RGB, Grayscale and CMYK keep their channels and embedded profile.
pub fn flat_page(doc: &Document, others: OtherModes) -> std::result::Result<PrintPage, IoError> {
    flat_page_over(doc, others, 1.0)
}

/// [`flat_page`] with transparency showing `paper`, a gray level (1 = white, 0 = black): the
/// same light in RGB and Gray, and black ink alone in CMYK (PDF Presentation's Background).
pub fn flat_page_over(doc: &Document, others: OtherModes, paper: f32) -> std::result::Result<PrintPage, IoError> {
    let (w, h) = (usize::try_from(doc.size.width).unwrap_or(0), usize::try_from(doc.size.height).unwrap_or(0));
    if w == 0 || h == 0 {
        return Err(IoError::Unsupported("PDF needs a document with non-zero width and height; nothing was written".into()));
    }
    let profile = || doc.icc_profile.as_ref().map(|p| p.to_vec());
    let (mode, device, icc) = match doc.mode {
        ColorMode::Rgb => (ColorMode::Rgb, 3, profile()),
        ColorMode::Grayscale => (ColorMode::Grayscale, 1, profile()),
        ColorMode::Cmyk => (ColorMode::Cmyk, 4, profile()),
        _ if others == OtherModes::AsRgb => (ColorMode::Rgb, 3, None),
        m => return Err(IoError::Unsupported(format!("PDF saves RGB, Grayscale and CMYK documents; {m:?} needs converting to RGB first (Image > Mode)"))),
    };
    let fmt = PixelFormat::new(mode, doc.pixel_format().sample, true);
    let n = fmt.channels().max(1);
    let canvas = photocraft_geom::Rect::new(0, 0, w as i32, h as i32);
    let surface = photocraft_compose::flatten_to_surface(doc, fmt, None);
    let values = surface.read_region(canvas);
    let to8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    // Straight alpha, composited here over paper: its light in RGB/Gray (1 = full light), and in
    // CMYK no C, M or Y with the matching K (0 = no ink).
    let paper = if paper.is_finite() { paper.clamp(0.0, 1.0) } else { 1.0 };
    let under: Vec<f32> = if mode == ColorMode::Cmyk { vec![0.0, 0.0, 0.0, 1.0 - paper] } else { vec![paper; device] };
    let mut data = Vec::with_capacity((w * h).saturating_mul(device));
    for px in values.chunks(n) {
        let a = px.last().copied().unwrap_or(1.0).clamp(0.0, 1.0);
        for (v, u) in px.iter().zip(&under) {
            data.push(to8(v * a + u * (1.0 - a)));
        }
    }
    let image = PrintImage { width: doc.size.width, height: doc.size.height, channels: device, data, icc, encoding: ImageEncoding::Flate };
    let (pw, ph) = print_size(doc);
    Ok(PrintPage { paper: (pw, ph), image, rect: (0.0, 0.0, pw, ph), marks: Marks::default(), description: None, label: None })
}

/// `doc`'s size in points at its resolution.
pub fn print_size(doc: &Document) -> (f64, f64) {
    let dpi = f64::from(doc.resolution_dpi.max(1.0));
    (f64::from(doc.size.width) * 72.0 / dpi, f64::from(doc.size.height) * 72.0 / dpi)
}

/// File › Save As / Save a Copy › PDF (#3401): the flattened document, at its print size
/// (pixels ÷ resolution × 72 pt), as a one-page PDF. Layers and transparency are dropped — the
/// warning says so — and the embedded profile travels with the image.
pub fn export(doc: &Document) -> std::result::Result<ExportResult, IoError> {
    let page = flat_page(doc, OtherModes::Refuse)?;
    let mut warnings = vec!["PDF was saved flattened: layers, masks-as-editable and transparency are not preserved (#3401)".into()];
    if !doc.metadata.text.is_empty() {
        warnings.push("free-form text metadata is not supported by PDF; it will be dropped".into());
    }
    Ok(ExportResult { bytes: print_pdf(&page)?, warnings })
}

#[cfg(test)]
mod tests;
