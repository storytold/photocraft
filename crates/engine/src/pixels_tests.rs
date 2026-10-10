use photocraft_color::{ColorMode, PixelFormat, SampleType};
use serde_json::json;

use super::*;

/// The single pass over the content bounds that the tiled `adjust_surface` replaced (#1772).
fn one_pass(s: &Surface, adj: &Adjustment, selection: Option<&Surface>, mode: ColorMode) -> Surface {
    let mut s = s.clone();
    let r = s.content_bounds();
    if !r.is_empty() {
        let out = adjusted(&s, r, adj, selection, mode);
        s.write_region(r, &out);
    }
    s
}

/// Two patches over a 3×3 tile grid, not tile-aligned, with partial alpha; the bottom-left tiles
/// inside the content bounds are never allocated.
fn layer(fmt: PixelFormat) -> Surface {
    let mut s = Surface::new(fmt);
    for r in [Rect::new(200, 240, 300, 300), Rect::new(530, 250, 560, 530)] {
        let mut data = Vec::new();
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let a = if fmt.alpha { 0.3 + 0.7 * ((x + y) % 7) as f32 / 6.0 } else { 1.0 };
                data.extend(from_rgba(&fmt, [x as f32 / 600.0, y as f32 / 540.0, 0.5 + 0.4 * (x as f32 * 0.05).sin(), a]));
            }
        }
        s.write_region(r, &data);
    }
    assert!(!s.has_tiles_in(Rect::new(200, 512, 256, 530)), "a hole inside the content bounds");
    s
}

#[test]
fn tiled_adjustment_matches_one_pass_over_the_content() {
    let adjustments = [
        ("invert", json!({})),
        ("levels", json!({"inBlack": 20, "gamma": 1.3, "inWhite": 230})),
        ("curves", json!({"points": [[0, 10], [128, 160], [255, 240]]})),
        ("hueSaturation", json!({"hue": 40, "saturation": 20})),
        ("brightnessContrast", json!({"brightness": 30, "contrast": 20})),
        ("exposure", json!({"exposure": 0.8})),
        ("posterize", json!({"levels": 5})),
        ("threshold", json!({"level": 120})),
        ("colorBalance", json!({"midtones": [20, -10, 5]})),
        ("blackWhite", json!({"tint": true})),
        ("photoFilter", json!({"density": 50})),
        ("channelMixer", json!({"red": [80, 30, -10, 5]})),
        ("vibrance", json!({"vibrance": 40})),
        ("selectiveColor", json!({"reds": [10, -20, 30, 5]})),
        // Dithered kinds vary by position, so they show a tile's pixels kept their coordinates.
        ("gradientMap", json!({"stops": [[0, "#102030"], [1, "#f0e0a0"]], "dither": true})),
        ("colorLookup", json!({"lut": "warm", "dither": true})),
    ];
    let mut formats = Vec::new();
    for mode in [ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Lab, ColorMode::Grayscale] {
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            formats.push(PixelFormat { mode, sample, alpha: true });
        }
    }
    formats.push(PixelFormat { mode: ColorMode::Rgb, sample: SampleType::U8, alpha: false });
    formats.push(PixelFormat { mode: ColorMode::Grayscale, sample: SampleType::U16, alpha: false });
    // A soft selection that ends inside the content.
    let mut soft = Surface::new(PixelFormat::GRAY8);
    let sr = Rect::new(0, 0, 500, 400);
    soft.write_region(sr, &(0..sr.height()).flat_map(|_| (0..sr.width()).map(|x| x as f32 / 500.0)).collect::<Vec<_>>());
    for fmt in formats {
        let s = layer(fmt);
        for (kind, p) in &adjustments {
            let adj = crate::adjust_params::from_params(kind, p, None, fmt.mode).unwrap_or_else(|e| panic!("{kind}: {e}"));
            for sel in [None, Some(&soft)] {
                let want = one_pass(&s, &adj, sel, fmt.mode);
                let mut got = s.clone();
                adjust_surface(&mut got, &adj, sel, fmt.mode);
                assert!(want != s, "{kind} {fmt:?}: the adjustment changes the layer");
                assert!(got == want, "{kind} {fmt:?} selection {}: tiled result differs", sel.is_some());
            }
        }
    }
}

#[test]
fn an_empty_layer_is_left_alone() {
    let mut s = Surface::new(PixelFormat::RGBA8);
    adjust_surface(&mut s, &Adjustment::Invert, None, ColorMode::Rgb);
    assert_eq!(s.tile_count(), 0);
}
