//! Spot inks printed over a picture: every spot (ink) channel of a Multichannel document over
//! white paper, and the visible spot channels of an RGB or CMYK document over its composite
//! (the canvas preview and the PSD thumbnail; never `flatten`, export or the merged image).
//!
//! Ink density `v` (the channel value, 1 = solid) first gets the Spot working space's dot gain
//! (Edit › Color Settings › Spot, [`set_spot_dot_gain`]): it prints with coverage `e` =
//! [`dot_gain`]`(v)`, as on press. An ink with colour `c` and solidity `s` then turns the colour
//! underneath `p` into `(1 − e)·p + e·(s·c + (1 − s)·p·c)`, per channel, in linear light (the
//! composite's tone curve decoded, [`InkLight`]): transparent inks (solidity 0) multiply like
//! overprinted process inks, opaque ones cover what is below. Inks print in Channels panel order.
//! This is the reference editor's Merge Spot Channels, measured on a chart of one ink at five
//! solidities × eleven coverages and two overlapping inks both ways round (mean 0.35 levels, max
//! 1.3 at a 38 % gain in Adobe RGB); its spot preview shows the same. Over white the result does
//! not depend on solidity. An alpha-type channel of a Multichannel document (selected / masked
//! areas) still prints, as a transparent black ink, like the reference editor shows it. Its value
//! is stored as-is (1 = selected, white in the Channels panel), so its ink is `1 − value`.
//!
//! The canvas preview, the saved thumbnail ([`thumbnail_with_inks`], as Photoshop bakes the gain
//! set at save time into a document's preview), Image › Mode out of Multichannel and Merge Spot
//! Channel all print through [`Press`], so they agree.

//! This is a screen approximation, not a press proof: stored RGB alternate colours are taken as
//! sRGB and, like Lab/gray alternates, converted into an RGB document's own profile; CMYK
//! alternates use the document CMYK profile. No spectral ink or paper interaction is modelled.
//! At full coverage, solidity 0 multiplies the underlying colour and solidity 1 replaces it.
//! Previewing borrows the document and only changes the supplied display buffer.

use std::sync::Arc;

use photocraft_cms::{Builtin, ColorSpace, Curve, Profile, Transform, TransformOptions};
use photocraft_color::convert::CmykSpace;
use photocraft_color::{Color, ColorMode};
use photocraft_doc::{AlphaChannel, Document};
use photocraft_geom::Rect;

use crate::Buffer;

/// Photoshop's default Spot working space: Dot Gain 20 %.
pub const SPOT_DOT_GAIN: f32 = 20.0;
/// The largest dot gain the setting takes: a 50 % dot at 99 % (a gain of 50 % or more would
/// print every midtone solid).
pub const MAX_SPOT_DOT_GAIN: f32 = 49.0;

static SPOT_DOT_GAIN_SETTING: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x41a0_0000); // 20.0f32

/// Sets Color Settings › Spot: the dot gain, in percent at a 50 % dot, that the ink preview adds
/// to every spot channel. It is an application setting, not stored in documents; values are
/// clamped to 0–[`MAX_SPOT_DOT_GAIN`].
///
/// The value is process-wide: a test that changes it races every concurrently running test that
/// renders a Multichannel document, so such tests belong in their own test binary (see
/// `crates/compose/tests/spot_dot_gain.rs`).
pub fn set_spot_dot_gain(percent: f32) {
    let g = if percent.is_finite() { percent.clamp(0.0, MAX_SPOT_DOT_GAIN) } else { SPOT_DOT_GAIN };
    SPOT_DOT_GAIN_SETTING.store(g.to_bits(), std::sync::atomic::Ordering::Relaxed);
}

/// The current Spot dot gain in percent (see [`set_spot_dot_gain`]).
pub fn spot_dot_gain() -> f32 {
    f32::from_bits(SPOT_DOT_GAIN_SETTING.load(std::sync::atomic::Ordering::Relaxed))
}

/// The ink coverage that prints for channel value `v` (0..1) with a dot gain of `percent` at a
/// 50 % dot: `1 − (1 − v)^γ`, with γ chosen so a 50 % dot prints at 50 % + `percent`. Solids and
/// paper stay as they are. This is the curve family of the built-in "Dot Gain 20 %-compatible"
/// gray (a gamma fitted on Photoshop's Dot Gain 20 % profile; 20 % here gives γ ≈ 1.737).
pub fn dot_gain(v: f32, percent: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    let g = if percent.is_finite() { percent.clamp(0.0, MAX_SPOT_DOT_GAIN) / 100.0 } else { 0.0 };
    if g <= 0.0 {
        return v;
    }
    let gamma = (0.5 - g).ln() / 0.5f32.ln();
    1.0 - (1.0 - v).powf(gamma)
}

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
    /// transparent black. Printing several inks, build one [`InkSpace`] and use [`InkSpace::ink`].
    pub fn of(doc: &Document, ch: &AlphaChannel) -> Ink {
        InkSpace::of(doc).ink(ch)
    }

    /// The ink amount channel value `v` stands for: `v` for a spot channel, `1 − v` for an alpha
    /// channel; `None` where nothing prints.
    fn amount(&self, v: f32) -> Option<f32> {
        let v = if self.alpha { 1.0 - v } else { v };
        (v > 0.0).then_some(v)
    }
}

/// The tone curve of a composite's colour space, which turns its encoded values into linear light
/// (the light inks mix in): the document's profile's curves, or sRGB's when it has none.
#[derive(Clone, Debug, PartialEq)]
pub struct InkLight {
    /// One curve per RGB channel; `None` when every one is the identity (linear light already).
    trc: Option<Arc<[Curve; 3]>>,
}

impl InkLight {
    /// The sRGB tone curve.
    pub fn srgb() -> InkLight {
        let t = photocraft_cms::curve::srgb_trc();
        InkLight { trc: Some(Arc::new([t.clone(), t.clone(), t])) }
    }

    /// The tone curves of `profile`: an RGB matrix/TRC profile's three curves, a gray profile's
    /// curve on every channel; `None` for a profile without them (a LUT-based or CMYK one).
    pub fn of_profile(profile: &Profile) -> Option<InkLight> {
        if profile.a2b.iter().any(Option::is_some) {
            return None;
        }
        let trc = match (profile.color_space, &profile.trc, &profile.gray_trc) {
            (ColorSpace::Rgb, Some(t), _) if profile.matrix.is_some() => t.clone(),
            (ColorSpace::Gray, _, Some(g)) => [g.clone(), g.clone(), g.clone()],
            _ => return None,
        };
        Some(InkLight { trc: (!trc.iter().all(Curve::is_identity)).then(|| Arc::new(trc)) })
    }

    /// Encoded values → linear light.
    fn decode(&self, p: [f32; 3]) -> [f32; 3] {
        match &self.trc {
            Some(t) => std::array::from_fn(|j| t[j].eval(p[j])),
            None => p,
        }
    }

    /// Linear light → encoded values.
    fn encode(&self, p: [f32; 3]) -> [f32; 3] {
        match &self.trc {
            Some(t) => std::array::from_fn(|j| t[j].eval_inverse(p[j])),
            None => p,
        }
    }
}

/// Inks printing, one after another, over the opaque pixels `px` (see the module docs). A pixel
/// is turned into linear light the first time an ink reaches it and back when the press is
/// dropped, so pixels no ink reaches stay exactly as they were.
pub struct Press<'a> {
    px: &'a mut [[f32; 4]],
    light: &'a InkLight,
    gain: f32,
    /// The pixels in linear light now.
    lit: Vec<bool>,
}

impl<'a> Press<'a> {
    /// A press over `px` whose inks print in `light` with a dot gain of `gain` percent at a
    /// 50 % dot ([`dot_gain`]).
    pub fn new(px: &'a mut [[f32; 4]], light: &'a InkLight, gain: f32) -> Press<'a> {
        let lit = vec![false; px.len()];
        Press { px, light, gain, lit }
    }

    /// Print `ink` with channel values `values` (one per pixel, in order) over what is there.
    pub fn print(&mut self, ink: &Ink, values: impl IntoIterator<Item = f32>) {
        let c = self.light.decode(ink.rgb);
        let s = ink.solidity;
        for ((p, lit), v) in self.px.iter_mut().zip(self.lit.iter_mut()).zip(values) {
            let Some(v) = ink.amount(v) else { continue };
            if !*lit {
                let l = self.light.decode([p[0], p[1], p[2]]);
                p[..3].copy_from_slice(&l);
                *lit = true;
            }
            let e = dot_gain(v, self.gain);
            for j in 0..3 {
                p[j] = (1.0 - e) * p[j] + e * (s * c[j] + (1.0 - s) * p[j] * c[j]);
            }
        }
    }
}

impl Drop for Press<'_> {
    fn drop(&mut self) {
        for (p, lit) in self.px.iter_mut().zip(&self.lit) {
            if *lit {
                let e = self.light.encode([p[0], p[1], p[2]]);
                p[..3].copy_from_slice(&e);
            }
        }
    }
}

/// How spot colours of one document map into its composite space, and the light they mix in.
/// Build it once per render: the document's profiles are parsed here, not per ink or per damage
/// rect.
pub struct InkSpace {
    cmyk: Option<Arc<CmykSpace>>,
    /// sRGB → an RGB document's own profile, when that differs from sRGB.
    to_own: Option<Arc<Transform>>,
    light: InkLight,
}

impl InkSpace {
    pub fn of(doc: &Document) -> InkSpace {
        let own = doc.icc_profile.as_ref().and_then(|b| Profile::parse(b).ok());
        let to_own = if doc.mode == ColorMode::Rgb {
            own.as_ref()
                .filter(|p| p.color_space == ColorSpace::Rgb && !p.same_colors(Builtin::Srgb.profile()))
                .and_then(|own| photocraft_cms::transform::cached(Builtin::Srgb.profile(), own, TransformOptions::default()).ok())
        } else {
            None
        };
        // RGB and Grayscale composites are in the document's own profile; the others' (CMYK, Lab,
        // Multichannel) are sRGB.
        let light = own
            .as_ref()
            .filter(|p| matches!((doc.mode, p.color_space), (ColorMode::Rgb, ColorSpace::Rgb) | (ColorMode::Grayscale, ColorSpace::Gray)))
            .and_then(InkLight::of_profile)
            .unwrap_or_else(InkLight::srgb);
        InkSpace { cmyk: crate::cmyk_space(doc), to_own, light }
    }

    /// The same inks, mixed in `light` instead: Image › Mode out of Multichannel prints them in the
    /// light of the colour space the image goes to.
    pub fn in_light(self, light: InkLight) -> InkSpace {
        InkSpace { light, ..self }
    }

    /// The light the inks mix in.
    pub fn light(&self) -> &InkLight {
        &self.light
    }

    /// A [`Press`] over `px` in this space's light, with a dot gain of `gain` percent.
    pub fn press<'a>(&'a self, px: &'a mut [[f32; 4]], gain: f32) -> Press<'a> {
        Press::new(px, &self.light, gain)
    }

    /// The ink of channel `ch`: a spot channel's colour and solidity, an alpha channel's
    /// transparent black.
    pub fn ink(&self, ch: &AlphaChannel) -> Ink {
        match ch.spot {
            Some((color, solidity)) => Ink { rgb: self.rgb(color), solidity: solidity.clamp(0.0, 1.0), alpha: false },
            None => Ink { rgb: [0.0; 3], solidity: 0.0, alpha: true },
        }
    }

    /// The display colour of spot colour `color` in the composite space: CMYK colours through the
    /// document's CMYK profile (the built-in coated CMYK otherwise), and RGB (taken as sRGB), CMYK,
    /// Lab and gray colours of an RGB document converted from sRGB into its own profile, so an ink
    /// keeps its colour in a linear (32-bit), Adobe RGB or ProPhoto document.
    pub fn rgb(&self, color: Color) -> [f32; 3] {
        if matches!(color.mode, ColorMode::Indexed | ColorMode::Multichannel) {
            return color.to_rgb();
        }
        let mut rgb = photocraft_color::convert::with_cmyk_space(self.cmyk.as_ref(), || color.to_rgb());
        if let Some(t) = &self.to_own {
            t.apply(&mut rgb, 3);
        }
        rgb.map(|v| if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) })
    }
}

/// Print the channels `visible(i)` picks from `doc.channels` (in panel order) as inks over the
/// opaque `buf`, reading them over `buf.rect`, with the Spot dot gain ([`spot_dot_gain`]).
pub fn overprint(buf: &mut Buffer, doc: &Document, visible: impl Fn(usize) -> bool) {
    let rect = buf.rect;
    overprint_in(&mut buf.px, rect, doc, &InkSpace::of(doc), visible);
}

/// [`overprint`] of the pixels `px` of `rect` with the inks of `space`.
fn overprint_in(px: &mut [[f32; 4]], rect: Rect, doc: &Document, space: &InkSpace, visible: impl Fn(usize) -> bool) {
    let mut press = space.press(px, spot_dot_gain());
    let mut vals = Vec::new();
    for (i, ch) in doc.channels.iter().enumerate() {
        if !visible(i) {
            continue;
        }
        let ink = space.ink(ch);
        // An empty alpha channel (all black) is solid black ink, so it can't be skipped.
        if !ink.alpha && ch.surface.content_bounds().intersect(&rect).is_empty() {
            continue;
        }
        let n = ch.surface.channels().max(1);
        ch.surface.read_region_into(rect, &mut vals);
        press.print(&ink, vals.chunks_exact(n).map(|v| v[0]));
    }
}

/// Whether channel `i` of `doc` is a spot ink that prints over an RGB or CMYK composite.
pub fn is_overprint_spot(doc: &Document, i: usize) -> bool {
    matches!(doc.mode, ColorMode::Rgb | ColorMode::Cmyk | ColorMode::Grayscale | ColorMode::Lab) && doc.channels.get(i).is_some_and(|c| c.spot.is_some())
}

/// The printed inks of `doc` over `rect` (opaque).
pub fn inks(doc: &Document, rect: Rect) -> Buffer {
    inks_in(doc, rect, &InkSpace::of(doc))
}

/// [`inks`] with the inks of `space` (Image › Mode out of Multichannel mixes them in the light of
/// the colour space the image goes to, [`InkSpace::in_light`]).
/// Off wasm, bands of rows print in parallel, so a whole canvas (the mode conversion) uses every
/// core.
pub fn inks_in(doc: &Document, rect: Rect, space: &InkSpace) -> Buffer {
    /// Rows per band.
    const BAND: usize = 64;
    let mut buf = Buffer::filled(rect, [1.0, 1.0, 1.0, 1.0]);
    let w = rect.width() as usize;
    if w == 0 || buf.px.len() != w * rect.height() as usize {
        return buf;
    }
    let band = |(k, px): (usize, &mut [[f32; 4]])| {
        let y0 = rect.y0.saturating_add(i32::try_from(k * BAND).unwrap_or(i32::MAX));
        let rows = i32::try_from(px.len() / w).unwrap_or(0);
        overprint_in(px, Rect::new(rect.x0, y0, rect.x1, y0.saturating_add(rows)), doc, space, |_| true);
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        buf.px.par_chunks_mut(w * BAND).enumerate().for_each(band);
    }
    #[cfg(target_arch = "wasm32")]
    buf.px.chunks_mut(w * BAND).enumerate().for_each(band);
    buf
}

/// The starting backdrop of a document render: the inks for Multichannel, else transparent.
pub(crate) fn backdrop(doc: &Document, rect: Rect) -> Buffer {
    if doc.mode == ColorMode::Multichannel { inks(doc, rect) } else { Buffer::transparent(rect) }
}

/// [`crate::thumbnail_buffer`] of an RGB or CMYK document with its spot channels printed over it
/// (matted over white, so opaque, where there is ink) with the Spot dot gain ([`spot_dot_gain`]):
/// the document's preview, as the reference editor saves it.
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
    let space = InkSpace::of(doc);
    let inks: Vec<(Ink, Vec<f32>)> =
        spots.into_iter().filter_map(|i| doc.channels.get(i)).map(|ch| (space.ink(ch), reduce(&ch.surface, doc.bounds(), w, h))).collect();
    // Only pixels with ink are matted over white (the inks print on paper); the rest keep their
    // transparency.
    for (k, p) in buf.px.iter_mut().enumerate() {
        if inks.iter().any(|(_, vals)| vals.get(k).is_some_and(|v| *v > 0.0)) {
            let a = if p[3].is_nan() { 0.0 } else { p[3].clamp(0.0, 1.0) };
            *p = [p[0] * a + 1.0 - a, p[1] * a + 1.0 - a, p[2] * a + 1.0 - a, 1.0];
        }
    }
    {
        let mut press = space.press(&mut buf.px, spot_dot_gain());
        for (ink, vals) in inks {
            press.print(&ink, vals);
        }
    }
    buf
}

/// Samples per side of a thumbnail pixel that [`reduce`] averages at most.
const REDUCE_SAMPLES: i32 = 4;

/// The first channel of `s` over `area`, averaged down to `w`×`h`, one output row at a time.
/// A box up to [`REDUCE_SAMPLES`] pixels a side is averaged whole; a bigger one at that many
/// evenly spaced rows and columns, so a huge document costs about `h × 4` row reads, not a scan.
fn reduce(s: &photocraft_raster::Surface, area: Rect, w: u32, h: u32) -> Vec<f32> {
    let (sw, sh) = (u64::from(area.width()), u64::from(area.height()));
    let n = s.channels().max(1);
    let edge = |i: u32, out: u32, len: u64| (u64::from(i) * len / u64::from(out.max(1))) as i32;
    // The offsets from a box's start that are sampled.
    let picks = |len: i32| -> Vec<i32> {
        if len <= REDUCE_SAMPLES { (0..len).collect() } else { (0..REDUCE_SAMPLES).map(|j| ((2 * j + 1) * len) / (2 * REDUCE_SAMPLES)).collect() }
    };
    let mut out = Vec::with_capacity(w as usize * h as usize);
    let mut row = Vec::new();
    let mut sums = vec![(0.0f32, 0u32); w as usize];
    for y in 0..h {
        let (y0, y1) = (edge(y, h, sh), edge(y + 1, h, sh));
        sums.iter_mut().for_each(|t| *t = (0.0, 0));
        for dy in picks((y1 - y0).max(1)) {
            let sy = area.y0 + y0 + dy;
            s.read_region_into(Rect::new(area.x0, sy, area.x1, sy + 1), &mut row);
            for (x, t) in sums.iter_mut().enumerate() {
                let (x0, x1) = (edge(x as u32, w, sw), edge(x as u32 + 1, w, sw));
                for dx in picks((x1 - x0).max(1)) {
                    if let Some(v) = row.get((x0 + dx) as usize * n) {
                        t.0 += v;
                        t.1 += 1;
                    }
                }
            }
        }
        out.extend(sums.iter().map(|&(sum, count)| if count > 0 { sum / count as f32 } else { 0.0 }));
    }
    out
}

#[cfg(test)]
mod tests {
    use photocraft_color::convert::{linear_to_srgb, srgb_to_linear};
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
        let near = |a: [f32; 4], b: [f32; 4]| a.iter().zip(&b).all(|(x, y)| (x - y).abs() <= 1e-5);
        assert_eq!(r.px[0], [1.0, 1.0, 1.0, 1.0]);
        assert!(near(r.px[1], [0.0, 1.0, 1.0, 1.0]), "{:?}", r.px[1]);
        assert!(near(r.px[2], [0.0, 0.0, 1.0, 1.0]), "{:?}", r.px[2]);
        assert!(near(r.px[3], [1.0, 0.0, 1.0, 1.0]), "{:?}", r.px[3]);
        // An alpha-type channel (stored as-is: 1 = selected) prints as a black ink where it is
        // unselected, as the reference editor shows it.
        let mut k = Surface::new(fmt);
        k.write_region(Rect::new(0, 0, 4, 1), &[0.5, 1.0, 1.0, 0.0]);
        doc.channels.push(AlphaChannel::new("Alpha", k));
        let r = crate::flatten(&doc);
        // 0.5 is stored as 128/255 in an 8-bit channel, so its ink is 127/255; the preview adds
        // the Spot dot gain, and the ink takes that much light away (an untagged document mixes in
        // sRGB's light).
        let paper = linear_to_srgb(1.0 - dot_gain(1.0 - 128.0 / 255.0, spot_dot_gain()));
        assert!((r.px[0][0] - paper).abs() <= 1e-4 && (r.px[0][1] - paper).abs() <= 1e-4, "{:?} vs {paper}", r.px[0]);
        assert!(near(r.px[1], [0.0, 1.0, 1.0, 1.0]), "{:?}", r.px[1]);
        assert!(near(r.px[3], [0.0, 0.0, 0.0, 1.0]), "{:?}", r.px[3]);
        // Other modes have no inks.
        doc.mode = ColorMode::Rgb;
        assert_eq!(crate::flatten(&doc).px[1][3], 0.0);
    }

    #[test]
    fn dot_gain_lifts_a_50_percent_dot_by_the_gain_and_keeps_paper_and_solids() {
        for gain in [10.0, 15.0, 20.0, 25.0, 30.0, 38.0, MAX_SPOT_DOT_GAIN] {
            assert!((dot_gain(0.5, gain) - (0.5 + gain / 100.0)).abs() < 1e-5, "{gain}");
            assert_eq!(dot_gain(0.0, gain), 0.0);
            assert!((dot_gain(1.0, gain) - 1.0).abs() < 1e-6);
            let mut last = 0.0;
            for i in 1..=100 {
                let v = dot_gain(i as f32 / 100.0, gain);
                assert!(v >= last && v >= i as f32 / 100.0, "{gain}: rises and never lightens");
                last = v;
            }
        }
        assert_eq!(dot_gain(0.3, 0.0), 0.3, "no gain is linear");
        assert_eq!(dot_gain(0.3, f32::NAN), 0.3);
        // The same curve family as the built-in Dot Gain 20 %-compatible gray (gamma 1.732).
        assert!((dot_gain(0.5, 20.0) - (1.0 - 0.5f32.powf(1.732))).abs() < 2e-3);
    }

    /// One spot channel of `depth` holding `vals` (a row), ink `rgb` at `solidity`.
    fn spot(depth: SampleType, name: &str, rgb: [f32; 3], solidity: f32, vals: &[f32]) -> AlphaChannel {
        let mut s = Surface::new(PixelFormat::new(ColorMode::Grayscale, depth, false));
        s.write_region(Rect::new(0, 0, vals.len() as i32, 1), vals);
        AlphaChannel { spot: Some((Color::rgb(rgb[0], rgb[1], rgb[2]), solidity)), ..AlphaChannel::new(name, s) }
    }

    /// The ink formula written out for one ink over `p`, in sRGB's light (the documents here are
    /// untagged). The overprint adds the Spot dot gain (read, not changed: the setting is
    /// process-wide).
    fn print(p: [f32; 3], c: [f32; 3], s: f32, v: f32) -> [f32; 3] {
        let e = dot_gain(v, spot_dot_gain());
        std::array::from_fn(|j| {
            let (p, c) = (srgb_to_linear(p[j]), srgb_to_linear(c[j]));
            linear_to_srgb((1.0 - e) * p + e * (s * c + (1.0 - s) * p * c))
        })
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
                        assert!(close(buf.px[2], print(under, a, sa, 1.0), 1e-4));
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
        assert!(close(buf.px[0], teal, 1e-5), "{:?}", buf.px[0]);
        doc.channels[0].spot = Some((Color::rgb(teal[0], teal[1], teal[2]), 1.0));
        let mut buf = Buffer::filled(doc.bounds(), [0.2, 0.9, 0.1, 1.0]);
        overprint(&mut buf, &doc, |_| true);
        assert!(close(buf.px[0], teal, 1e-5), "{:?}", buf.px[0]);
    }

    #[test]
    fn hidden_spot_leaves_the_process_composite_unchanged() {
        for mode in [ColorMode::Rgb, ColorMode::Cmyk] {
            for depth in [SampleType::U8, SampleType::U16] {
                let mut doc = Document::with_background("hidden", Size::new(1, 1), mode, depth, Color::rgb(0.8, 0.4, 0.2));
                doc.channels.push(spot(depth, "opaque", [0.0, 0.6, 0.65], 1.0, &[1.0]));
                let before = crate::flatten(&doc);
                let mut preview = before.clone();
                overprint(&mut preview, &doc, |_| false);
                assert_eq!(preview.px, before.px);
                overprint(&mut preview, &doc, |_| true);
                assert!(close(preview.px[0], [0.0, 0.6, 0.65], 1e-5));
                assert_eq!(crate::flatten(&doc).px, before.px);
                assert_eq!(doc.channels[0].spot, Some((Color::rgb(0.0, 0.6, 0.65), 1.0)));
            }
        }
    }

    /// A CMYK spot colour displays through the document's CMYK profile, as the CMYK composite does.
    #[test]
    fn cmyk_ink_colours_go_through_the_document_profile() {
        let doc = Document::new("c", Size::new(1, 1), ColorMode::Cmyk, SampleType::U8);
        let cyan = Color { mode: ColorMode::Cmyk, c: [1.0, 0.0, 0.0, 0.0], alpha: 1.0 };
        let space = crate::cmyk_space(&doc);
        let want = photocraft_color::convert::with_cmyk_space(space.as_ref(), || cyan.to_rgb());
        assert_eq!(InkSpace::of(&doc).rgb(cyan), want.map(|v| v.clamp(0.0, 1.0)));
        // RGB inks are taken as stored.
        assert_eq!(InkSpace::of(&doc).rgb(Color::rgb(0.1, 0.2, 0.3)), [0.1, 0.2, 0.3]);
    }

    /// An RGB spot colour is sRGB, like the other alternates: an sRGB document takes it as stored,
    /// a linear (32-bit) one gets its linear values, which the canvas encodes back to the colour.
    #[test]
    fn rgb_ink_colours_convert_into_the_document_profile() {
        let teal = Color::rgb(0.0, 0.6, 0.65);
        let mut doc = Document::new("l", Size::new(1, 1), ColorMode::Rgb, SampleType::F32);
        assert_eq!(InkSpace::of(&doc).rgb(teal), [0.0, 0.6, 0.65]);
        doc.icc_profile = Some(Builtin::LinearSrgb.profile().to_bytes());
        let got = InkSpace::of(&doc).rgb(teal);
        let want = [0.0, 0.6, 0.65].map(photocraft_color::convert::srgb_to_linear);
        assert!(got.iter().zip(&want).all(|(g, w)| (g - w).abs() <= 0.005), "{got:?} vs {want:?}");
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
            // No ink: the transparent pixel stays transparent (the writer mattes it later).
            assert_eq!(t.px[3], plain.px[3], "{mode:?}");
            // Halved: each thumbnail pixel averages two document pixels.
            let t = thumbnail_with_inks(&doc, 2);
            assert!(close(t.px[0], print([1.0; 3], [0.0, 0.6, 0.65], 0.0, 1.0), 0.003), "{:?}", t.px);
            assert_eq!(t.px[1], crate::thumbnail_buffer(&doc, 2).px[1], "no ink: unchanged");
        }
    }

    /// A box bigger than the sample grid averages evenly spaced samples: ink on the left half of
    /// a 64-pixel box reads half covered.
    #[test]
    fn reduce_samples_big_boxes() {
        let mut sf = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false));
        let vals: Vec<f32> = (0..64).map(|x| if x < 32 { 1.0 } else { 0.0 }).collect();
        sf.write_region(Rect::new(0, 0, 64, 1), &vals);
        let r = reduce(&sf, Rect::new(0, 0, 64, 1), 1, 1);
        assert_eq!(r, [0.5]);
        // Small boxes stay exact.
        let r = reduce(&sf, Rect::new(0, 0, 64, 1), 16, 1);
        assert_eq!(r, (0..16).map(|x| if x < 8 { 1.0 } else { 0.0 }).collect::<Vec<f32>>());
    }

    fn adobe() -> InkLight {
        InkLight::of_profile(Builtin::AdobeRgbCompat.profile()).unwrap()
    }

    /// 50 % of #3366cc over white in Adobe RGB with a 38 % spot dot gain is #6a84d3 in the
    /// reference editor's Merge Spot Channels at every solidity (the ink prints at 88 %, mixed in
    /// linear light). The old gamma-space mix without the gain gave #99b2e5 at solidity 0 and
    /// #668cd9 at 100.
    #[test]
    fn half_tint_over_white_matches_the_reference_at_every_solidity() {
        let blue = [0x33, 0x66, 0xcc].map(|v| v as f32 / 255.0);
        let light = adobe();
        for s in [0.0, 0.3, 0.5, 0.8, 1.0] {
            let mut px = vec![[1.0; 4]];
            Press::new(&mut px, &light, 38.0).print(&Ink { rgb: blue, solidity: s, alpha: false }, [0.5]);
            let got = px[0].map(|v| v * 255.0);
            for (g, want) in got.iter().zip([106.0, 132.0, 211.0]) {
                assert!((g - want).abs() <= 1.0, "solidity {s}: {got:?}");
            }
            assert_eq!(px[0][3], 1.0);
        }
    }

    /// Pixels no ink reaches keep their exact values (they never go through the tone curve).
    #[test]
    fn press_leaves_uninked_pixels_exact() {
        let light = adobe();
        let before = vec![[0.123_456_7, 0.5, 0.987_654_3, 1.0], [0.3, 0.2, 0.1, 1.0]];
        let mut px = before.clone();
        {
            let mut press = Press::new(&mut px, &light, 20.0);
            press.print(&Ink { rgb: [0.0, 0.6, 0.65], solidity: 0.5, alpha: false }, [0.0, 1.0]);
            press.print(&Ink { rgb: [0.0; 3], solidity: 0.0, alpha: true }, [1.0, f32::NAN]);
        }
        assert_eq!(px[0], before[0]);
        assert_ne!(px[1], before[1]);
    }

    /// The light comes from the document's profile: Adobe RGB's gamma, none for linear sRGB, and
    /// sRGB's curve for untagged documents and modes whose composite is sRGB.
    #[test]
    fn ink_light_follows_the_document_profile() {
        assert!(adobe().trc.is_some());
        assert_eq!(InkLight::of_profile(Builtin::LinearSrgb.profile()), Some(InkLight { trc: None }));
        assert_eq!(InkLight::of_profile(Builtin::CoatedCmyk.profile()), None);
        let mut doc = Document::new("a", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
        assert_eq!(InkSpace::of(&doc).light(), &InkLight::srgb());
        doc.icc_profile = Some(Builtin::AdobeRgbCompat.profile().to_bytes());
        assert_eq!(InkSpace::of(&doc).light(), &adobe());
        doc.mode = ColorMode::Cmyk;
        assert_eq!(InkSpace::of(&doc).light(), &InkLight::srgb(), "a CMYK composite is sRGB");
        assert_eq!(InkSpace::of(&doc).in_light(adobe()).light(), &adobe());
    }

    /// The whole-canvas print in bands is the one-pass print.
    #[test]
    fn inks_in_bands_match_one_pass() {
        let (w, h) = (3, 150);
        let mut doc = Document::new("b", Size::new(w, h), ColorMode::Multichannel, SampleType::U16);
        let ramp: Vec<f32> = (0..w * h).map(|i| (i % 97) as f32 / 96.0).collect();
        let rev: Vec<f32> = ramp.iter().map(|v| 1.0 - v).collect();
        for (name, rgb, s, vals) in [("a", [0.0, 0.6, 0.65], 0.3, &ramp), ("b", [0.9, 0.2, 0.5], 1.0, &rev)] {
            let mut sf = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U16, false));
            sf.write_region(Rect::new(0, 0, w as i32, h as i32), vals);
            doc.channels.push(AlphaChannel { spot: Some((Color::rgb(rgb[0], rgb[1], rgb[2]), s)), ..AlphaChannel::new(name, sf) });
        }
        let space = InkSpace::of(&doc).in_light(adobe());
        let banded = inks_in(&doc, doc.bounds(), &space);
        let mut one = Buffer::filled(doc.bounds(), [1.0; 4]);
        overprint_in(&mut one.px, doc.bounds(), &doc, &space, |_| true);
        assert_eq!(banded.px, one.px);
        assert_eq!(inks(&doc, doc.bounds()).px.len(), (w * h) as usize);
    }
}
