//! Wave's Scale Horiz. and Vert. (Photoshop's Wave dialog): each one scales the displacement
//! along its own axis and leaves the other axis alone.

use super::*;

const AREA: Rect = Rect { x0: 0, y0: 0, x1: 96, y1: 80 };

/// Pixels whose source stays inside [`AREA`] for amplitudes up to 6 px, so no edge rule applies.
const INNER: Rect = Rect { x0: 8, y0: 8, x1: 88, y1: 72 };

/// A 32-bit image whose red and green hold each pixel centre's x and y: bilinear sampling of it
/// returns the very position sampled, so a filtered pixel reads back where it came from.
fn coordinates() -> Surface {
    let mut surf = Surface::new(fmt(SampleType::F32));
    let mut v = Vec::new();
    for y in AREA.y0..AREA.y1 {
        for x in AREA.x0..AREA.x1 {
            v.extend_from_slice(&[x as f32 + 0.5, y as f32 + 0.5, 0.0, 1.0]);
        }
    }
    surf.write_region(AREA, &v);
    surf
}

/// A sine Wave of four generators, scaled by `horizontal` and `vertical` percent.
fn wave(horizontal: f32, vertical: f32) -> FilterParams {
    FilterParams::Wave {
        generators: 4,
        wavelength_min: 20.0,
        wavelength_max: 60.0,
        amplitude_min: 2.0,
        amplitude_max: 6.0,
        scale_horizontal: horizontal,
        scale_vertical: vertical,
        wave_type: WaveType::Sine,
        undefined: UndefinedAreas::Repeat,
        seed: 3,
    }
}

/// How far `params` moved each pixel of [`INNER`]: (source − destination) along x and y.
fn displacement(params: &FilterParams) -> Vec<(f32, f32)> {
    let out = apply(&coordinates(), params, AREA, AREA, None).read_region(INNER);
    let w = INNER.width() as i32;
    out.chunks_exact(4)
        .enumerate()
        .map(|(i, px)| {
            let (x, y) = (INNER.x0 + i as i32 % w, INNER.y0 + i as i32 / w);
            (px[0] - (x as f32 + 0.5), px[1] - (y as f32 + 0.5))
        })
        .collect()
}

#[test]
fn each_scale_multiplies_only_its_own_axis() {
    let full = displacement(&wave(100.0, 100.0));
    let peak = |axis: fn(&(f32, f32)) -> f32| full.iter().map(axis).fold(0.0f32, |m, d| m.max(d.abs()));
    // The unscaled wave moves pixels well over a pixel both ways, so a scale has something to scale.
    assert!(peak(|d| d.0) > 1.5 && peak(|d| d.1) > 1.5, "unscaled peaks {} {}", peak(|d| d.0), peak(|d| d.1));
    for (horizontal, vertical) in [(50.0, 100.0), (100.0, 25.0), (30.0, 70.0), (1.0, 1.0)] {
        let scaled = displacement(&wave(horizontal, vertical));
        for (i, (s, f)) in scaled.iter().zip(&full).enumerate() {
            let want = (f.0 * horizontal / 100.0, f.1 * vertical / 100.0);
            assert!((s.0 - want.0).abs() < 1e-3 && (s.1 - want.1).abs() < 1e-3, "{horizontal}/{vertical} % at pixel {i}: moved {s:?}, want {want:?}");
        }
    }
}

/// One square-wave generator of a fixed 4 px amplitude moves every pixel exactly ±4 px along
/// each axis, so the scales give exact sample positions: 50 % horizontal moves pixels exactly
/// ±2 px sideways and 25 % vertical exactly ±1 px up or down, in the unscaled wave's directions.
#[test]
fn a_square_wave_moves_pixels_by_exactly_the_scaled_amplitude() {
    let square = |horizontal: f32, vertical: f32| FilterParams::Wave {
        generators: 1,
        wavelength_min: 30.0,
        wavelength_max: 30.0,
        amplitude_min: 4.0,
        amplitude_max: 4.0,
        scale_horizontal: horizontal,
        scale_vertical: vertical,
        wave_type: WaveType::Square,
        undefined: UndefinedAreas::Repeat,
        seed: 7,
    };
    let full = displacement(&square(100.0, 100.0));
    let scaled = displacement(&square(50.0, 25.0));
    assert!(full.iter().all(|d| d.0.abs() == 4.0 && d.1.abs() == 4.0), "unscaled: {:?}", &full[..8]);
    for axis in [|d: &(f32, f32)| d.0, |d: &(f32, f32)| d.1] {
        assert!(full.iter().any(|d| axis(d) > 0.0) && full.iter().any(|d| axis(d) < 0.0), "the wave moves pixels both ways");
    }
    for (i, (s, f)) in scaled.iter().zip(&full).enumerate() {
        assert_eq!(*s, (f.0 / 2.0, f.1 / 4.0), "pixel {i}");
    }
}

#[test]
fn scales_outside_photoshops_range_render_at_its_limits() {
    let same = |a: f32, b: f32, c: f32, d: f32| assert_eq!(displacement(&wave(a, b)), displacement(&wave(c, d)), "{a}/{b} vs {c}/{d}");
    same(250.0, 0.0, 100.0, 1.0);
    same(-40.0, 1e9, 1.0, 100.0);
    // A non-finite scale (only a hand-edited record could hold one) is the whole wave.
    same(f32::NAN, f32::INFINITY, 100.0, 100.0);
}

#[test]
fn wave_parameters_saved_before_the_scales_existed_are_unscaled() {
    let params = serde_json::json!({
        "filter": "wave",
        "generators": 4,
        "wavelengthMin": 20.0,
        "wavelengthMax": 60.0,
        "amplitudeMin": 2.0,
        "amplitudeMax": 6.0,
        "waveType": "sine",
        "undefined": "repeat",
        "seed": 3
    });
    let decoded: FilterParams = serde_json::from_value(params).unwrap();
    assert_eq!(decoded, wave(100.0, 100.0));
    let written = serde_json::to_value(wave(40.0, 75.0)).unwrap();
    assert_eq!((written["scaleHorizontal"].as_f64(), written["scaleVertical"].as_f64()), (Some(40.0), Some(75.0)));
}
