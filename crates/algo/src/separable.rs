//! Separable blurs described as data, so another device (the GPU compositor's layer operators,
//! #5772) runs exactly the blur the CPU runs: the kernel weights or box widths come from here,
//! and [`crate::blur`] builds its own passes from the same description, so the two can't drift.
//!
//! [`OperatorRegion`] is the region rule that goes with it: which source pixels an output area
//! reads, with the edge pixels repeated at the extent as the CPU filters do.

use photocraft_geom::Rect;

use crate::{FilterParams, blur};

/// A neighbourhood filter as the CPU runs it, as data: a blur made of passes along each axis
/// (horizontal first), or one of the other window filters the GPU's layer operators run.
#[derive(Clone, Debug, PartialEq)]
pub enum Separable {
    /// One pass of this normalised kernel (odd length, centred) along each axis.
    Kernel(Vec<f32>),
    /// Running-mean box passes of these odd widths, in turn, along each axis.
    Boxes(Vec<usize>),
    /// Minimum or Maximum over a square (`other::min_max`): per channel, alpha included, of the
    /// straight values, the extreme over `x - radius ..= x + radius` along each axis in turn.
    Extreme { radius: u32, max: bool },
    /// Minimum or Maximum over a disc (`other::min_max_round`): per channel of the straight
    /// values, the extreme over the window of half-width `half_widths[dy + r]` on each row
    /// `dy` in `-r ..= r` (`r = half_widths.len() / 2`).
    Disc { half_widths: Vec<u32>, max: bool },
    /// Custom (`other2::custom`): per colour channel of the straight values, the sum of
    /// `weight * pixel(x + dx, y + dy)` over `taps` in order, divided by `scale` and offset by
    /// `offset` (a fraction of 1); alpha stays.
    Convolve { taps: Vec<(i32, i32, f32)>, scale: f32, offset: f32 },
}

impl Separable {
    /// The blur `params` runs on the CPU, or `None` for a filter that isn't one (Gaussian Blur,
    /// Box Blur, and the blur inside Unsharp Mask and High Pass). Gaussian Blur's small radii use
    /// the exact kernel and larger ones three box passes; Box Blur's small radii a uniform kernel
    /// and larger ones one box pass; Unsharp Mask and High Pass always blur with the exact kernel
    /// (`sharpen`).
    pub fn of(params: &FilterParams) -> Option<Self> {
        match *params {
            FilterParams::GaussianBlur { radius } => Some(match blur::box_widths(params) {
                Some(boxes) => Separable::Boxes(boxes),
                None => Separable::Kernel(blur::gaussian_kernel(radius)),
            }),
            FilterParams::UnsharpMask { radius, .. } | FilterParams::HighPass { radius } if radius.is_finite() && radius >= 0.0 => {
                Some(Separable::Kernel(blur::gaussian_kernel(radius)))
            }
            // `blur::boxed`.
            FilterParams::BoxBlur { radius } if radius.is_finite() => Some(match blur::box_widths(params) {
                Some(boxes) => Separable::Boxes(boxes),
                None => {
                    let r = radius.max(0.0).round() as usize;
                    Separable::Kernel(vec![1.0 / (2 * r + 1) as f32; 2 * r + 1])
                }
            }),
            // RGB and Gray only: CMYK's Minimum/Maximum and Custom work on inverted inks.
            FilterParams::Minimum { radius, preserve } | FilterParams::Maximum { radius, preserve } if radius.is_finite() => {
                let max = matches!(params, FilterParams::Maximum { .. });
                Some(match preserve {
                    crate::Preserve::Squareness => Separable::Extreme { radius: radius.max(0.0).round() as u32, max },
                    crate::Preserve::Roundness => Separable::Disc { half_widths: crate::other::disc_half_widths(radius), max },
                })
            }
            FilterParams::Custom { ref kernel, scale, offset } if scale.is_finite() && offset.is_finite() && kernel.iter().all(|w| w.is_finite()) => {
                Some(Separable::Convolve { taps: crate::other2::custom_taps(kernel), scale: if scale == 0.0 { 1.0 } else { scale }, offset: offset / 255.0 })
            }
            _ => None,
        }
    }

    /// How far outside an output pixel the filter reads, along each axis.
    pub fn reach(&self) -> i32 {
        let reach = match self {
            Separable::Kernel(k) => k.len() / 2,
            Separable::Boxes(boxes) => boxes.iter().fold(0usize, |sum, w| sum.saturating_add(w / 2)),
            Separable::Extreme { radius, .. } => *radius as usize,
            Separable::Disc { half_widths, .. } => half_widths.iter().copied().max().unwrap_or(0).max((half_widths.len() / 2) as u32) as usize,
            Separable::Convolve { taps, .. } => taps.iter().map(|&(dx, dy, _)| dx.unsigned_abs().max(dy.unsigned_abs())).max().unwrap_or(0) as usize,
        };
        i32::try_from(reach).unwrap_or(i32::MAX)
    }
}

/// A distort filter's mapping as data (`distort::remap`): each output pixel samples the source at
/// the point the mapping gives, so another device runs exactly the CPU's mapping. Whatever the
/// CPU works out once per filter (Wave's seeded generators, Shear's row offsets, Twirl's angle in
/// radians) comes ready-made, from the same helpers the CPU filters use.
#[derive(Clone, Debug, PartialEq)]
pub enum Warp {
    /// `distort::twirl`: the angle in radians.
    Twirl { angle: f32 },
    /// `distort::pinch`: Amount / 100, clamped to -1..=1.
    Pinch { amount: f32 },
    /// `distort::spherize`: Amount / 100 (clamped), and the mode (0 Normal, 1 Horizontal Only,
    /// 2 Vertical Only).
    Spherize { amount: f32, mode: u32 },
    /// `distort::wave`: the generators `[wavelength, amplitude, phase x, phase y]` and the type
    /// (0 Sine, 1 Triangle, 2 Square).
    Wave { generators: Vec<[f32; 4]>, wave_type: u32 },
    /// `distort::ripple`: the amplitude and wavelength in pixels.
    Ripple { amplitude: f32, wavelength: f32 },
    /// `distort::polar`: Rectangular to Polar, else Polar to Rectangular.
    Polar { to_polar: bool },
    /// `distort2::zigzag`: Amount / 100 (clamped), the ridges (clamped), and the style (0 Around
    /// Center, 1 Out From Center, 2 Pond Ripples).
    ZigZag { amount: f32, ridges: f32, style: u32 },
    /// `distort2::shear`: the horizontal offset of each row of the bounds, top to bottom.
    Shear { rows: Vec<f32> },
}

impl Warp {
    /// The mapping `params` runs over `bounds` on the CPU, and the edge its samples use past the
    /// bounds; `None` for a filter that isn't a mapping (or Displace, whose map the caller has).
    pub fn of(params: &FilterParams, bounds: Rect) -> Option<(Self, crate::Edge)> {
        use crate::Edge;
        Some(match params {
            FilterParams::Twirl { angle } => (Warp::Twirl { angle: angle.to_radians() }, Edge::Repeat),
            FilterParams::Pinch { amount } => (Warp::Pinch { amount: (amount / 100.0).clamp(-1.0, 1.0) }, Edge::Repeat),
            FilterParams::Spherize { amount, mode } => {
                let mode = match mode {
                    crate::SpherizeMode::Normal => 0,
                    crate::SpherizeMode::HorizontalOnly => 1,
                    crate::SpherizeMode::VerticalOnly => 2,
                };
                (Warp::Spherize { amount: (amount / 100.0).clamp(-1.0, 1.0), mode }, Edge::Repeat)
            }
            FilterParams::Wave { generators, wavelength_min, wavelength_max, amplitude_min, amplitude_max, wave_type, undefined, seed } => {
                let spec = crate::distort::WaveSpec {
                    generators: *generators,
                    wavelength: (*wavelength_min, *wavelength_max),
                    amplitude: (*amplitude_min, *amplitude_max),
                    wave_type: *wave_type,
                    undefined: *undefined,
                    seed: *seed,
                };
                let generators = crate::distort::wave_generators(&spec).into_iter().map(|(l, a, p1, p2)| [l, a, p1, p2]).collect();
                let wave_type = match wave_type {
                    crate::WaveType::Sine => 0,
                    crate::WaveType::Triangle => 1,
                    crate::WaveType::Square => 2,
                };
                (Warp::Wave { generators, wave_type }, crate::other::edge_of(*undefined))
            }
            FilterParams::Ripple { amount, size } => {
                let wavelength = match size {
                    crate::RippleSize::Small => 8.0,
                    crate::RippleSize::Medium => 16.0,
                    crate::RippleSize::Large => 32.0,
                };
                (Warp::Ripple { amplitude: amount / 100.0 * wavelength / 8.0, wavelength }, Edge::Repeat)
            }
            FilterParams::PolarCoordinates { mode } => (Warp::Polar { to_polar: matches!(mode, crate::PolarMode::RectangularToPolar) }, Edge::Repeat),
            FilterParams::ZigZag { amount, ridges, style } => {
                let style = match style {
                    crate::ZigZagStyle::AroundCenter => 0,
                    crate::ZigZagStyle::OutFromCenter => 1,
                    crate::ZigZagStyle::PondRipples => 2,
                };
                (Warp::ZigZag { amount: (amount / 100.0).clamp(-1.0, 1.0), ridges: ridges.clamp(0.0, 20.0).max(0.5), style }, Edge::Repeat)
            }
            FilterParams::Shear { points, undefined } => {
                (Warp::Shear { rows: crate::distort2::shear_rows(points, bounds, bounds.y0, bounds.y1) }, crate::other::edge_of(*undefined))
            }
            _ => return None,
        })
    }
}

/// An iris's rows: `(dy, dx0, dx1)`, inclusive.
pub type IrisRows = Vec<(i32, i32, i32)>;

/// Lens Blur as data (`blur2::lens_blur`): the iris kernels and the settings, clamped as the CPU
/// clamps them, so another device runs exactly the CPU's Lens Blur.
#[derive(Clone, Debug, PartialEq)]
pub struct Lens {
    /// The iris at each depth level (no blur at 0, the full radius at the last): its rows
    /// `(dy, dx0, dx1)` (inclusive) and how many pixels it covers.
    pub levels: Vec<(IrisRows, f64)>,
    /// Specular Highlights: the boost (Brightness / 100 × 4) above the threshold (a fraction).
    pub boost: f32,
    pub threshold: f32,
    /// Where depth comes from: `None`, the layer's transparency, or a map the caller supplies
    /// (the layer mask or an alpha channel).
    pub depth: crate::DepthSource,
    pub invert: bool,
    /// Blur Focal Distance as a fraction.
    pub focal: f32,
    /// The noise amount (Amount / 100 × 0.25), Gaussian or uniform, monochromatic, its seed.
    pub noise: f32,
    pub gaussian: bool,
    pub mono: bool,
    pub seed: u32,
}

impl Lens {
    /// Lens Blur's description, or `None` for another filter.
    pub fn of(params: &FilterParams) -> Option<Self> {
        let FilterParams::LensBlur {
            radius,
            blades,
            curvature,
            rotation,
            depth,
            focal_distance,
            invert_depth,
            brightness,
            threshold,
            noise,
            distribution,
            monochromatic,
            seed,
            ..
        } = params
        else {
            return None;
        };
        if ![*radius, *curvature, *rotation, *focal_distance, *brightness, *threshold, *noise].iter().all(|v| v.is_finite()) {
            return None;
        }
        let levels = crate::blur2::lens_levels(*radius, *blades, *curvature, *rotation).into_iter().map(|s| (s.rows, s.count)).collect();
        Some(Lens {
            levels,
            boost: brightness.clamp(0.0, 100.0) / 100.0 * 4.0,
            threshold: threshold.clamp(0.0, 255.0) / 255.0,
            depth: *depth,
            invert: *invert_depth,
            focal: focal_distance.clamp(0.0, 255.0) / 255.0,
            noise: noise.clamp(0.0, 100.0) / 100.0 * 0.25,
            gaussian: matches!(distribution, crate::Distribution::Gaussian),
            mono: *monochromatic,
            seed: *seed,
        })
    }

    /// How far the largest iris reaches.
    pub fn reach(&self) -> i32 {
        self.levels.iter().flat_map(|(rows, _)| rows.iter().map(|&(dy, a, b)| dy.abs().max(a.abs()).max(b.abs()))).max().unwrap_or(0)
    }
}

/// The area a neighbourhood filter on a layer repeats its edge pixels at: the canvas, or past it
/// where the layer has pixels off the canvas, as Photoshop does.
pub fn edge_extent(canvas: Rect, content: Rect) -> Rect {
    canvas.union(&content)
}

/// Which source pixels an output area of a blur reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OperatorRegion {
    /// How far the blur reads outside an output pixel ([`Separable::reach`]).
    pub reach: i32,
    /// Where the source's edge pixels repeat ([`edge_extent`]): nothing outside it is read.
    pub extent: Rect,
}

impl OperatorRegion {
    /// The source pixels output `out` reads: `out` grown by the reach, inside the extent. Pixels
    /// it needs beyond the extent repeat the edge of this area, so they need no source of their
    /// own. Empty when `out` is empty or doesn't come within reach of the extent.
    pub fn input(&self, out: Rect) -> Rect {
        if out.is_empty() {
            return Rect::EMPTY;
        }
        out.inflate(self.reach.max(0)).intersect(&self.extent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Halo;
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_raster::{Interrupt, Surface};

    const RADII: [f32; 9] = [0.01, 0.3, 0.8, 2.0, 4.0, 4.5, 12.0, 40.0, 250.0];

    /// The CPU blur's own choice: the exact kernel up to radius 4, boxes beyond; and its halo
    /// (how far the CPU grows an output area) covers the reach.
    #[test]
    fn the_description_is_the_cpus_blur() {
        for radius in RADII {
            let params = FilterParams::GaussianBlur { radius };
            let blur = Separable::of(&params).unwrap();
            match &blur {
                Separable::Kernel(k) => {
                    assert!(radius <= 4.0, "radius {radius}");
                    assert_eq!(k, &blur::gaussian_kernel(radius));
                    assert_eq!(k.len() % 2, 1);
                    assert!((k.iter().sum::<f32>() - 1.0).abs() < 1e-5);
                }
                Separable::Boxes(b) => {
                    assert!(radius > 4.0, "radius {radius}");
                    assert_eq!(Some(b.clone()), blur::box_widths(&params));
                    assert!(b.iter().all(|w| w % 2 == 1));
                }
                other => panic!("radius {radius}: not a blur: {other:?}"),
            }
            let Halo::Radius(halo) = params.halo() else { panic!("a Gaussian has a radius halo") };
            assert!(blur.reach() <= halo, "radius {radius}: reach {} past the halo {halo}", blur.reach());
        }
        assert_eq!(Separable::of(&FilterParams::Median { radius: 9.0 }), None, "not a separable blur");
        assert_eq!(Separable::Boxes(vec![usize::MAX, 3]).reach(), i32::MAX, "saturates");
    }

    /// One opaque pixel in a transparent layer, blurred on the CPU, spreads no further than the
    /// reach (beyond it the box passes' running sums leave only float residue, far below a
    /// 16-bit level), and the exact kernel spreads exactly that far.
    #[test]
    fn an_impulse_spreads_no_further_than_the_reach() {
        for radius in RADII {
            let params = FilterParams::GaussianBlur { radius };
            let reach = Separable::of(&params).unwrap().reach();
            let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::F32, true);
            let size = 2 * reach + 41;
            let canvas = Rect::new(0, 0, size, size);
            let mut s = Surface::new(fmt);
            let dot = Rect::new(size / 2, size / 2, size / 2 + 1, size / 2 + 1);
            s.fill_rect(dot, &[1.0, 1.0, 1.0, 1.0]);
            let out = crate::apply_in_with(&s, &params, canvas, canvas, None, canvas, &Interrupt::NONE).unwrap();
            let mut px = Vec::new();
            out.read_region_into(canvas, &mut px);
            let touched = px
                .as_chunks::<4>()
                .0
                .iter()
                .enumerate()
                // The box passes' running sums leave float residue (~1e-8) after the impulse.
                .filter(|(_, p)| p[3] > 1e-6)
                .fold(Rect::EMPTY, |r, (i, _)| {
                    let (x, y) = ((i as i32) % size, (i as i32) / size);
                    r.union(&Rect::new(x, y, x + 1, y + 1))
                });
            // Nothing past the reach; the exact kernel reaches all of it. (A single pixel's box
            // tails are smaller than the running sums' residue, so they aren't checked for that.)
            assert!(dot.inflate(reach).contains_rect(&touched), "radius {radius}: {touched:?} past the reach {reach}");
            if matches!(Separable::of(&params), Some(Separable::Kernel(_))) {
                assert_eq!(touched, dot.inflate(reach), "radius {radius}");
            }
        }
    }

    /// Pixels outside [`OperatorRegion::input`] don't change an output area's result, and the
    /// edge repeats at the extent: a layer's area next to the extent's edge blurs the same
    /// whatever lies beyond it.
    #[test]
    fn output_depends_only_on_the_input_region() {
        let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::F32, true);
        let extent = Rect::new(0, 0, 300, 200);
        let noise = |seed: u32| {
            let mut s = Surface::new(fmt);
            for y in 0..200 {
                for x in 0..300 {
                    let h = (x as u32).wrapping_mul(73_856_093) ^ (y as u32).wrapping_mul(19_349_663) ^ seed;
                    let v = (h % 1000) as f32 / 999.0;
                    s.fill_rect(Rect::new(x, y, x + 1, y + 1), &[v, 1.0 - v, (h % 7) as f32 / 6.0, (h % 5) as f32 / 4.0]);
                }
            }
            s
        };
        let a = noise(1);
        for radius in [0.8, 4.0, 12.0] {
            let params = FilterParams::GaussianBlur { radius };
            let region = OperatorRegion { reach: Separable::of(&params).unwrap().reach(), extent };
            for out in [Rect::new(100, 80, 140, 120), Rect::new(0, 0, 30, 25), Rect::new(270, 170, 300, 200)] {
                let input = region.input(out);
                assert!(extent.contains_rect(&input) && input.contains_rect(&out));
                // Everything outside the input region replaced by other pixels.
                let mut b = noise(2);
                let mut inside = Vec::new();
                a.read_region_into(input, &mut inside);
                b.write_region(input, &inside);
                let blur = |s: &Surface| {
                    let r = crate::apply_in_with(s, &params, out, extent, None, extent, &Interrupt::NONE).unwrap();
                    let mut px = Vec::new();
                    r.read_region_into(out, &mut px);
                    px
                };
                assert_eq!(blur(&a), blur(&b), "radius {radius}, {out:?}");
            }
        }
        assert_eq!(OperatorRegion { reach: 5, extent }.input(Rect::EMPTY), Rect::EMPTY);
        assert_eq!(OperatorRegion { reach: 5, extent }.input(Rect::new(400, 400, 410, 410)), Rect::EMPTY, "out of reach of the extent");
        assert_eq!(OperatorRegion { reach: -3, extent }.input(Rect::new(10, 10, 20, 20)), Rect::new(10, 10, 20, 20));
        assert_eq!(edge_extent(extent, Rect::new(-10, 50, 20, 60)), Rect::new(-10, 0, 300, 200));
    }

    /// Unsharp Mask's blur is always the exact kernel `sharpen::exact` uses, whatever the radius
    /// (Gaussian Blur's larger radii are box passes); a radius that isn't one has none.
    #[test]
    fn unsharp_mask_blurs_with_the_exact_kernel() {
        for radius in [0.5f32, 4.0, 40.0, 250.0] {
            let p = FilterParams::UnsharpMask { amount: 100.0, radius, threshold: 0.0 };
            assert_eq!(Separable::of(&p), Some(Separable::Kernel(blur::gaussian_kernel(radius))), "radius {radius}");
        }
        for radius in [-1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(Separable::of(&FilterParams::UnsharpMask { amount: 100.0, radius, threshold: 0.0 }), None);
            assert_eq!(Separable::of(&FilterParams::HighPass { radius }), None);
        }
        assert_eq!(Separable::of(&FilterParams::HighPass { radius: 40.0 }), Some(Separable::Kernel(blur::gaussian_kernel(40.0))));
    }

    /// Box Blur as `blur::boxed` runs it: a uniform kernel up to radius 4, one box pass beyond.
    #[test]
    fn box_blur_is_a_uniform_kernel_then_one_box() {
        assert_eq!(Separable::of(&FilterParams::BoxBlur { radius: 2.4 }), Some(Separable::Kernel(vec![0.2; 5])));
        assert_eq!(Separable::of(&FilterParams::BoxBlur { radius: 0.0 }), Some(Separable::Kernel(vec![1.0])));
        assert_eq!(Separable::of(&FilterParams::BoxBlur { radius: 10.0 }), Some(Separable::Boxes(vec![21])));
        assert_eq!(Separable::of(&FilterParams::BoxBlur { radius: f32::NAN }), None);
    }
}
