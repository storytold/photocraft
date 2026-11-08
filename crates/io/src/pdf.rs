//! The PDF writer (ISO 32000-1 basics): one raster page per file.
//!
//! A small, self-contained writer used by two paths: File › Save a Copy / Save As › PDF (#3401)
//! through [`export`], and File › Print through [`print_pdf`] (called by the engine, which adds
//! paper geometry, printer marks and labels around the image). One page holds a
//! Flate-compressed image XObject, optional printer marks as vector operators and optional
//! Helvetica text; an embedded ICC profile rides in an ICCBased colour space. [`export_with_options`]
//! writes PDF/X-1a:2001, PDF/X-3:2002 and PDF/X-4 for print ([`PdfXStandard`]). Multi-page PDF,
//! layer-preserving PDF and PDF import stay open (FILE-215-10).

use std::io::Write as _;

use photocraft_color::{ColorMode, PixelFormat};
use photocraft_doc::Document;

use crate::{ExportResult, IoError};

mod selection;
mod shapes;
mod text;

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
    /// The page's name in the PDF `/PageLabels` tree (Unicode-capable; not drawn on the page).
    pub page_label: Option<String>,
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
    print_pdf_pages(std::slice::from_ref(page), &[String::new()], &selection::SelectionFont::default(), &photocraft_raster::Interrupt::NONE).unwrap_or_default()
}

/// A PDF with one page per entry; `vectors[i]` (when present) is drawn over page `i`.
fn print_pdf_pages(
    pages: &[PrintPage],
    vectors: &[String],
    selection_font: &selection::SelectionFont,
    ctl: &photocraft_raster::Interrupt,
) -> std::result::Result<Vec<u8>, IoError> {
    if ctl.check().is_err() {
        return Err(IoError::Cancelled);
    }
    let Some(first) = pages.first() else { return Ok(Vec::new()) };
    let mut out: Vec<u8> = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets: Vec<usize> = Vec::new();
    let mut obj = |out: &mut Vec<u8>, body: &[u8]| {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", offsets.len()).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };
    let n = pages.len();
    // A label tree is written only for a non-empty name: an all-empty tree would be `/Nums []`,
    // and filtering the first empty name away would start the tree at page index 1, not 0.
    let labels_on = pages.iter().any(|p| p.page_label.as_deref().is_some_and(|label| !label.is_empty()));
    // 1 catalog, 2 pages, then per page: page, contents, image; then the shared font/ICC/selection.
    let font_obj = 3 + n * 3;
    let icc = first.image.icc.as_ref();
    let icc_obj = icc.map(|_| font_obj + 1);
    let selection_base = font_obj + if icc.is_some() { 2 } else { 1 };
    let selection_objects = selection_font.objects(selection_base);
    let labels_obj = labels_on.then_some((selection_base + selection_objects.len()) as u32);
    obj(&mut out, format!("<< /Type /Catalog /Pages 2 0 R{} >>", labels_obj.map(|o| format!(" /PageLabels {o} 0 R")).unwrap_or_default()).as_bytes());
    let kids = (0..n).map(|i| format!("{} 0 R", 3 + i * 3)).collect::<Vec<_>>().join(" ");
    obj(&mut out, format!("<< /Type /Pages /Kids [{kids}] /Count {n} >>").as_bytes());
    for (i, page) in pages.iter().enumerate() {
        let page_obj = 3 + i * 3;
        let content_obj = page_obj + 1;
        let image_obj = page_obj + 2;
        let (pw, ph) = page.paper;
        let (x, y, w, h) = page.rect;
        obj(
            &mut out,
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {pw:.2} {ph:.2}] /Resources << /XObject << /Im0 {image_obj} 0 R >> /Font << /F1 {font_obj} 0 R /FSelect {selection_base} 0 R >> >> /Contents {content_obj} 0 R >>"
            )
            .as_bytes(),
        );
        // An explicit white page, so every viewer (and rasteriser) shows paper, not transparency.
        let mut content = format!("q 1 g 0 0 {pw:.2} {ph:.2} re f Q\nq {w:.3} 0 0 {h:.3} {x:.3} {y:.3} cm /Im0 Do Q\n");
        if let Some(v) = vectors.get(i) {
            content.push_str(v);
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
        let c = deflate(content.as_bytes());
        let mut body = format!("<< /Length {} /Filter /FlateDecode >>\nstream\n", c.len()).into_bytes();
        body.extend_from_slice(&c);
        body.extend_from_slice(b"\nendstream");
        obj(&mut out, &body);
        let img = &page.image;
        let device = match img.channels {
            1 => "/DeviceGray",
            4 => "/DeviceCMYK",
            _ => "/DeviceRGB",
        };
        let cs = match icc_obj {
            Some(obj) => format!("[/ICCBased {obj} 0 R]"),
            None => device.to_string(),
        };
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
    }
    obj(&mut out, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>");
    if let Some(icc) = icc {
        let device = match first.image.channels {
            1 => "/DeviceGray",
            4 => "/DeviceCMYK",
            _ => "/DeviceRGB",
        };
        let z = deflate(icc);
        let mut body = format!("<< /N {} /Alternate {device} /Filter /FlateDecode /Length {} >>\nstream\n", first.image.channels, z.len()).into_bytes();
        body.extend_from_slice(&z);
        body.extend_from_slice(b"\nendstream");
        obj(&mut out, &body);
    }
    for body in &selection_objects {
        obj(&mut out, body);
    }
    if labels_obj.is_some() {
        // Every page gets an entry: a number-tree range runs until the next key, so omitting an
        // empty name would make that page inherit the previous page's label. An empty
        // dictionary gives the page the default (decimal) label instead.
        let nums = pages
            .iter()
            .enumerate()
            .map(|(i, p)| match p.page_label.as_deref().filter(|label| !label.is_empty()) {
                Some(label) => format!("{i} << /P {} >>", pdf_unicode_string(label)),
                None => format!("{i} << >>"),
            })
            .collect::<Vec<_>>()
            .join(" ");
        obj(&mut out, format!("<< /Nums [{nums}] >>").as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes());
    for o in &offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", offsets.len() + 1).as_bytes());
    Ok(out)
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
pub fn export(doc: &Document, ctl: &photocraft_raster::Interrupt) -> std::result::Result<ExportResult, IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
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
    let dpi = f64::from(doc.resolution_dpi.max(1.0));
    let paper = if doc.mode == ColorMode::Cmyk { 0.0 } else { 1.0 };
    let to8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let icc = doc.icc_profile.as_ref().map(|p| p.to_vec());
    let artboards = doc.artboards();
    if !artboards.is_empty() {
        // Every page's encoded pixels are kept until the PDF is assembled; bound that peak.
        const MAX_PDF_EXPORT_BYTES: u64 = 1 << 30;
        let page_pixels: u64 = artboards.iter().fold(0u64, |bytes, (_, _, a)| bytes.saturating_add(rect_pixels(a.rect)));
        let retained = page_pixels.saturating_mul(device as u64);
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
            let (aw, ah) = (rect.width(), rect.height());
            if aw == 0 || ah == 0 {
                continue;
            }
            // A hidden artboard still gets its page, rendered from its own content rather than
            // left blank by the group's visibility.
            let Some(layer) = doc.layer(*id) else { continue };
            let mut group = layer.clone();
            group.visible = true;
            one.layers.clear();
            one.layers.push(group);
            // Render only this artboard's rect; the surface never holds the whole document.
            let surface = photocraft_compose::composite_to_surface(&one, rect, PixelFormat::new(fmt.mode, fmt.sample, true), None);
            let values = surface.read_region(rect);
            let mut data = Vec::with_capacity((aw as usize * ah as usize).saturating_mul(device));
            for px in values.chunks(n) {
                let a = px.last().copied().unwrap_or(1.0).clamp(0.0, 1.0);
                for v in px.iter().take(device) {
                    data.push(to8(v * a + paper * (1.0 - a)));
                }
            }
            let image = PrintImage { width: aw, height: ah, channels: device, data, icc: icc.clone() };
            let (pw, ph) = (f64::from(aw) * 72.0 / dpi, f64::from(ah) * 72.0 / dpi);
            pages.push(PrintPage {
                paper: (pw, ph),
                image,
                rect: (0.0, 0.0, pw, ph),
                marks: Marks::default(),
                description: None,
                label: None,
                page_label: Some((*name).to_string()),
            });
        }
        if pages.is_empty() {
            return Err(IoError::Unsupported("PDF: the document's artboards have no area".into()));
        }
        let mut warnings = vec!["PDF was saved flattened: layers, masks-as-editable and transparency are not preserved (#3401)".into()];
        warnings.push(format!("PDF exported {} artboards as {} pages; vector/text layers on artboards are rasterised", artboards.len(), pages.len()));
        if !doc.metadata.text.is_empty() {
            warnings.push("free-form text metadata is not supported by PDF; it will be dropped".into());
        }
        let vectors = vec![String::new(); pages.len()];
        return Ok(ExportResult { bytes: print_pdf_pages(&pages, &vectors, &selection::SelectionFont::default(), ctl)?, warnings });
    }
    let canvas = photocraft_geom::Rect::new(0, 0, w as i32, h as i32);
    // Simple topmost shapes and text layers can be drawn over the flattened backdrop without changing
    // compositing. Stop at the first unsupported visible layer, since it may depend on the
    // backdrop (adjustments, blend modes, masks, effects, or clipping).
    let (vector_layers, selection_font) = collect_vectors(doc);
    let mut backdrop = doc.clone();
    for (index, _) in &vector_layers {
        if let Some(layer) = backdrop.layers.get_mut(*index) {
            layer.visible = false;
        }
    }
    let surface = photocraft_compose::flatten_to_surface(&backdrop, PixelFormat::new(fmt.mode, fmt.sample, true), None);
    let values = surface.read_region(canvas);
    // Straight alpha, composited here over paper: white in RGB/Gray (1 = full light), zero ink
    // in CMYK (0 = paper).
    let mut data = Vec::with_capacity((w * h).saturating_mul(device));
    for px in values.chunks(n) {
        let a = px.last().copied().unwrap_or(1.0).clamp(0.0, 1.0);
        for v in px.iter().take(device) {
            data.push(to8(v * a + paper * (1.0 - a)));
        }
    }
    let image = PrintImage { width: doc.size.width, height: doc.size.height, channels: device, data, icc: icc.clone() };
    let (pw, ph) = (f64::from(doc.size.width) * 72.0 / dpi, f64::from(doc.size.height) * 72.0 / dpi);
    let page = PrintPage { paper: (pw, ph), image, rect: (0.0, 0.0, pw, ph), marks: Marks::default(), description: None, label: None, page_label: None };
    let mut warnings = vec!["PDF was saved flattened: layers, masks-as-editable and transparency are not preserved (#3401)".into()];
    if !doc.metadata.text.is_empty() {
        warnings.push("free-form text metadata is not supported by PDF; it will be dropped".into());
    }
    let scale = 72.0 / dpi;
    let mut vectors = format!("q {scale} 0 0 -{scale} 0 {ph} cm\n");
    for (_, ops) in vector_layers.iter().rev() {
        vectors.push_str(ops);
    }
    vectors.push_str("Q\n");
    Ok(ExportResult { bytes: print_pdf_pages(&[page], &[vectors], &selection_font, ctl)?, warnings })
}

/// A PDF/X standard (ISO 15930) a save can claim, as Photoshop's PDF presets name them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PdfXStandard {
    /// PDF/X-1a:2001 (ISO 15930-1, PDF 1.3): device CMYK or gray only, in the output condition.
    X1a2001,
    /// PDF/X-3:2002 (ISO 15930-3, PDF 1.3): device colour in the output condition, or colour
    /// tagged with an ICC version 2 profile and converted by the printer.
    X3_2002,
    /// PDF/X-4 (ISO 15930-7, PDF 1.6): as PDF/X-3, with ICC version 4 profiles and XMP metadata.
    X4,
}

impl PdfXStandard {
    pub const ALL: [PdfXStandard; 3] = [PdfXStandard::X1a2001, PdfXStandard::X3_2002, PdfXStandard::X4];

    /// The id scripts and recorded actions use (`"pdfStandard"`).
    pub fn id(self) -> &'static str {
        match self {
            PdfXStandard::X1a2001 => "x1a2001",
            PdfXStandard::X3_2002 => "x3-2002",
            PdfXStandard::X4 => "x4-2008",
        }
    }

    /// The standard an id names; the short forms `x1a`, `x3` and `x4` are accepted too.
    pub fn from_id(id: &str) -> Option<Self> {
        match id.to_ascii_lowercase().as_str() {
            "x1a2001" | "x1a" | "x1a-2001" => Some(PdfXStandard::X1a2001),
            "x3-2002" | "x3" | "x32002" => Some(PdfXStandard::X3_2002),
            "x4-2008" | "x4" | "x42008" => Some(PdfXStandard::X4),
            _ => None,
        }
    }

    /// The name Photoshop's Standard menu shows.
    pub fn name(self) -> &'static str {
        match self {
            PdfXStandard::X1a2001 => "PDF/X-1a:2001",
            PdfXStandard::X3_2002 => "PDF/X-3:2002",
            PdfXStandard::X4 => "PDF/X-4:2008",
        }
    }

    /// Whether the file may embed an ICC profile of major version `major`: version 2 always,
    /// version 4 in PDF/X-4 only (PDF 1.3 predates it). There is no version 3.
    pub fn takes_icc_version(self, major: u8) -> bool {
        major == 2 || (major == 4 && self == PdfXStandard::X4)
    }

    /// The newest ICC major version the file may embed: PDF 1.3 knows version 2 only.
    pub fn max_icc_version(self) -> u8 {
        match self {
            PdfXStandard::X1a2001 | PdfXStandard::X3_2002 => 2,
            PdfXStandard::X4 => 4,
        }
    }

    /// Whether colour may stay in the document's own ICC profile (No Conversion) rather than be
    /// converted to the output condition; PDF/X-1a allows device colour only.
    pub fn keeps_icc_colour(self) -> bool {
        self != PdfXStandard::X1a2001
    }

    fn pdf_version(self) -> &'static str {
        match self {
            PdfXStandard::X1a2001 | PdfXStandard::X3_2002 => "1.3",
            PdfXStandard::X4 => "1.6",
        }
    }

    /// The Info dictionary's `GTS_PDFXVersion` (and the XMP `pdfxid:GTS_PDFXVersion`).
    fn version_key(self) -> &'static str {
        match self {
            PdfXStandard::X1a2001 => "PDF/X-1:2001",
            PdfXStandard::X3_2002 => "PDF/X-3:2002",
            PdfXStandard::X4 => "PDF/X-4",
        }
    }
}

/// What a PDF/X file adds to the page: the output intent, the trim and bleed boxes and the
/// identification its standard requires.
struct XPage<'a> {
    standard: PdfXStandard,
    /// The output profile (CMYK), embedded as the intent's `DestOutputProfile`.
    profile: &'a [u8],
    /// The profile's description.
    condition: &'a str,
    title: &'a str,
    /// The trim box (pt, origin bottom-left); the media box is the bleed box.
    trim: (f64, f64, f64, f64),
    /// When the file was written (UTC): the Info and XMP dates.
    created: chrono::DateTime<chrono::Utc>,
    /// 32 hex digits for the trailer's `/ID` (and the XMP document id).
    id: String,
}

/// A PDF text string: literal for printable ASCII, else UTF-16BE with a byte order mark, so a
/// title in any script survives (and matches the XMP title PDF/X-4 checks it against).
fn pdf_text(s: &str) -> String {
    if s.chars().all(|c| c.is_ascii() && !c.is_ascii_control()) {
        return pdf_string(s);
    }
    let mut o = String::from("<FEFF");
    for u in s.encode_utf16() {
        o.push_str(&format!("{u:04X}"));
    }
    o.push('>');
    o
}

fn xml_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            c if c.is_control() => o.push(' '),
            c => o.push(c),
        }
    }
    o
}

/// The XMP packet a PDF/X-4 file carries (ISO 15930-7): the same title, dates, producer and
/// trapping as the Info dictionary, the document ids and the PDF/X version.
fn xmp_packet(x: &XPage, producer: &str) -> String {
    let date = x.created.format("%Y-%m-%dT%H:%M:%S+00:00").to_string();
    let part = |a: usize, b: usize| x.id.get(a..b).unwrap_or("0");
    let doc_id = format!("uuid:{}-{}-{}-{}-{}", part(0, 8), part(8, 12), part(12, 16), part(16, 20), part(20, 32));
    let title = xml_escape(x.title);
    let producer = xml_escape(producer);
    let version = x.standard.version_key();
    [
        "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>",
        "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">",
        " <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">",
        "  <rdf:Description rdf:about=\"\"",
        "    xmlns:dc=\"http://purl.org/dc/elements/1.1/\"",
        "    xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\"",
        "    xmlns:pdf=\"http://ns.adobe.com/pdf/1.3/\"",
        "    xmlns:xmpMM=\"http://ns.adobe.com/xap/1.0/mm/\"",
        "    xmlns:pdfxid=\"http://www.npes.org/pdfx/ns/id/\">",
        "   <dc:format>application/pdf</dc:format>",
        &format!("   <dc:title><rdf:Alt><rdf:li xml:lang=\"x-default\">{title}</rdf:li></rdf:Alt></dc:title>"),
        &format!("   <xmp:CreateDate>{date}</xmp:CreateDate>"),
        &format!("   <xmp:ModifyDate>{date}</xmp:ModifyDate>"),
        &format!("   <xmp:MetadataDate>{date}</xmp:MetadataDate>"),
        "   <xmp:CreatorTool>PhotoCraft</xmp:CreatorTool>",
        &format!("   <pdf:Producer>{producer}</pdf:Producer>"),
        "   <pdf:Trapped>False</pdf:Trapped>",
        &format!("   <xmpMM:DocumentID>{doc_id}</xmpMM:DocumentID>"),
        &format!("   <xmpMM:InstanceID>{doc_id}</xmpMM:InstanceID>"),
        "   <xmpMM:VersionID>1</xmpMM:VersionID>",
        "   <xmpMM:RenditionClass>default</xmpMM:RenditionClass>",
        &format!("   <pdfxid:GTS_PDFXVersion>{version}</pdfxid:GTS_PDFXVersion>"),
        "  </rdf:Description>",
        " </rdf:RDF>",
        "</x:xmpmeta>",
        "<?xpacket end=\"w\"?>",
    ]
    .join("\n")
}

/// A stream object's body: `dict` (without the brackets) plus its length, then `data`.
fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut body = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
    body.extend_from_slice(data);
    body.extend_from_slice(b"\nendstream");
    body
}

/// A PDF/X file of one page: the image alone, with no text (an unembedded font isn't allowed).
fn write_pdf_x(page: &PrintPage, x: &XPage) -> Vec<u8> {
    let version = x.standard.pdf_version();
    let mut out: Vec<u8> = format!("%PDF-{version}\n").into_bytes();
    out.extend_from_slice(b"%\xe2\xe3\xcf\xd3\n");
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
    let icc = img.icc.as_deref();
    // 1 catalog, 2 pages, 3 page, 4 contents, 5 image, 6 the output profile, 7 the Info
    // dictionary, then the image's ICC profile (optional) and the XMP metadata (PDF/X-4).
    let icc_obj = 8;
    let xmp_obj = if icc.is_some() { 9 } else { 8 };
    let producer = format!("PhotoCraft {}", env!("CARGO_PKG_VERSION"));
    let metadata = if x.standard == PdfXStandard::X4 { format!(" /Metadata {xmp_obj} 0 R") } else { String::new() };
    obj(
                &mut out,
                format!(
                    // `Custom`: the embedded profile defines the condition. A registered name
                    // (FOGRA39, …) can't be verified from a profile, and a wrong one would claim
                    // another press condition. No `RegistryName`: it would say the identifier is
                    // an entry of that registry, which `Custom` isn't.
                    "<< /Type /Catalog /Pages 2 0 R{metadata} /OutputIntents [<< /Type /OutputIntent /S /GTS_PDFX /OutputConditionIdentifier (Custom) /OutputCondition {} /Info {} /DestOutputProfile 6 0 R >>] >>",
                    pdf_text(x.condition),
                    pdf_text(x.condition)
                )
                .as_bytes(),
            );
    obj(&mut out, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    let (tx0, ty0, tx1, ty1) = x.trim;
    obj(
                &mut out,
                format!(
                    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {pw:.2} {ph:.2}] /BleedBox [0 0 {pw:.2} {ph:.2}] /TrimBox [{tx0:.2} {ty0:.2} {tx1:.2} {ty1:.2}] /Resources << /XObject << /Im0 5 0 R >> >> /Contents 4 0 R >>"
                )
                .as_bytes(),
            );
    // An explicit white page, so every viewer (and rasteriser) shows paper, not transparency.
    let mut content = format!("q 1 g 0 0 {pw:.2} {ph:.2} re f Q\nq {w:.3} 0 0 {h:.3} {x0:.3} {y0:.3} cm /Im0 Do Q\n");
    content.push_str(&marks_ops(page.marks, page.rect));
    obj(&mut out, &stream("/Filter /FlateDecode", &deflate(content.as_bytes())));
    let device = match img.channels {
        1 => "/DeviceGray",
        4 => "/DeviceCMYK",
        _ => "/DeviceRGB",
    };
    let cs = if icc.is_some() { format!("[/ICCBased {icc_obj} 0 R]") } else { device.to_string() };
    obj(
        &mut out,
        &stream(
            &format!("/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace {cs} /BitsPerComponent 8 /Filter /FlateDecode", img.width, img.height),
            &deflate(&img.data),
        ),
    );
    obj(&mut out, &stream("/N 4 /Filter /FlateDecode", &deflate(x.profile)));
    let date = pdf_string(&x.created.format("D:%Y%m%d%H%M%S+00'00'").to_string());
    let conformance = if x.standard == PdfXStandard::X1a2001 { " /GTS_PDFXConformance (PDF/X-1a:2001)" } else { "" };
    obj(
        &mut out,
        format!(
            "<< /Title {} /Creator (PhotoCraft) /Producer {} /CreationDate {date} /ModDate {date} /Trapped /False /GTS_PDFXVersion {}{conformance} >>",
            pdf_text(x.title),
            pdf_string(&producer),
            pdf_string(x.standard.version_key()),
        )
        .as_bytes(),
    );
    if let Some(icc) = icc {
        obj(&mut out, &stream(&format!("/N {} /Alternate {device} /Filter /FlateDecode", img.channels), &deflate(icc)));
    }
    if x.standard == PdfXStandard::X4 {
        // Unfiltered, so preflight tools and asset managers read it without decoding.
        obj(&mut out, &stream("/Type /Metadata /Subtype /XML", xmp_packet(x, &producer).as_bytes()));
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes());
    for o in &offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    let extra = format!(" /Info 7 0 R /ID [<{}> <{}>]", x.id, x.id);
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

/// The PDF Options a save asks for (File › Save As › PDF): a plain PDF, or a PDF/X file for
/// print.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PdfOptions {
    /// `Some` writes a PDF/X file (ISO 15930), as Photoshop's PDF/X presets do.
    pub x: Option<PdfX>,
}

/// A PDF/X file for one output condition, described by an embedded CMYK output profile, with the
/// trim and bleed boxes a printer imposes by. With [`Self::convert`] (always for PDF/X-1a), colour
/// that isn't already in that condition is converted to it, as Photoshop's Output › Convert to
/// Destination does; otherwise (PDF/X-3 and PDF/X-4 only) it stays in the document's profile,
/// embedded with the image, and the printer converts it. A CMYK or gray document already in the
/// condition keeps its numbers either way.
#[derive(Clone, Debug, PartialEq)]
pub struct PdfX {
    pub standard: PdfXStandard,
    /// The CMYK output profile: ICC version 2 for PDF/X-1a and PDF/X-3 (PDF 1.3 predates v4),
    /// version 2 or 4 for PDF/X-4.
    pub profile: std::sync::Arc<Vec<u8>>,
    /// The profile the document's colour is in. `None` keeps a CMYK or gray document's numbers as
    /// the output condition's; an RGB document is then read as sRGB.
    pub source: Option<std::sync::Arc<Vec<u8>>>,
    /// Convert colour to [`Self::profile`] (Convert to Destination) rather than tag it with
    /// [`Self::source`] (No Conversion). PDF/X-1a always converts.
    pub convert: bool,
    pub intent: photocraft_cms::Intent,
    pub bpc: bool,
    /// The bleed already inside the document, in points: the trim box sits this far inside the
    /// page on every side.
    pub bleed_pt: f64,
}

/// 32 hex digits from the page `data`, the Info values in `info` (title, standard, output
/// condition) and `salt` (the save time), two FNV-1a passes, for the trailer's file `/ID` and the
/// XMP document id: ISO 32000 asks for the Info values in it, so two saves of the same pixels in
/// one second under other names get different ids.
fn file_id(data: &[u8], info: &[&str], salt: u64) -> String {
    let bytes = info.iter().flat_map(|s| s.as_bytes().iter().chain([&0u8])).chain(data.iter());
    let fnv = |seed: u64| bytes.clone().fold(seed ^ salt, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3));
    format!("{:016x}{:016x}", fnv(0xcbf2_9ce4_8422_2325), fnv(0x8422_2325_cbf2_9ce4))
}

/// File › Save As › PDF with [`PdfOptions`].
pub fn export_with_options(doc: &Document, opts: &PdfOptions, ctl: &photocraft_raster::Interrupt) -> std::result::Result<ExportResult, IoError> {
    let Some(x) = &opts.x else { return export(doc, ctl) };
    // `web_time` reads the browser's clock on wasm, where `std`'s has none.
    let secs = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    export_x(doc, x, secs, ctl)
}

fn export_x(doc: &Document, x: &PdfX, secs: u64, ctl: &photocraft_raster::Interrupt) -> std::result::Result<ExportResult, IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    use photocraft_cms::{Builtin, ColorSpace, Profile, Transform};
    let name = x.standard.name();
    let bad = |m: String| IoError::Unsupported(format!("{name}: {m}"));
    let dst = Profile::parse(&x.profile).map_err(|e| bad(format!("the output profile can't be read ({e})")))?;
    if dst.color_space != ColorSpace::Cmyk {
        return Err(bad(format!("the output condition needs a CMYK profile; \"{}\" is {:?}", dst.description, dst.color_space)));
    }
    if dst.class != photocraft_cms::ProfileClass::Output {
        return Err(bad(format!(
            "the output condition needs a printer (output) profile and \"{}\" is a {:?} profile; choose the printer's profile",
            dst.description, dst.class
        )));
    }
    let max = x.standard.max_icc_version();
    if !x.standard.takes_icc_version(dst.version.0) {
        return Err(bad(format!(
            "the output profile must be ICC version {} and \"{}\" is version {}; choose the printer's profile (for example Coated FOGRA39 or ISO Coated v2){}",
            if max == 2 { "2" } else { "2 or 4" },
            dst.description,
            dst.version.0,
            if max == 2 { ", or save PDF/X-4, which takes version 4 profiles" } else { "" }
        )));
    }
    let (pw, ph) = page_size(doc);
    let b = x.bleed_pt.max(0.0);
    // The boxes are written to 0.01 pt: the trim box has to stay positive as written, not only
    // before rounding.
    let q = |v: f64| (v * 100.0).round() / 100.0;
    if !b.is_finite() || q(pw - b) - q(b) < 0.005 || q(ph - b) - q(b) < 0.005 {
        return Err(bad(format!("a {b:.2} pt bleed leaves no trim on a {pw:.2} × {ph:.2} pt page")));
    }
    let (device, samples) = page_samples(doc)?;
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let mut warnings = vec![FLATTENED.to_string()];
    // A plain PDF gets a page per artboard; this writer has one page, the whole canvas.
    if !doc.artboards().is_empty() {
        warnings.push(format!("{name}: the document's artboards were saved together on one page"));
    }
    let space = |m: ColorMode| match m {
        ColorMode::Grayscale => ColorSpace::Gray,
        ColorMode::Cmyk => ColorSpace::Cmyk,
        _ => ColorSpace::Rgb,
    };
    let source = || -> std::result::Result<(Profile, std::sync::Arc<Vec<u8>>), IoError> {
        let (src, bytes) = match &x.source {
            Some(bytes) => (Profile::parse(bytes).map_err(|e| bad(format!("the document's profile can't be read ({e})")))?, bytes.clone()),
            None => {
                let p = if doc.mode == ColorMode::Grayscale { Builtin::SGray.profile().clone() } else { Builtin::Srgb.profile().clone() };
                let bytes = p.to_bytes();
                (p, bytes)
            }
        };
        // ICCBased takes input, display, output and colour space profiles; a device link or an
        // abstract profile describes no colour space of its own.
        use photocraft_cms::ProfileClass as C;
        if !matches!(src.class, C::Input | C::Display | C::Output | C::ColorSpace) {
            return Err(bad(format!(
                "the document's profile \"{}\" is a {:?} profile, which can't describe its colour; assign an RGB, gray or CMYK profile",
                src.description, src.class
            )));
        }
        if src.color_space != space(doc.mode) {
            return Err(bad(format!("\"{}\" is a {:?} profile, not the document's {:?}", src.description, src.color_space, doc.mode)));
        }
        Ok((src, bytes))
    };
    let keeps_numbers = matches!((doc.mode, &x.source), (ColorMode::Cmyk | ColorMode::Grayscale, None));
    let (channels, samples, icc) = if keeps_numbers {
        (device, samples, None)
    } else if x.convert || !x.standard.keeps_icc_colour() {
        let (src, _) = source()?;
        let t = Transform::new(&src, &dst, x.intent, x.bpc).map_err(|e| bad(format!("can't convert to \"{}\" ({e})", dst.description)))?;
        let mut out = vec![0.0f32; samples.len() / device * 4];
        t.convert_f32(&samples, device, &mut out, 4, false);
        // Paper stays paper: the transform lands white a fraction of a percent off zero ink,
        // which would print a tint over every blank area. Not for absolute colorimetric, whose
        // point is to simulate the source's paper white (the CMS leaves its white alone too).
        if doc.mode != ColorMode::Cmyk && x.intent != photocraft_cms::Intent::AbsoluteColorimetric {
            for (src, ink) in samples.chunks_exact(device).zip(out.as_chunks_mut::<4>().0) {
                if src.iter().all(|&v| v >= 1.0) {
                    ink.fill(0.0);
                }
            }
        }
        warnings.push(format!("{name}: colour converted from \"{}\" to \"{}\"", src.description, dst.description));
        (4, out, None)
    } else {
        let (src, bytes) = source()?;
        if !x.standard.takes_icc_version(src.version.0) {
            return Err(bad(format!(
                "the document's profile \"{}\" is ICC version {}, which this standard can't embed (PDF/X-1a and PDF/X-3 take version 2, PDF/X-4 versions 2 and 4); convert the colour to the output profile{}",
                src.description,
                src.version.0,
                if x.standard == PdfXStandard::X4 { "" } else { ", or save PDF/X-4" }
            )));
        }
        (device, samples, Some(bytes.to_vec()))
    };
    if !doc.metadata.text.is_empty() {
        warnings.push("free-form text metadata is not supported by PDF; it will be dropped".into());
    }
    let data: Vec<u8> = samples.into_iter().map(to8).collect();
    // Control characters become spaces, in the Info title and the XMP alike, so the two agree.
    let title: String = doc.name.rsplit_once('.').map_or(doc.name.as_str(), |(stem, _)| stem).chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let title = match title.trim() {
        "" => "Untitled",
        t => t,
    };
    let bleed = format!("{b:.4}");
    let profile_hash = file_id(&x.profile, &[], 0);
    let id = file_id(&data, &[title, x.standard.id(), &dst.description, &bleed, &profile_hash], secs);
    let image = PrintImage { width: doc.size.width, height: doc.size.height, channels, data, icc };
    let page = PrintPage { paper: (pw, ph), image, rect: (0.0, 0.0, pw, ph), marks: Marks::default(), description: None, label: None, page_label: None };
    let created = i64::try_from(secs).ok().and_then(|s| chrono::DateTime::from_timestamp(s, 0)).unwrap_or_default();
    let xp = XPage { standard: x.standard, profile: &x.profile, condition: &dst.description, title, trim: (b, b, pw - b, ph - b), created, id };
    Ok(ExportResult { bytes: write_pdf_x(&page, &xp), warnings })
}

#[cfg(test)]
mod tests;
