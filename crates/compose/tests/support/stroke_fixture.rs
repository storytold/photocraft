//! Original procedural artwork: a blue oblique bar cached in a smart object, inside a styled
//! pass-through group. No external artwork, fonts or image files are needed.

use photocraft_color::{BlendMode, Color, ColorMode, SampleType};
use photocraft_doc::{Contour, Document, Effect, FxCommon, FxPaint, Layer, LayerContent, Shadow, SmartObject, SmartSource, StrokeFx, StrokePosition};
use photocraft_geom::{Affine, Size};
use photocraft_raster::Surface;

/// Pixel coverage of a rotated rectangle. Only edge pixels need supersampling.
pub fn coverage(x: u32, y: u32, scale: u32, expanded: f64) -> f32 {
    coverage_at(f64::from(x), f64::from(y), scale, expanded)
}

/// Pixel coverage at a fractional position, for a placed smart object's subpixel translation.
pub fn coverage_at(x: f64, y: f64, scale: u32, expanded: f64) -> f32 {
    let s = f64::from(scale);
    let at = |x: f64, y: f64| {
        let (x, y) = (x - 98.0 * s, y - 98.0 * s);
        let (u, v) = (0.8 * x + 0.6 * y, -0.6 * x + 0.8 * y);
        let (du, dv) = (u.abs() - 65.0 * s, v.abs() - 24.0 * s);
        expanded - (du.max(0.0).hypot(dv.max(0.0)) + du.max(dv).min(0.0))
    };
    let centre = at(x + 0.5, y + 0.5);
    if centre.abs() > 0.71 {
        return if centre > 0.0 { 1.0 } else { 0.0 };
    }
    let mut hits = 0;
    const N: u32 = 32;
    for sy in 0..N {
        for sx in 0..N {
            hits += u32::from(at(x + (f64::from(sx) + 0.5) / f64::from(N), y + (f64::from(sy) + 0.5) / f64::from(N)) >= 0.0);
        }
    }
    hits as f32 / (N * N) as f32
}

pub fn fixture(scale: u32, depth: SampleType, shadow: bool) -> Document {
    let n = 196 * scale;
    let mut d = Document::new("Oblique bar stroke reproduction", Size::new(n, n), ColorMode::Rgb, depth);
    let mut pixels = Surface::new(d.pixel_format());
    for y in 0..n {
        for x in 0..n {
            let a = coverage(x, y, scale, 0.0);
            if a > 0.0 {
                pixels.write_pixel(x as i32, y as i32, &[0.1, 0.5, 0.85, a]);
            }
        }
    }
    let smart = SmartObject::new(SmartSource::Embedded { file_name: "bar.psb".into(), bytes: Default::default() }, Affine::IDENTITY, Some(pixels));
    let mut group = Layer::group("Styled icons", vec![Layer::new("Bar", LayerContent::Smart(smart))]);
    group.effects.items.push(Effect::Stroke(StrokeFx {
        common: FxCommon::new(BlendMode::Normal, 1.0),
        size: 6.0 * scale as f32,
        position: StrokePosition::Outside,
        paint: FxPaint::Color(Color::WHITE),
        overprint: false,
    }));
    if shadow {
        d.global_light.angle = 90.0;
        group.effects.items.push(Effect::DropShadow(Shadow {
            common: FxCommon::new(BlendMode::Normal, 0.96),
            color: Color::BLACK,
            angle: 90.0,
            use_global_light: true,
            distance: 2.0 * scale as f32,
            spread: 0.16,
            size: 12.0 * scale as f32,
            contour: Contour::Linear,
            anti_alias: false,
            noise: 0.0,
            knocks_out: true,
        }));
    }
    d.layers.push(group);
    d
}
