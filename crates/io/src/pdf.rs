//! PDF export model mapping for PhotoCraft.
//!
//! This module projects document rasters, vectors, spot channels and print policy into the
//! neutral page records in craft-pdf. PDF object layout and serialization live in craft-pdf.

use photocraft_color::{ColorMode, PixelFormat};
use photocraft_doc::Document;

use crate::{ExportResult, IoError};

mod shapes;
mod text;

/// Neutral page records and PDF syntax are owned by craft-pdf; this module only maps the
/// PhotoCraft document to those records and supplies its product-level cancellation policy.
pub use craft_pdf::write_raster::{ContentOp, Image as PrintImage, Marks, Page as PrintPage, SelectionFont, SpotPlate, VectorContent};

/// A one-page PDF.
pub fn print_pdf(page: &PrintPage) -> Vec<u8> {
    craft_pdf::write_raster::write_pages_with_vector_content(std::slice::from_ref(page), &[VectorContent::default()], &SelectionFont::default(), None)
        .unwrap_or_default()
}

fn print_pdf_pages(
    pages: &[PrintPage],
    vectors: &[VectorContent],
    selection_font: &SelectionFont,
    xmp: Option<&str>,
    ctl: &photocraft_raster::Interrupt,
) -> std::result::Result<Vec<u8>, IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    craft_pdf::write_raster::write_pages_with_vector_content(pages, vectors, selection_font, xmp)
        .ok_or_else(|| IoError::Unsupported("PDF vector content contains an invalid number".into()))
}

/// The namespace of the spot-plate description written by the shared PDF metadata encoder.
pub const SPOT_PLATES_NS: &str = craft_pdf::write_raster::SPOT_PLATES_NAMESPACE;

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
/// image or its tint transform, and only the shared XMP description writer carries it.
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
fn collect_vectors(doc: &Document) -> (Vec<(usize, Vec<ContentOp>)>, SelectionFont) {
    enum Candidate<'a> {
        Ops(Vec<ContentOp>),
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
                photocraft_doc::LayerContent::Shape(shape) => match shapes::shape_content(shape, doc.mode) {
                    Some(path) => Candidate::Ops(vec![ContentOp::Path(path)]),
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
    let selection_font = SelectionFont::new(candidates.iter().filter_map(|(_, candidate)| match candidate {
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
/// embeds no metadata). With spot plates the packet gains their description through the shared
/// PDF writer;
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
        PrintImage { width: rect.width(), height: rect.height(), channels: device, data, icc: icc.clone() }
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
        let vectors = vec![VectorContent::default(); pages.len()];
        (pages, vectors, SelectionFont::default())
    } else {
        let canvas = photocraft_geom::Rect::new(0, 0, w as i32, h as i32);
        // Simple topmost shapes and text layers can be drawn over the flattened backdrop without
        // changing compositing. Stop at the first unsupported visible layer, since it may depend
        // on the backdrop (adjustments, blend modes, masks, effects, or clipping). A Multichannel
        // document has no composite for them to sit on: it is its plates alone.
        let (vector_layers, selection_font) = if device.is_some() { collect_vectors(doc) } else { (Vec::new(), SelectionFont::default()) };
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
        let mut operations = Vec::new();
        if !vector_layers.is_empty() {
            let scale = 72.0 / dpi;
            let ph = f64::from(doc.size.height) * scale;
            operations.push(ContentOp::Save);
            operations.push(ContentOp::Concat([scale, 0.0, 0.0, -scale, 0.0, ph]));
            for (_, ops) in vector_layers.iter().rev() {
                operations.extend(ops.iter().cloned());
            }
            operations.push(ContentOp::Restore);
        }
        (vec![page_of(image, canvas, None)], vec![VectorContent { operations }], selection_font)
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
    let xmp = craft_pdf::write_raster::xmp_with_spot_plates(doc_xmp, plates);
    if !doc.metadata.text.is_empty() {
        warnings.push("free-form text metadata is not supported by PDF; it will be dropped".into());
    }
    let mut bytes = print_pdf_pages(&pages, &vectors, &selection_font, xmp.as_deref(), ctl)?;
    // The notes go on the document's one page (#11796). Artboard pages were not measured, so they
    // keep writing none, and say so.
    if !doc.notes.is_empty() {
        if artboards.is_empty() {
            let page_height = f64::from(doc.size.height) * 72.0 / dpi;
            bytes = with_notes(bytes, &doc.notes, 72.0 / dpi, page_height)?;
        } else {
            warnings.push("notes were not written: a PDF of artboards keeps no notes".into());
        }
    }
    Ok(ExportResult { bytes, warnings })
}

/// The side of a note's icon annotation, in PDF points.
const NOTE_ICON_PT: f64 = 20.0;

/// `pdf` (the shared writer's output: one classic cross-reference table, plain page dictionaries)
/// with `notes` added to its first page as PDF text annotations (ISO 32000 `/Subtype /Text`),
/// each with its `/Popup`. They go in an incremental update (§7.5.6): the new annotation objects,
/// the page again with `/Annots`, and a cross-reference section chained to the original by
/// `/Prev`, so the bytes the writer produced stay untouched. `scale` is points per document pixel
/// and `page_height` the page height in points (PDF's y axis points up). A note with a position
/// that isn't a finite number is left out.
fn with_notes(mut pdf: Vec<u8>, notes: &[photocraft_doc::Note], scale: f64, page_height: f64) -> Result<Vec<u8>, IoError> {
    let broken = || IoError::Unsupported("the PDF writer's output has no page to put the notes on".into());
    let (prev, size, root) = trailer_of(&pdf).ok_or_else(broken)?;
    let (page, page_dict) = first_page(&pdf).ok_or_else(broken)?;
    let num = |v: f64| {
        let t = format!("{v:.3}");
        let t = t.trim_end_matches('0').trim_end_matches('.');
        if t == "-0" { "0".to_string() } else { t.to_string() }
    };
    let (x, y) = (|px: f64| px * scale, |py: f64| page_height - py * scale);
    let mut objects: Vec<(u32, String)> = Vec::new();
    let mut refs = Vec::new();
    let mut next = size;
    for n in notes {
        let (left, top) = (x(n.position[0]), y(n.position[1]));
        if !left.is_finite() || !top.is_finite() {
            continue;
        }
        let (annot, popup) = (next, next + 1);
        next += 2;
        let [r, g, b] = n.color.to_rgb().map(|c| f64::from(c.clamp(0.0, 1.0)));
        let mut d = format!(
            "<< /Type /Annot /Subtype /Text /Rect [{} {} {} {}] /Contents {} /T {} /C [{} {} {}] /Name /Comment /F 28 /Open {} /Popup {popup} 0 R",
            num(left),
            num(top - NOTE_ICON_PT),
            num(left + NOTE_ICON_PT),
            num(top),
            text_string(&n.text),
            text_string(&n.author),
            num(r),
            num(g),
            num(b),
            n.open
        );
        // The stored date is already PDF date text; anything else is left out rather than escaped.
        if n.modified.starts_with("D:") && n.modified.bytes().all(|c| c.is_ascii_graphic() && !matches!(c, b'(' | b')' | b'\\')) {
            d.push_str(&format!(" /M ({})", n.modified));
        }
        d.push_str(" >>");
        let [l, t, rr, bb] = n.popup;
        let rect = if [l, t, rr, bb].iter().all(|v| v.is_finite()) && rr > l && bb > t {
            [x(l), y(bb), x(rr), y(t)]
        } else {
            [left + NOTE_ICON_PT, top - 140.0, left + NOTE_ICON_PT + 240.0, top]
        };
        let p = format!(
            "<< /Type /Annot /Subtype /Popup /Parent {annot} 0 R /Rect [{} {} {} {}] /F 28 /Open {} >>",
            num(rect[0]),
            num(rect[1]),
            num(rect[2]),
            num(rect[3]),
            n.open
        );
        objects.push((annot, d));
        objects.push((popup, p));
        refs.push(format!("{annot} 0 R {popup} 0 R"));
    }
    if refs.is_empty() {
        return Ok(pdf);
    }
    let body = page_dict.trim_end();
    let Some(open) = body.strip_suffix(">>") else { return Err(broken()) };
    let page_text = format!("{open}/Annots [{}] >>", refs.join(" "));
    if !pdf.ends_with(b"\n") {
        pdf.push(b'\n');
    }
    let mut offsets = Vec::with_capacity(objects.len() + 1);
    offsets.push((page, pdf.len()));
    pdf.extend_from_slice(format!("{page} 0 obj\n{page_text}\nendobj\n").as_bytes());
    for (n, text) in &objects {
        offsets.push((*n, pdf.len()));
        pdf.extend_from_slice(format!("{n} 0 obj\n{text}\nendobj\n").as_bytes());
    }
    let xref = pdf.len();
    pdf.extend_from_slice(format!("xref\n{page} 1\n{:010} 00000 n \n{size} {}\n", offsets[0].1, objects.len()).as_bytes());
    for (_, at) in &offsets[1..] {
        pdf.extend_from_slice(format!("{at:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(format!("trailer\n<< /Size {next} /Root {root} /Prev {prev} >>\nstartxref\n{xref}\n%%EOF\n").as_bytes());
    Ok(pdf)
}

/// A PDF text string: UTF-16BE with its byte-order mark, as a hex string, so any text needs no
/// escaping.
fn text_string(text: &str) -> String {
    let mut out = String::from("<FEFF");
    for unit in text.encode_utf16() {
        out.push_str(&format!("{unit:04X}"));
    }
    out.push('>');
    out
}

/// The last cross-reference section's offset, the trailer's `/Size` and its `/Root` reference
/// (as `N G R`), from the end of the shared writer's output.
fn trailer_of(pdf: &[u8]) -> Option<(usize, u32, String)> {
    let text = std::str::from_utf8(&pdf[pdf.len().saturating_sub(512)..]).ok()?;
    let trailer = &text[text.rfind("trailer")?..];
    let after = |key: &str| trailer.find(key).map(|i| trailer[i + key.len()..].trim_start());
    let size = after("/Size")?.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()?;
    let root: Vec<&str> = after("/Root")?.split_whitespace().take(3).collect();
    let prev = after("startxref")?.lines().next()?.trim().parse().ok()?;
    (root.len() == 3 && root[2].starts_with('R')).then(|| (prev, size, format!("{} {} R", root[0], root[1])))
}

/// The first page object's number and its dictionary text, as the shared writer prints it.
fn first_page(pdf: &[u8]) -> Option<(u32, String)> {
    let needle = b" 0 obj\n<< /Type /Page /Parent";
    let at = pdf.windows(needle.len()).position(|w| w == needle)?;
    let start = pdf[..at].iter().rposition(|c| *c == b'\n').map_or(0, |i| i + 1);
    let number = std::str::from_utf8(&pdf[start..at]).ok()?.trim().parse().ok()?;
    let dict = at + b" 0 obj\n".len();
    let end = dict + pdf[dict..].windows(b"\nendobj".len()).position(|w| w == b"\nendobj")?;
    Some((number, std::str::from_utf8(&pdf[dict..end]).ok()?.to_string()))
}

/// The text annotations on a PDF's first page, read back as notes (#11796), and the page's
/// height. Positions are in points from the page's top-left (its crop box), y down, so
/// [`Note::position`](photocraft_doc::Note) scales onto a canvas by canvas height / page height.
/// Text, author, colour, open state and date come from `/Contents`, `/T`, `/C`, `/Open` and `/M`;
/// the popup rectangle from the annotation's `/Popup`. Other annotation types are skipped. A PDF
/// that can't be read is an error; one without text annotations gives no notes.
pub fn read_notes(bytes: &[u8]) -> Result<(Vec<photocraft_doc::Note>, f64), IoError> {
    use craft_pdf::syntax::object::{Array, Dict, Name, Rect, String as PdfString};
    let pdf = craft_pdf::open_slice_with_options(bytes, None, &craft_pdf::OpenOptions::default())
        .map_err(|e| IoError::Unsupported(format!("the PDF could not be read: {e:?}")))?;
    let pages = pdf.pages();
    let Some(page) = pages.first() else {
        return Err(IoError::Unsupported("the PDF has no pages".into()));
    };
    let bounds = page.intersected_crop_box();
    let (left, top) = (bounds.x0.min(bounds.x1), bounds.y0.max(bounds.y1));
    let height = (bounds.y1 - bounds.y0).abs();
    let to_note_space = |r: &Rect| [r.x0.min(r.x1) - left, top - r.y0.max(r.y1), r.x0.max(r.x1) - left, top - r.y0.min(r.y1)];
    let mut notes = Vec::new();
    let Some(annots) = page.raw().get::<Array>(b"Annots") else {
        return Ok((notes, height));
    };
    for a in annots.iter::<Dict>() {
        if a.get::<Name>(b"Subtype").is_none_or(|s| s.as_ref() != b"Text") {
            continue;
        }
        let Some(rect) = a.get::<Rect>(b"Rect") else { continue };
        let [x0, y0, ..] = to_note_space(&rect);
        if !x0.is_finite() || !y0.is_finite() {
            continue;
        }
        let text = |key: &[u8]| a.get::<PdfString>(key).map(|s| decode_text_string(s.as_bytes())).unwrap_or_default();
        let color = a.get::<Vec<f64>>(b"C").filter(|c| c.len() == 3 && c.iter().all(|v| v.is_finite())).map_or_else(
            || photocraft_doc::Note::default().color,
            |c| photocraft_color::Color::rgb(c[0].clamp(0.0, 1.0) as f32, c[1].clamp(0.0, 1.0) as f32, c[2].clamp(0.0, 1.0) as f32),
        );
        let popup = a.get::<Dict>(b"Popup").and_then(|p| p.get::<Rect>(b"Rect")).map(|r| to_note_space(&r)).filter(|r| r.iter().all(|v| v.is_finite()));
        notes.push(photocraft_doc::Note {
            author: text(b"T"),
            color,
            text: text(b"Contents"),
            position: [x0, y0],
            popup: popup.unwrap_or([x0, y0, x0, y0]),
            open: a.get::<bool>(b"Open").unwrap_or(false),
            modified: text(b"M"),
        });
    }
    Ok((notes, height))
}

/// PDFDocEncoding's code points that differ from Latin-1 (ISO 32000-1 Annex D): 0x18–0x1F, 0x80–0x9E
/// and 0xA0. The other codes are the same as Latin-1's.
fn pdf_doc_char(c: u8) -> char {
    const LOW: [char; 8] = ['\u{2D8}', '\u{2C7}', '\u{2C6}', '\u{2D9}', '\u{2DD}', '\u{2DB}', '\u{2DA}', '\u{2DC}'];
    const HIGH: [char; 31] = [
        '\u{2022}', '\u{2020}', '\u{2021}', '\u{2026}', '\u{2014}', '\u{2013}', '\u{192}', '\u{2044}', '\u{2039}', '\u{203A}', '\u{2212}', '\u{2030}',
        '\u{201E}', '\u{201C}', '\u{201D}', '\u{2018}', '\u{2019}', '\u{201A}', '\u{2122}', '\u{FB01}', '\u{FB02}', '\u{141}', '\u{152}', '\u{160}', '\u{178}',
        '\u{17D}', '\u{131}', '\u{142}', '\u{153}', '\u{161}', '\u{17E}',
    ];
    match c {
        0x18..=0x1F => LOW[usize::from(c - 0x18)],
        0x80..=0x9E => HIGH[usize::from(c - 0x80)],
        0xA0 => '\u{20AC}',
        _ => char::from(c),
    }
}

/// A PDF text string's bytes as text: UTF-16BE after its byte-order mark, UTF-8 after its mark,
/// otherwise PDFDocEncoding.
fn decode_text_string(b: &[u8]) -> String {
    if let Some(rest) = b.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> = rest.as_chunks::<2>().0.iter().map(|p| u16::from_be_bytes(*p)).collect();
        return String::from_utf16_lossy(&units);
    }
    if let Some(rest) = b.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    b.iter().map(|&c| pdf_doc_char(c)).collect()
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod notes_tests;
