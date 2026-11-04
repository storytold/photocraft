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

fn write_pdf(page: &PrintPage, x: Option<&XPage>) -> Vec<u8> {
    let version = x.map_or("1.4", |x| x.standard.pdf_version());
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
    // 1 catalog, 2 pages, 3 page, 4 contents, 5 image, then for a plain PDF 6 font and 7 the
    // image's ICC profile (optional). PDF/X has no text (an unembedded font isn't allowed): 6 is
    // the output profile, 7 the Info dictionary, then the image's ICC profile (optional) and the
    // XMP metadata (PDF/X-4).
    let icc_obj = if x.is_some() { 8 } else { 7 };
    let xmp_obj = if icc.is_some() { 9 } else { 8 };
    let producer = format!("PhotoCraft {}", env!("CARGO_PKG_VERSION"));
    match x {
        Some(x) => {
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
        }
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
    match x {
        Some(x) => obj(&mut out, &stream("/N 4 /Filter /FlateDecode", &deflate(x.profile))),
        None => obj(&mut out, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"),
    }
    if let Some(x) = x {
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
    }
    if let Some(icc) = icc {
        obj(&mut out, &stream(&format!("/N {} /Alternate {device} /Filter /FlateDecode", img.channels), &deflate(icc)));
    }
    if let Some(x) = x.filter(|x| x.standard == PdfXStandard::X4) {
        // Unfiltered, so preflight tools and asset managers read it without decoding.
        obj(&mut out, &stream("/Type /Metadata /Subtype /XML", xmp_packet(x, &producer).as_bytes()));
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
pub fn export_with_options(doc: &Document, opts: &PdfOptions) -> std::result::Result<ExportResult, IoError> {
    let Some(x) = &opts.x else { return export(doc) };
    // `web_time` reads the browser's clock on wasm, where `std`'s has none.
    let secs = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    export_x(doc, x, secs)
}

fn export_x(doc: &Document, x: &PdfX, secs: u64) -> std::result::Result<ExportResult, IoError> {
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
    if !b.is_finite() || 2.0 * b >= pw.min(ph) {
        return Err(bad(format!("a {b:.2} pt bleed leaves no trim on a {pw:.2} × {ph:.2} pt page")));
    }
    let (device, samples) = page_samples(doc)?;
    let mut warnings = vec![FLATTENED.to_string()];
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
        // which would print a tint over every blank area.
        if doc.mode != ColorMode::Cmyk {
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
    let page = PrintPage { paper: (pw, ph), image, rect: (0.0, 0.0, pw, ph), marks: Marks::default(), description: None, label: None };
    let created = i64::try_from(secs).ok().and_then(|s| chrono::DateTime::from_timestamp(s, 0)).unwrap_or_default();
    let xp = XPage { standard: x.standard, profile: &x.profile, condition: &dst.description, title, trim: (b, b, pw - b, ph - b), created, id };
    Ok(ExportResult { bytes: write_pdf(&page, Some(&xp)), warnings })
}

#[cfg(test)]
mod tests;
