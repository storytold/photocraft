//! Rendering of [`LayerContent::Deep`]: depth-compositing a variable-length sample list per
//! pixel. The math mirrors the codecs-side deep-EXR flatten (samples sorted by Z, front-to-back
//! `over` with premultiplied colour, per-channel alpha), but renders straight into the
//! compositor's premultiplied buffers over a region, and into a straight-alpha surface for
//! proxies and exports.

use photocraft_color::{ColorMode, PixelFormat};
use photocraft_doc::{DeepData, LayerContent, Surface};
use photocraft_geom::Rect;

use crate::Buffer;

/// The channels a deep layer composites from, resolved once per layer: `R`/`G`/`B` or `Y`
/// luminance (falling back to the only channel), alpha as `A` or per-channel `AR`/`AG`/`AB`,
/// and the depth `Z`.
struct DeepChannels<'a> {
    rgb: Option<[&'a [f32]; 3]>,
    lum: &'a [f32],
    a: &'a [f32],
    ar: &'a [f32],
    ag: &'a [f32],
    ab: &'a [f32],
    z: &'a [f32],
}

fn resolve(d: &DeepData) -> Option<DeepChannels<'_>> {
    let samples = |n: &str| -> &[f32] { d.channel(n).map(|c| c.samples.as_slice()).unwrap_or(&[]) };
    let r = samples("R");
    let g = samples("G");
    let b = samples("B");
    let rgb: Option<[&[f32]; 3]> = (!r.is_empty() && !g.is_empty() && !b.is_empty()).then_some([r, g, b]);
    let lum = samples("Y");
    // A single channel that is neither colour nor alpha composites as luminance (a depth-only
    // file previews as its depth); with no channel to draw from there is nothing to render.
    let lum = if lum.is_empty() { d.channels.first().map(|c| c.samples.as_slice()).unwrap_or(&[]) } else { lum };
    if rgb.is_none() && lum.is_empty() {
        return None;
    }
    Some(DeepChannels { rgb, lum, a: samples("A"), ar: samples("AR"), ag: samples("AG"), ab: samples("AB"), z: samples("Z") })
}

/// Per-colour-channel alpha of sample `i`: `AR`/`AG`/`AB` when present, else `A`, else opaque.
fn alpha(c: &DeepChannels, base: u64, i: u64) -> [f32; 3] {
    let at = |ch: &[f32]| ch.get((base + i) as usize).copied().unwrap_or(0.0);
    let s = at(c.a);
    let pick = |per: &[f32], shared: f32| {
        if !c.a.is_empty() && per.is_empty() {
            shared
        } else if !per.is_empty() {
            at(per)
        } else {
            1.0
        }
    };
    [pick(c.ar, s), pick(c.ag, s), pick(c.ab, s)]
}

/// The composited premultiplied colour and alpha of one pixel's samples.
/// `order` is scratch space reused across pixels.
fn composite_pixel(d: &DeepData, c: &DeepChannels, pixel: usize, order: &mut Vec<u32>) -> Option<([f32; 3], f32)> {
    let range = d.sample_range(pixel);
    let (base, n) = (range.start, range.end.saturating_sub(range.start));
    if n == 0 {
        return None; // no samples: transparent
    }
    order.clear();
    order.extend(0u32..n as u32);
    if !c.z.is_empty() {
        // Stable sort by Z; a NaN depth (invalid) sorts last, deterministically.
        order.sort_by(|&i, &j| {
            let zi = c.z.get((base + u64::from(i)) as usize).copied().unwrap_or(f32::NAN);
            let zj = c.z.get((base + u64::from(j)) as usize).copied().unwrap_or(f32::NAN);
            zi.total_cmp(&zj)
        });
    }
    let at = |ch: &[f32], i: u32| ch.get((base + u64::from(i)) as usize).copied().unwrap_or(0.0);
    let mut cacc = [0f32; 3]; // premultiplied colour
    let mut aacc = [0f32; 3];
    let mut covered = true;
    for &i in order.iter() {
        let ai = alpha(c, base, u64::from(i));
        let src = match c.rgb {
            Some([r, g, b]) => [at(r, i), at(g, i), at(b, i)],
            None => [at(c.lum, i); 3],
        };
        for k in 0..3 {
            let t = (1.0 - aacc[k]).max(0.0);
            cacc[k] += src[k] * t;
            aacc[k] += t * ai[k];
            covered &= aacc[k] >= 1.0;
        }
        if covered {
            break; // everything behind is hidden
        }
    }
    let a = if c.a.is_empty() && c.ar.is_empty() && c.ag.is_empty() && c.ab.is_empty() { 1.0 } else { (aacc[0] + aacc[1] + aacc[2]) / 3.0 };
    Some((cacc, a.clamp(0.0, 1.0)))
}

/// The layer's premultiplied pixels over `rect`, in document coordinates. Pixels outside the
/// data extent are transparent.
pub fn render(d: &DeepData, rect: Rect) -> Buffer {
    let mut buf = Buffer::transparent(rect);
    let Some(c) = resolve(d) else { return buf };
    let data = Rect::from_xywh(d.x, d.y, d.width, d.height);
    let clip = rect.intersect(&data);
    if clip.is_empty() {
        return buf;
    }
    let w = rect.width() as usize;
    let dw = d.width as usize;
    let mut order = Vec::new();
    for y in clip.y0..clip.y1 {
        for x in clip.x0..clip.x1 {
            let pixel = (y - rect.y0) as usize * w + (x - rect.x0) as usize;
            let data_px = ((y - d.y) as usize) * dw + (x - d.x) as usize;
            if let Some((cacc, a)) = composite_pixel(d, &c, data_px, &mut order)
                && let Some(px) = buf.px.get_mut(pixel)
            {
                *px = [cacc[0], cacc[1], cacc[2], a];
            }
        }
    }
    buf
}

/// The data composited once into a straight-alpha surface of `fmt` (the document's pixel
/// format), for proxies, exports and PSD materialization. The surface spans the data extent.
pub fn flat_surface(d: &DeepData, fmt: PixelFormat) -> Result<Surface, String> {
    // RGB and grayscale keep their meaning; converting into a CMYK or Lab document needs the
    // document's profile, which this call does not carry — refuse instead of writing
    // red-as-cyan and alpha-into-black.
    if !matches!(fmt.mode, ColorMode::Rgb | ColorMode::Grayscale) {
        return Err(format!("deep layers render to {:?} documents only", fmt.mode));
    }
    // Checked: a hostile `.pcraft` can claim huge deep dimensions, and the byte size must
    // not overflow (it does at ~4 GiB on wasm32, where usize is 32 bits).
    let npx = usize::try_from(d.width)
        .ok()
        .and_then(|w| usize::try_from(d.height).ok().and_then(|h| w.checked_mul(h)))
        .ok_or_else(|| "the deep data extent is too large to render".to_string())?;
    let nc = fmt.mode.color_channels();
    let per_pixel = nc.checked_add(1).ok_or_else(|| "too many channels to render".to_string())?;
    // f32 samples; cap the flat buffer at 2^31 bytes so no platform overflows the allocation.
    let flat = npx
        .checked_mul(per_pixel)
        .filter(|n| n.checked_mul(4).is_some_and(|b| b <= i32::MAX as usize))
        .ok_or_else(|| "the deep data extent is too large to render".to_string())?;
    let transparent = vec![0.0; nc + 1];
    let mut s = Surface::with_default(fmt, &transparent);
    let Some(c) = resolve(d) else { return Ok(s) };
    let gray = c.rgb.is_none();
    let mut vals = Vec::with_capacity(flat);
    let mut order = Vec::new();
    for p in 0..npx {
        match composite_pixel(d, &c, p, &mut order) {
            Some((cacc, a)) => {
                // Un-premultiply into straight samples (the deep colours are premultiplied).
                let straight = |v: f32| if a > 1e-6 { v / a } else { 0.0 };
                let px = [straight(cacc[0]), straight(cacc[1]), straight(cacc[2]), a];
                for k in 0..nc {
                    vals.push(if gray { px[0] } else { px[k] });
                }
                vals.push(a);
            }
            None => vals.extend(std::iter::repeat_n(0.0, nc + 1)),
        }
    }
    s.write_region(Rect::from_xywh(d.x, d.y, d.width, d.height), &vals);
    Ok(s)
}

/// `true` if the document holds any deep layer (canvas ops that rewrite pixels in place would
/// desync deep data; the GPU plan declines these documents).
pub fn any_deep(layers: &[photocraft_doc::Layer]) -> bool {
    layers.iter().any(|l| match &l.content {
        LayerContent::Deep(_) => true,
        LayerContent::Group(g) => any_deep(&g.children),
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::PixelFormat;
    use photocraft_doc::DeepChannel;

    /// 1×1 deep data whose only pixel holds two unsorted samples: red (a 0.5, z 1) in front of
    /// blue (a 1.0, z 9). Premultiplied over: (0.5, 0, 0.5, 1.0).
    fn two_samples() -> DeepData {
        let ch = |name: &str, v: Vec<f32>| DeepChannel { name: name.to_string(), samples: v };
        DeepData {
            x: 0,
            y: 0,
            width: 1,
            height: 2,
            channels: vec![
                ch("A", vec![1.0, 0.5, 0.5]),
                ch("B", vec![1.0, 0.0, 0.0]),
                ch("G", vec![0.0, 0.0, 0.0]),
                ch("R", vec![0.0, 0.25, 0.0]),
                ch("Z", vec![9.0, 1.0, 5.0]),
            ],
            counts: vec![0, 2, 3],
        }
    }

    #[test]
    fn sorts_by_z_and_overs_front_to_back() {
        let d = two_samples();
        let buf = render(&d, Rect::from_xywh(0, 0, 1, 2));
        let front = buf.px[0];
        for (got, want) in front.iter().zip([0.25, 0.0, 0.5, 1.0]) {
            assert!((got - want).abs() < 1e-6, "{front:?}");
        }
        // The second pixel has one opaque blue sample behind nothing.
        let back = buf.px[1];
        for (got, want) in back.iter().zip([0.0, 0.0, 0.0, 0.5]) {
            assert!((got - want).abs() < 1e-6, "{back:?}");
        }
    }

    #[test]
    fn origin_places_the_data_on_the_canvas() {
        let mut d = two_samples();
        d.x = 1;
        d.y = 1;
        let buf = render(&d, Rect::from_xywh(0, 0, 2, 2));
        assert_eq!(buf.px[0], [0.0; 4], "outside the data: transparent");
        assert!((buf.px[3][2] - 0.5).abs() < 1e-6, "{:?}", buf.px[3]);
    }

    #[test]
    fn empty_pixels_are_transparent() {
        let d = DeepData { x: 0, y: 0, width: 1, height: 1, channels: vec![DeepChannel { name: "Z".into(), samples: vec![] }], counts: vec![0, 0] };
        let buf = render(&d, Rect::from_xywh(0, 0, 1, 1));
        assert_eq!(buf.px[0], [0.0; 4]);
    }

    #[test]
    fn flat_surface_un_premultiplies() {
        let d = two_samples();
        let fmt = PixelFormat::new(photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::F32, true);
        let s = flat_surface(&d, fmt).expect("rgb flattens");
        let px = s.pixel(0, 0);
        // Straight alpha: the half-covered red front over opaque blue: c = (0.25,0,0.5), a = 1.
        assert!((px[0] - 0.25).abs() < 1e-6 && (px[2] - 0.5).abs() < 1e-6 && (px[3] - 1.0).abs() < 1e-6, "{px:?}");
        let px = s.pixel(0, 1);
        assert!((px[3] - 0.5).abs() < 1e-6, "{px:?}");
    }
}
