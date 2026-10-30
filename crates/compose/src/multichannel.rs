//! Spot inks printed over a picture: every spot (ink) channel of a Multichannel document over
//! white paper, and the visible spot channels of an RGB or CMYK document over its composite
//! (the canvas preview and the PSD thumbnail; never `flatten`, export or the merged image).
//!
//! Ink density `v` (the channel value, 1 = solid) of an ink with display colour `c` and solidity
//! `s` turns the colour underneath `p` into `(1 − s·v)·p·(1 − v + v·c) + s·v·c`: transparent inks
//! (solidity 0) multiply like overprinted process inks, opaque ones cover what is below, which is
//! how the reference editor previews spot channels. Inks print in Channels panel order. An
//! alpha-type channel of a Multichannel document (selected / masked areas) still prints, as a
//! transparent black ink, like the reference editor shows it. Its value is stored as-is
//! (1 = selected, white in the Channels panel), so its ink is `1 − value`.

use photocraft_cms::{Builtin, ColorSpace, Profile, TransformOptions};
use photocraft_color::{Color, ColorMode};
use photocraft_doc::{AlphaChannel, Document};
use photocraft_geom::Rect;

use crate::Buffer;

/// One channel printed as an ink: its display colour in the composite's colour space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ink {
    pub rgb: [f32; 3],
    pub solidity: f32,
    /// An alpha-type channel: black ink where the channel is unselected (`1 − value`).
    pub alpha: bool,
}

impl Ink {
    /// The ink of channel `ch` of `doc`: a spot channel's colour and solidity, an alpha channel's
    /// transparent black.
    pub fn of(doc: &Document, ch: &AlphaChannel) -> Ink {
        match ch.spot {
            Some((color, solidity)) => Ink { rgb: ink_rgb(doc, color), solidity: solidity.clamp(0.0, 1.0), alpha: false },
            None => Ink { rgb: [0.0; 3], solidity: 0.0, alpha: true },
        }
    }

    /// Print this ink with channel values `values` (one per pixel) over the opaque `px`.
    pub fn apply(&self, px: &mut [[f32; 4]], values: impl IntoIterator<Item = f32>) {
        let (c, s) = (self.rgb, self.solidity);
        for (p, v) in px.iter_mut().zip(values) {
            let v = if self.alpha { 1.0 - v } else { v };
            if v.is_nan() || v <= 0.0 {
                continue;
            }
            let v = v.min(1.0);
            let keep = 1.0 - s * v;
            for j in 0..3 {
                p[j] = keep * p[j] * (1.0 - v + v * c[j]) + s * v * c[j];
            }
        }
    }
}

/// The display colour of spot colour `color` in `doc`'s composite space: RGB colours as stored,
/// CMYK ones through the document's CMYK profile (the built-in coated CMYK otherwise), and
/// CMYK, Lab and gray colours of an RGB document converted from sRGB into its own profile.
pub fn ink_rgb(doc: &Document, color: Color) -> [f32; 3] {
    if matches!(color.mode, ColorMode::Rgb | ColorMode::Indexed | ColorMode::Multichannel) {
        return color.to_rgb();
    }
    let space = crate::cmyk_space(doc);
    let mut rgb = photocraft_color::convert::with_cmyk_space(space.as_ref(), || color.to_rgb());
    if doc.mode == ColorMode::Rgb
        && let Some(own) = doc.icc_profile.as_ref().and_then(|b| Profile::parse(b).ok()).filter(|p| p.color_space == ColorSpace::Rgb)
        && !own.same_colors(Builtin::Srgb.profile())
        && let Ok(t) = photocraft_cms::transform::cached(Builtin::Srgb.profile(), &own, TransformOptions::default())
    {
        t.apply(&mut rgb, 3);
    }
    rgb.map(|v| if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) })
}

/// Print the channels `visible(i)` picks from `doc.channels` (in panel order) as inks over the
/// opaque `buf`, reading them over `buf.rect`.
pub fn overprint(buf: &mut Buffer, doc: &Document, visible: impl Fn(usize) -> bool) {
    let rect = buf.rect;
    let mut vals = Vec::new();
    for (i, ch) in doc.channels.iter().enumerate() {
        if !visible(i) {
            continue;
        }
        let ink = Ink::of(doc, ch);
        // An empty alpha channel (all black) is solid black ink, so it can't be skipped.
        if !ink.alpha && ch.surface.content_bounds().intersect(&rect).is_empty() {
            continue;
        }
        let n = ch.surface.channels().max(1);
        ch.surface.read_region_into(rect, &mut vals);
        ink.apply(&mut buf.px, vals.chunks_exact(n).map(|v| v[0]));
    }
}

/// Whether channel `i` of `doc` is a spot ink that prints over an RGB or CMYK composite.
pub fn is_overprint_spot(doc: &Document, i: usize) -> bool {
    matches!(doc.mode, ColorMode::Rgb | ColorMode::Cmyk) && doc.channels.get(i).is_some_and(|c| c.spot.is_some())
}

/// The printed inks of `doc` over `rect` (opaque).
pub fn inks(doc: &Document, rect: Rect) -> Buffer {
    let mut buf = Buffer::filled(rect, [1.0, 1.0, 1.0, 1.0]);
    overprint(&mut buf, doc, |_| true);
    buf
}

/// The starting backdrop of a document render: the inks for Multichannel, else transparent.
pub(crate) fn backdrop(doc: &Document, rect: Rect) -> Buffer {
    if doc.mode == ColorMode::Multichannel { inks(doc, rect) } else { Buffer::transparent(rect) }
}

/// [`crate::thumbnail_buffer`] of an RGB or CMYK document with its spot channels printed over it
/// (matted over white, so opaque): the document's preview, as the reference editor saves it.
/// Unchanged when no spot channel has ink.
pub fn thumbnail_with_inks(doc: &Document, max_side: u32) -> Buffer {
    let mut buf = crate::thumbnail_buffer(doc, max_side);
    let spots: Vec<usize> = (0..doc.channels.len())
        .filter(|&i| is_overprint_spot(doc, i) && doc.channels.get(i).is_some_and(|c| !c.surface.content_bounds().intersect(&doc.bounds()).is_empty()))
        .collect();
    let (w, h) = (buf.rect.width(), buf.rect.height());
    if spots.is_empty() || w == 0 || h == 0 || buf.px.len() != w as usize * h as usize {
        return buf;
    }
    for p in &mut buf.px {
        let a = if p[3].is_nan() { 0.0 } else { p[3].clamp(0.0, 1.0) };
        *p = [p[0] * a + 1.0 - a, p[1] * a + 1.0 - a, p[2] * a + 1.0 - a, 1.0];
    }
    for i in spots {
        let Some(ch) = doc.channels.get(i) else { continue };
        let values = reduce(&ch.surface, doc.bounds(), w, h);
        Ink::of(doc, ch).apply(&mut buf.px, values);
    }
    buf
}

/// The first channel of `s` over `area`, area-averaged down to `w`×`h`, one output row at a time.
fn reduce(s: &photocraft_raster::Surface, area: Rect, w: u32, h: u32) -> Vec<f32> {
    let (sw, sh) = (u64::from(area.width()), u64::from(area.height()));
    let n = s.channels().max(1);
    let edge = |i: u32, out: u32, len: u64| (u64::from(i) * len / u64::from(out.max(1))) as i32;
    let mut out = Vec::with_capacity(w as usize * h as usize);
    let mut vals = Vec::new();
    for y in 0..h {
        let (y0, y1) = (edge(y, h, sh), edge(y + 1, h, sh));
        let y1 = y1.max(y0 + 1);
        s.read_region_into(Rect::new(area.x0, area.y0 + y0, area.x1, area.y0 + y1), &mut vals);
        let rows = (y1 - y0) as usize;
        let stride = sw as usize * n;
        for x in 0..w {
            let (x0, x1) = (edge(x, w, sw), edge(x + 1, w, sw));
            let x1 = x1.max(x0 + 1);
            let mut sum = 0.0f32;
            let mut count = 0u32;
            for r in 0..rows {
                for xi in x0..x1 {
                    if let Some(v) = vals.get(r * stride + xi as usize * n) {
                        sum += v;
                        count += 1;
                    }
                }
            }
            out.push(if count > 0 { sum / count as f32 } else { 0.0 });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use photocraft_color::{Color, ColorMode, PixelFormat, SampleType};
    use photocraft_doc::{AlphaChannel, Document, Size};
    use photocraft_raster::Surface;

    use super::*;

    #[test]
    fn inks_multiply_over_paper() {
        let mut doc = Document::new("m", Size::new(4, 1), ColorMode::Multichannel, SampleType::U8);
        let fmt = PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false);
        let mut a = Surface::new(fmt);
        a.write_region(Rect::new(0, 0, 4, 1), &[0.0, 1.0, 1.0, 0.0]);
        let mut b = Surface::new(fmt);
        b.write_region(Rect::new(0, 0, 4, 1), &[0.0, 0.0, 1.0, 1.0]);
        doc.channels.push(AlphaChannel { spot: Some((Color::rgb(0.0, 1.0, 1.0), 0.0)), ..AlphaChannel::new("Cyan", a) });
        doc.channels.push(AlphaChannel { spot: Some((Color::rgb(1.0, 0.0, 1.0), 0.0)), ..AlphaChannel::new("Magenta", b) });
        let r = crate::flatten(&doc);
        assert_eq!(r.px[0], [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(r.px[1], [0.0, 1.0, 1.0, 1.0]);
        assert_eq!(r.px[2], [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(r.px[3], [1.0, 0.0, 1.0, 1.0]);
        // An alpha-type channel (stored as-is: 1 = selected) prints as a black ink where it is
        // unselected, as the reference editor shows it.
        let mut k = Surface::new(fmt);
        k.write_region(Rect::new(0, 0, 4, 1), &[0.5, 1.0, 1.0, 0.0]);
        doc.channels.push(AlphaChannel::new("Alpha", k));
        let r = crate::flatten(&doc);
        // 0.5 is stored as 128/255 in an 8-bit channel: within one 8-bit step.
        assert!((r.px[0][0] - 0.5).abs() <= 1.0 / 255.0 && (r.px[0][1] - 0.5).abs() <= 1.0 / 255.0, "{:?}", r.px[0]);
        assert_eq!(r.px[1], [0.0, 1.0, 1.0, 1.0]);
        assert_eq!(r.px[3], [0.0, 0.0, 0.0, 1.0]);
        // Other modes have no inks.
        doc.mode = ColorMode::Rgb;
        assert_eq!(crate::flatten(&doc).px[1][3], 0.0);
    }

    /// One spot channel of `depth` holding `vals` (a row), ink `rgb` at `solidity`.
    fn spot(depth: SampleType, name: &str, rgb: [f32; 3], solidity: f32, vals: &[f32]) -> AlphaChannel {
        let mut s = Surface::new(PixelFormat::new(ColorMode::Grayscale, depth, false));
        s.write_region(Rect::new(0, 0, vals.len() as i32, 1), vals);
        AlphaChannel { spot: Some((Color::rgb(rgb[0], rgb[1], rgb[2]), solidity)), ..AlphaChannel::new(name, s) }
    }

    /// The ink formula written out for one ink over `p`.
    fn print(p: [f32; 3], c: [f32; 3], s: f32, v: f32) -> [f32; 3] {
        std::array::from_fn(|j| (1.0 - s * v) * p[j] * (1.0 - v + v * c[j]) + s * v * c[j])
    }

    fn close(a: [f32; 4], b: [f32; 3], tol: f32) -> bool {
        a[3] == 1.0 && a.iter().zip(&b).all(|(x, y)| (x - y).abs() <= tol)
    }

    /// Spot channels never reach the composite of an RGB or CMYK document: flatten, the merged
    /// image and export are the same with and without them.
    #[test]
    fn spots_stay_out_of_rgb_and_cmyk_flatten() {
        for mode in [ColorMode::Rgb, ColorMode::Cmyk] {
            for depth in [SampleType::U8, SampleType::U16] {
                let mut doc = Document::with_background("d", Size::new(4, 1), mode, depth, Color::rgb(0.8, 0.4, 0.2));
                let plain = crate::flatten(&doc);
                doc.channels.push(spot(depth, "teal", [0.0, 0.6, 0.65], 0.0, &[1.0, 0.5, 0.0, 1.0]));
                doc.channels.push(spot(depth, "pink", [1.0, 0.3, 0.6], 1.0, &[1.0, 1.0, 1.0, 1.0]));
                assert_eq!(crate::flatten(&doc).px, plain.px, "{mode:?} {depth:?}");
                let mut bare = doc.clone();
                bare.channels.clear();
                assert_eq!(crate::render(&doc, doc.bounds()).px, crate::render(&bare, doc.bounds()).px, "{mode:?} {depth:?}");
            }
        }
    }

    /// Two overlapping inks over a coloured picture, at every solidity and coverage, in both
    /// orders and at 8 and 16 bits: the second ink prints over the first.
    #[test]
    fn overprint_follows_the_ink_formula_in_panel_order() {
        let (a, b) = ([0.0, 0.6, 0.65], [0.9, 0.2, 0.5]);
        let under = [0.8, 0.7, 0.3];
        for depth in [SampleType::U8, SampleType::U16] {
            for mode in [ColorMode::Rgb, ColorMode::Cmyk] {
                for sa in [0.0, 0.5, 1.0] {
                    for sb in [0.0, 0.5, 1.0] {
                        let cover = [0.0, 0.5, 1.0];
                        let mut doc = Document::new("o", Size::new(3, 1), mode, depth);
                        doc.channels.push(spot(depth, "a", a, sa, &cover));
                        doc.channels.push(spot(depth, "b", b, sb, &[1.0, 0.5, 0.0]));
                        for swapped in [false, true] {
                            if swapped {
                                doc.channels.reverse();
                            }
                            let mut buf = Buffer::filled(doc.bounds(), [under[0], under[1], under[2], 1.0]);
                            overprint(&mut buf, &doc, |_| true);
                            let vals: Vec<(f32, f32)> = vec![(0.0, 1.0), (0.5, 0.5), (1.0, 0.0)];
                            for (x, &(va, vb)) in vals.iter().enumerate() {
                                let want = if swapped { print(print(under, b, sb, vb), a, sa, va) } else { print(print(under, a, sa, va), b, sb, vb) };
                                // 0.5 is stored as 128/255 at 8 bits.
                                assert!(close(buf.px[x], want, 0.006), "{mode:?} {depth:?} s {sa}/{sb} swapped {swapped} x {x}: {:?} vs {want:?}", buf.px[x]);
                            }
                            if swapped {
                                doc.channels.reverse();
                            }
                        }
                        // Only the channels `visible` picks print.
                        let mut buf = Buffer::filled(doc.bounds(), [under[0], under[1], under[2], 1.0]);
                        overprint(&mut buf, &doc, |i| i == 0);
                        assert!(close(buf.px[2], print(under, a, sa, 1.0), 1e-5));
                    }
                }
            }
        }
    }

    /// A solid transparent ink over white shows its own colour; a solid opaque one covers any colour.
    #[test]
    fn solid_inks_show_their_colour() {
        let teal = [0.0, 157.0 / 255.0, 165.0 / 255.0];
        let mut doc = Document::new("s", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
        doc.channels.push(spot(SampleType::U8, "teal", teal, 0.0, &[1.0]));
        let mut buf = Buffer::filled(doc.bounds(), [1.0; 4]);
        overprint(&mut buf, &doc, |_| true);
        assert!(close(buf.px[0], teal, 1e-6), "{:?}", buf.px[0]);
        doc.channels[0].spot = Some((Color::rgb(teal[0], teal[1], teal[2]), 1.0));
        let mut buf = Buffer::filled(doc.bounds(), [0.2, 0.9, 0.1, 1.0]);
        overprint(&mut buf, &doc, |_| true);
        assert!(close(buf.px[0], teal, 1e-6), "{:?}", buf.px[0]);
    }

    /// A CMYK spot colour displays through the document's CMYK profile, as the CMYK composite does.
    #[test]
    fn cmyk_ink_colours_go_through_the_document_profile() {
        let doc = Document::new("c", Size::new(1, 1), ColorMode::Cmyk, SampleType::U8);
        let cyan = Color { mode: ColorMode::Cmyk, c: [1.0, 0.0, 0.0, 0.0], alpha: 1.0 };
        let space = crate::cmyk_space(&doc);
        let want = photocraft_color::convert::with_cmyk_space(space.as_ref(), || cyan.to_rgb());
        assert_eq!(ink_rgb(&doc, cyan), want.map(|v| v.clamp(0.0, 1.0)));
        // RGB inks are taken as stored.
        assert_eq!(ink_rgb(&doc, Color::rgb(0.1, 0.2, 0.3)), [0.1, 0.2, 0.3]);
    }

    /// The thumbnail prints the spot inks over the composite (matted over white).
    #[test]
    fn thumbnail_prints_spot_inks() {
        for mode in [ColorMode::Rgb, ColorMode::Cmyk] {
            let mut doc = Document::new("t", Size::new(4, 1), mode, SampleType::U8);
            let plain = thumbnail_with_inks(&doc, 160);
            assert_eq!(plain.px, crate::thumbnail_buffer(&doc, 160).px, "no ink: unchanged");
            doc.channels.push(spot(SampleType::U8, "teal", [0.0, 0.6, 0.65], 0.0, &[1.0, 1.0, 0.0, 0.0]));
            let t = thumbnail_with_inks(&doc, 160);
            assert!(close(t.px[0], [0.0, 0.6, 0.65], 0.003), "{mode:?} {:?}", t.px[0]);
            assert!(close(t.px[3], [1.0, 1.0, 1.0], 1e-6), "{mode:?} {:?}", t.px[3]);
            // Halved: each thumbnail pixel averages two document pixels.
            let t = thumbnail_with_inks(&doc, 2);
            assert!(close(t.px[0], print([1.0; 3], [0.0, 0.6, 0.65], 0.0, 1.0), 0.003), "{:?}", t.px);
            assert!(close(t.px[1], [1.0; 3], 1e-6), "{:?}", t.px);
        }
    }
}
