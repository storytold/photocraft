//! The PDF writer (ISO 32000-1 basics): raster pages, one or many per file.
//!
//! A small, self-contained writer. [`PdfWriter`] takes pages one at a time, compressing each
//! image as it arrives and writing it into its sink (a `Vec`, or the temporary file of an atomic
//! save), so a long document never holds more than one page's samples. [`export`] (File › Save a
//! Copy / Save As › PDF, #3401) writes the document, or one page per artboard, through it; the
//! engine's File › Export › Artboards to PDF and File › Automate › PDF Presentation write one
//! page per board or file. A page holds a Flate- or JPEG-compressed image XObject, optional
//! printer marks as vector operators and optional Helvetica text; an embedded ICC profile rides
//! in an ICCBased colour space. [`export`] also draws simple top shape and text layers as vectors
//! over the image (with a selection font for their text). Layer-preserving PDF and PDF import
//! stay open (FILE-215-10).
//!
//! Spot channels leave as plates (FILE-215-17): one 8-bit image XObject per channel in a
//! `/Separation` colour space named after the channel, painted over the composite with overprint
//! on, so a RIP or a separating rasteriser (Ghostscript `tiffsep`, `psdcmyk`) keeps each one as
//! its own named plate instead of mixing it into CMYK. The plates carry no mask: a viewer that
//! ignores overprint shows the last plate opaque, Acrobat's Overprint Preview and any separating
//! RIP show the real result, and an explicit image mask makes Illustrator report "An unknown
//! imaging construct was encountered". An artboard page carries its own region of every plate.

use std::io::Write;

use photocraft_color::{ColorMode, PixelFormat};
use photocraft_doc::Document;

use crate::{ExportResult, IoError};

mod selection;
mod shapes;
mod text;

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

/// One spot plate: a named `/Separation` colorant with 8-bit ink density over the whole image
/// box (`width × height` samples, 255 = solid ink), and the CMYK a viewer mixes to show it.
pub struct SpotPlate {
    /// The colorant name, as the channel is named. A RIP names the plate after it.
    pub name: String,
    /// The alternate colour (`/DeviceCMYK`, 1 = full ink) of the tint transform: how a viewer
    /// previews the ink. A RIP prints the plate, not this.
    pub cmyk: [f32; 4],
    /// The ink colour as CIE L*a*b* (D50) and as encoded sRGB, and the solidity (0..1), for the
    /// XMP description a converter reads to rebuild the spot channel (see [`plates_xmp`]).
    pub lab: [f32; 3],
    pub srgb: [f32; 3],
    pub solidity: f32,
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// Printer marks around the image.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Marks {
    pub corner_crop: bool,
    pub center_crop: bool,
    pub registration: bool,
}

/// One page: paper (pt), image box (pt, origin bottom-left), marks, texts and spot plates.
///
/// The image is optional: a Multichannel document is nothing but its plates. The plates are
/// painted over the image box in order, each with overprint on, so painting one never knocks
/// the others (or the image) out.
pub struct PrintPage {
    pub paper: (f64, f64),
    pub image: Option<PrintImage>,
    pub rect: (f64, f64, f64, f64),
    pub marks: Marks,
    pub description: Option<String>,
    pub label: Option<String>,
    /// The page's name in the PDF `/PageLabels` tree (Unicode-capable; not drawn on the page).
    pub page_label: Option<String>,
    pub plates: Vec<SpotPlate>,
}

pub(super) fn pdf_string(s: &str) -> String {
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

/// A PDF text string with UTF-16BE hex (the Unicode-capable form, byte-order mark included).
fn pdf_unicode_string(s: &str) -> String {
    let mut o = String::from("<FEFF");
    for u in s.encode_utf16() {
        o.push_str(&format!("{u:04X}"));
    }
    o.push('>');
    o
}

/// `s` as a PDF name object (ISO 32000-1 §7.3.5): `/` then the UTF-8 bytes, every byte outside
/// the regular printable ASCII range, `#` and the delimiters as `#xx`. An empty name would be
/// the bare `/`, which some readers reject, so it becomes `/Spot`.
fn pdf_name(s: &str) -> String {
    let mut o = String::from("/");
    for b in s.bytes() {
        let regular = (0x21..=0x7e).contains(&b) && !matches!(b, b'#' | b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%');
        if regular {
            o.push(char::from(b));
        } else {
            o.push_str(&format!("#{b:02X}"));
        }
    }
    if o.len() == 1 {
        o.push_str("Spot");
    }
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

/// Largest file [`PdfWriter`] writes. The cross-reference table gives every object's offset in
/// exactly ten digits (ISO 32000-1 §7.5.4), so nothing may start at 10,000,000,000 bytes or
/// later; this stops a little before, leaving room for what [`PdfWriter::finish`] appends.
pub const MAX_BYTES: u64 = 9_900_000_000;

/// A PDF built page by page into `W` (a `Vec<u8>`, or the temporary file of an atomic save, so a
/// long document is never held in memory). Objects 1–3 are the catalog and the page tree (both
/// written by [`PdfWriter::finish`], once every page is known) and the shared Helvetica; each
/// page then adds its page, contents and image objects, its ICC profile unless the previous
/// page's is the same, one XObject per spot plate (and, on the first page with plates, the
/// overprint ExtGState they share), and the selection font of its live text if it has any.
/// [`PdfWriter::finish`] adds the XMP metadata stream and the page-label tree when present.
pub struct PdfWriter<W: Write = Vec<u8>> {
    out: W,
    /// Bytes written so far.
    pos: u64,
    /// Largest file this writer writes ([`MAX_BYTES`]).
    limit: u64,
    /// Byte offset of object `i + 1`.
    offsets: Vec<u64>,
    kids: Vec<usize>,
    presentation: Option<Presentation>,
    /// The last ICC profile written and its object, reused by the next page with the same one.
    icc: Option<(Vec<u8>, usize)>,
    /// The overprint ExtGState every plate is painted with, once a page has plates.
    overprint: Option<usize>,
    /// Each page's `/PageLabels` name (empty for none).
    labels: Vec<String>,
    /// The catalog's `/Metadata` packet.
    xmp: Option<String>,
}

impl Default for PdfWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl PdfWriter {
    /// A writer that builds the file in memory.
    pub fn new() -> Self {
        #[allow(clippy::expect_used, clippy::let_and_return)]
        let w = Self::to(Vec::new()).expect("a Vec takes the header and the font object, far below MAX_BYTES");
        w
    }

    /// An in-memory writer whose file is a full-screen presentation (see [`PdfWriter::presentation_to`]).
    pub fn presentation(p: Presentation) -> std::result::Result<Self, IoError> {
        Self::presentation_to(Vec::new(), p)
    }
}

impl<W: Write> PdfWriter<W> {
    /// A writer that streams the file into `out` as pages arrive.
    pub fn to(out: W) -> std::result::Result<Self, IoError> {
        let mut w = Self {
            out,
            pos: 0,
            limit: MAX_BYTES,
            offsets: vec![0, 0],
            kids: Vec::new(),
            presentation: None,
            icc: None,
            overprint: None,
            labels: Vec::new(),
            xmp: None,
        };
        w.put(b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n")?;
        // 1 and 2 (catalog, page tree) are written by `finish`.
        w.obj(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>")?;
        Ok(w)
    }

    /// A streaming writer whose file is a full-screen presentation. An `advance` that isn't a
    /// positive number of seconds up to [`MAX_ADVANCE`] is refused.
    pub fn presentation_to(out: W, p: Presentation) -> std::result::Result<Self, IoError> {
        if let Some(s) = p.advance
            && !(s.is_finite() && s > 0.0 && s <= MAX_ADVANCE)
        {
            return Err(IoError::Unsupported(format!("Advance Every takes more than 0 and at most {MAX_ADVANCE} seconds, not {s}")));
        }
        let mut w = Self::to(out)?;
        w.presentation = Some(p);
        Ok(w)
    }

    /// Pages added so far.
    pub fn pages(&self) -> usize {
        self.kids.len()
    }

    /// Embed `xmp` as the catalog's `/Metadata` stream (unfiltered, as the XMP specification asks
    /// of a packet in a PDF, so a reader can find it by scanning for the packet header).
    pub fn set_metadata(&mut self, xmp: Option<String>) {
        self.xmp = xmp;
    }

    /// Append `bytes`, refusing to grow the file past its limit ([`MAX_BYTES`]).
    fn put(&mut self, bytes: &[u8]) -> std::result::Result<(), IoError> {
        let end = self.pos.saturating_add(bytes.len() as u64);
        if end > self.limit {
            return Err(IoError::Unsupported(format!("the PDF would be larger than {} bytes, the most its cross-reference table can address", self.limit)));
        }
        self.out.write_all(bytes).map_err(|e| IoError::Unsupported(format!("writing the PDF failed: {e}")))?;
        self.pos = end;
        Ok(())
    }

    /// Write `body` as the next object; returns its number.
    fn obj(&mut self, body: &[u8]) -> std::result::Result<usize, IoError> {
        self.offsets.push(self.pos);
        let n = self.offsets.len();
        self.write_obj(n, body)?;
        Ok(n)
    }

    /// Write object `n`, whose number was reserved earlier, here.
    fn write_at(&mut self, n: usize, body: &[u8]) -> std::result::Result<(), IoError> {
        let pos = self.pos;
        if let Some(slot) = n.checked_sub(1).and_then(|i| self.offsets.get_mut(i)) {
            *slot = pos;
        }
        self.write_obj(n, body)
    }

    fn write_obj(&mut self, n: usize, body: &[u8]) -> std::result::Result<(), IoError> {
        self.put(format!("{n} 0 obj\n").as_bytes())?;
        self.put(body)?;
        self.put(b"\nendobj\n")
    }

    fn stream(&mut self, dict: &str, data: &[u8]) -> std::result::Result<usize, IoError> {
        self.offsets.push(self.pos);
        let n = self.offsets.len();
        self.put(format!("{n} 0 obj\n<< {dict} /Length {} >>\nstream\n", data.len()).as_bytes())?;
        self.put(data)?;
        self.put(b"\nendstream\nendobj\n")?;
        Ok(n)
    }

    /// Append `page`. Its samples are compressed and written here, so the caller can drop them
    /// right after.
    pub fn add_page(&mut self, page: &PrintPage) -> std::result::Result<(), IoError> {
        self.add_page_with_vectors(page, "", None)
    }

    /// [`PdfWriter::add_page`] with vector operators drawn over the image (live shapes and text,
    /// see [`collect_vectors`]) and the selection font their text uses.
    fn add_page_with_vectors(
        &mut self,
        page: &PrintPage,
        vectors: &str,
        selection_font: Option<&selection::SelectionFont>,
    ) -> std::result::Result<(), IoError> {
        if self.kids.len() >= MAX_PAGES {
            return Err(IoError::Unsupported(format!("a PDF holds at most {MAX_PAGES} pages")));
        }
        if let Some(img) = &page.image {
            if img.width == 0 || img.height == 0 || !matches!(img.channels, 1 | 3 | 4) {
                return Err(IoError::Unsupported("a PDF page needs a non-empty gray, RGB or CMYK image".into()));
            }
            let samples = (img.width as usize).saturating_mul(img.height as usize).saturating_mul(img.channels);
            if img.encoding == ImageEncoding::Flate && img.data.len() != samples {
                return Err(IoError::Unsupported(format!("page image has {} bytes, expected {samples}", img.data.len())));
            }
        } else if page.plates.is_empty() {
            return Err(IoError::Unsupported("a PDF page needs an image or spot plates".into()));
        }
        for p in &page.plates {
            let samples = (p.width as usize).saturating_mul(p.height as usize);
            if p.width == 0 || p.height == 0 || p.data.len() != samples {
                return Err(IoError::Unsupported(format!("spot plate {:?} has {} bytes for {}×{} samples", p.name, p.data.len(), p.width, p.height)));
            }
        }
        let (pw, ph) = page.paper;
        let (x, y, w, h) = page.rect;
        // Numbers are known before writing, handed out in the order the objects are written: page,
        // contents, image, the profile unless the last one is reused, the plates, the overprint
        // state the first time plates appear, then the selection font's three objects.
        let mut next = self.offsets.len() + 1;
        let mut take = |present: bool| {
            let o = present.then_some(next);
            next += usize::from(present);
            o
        };
        let n = take(true).unwrap_or_default();
        let contents = take(true).unwrap_or_default();
        let image = take(page.image.is_some());
        let profile = page.image.as_ref().and_then(|i| i.icc.as_deref());
        let reused_icc = match (profile, &self.icc) {
            (Some(p), Some((last, o))) if p == last.as_slice() => Some(*o),
            _ => None,
        };
        let new_icc = take(profile.is_some() && reused_icc.is_none());
        let icc = reused_icc.or(new_icc);
        let plates: Vec<usize> = page.plates.iter().filter_map(|_| take(true)).collect();
        let new_gs = take(!page.plates.is_empty() && self.overprint.is_none());
        let gs = self.overprint.or(new_gs);
        let selection_base = next;
        let mut show = String::new();
        if let Some(p) = self.presentation {
            if let Some(s) = p.advance {
                show.push_str(&format!(" /Dur {s}"));
            }
            if let Some(t) = p.transition.dict(self.kids.len()) {
                show.push_str(&format!(" /Trans {t}"));
            }
        }
        let mut xobjects = String::new();
        if let Some(o) = image {
            xobjects.push_str(&format!("/Im0 {o} 0 R "));
        }
        for (k, o) in plates.iter().enumerate() {
            xobjects.push_str(&format!("/Sp{k} {o} 0 R "));
        }
        let ext_gstate = match gs {
            Some(o) if !page.plates.is_empty() => format!("/ExtGState << /GS0 {o} 0 R >> "),
            _ => String::new(),
        };
        let select = if selection_font.is_some() { format!(" /FSelect {selection_base} 0 R") } else { String::new() };
        self.obj(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {pw:.2} {ph:.2}] /Resources << /XObject << {xobjects}>> {ext_gstate}/Font << /F1 3 0 R{select} >> >> /Contents {contents} 0 R{show} >>"
            )
            .as_bytes(),
        )?;
        self.kids.push(n);
        self.labels.push(page.page_label.clone().unwrap_or_default());
        // An explicit white page, so every viewer (and rasteriser) shows paper, not transparency.
        let mut content = format!("q 1 g 0 0 {pw:.2} {ph:.2} re f Q\n");
        if image.is_some() {
            content.push_str(&format!("q {w:.3} 0 0 {h:.3} {x:.3} {y:.3} cm /Im0 Do Q\n"));
        }
        content.push_str(vectors);
        // Each plate overprints what is already on the page: with `/OP true` a Separation paints
        // its own colorant only, leaving the others (and the composite) as they are.
        for k in 0..page.plates.len() {
            content.push_str(&format!("q /GS0 gs {w:.3} 0 0 {h:.3} {x:.3} {y:.3} cm /Sp{k} Do Q\n"));
        }
        content.push_str(&marks_ops(page.marks, page.rect));
        let any_marks = page.marks.corner_crop || page.marks.center_crop || page.marks.registration;
        let pad = if any_marks { 30.0 } else { 6.0 };
        if let Some(l) = page.label.as_deref().filter(|l| !l.is_empty()) {
            content.push_str(&format!("BT /F1 8 Tf 0 g {:.2} {:.2} Td {} Tj ET\n", x, y + h + pad, pdf_string(l)));
        }
        if let Some(d) = page.description.as_deref().filter(|d| !d.is_empty()) {
            content.push_str(&format!("BT /F1 8 Tf 0 g {:.2} {:.2} Td {} Tj ET\n", x, y - pad - 8.0, pdf_string(d)));
        }
        self.stream("/Filter /FlateDecode", &deflate(content.as_bytes()))?;
        if let Some(img) = &page.image {
            let device = match img.channels {
                1 => "/DeviceGray",
                4 => "/DeviceCMYK",
                _ => "/DeviceRGB",
            };
            let cs = match icc {
                Some(o) => format!("[/ICCBased {o} 0 R]"),
                None => device.to_string(),
            };
            let dict = format!("/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace {cs} /BitsPerComponent 8", img.width, img.height);
            match img.encoding {
                ImageEncoding::Flate => self.stream(&format!("{dict} /Filter /FlateDecode"), &deflate(&img.data))?,
                ImageEncoding::Jpeg => self.stream(&format!("{dict} /Filter /DCTDecode"), &img.data)?,
            };
            if let (Some(o), Some(profile)) = (new_icc, &img.icc) {
                self.stream(&format!("/N {} /Alternate {device} /Filter /FlateDecode", img.channels), &deflate(profile))?;
                self.icc = Some((profile.clone(), o));
            }
        }
        for p in &page.plates {
            // A tint transform from the colorant's density to its alternate: 0 → paper, 1 → `cmyk`
            // (an exponential interpolation function with N = 1 is linear).
            let cmyk = p.cmyk.map(|v| format!("{:.4}", v.clamp(0.0, 1.0))).join(" ");
            let cs = format!("[/Separation {} /DeviceCMYK << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [{cmyk}] /N 1 >>]", pdf_name(&p.name));
            // No mask on the plate: where it has no ink it paints "0" of its own colorant, which a
            // separating RIP reads as no ink and overprint keeps the other plates under. (A stencil
            // or colour-key mask would let plain viewers show every plate, but Illustrator reports
            // an explicit image mask as an unknown imaging construct, and Quartz draws nothing for a
            // colour-key-masked Separation image.)
            let dict =
                format!("/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace {cs} /BitsPerComponent 8 /Filter /FlateDecode", p.width, p.height);
            self.stream(&dict, &deflate(&p.data))?;
        }
        if let Some(o) = new_gs {
            self.obj(b"<< /Type /ExtGState /OP true /op true /OPM 1 >>")?;
            self.overprint = Some(o);
        }
        if let Some(font) = selection_font {
            for body in font.objects(selection_base) {
                self.obj(&body)?;
            }
        }
        Ok(())
    }

    /// The finished file: the page tree, the cross-reference table and the trailer. Returns the
    /// sink (the bytes, for a `Vec`).
    pub fn finish(mut self) -> std::result::Result<W, IoError> {
        let kids: Vec<String> = self.kids.iter().map(|k| format!("{k} 0 R")).collect();
        let tree = format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), self.kids.len());
        self.write_at(2, tree.as_bytes())?;
        let mut catalog = String::from("<< /Type /Catalog /Pages 2 0 R");
        if let Some(p) = self.presentation {
            catalog.push_str(" /PageMode /FullScreen");
            if p.loop_after_last {
                let script = self.obj(b"<< /S /JavaScript /JS (app.fs.loop = true;) >>")?;
                let names = self.obj(format!("<< /Names [(PhotoCraft Presentation) {script} 0 R] >>").as_bytes())?;
                catalog.push_str(&format!(" /Names << /JavaScript {names} 0 R >>"));
            }
        }
        if let Some(xmp) = self.xmp.take() {
            let n = self.stream("/Type /Metadata /Subtype /XML", xmp.as_bytes())?;
            catalog.push_str(&format!(" /Metadata {n} 0 R"));
        }
        // A label tree is written only for a non-empty name: an all-empty tree would be `/Nums []`.
        // Every page then gets an entry: a number-tree range runs until the next key, so omitting
        // an empty name would make that page inherit the previous page's label. An empty
        // dictionary gives the page the default (decimal) label instead.
        if self.labels.iter().any(|l| !l.is_empty()) {
            let nums = self
                .labels
                .iter()
                .enumerate()
                .map(|(i, l)| if l.is_empty() { format!("{i} << >>") } else { format!("{i} << /P {} >>", pdf_unicode_string(l)) })
                .collect::<Vec<_>>()
                .join(" ");
            let o = self.obj(format!("<< /Nums [{nums}] >>").as_bytes())?;
            catalog.push_str(&format!(" /PageLabels {o} 0 R"));
        }
        catalog.push_str(" >>");
        self.write_at(1, catalog.as_bytes())?;
        let xref = self.pos;
        let mut table = format!("xref\n0 {}\n0000000000 65535 f \n", self.offsets.len() + 1);
        for o in &self.offsets {
            table.push_str(&format!("{o:010} 00000 n \n"));
        }
        table.push_str(&format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", self.offsets.len() + 1));
        // The table may pass MAX_BYTES: every offset in it is already below it.
        self.out.write_all(table.as_bytes()).map_err(|e| IoError::Unsupported(format!("writing the PDF failed: {e}")))?;
        self.out.flush().map_err(|e| IoError::Unsupported(format!("writing the PDF failed: {e}")))?;
        Ok(self.out)
    }
}

/// A one-page PDF.
pub fn print_pdf(page: &PrintPage) -> std::result::Result<Vec<u8>, IoError> {
    let mut w = PdfWriter::new();
    w.add_page(page)?;
    w.finish()
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
    Ok(PrintPage {
        paper: (pw, ph),
        image: Some(image),
        rect: (0.0, 0.0, pw, ph),
        marks: Marks::default(),
        description: None,
        label: None,
        page_label: None,
        plates: Vec::new(),
    })
}

/// `doc`'s size in points at its resolution.
pub fn print_size(doc: &Document) -> (f64, f64) {
    let dpi = f64::from(doc.resolution_dpi.max(1.0));
    (f64::from(doc.size.width) * 72.0 / dpi, f64::from(doc.size.height) * 72.0 / dpi)
}

/// `s` as XML character data.
fn xml_text(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            c => o.push(c),
        }
    }
    o
}

/// The RDF namespace every XMP packet's `rdf:RDF` element is in.
const RDF_NS: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";

/// Where a description joins an XMP packet's RDF element, found as XML rather than by its
/// spelling, so the prefix the packet binds to RDF, whitespace in its end tag and markup in
/// comments make no difference: the byte offset of the element's end tag (`closed` false), or of
/// the `/>` of a self-closing element (`closed` true) together with its qualified name, so the
/// caller can expand it. None when the packet does not parse or has no RDF element.
fn rdf_insertion(xmp: &str) -> Option<(usize, Option<&str>)> {
    // roxmltree does not take a byte order mark ahead of the prolog; offsets below are into `xmp`.
    let bom = xmp.strip_prefix('\u{feff}').map_or(0, |_| '\u{feff}'.len_utf8());
    let body = xmp.get(bom..)?;
    let doc = crate::xml::parse(body, roxmltree::ParsingOptions::default()).ok()?;
    let rdf = doc.descendants().find(|n| n.is_element() && n.tag_name().namespace() == Some(RDF_NS) && n.tag_name().name() == "RDF")?;
    let range = rdf.range();
    let text = body.get(range.clone())?;
    if let Some(open) = text.strip_suffix("/>") {
        // `<r:RDF …/>`: the qualified name follows the `<`, up to whitespace or the `/`.
        let name = open.get(1..)?;
        let end = name.find(|c: char| c.is_whitespace() || c == '/').unwrap_or(name.len());
        return Some((bom + range.end - 2, Some(name.get(..end)?)));
    }
    // The end tag is the last thing in the element's own range.
    Some((bom + range.start + text.rfind("</")?, None))
}

/// The XMP namespace of the spot-plate description.
pub const SPOT_PLATES_NS: &str = "http://ns.getartcraft.com/photocraft/spot-plates/1.0/";

/// The XMP packet of a spot-plate PDF: the document's own packet (`doc_xmp`, if any) with one
/// `rdf:Description` added under [`SPOT_PLATES_NS`] that lists the plates in painting order, so a
/// converter can rebuild each spot channel with its real ink colour, which the Separation
/// colour space's CMYK alternate cannot carry:
///
/// ```xml
/// <rdf:Description rdf:about="" xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"
///                  xmlns:pcsp="http://ns.getartcraft.com/photocraft/spot-plates/1.0/">
///  <pcsp:plates>
///   <rdf:Seq>
///    <rdf:li rdf:parseType="Resource">
///     <pcsp:index>0</pcsp:index>                 <!-- 0-based, the order the plates are painted -->
///     <pcsp:name>Underbase White</pcsp:name>     <!-- the Separation colorant name, as the channel is named -->
///     <pcsp:labL>100.00</pcsp:labL>              <!-- ink colour, CIE L*a*b*, D50 -->
///     <pcsp:labA>0.00</pcsp:labA>
///     <pcsp:labB>0.00</pcsp:labB>
///     <pcsp:srgb>#FFFFFF</pcsp:srgb>             <!-- the same colour as encoded sRGB, for convenience -->
///     <pcsp:solidity>100</pcsp:solidity>         <!-- 0–100, Photoshop's Spot Channel Options › Solidity -->
///    </rdf:li>
///   </rdf:Seq>
///  </pcsp:plates>
/// </rdf:Description>
/// ```
///
/// An alpha-type channel of a Multichannel document lists as black with solidity 0. The plates
/// are the document's, the same on every page of an artboard export.
fn plates_xmp(doc_xmp: Option<&str>, plates: &[SpotPlate]) -> String {
    // The description binds `rdf` itself: the packet it joins may bind another prefix to RDF.
    let mut d = format!("  <rdf:Description rdf:about=\"\" xmlns:rdf=\"{RDF_NS}\" xmlns:pcsp=\"{SPOT_PLATES_NS}\">\n   <pcsp:plates>\n    <rdf:Seq>\n");
    // Two decimals, never "-0.00".
    let two = |v: f32| {
        let v = (v * 100.0).round() / 100.0;
        if v == 0.0 { 0.0 } else { v }
    };
    for (i, p) in plates.iter().enumerate() {
        let hex = p.srgb.map(|v| format!("{:02X}", (v.clamp(0.0, 1.0) * 255.0).round() as u8)).concat();
        d.push_str(&format!(
            "     <rdf:li rdf:parseType=\"Resource\">\n      <pcsp:index>{i}</pcsp:index>\n      <pcsp:name>{}</pcsp:name>\n      <pcsp:labL>{:.2}</pcsp:labL>\n      <pcsp:labA>{:.2}</pcsp:labA>\n      <pcsp:labB>{:.2}</pcsp:labB>\n      <pcsp:srgb>#{hex}</pcsp:srgb>\n      <pcsp:solidity>{}</pcsp:solidity>\n     </rdf:li>\n",
            xml_text(&p.name),
            two(p.lab[0]),
            two(p.lab[1]),
            two(p.lab[2]),
            (p.solidity * 100.0).round() as u32
        ));
    }
    d.push_str("    </rdf:Seq>\n   </pcsp:plates>\n  </rdf:Description>\n");
    // Into the document's packet when it has one, inside its RDF element; else a packet of its
    // own. A packet that does not parse or has no RDF element has nowhere for the description to
    // go and is kept as it is rather than replaced; so is one whose insertion offset falls off a
    // char boundary, which the parser never yields but a document-supplied packet must not panic.
    if let Some(xmp) = doc_xmp {
        let Some((at, self_closing)) = rdf_insertion(xmp) else {
            return xmp.to_owned();
        };
        // A self-closing element's tail skips its `/>`.
        let tail_at = if self_closing.is_some() { at + 2 } else { at };
        let (Some(head), Some(tail)) = (xmp.get(..at), xmp.get(tail_at..)) else {
            return xmp.to_owned();
        };
        let mut out = String::with_capacity(xmp.len() + d.len() + 16);
        out.push_str(head);
        match self_closing {
            // `<r:RDF …/>` becomes `<r:RDF …>` + the description + `</r:RDF>`.
            Some(name) => {
                out.push_str(">\n");
                out.push_str(&d);
                out.push_str(" </");
                out.push_str(name);
                out.push('>');
            }
            None => out.push_str(&d),
        }
        out.push_str(tail);
        return out;
    }
    format!(
        "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n <rdf:RDF xmlns:rdf=\"{RDF_NS}\">\n{d} </rdf:RDF>\n</x:xmpmeta>\n<?xpacket end=\"w\"?>"
    )
}

/// The channels of `doc` that leave as plates, in channel order (the first prints first,
/// underneath): every channel of a Multichannel document, the spot channels of any other.
fn plate_channels(doc: &Document) -> impl Iterator<Item = &photocraft_doc::AlphaChannel> {
    let multichannel = doc.mode == ColorMode::Multichannel;
    doc.channels.iter().filter(move |ch| multichannel || ch.spot.is_some())
}

/// The spot plates of `doc` over `rect` (the canvas, or one artboard's rect), in channel order.
///
/// In a Multichannel document every channel is a plate: its spot channels at the ink density
/// they store (1 = solid), an alpha-type channel as the transparent black ink the composite shows
/// it as (1 − value, as `photocraft_compose::multichannel` prints it). Any other document keeps
/// its alpha channels as masks and exports only its spot channels. The alternate colour is the
/// ink colour separated through the CMYK profile in use (`Color::in_mode`); it only previews the
/// plate, as Photoshop's does. Solidity is a preview attribute too: it is not part of the plate
/// image or its tint transform, and only the XMP description ([`plates_xmp`]) carries it.
fn plates_of(doc: &Document, rect: photocraft_geom::Rect) -> Vec<SpotPlate> {
    let to8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    plate_channels(doc)
        .map(|ch| {
            let (ink, alpha_type) = match ch.spot {
                Some((ink, _)) => (ink, false),
                None => (photocraft_color::Color::BLACK, true),
            };
            let n = ch.surface.channels().max(1);
            let values = ch.surface.read_region(rect);
            let data: Vec<u8> = values.chunks(n).map(|px| px.first().copied().unwrap_or(0.0)).map(|v| to8(if alpha_type { 1.0 - v } else { v })).collect();
            let srgb = ink.to_rgb();
            SpotPlate {
                name: ch.name.clone(),
                cmyk: ink.in_mode(ColorMode::Cmyk).c,
                lab: photocraft_color::convert::srgb_to_lab(srgb),
                srgb,
                solidity: ch.spot.map_or(0.0, |(_, s)| s.clamp(0.0, 1.0)),
                width: rect.width(),
                height: rect.height(),
                data,
            }
        })
        .collect()
}

/// The vector operators for the contiguous visible top prefix of `doc`, top to bottom, with the
/// indices of the layers they replace, and the selection font covering their text (plus, when a
/// lower candidate is rejected during emission, that layer's unused characters).
/// Hidden and blocked layers contribute no characters, so they cannot exhaust the font's codes.
fn collect_vectors(doc: &Document) -> (Vec<(usize, String)>, selection::SelectionFont) {
    enum Candidate<'a> {
        Ops(String),
        Text(text::PreparedText<'a>),
    }
    let mut candidates = Vec::new();
    if doc.icc_profile.is_none() {
        let mut repertoire = std::collections::BTreeSet::new();
        for (index, layer) in doc.layers.iter().enumerate().rev() {
            if !layer.visible {
                continue;
            }
            if layer.blend != photocraft_color::BlendMode::Normal
                || layer.opacity != 1.0
                || layer.fill_opacity != 1.0
                || layer.clipped
                || layer.mask.is_some()
                || layer.vector_mask.is_some()
                || (layer.effects.enabled && !layer.effects.items.is_empty())
                || layer.excluded_channels != 0
                || layer.blend_if != Default::default()
                || layer.advanced != Default::default()
                || layer.video.is_some()
            {
                break;
            }
            let candidate = match &layer.content {
                photocraft_doc::LayerContent::Shape(shape) => match shapes::shape_path(shape, doc.mode) {
                    Some(ops) => Candidate::Ops(ops),
                    None => break,
                },
                photocraft_doc::LayerContent::Text(text_layer) => {
                    let Some(prepared) = text::prepare(text_layer, doc.mode, doc.resolution_dpi) else { break };
                    for ch in text_layer.text.chars() {
                        repertoire.insert(ch);
                        if repertoire.len() > 255 {
                            break;
                        }
                    }
                    if repertoire.len() > 255 {
                        break;
                    }
                    Candidate::Text(prepared)
                }
                _ => break,
            };
            candidates.push((index, candidate));
        }
    }
    let selection_font = selection::SelectionFont::new(candidates.iter().filter_map(|(_, candidate)| match candidate {
        Candidate::Text(prepared) => Some(prepared.text()),
        Candidate::Ops(_) => None,
    }));
    let mut layers = Vec::with_capacity(candidates.len());
    for (index, candidate) in candidates {
        let ops = match candidate {
            Candidate::Ops(ops) => ops,
            Candidate::Text(prepared) => match text::ops(&prepared, doc.mode, &selection_font) {
                Some(ops) => ops,
                // Keep the successful upper vectors; this layer and everything below stay raster.
                None => break,
            },
        };
        layers.push((index, ops));
    }
    (layers, selection_font)
}

/// Pixels in `rect`, saturating on overflow.
fn rect_pixels(rect: photocraft_geom::Rect) -> u64 {
    u64::from(rect.width()).saturating_mul(u64::from(rect.height()))
}

/// File › Save As / Save a Copy › PDF (#3401): the flattened document at its print size
/// (pixels ÷ resolution × 72 pt). One page per artboard when the document has artboards, else one
/// page for the whole document. Layers and transparency are dropped — the warning says so — and
/// the embedded profile travels with the images.
///
/// On a whole-document page the topmost simple shape and text layers stay vectors over the
/// flattened rest, with selectable text ([`collect_vectors`]); artboard pages are raster.
///
/// With `spot_plates`, the document's spot channels follow each page's image and vectors as
/// overprinting `/Separation` plates (FILE-215-17), an artboard page carrying its own region of
/// each, and a Multichannel document, which is nothing but its channels, is written as plates
/// alone. Without it a Multichannel document has nothing to write and is refused.
pub fn export(doc: &Document, spot_plates: bool, ctl: &photocraft_raster::Interrupt) -> std::result::Result<ExportResult, IoError> {
    export_with(doc, spot_plates, doc.metadata.xmp.as_deref(), ctl)
}

/// [`export`] with the XMP packet to embed (`doc_xmp`: the document's, or `None` when the save
/// embeds no metadata). With spot plates the packet gains their description ([`plates_xmp`]);
/// without plates it is written as it is, and nothing when there is none.
pub fn export_with(doc: &Document, spot_plates: bool, doc_xmp: Option<&str>, ctl: &photocraft_raster::Interrupt) -> std::result::Result<ExportResult, IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let (w, h) = (usize::try_from(doc.size.width).unwrap_or(0), usize::try_from(doc.size.height).unwrap_or(0));
    if w == 0 || h == 0 {
        return Err(IoError::Unsupported("PDF needs a document with non-zero width and height; nothing was written".into()));
    }
    // The composite's channels per pixel; None for a Multichannel document, which has no composite.
    let device = match doc.mode {
        ColorMode::Rgb => Some(3),
        ColorMode::Grayscale => Some(1),
        ColorMode::Cmyk => Some(4),
        ColorMode::Multichannel if spot_plates && !doc.channels.is_empty() => None,
        ColorMode::Multichannel => {
            return Err(IoError::Unsupported(
                "a Multichannel PDF is its channels as spot plates; with no channels, or spot plates left out, there is nothing to write".into(),
            ));
        }
        m => {
            return Err(IoError::Unsupported(format!(
                "PDF saves RGB, Grayscale, CMYK and Multichannel documents; {m:?} needs converting to RGB first (Image > Mode)"
            )));
        }
    };
    let fmt = doc.pixel_format();
    let n = fmt.channels().max(1);
    let dpi = f64::from(doc.resolution_dpi.max(1.0));
    let paper = if doc.mode == ColorMode::Cmyk { 0.0 } else { 1.0 };
    let to8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let icc = doc.icc_profile.as_ref().map(|p| p.to_vec());
    let plate_count = if spot_plates { plate_channels(doc).count() } else { 0 };
    // Straight alpha, composited here over paper: white in RGB/Gray (1 = full light), zero ink
    // in CMYK (0 = paper).
    let image_of = |values: &[f32], rect: photocraft_geom::Rect, device: usize| {
        let mut data = Vec::with_capacity((rect.width() as usize * rect.height() as usize).saturating_mul(device));
        for px in values.chunks(n) {
            let a = px.last().copied().unwrap_or(1.0).clamp(0.0, 1.0);
            for v in px.iter().take(device) {
                data.push(to8(v * a + paper * (1.0 - a)));
            }
        }
        PrintImage { width: rect.width(), height: rect.height(), channels: device, data, icc: icc.clone(), encoding: ImageEncoding::Flate }
    };
    let page_of = |image: Option<PrintImage>, rect: photocraft_geom::Rect, page_label: Option<String>| {
        let (pw, ph) = (f64::from(rect.width()) * 72.0 / dpi, f64::from(rect.height()) * 72.0 / dpi);
        let plates = if spot_plates { plates_of(doc, rect) } else { Vec::new() };
        PrintPage { paper: (pw, ph), image, rect: (0.0, 0.0, pw, ph), marks: Marks::default(), description: None, label: None, page_label, plates }
    };
    let mut warnings = Vec::new();
    if device.is_some() {
        warnings.push("PDF was saved flattened: layers, masks-as-editable and transparency are not preserved (#3401)".into());
    }
    let artboards = doc.artboards();
    let (pages, vectors, selection_font) = if !artboards.is_empty() {
        // Every page's encoded pixels are kept until the PDF is assembled; bound that peak.
        const MAX_PDF_EXPORT_BYTES: u64 = 1 << 30;
        let page_pixels: u64 = artboards.iter().fold(0u64, |bytes, (_, _, a)| bytes.saturating_add(rect_pixels(a.rect)));
        // A page keeps `device` bytes a pixel for its composite and one a pixel for each plate.
        let retained = page_pixels.saturating_mul((device.unwrap_or(0) + plate_count) as u64);
        // One page renders at a time: its f32 composite surface plus the region read back from
        // it, both `n` channels at 4 bytes per pixel. The whole-document canvas is never held.
        let largest = artboards.iter().map(|(_, _, a)| rect_pixels(a.rect)).max().unwrap_or(0);
        let temporary = largest.saturating_mul(n as u64).saturating_mul(4).saturating_mul(2);
        let estimate = retained.saturating_add(temporary);
        if estimate > MAX_PDF_EXPORT_BYTES {
            return Err(IoError::Unsupported(format!(
                "PDF export would need about {} MB for {} artboards; export them in smaller batches",
                estimate >> 20,
                artboards.len()
            )));
        }
        // One document shell is reused for every page. Each page renders a document holding only
        // its artboard group (the same isolation as `artboard_document`), so an overlapping board
        // cannot paint on another board's page. Layer pixels are shared with `doc`; this one
        // clone and the per-page group clone hold no new pixel buffers, but they are outside the
        // 1 GiB estimate because they are not retained page data.
        let mut one = doc.clone();
        let mut pages = Vec::with_capacity(artboards.len());
        for (id, name, artboard) in &artboards {
            ctl.check().map_err(|_| IoError::Cancelled)?;
            let rect = artboard.rect;
            if rect.width() == 0 || rect.height() == 0 {
                continue;
            }
            // A hidden artboard still gets its page, rendered from its own content rather than
            // left blank by the group's visibility.
            let Some(layer) = doc.layer(*id) else { continue };
            let image = device.map(|device| {
                let mut group = layer.clone();
                group.visible = true;
                one.layers.clear();
                one.layers.push(group);
                // Render only this artboard's rect; the surface never holds the whole document.
                let surface = photocraft_compose::composite_to_surface(&one, rect, PixelFormat::new(fmt.mode, fmt.sample, true), None);
                image_of(&surface.read_region(rect), rect, device)
            });
            pages.push(page_of(image, rect, Some((*name).to_string())));
        }
        if pages.is_empty() {
            return Err(IoError::Unsupported("PDF: the document's artboards have no area".into()));
        }
        warnings.push(format!("PDF exported {} artboards as {} pages; vector/text layers on artboards are rasterised", artboards.len(), pages.len()));
        let vectors = vec![String::new(); pages.len()];
        (pages, vectors, selection::SelectionFont::default())
    } else {
        let canvas = photocraft_geom::Rect::new(0, 0, w as i32, h as i32);
        // Simple topmost shapes and text layers can be drawn over the flattened backdrop without
        // changing compositing. Stop at the first unsupported visible layer, since it may depend
        // on the backdrop (adjustments, blend modes, masks, effects, or clipping). A Multichannel
        // document has no composite for them to sit on: it is its plates alone.
        let (vector_layers, selection_font) = if device.is_some() { collect_vectors(doc) } else { (Vec::new(), selection::SelectionFont::default()) };
        let image = device.map(|device| {
            let mut backdrop = doc.clone();
            for (index, _) in &vector_layers {
                if let Some(layer) = backdrop.layers.get_mut(*index) {
                    layer.visible = false;
                }
            }
            let surface = photocraft_compose::flatten_to_surface(&backdrop, PixelFormat::new(fmt.mode, fmt.sample, true), None);
            image_of(&surface.read_region(canvas), canvas, device)
        });
        // The vector layers, bottom to top, in the document's pixel space (y down), over the
        // image and under the plates.
        let mut vectors = String::new();
        if !vector_layers.is_empty() {
            let scale = 72.0 / dpi;
            let ph = f64::from(doc.size.height) * scale;
            vectors.push_str(&format!("q {scale} 0 0 -{scale} 0 {ph} cm\n"));
            for (_, ops) in vector_layers.iter().rev() {
                vectors.push_str(ops);
            }
            vectors.push_str("Q\n");
        }
        (vec![page_of(image, canvas, None)], vec![vectors], selection_font)
    };
    // The plates are the document's: every page has the same ones, over its own region.
    let plates = pages.first().map(|p| p.plates.as_slice()).unwrap_or_default();
    if !plates.is_empty() {
        let names: Vec<&str> = plates.iter().map(|p| p.name.as_str()).collect();
        warnings.push(format!(
            "{} spot plate{} written as overprinting Separation colorants ({}), 8-bit; their preview colours are the inks separated to CMYK",
            plates.len(),
            if plates.len() == 1 { "" } else { "s" },
            names.join(", ")
        ));
    } else if doc.channels.iter().any(|ch| ch.spot.is_some()) {
        warnings.push("spot channels were not written: spot plates were left out".into());
    }
    let xmp = if plates.is_empty() { doc_xmp.map(str::to_owned) } else { Some(plates_xmp(doc_xmp, plates)) };
    if !doc.metadata.text.is_empty() {
        warnings.push("free-form text metadata is not supported by PDF; it will be dropped".into());
    }
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let mut out = PdfWriter::new();
    out.set_metadata(xmp);
    for (i, page) in pages.iter().enumerate() {
        // Whole-document pages may carry live vectors and their selection font; artboard pages
        // are raster only.
        let v = vectors.get(i).map(String::as_str).unwrap_or_default();
        let font = artboards.is_empty().then_some(&selection_font);
        out.add_page_with_vectors(page, v, font)?;
    }
    Ok(ExportResult { bytes: out.finish()?, warnings })
}

#[cfg(test)]
mod tests;
