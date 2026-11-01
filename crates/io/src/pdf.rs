//! The PDF writer (ISO 32000-1 basics): one raster page per file.
//!
//! A small, self-contained writer used by two paths: File › Save a Copy / Save As › PDF (#3401)
//! through [`export`], and File › Print through [`print_pdf`] (called by the engine, which adds
//! paper geometry, printer marks and labels around the image). One page holds a
//! Flate-compressed image XObject, optional printer marks as vector operators and optional
//! Helvetica text; an embedded ICC profile rides in an ICCBased colour space.
//!
//! Spot channels leave as plates (FILE-215-17): one 8-bit image XObject per channel in a
//! `/Separation` colour space named after the channel, painted over the composite with overprint
//! on, so a RIP or a separating rasteriser (Ghostscript `tiffsep`, `psdcmyk`) keeps each one as
//! its own named plate instead of mixing it into CMYK. The plates carry no mask: a viewer that
//! ignores overprint shows the last plate opaque, Acrobat's Overprint Preview and any separating
//! RIP show the real result, and an explicit image mask makes Illustrator report "An unknown
//! imaging construct was encountered". Multi-page PDF, layer-preserving PDF and PDF
//! import stay open (FILE-215-10).

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
    pub plates: Vec<SpotPlate>,
    /// The XMP packet for the catalog's `/Metadata` stream, if any.
    pub xmp: Option<String>,
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

/// An image XObject dictionary and its Flate-compressed samples, as one object body.
fn image_object(width: u32, height: u32, colour_space: &str, data: &[u8]) -> Vec<u8> {
    let z = deflate(data);
    let mut body = format!(
        "<< /Type /XObject /Subtype /Image /Width {width} /Height {height} /ColorSpace {colour_space} /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream\n",
        z.len()
    )
    .into_bytes();
    body.extend_from_slice(&z);
    body.extend_from_slice(b"\nendstream");
    body
}

/// A one-page PDF.
///
/// Objects: 1 catalog, 2 pages, 3 page, 4 contents, 5 font, then the image, its ICC profile,
/// the overprint ExtGState, one image XObject per plate and the XMP metadata stream, each only
/// when present.
pub fn print_pdf(page: &PrintPage) -> Vec<u8> {
    let mut out: Vec<u8> = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets: Vec<usize> = Vec::new();
    let mut obj = |out: &mut Vec<u8>, body: &[u8]| {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", offsets.len()).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };
    let (pw, ph) = page.paper;
    let (x, y, w, h) = page.rect;
    // Object numbers of the optional objects, in the order they are written.
    let mut next = 6usize;
    let mut take = |present: bool| {
        let n = present.then_some(next);
        next += usize::from(present);
        n
    };
    let image_obj = take(page.image.is_some());
    let icc_obj = take(page.image.as_ref().is_some_and(|i| i.icc.is_some()));
    let gs_obj = take(!page.plates.is_empty());
    let plate_objs: Vec<usize> = page.plates.iter().filter_map(|_| take(true)).collect();
    let xmp_obj = take(page.xmp.is_some());

    let metadata = xmp_obj.map(|n| format!(" /Metadata {n} 0 R")).unwrap_or_default();
    obj(&mut out, format!("<< /Type /Catalog /Pages 2 0 R{metadata} >>").as_bytes());
    obj(&mut out, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    let mut xobjects = String::new();
    if let Some(n) = image_obj {
        xobjects.push_str(&format!("/Im0 {n} 0 R "));
    }
    for (i, n) in plate_objs.iter().enumerate() {
        xobjects.push_str(&format!("/Sp{i} {n} 0 R "));
    }
    let ext_gstate = gs_obj.map(|n| format!("/ExtGState << /GS0 {n} 0 R >> ")).unwrap_or_default();
    obj(
        &mut out,
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {pw:.2} {ph:.2}] /Resources << /XObject << {xobjects}>> {ext_gstate}/Font << /F1 5 0 R >> >> /Contents 4 0 R >>"
        )
        .as_bytes(),
    );
    // An explicit white page, so every viewer (and rasteriser) shows paper, not transparency.
    let mut content = format!("q 1 g 0 0 {pw:.2} {ph:.2} re f Q\n");
    if image_obj.is_some() {
        content.push_str(&format!("q {w:.3} 0 0 {h:.3} {x:.3} {y:.3} cm /Im0 Do Q\n"));
    }
    // Each plate overprints what is already on the page: with `/OP true` a Separation paints its
    // own colorant only, leaving the others (and the composite) as they are.
    for i in 0..page.plates.len() {
        content.push_str(&format!("q /GS0 gs {w:.3} 0 0 {h:.3} {x:.3} {y:.3} cm /Sp{i} Do Q\n"));
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
    obj(&mut out, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>");
    if let Some(img) = &page.image {
        let device = match img.channels {
            1 => "/DeviceGray",
            4 => "/DeviceCMYK",
            _ => "/DeviceRGB",
        };
        let cs = match icc_obj {
            Some(n) => format!("[/ICCBased {n} 0 R]"),
            None => device.to_string(),
        };
        obj(&mut out, &image_object(img.width, img.height, &cs, &img.data));
        if let Some(icc) = &img.icc {
            let z = deflate(icc);
            let mut body = format!("<< /N {} /Alternate {device} /Filter /FlateDecode /Length {} >>\nstream\n", img.channels, z.len()).into_bytes();
            body.extend_from_slice(&z);
            body.extend_from_slice(b"\nendstream");
            obj(&mut out, &body);
        }
    }
    if gs_obj.is_some() {
        obj(&mut out, b"<< /Type /ExtGState /OP true /op true /OPM 1 >>");
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
        obj(&mut out, &image_object(p.width, p.height, &cs, &p.data));
    }
    if let Some(xmp) = &page.xmp {
        // Unfiltered, as the XMP specification asks of a packet in a PDF, so a reader can find it
        // by scanning for the packet header.
        let mut body = format!("<< /Type /Metadata /Subtype /XML /Length {} >>\nstream\n", xmp.len()).into_bytes();
        body.extend_from_slice(xmp.as_bytes());
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
    let body = &xmp[bom..];
    let doc = crate::xml::parse(body, roxmltree::ParsingOptions::default()).ok()?;
    let rdf = doc.descendants().find(|n| n.is_element() && n.tag_name().namespace() == Some(RDF_NS) && n.tag_name().name() == "RDF")?;
    let range = rdf.range();
    let text = body.get(range.clone())?;
    if let Some(open) = text.strip_suffix("/>") {
        // `<r:RDF …/>`: the qualified name follows the `<`, up to whitespace or the `/`.
        let name = open.get(1..)?;
        let end = name.find(|c: char| c.is_whitespace() || c == '/').unwrap_or(name.len());
        return Some((bom + range.end - 2, Some(&name[..end])));
    }
    // The end tag is the last thing in the element's own range.
    Some((bom + range.start + text.rfind("</")?, None))
}

/// The XMP namespace of the spot-plate description.
pub const SPOT_PLATES_NS: &str = "http://ns.getartcraft.com/photocraft/spot-plates/1.0/";

/// The XMP packet of a spot-plate PDF: the document's own packet (`doc_xmp`, if any) with one
/// `rdf:Description` added under [`SPOT_PLATES_NS`] that lists the plates in page order, so a
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
/// An alpha-type channel of a Multichannel document lists as black with solidity 0.
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
    // go and is kept as it is rather than replaced.
    if let Some(xmp) = doc_xmp {
        let Some((at, self_closing)) = rdf_insertion(xmp) else {
            return xmp.to_owned();
        };
        let mut out = String::with_capacity(xmp.len() + d.len() + 16);
        out.push_str(&xmp[..at]);
        match self_closing {
            // `<r:RDF …/>` becomes `<r:RDF …>` + the description + `</r:RDF>`.
            Some(name) => {
                out.push_str(">\n");
                out.push_str(&d);
                out.push_str(" </");
                out.push_str(name);
                out.push('>');
                out.push_str(&xmp[at + 2..]);
            }
            None => {
                out.push_str(&d);
                out.push_str(&xmp[at..]);
            }
        }
        return out;
    }
    format!(
        "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n <rdf:RDF xmlns:rdf=\"{RDF_NS}\">\n{d} </rdf:RDF>\n</x:xmpmeta>\n<?xpacket end=\"w\"?>"
    )
}

/// The spot plates of `doc`, in channel order (the first channel prints first, underneath).
///
/// In a Multichannel document every channel is a plate: its spot channels at the ink density
/// they store (1 = solid), an alpha-type channel as the transparent black ink the composite shows
/// it as (1 − value, as `photocraft_compose::multichannel` prints it). Any other document keeps
/// its alpha channels as masks and exports only its spot channels. The alternate colour is the
/// ink colour separated through the CMYK profile in use (`Color::in_mode`); it only previews the
/// plate, as Photoshop's does. Solidity is a preview attribute too: it is not part of the plate
/// image or its tint transform, and only the XMP description ([`plates_xmp`]) carries it.
fn plates_of(doc: &Document, w: usize, h: usize) -> Vec<SpotPlate> {
    let multichannel = doc.mode == ColorMode::Multichannel;
    let canvas = photocraft_geom::Rect::new(0, 0, w as i32, h as i32);
    let to8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    doc.channels
        .iter()
        .filter(|ch| multichannel || ch.spot.is_some())
        .map(|ch| {
            let (ink, alpha_type) = match ch.spot {
                Some((ink, _)) => (ink, false),
                None => (photocraft_color::Color::BLACK, true),
            };
            let n = ch.surface.channels().max(1);
            let values = ch.surface.read_region(canvas);
            let data: Vec<u8> = values.chunks(n).map(|px| px.first().copied().unwrap_or(0.0)).map(|v| to8(if alpha_type { 1.0 - v } else { v })).collect();
            let srgb = ink.to_rgb();
            SpotPlate {
                name: ch.name.clone(),
                cmyk: ink.in_mode(ColorMode::Cmyk).c,
                lab: photocraft_color::convert::srgb_to_lab(srgb),
                srgb,
                solidity: ch.spot.map_or(0.0, |(_, s)| s.clamp(0.0, 1.0)),
                width: w as u32,
                height: h as u32,
                data,
            }
        })
        .collect()
}

/// File › Save As / Save a Copy › PDF (#3401): the flattened document, at its print size
/// (pixels ÷ resolution × 72 pt), as a one-page PDF. Layers and transparency are dropped — the
/// warning says so — and the embedded profile travels with the image.
///
/// With `spot_plates`, the document's spot channels follow the image as overprinting
/// `/Separation` plates (FILE-215-17), and a Multichannel document, which is nothing but its
/// channels, is written as plates alone. Without it a Multichannel document has nothing to
/// write and is refused.
pub fn export(doc: &Document, spot_plates: bool) -> std::result::Result<ExportResult, IoError> {
    export_with(doc, spot_plates, doc.metadata.xmp.as_deref())
}

/// [`export`] with the XMP packet to embed (`doc_xmp`: the document's, or `None` when the save
/// embeds no metadata). With spot plates the packet gains their description ([`plates_xmp`]);
/// without plates it is written as it is, and nothing when there is none.
pub fn export_with(doc: &Document, spot_plates: bool, doc_xmp: Option<&str>) -> std::result::Result<ExportResult, IoError> {
    let (w, h) = (usize::try_from(doc.size.width).unwrap_or(0), usize::try_from(doc.size.height).unwrap_or(0));
    if w == 0 || h == 0 {
        return Err(IoError::Unsupported("PDF needs a document with non-zero width and height; nothing was written".into()));
    }
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
    let dpi = f64::from(doc.resolution_dpi.max(1.0));
    let (pw, ph) = (f64::from(doc.size.width) * 72.0 / dpi, f64::from(doc.size.height) * 72.0 / dpi);
    let mut warnings = Vec::new();
    let image = device.map(|device| {
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
        warnings.push("PDF was saved flattened: layers, masks-as-editable and transparency are not preserved (#3401)".into());
        PrintImage { width: doc.size.width, height: doc.size.height, channels: device, data, icc: doc.icc_profile.as_ref().map(|p| p.to_vec()) }
    });
    let plates = if spot_plates { plates_of(doc, w, h) } else { Vec::new() };
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
    let xmp = if plates.is_empty() { doc_xmp.map(str::to_owned) } else { Some(plates_xmp(doc_xmp, &plates)) };
    let page = PrintPage { paper: (pw, ph), image, rect: (0.0, 0.0, pw, ph), marks: Marks::default(), description: None, label: None, plates, xmp };
    if !doc.metadata.text.is_empty() {
        warnings.push("free-form text metadata is not supported by PDF; it will be dropped".into());
    }
    Ok(ExportResult { bytes: print_pdf(&page), warnings })
}

#[cfg(test)]
mod tests;
