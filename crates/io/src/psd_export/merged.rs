use super::{ColorMode, Document, Interrupt, PixelFormat, SampleType, encode_be, q255};

pub(super) fn cmyk_of(fmt: &PixelFormat) -> bool {
    fmt.mode == ColorMode::Cmyk
}

/// The merged image's planes (colour channels, then alpha), big-endian, matted against white
/// when `matte`; with whether any pixel needs the alpha channel and whether any is below 1.
/// `ctl` gets the fraction of rows done after each band; once it is cancelled the rest of the
/// image is left blank (the caller discards it).
pub(super) fn merged_planes(doc: &Document, fmt: &PixelFormat, cmyk: bool, matte: bool, ctl: &Interrupt) -> (Vec<u8>, bool, bool) {
    if cmyk && let Some(planes) = merged_ink_planes(doc, fmt.sample, matte, ctl) {
        return planes;
    }
    merged_planes_in_bands(doc, fmt, cmyk, matte, MERGED_BAND_PIXELS, ctl)
}

/// Use the compositor's default band budget natively; keep browser working buffers smaller.
const MERGED_BAND_PIXELS: u64 = if cfg!(target_arch = "wasm32") { photocraft_compose::BAND_PIXELS / 8 } else { photocraft_compose::BAND_PIXELS };

/// Allocate merged planes with spare capacity for the document's saved channels.
fn plane_buffer(doc: &Document, plane: usize, channels: usize) -> Vec<u8> {
    let extra = doc.channels.len().saturating_add(usize::from(doc.quick_mask.is_some())).min(56 - channels);
    let mut out = photocraft_raster::scratch::take_cap(plane * (channels + extra));
    out.resize(plane * channels, 0);
    out
}

/// [`merged_planes`] (its composite path) rendered in bands of about `band_pixels`.
fn merged_planes_in_bands(doc: &Document, fmt: &PixelFormat, cmyk: bool, matte: bool, band_pixels: u64, ctl: &Interrupt) -> (Vec<u8>, bool, bool) {
    let sample = fmt.sample;
    let cc = fmt.mode.color_channels();
    let bps = sample.bytes();
    let n = doc.size.area() as usize;
    let plane = n * bps;
    let mut planes = plane_buffer(doc, plane, cc + 1);
    let (mut has_alpha, mut translucent) = (false, false);
    let white = photocraft_raster::from_rgba(fmt, [1.0, 1.0, 1.0, 1.0]);
    let canvas = doc.bounds();
    let w = canvas.width() as usize;
    let space = photocraft_compose::cmyk_space(doc);
    let lab16 = fmt.mode == ColorMode::Lab && sample == SampleType::U16;
    let rows = canvas.height().max(1) as f32;
    let band_rows = i32::try_from((band_pixels / u64::from(canvas.width().max(1))).max(1)).unwrap_or(i32::MAX);
    let _ = photocraft_compose::render_bands(doc, canvas, band_rows, |band| -> Result<(), ()> {
        if ctl.cancelled() {
            return Err(());
        }
        let start = (band.rect.y0 - canvas.y0) as usize * w;
        let ranges = crate::pixels::bands(band.px.len());
        let mut slots: Vec<Vec<&mut [u8]>> = ranges.iter().map(|_| Vec::with_capacity(cc + 1)).collect();
        for chunk in planes.chunks_mut(plane.max(1)).take(cc + 1) {
            let Some(mut rest) = chunk.get_mut(start * bps..(start + band.px.len()) * bps) else { continue };
            for (slot, range) in slots.iter_mut().zip(&ranges) {
                let Some((part, tail)) = rest.split_at_mut_checked(range.len() * bps) else { break };
                slot.push(part);
                rest = tail;
            }
        }
        // Converted and encoded on all cores. The composite came through the document's CMYK
        // profile: convert back through it too.
        let flags = crate::pixels::par_map_cpu(ranges.into_iter().zip(slots).collect(), |(range, mut out)| {
            let px = band.px.get(range).unwrap_or(&[]);
            let k = Encode { fmt, cc, matte, cmyk, lab16, white: &white };
            photocraft_color::convert::with_cmyk_space(space.as_ref(), || match sample {
                SampleType::U8 => k.run(
                    px,
                    &mut out,
                    |a| q255(a) < 255,
                    |x, d, i| {
                        if let Some(b) = d.get_mut(i) {
                            *b = (x.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                        }
                    },
                ),
                SampleType::U16 => k.run(
                    px,
                    &mut out,
                    |a| (a.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16 != u16::MAX,
                    |x, d, i| {
                        if let Some(b) = d.get_mut(i * 2..i * 2 + 2) {
                            b.copy_from_slice(&((x.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16).to_be_bytes());
                        }
                    },
                ),
                SampleType::F32 => k.run(
                    px,
                    &mut out,
                    |a| a != 1.0,
                    |x, d, i| {
                        if let Some(b) = d.get_mut(i * 4..i * 4 + 4) {
                            b.copy_from_slice(&x.to_be_bytes());
                        }
                    },
                ),
            })
        });
        for (a, t) in flags {
            has_alpha |= a;
            translucent |= t;
        }
        ctl.progress((band.rect.y1 - canvas.y0) as f32 / rows);
        Ok(())
    });
    (planes, has_alpha, translucent)
}

struct Encode<'a> {
    fmt: &'a PixelFormat,
    cc: usize,
    matte: bool,
    cmyk: bool,
    lab16: bool,
    white: &'a [f32],
}

impl Encode<'_> {
    fn run(&self, px: &[[f32; 4]], out: &mut [&mut [u8]], stored_translucent: impl Fn(f32) -> bool, put: impl Fn(f32, &mut [u8], usize)) -> (bool, bool) {
        let Self { fmt, cc, matte, cmyk, lab16, white } = *self;
        let (mut has_alpha, mut translucent) = (false, false);
        let mut v = [0.0f32; 5];
        for (i, p) in px.iter().enumerate() {
            // Retain alpha whenever it differs from opaque at the stored precision.
            has_alpha |= stored_translucent(p[3]);
            translucent |= p[3] < 1.0;
            photocraft_raster::from_rgba_into(fmt, *p, &mut v);
            for (c, dst) in out.iter_mut().enumerate() {
                // Matte against white like Photoshop (see `pixels::matte`).
                let m = if c < cc && matte { crate::pixels::matte(v[c], v[cc], white.get(c).copied().unwrap_or(1.0)) } else { v[c] };
                let x = if cmyk && c < cc { 1.0 - m } else { m };
                // 16-bit Lab a*/b* use Photoshop's 0..65280 scale (`pixels::lab16_chroma`).
                let x = if lab16 && (c == 1 || c == 2) { x * (crate::pixels::LAB16_CHROMA_MAX / 65535.0) } else { x };
                put(x, dst, i);
            }
        }
        (has_alpha, translucent)
    }
}

/// [`merged_planes`] of a CMYK document whose layers are mixed on their ink values
/// (`photocraft_compose::cmyk`), so opaque pixels keep their exact inks; `None` for documents
/// that composite in RGB. Reports progress and stops on cancel like [`merged_planes`].
fn merged_ink_planes(doc: &Document, sample: SampleType, matte: bool, ctl: &Interrupt) -> Option<(Vec<u8>, bool, bool)> {
    let bps = sample.bytes();
    let n = doc.size.area() as usize;
    let plane = n * bps;
    let mut planes = plane_buffer(doc, plane, 5);
    let (mut has_alpha, mut translucent) = (false, false);
    let canvas = doc.bounds();
    let w = canvas.width() as usize;
    let rows = canvas.height().max(1) as f32;
    // A cancelled export leaves the rest blank (the caller discards it), so the result stands either way.
    let band_rows = i32::try_from((MERGED_BAND_PIXELS / u64::from(canvas.width().max(1))).max(1)).unwrap_or(i32::MAX);
    let _ = photocraft_compose::cmyk::render_cmyk_bands(doc, canvas, band_rows, |band| -> Result<(), ()> {
        if ctl.cancelled() {
            return Err(());
        }
        let start = (band.rect.y0 - canvas.y0) as usize * w;
        let ranges = crate::pixels::bands(band.px.len());
        let mut slots: Vec<Vec<&mut [u8]>> = ranges.iter().map(|_| Vec::with_capacity(5)).collect();
        for chunk in planes.chunks_mut(plane.max(1)).take(5) {
            let Some(mut rest) = chunk.get_mut(start * bps..(start + band.px.len()) * bps) else { continue };
            for (slot, range) in slots.iter_mut().zip(&ranges) {
                let Some((part, tail)) = rest.split_at_mut_checked(range.len() * bps) else { break };
                slot.push(part);
                rest = tail;
            }
        }
        let flags = crate::pixels::par_map_cpu(ranges.into_iter().zip(slots).collect(), |(range, mut out)| {
            let px = band.px.get(range).unwrap_or(&[]);
            let (mut alpha, mut translucent) = (false, false);
            for (c, dst) in out.iter_mut().enumerate() {
                for (dst, p) in dst.chunks_exact_mut(bps).zip(px) {
                    // Inks matted against white paper (no ink) like Photoshop (see
                    // `pixels::matte`), and stored inverted as PSD's CMYK is.
                    let Some(&ink) = p.get(c) else { continue };
                    let v = if c < 4 { 1.0 - if matte { crate::pixels::matte(ink, p[4], 0.0) } else { ink } } else { p[4] };
                    match sample {
                        SampleType::U8 => dst.copy_from_slice(&[(v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8]),
                        SampleType::U16 => dst.copy_from_slice(&((v.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16).to_be_bytes()),
                        SampleType::F32 => dst.copy_from_slice(&v.to_be_bytes()),
                    }
                    if c == 4 {
                        alpha |= match sample {
                            SampleType::U8 => q255(p[4]) < 255,
                            SampleType::U16 => (p[4].clamp(0.0, 1.0) * 65535.0 + 0.5) as u16 != u16::MAX,
                            SampleType::F32 => p[4] != 1.0,
                        };
                        translucent |= p[4] < 1.0;
                    }
                }
            }
            (alpha, translucent)
        });
        for (alpha, trans) in flags {
            has_alpha |= alpha;
            translucent |= trans;
        }
        ctl.progress((band.rect.y1 - canvas.y0) as f32 / rows);
        Ok(())
    })?;
    Some((planes, has_alpha, translucent))
}

/// The merged image of a file saved without Maximize Compatibility: Photoshop's white placeholder
/// in the document's colour channels, with no alpha. Encoded like [`merged_planes`], so CMYK white
/// is no ink and Lab white is L 100 with neutral a*/b*.
pub(super) fn placeholder_planes(doc: &Document, fmt: &PixelFormat) -> Vec<u8> {
    let cc = fmt.mode.color_channels();
    let cmyk = cmyk_of(fmt);
    let lab16 = fmt.mode == ColorMode::Lab && fmt.sample == SampleType::U16;
    let space = photocraft_compose::cmyk_space(doc);
    let white = photocraft_color::convert::with_cmyk_space(space.as_ref(), || photocraft_raster::from_rgba(fmt, [1.0, 1.0, 1.0, 1.0]));
    let n = doc.size.area() as usize;
    let mut planes = Vec::with_capacity(n * fmt.sample.bytes() * cc);
    for c in 0..cc {
        let v = white.get(c).copied().unwrap_or(1.0);
        let x = if cmyk { 1.0 - v } else { v };
        let x = if lab16 && (c == 1 || c == 2) { x * (crate::pixels::LAB16_CHROMA_MAX / 65535.0) } else { x };
        let mut sample = Vec::with_capacity(4);
        encode_be(x, fmt.sample, &mut sample);
        planes.extend(sample.iter().copied().cycle().take(n * sample.len()));
    }
    planes
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{Layer, LayerContent};
    use photocraft_raster::Surface;

    /// Verify reserved saved-channel capacity avoids moving completed merged planes.
    #[test]
    fn saved_channels_fit_without_moving_the_merged_planes() {
        let mut doc = Document::new("extra", photocraft_geom::Size::new(13, 7), ColorMode::Cmyk, SampleType::F32);
        doc.channels.push(photocraft_doc::AlphaChannel::new("saved", Surface::new(PixelFormat::GRAY8)));
        doc.quick_mask = Some(photocraft_doc::AlphaChannel::new("quick", Surface::new(PixelFormat::GRAY8)));
        let plane = doc.size.area() as usize * 4;
        let mut bytes = plane_buffer(&doc, plane, 5);
        let ptr = bytes.as_ptr();
        bytes.resize(plane * 7, 0);
        assert_eq!(bytes.as_ptr(), ptr);
    }

    /// Render merged planes with the previous staged conversion as a byte oracle.
    fn previous_merged_planes(doc: &Document, fmt: &PixelFormat, cmyk: bool, matte: bool) -> (Vec<u8>, bool, bool) {
        if cmyk {
            let mut planes = vec![Vec::new(); 5];
            let (mut alpha, mut translucent) = (false, false);
            if photocraft_compose::cmyk::render_cmyk_bands(doc, doc.bounds(), 0, |band| -> Result<(), ()> {
                for p in &band.px {
                    alpha |= match fmt.sample {
                        SampleType::U8 => q255(p[4]) < 255,
                        SampleType::U16 => (p[4].clamp(0.0, 1.0) * 65535.0 + 0.5) as u16 != u16::MAX,
                        SampleType::F32 => p[4] != 1.0,
                    };
                    translucent |= p[4] < 1.0;
                    for (c, out) in planes.iter_mut().enumerate() {
                        let v = if c < 4 { 1.0 - if matte { crate::pixels::matte(p[c], p[4], 0.0) } else { p[c] } } else { p[4] };
                        encode_be(v, fmt.sample, out);
                    }
                }
                Ok(())
            })
            .is_some()
            {
                return (planes.concat(), alpha, translucent);
            }
        }
        let sample = fmt.sample;
        let cc = fmt.mode.color_channels();
        let bps = sample.bytes();
        let plane = doc.size.area() as usize * bps;
        let mut planes = vec![0u8; plane * (cc + 1)];
        let (mut has_alpha, mut translucent) = (false, false);
        let white = photocraft_raster::from_rgba(fmt, [1.0, 1.0, 1.0, 1.0]);
        let canvas = doc.bounds();
        let w = canvas.width() as usize;
        let space = photocraft_compose::cmyk_space(doc);
        let lab16 = fmt.mode == ColorMode::Lab && sample == SampleType::U16;
        let _ = photocraft_compose::render_bands(doc, canvas, 0, |band| -> Result<(), ()> {
            has_alpha |= band.px.iter().any(|p| match sample {
                SampleType::U8 => q255(p[3]) < 255,
                SampleType::U16 => (p[3].clamp(0.0, 1.0) * 65535.0 + 0.5) as u16 != u16::MAX,
                SampleType::F32 => p[3] != 1.0,
            });
            translucent |= band.px.iter().any(|p| p[3] < 1.0);
            let start = (band.rect.y0 - canvas.y0) as usize * w;
            photocraft_color::convert::with_cmyk_space(space.as_ref(), || {
                let mut v = [0.0f32; 5];
                for (i, p) in band.px.iter().enumerate() {
                    photocraft_raster::from_rgba_into(fmt, *p, &mut v);
                    for c in 0..=cc {
                        let m = if c < cc && matte { crate::pixels::matte(v[c], v[cc], white[c]) } else { v[c] };
                        let x = if cmyk && c < cc { 1.0 - m } else { m };
                        let x = if lab16 && (c == 1 || c == 2) { x * (crate::pixels::LAB16_CHROMA_MAX / 65535.0) } else { x };
                        let mut b = Vec::new();
                        encode_be(x, sample, &mut b);
                        let o = c * plane + (start + i) * bps;
                        planes[o..o + bps].copy_from_slice(&b);
                    }
                }
            });
            Ok(())
        });
        (planes, has_alpha, translucent)
    }

    #[test]
    fn merged_planes_match_the_per_band_reference() {
        let size = photocraft_geom::Size::new(37, 23);
        for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Lab, ColorMode::Cmyk] {
            for sample in SampleType::ALL {
                for alpha_floor in [0.0f32, 0.999_9, 1.0] {
                    let mut doc = Document::new("merged", size, mode, sample);
                    let fmt = doc.pixel_format();
                    let mut s = Surface::new(fmt);
                    for y in 0..23 {
                        for x in 0..37 {
                            let a = (alpha_floor + (1.0 - alpha_floor) * ((x * 7 + y * 3) % 11) as f32 / 10.0).min(1.0);
                            let rgba = [x as f32 / 36.0, y as f32 / 22.0, ((x + y) % 5) as f32 / 4.0, a];
                            let mut v = [0.0f32; 5];
                            let n = photocraft_raster::from_rgba_into(&fmt, rgba, &mut v);
                            s.write_pixel(x, y, &v[..n]);
                        }
                    }
                    doc.layers.push(Layer::new("pixels", LayerContent::Raster(s)));
                    for matte in [false, true] {
                        let got = merged_planes(&doc, &fmt, cmyk_of(&fmt), matte, &Interrupt::NONE);
                        let want = previous_merged_planes(&doc, &fmt, cmyk_of(&fmt), matte);
                        assert_eq!(got, want, "{mode:?} {sample:?} alpha >= {alpha_floor} matte {matte}");
                    }
                }
            }
        }
    }

    /// Shadows and translucent pixels must encode identically across band boundaries.
    #[test]
    fn the_merged_image_is_the_same_in_any_band_height() {
        use photocraft_doc::{Effect, Shadow};
        let mut doc = Document::new("bands", photocraft_geom::Size::new(300, 1100), ColorMode::Rgb, SampleType::U8);
        let fmt = photocraft_color::PixelFormat::RGBA8;
        for (i, y) in [240, 500, 760].into_iter().enumerate() {
            let mut l = Layer::raster(format!("blob {i}"), fmt);
            let (x, f) = (60 * i as i32, i as f32);
            l.surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(30 + x, y, 150 + x, y + 40), &[0.8, 0.2 * f, 0.1, 0.6 + 0.2 * f]);
            let shadow = |distance: f32, size: f32| match Effect::default_drop_shadow() {
                Effect::DropShadow(s) => Effect::DropShadow(Shadow { distance, size, angle: 90.0, use_global_light: false, ..s }),
                other => other,
            };
            l.effects.items = vec![shadow(24.0, 18.0), shadow(9.0, 40.0)];
            doc.layers.push(l);
        }
        let dfmt = doc.pixel_format();
        let tall = merged_planes_in_bands(&doc, &dfmt, false, true, photocraft_compose::BAND_PIXELS, &Interrupt::NONE);
        assert!(photocraft_compose::band_rows_for(300, (photocraft_compose::BAND_PIXELS / 300) as i32) >= 1100, "one band");
        for band_pixels in [1, 300 * 256, 300 * 512, MERGED_BAND_PIXELS] {
            let banded = merged_planes_in_bands(&doc, &dfmt, false, true, band_pixels, &Interrupt::NONE);
            assert!(banded == tall, "{band_pixels} px bands changed the merged image");
        }
        assert!(tall.1 && tall.2, "the test image has translucent pixels");
    }
}
