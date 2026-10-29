//! Printing to a network printer: the print the dialog describes, laid out once, as the print-ready
//! PDF (what a printer that lists PDF gets) and as a renderer of the same page at any resolution (what
//! the registry's converter asks for when the printer lists only PWG raster). The sending is not
//! here: the app shell owns the confirmation, the connection and the job (`photocraft-print`).
//!
//! The raster path draws what the PDF draws: the image in its box (mirrored for Emulsion Down), then
//! the corner/centre crop and registration marks. Page text (Description, Labels) and CMYK output are
//! PDF-only, and a printer that lists no PDF is refused them with the reason, rather than printing a
//! page that silently lacks what was asked for.

use std::sync::Arc;

use craft_print::pwg::RasterPage;
use craft_print::rgb::{Primitive, Shape};
use photocraft_doc::Document;
use serde_json::Value;

use super::{PAPERS, PrintLayout, PrintPage, make_page, page_rect_top_left, print_pdf, windows_marks};
use crate::{EngineError, Result};

/// Most output pixels one page may have (about A3 at 600 dpi is 70 M): a request beyond it is refused
/// with the advice to lower the resolution.
const MAX_PIXELS: u64 = 250_000_000;

/// A print laid out for a network printer.
#[derive(Clone)]
pub struct NetworkPrint {
    /// The print-ready one-page PDF.
    pub pdf: Arc<Vec<u8>>,
    /// The sheet in points (after orientation).
    pub paper: (f64, f64),
    /// The document's name, for the job title.
    pub title: String,
    /// Copies the dialog asked for.
    pub copies: u32,
    /// The PWG media name of the sheet, when it is one of the standard sizes.
    pub media: Option<&'static str>,
    /// The image's scale on the sheet, as a fraction (1.0 is 100 %).
    pub scale: f64,
    page: Arc<PrintPage>,
}

/// The PWG media name for a paper preset of the dialog (`PAPERS`), when printers list it.
pub fn media_for_paper(name: &str) -> Option<&'static str> {
    Some(match name.to_ascii_lowercase().as_str() {
        "letter" => "na_letter_8.5x11in",
        "legal" => "na_legal_8.5x14in",
        "tabloid" => "na_ledger_11x17in",
        "a3" => "iso_a3_297x420mm",
        "a4" => "iso_a4_210x297mm",
        "a5" => "iso_a5_148x210mm",
        "4x6" => "na_index-4x6_4x6in",
        "5x7" => "na_5x7_5x7in",
        _ => return None,
    })
}

/// The PWG media name for a sheet given in points (the Windows driver's paper is stored as `[w, h]`):
/// the dialog's preset within about a point, in either orientation. A size that is no preset has none.
pub fn media_for_size(w: f64, h: f64) -> Option<&'static str> {
    let near = |a: f64, b: f64| (a - b).abs() <= 1.0;
    PAPERS.iter().find(|(_, pw, ph)| (near(w, *pw) && near(h, *ph)) || (near(w, *ph) && near(h, *pw))).and_then(|(name, _, _)| media_for_paper(name))
}

/// Lay out `doc` for a network printer with the Print parameters `p` (the same ones `file.print`
/// takes: paper, orientation, position, scale, marks, colour handling). Sends nothing.
pub fn prepare_network_print(doc: &Document, p: &Value) -> Result<NetworkPrint> {
    let (page, layout, _) = make_page(doc, p, "file.print")?;
    let PrintLayout { paper, scale, .. } = layout;
    let copies = crate::commands::int(p, "copies").unwrap_or(1).clamp(1, 999) as u32;
    let media = match p.get("paper") {
        Some(Value::String(name)) if PAPERS.iter().any(|(n, _, _)| n.eq_ignore_ascii_case(name)) => media_for_paper(name),
        Some(Value::Array(size)) => match (size.first().and_then(Value::as_f64), size.get(1).and_then(Value::as_f64)) {
            (Some(w), Some(h)) => media_for_size(w, h),
            _ => None,
        },
        _ => None,
    };
    Ok(NetworkPrint { pdf: Arc::new(print_pdf(&page)), paper, title: doc.name.clone(), copies, media, scale, page: Arc::new(page) })
}

impl NetworkPrint {
    /// The page as 8-bit raster at `dpi`, in colour (RGB) or grey. Fails, with the reason in words, for
    /// content only a PDF carries (page text, CMYK output) and for a page too large at that resolution.
    pub fn raster_pages(&self, dpi: (u32, u32), colour: bool) -> std::result::Result<Vec<RasterPage>, String> {
        render_page(&self.page, dpi, colour).map(|p| vec![p]).map_err(|e| e.to_string())
    }
}

fn other(msg: impl Into<String>) -> EngineError {
    EngineError::Other(msg.into())
}

fn render_page(page: &PrintPage, (dx, dy): (u32, u32), colour: bool) -> Result<RasterPage> {
    let img = &page.image;
    if !matches!(img.channels, 1 | 3) {
        return Err(other(
            "this print is CMYK, which the printer cannot take as raster; choose a printer that accepts PDF, or the printer's own (RGB) profile",
        ));
    }
    if page.description.as_deref().is_some_and(|s| !s.is_empty()) || page.label.as_deref().is_some_and(|s| !s.is_empty()) {
        return Err(other(
            "Description and Labels are printed as page text, which this printer cannot take as raster; turn them off, or use a printer that accepts PDF",
        ));
    }
    let (dx, dy) = (dx.max(1), dy.max(1));
    let (pw, ph) = page.paper;
    let to_px = |points: f64, dpi: u32| (points * f64::from(dpi) / 72.0).ceil();
    let (wf, hf) = (to_px(pw, dx), to_px(ph, dy));
    if !(wf >= 1.0 && hf >= 1.0 && wf.is_finite() && hf.is_finite()) || wf * hf > MAX_PIXELS as f64 {
        return Err(other("the page is too large to render at this resolution: lower the resolution"));
    }
    let (w, h) = (wf as usize, hf as usize);
    let bpp = if colour { 3 } else { 1 };
    let mut out = vec![255u8; w * h * bpp];
    let (sw, sh) = (img.width as usize, img.height as usize);
    let row_len = sw.checked_mul(img.channels).ok_or_else(|| other("print image is too large"))?;
    if sw == 0 || sh == 0 || row_len.checked_mul(sh) != Some(img.data.len()) {
        return Err(other("nothing to print"));
    }
    // The image box in output pixels, origin top-left; Emulsion Down mirrors the box with the page.
    let (rx, ry, rw, rh) = page_rect_top_left(page);
    let (kx, ky) = (f64::from(dx) / 72.0, f64::from(dy) / 72.0);
    let (x0, y0, x1, y1) = (rx * kx, ry * ky, (rx + rw) * kx, (ry + rh) * ky);
    let (bw, bh) = (x1 - x0, y1 - y0);
    if bw > 0.0 && bh > 0.0 {
        // Source pixels per output pixel: sampled with a small box so a large photo does not alias.
        let (fx, fy) = (sw as f64 / bw, sh as f64 / bh);
        let taps = |f: f64| (f.ceil() as usize).clamp(1, 8);
        let (tx, ty) = (taps(fx), taps(fy));
        let first_x = x0.floor().max(0.0) as usize;
        let last_x = (x1.ceil().max(0.0) as usize).min(w);
        let first_y = y0.floor().max(0.0) as usize;
        let last_y = (y1.ceil().max(0.0) as usize).min(h);
        let src = |sx: usize, sy: usize, c: usize| -> u32 {
            let i = sy.min(sh - 1) * row_len + sx.min(sw - 1) * img.channels + if img.channels == 1 { 0 } else { c };
            u32::from(img.data.get(i).copied().unwrap_or(255))
        };
        for oy in first_y..last_y {
            let v = ((oy as f64 + 0.5) - y0) / bh;
            if !(0.0..1.0).contains(&v) {
                continue;
            }
            for ox in first_x..last_x {
                let u = ((ox as f64 + 0.5) - x0) / bw;
                if !(0.0..1.0).contains(&u) {
                    continue;
                }
                // Mirrored pages were mirrored in the box already; the picture itself mirrors too.
                let u = if page.emulsion_down { 1.0 - u } else { u };
                let (cx, cy) = (u * sw as f64, v * sh as f64);
                let mut sum = [0u32; 3];
                for j in 0..ty {
                    let sy = (cy + (j as f64 + 0.5) * fy / ty as f64 - fy / 2.0).max(0.0) as usize;
                    for i in 0..tx {
                        let sx = (cx + (i as f64 + 0.5) * fx / tx as f64 - fx / 2.0).max(0.0) as usize;
                        for (c, s) in sum.iter_mut().enumerate() {
                            *s += src(sx, sy, c);
                        }
                    }
                }
                let n = (tx * ty) as u32;
                let rgb = [(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8];
                put(&mut out, (w, ox, oy), rgb, colour);
            }
        }
    }
    for primitive in windows_marks::primitives(page.marks, (rx, ry, rw, rh)) {
        draw(&mut out, (w, h), (kx, ky), &primitive, colour);
    }
    Ok(RasterPage { width: w as u32, height: h as u32, gray: !colour, pixels: out })
}

/// Write one pixel: RGB, or its luma for a grey page.
fn put(out: &mut [u8], (w, x, y): (usize, usize, usize), rgb: [u8; 3], colour: bool) {
    if colour {
        let at = (y * w + x) * 3;
        if let Some(px) = out.get_mut(at..at + 3) {
            px.copy_from_slice(&rgb);
        }
    } else if let Some(px) = out.get_mut(y * w + x) {
        *px = super::luma709(rgb[0], rgb[1], rgb[2]);
    }
}

/// A mark primitive (points from the sheet's top-left) drawn as a line of at least one pixel.
fn draw(out: &mut [u8], (w, h): (usize, usize), (kx, ky): (f64, f64), p: &Primitive, colour: bool) {
    let radius = (p.width * kx.max(ky) / 2.0).max(0.5);
    let mut dot = |x: f64, y: f64| {
        let (lo_x, hi_x) = ((x - radius).floor().max(0.0) as usize, ((x + radius).ceil().max(0.0) as usize).min(w.saturating_sub(1)));
        let (lo_y, hi_y) = ((y - radius).floor().max(0.0) as usize, ((y + radius).ceil().max(0.0) as usize).min(h.saturating_sub(1)));
        for py in lo_y..=hi_y {
            for px in lo_x..=hi_x {
                let (ddx, ddy) = (px as f64 + 0.5 - x, py as f64 + 0.5 - y);
                if ddx * ddx + ddy * ddy <= radius * radius + 0.25 {
                    put(out, (w, px, py), p.color, colour);
                }
            }
        }
    };
    let mut line = |a: (f64, f64), b: (f64, f64)| {
        let (ax, ay, bx, by) = (a.0 * kx, a.1 * ky, b.0 * kx, b.1 * ky);
        let steps = ((bx - ax).abs().max((by - ay).abs()).ceil() as usize).clamp(1, 20_000);
        for i in 0..=steps {
            let t = i as f64 / steps as f64;
            dot(ax + (bx - ax) * t, ay + (by - ay) * t);
        }
    };
    match p.shape {
        Shape::Line { from, to } => line(from, to),
        Shape::Ellipse { bounds: (x, y, bw, bh) } => {
            let segments = 48;
            let at = |i: usize| {
                let a = std::f64::consts::TAU * i as f64 / f64::from(segments);
                (x + bw / 2.0 + bw / 2.0 * a.cos(), y + bh / 2.0 + bh / 2.0 * a.sin())
            };
            for i in 0..segments as usize {
                line(at(i), at(i + 1));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Session;
    use serde_json::json;

    fn doc() -> Document {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 60, "height": 40, "name": "net"})).unwrap();
        s.execute("edit.fill", json!({"color": [255, 0, 0]})).ok();
        s.active().unwrap().doc.as_ref().clone()
    }

    fn params(extra: Value) -> Value {
        let mut p = json!({"paper": "a4", "orientation": "portrait", "copies": 2, "scaleToFit": true, "center": true});
        if let (Some(o), Some(e)) = (p.as_object_mut(), extra.as_object()) {
            o.extend(e.clone());
        }
        p
    }

    #[test]
    fn the_print_is_laid_out_as_pdf_and_names_the_media() {
        let n = prepare_network_print(&doc(), &params(json!({}))).unwrap();
        assert!(n.pdf.starts_with(b"%PDF-"));
        assert_eq!((n.copies, n.media, n.title.as_str()), (2, Some("iso_a4_210x297mm"), "net"));
        assert!((n.paper.0 - 595.28).abs() < 0.01);
        assert!(prepare_network_print(&doc(), &params(json!({"paper": [300.0, 400.0]}))).unwrap().media.is_none());
        assert!(prepare_network_print(&doc(), &params(json!({"paper": "nope"}))).is_err());
    }

    /// The Windows dialog keeps the driver's paper as `[w, h]` (a landscape sheet as its swapped size):
    /// a standard size still names its media for the network printer.
    #[test]
    fn a_paper_given_as_a_size_names_the_media_of_its_preset() {
        let media = |paper: Value| prepare_network_print(&doc(), &params(json!({ "paper": paper }))).unwrap().media;
        assert_eq!(media(json!([595.28, 841.89])), Some("iso_a4_210x297mm"));
        assert_eq!(media(json!([841.89, 595.28])), Some("iso_a4_210x297mm"));
        assert_eq!(media(json!([595.0, 842.0])), Some("iso_a4_210x297mm"), "a driver's rounding is within a point");
        assert_eq!(media(json!([612, 792])), Some("na_letter_8.5x11in"));
        assert_eq!(media(json!([500.0, 700.0])), None);
    }

    #[test]
    fn the_raster_has_the_paper_size_at_the_resolution_and_the_image_inside_its_box() {
        let n = prepare_network_print(&doc(), &params(json!({}))).unwrap();
        let page = n.raster_pages((72, 72), true).unwrap().remove(0);
        assert_eq!((page.width, page.height), (596, 842));
        assert!(!page.gray && page.pixels.len() == 596 * 842 * 3);
        // The sheet is white at its corner and the (white, new) image fills the middle: no panic, right size.
        assert_eq!(&page.pixels[..3], &[255, 255, 255]);
        let grey = n.raster_pages((72, 72), false).unwrap().remove(0);
        assert!(grey.gray && grey.pixels.len() == 596 * 842);
        let wide = n.raster_pages((144, 72), true).unwrap().remove(0);
        assert_eq!((wide.width, wide.height), (1191, 842));
    }

    #[test]
    fn marks_and_the_image_are_drawn_and_text_or_huge_pages_are_refused() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 40, "height": 40, "name": "black", "background": "black"})).unwrap();
        let black = s.active().unwrap().doc.as_ref().clone();
        let n = prepare_network_print(&black, &params(json!({"scaleToFit": false, "scale": 100.0, "cornerCropMarks": true}))).unwrap();
        let page = n.raster_pages((72, 72), false).unwrap().remove(0);
        // The image is black and 40 pt wide on a 72 dpi sheet: dark pixels are the picture and the marks.
        let dark = page.pixels.iter().filter(|&&v| v < 128).count();
        assert!(dark > 0, "the image or the marks drew nothing");
        let texted = prepare_network_print(&black, &params(json!({"labels": true}))).unwrap();
        assert!(texted.raster_pages((72, 72), true).unwrap_err().contains("page text"));
        assert!(n.raster_pages((100_000, 100_000), true).unwrap_err().contains("too large"));
    }

    fn two_tone(emulsion_down: bool) -> PrintPage {
        // Black on the left, white on the right, filling an 72 pt square sheet.
        let image = super::super::PrintImage { width: 2, height: 1, channels: 3, data: vec![0, 0, 0, 255, 255, 255], icc: None };
        PrintPage {
            paper: (72.0, 72.0),
            image,
            rect: (0.0, 0.0, 72.0, 72.0),
            marks: super::super::Marks::default(),
            description: None,
            label: None,
            emulsion_down,
        }
    }

    #[test]
    fn emulsion_down_mirrors_the_picture() {
        let plain = render_page(&two_tone(false), (8, 8), false).unwrap();
        let mirrored = render_page(&two_tone(true), (8, 8), false).unwrap();
        assert_eq!((plain.width, plain.height), (8, 8));
        let row = |p: &RasterPage| p.pixels[8 * 4..8 * 5].to_vec();
        assert_eq!(row(&plain), [0, 0, 0, 0, 255, 255, 255, 255]);
        assert_eq!(row(&mirrored), [255, 255, 255, 255, 0, 0, 0, 0]);
    }

    #[test]
    fn a_grey_raster_pixel_is_rec_709_like_the_pdf_and_the_preview() {
        let mut out = [0u8; 1];
        put(&mut out, (1, 0, 0), [255, 0, 0], false);
        assert_eq!(out[0], super::super::luma709(255, 0, 0), "red is 54, not Rec. 601's 76");
        put(&mut out, (1, 0, 0), [255, 255, 255], false);
        assert_eq!(out[0], 255);
    }
}
