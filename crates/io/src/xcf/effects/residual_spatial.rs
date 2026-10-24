//! Frozen B06-05/06: finite native geometry, styles and spatial approximations.
//! Source controls are input data only; output is ordinary editable native data.
use super::{ArgumentValue, MappingContext, MappingFailure, MappingResult, NativeBranch, NativeMapping, NativeStructure, StoredEffect, native_filter};
use photocraft_color::{BlendMode, Color};
use photocraft_doc::effects::{Contour, Effect, FxCommon, FxPaint, Glow, GlowSource, GlowTechnique, Shadow};
use photocraft_doc::{Fill, Group, Layer, LayerContent, Path, ShapeLayer, Subpath, VectorMask};
use photocraft_geom::{Affine, Rect};
use serde_json::{Value, json};
type Result<T> = std::result::Result<T, MappingFailure>;
const MAX_GEOMETRY: usize = 65_536;
pub(super) fn invalid(message: impl Into<String>) -> MappingFailure {
    MappingFailure { reason: photocraft_doc::import_conversion::ConversionReason::InvalidOptionalData, message: message.into() }
}
pub(super) fn generation_check(c: &MappingContext) -> Result<()> {
    c.interrupt.check().map_err(|_| MappingFailure::resource("native generation was cancelled"))
}
pub(super) fn generation_bytes(c: &MappingContext, bytes: usize) -> Result<()> {
    generation_check(c)?;
    if bytes > c.remaining_generation_bytes {
        return Err(MappingFailure::resource("native generated data exceeds the actual remaining import byte allowance"));
    }
    Ok(())
}
pub(super) fn geometry_bytes(c: &MappingContext, paths: usize, knots: usize) -> Result<()> {
    let bytes = paths
        .checked_mul(std::mem::size_of::<Subpath>())
        .and_then(|v| knots.checked_mul(std::mem::size_of::<photocraft_doc::Knot>()).and_then(|k| v.checked_add(k)))
        .and_then(|v| v.checked_mul(2))
        .and_then(|v| v.checked_add(4096))
        .ok_or_else(|| MappingFailure::resource("native geometry allocation size overflows"))?;
    if bytes > c.remaining_metadata_bytes {
        return Err(MappingFailure::resource("native geometry exceeds the actual remaining metadata allowance"));
    }
    generation_bytes(c, bytes)
}
pub(super) fn num(e: &StoredEffect<'_>, key: &str, default: f64, lo: f64, hi: f64) -> Result<f64> {
    let v = e.number(key).map_err(invalid)?.unwrap_or(default);
    if !v.is_finite() || v < f64::from(lo as f32) || v > f64::from(hi as f32) {
        return Err(invalid(format!("{}: {key} outside source domain", e.operation)));
    }
    Ok(v)
}
pub(super) fn integer(e: &StoredEffect<'_>, key: &str, default: i32, lo: i32, hi: i32) -> Result<i32> {
    // Stored gint must stay integer; do not pass i32 endpoints through rounded F32.
    let v = e.number(key).map_err(invalid)?.unwrap_or(f64::from(default));
    if !v.is_finite() || v.fract() != 0. || v < f64::from(lo) || v > f64::from(hi) {
        return Err(invalid(format!("{key} must be an in-range source integer")));
    }
    Ok(v as i32)
}
pub(super) fn choice(e: &StoredEffect<'_>, key: &str, default: i32, values: &[i32]) -> Result<i32> {
    let v = e.enum_value(key).map_err(invalid)?.unwrap_or(default);
    if !values.contains(&v) {
        return Err(invalid(format!("{key}: unknown enum {v}")));
    }
    Ok(v)
}
pub(super) fn boolean(e: &StoredEffect<'_>, key: &str, default: bool) -> Result<bool> {
    Ok(e.boolean(key).map_err(invalid)?.unwrap_or(default))
}
pub(super) fn seed(e: &StoredEffect<'_>) -> Result<u32> {
    match e.argument("seed").map_err(invalid)? {
        None => Ok(0),
        Some(ArgumentValue::UInt(v)) => Ok(*v),
        _ => Err(invalid("seed must be stored guint")),
    }
}
pub(super) fn color(e: &StoredEffect<'_>, key: &str, default: [f32; 4], c: &MappingContext) -> Result<Color> {
    let result =
        if e.argument(key).map_err(invalid)?.is_some() { super::resources::native_color(e, key, c) } else { super::resources::native_rgba(default, c) };
    result.map_err(|s| if s.starts_with("resource unavailable:") { MappingFailure::resource(s) } else { invalid(s) })
}
pub(super) fn mapped(filters: Vec<photocraft_doc::SmartFilter>, note: impl Into<String>) -> NativeMapping {
    NativeMapping { filters, notes: vec![note.into()], ..Default::default() }
}
pub(super) fn one(command: &str, params: Value, note: impl Into<String>) -> NativeMapping {
    mapped(vec![native_filter(command, params)], note)
}
pub(super) fn replace(layer: Layer, note: impl Into<String>) -> NativeMapping {
    NativeMapping { structure: Some(NativeStructure::Replace { layer: Box::new(layer), patterns: Vec::new() }), notes: vec![note.into()], ..Default::default() }
}
pub(super) fn composite(branches: Vec<NativeBranch>, note: impl Into<String>) -> NativeMapping {
    NativeMapping { structure: Some(NativeStructure::Composite { branches }), notes: vec![note.into()], ..Default::default() }
}
pub(super) fn bounds(c: &MappingContext) -> Result<Rect> {
    if c.native_filter_bounds.is_empty() { Err(invalid("native geometry needs nonempty evaluation bounds")) } else { Ok(c.native_filter_bounds) }
}
pub(super) fn rect(x: f64, y: f64, w: f64, h: f64) -> Subpath {
    Subpath::polygon(&[(x, y), (x + w, y), (x + w, y + h), (x, y + h)])
}
pub(super) fn shape(name: &str, paths: Vec<Subpath>, paint: Color) -> Layer {
    Layer::new(name, LayerContent::Shape(ShapeLayer { path: Path::new(paths), fill: Some(Fill::Solid(paint)), ..Default::default() }))
}
fn grid(e: &StoredEffect<'_>, c: &MappingContext) -> Result<NativeMapping> {
    let b = bounds(c)?;
    let sx = i64::from(integer(e, "x", 32, 1, i32::MAX)?);
    let sy = i64::from(integer(e, "y", 32, 1, i32::MAX)?);
    let ox = i64::from(integer(e, "x-offset", 0, i32::MIN, i32::MAX)?);
    let oy = i64::from(integer(e, "y-offset", 0, i32::MIN, i32::MAX)?);
    let lw = i64::from(integer(e, "line-width", 4, 0, i32::MAX)?);
    let lh = i64::from(integer(e, "line-height", 4, 0, i32::MAX)?);
    let paint = color(e, "line-color", [0., 0., 0., 1.], c)?;
    let estimated = [(i64::from(b.x0), i64::from(b.x1), sx, ox, lw), (i64::from(b.y0), i64::from(b.y1), sy, oy, lh)].into_iter().try_fold(
        0usize,
        |total, (start, end, spacing, origin, width)| {
            let count = if width == 0 {
                0
            } else if width >= spacing {
                1
            } else {
                let first = start - (start - origin).rem_euclid(spacing);
                usize::try_from((end - first + spacing - 1) / spacing).map_err(|_| invalid("grid count"))?
            };
            total.checked_add(count).ok_or_else(|| invalid("grid rectangle count overflow"))
        },
    )?;
    if estimated > MAX_GEOMETRY {
        return Err("native grid exceeds the shared geometry work-item allowance".into());
    }
    geometry_bytes(c, estimated, estimated.checked_mul(4).ok_or_else(|| invalid("grid knot count overflow"))?)?;
    let mut paths = Vec::new();
    paths.try_reserve_exact(estimated).map_err(|_| MappingFailure::resource("grid path reservation unavailable"))?;
    for (start, end, spacing, origin, width, vertical) in
        [(i64::from(b.x0), i64::from(b.x1), sx, ox, lw, true), (i64::from(b.y0), i64::from(b.y1), sy, oy, lh, false)]
    {
        if width == 0 {
            continue;
        }
        if width >= spacing {
            paths.push(rect(f64::from(b.x0), f64::from(b.y0), f64::from(b.width()), f64::from(b.height())));
            continue;
        }
        let first = start - (start - origin).rem_euclid(spacing);
        let count = (end - first + spacing - 1) / spacing;
        let count = usize::try_from(count).map_err(|_| invalid("grid count"))?;
        if paths.len().checked_add(count).is_none_or(|n| n > MAX_GEOMETRY) {
            return Err("native grid exceeds the shared 65,536 geometry work-item allowance".into());
        }
        for n in 0..count {
            generation_check(c)?;
            let at = first + spacing * n as i64;
            let a = at.max(start);
            let z = (at + width).min(end);
            if a < z {
                paths.push(if vertical {
                    rect(a as f64, f64::from(b.y0), (z - a) as f64, f64::from(b.height()))
                } else {
                    rect(f64::from(b.x0), a as f64, f64::from(b.width()), (z - a) as f64)
                });
            }
        }
    }
    Ok(replace(
        shape("Imported grid lines", paths, paint),
        "Source spacing, origin, zero thickness and paint become editable native vector rectangles. Native vector antialiasing and compositing apply; source raster sampling is not emulated",
    ))
}
fn flare(e: &StoredEffect<'_>, c: &MappingContext) -> Result<NativeMapping> {
    let b = bounds(c)?;
    let owner = c.owner_bounds;
    let x = (f64::from(owner.x0) + num(e, "pos-x", 0.5, -f64::MAX, f64::MAX)? * f64::from(owner.width()) - f64::from(b.x0)) / f64::from(b.width());
    let y = (f64::from(owner.y0) + num(e, "pos-y", 0.5, -f64::MAX, f64::MAX)? * f64::from(owner.height()) - f64::from(b.y0)) / f64::from(b.height());
    if !(-0.5..=1.5).contains(&x) || !(-0.5..=1.5).contains(&y) {
        return Err("source flare center lies outside native Lens Flare center domain; no silent clamp".into());
    }
    Ok(one(
        "filter.render.lensFlare",
        json!({"centerX":x,"centerY":y,"brightness":100,"lens":"zoom"}),
        "Source normalized flare center becomes native Lens Flare. Source has no intensity or lens-type control; native 100% zoom lens is an explicitly selected optical approximation with different flare geometry and photometry",
    ))
}
fn glow(e: &StoredEffect<'_>, c: &MappingContext) -> Result<NativeMapping> {
    let morphology = choice(e, "grow-shape", 1, &[0, 1, 2])?;
    let x = num(e, "x", 0., -f64::MAX, f64::MAX)?;
    let y = num(e, "y", 0., -f64::MAX, f64::MAX)?;
    let radius = num(e, "radius", 7.5, 0., 40.)?;
    let grow = num(e, "grow-radius", 4., 1., 30.)?;
    let opacity = num(e, "opacity", 1.2, 0., 2.)?;
    let cover = num(e, "cover", 60., 50., 100.)?;
    let mut paint = color(e, "value", [251. / 255., 1., 0., 1.], c)?;
    for channel in &mut paint.c {
        *channel *= opacity.max(1.) as f32;
    }
    let common = FxCommon::new(BlendMode::Normal, opacity.min(1.) as f32 * paint.alpha);
    paint.alpha = 1.;
    let spread = (grow / (grow + radius.max(0.001))) as f32;
    let effect = if x == 0. && y == 0. {
        Effect::InnerGlow(Glow {
            common,
            paint: FxPaint::Color(paint),
            technique: GlowTechnique::Precise,
            spread,
            size: radius as f32,
            contour: Contour::Linear,
            anti_alias: true,
            range: 1.,
            jitter: 0.,
            noise: 0.,
            source: GlowSource::Edge,
        })
    } else {
        let distance = x.hypot(y);
        if !distance.is_finite() || distance > 1_000_000. {
            return Err("source directional inner glow exceeds bounded native style distance".into());
        }
        Effect::InnerShadow(Shadow {
            common,
            color: paint,
            angle: (-y.atan2(x).to_degrees() + 180.) as f32,
            use_global_light: false,
            distance: distance as f32,
            spread,
            size: radius as f32,
            contour: Contour::Linear,
            anti_alias: true,
            noise: 0.,
            knocks_out: false,
        })
    };
    Ok(NativeMapping {
        layer_effects: vec![effect],
        notes: vec![format!(
            "Native inner {} preserves directional offset/blur/paint intent. Native choke uses grow/(grow+radius); source morphology {morphology}, cover {cover}% and coverage amplification are explicitly approximated by native precise style and HDR paint rather than a source coverage interpreter",
            if x == 0. && y == 0. { "glow" } else { "shadow" }
        )],
        ..Default::default()
    })
}
fn oil(e: &StoredEffect<'_>, c: &MappingContext) -> Result<NativeMapping> {
    let radius = integer(e, "mask-radius", 4, 1, 100)?;
    let exponent = integer(e, "exponent", 8, 1, 20)?;
    let bins = integer(e, "intensities", 128, 8, 256)?;
    let intensity = boolean(e, "use-inten", true)?;
    generation_check(c)?;
    let length = 2. * f64::from(radius);
    let stylization = ((length - 1.5) / 2.).clamp(0.1, 10.);
    let mut paint = Vec::new();
    if length > 21.5 {
        paint.push(native_filter("filter.blur.surfaceBlur", json!({"radius":radius,"threshold":15.})));
    }
    paint.push(native_filter(
        "filter.stylize.oilPaint",
        json!({"stylization":stylization,"cleanliness":f64::from(exponent-1)/19.*10.,"scale":1.,"bristleDetail":0.,"lighting":false,"shine":0.}),
    ));
    let note = format!(
        "Source mask-radius{radius} defines native stroke diameter2r; native Oil Paint supports stroke length1.7..21.5 pixels, so wider radii add ordinary native Surface Blur at radius r with named native default threshold15. Source exponent{exponent} selects native cleanliness smoothness0..10 across its declared1..20 range. use-inten={intensity} selects native joint-color intensity-guided strokes or independent native component-guided strokes. Source histogram size{bins} is explicitly superseded estimator-resolution provenance: deterministic native flow smoothing has no histogram estimator or histogram-size control, so bins intentionally do not change native pixels and are not a missing mapping. Native field/tensor, per-channel palette/color mixing, premultiplied smoothing, ICC/transfer, boundaries and native alpha govern edits; no source histogram kernel, output posterization or invented bristle lighting is added"
    );
    if intensity || c.mode == photocraft_color::ColorMode::Grayscale {
        let mut mapped = mapped(paint, note);
        if !intensity {
            mapped.notes.push("Gray has one ordinary native color component, so independent-component and joint-intensity native painting coincide without adding invented RGB channels".into());
        }
        return Ok(mapped);
    }
    if c.mode != photocraft_color::ColorMode::Rgb {
        return Err("Native independent-component oil painting currently supports ordinary RGB or scalar Gray owners".into());
    }
    let bytes = 32_768;
    generation_bytes(c, bytes)?;
    if bytes > c.remaining_metadata_bytes {
        return Err(MappingFailure::resource("native independent-component paint branches exceed remaining metadata allowance"));
    }
    let mut branches = Vec::new();
    for axis in 0..3 {
        generation_check(c)?;
        let mut selected = [0.; 4];
        if let Some(value) = selected.get_mut(axis) {
            *value = 100.;
        }
        let mut filters = vec![native_filter("image.adjustments.channelMixer", json!({"monochrome":true,"gray":selected}))];
        filters.extend(paint.iter().cloned());
        let mut rows = [[0.; 4]; 3];
        if let Some(row) = rows.get_mut(axis) {
            row[0] = 100.;
        }
        filters.push(native_filter("image.adjustments.channelMixer", json!({"red":rows[0],"green":rows[1],"blue":rows[2]})));
        if axis != 0 {
            filters.push(native_filter("filter.other.setAlpha", json!({"value":1.})));
        }
        branches.push(NativeBranch {
            filters,
            blend: if axis == 0 { BlendMode::Normal } else { BlendMode::LinearDodge },
            clipped: axis != 0,
            ..Default::default()
        });
    }
    Ok(composite(branches, note))
}

fn image_gradient(e: &StoredEffect<'_>, c: &MappingContext) -> Result<NativeMapping> {
    let mode = choice(e, "output-mode", 0, &[0, 1, 2])?;
    if mode != 1 {
        return Ok(mapped(
            vec![native_filter("filter.stylize.findEdges", json!({})), native_filter("image.adjustments.invert", json!({}))],
            "Native inverted Find Edges provides a black-flat gradient-magnitude approximation. Stock drawable output for Both also presents magnitude, rather than an invented RGB vector layout. Native edge metric, working color/profile, kernel, alpha and boundaries replace the source derivative norm; native source coverage is preserved rather than made opaque",
        ));
    }
    if !matches!(c.mode, photocraft_color::ColorMode::Rgb | photocraft_color::ColorMode::Grayscale) {
        return Err("Native direction approximation requires RGB axes or ordinary native RGB working contents for a Gray owner".into());
    }
    let bytes = 17usize.pow(3) * 96 + 24_576;
    generation_bytes(c, bytes)?;
    if bytes > c.remaining_metadata_bytes {
        return Err(MappingFailure::resource("native gradient direction lookup exceeds actual remaining metadata allowance"));
    }
    let mut branches = Vec::new();
    for (axis, before, after) in [(0, 11, 13), (1, 7, 17)] {
        let mut kernel = vec![0.; 25];
        if let Some(value) = kernel.get_mut(before) {
            *value = -1.;
        }
        if let Some(value) = kernel.get_mut(after) {
            *value = 1.;
        }
        let mut rows = [[0.; 4]; 3];
        if let Some(row) = rows.get_mut(axis) {
            row[0] = 100.;
        }
        let mut filters = vec![
            native_filter("image.adjustments.channelMixer", json!({"monochrome":true,"gray":[21.26,71.52,7.22,0.]})),
            native_filter("filter.other.custom", json!({"kernel":kernel,"scale":2.,"offset":127.5})),
            native_filter("image.adjustments.channelMixer", json!({"red":rows[0],"green":rows[1],"blue":rows[2]})),
        ];
        if axis != 0 {
            filters.push(native_filter("filter.other.setAlpha", json!({"value":1.})));
        }
        branches.push(NativeBranch {
            filters,
            blend: if axis == 0 { BlendMode::Normal } else { BlendMode::LinearDodge },
            clipped: axis != 0,
            ..Default::default()
        });
    }
    let mut result = super::resources::lookup(17, "Native gradient direction", Vec::new(), |rgb| {
        let x = rgb[0] - 0.5;
        let y = rgb[1] - 0.5;
        let angle = if y == 0. { if x > 0. { std::f64::consts::PI } else { 0. } } else { (-y).atan2(-x) };
        [(angle + std::f64::consts::PI) / std::f64::consts::TAU; 3]
    })
    .map_err(invalid)?;
    let mut gain = vec![0.; 25];
    if let Some(value) = gain.get_mut(12) {
        *value = std::f64::consts::TAU;
    }
    result.filters.push(native_filter(
        "filter.other.convolution",
        json!({"kernel":gain,"scale":1.,"offset":-std::f64::consts::PI,"channels":[true,true,true,false],"alphaWeighted":false,"edge":"repeat"}),
    ));
    result.structure = Some(NativeStructure::Composite { branches });
    result.working_rgb = c.mode == photocraft_color::ColorMode::Grayscale;
    result.notes.push("Direction becomes ordinary native monochrome central-difference Custom filters, fixed native RGB axis routing/clipped additive composition, a joint 17-cube Color Lookup and native Convolution gain/offset for signed radians. Standard negative-gradient atan2 is sampled only while authoring native LUT data; no source algorithm/evaluator remains. Flat vectors select zero radians. Native three-point support, working RGB/profile/transfer, finite derivative range and LUT interpolation/angle branch seam differ from source. Native integer storage clips signed/HDR output; F32 retains signed radians. The clipped second derivative is ordinary native opaque paint so fractional source alpha cannot attenuate its angle component; base native source coverage is preserved rather than replaced with observed opaque source output".into());
    generation_check(c)?;
    Ok(result)
}

fn local_tone(e: &StoredEffect<'_>) -> Result<NativeMapping> {
    let contrast = num(e, "contrast", 0.1, 0., 1.)?;
    let sat = num(e, "saturation", 0.8, 0., 2.)?;
    let detail = num(e, "detail", 1., 1., 99.)?;
    let exponent = contrast.max(1. / 256.);
    let points: Vec<_> = (0..=255)
        .map(|n| {
            let x = f64::from(n) / 255.;
            json!(255. * x.powf(exponent))
        })
        .collect();
    let mut filters = vec![
        native_filter("image.adjustments.curves", json!({"pencil":true,"points":points})),
        native_filter("image.adjustments.hueSaturation", json!({"saturation":(sat-1.)*100.})),
    ];
    if detail > 1. {
        filters.push(native_filter("filter.sharpen.unsharpMask", json!({"radius":1.,"amount":detail.ln()*100.,"threshold":0.})));
    }
    Ok(mapped(
        filters,
        "Source HDR contrast/saturation/detail intent becomes editable native global power-curve compression, native saturation and logarithmic local-detail sharpening. This explicit policy differs from perceptual gradient-domain HDR optimization; native ICC, HDR curve endpoints, clipping and alpha apply. Contrast zero uses the finite 1/256 native power limit",
    ))
}
fn solid_noise(e: &StoredEffect<'_>, c: &MappingContext) -> Result<NativeMapping> {
    let x_size = num(e, "x-size", 4., 0.1, 16.)?;
    let y_size = num(e, "y-size", 4., 0.1, 16.)?;
    let detail = integer(e, "detail", 1, 0, 15)? as u32;
    let tile = boolean(e, "tileable", false)?;
    let turbulent = boolean(e, "turbulent", false)?;
    let seed = seed(e)?;
    let width = integer(e, "width", 1024, 0, i32::MAX)? as u32;
    let height = integer(e, "height", 768, 0, i32::MAX)? as u32;
    generation_check(c)?;
    if width == 0 || height == 0 {
        return Ok(replace(
            Layer::new("Empty native solid noise", LayerContent::Fill(Fill::Solid(Color::TRANSPARENT))),
            "Zero source solid-noise extent becomes ordinary native transparent fill; native document mode/alpha rules apply",
        ));
    }
    let octaves = detail + 1;
    let work = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|n| n.checked_mul(u64::from(octaves)))
        .and_then(|n| n.checked_mul(if tile { 16 } else { 4 }))
        .ok_or_else(|| invalid("Native solid noise work count overflow"))?;
    if work > 32_000_000 {
        return Err(MappingFailure::resource("Native solid-noise lattice work exceeds bounded import allowance"));
    }
    let sample = |x: f64, y: f64, octave: u32| {
        let ix = x.floor() as i64;
        let iy = y.floor() as i64;
        let smooth = |v: f64| v * v * (3. - 2. * v);
        let u = smooth(x - x.floor());
        let v = smooth(y - y.floor());
        let hash = |dx: i64, dy: i64| super::residual_transform::lattice(ix.saturating_add(dx), iy.saturating_add(dy), i64::from(octave), seed);
        let top = hash(0, 0) * (1. - u) + hash(1, 0) * u;
        let bottom = hash(0, 1) * (1. - u) + hash(1, 1) * u;
        top * (1. - v) + bottom * v
    };
    let mut converted = super::residual_tone::pattern_result(
        "Imported native solid fractal noise",
        photocraft_geom::Size::new(width, height),
        (0., 0.),
        0.,
        c,
        |px, py| {
            let u = if width == 1 { 0. } else { f64::from(px) / f64::from(width - 1) };
            let v = if height == 1 { 0. } else { f64::from(py) / f64::from(height - 1) };
            let (mut sum, mut weight, mut amplitude) = (0., 0., 1.);
            for octave in 0..octaves {
                let frequency = 2f64.powi(octave as i32);
                let fx = x_size * frequency;
                let fy = y_size * frequency;
                let x = u * fx;
                let y = v * fy;
                let value = if tile {
                    let smooth = |v: f64| v * v * (3. - 2. * v);
                    let a = smooth(u);
                    let b = smooth(v);
                    let top = sample(x, y, octave) * (1. - a) + sample(x - fx, y, octave) * a;
                    let bottom = sample(x, y - fy, octave) * (1. - a) + sample(x - fx, y - fy, octave) * a;
                    top * (1. - b) + bottom * b
                } else {
                    sample(x, y, octave)
                };
                sum += amplitude * if turbulent { (2. * value - 1.).abs() } else { value };
                weight += amplitude;
                amplitude *= 0.5;
            }
            let value = (sum / weight).clamp(0., 1.) as f32;
            Ok([value, value, value, 1.])
        },
        "All solid-noise controls author an ordinary finite native Pattern using original cubic lattice interpolation and normalized octave sums. x/y-size select cycles across the declared pixel extent; detail adds octaves, seed selects the native lattice, turbulent selects absolute signed noise. Tileable smoothly crossfades offset copies so opposite resource edges match even for fractional cycle counts. Source gradient-noise spectrum, random sequence, normalization and edge behavior differ; native target ICC, pattern sampling and editing apply, with no source noise evaluator after import",
    )?;
    if let Some(NativeStructure::Replace { layer, .. }) = &mut converted.structure {
        layer.vector_mask = Some(VectorMask::new(Path::new(vec![rect(0., 0., f64::from(width), f64::from(height))])));
    }
    Ok(converted)
}

fn mosaic(e: &StoredEffect<'_>, c: &MappingContext) -> Result<NativeMapping> {
    let kind = choice(e, "tile-type", 1, &[0, 1, 2, 3])?;
    let size = num(e, "tile-size", 15., 1., 1000.)?;
    let height = num(e, "tile-height", 4., 1., 1000.)?;
    let neat = num(e, "tile-neatness", 0.65, 0., 1.)?;
    let variation = num(e, "color-variation", 0.2, 0., 1.)?;
    let averaging = boolean(e, "color-averaging", true)?;
    let textured = boolean(e, "tile-surface", false)?;
    let split = boolean(e, "tile-allow-split", true)?;
    let aa = boolean(e, "antialiasing", true)?;
    let spacing = num(e, "tile-spacing", 1., 0., 1000.)?;
    let light = num(e, "light-dir", 135., 0., 360.)?;
    let mut state = seed(e)?;
    let joints = color(e, "joints-color", [0., 0., 0., 1.], c)?;
    let mut highlight = color(e, "light-color", [1., 1., 1., 1.], c)?;
    let highlight_opacity = highlight.alpha;
    highlight.alpha = 1.;
    let mut shadow = super::resources::native_rgba([0., 0., 0., 1.], c).map_err(invalid)?;
    shadow.alpha = 1.;
    let b = bounds(c)?;
    let step = size.ceil() as i32;
    if spacing >= f64::from(b.width().min(step as u32)) || spacing >= f64::from(b.height().min(step as u32)) {
        return Ok(replace(
            Layer::new("Native mosaic joints", LayerContent::Fill(Fill::Solid(joints))),
            "Requested spacing consumes every native polygon interior. Ordinary native joints Solid Fill replaces the empty cell stage; no invisible frame/style allocations are generated. Source topology and native inset geometry differ explicitly",
        ));
    }
    let nx = (b.width() as usize).div_ceil(step as usize);
    let ny = (b.height() as usize).div_ceil(step as usize);
    let count = nx.checked_mul(ny).and_then(|n| n.checked_mul(if split { 2 } else { 1 })).ok_or_else(|| invalid("native mosaic count overflow"))?;
    if count > 63 {
        return Err(MappingFailure::resource(
            "native editable mosaic exceeds63 polygon branches plus joints; denser cells exceed this representation's bounded branch allowance",
        ));
    }
    geometry_bytes(c, count, count * 10)?;
    let metadata = count * 12288 + 8192;
    if metadata > c.remaining_metadata_bytes {
        return Err(MappingFailure::resource("native mosaic exceeds actual metadata allowance"));
    }
    let fw = b.width().min(step as u32) as usize;
    let fh = b.height().min(step as u32) as usize;
    let padded_w = (fw as f64 + 2. * height + 4.).ceil() as usize;
    let padded_h = (fh as f64 + 2. * height + 4.).ceil() as usize;
    let tile = photocraft_geom::TILE_SIZE as usize;
    let framed_cache = fw.div_ceil(tile) * fh.div_ceil(tile) * 3 * tile * tile * (c.mode.color_channels() + 1) * 4;
    let work = padded_w
        .checked_mul(padded_h)
        .and_then(|n| n.checked_mul((c.mode.color_channels() + 1) * 4 * 6))
        .and_then(|n| n.checked_mul(count))
        .and_then(|n| n.checked_add(count * framed_cache))
        .and_then(|n| n.checked_add(metadata))
        .ok_or_else(|| invalid("native mosaic workspace overflow"))?;
    generation_bytes(c, work)?;
    let mut branches = Vec::new();
    branches.try_reserve_exact(count + 1).map_err(|_| MappingFailure::resource("native mosaic branches unavailable"))?;
    branches.push(NativeBranch { fill: Some(Fill::Solid(joints)), ..Default::default() });
    for y in 0..ny {
        for x in 0..nx {
            generation_check(c)?;
            let x0 = i64::from(b.x0) + x as i64 * i64::from(step);
            let y0 = i64::from(b.y0) + y as i64 * i64::from(step);
            let cell = Rect::new(x0 as i32, y0 as i32, (x0 + i64::from(step)).min(i64::from(b.x1)) as i32, (y0 + i64::from(step)).min(i64::from(b.y1)) as i32);
            let center = (f64::from(cell.x0) + f64::from(cell.width()) / 2., f64::from(cell.y0) + f64::from(cell.height()) / 2.);
            let w = (f64::from(cell.width()) - spacing).max(0.);
            let h = (f64::from(cell.height()) - spacing).max(0.);
            if w == 0. || h == 0. {
                continue;
            }
            let sides = match kind {
                0 => 4,
                1 => 6,
                2 => 8,
                _ => 3,
            };
            let mut points = Vec::new();
            points.try_reserve_exact(sides).map_err(|_| MappingFailure::resource("native mosaic vertices unavailable"))?;
            for n in 0..sides {
                let angle =
                    std::f64::consts::TAU * n as f64 / sides as f64 + if kind == 0 { std::f64::consts::FRAC_PI_4 } else { -std::f64::consts::FRAC_PI_2 };
                let jitter = 1. - (1. - neat) * 0.2 * (f64::from(random(&mut state)) / f64::from(u32::MAX));
                let factor = if kind == 0 { 2f64.sqrt() } else { 1. };
                points.push((center.0 + angle.cos() * w / 2. * factor * jitter, center.1 + angle.sin() * h / 2. * factor * jitter));
            }
            let polygons = if split {
                if sides == 3 {
                    let a = points[0];
                    let d = points[1];
                    let mid = ((a.0 + d.0) / 2., (a.1 + d.1) / 2.);
                    vec![vec![a, mid, points[2]], vec![mid, d, points[2]]]
                } else {
                    let half = sides / 2;
                    let first = points[..=half].to_vec();
                    let mut second = points[half..].to_vec();
                    second.push(points[0]);
                    vec![first, second]
                }
            } else {
                vec![points]
            };
            for polygon in polygons {
                let (frame, transform) = if averaging {
                    (cell, Affine::IDENTITY)
                } else {
                    let sx = (center.0.floor() as i32).min(cell.x1 - 1);
                    let sy = (center.1.floor() as i32).min(cell.y1 - 1);
                    (
                        Rect::from_xywh(sx, sy, 1, 1),
                        Affine::translate(f64::from(cell.x0), f64::from(cell.y0))
                            .mul(&Affine { m: [f64::from(cell.width()), 0., 0., f64::from(cell.height()), 0., 0.] })
                            .mul(&Affine::translate(-f64::from(sx), -f64::from(sy))),
                    )
                };
                let mut filters = Vec::new();
                if averaging {
                    filters.push(native_filter("filter.blur.average", json!({})));
                }
                if variation > 0. {
                    let delta = (f64::from(random(&mut state)) / f64::from(u32::MAX) * 2. - 1.) * variation;
                    filters.push(native_filter("image.adjustments.exposure", json!({"exposure":delta,"offset":0.,"gamma":1.})));
                }
                if textured {
                    filters.push(native_filter(
                        "filter.gallery.texturizer",
                        json!({"texture":"sandstone","scaling":100,"relief":1,"light":"topRight","invert":false}),
                    ));
                }
                let bevel = photocraft_doc::effects::Bevel {
                    enabled: true,
                    style: photocraft_doc::effects::BevelStyle::InnerBevel,
                    technique: photocraft_doc::effects::BevelTechnique::Smooth,
                    depth: 1.,
                    up: true,
                    size: height as f32,
                    soften: 0.,
                    angle: light as f32,
                    altitude: 45.,
                    use_global_light: false,
                    gloss_contour: Contour::Linear,
                    gloss_anti_alias: true,
                    highlight: FxCommon::new(BlendMode::Screen, highlight_opacity),
                    highlight_color: highlight,
                    shadow: FxCommon::new(BlendMode::Multiply, 0.5),
                    shadow_color: shadow,
                    contour: None,
                    texture: None,
                };
                branches.push(NativeBranch {
                    filter_frame: Some(frame),
                    transform,
                    filters,
                    vector_mask: Some(VectorMask::new(Path::new(vec![Subpath::polygon(&polygon)]))),
                    layer_effects: vec![Effect::BevelEmboss(bevel)],
                    ..Default::default()
                });
            }
        }
    }
    Ok(composite(
        branches,
        format!(
            "Native editable framed cell copies use requested tile size rounded upward to whole pixels, native square/inscribed hexagon/octagon/triangle masks, source joints paint and spacing, deterministic neatness jitter and native Average or center-point fill. Allow-split{split} selects deterministic polygon subdivisions rather than source content-dependent splitting. Source variation becomes bounded per-cell native Exposure; tile-surface{textured} becomes native sandstone texture. Requested height/light direction and actual CMS-converted light paint become native Bevel & Emboss with native alpha/working-color/shading. Source topology, region averaging, seed sequence and highlight response differ. Source AA{aa} is superseded by PhotoCraft's native vector-mask antialiasing policy. No source tessellator or pixel evaluator persists"
        ),
    ))
}
fn normal_map(e: &StoredEffect<'_>, c: &MappingContext) -> Result<NativeMapping> {
    let scale = num(e, "scale", 10., 0., f64::MAX)?;
    let x = choice(e, "x-component", 0, &[0, 1, 2])?;
    let y = choice(e, "y-component", 1, &[0, 1, 2])?;
    let flip_x = boolean(e, "flip-x", false)?;
    let flip_y = boolean(e, "flip-y", false)?;
    let full = boolean(e, "full-z", false)?;
    let tile = boolean(e, "tileable", false)?;
    if !matches!(c.mode, photocraft_color::ColorMode::Rgb | photocraft_color::ColorMode::Grayscale) {
        return Err("native normal-vector construction requires RGB or Gray with ordinary RGB Smart contents".into());
    }
    let stored_y = y;
    let y = if x == y { (x + 1) % 3 } else { y };
    let mut branches = Vec::new();
    for (component, horizontal, flip) in [(x, true, flip_x), (y, false, flip_y)] {
        let sign = if flip { -1. } else { 1. };
        let mut kernel = vec![0.; 25];
        let (before, after) = if horizontal { (11, 13) } else { (7, 17) };
        if let Some(v) = kernel.get_mut(before) {
            *v = -sign;
        }
        if let Some(v) = kernel.get_mut(after) {
            *v = sign;
        }
        let mut rows = [[0.; 4]; 3];
        if let Some(row) = rows.get_mut(component as usize) {
            row[0] = 100.;
        }
        let mut filters = vec![
            native_filter("image.adjustments.channelMixer", json!({"monochrome":true,"gray":[21.26,71.52,7.22,0.]})),
            native_filter("filter.other.custom", json!({"kernel":kernel,"scale":4.,"offset":127.5})),
            native_filter("image.adjustments.channelMixer", json!({"red":rows[0],"green":rows[1],"blue":rows[2]})),
        ];
        let clipped = !branches.is_empty();
        if clipped {
            filters.push(native_filter("filter.other.setAlpha", json!({"value":1.})));
        }
        branches.push(NativeBranch { filters, clipped, blend: if clipped { BlendMode::LinearDodge } else { BlendMode::Normal }, ..Default::default() });
    }
    let mut rows = [[0.; 4]; 3];
    for (index, row) in rows.iter_mut().enumerate() {
        if index != x as usize && index != y as usize {
            row[3] = 100.;
        }
    }
    branches.push(NativeBranch {
        filters: vec![
            native_filter("image.adjustments.channelMixer", json!({"red":rows[0],"green":rows[1],"blue":rows[2]})),
            native_filter("filter.other.setAlpha", json!({"value":1.})),
        ],
        blend: BlendMode::LinearDodge,
        clipped: true,
        ..Default::default()
    });
    let bytes = 17usize.pow(3) * 96 + 8192;
    generation_bytes(c, bytes)?;
    if bytes > c.remaining_metadata_bytes {
        return Err(MappingFailure::resource("native normal-vector LUT exceeds actual remaining metadata allowance"));
    }
    let z = 3 - x - y;
    let lut = super::resources::lookup(17, "Native normalized forward normals", Vec::new(), |input| {
        let vx = (2. * input.get(x as usize).copied().unwrap_or(0.5) - 1.) * scale;
        let vy = (2. * input.get(y as usize).copied().unwrap_or(0.5) - 1.) * scale;
        let maximum = vx.abs().max(vy.abs()).max(1.);
        let nx = vx / maximum;
        let ny = vy / maximum;
        let nz = 1. / maximum;
        let length = nx.hypot(ny).hypot(nz);
        let mut out = [0.; 3];
        if let Some(value) = out.get_mut(x as usize) {
            *value = 0.5 + 0.5 * nx / length;
        }
        if let Some(value) = out.get_mut(y as usize) {
            *value = 0.5 + 0.5 * ny / length;
        }
        if let Some(value) = out.get_mut(z as usize) {
            *value = if full { nz / length } else { 0.5 + 0.5 * nz / length };
        }
        out
    })
    .map_err(invalid)?;
    generation_check(c)?;
    let mut result = composite(
        branches,
        format!(
            "Native channel routing retains original X{x}/Y{stored_y}; equal components use independently observed cyclic Y fallback to{y}. Native unit monochrome central differences and routing feed a standard native17-cube joint vector-normalization lookup with source gain{scale} and constant forward Z before normalization. Max-component scaling keeps finite gain extremes safe without a Custom coefficient dialog cap. Full-Z selects observed unipolar Z rather than default half-range encoding. Native derivative support, working gradient clipping and LUT interpolation/resolution, color/profile and alpha composition differ from source; tileability={tile} follows native boundary policy. All normalization is fixed editable native LUT data, not an imported source evaluator"
        ),
    );
    result.filters = lut.filters;
    result.working_rgb = c.mode == photocraft_color::ColorMode::Grayscale;
    if result.working_rgb {
        result.notes.push("Gray drawable normals retain independent axes in an ordinary RGB/sRGB Smart contents stage. PhotoCraft's standard final RGB-to-Gray conversion projects the displayed result; editable RGB vector components and fixed LUT remain inside native contents, and following stages retain their original Gray working rules".into());
    }
    Ok(result)
}

fn lens_distortion(e: &StoredEffect<'_>, c: &MappingContext) -> Result<NativeMapping> {
    use photocraft_algo::lens::{LensCorrection, LensMap, LensProfile};
    let main = num(e, "main", 0., -100., 100.)?;
    let edge = num(e, "edge", 0., -100., 100.)?;
    let zoom = num(e, "zoom", 0., -100., 100.)?;
    let x_shift = num(e, "x-shift", 0., -100., 100.)?;
    let y_shift = num(e, "y-shift", 0., -100., 100.)?;
    let brighten = num(e, "brighten", 0., -100., 100.)?;
    generation_check(c)?;
    if main == 0. && edge == 0. && zoom == 0. {
        return Ok(mapped(
            Vec::new(),
            "Zero radial/zoom geometry preserves native pixels; optical-centre shifts have no geometry to move and source-probed brighten is inactive with both radial coefficients zero. Original settings remain provenance",
        ));
    }
    let b = bounds(c)?;
    let local = Rect::new(
        0,
        0,
        i32::try_from(b.width()).map_err(|_| invalid("Lens frame width exceeds native coordinates"))?,
        i32::try_from(b.height()).map_err(|_| invalid("Lens frame height exceeds native coordinates"))?,
    );
    let count = usize::try_from(u64::from(b.width()) * u64::from(b.height())).map_err(|_| invalid("Lens map size overflow"))?;
    let mask_bytes = super::residual_transform::coordinate_mask_work_bytes(b)?;
    let work =
        count.checked_mul(64).and_then(|n| n.checked_add(mask_bytes)).and_then(|n| n.checked_add(32768)).ok_or_else(|| invalid("Lens map work overflow"))?;
    let metadata =
        count.checked_mul(8).and_then(|n| n.checked_add(mask_bytes)).and_then(|n| n.checked_add(32768)).ok_or_else(|| invalid("Lens map metadata overflow"))?;
    generation_bytes(c, work)?;
    if metadata > c.remaining_metadata_bytes {
        return Err(MappingFailure::resource("Lens native map/mask/background exceed actual metadata allowance"));
    }
    let native = LensMap::new(
        &LensCorrection {
            profile: Some(LensProfile { name: "Native radial import policy".into(), k1: main / 200., k2: edge / 200., ..Default::default() }),
            scale: 100. * 2f64.powf(zoom / 100.),
            ..Default::default()
        },
        local,
    );
    let sx = f64::from(b.width()) * x_shift / 200.;
    let sy = f64::from(b.height()) * y_shift / 200.;
    let project = |x: f64, y: f64| native.sample(x - sx, y - sy).map(|sample| (sample.p[0] + sx, sample.p[1] + sy));
    let visible = |u: f64, v: f64| {
        project(u, v)
            .is_some_and(|(u, v)| u.is_finite() && v.is_finite() && u >= 0.5 && v >= 0.5 && u <= f64::from(b.width()) - 0.5 && v <= f64::from(b.height()) - 0.5)
    };
    let mut mapped = super::residual_transform::coordinate_map(
        c,
        |x, y| {
            let (u, v) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
            if visible(u, v) { project(u, v).unwrap_or((u, v)) } else { (u, v) }
        },
        "Original standard native Brown-Conrady radial geometry becomes ordinary native Displace data. Independently observed radial directions/relative magnitude select native k1=main/200,k2=edge/200 with native half-diagonal normalization; signed zoom selects native scale100*2^(zoom/100), and shifted optical centre uses source percent of half-width/height. These are explicit native authoring policies, not a source evaluator. Native inverse-map folds, U8 displacement quantization, bilinear resampling, bounded offset range and hard-edge native coverage differ from source interpolation/folded geometry",
    )?;
    let mask = super::residual_transform::coordinate_mask(c, b, |u, v| visible(u * f64::from(b.width()), v * f64::from(b.height())))?;
    let paint = color(e, "background", [0., 0., 0., 0.], c)?;
    let extent = VectorMask::new(Path::new(vec![rect(f64::from(b.x0), f64::from(b.y0), f64::from(b.width()), f64::from(b.height()))]));
    let amount = -brighten * (main + edge) / 200.;
    if amount != 0. {
        mapped.filters.push(native_filter(
            "filter.lensCorrection",
            json!({"profile":"none","distortion":0.,"scale":100.,"vignetteAmount":amount,"vignetteMidpoint":100.,"edge":"extension"}),
        ));
    }
    mapped.structure = Some(NativeStructure::Composite {
        branches: vec![
            NativeBranch { fill: Some(Fill::Solid(paint)), vector_mask: Some(extent), ..Default::default() },
            NativeBranch { filters: std::mem::take(&mut mapped.filters), mask: Some(mask), ..Default::default() },
        ],
    });
    mapped.notes.push("Outside optical samples use finite native Solid Fill with actual source paint ICC converted to the document profile. Native source-over compositing governs translucent pixels/background. Source brighten drives native Lens Correction vignette percent multiplied by negative average signed radial strength: -brighten*(main+edge)/200. Both-zero radial controls leave brighten inactive, independently observed. Native centred smooth exposure falloff/midpoint100 differs from source shifted polynomial energy compensation; opposed coefficients can cancel this chosen native illumination policy. Native alpha/HDR rules apply and original source controls remain historical only".into());
    Ok(mapped)
}
fn long_shadow(e: &StoredEffect<'_>, c: &MappingContext) -> Result<NativeMapping> {
    let b = bounds(c)?;
    let style = choice(e, "style", 0, &[0, 1, 2, 3])?;
    let angle = num(e, "angle", 45., -180., 180.)?.to_radians();
    let length = num(e, "length", 100., 0., f64::MAX)?;
    let midpoint = num(e, "midpoint", 100., 0., f64::MAX)?;
    let relative = num(e, "midpoint-rel", 0.5, 0., 1.)?;
    let composition = choice(e, "composition", 0, &[0, 1, 2])?;
    let mut paint = color(e, "color", [0., 0., 0., 1.], c)?;
    let paint_opacity = paint.alpha;
    paint.alpha = 1.;
    let length = if style == 1 || style == 2 { f64::from(b.width()).hypot(f64::from(b.height())) } else { length };
    if length > 1_000_000. {
        return Err("long shadow placement exceeds bounded native geometry extent".into());
    }
    let count = (length.ceil() as usize).clamp(1, 63);
    let bytes = count.checked_mul(4096).and_then(|v| v.checked_add(4096)).ok_or_else(|| invalid("shadow bytes overflow"))?;
    if bytes > c.remaining_metadata_bytes {
        return Err(MappingFailure::resource("native shadow branches exceed actual remaining metadata allowance"));
    }
    generation_bytes(c, bytes)?;
    let mut branches = Vec::new();
    for n in (1..=count).rev() {
        generation_check(c)?;
        let distance = length * n as f64 / count as f64;
        let opacity = match style {
            2 => {
                if midpoint == 0. {
                    0.
                } else {
                    2f64.powf(-distance / midpoint)
                }
            }
            3 => {
                if relative == 0. {
                    0.
                } else {
                    {
                        let t = distance / length.max(0.001);
                        if t <= relative { 1. - 0.5 * t / relative } else { 0.5 * (1. - t) / (1. - relative) }
                    }
                }
            }
            _ => 1.,
        };
        branches.push(NativeBranch {
            transform: Affine::translate(distance * angle.cos(), distance * angle.sin()),
            opacity: opacity as f32,
            layer_effects: vec![Effect::ColorOverlay { common: FxCommon::new(BlendMode::Normal, paint_opacity), color: paint }],
            ..Default::default()
        });
    }
    if composition == 0 {
        branches.push(NativeBranch::default());
    }
    if composition == 2 {
        branches.push(NativeBranch {
            fill_opacity: 0.,
            advanced: photocraft_doc::AdvancedBlending { knockout: photocraft_doc::Knockout::Shallow, ..Default::default() },
            ..Default::default()
        });
    }
    Ok(composite(
        branches,
        format!(
            "Source extrusion becomes {count} ordinary editable translated owner instances with native Color Overlay. Direction/length, finite versus canvas-bounded infinite style, composition and a native midpoint fade policy become native placements/opacities. Shadow-minus-image uses an original-source copy with native fill opacity0 and shallow knockout confined to the computed shadow document; native source-alpha coverage cuts the translated copies without erasing outside layers. Fixed-length fading uses a native piecewise-linear curve through half coverage at the requested relative midpoint; infinite fading uses half coverage at the requested pixel midpoint. Sparse copies approximate continuous extrusion when length exceeds63px; native alpha/compositing applies; no source shadow renderer"
        ),
    ))
}
fn mirrors(e: &StoredEffect<'_>, c: &MappingContext) -> Result<NativeMapping> {
    let b = bounds(c)?;
    generation_check(c)?;
    let a = num(e, "m-angle", 0., 0., 180.)?.to_radians();
    let rotation = num(e, "r-angle", 0., 0., 360.)?.to_radians();
    let segments = integer(e, "n-segs", 6, 2, 24)?;
    let cx = num(e, "c-x", 0.5, 0., 1.)? * f64::from(b.width()) + f64::from(b.x0);
    let cy = num(e, "c-y", 0.5, 0., 1.)? * f64::from(b.height()) + f64::from(b.y0);
    let ox = num(e, "o-x", 0., -1., 1.)? * f64::from(b.width());
    let oy = num(e, "o-y", 0., -1., 1.)? * f64::from(b.height());
    let tx = num(e, "trim-x", 0., 0., 0.5)?;
    let ty = num(e, "trim-y", 0., 0., 0.5)?;
    let input = num(e, "input-scale", 100., 0.1, 100.)? / 100.;
    let output = num(e, "output-scale", 1., 0., 100.)?;
    let clip = boolean(e, "clip", true)?;
    let warp = boolean(e, "warp", true)?;
    if output == 0. {
        return Ok(replace(
            Layer::new("Native empty mirror result", LayerContent::Fill(Fill::Solid(Color::TRANSPARENT))),
            "Stock output-scale0 produces a zero-area transparent result. Ordinary native transparent Solid Fill preserves that result without constructing a singular Smart placement",
        ));
    }
    // A disclosed native leading-margin crop, in actual owner coordinates. It
    // must be inside each instance BEFORE angular placement, not a destination
    // mask which would leave trimmed pixels available to other mirror sectors.
    let frame = Rect::new(
        b.x0.saturating_add((f64::from(b.width()) * tx).floor() as i32),
        b.y0.saturating_add((f64::from(b.height()) * ty).floor() as i32),
        b.x1,
        b.y1,
    );
    let radius = f64::from(b.width()).hypot(f64::from(b.height())) * 4.;
    let step = std::f64::consts::TAU / f64::from(segments);
    geometry_bytes(c, segments as usize, segments as usize * 18)?;
    let mut branches = Vec::new();
    for index in 0..segments {
        generation_check(c)?;
        let start = a + step * f64::from(index);
        let end = start + step;
        let mut points = vec![(cx + ox, cy + oy)];
        for n in 0..=16 {
            let t = start + (end - start) * f64::from(n) / 16.;
            points.push((cx + ox + radius * t.cos(), cy + oy + radius * t.sin()));
        }
        let reflection = if index % 2 == 0 { Affine::IDENTITY } else { Affine { m: [1., 0., 0., -1., 0., 0.] } };
        let transform = Affine::translate(cx + ox, cy + oy)
            .mul(&Affine::rotate(rotation + step * f64::from(index)))
            .mul(&Affine::scale(output / input))
            .mul(&reflection)
            .mul(&Affine::translate(-cx, -cy));
        branches.push(NativeBranch {
            transform,
            filter_frame: (tx != 0. || ty != 0.).then_some(frame),
            vector_mask: Some(VectorMask::new(Path::new(vec![Subpath::polygon(&points)]))),
            ..Default::default()
        });
    }
    Ok(composite(
        branches,
        format!(
            "Native angular wedge masks and alternating reflected/rotated editable owner instances preserve kaleidoscope intent, segment count, centers and scales. Source trim ratios {tx}/{ty} become an integer leading-margin native input crop {frame:?} inside each instance before angular folding; fractional crop edges round down and original source expanse conventions differ. Source clip={clip} uses the fixed native owner canvas: clip=false source expansion is deliberately superseded, not falsely claimed preserved. Source wrap={warp} uses native transparent resampling outside the cropped input rather than GIMP repeated-edge folding. These explicit native extent/boundary policies, native affine sector geometry and antialiasing remain editable ordinary data"
        ),
    ))
}
// Native ordered-threshold parameter tiles. They contain no source pixels and
// are authored independently from simple line/radial/L1/L-infinity geometry.
fn newsprint_tile(kind: i32, c: &MappingContext) -> Result<photocraft_doc::Pattern> {
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_raster::Surface;
    let mut surface = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
    for y in 0..64 {
        generation_check(c)?;
        let mut row = [0.; 64];
        for (x, value) in row.iter_mut().enumerate() {
            let dx = (((x as f64 + 0.5) / 32.) - 1.).abs();
            let dy = (((f64::from(y) + 0.5) / 32.) - 1.).abs();
            *value = match kind {
                0 => (f64::from(y) + 0.5) / 64.,
                1 => (dx * dx + dy * dy) / 2.,
                2 => (dx + dy) / 2.,
                3 => dx.max(dy),
                _ => dx.min(dy),
            } as f32;
        }
        surface.try_write_region(Rect::new(0, y, 64, y + 1), &row).map_err(|error| MappingFailure::resource(error.to_string()))?;
    }
    photocraft_doc::Pattern::try_new_in(format!("Native geometric screen {kind}"), surface, 64, 64, 64 * 4, &c.interrupt)
        .map_err(|error| MappingFailure::resource(error.to_string()))
}
fn newsprint_ink_filter(index: usize, pullout: f64, c: &MappingContext) -> Result<photocraft_doc::SmartFilter> {
    use photocraft_cms::{Builtin, ColorSpace, Intent, Profile, Transform};
    let fallback = if c.mode == photocraft_color::ColorMode::Grayscale || !c.linear { Builtin::Srgb } else { Builtin::LinearSrgb };
    let profile = if c.mode == photocraft_color::ColorMode::Grayscale { None } else { c.target_profile.as_ref() };
    let source = match profile {
        Some(bytes) => Profile::parse(bytes).map_err(|error| invalid(format!("Native screen source ICC: {error}")))?,
        None => fallback.profile().clone(),
    };
    if source.color_space != ColorSpace::Rgb {
        return Err(invalid("Native CMYK screening requires an actual RGB document profile"));
    }
    let separation = Transform::new(&source, Builtin::CoatedCmyk.profile(), Intent::RelativeColorimetric, true)
        .map_err(|error| invalid(format!("Native screening separation: {error}")))?;
    let cancelled = std::cell::Cell::new(false);
    let mapped = super::resources::lookup(17, "Native screened ink reflectance", Vec::new(), |sample| {
        if c.interrupt.check().is_err() {
            cancelled.set(true);
            return [0.; 3];
        }
        let mut ink = [0.; 4];
        separation.convert_f32(&sample.map(|v| v as f32), 3, &mut ink, 4, false);
        let original_k = f64::from(ink[3]).clamp(0., 1.);
        let k = original_k * pullout;
        if index == 3 {
            return [1. - k; 3];
        }
        // Native GCR policy: preserve each multiplicative ink density while
        // moving the selected fraction of neutral density into the K plate.
        let density = f64::from(ink.get(index).copied().unwrap_or(0.)).clamp(0., 1.) * (1. - original_k) + original_k;
        let remaining = if k >= 1. { 0. } else { ((density - k) / (1. - k)).clamp(0., 1.) };
        let mut rgb = [1.; 3];
        if let Some(value) = rgb.get_mut(index) {
            *value = 1. - remaining;
        }
        rgb
    })
    .map_err(invalid)?;
    if cancelled.get() {
        return Err(MappingFailure::resource("Native screen separation was cancelled"));
    }
    generation_check(c)?;
    mapped.filters.into_iter().next().ok_or_else(|| invalid("Native screen separation has no filter"))
}
fn newsprint(e: &StoredEffect<'_>, c: &MappingContext) -> Result<NativeMapping> {
    let model = choice(e, "color-model", 1, &[0, 1, 2, 3])?;
    let mut periods = [0.; 4];
    let mut angles = [0.; 4];
    let mut kinds = [0; 4];
    for (index, (suffix, default_angle)) in [("2", 15.), ("3", 45.), ("4", 0.), ("", 75.)].into_iter().enumerate() {
        kinds[index] = choice(e, &format!("pattern{suffix}"), 0, &[0, 1, 2, 3, 4])?;
        periods[index] = num(e, &format!("period{suffix}"), 12., 0., 200.)?;
        angles[index] = num(e, &format!("angle{suffix}"), default_angle, -180., 180.)?;
    }
    let pullout = num(e, "black-pullout", 1., 0., 1.)?;
    let aa = integer(e, "aa-samples", 16, 1, 128)?;
    let turbulence = num(e, "turbulence", 0., 0., 1.)?;
    let block = num(e, "blocksize", -1., -1., 64.)?;
    let boost = num(e, "angleboost", 0., 0., 4.)?;
    let gray = c.mode == photocraft_color::ColorMode::Grayscale;
    if !matches!(c.mode, photocraft_color::ColorMode::Rgb | photocraft_color::ColorMode::Grayscale) {
        return Err(invalid("Native screening requires an RGB or Gray owner"));
    }
    let effective = model;
    let count = match effective {
        0 | 1 => 1usize,
        2 => 3,
        _ => 4,
    };
    // Charge actual256px padded Gray F32 tiles, prepared RGBA copies, encoded
    // data/clone allowance, hash rows and branch/style/lookup metadata BEFORE
    // any pattern or CMYK parameter resource is authored.
    let per_pattern = 256 * 256 * 4 + 64 * 64 * (16 + 4 * 4) + 64 * 4 + 16_384;
    let bytes = count * per_pattern + if effective == 3 { 4 * 17usize.pow(3) * 96 } else { 0 } + (count + 1) * 8192;
    generation_bytes(c, bytes)?;
    if bytes > c.remaining_metadata_bytes {
        return Err(MappingFailure::resource("Native screen patterns/styles/separations exceed remaining metadata allowance"));
    }
    let white = effective == 3;
    // Base Color Overlay would repaint the completed clipping group under
    // native style ordering. A native constant Curves filter acts INSIDE the
    // owner first, leaving the subsequent screen clipping group untouched.
    let mut branches = vec![NativeBranch {
        filters: vec![native_filter("image.adjustments.curves", json!({"pencil":true,"points":vec![if white {255.} else {0.};256]}))],
        ..Default::default()
    }];
    let mut patterns = Vec::new();
    for channel in 0..count {
        generation_check(c)?;
        let selected = if count == 1 { 3 } else { channel };
        let mut filters = Vec::new();
        if effective == 3 {
            filters.push(newsprint_ink_filter(channel, pullout, c)?);
        } else if count == 1 {
            filters.push(native_filter("image.adjustments.channelMixer", json!({"monochrome":true,"gray":[21.26,71.52,7.22,0.]})));
            if effective == 0 {
                filters.push(native_filter("image.adjustments.invert", json!({})));
            }
        } else {
            let mut rows = [[0.; 4]; 3];
            if let Some(value) = rows.get_mut(channel).and_then(|row| row.get_mut(channel)) {
                *value = 100.;
            }
            filters.push(native_filter("image.adjustments.channelMixer", json!({"red":rows[0],"green":rows[1],"blue":rows[2]})));
        }
        filters.push(native_filter("filter.other.setAlpha", json!({"value":1.})));
        let mut styles = Vec::new();
        if periods[selected] == 0. {
            let value = if kinds[selected] == 3 { 0. } else { 1. };
            let mut rgb = if effective == 3 { [1.; 3] } else { [0.; 3] };
            if count == 1 || channel == 3 {
                rgb.fill(value);
            } else if let Some(component) = rgb.get_mut(channel) {
                *component = value;
            }
            styles.push(Effect::ColorOverlay { common: FxCommon::new(BlendMode::Normal, 1.), color: Color::rgb(rgb[0], rgb[1], rgb[2]) });
        } else {
            let pattern = newsprint_tile(kinds[selected], c)?;
            styles.push(Effect::PatternOverlay {
                common: FxCommon::new(BlendMode::HardMix, 1.),
                name: pattern.name.clone(),
                id: pattern.id.clone(),
                scale: (periods[selected].max(1.) / 64.) as f32,
                angle: angles[selected] as f32,
                link: false,
                phase: (0., 0.),
            });
            patterns.push(pattern);
        }
        if effective == 0 {
            styles.push(Effect::ColorOverlay { common: FxCommon::new(BlendMode::Difference, 1.), color: Color::WHITE });
        }
        branches.push(NativeBranch {
            filters,
            clipped: true,
            blend: if effective == 3 {
                BlendMode::Multiply
            } else if count == 1 {
                BlendMode::Normal
            } else {
                BlendMode::LinearDodge
            },
            layer_effects: styles,
            ..Default::default()
        });
    }
    generation_check(c)?;
    Ok(NativeMapping {
        patterns,
        working_rgb: gray && model >= 2,
        structure: Some(NativeStructure::Composite { branches }),
        notes: vec![format!(
            "Independent native geometric64px threshold Pattern tiles and ordinary Hard Mix Pattern Overlay produce line/circular/diamond/square/cross ordered screens. Each selected channel has its own editable period, pattern and native counter-clockwise angle. Native bilinear pattern sampling replaces source screen growth/AA; positive periods below1px use an explicit native1px sampling floor. Source monochrome period0 observes a white endpoint (square black endpoint); other plates use this declared native zero-period endpoint policy. RGB uses channel-isolated clipped Add over a black source-shaped base; CMYK uses actual document-ICC→native coated-CMYK separation and multiplicative native ink reflectance, with black-pullout{pullout} selecting a disclosed native gray-component-replacement policy. Opaque intermediate screen branches clip to the original source shape, preserving native base alpha. Native depth/HDR/gamut and Photoshop blend rules govern results. Source aa-samples{aa}, stochastic turbulence{turbulence}, blocksize{block} and angleboost{boost} are superseded by deterministic native geometric screen quality, not evaluated as a GIMP screen algorithm. Gray source color-model{model} colored plates use ordinary RGB/sRGB Smart working contents followed by ordinary native conversion back to the Gray owner; independent plate controls are preserved. Source monochrome stays native Gray. Original controls remain historical"
        )],
        ..Default::default()
    })
}

pub(super) fn random(state: &mut u32) -> u32 {
    *state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    *state
}
fn maze(e: &StoredEffect<'_>, c: &MappingContext) -> Result<NativeMapping> {
    let b = bounds(c)?;
    let sx = integer(e, "x", 16, 1, i32::MAX)?;
    let sy = integer(e, "y", 16, 1, i32::MAX)?;
    let algorithm = choice(e, "algorithm-type", 0, &[0, 1])?;
    let tile = boolean(e, "tileable", false)?;
    let mut rng = seed(e)?;
    let fg = color(e, "fg-color", [0., 0., 0., 1.], c)?;
    let bg = color(e, "bg-color", [1., 1., 1., 1.], c)?;
    let nx = (i64::from(b.width()) + i64::from(sx) - 1) / i64::from(sx);
    let ny = (i64::from(b.height()) + i64::from(sy) - 1) / i64::from(sy);
    let count = nx.checked_mul(ny).and_then(|n| usize::try_from(n).ok()).ok_or_else(|| invalid("maze cell count overflow"))?;
    if count.checked_mul(4).is_none_or(|n| n > MAX_GEOMETRY) {
        return Err("native maze exceeds65,536 bounded cell/wall work items".into());
    }
    geometry_bytes(
        c,
        count.checked_mul(4).ok_or_else(|| invalid("maze geometry overflow"))?,
        count.checked_mul(16).ok_or_else(|| invalid("maze knot count overflow"))?,
    )?;
    let mut walls = vec![15u8; count];
    let mut seen = vec![false; count];
    let mut frontier = vec![0usize];
    if let Some(v) = seen.first_mut() {
        *v = true;
    }
    while !frontier.is_empty() {
        generation_check(c)?;
        let index = if algorithm == 0 { frontier.len() - 1 } else { random(&mut rng) as usize % frontier.len() };
        let cell = *frontier.get(index).ok_or_else(|| invalid("maze frontier"))?;
        let cell_i = i64::try_from(cell).map_err(|_| invalid("maze cell index"))?;
        let x = cell_i % nx;
        let y = cell_i / nx;
        let mut next = Vec::new();
        for (dx, dy, bit, opposite) in [(0, -1, 1u8, 4u8), (1, 0, 2, 8), (0, 1, 4, 1), (-1, 0, 8, 2)] {
            let mut px = x + dx;
            let mut py = y + dy;
            if tile {
                px = px.rem_euclid(nx);
                py = py.rem_euclid(ny);
            } else if px < 0 || px >= nx || py < 0 || py >= ny {
                continue;
            }
            let dest = usize::try_from(py * nx + px).map_err(|_| invalid("maze neighbor"))?;
            if !seen.get(dest).copied().ok_or_else(|| invalid("maze visited index"))? {
                next.push((dest, bit, opposite));
            }
        }
        if next.is_empty() {
            frontier.swap_remove(index);
            continue;
        }
        let n = random(&mut rng) as usize % next.len();
        let (dest, bit, opposite) = next.get(n).copied().ok_or_else(|| invalid("maze random neighbor"))?;
        *walls.get_mut(cell).ok_or_else(|| invalid("maze source cell"))? &= !bit;
        *walls.get_mut(dest).ok_or_else(|| invalid("maze destination cell"))? &= !opposite;
        *seen.get_mut(dest).ok_or_else(|| invalid("maze mark"))? = true;
        frontier.push(dest);
    }
    let mut paths = Vec::new();
    for (cell, wall) in walls.iter().enumerate() {
        generation_check(c)?;
        let cell_i = cell as i64;
        let x = f64::from(b.x0) + (cell_i % nx) as f64 * f64::from(sx);
        let y = f64::from(b.y0) + (cell_i / nx) as f64 * f64::from(sy);
        let w = f64::from(sx).min(f64::from(b.x1) - x);
        let h = f64::from(sy).min(f64::from(b.y1) - y);
        if wall & 1 != 0 {
            paths.push(rect(x, y, w, 1.));
        }
        if wall & 8 != 0 {
            paths.push(rect(x, y, 1., h));
        }
        if cell_i / nx == ny - 1 && wall & 4 != 0 {
            paths.push(rect(x, y + h - 1., w, 1.));
        }
        if cell_i % nx == nx - 1 && wall & 2 != 0 {
            paths.push(rect(x + w - 1., y, 1., h));
        }
    }
    let mut background = Layer::new("Maze background", LayerContent::Fill(Fill::Solid(bg)));
    background.vector_mask = Some(VectorMask::new(Path::new(vec![rect(f64::from(b.x0), f64::from(b.y0), f64::from(b.width()), f64::from(b.height()))])));
    let layer = Layer::new(
        "Imported editable maze",
        LayerContent::Group(Group { children: vec![background, shape("Maze walls", paths, fg)], expanded: true, artboard: None }),
    );
    Ok(replace(
        layer,
        format!(
            "Native editable maze walls use source cell spacing/seed/colors and {} spanning-tree construction; source tileability={tile} connects opposite-edge cells. Independently specified native random sequence, one-pixel wall width and antialiasing differ from source maze rasterization; all generated geometry remains editable",
            if algorithm == 0 { "depth-first" } else { "random frontier" }
        ),
    ))
}
fn sinusoid(e: &StoredEffect<'_>, c: &MappingContext) -> Result<NativeMapping> {
    let b = bounds(c)?;
    let px = num(e, "x-period", 128., 0., f64::MAX)?;
    let py = num(e, "y-period", 128., 0., f64::MAX)?;
    let ax = num(e, "x-amplitude", 0., -f64::MAX, f64::MAX)?;
    let ay = num(e, "y-amplitude", 0., -f64::MAX, f64::MAX)?;
    let phx = num(e, "x-phase", 0., -f64::MAX, f64::MAX)?;
    let phy = num(e, "y-phase", 0., -f64::MAX, f64::MAX)?;
    let angle = num(e, "angle", 90., 0., 360.)?;
    let offset = num(e, "offset", 0., -f64::MAX, f64::MAX)?;
    let exponent = num(e, "exponent", 0., -f64::MAX, f64::MAX)?;
    let ox = num(e, "x-offset", 0., -f64::MAX, f64::MAX)?;
    let oy = num(e, "y-offset", 0., -f64::MAX, f64::MAX)?;
    let rotation = num(e, "rotation", 0., 0., 360.)?;
    let aa = integer(e, "supersampling", 1, 1, 8)?;
    if px == 0. || py == 0. {
        let power = 2f64.powf(exponent);
        let value = (0.5 + offset).powf(power);
        if !power.is_finite() || power == 0. || !value.is_finite() || value.abs() > f64::from(f32::MAX) {
            return Err(MappingFailure::resource("degenerate sinusoid native constant exceeds finite native paint representation"));
        }
        let paint = super::resources::native_rgba([value as f32, value as f32, value as f32, 1.], c).map_err(invalid)?;
        return Ok(replace(
            Layer::new("Native constant sinusoid", LayerContent::Fill(Fill::Solid(paint))),
            "Independent stock probes establish that either zero period makes both spatial axes inactive and selects uniform base0.5, regardless other-axis phase/amplitude. Native constant-paint policy applies the retained offset and exponential power once, then CMS-converts the finite paint into this document. Observed offset/exponent controls support this policy at recorded probe points; native HDR/ICC/range behavior remains authoritative. Position/rotation/axis/supersampling controls have no geometry in this degenerate native Solid Fill; no source evaluator persists",
        ));
    }
    let spread = 0.5 + 0.25 * (2f64.powf(ax) + 2f64.powf(ay));
    if !spread.is_finite() || offset.abs() + spread > f64::from(f32::MAX) {
        return Err(MappingFailure::resource("sinusoid axes plus requested offset exceed finite native F32 paint/filter representation"));
    }
    let power = 2f64.powf(exponent);
    if !power.is_finite() || power == 0. {
        return Err("sinusoid post-sum exponent exceeds finite native curve representation".into());
    }
    let mut required = 8192usize;
    for (period, axis) in [(px, rotation), (py, rotation + angle)] {
        let (dy, dx) = axis.to_radians().sin_cos();
        let span = f64::from(b.width()) * dx.abs() + f64::from(b.height()) * dy.abs();
        let count = (span / period * 32.).ceil().max(2.);
        if count > MAX_GEOMETRY as f64 / 2. {
            return Err("sinusoid editable stops exceed native work allowance".into());
        }
        required = required
            .checked_add((count as usize + 1).checked_mul(std::mem::size_of::<(f32, Color)>() * 2).ok_or_else(|| invalid("gradient stop bytes overflow"))?)
            .ok_or_else(|| invalid("gradient allocation overflow"))?;
    }
    if required > c.remaining_metadata_bytes {
        return Err(MappingFailure::resource("native gradient stops exceed actual remaining metadata allowance"));
    }
    generation_bytes(c, required)?;
    let mapper = super::resources::NativeColorMapper::new(c).map_err(invalid)?;
    let mut children = Vec::new();
    for (period, amplitude, phase, axis) in [(px, ax, phx, rotation), (py, ay, phy, rotation + angle)] {
        let rad = axis.to_radians();
        let (dy, dx) = rad.sin_cos();
        let span = f64::from(b.width()) * dx.abs() + f64::from(b.height()) * dy.abs();
        let chord = (f64::from(b.width()) / dx.abs().max(1e-6)).min(f64::from(b.height()) / dy.abs().max(1e-6)).max(1.);
        let count = (span / period * 32.).ceil().max(2.);
        if count > MAX_GEOMETRY as f64 / 2. {
            return Err("sinusoid editable gradient stops exceed bounded native geometry work allowance".into());
        }
        let count = count as u32;
        let gain = 2f64.powf(amplitude);
        if !gain.is_finite() || gain > f64::from(f32::MAX) / 4. {
            return Err("sinusoid amplitude exceeds finite native color representation".into());
        }
        let center = (f64::from(b.x0) + f64::from(b.width()) / 2. - ox) * dx + (f64::from(b.y0) + f64::from(b.height()) / 2. - oy) * dy;
        let mut stops = Vec::new();
        for n in 0..=count {
            generation_check(c)?;
            let t = f64::from(n) / f64::from(count);
            let value = (0.25 - 0.25 * gain * (std::f64::consts::TAU * (center + (t - 0.5) * span + phase) / period).cos()) as f32;
            let paint = mapper.convert([value, value, value, 1.]).map_err(invalid)?;
            stops.push((t as f32, paint));
        }
        let fill = Fill::gradient(stops, -axis as f32, (span / chord) as f32, photocraft_doc::GradientStyle::Linear, false);
        let mut layer = Layer::new("Sinusoid axis gradient", LayerContent::Fill(fill));
        layer.blend = if children.is_empty() { BlendMode::Normal } else { BlendMode::LinearDodge };
        children.push(layer);
    }
    let mut mapping = replace(
        Layer::new("Imported sinusoid gradients", LayerContent::Group(Group { children, expanded: true, artboard: None })),
        format!(
            "Independent stock observations establish period/phase in pixels and amplitude gain2^a. Two editable native linear gradients approximate the axes with32 linear stops per period and native additive composition; source angle/rotation/offset position are explicit. Native endpoint snapping, ICC interpolation, inter-axis clipping and antialiasing replace source supersampling{aa}; no bitmap is baked"
        ),
    );
    if offset != 0. {
        if (-2.0..=2.0).contains(&offset) {
            mapping.filters.push(native_filter(
                "image.adjustments.channelMixer",
                json!({"red":[100.,0.,0.,offset*100.],"green":[0.,100.,0.,offset*100.],"blue":[0.,0.,100.,offset*100.],"gray":[100.,0.,0.,offset*100.]}),
            ));
        } else {
            let mut kernel = [0.; 25];
            if let Some(value) = kernel.get_mut(12) {
                *value = 1.;
            }
            let mut channels = vec![true; c.mode.color_channels() + 1];
            if let Some(alpha) = channels.last_mut() {
                *alpha = false;
            }
            mapping.filters.push(native_filter(
                "filter.other.convolution",
                json!({"kernel":kernel,"scale":1.,"offset":offset,"channels":channels,"alphaWeighted":false,"edge":"repeat"}),
            ));
        }
    }
    if exponent != 0. {
        let table: Vec<_> = (0..256).map(|n| 255. * (f64::from(n) / 255.).powf(power)).collect();
        mapping.filters.push(native_filter("image.adjustments.curves", json!({"pencil":true,"points":table})));
    }
    mapping.notes.push("Source offset and exponential power apply once after adding both native axes. Native Channel Mixer handles its ordinary constant range; larger finite offsets use ordinary native identity Convolution with color channels selected and alpha unchanged, avoiding a dialog cap. Native working-color/polarity and storage-depth rules apply. The ordinary256-point pencil Curve supports finite positive exp2(power); finite-range curve/HDR extrapolation differs explicitly and unrepresentable exp2 values remain reported rather than silently clamped".into());
    Ok(mapping)
}

fn illusion(e: &StoredEffect<'_>, c: &MappingContext) -> Result<NativeMapping> {
    let division = integer(e, "division", 8, 0, 64)?;
    let kind = choice(e, "illusion-type", 0, &[0, 1])?;
    let b = bounds(c)?;
    let cycles = f64::from(division.max(1));
    let cx = f64::from(b.width()) * 0.5;
    let cy = f64::from(b.height()) * 0.5;
    let radius = cx.min(cy).max(0.5);
    super::residual_transform::coordinate_map(
        c,
        |x, y| {
            let dx = f64::from(x) + 0.5 - cx;
            let dy = f64::from(y) + 0.5 - cy;
            let r = dx.hypot(dy);
            let angle = dy.atan2(dx);
            let direction = if kind == 0 { 1. } else { -1. };
            // Independently authored native polar fan: smooth alternating sector refraction.
            let wave = (cycles * angle).sin();
            let turn = direction * std::f64::consts::PI / (2. * cycles) * wave * (r / radius).min(1.);
            let distance = (r + radius * 0.125 * wave).max(0.);
            (cx + distance * (angle + turn).cos(), cy + distance * (angle + turn).sin())
        },
        "Radial illusion intent becomes an independently authored native polar-fan Displace map: source division controls fan repetition and type reverses its angular direction. Division0 uses an explicit native one-cycle policy, not identity. Native alternating radial/sector refraction differs from source geometry; native alpha is preserved whereas stock source observations halve opaque input alpha. Native embedded U8 displacement quantization, bilinear sampling, wrapped boundaries and bounded offset range govern edits; no source evaluator remains",
    )
}

pub(super) fn map(e: &StoredEffect<'_>, c: &MappingContext) -> MappingResult {
    if !matches!(
        e.canonical_operation(),
        "gegl:grid"
            | "gegl:lens-flare"
            | "gegl:inner-glow"
            | "gegl:oilify"
            | "gegl:image-gradient"
            | "gegl:mantiuk06"
            | "gegl:noise-solid"
            | "gegl:mosaic"
            | "gegl:normal-map"
            | "gegl:lens-distortion"
            | "gegl:long-shadow"
            | "gegl:mirrors"
            | "gegl:newsprint"
            | "gegl:maze"
            | "gegl:linear-sinusoid"
            | "gegl:illusion"
    ) {
        return None;
    }
    if let Err(error) = super::residual_spatial::generation_bytes(c, 4096) {
        return Some(Err(error));
    }
    Some(match e.canonical_operation() {
        "gegl:grid" => grid(e, c),
        "gegl:lens-flare" => flare(e, c),
        "gegl:inner-glow" => glow(e, c),
        "gegl:oilify" => oil(e, c),
        "gegl:image-gradient" => image_gradient(e, c),
        "gegl:mantiuk06" => local_tone(e),
        "gegl:noise-solid" => solid_noise(e, c),
        "gegl:mosaic" => mosaic(e, c),
        "gegl:normal-map" => normal_map(e, c),
        "gegl:lens-distortion" => lens_distortion(e, c),
        "gegl:long-shadow" => long_shadow(e, c),
        "gegl:mirrors" => mirrors(e, c),
        "gegl:newsprint" => newsprint(e, c),
        "gegl:maze" => maze(e, c),
        "gegl:linear-sinusoid" => sinusoid(e, c),
        "gegl:illusion" => illusion(e, c),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::super::{Argument, ColorLiteral};
    use super::*;
    use photocraft_color::ColorMode;
    fn context() -> MappingContext<'static> {
        MappingContext {
            mode: ColorMode::Rgb,
            linear: false,
            owner_bounds: Rect::from_xywh(0, 0, 64, 48),
            native_filter_bounds: Rect::from_xywh(0, 0, 64, 48),
            target_profile: None,
            resolution_dpi: 72.0,
            remaining_generation_bytes: super::super::super::MAX_PIXEL_BYTES,
            remaining_metadata_bytes: crate::xcf::MAX_METADATA_BYTES,
            interrupt: photocraft_raster::Interrupt::NONE,
        }
    }
    fn int(name: &'static str, value: i32) -> Argument<'static> {
        Argument { name, value: ArgumentValue::Int(value) }
    }
    fn float(name: &'static str, value: f32) -> Argument<'static> {
        Argument { name, value: ArgumentValue::Float(value) }
    }
    fn enm(name: &'static str, value: i32) -> Argument<'static> {
        Argument { name, value: ArgumentValue::Enum(value) }
    }
    fn result(op: &'static str, args: Vec<Argument<'static>>) -> NativeMapping {
        map(&StoredEffect::test(op, args), &context()).unwrap().unwrap()
    }
    fn paint(name: &'static str) -> Argument<'static> {
        Argument { name, value: ArgumentValue::Color(Some(ColorLiteral { encoding: "R'G'B'A u8", pixels: &[128, 64, 32, 128], profile: &[] })) }
    }
    #[test]
    fn finite_batch_recognizes_all_sixteen_ids_without_default_filter_substitutions() {
        let mut converted = 0;
        for op in [
            "gegl:grid",
            "gegl:illusion",
            "gegl:image-gradient",
            "gegl:inner-glow",
            "gegl:lens-distortion",
            "gegl:lens-flare",
            "gegl:linear-sinusoid",
            "gegl:long-shadow",
            "gegl:mantiuk06",
            "gegl:maze",
            "gegl:mirrors",
            "gegl:mosaic",
            "gegl:newsprint",
            "gegl:noise-solid",
            "gegl:normal-map",
            "gegl:oilify",
        ] {
            let mapping = map(&StoredEffect::test(op, Vec::new()), &context()).unwrap();
            if let Ok(mapping) = mapping {
                converted += 1;
                assert!(!mapping.notes.is_empty(), "{op}");
                assert!(mapping.filters.iter().all(|f| !f.command.starts_with("gegl:")), "{op}");
            }
        }
        assert_eq!(converted, 16);
        assert!(map(&StoredEffect::test("not-our-batch", Vec::new()), &context()).is_none());
    }
    #[test]
    fn grid_origin_negative_offsets_and_zero_width_become_editable_clipped_rectangles() {
        let mut c = context();
        c.native_filter_bounds = Rect::from_xywh(0, 0, 10, 8);
        let mapping =
            grid(&StoredEffect::test("gegl:grid", vec![int("x", 4), int("y", 4), int("x-offset", -1), int("line-width", 2), int("line-height", 0)]), &c)
                .unwrap();
        let Some(NativeStructure::Replace { layer, .. }) = mapping.structure else { panic!("shape structure") };
        let LayerContent::Shape(shape) = layer.content else { panic!("shape") };
        assert_eq!(shape.path.subpaths.len(), 3);
        assert_eq!(shape.path.subpaths[0].knots[0].anchor.x, 0.);
        assert_eq!(shape.path.subpaths[0].knots[1].anchor.x, 1.);
        assert_eq!(shape.path.subpaths[1].knots[0].anchor.x, 3.);
        assert_eq!(shape.path.subpaths[1].knots[1].anchor.x, 5.);
        let empty = grid(&StoredEffect::test("gegl:grid", vec![int("line-width", 0), int("line-height", 0)]), &c).unwrap();
        let Some(NativeStructure::Replace { layer, .. }) = empty.structure else { panic!("shape") };
        let LayerContent::Shape(shape) = layer.content else { panic!("shape") };
        assert!(shape.path.is_empty());
    }
    #[test]
    fn maze_is_native_editable_geometry_and_seed_deterministic() {
        let a = result("gegl:maze", Vec::new());
        let b = result("gegl:maze", Vec::new());
        let (Some(NativeStructure::Replace { layer: a, .. }), Some(NativeStructure::Replace { layer: b, .. })) = (a.structure, b.structure) else {
            panic!("maze structure")
        };
        let (LayerContent::Group(a), LayerContent::Group(b)) = (&a.content, &b.content) else { panic!("maze group") };
        assert_eq!(a.children.len(), 2);
        assert_eq!(a.children[1].content, b.children[1].content);
        assert!(matches!(a.children[0].content, LayerContent::Fill(_)));
        assert!(matches!(a.children[1].content, LayerContent::Shape(_)));
    }
    #[test]
    fn sinusoid_uses_native_additive_gradient_axes_not_raster_caches() {
        let mapping = result("gegl:linear-sinusoid", vec![float("x-period", 8.), float("y-period", 16.), float("x-phase", 1.)]);
        let Some(NativeStructure::Replace { layer, .. }) = mapping.structure else { panic!("gradient structure") };
        let LayerContent::Group(g) = layer.content else { panic!("gradient group") };
        assert_eq!(g.children.len(), 2);
        assert_eq!(g.children[1].blend, BlendMode::LinearDodge);
        for layer in g.children {
            let LayerContent::Fill(Fill::Gradient { stops, .. }) = layer.content else { panic!("native gradient") };
            assert!(stops.len() > 2);
            assert!(stops.windows(2).all(|p| p[0].0 < p[1].0));
        }
    }
    #[test]
    fn component_alpha_moves_to_native_style_opacity_and_positions_are_editable() {
        let mapping = result("gegl:inner-glow", vec![paint("value"), float("opacity", 0.5)]);
        let Effect::InnerGlow(g) = &mapping.layer_effects[0] else { panic!("glow") };
        assert!((g.common.opacity - 0.5 * 128. / 255.).abs() < 1e-6);
        let FxPaint::Color(p) = g.paint else { panic!("color") };
        assert_eq!(p.alpha, 1.);
        let shadow = result("gegl:long-shadow", vec![paint("color"), float("length", 4.)]);
        let Some(NativeStructure::Composite { branches }) = shadow.structure else { panic!("shadow") };
        assert_eq!(branches.len(), 5);
        assert_eq!(branches[0].transform.m[4], 4. * 45f64.to_radians().cos());
        let Effect::ColorOverlay { common, color } = &branches[0].layer_effects[0] else { panic!("paint") };
        assert!((common.opacity - 128. / 255.).abs() < 1e-6);
        assert_eq!(color.alpha, 1.);
        let mirror = result("gegl:mirrors", vec![int("n-segs", 4)]);
        let Some(NativeStructure::Composite { branches }) = mirror.structure else { panic!("mirror") };
        assert_eq!(branches.len(), 4);
        assert!(branches.iter().all(|b| b.vector_mask.is_some()));
        assert!(branches[1].transform.determinant() < 0.);
    }
    #[test]
    fn illusion_native_fan_embeds_both_types_and_zero_cycle_policy_with_real_budgets() {
        let first = result("gegl:illusion", Vec::new());
        assert_eq!(first.filters[0].command, "filter.distort.displace");
        let second = result("gegl:illusion", vec![enm("illusion-type", 1)]);
        assert_ne!(first.filters[0].params, second.filters[0].params);
        let zero = result("gegl:illusion", vec![int("division", 0)]);
        assert!(zero.notes[0].contains("one-cycle"));
        assert!(zero.notes[0].contains("halve opaque input alpha"));
        let small = MappingContext { remaining_metadata_bytes: 1, ..context() };
        assert!(map(&StoredEffect::test("gegl:illusion", Vec::new()), &small).unwrap().is_err());
    }
    #[test]
    fn shadow_minus_image_uses_native_shallow_knockout_and_preserves_its_scope_on_reopen() {
        let c = context();
        let result = long_shadow(&StoredEffect::test("gegl:long-shadow", vec![enm("composition", 2), float("length", 4.), float("angle", 0.)]), &c).unwrap();
        let Some(NativeStructure::Composite { branches }) = result.structure else { panic!("native shadow copies") };
        assert_eq!(branches.last().unwrap().fill_opacity, 0.);
        assert_eq!(branches.last().unwrap().advanced.knockout, photocraft_doc::Knockout::Shallow);
        let mut doc = photocraft_doc::Document::new(
            "native shadow knockout",
            photocraft_geom::Size::new(32, 24),
            photocraft_color::ColorMode::Rgb,
            photocraft_color::SampleType::F32,
        );
        let mut source = photocraft_raster::Surface::new(doc.pixel_format());
        source.fill_rect(photocraft_geom::Rect::new(8, 8, 12, 12), &[1., 0., 0., 1.]);
        for branch in branches {
            let placed = photocraft_algo::warp::place_source(&source, doc.bounds(), &branch.transform, None).unwrap();
            let mut layer = Layer::new("native shadow branch", LayerContent::Raster(placed));
            layer.effects.items = branch.layer_effects;
            layer.opacity = branch.opacity;
            layer.fill_opacity = branch.fill_opacity;
            layer.advanced = branch.advanced;
            doc.layers.push(layer);
        }
        let out = photocraft_compose::render(&doc, doc.bounds()).px;
        for y in 8..12 {
            for x in 8..12 {
                assert_eq!(out[y * 32 + x][3], 0.);
            }
        }
        assert_eq!(out[9 * 32 + 14][3], 1.);
        let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
        assert_eq!(out, photocraft_compose::render(&reopened, reopened.bounds()).px);
    }
    #[test]
    fn mosaic_controls_author_native_polygons_joint_paint_and_persisted_relief() {
        use photocraft_color::{ColorMode, PixelFormat, SampleType};
        use photocraft_doc::{Document, Layer, LayerContent};
        use photocraft_geom::Size;
        use photocraft_raster::Surface;
        let c = MappingContext { native_filter_bounds: Rect::from_xywh(0, 0, 32, 24), owner_bounds: Rect::from_xywh(0, 0, 32, 24), ..context() };
        let mapping = mosaic(
            &StoredEffect::test(
                "gegl:mosaic",
                vec![
                    enm("tile-type", 0),
                    float("color-variation", 0.),
                    float("tile-neatness", 1.),
                    float("tile-spacing", 2.),
                    float("tile-height", 1.),
                    Argument { name: "tile-allow-split", value: ArgumentValue::Bool(false) },
                ],
            ),
            &c,
        )
        .unwrap();
        let Some(NativeStructure::Composite { branches }) = mapping.structure else { panic!("native mosaic cells") };
        // Four cells have interior; the2px fractional column is entirely joints.
        assert_eq!(branches.len(), 5);
        assert!(branches[0].fill.is_some());
        assert!(branches[1..].iter().all(|b| b.filter_frame.is_some() && b.vector_mask.is_some() && b.filters[0].command == "filter.blur.average"));
        let mut doc = Document::new("Native polygon mosaic", Size::new(32, 24), ColorMode::Rgb, SampleType::F32);
        for b in branches {
            let content = if let Some(fill) = b.fill {
                LayerContent::Fill(fill)
            } else {
                let mut s = Surface::new(PixelFormat::RGBA32F);
                s.fill_rect(doc.bounds(), &[1., 0., 0., 1.]);
                LayerContent::Raster(s)
            };
            let mut layer = Layer::new("Editable native cell", content);
            layer.vector_mask = b.vector_mask;
            layer.effects.items = b.layer_effects;
            layer.effects.enabled = true;
            doc.layers.push(layer);
        }
        let before = photocraft_compose::render(&doc, doc.bounds()).px;
        assert_eq!(before[0], [0., 0., 0., 1.]);
        assert!(before[7 * 32 + 7][0] > 0.5);
        let saved = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
        assert_eq!(before, photocraft_compose::render(&reopened, reopened.bounds()).px);
        for kind in 0..4 {
            let m = result("gegl:mosaic", vec![enm("tile-type", kind), float("tile-size", 1000.), float("tile-height", 1.)]);
            let Some(NativeStructure::Composite { branches }) = m.structure else { panic!() };
            assert_eq!(branches.len(), 3);
            assert!(branches[1].vector_mask.is_some());
        }
        let empty = result("gegl:mosaic", vec![float("tile-size", 1.), float("tile-spacing", 1000.)]);
        let Some(NativeStructure::Replace { layer, .. }) = empty.structure else { panic!("native joints-only fill") };
        assert!(matches!(layer.content, LayerContent::Fill(Fill::Solid(_))));
        let tiny = MappingContext { remaining_generation_bytes: 1, ..context() };
        assert!(map(&StoredEffect::test("gegl:mosaic", Vec::new()), &tiny).unwrap().is_err());
    }
    #[test]
    fn zero_axis_sinusoid_is_native_constant_paint_with_observed_post_controls() {
        use photocraft_color::{ColorMode, SampleType};
        use photocraft_doc::Document;
        use photocraft_geom::Size;
        for (args, expected) in [
            (vec![float("x-period", 0.)], 0.5),
            (vec![float("y-period", 0.), float("x-amplitude", 7.), float("x-phase", 1.)], 0.5),
            (vec![float("x-period", 0.), float("offset", 0.2), float("exponent", 1.)], 0.49),
        ] {
            let mapping = result("gegl:linear-sinusoid", args);
            let Some(NativeStructure::Replace { layer, .. }) = mapping.structure else { panic!("native constant fill") };
            let LayerContent::Fill(Fill::Solid(paint)) = &layer.content else { panic!("native solid paint") };
            assert!(paint.alpha == 1. && paint.c.iter().all(|n| n.is_finite()));
            assert!((paint.c[0] - expected).abs() < 1e-5);
            let mut document = Document::new("Native constant sinusoid", Size::new(3, 2), ColorMode::Rgb, SampleType::F32);
            document.layers.push(*layer);
            let rendered = photocraft_compose::render(&document, document.bounds()).px;
            assert!(rendered.windows(2).all(|p| p[0] == p[1]));
            let bytes = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
            assert_eq!(rendered, photocraft_compose::render(&reopened, reopened.bounds()).px);
        }
    }
    #[test]
    fn sinusoid_large_offsets_use_native_color_convolution_without_alpha_or_dialog_clamping() {
        use photocraft_color::{ColorMode, PixelFormat, SampleType};
        use photocraft_raster::{Interrupt, Surface};
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            let c = MappingContext { mode, ..context() };
            for offset in [-3., 3., f32::MAX] {
                let mapping = sinusoid(&StoredEffect::test("gegl:linear-sinusoid", vec![float("offset", offset)]), &c).unwrap();
                let filter = &mapping.filters[0];
                assert_eq!(filter.command, "filter.other.convolution");
                assert_eq!(filter.params["offset"], json!(f64::from(offset)));
                let params = photocraft_algo::convolution::Params {
                    kernel: filter.params["kernel"].as_array().unwrap().iter().map(|n| n.as_f64().unwrap()).collect(),
                    scale: 1.,
                    offset: f64::from(offset),
                    channels: filter.params["channels"].as_array().unwrap().iter().map(|v| v.as_bool().unwrap()).collect(),
                    alpha_weighted: false,
                    edge: photocraft_algo::convolution::Edge::Repeat,
                };
                assert_eq!(params.channels.len(), mode.color_channels() + 1);
                assert!(!params.channels.last().unwrap());
                for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
                    let mut input = Surface::new(PixelFormat::new(mode, sample, true));
                    let mut pixel = vec![0.25; mode.color_channels() + 1];
                    *pixel.last_mut().unwrap() = 0.5;
                    let b = Rect::new(0, 0, 2, 1);
                    input.fill_rect(b, &pixel);
                    let original = input.pixel(0, 0);
                    let output = photocraft_algo::convolution::apply_with(&input, &params, b, b, None, &Interrupt::NONE).unwrap();
                    let actual = output.pixel(0, 0);
                    let expected = (f64::from(original[0]) + f64::from(offset)) as f32;
                    let expected = if sample == SampleType::F32 { expected } else { expected.clamp(0., 1.) };
                    assert_eq!(actual[0], expected);
                    assert_eq!(actual.last(), original.last());
                }
            }
        }
        let power = sinusoid(&StoredEffect::test("gegl:linear-sinusoid", vec![float("offset", 3.), float("exponent", 1.)]), &context()).unwrap();
        assert_eq!(power.filters[0].command, "filter.other.convolution");
        assert_eq!(power.filters[1].command, "image.adjustments.curves");
        let impossible = StoredEffect::test("gegl:linear-sinusoid", vec![float("offset", f32::MAX), float("x-amplitude", 126.)]);
        assert_eq!(sinusoid(&impossible, &context()).unwrap_err().reason, photocraft_doc::import_conversion::ConversionReason::ResourceUnavailable);
    }
    #[test]
    fn normal_vector_lut_has_unit_length_and_observed_full_z_encoding() {
        let half = result("gegl:normal-map", Vec::new());
        let full = result("gegl:normal-map", vec![Argument { name: "full-z", value: ArgumentValue::Bool(true) }]);
        let cube =
            |mapping: &NativeMapping| photocraft_cms::lutfile::parse("normal.cube", mapping.filters[0].params["data"].as_str().unwrap().as_bytes()).unwrap();
        let half = cube(&half);
        let full = cube(&full);
        let i = (8 * 17 * 17 + 8 * 17 + 16) * 3;
        let vx = 2. * half.data[i] - 1.;
        let vy = 2. * half.data[i + 1] - 1.;
        let vz = 2. * half.data[i + 2] - 1.;
        assert!((vx * vx + vy * vy + vz * vz - 1.).abs() < 1e-5);
        assert!((full.data[i + 2] - (2. * half.data[i + 2] - 1.)).abs() < 1e-6);
        assert_eq!(half.data[i], full.data[i]);
        assert_eq!(half.data[i + 1], full.data[i + 1]);
    }
    #[test]
    fn normal_axes_keep_independent_native_values_and_fractional_source_alpha() {
        use photocraft_color::{ColorMode, PixelFormat, SampleType};
        use photocraft_doc::Document;
        use photocraft_geom::Size;
        use photocraft_raster::{Interrupt, Surface};
        let b = Rect::new(0, 0, 3, 3);
        let c = MappingContext { native_filter_bounds: b, owner_bounds: b, ..context() };
        for alpha in [0.25, 0.5, 1.] {
            let mut source = Surface::new(PixelFormat::RGBA32F);
            for y in 0..3 {
                for x in 0..3 {
                    let value = (x + 2 * y) as f32 / 8.;
                    source.write_pixel(x, y, &[value, value, value, alpha]);
                }
            }
            let mapping = normal_map(&StoredEffect::test("gegl:normal-map", Vec::new()), &c).unwrap();
            let Some(NativeStructure::Composite { branches }) = mapping.structure else { panic!("native normal axes") };
            let mut doc = Document::new("Native independent derivative axes", Size::new(3, 3), ColorMode::Rgb, SampleType::F32);
            for branch in branches {
                let mut surface = source.clone();
                for filter in branch.filters {
                    if filter.command == "filter.other.custom" {
                        let kernel: Vec<f32> = filter.params["kernel"].as_array().unwrap().iter().map(|n| n.as_f64().unwrap() as f32).collect();
                        surface =
                            photocraft_algo::apply_in(&surface, &photocraft_algo::FilterParams::Custom { kernel, scale: 4., offset: 127.5 }, b, b, None, b);
                    } else if filter.command == "filter.other.setAlpha" {
                        surface =
                            photocraft_algo::coverage::apply_with(&surface, photocraft_algo::coverage::Operation::SetAlpha(1.), b, b, None, &Interrupt::NONE)
                                .unwrap();
                    } else {
                        let monochrome = filter.params["monochrome"].as_bool().unwrap_or(false);
                        let mut matrix = [[0.; 4]; 3];
                        for (row, key) in matrix.iter_mut().zip(if monochrome { ["gray", "gray", "gray"] } else { ["red", "green", "blue"] }) {
                            for (value, stored) in row.iter_mut().zip(filter.params[key].as_array().unwrap()) {
                                *value = stored.as_f64().unwrap() as f32 / 100.;
                            }
                        }
                        let adjustment = photocraft_doc::Adjustment::ChannelMixer { matrix, monochrome };
                        let mut buffer = photocraft_compose::Buffer { rect: b, px: surface.read_region(b).as_chunks::<4>().0.to_vec() };
                        photocraft_compose::adjust::apply_with(&adjustment, &mut buffer, photocraft_compose::adjust::Transfer::Gamma(1.));
                        surface.write_region(b, &buffer.px.into_iter().flatten().collect::<Vec<_>>());
                    }
                }
                let mut layer = Layer::new("Editable native derivative", LayerContent::Raster(surface));
                layer.blend = branch.blend;
                layer.clipped = branch.clipped;
                doc.layers.push(layer);
            }
            let raw = photocraft_compose::render(&doc, b).px;
            // Independent plane h=(x+2y)/8: centered differences are .25/.5;
            // native Custom divides by4 and adds.5, independently of alpha.
            for (actual, expected) in raw[4].into_iter().zip([0.5625, 0.625, 1., alpha]) {
                assert!((actual - expected).abs() < 1e-6, "alpha{alpha}: {raw:?}");
            }
            let lut = photocraft_cms::lutfile::parse("normal.cube", mapping.filters[0].params["data"].as_str().unwrap().as_bytes()).unwrap();
            let mut buffer = photocraft_compose::Buffer { rect: b, px: raw };
            photocraft_compose::adjust::apply(
                &photocraft_doc::Adjustment::ColorLookup {
                    name: "Native vector".into(),
                    lut: Some(std::sync::Arc::new(lut.data)),
                    size: 17,
                    tetrahedral: true,
                    dither: false,
                },
                &mut buffer,
            );
            let norm = (1.25f32.powi(2) + 2.5f32.powi(2) + 1.).sqrt();
            for (actual, expected) in buffer.px[4].into_iter().zip([0.5 + 0.5 * 1.25 / norm, 0.5 + 0.5 * 2.5 / norm, 0.5 + 0.5 / norm, alpha]) {
                assert!((actual - expected).abs() < 1e-5);
            }
            let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
            assert_eq!(photocraft_compose::render(&doc, b).px, photocraft_compose::render(&reopened, b).px);
        }
    }
    #[test]
    fn gray_normal_vectors_reuse_native_rgb_contents_and_keep_outer_gray_data() {
        use photocraft_color::{ColorMode, SampleType};
        use photocraft_doc::{Document, SmartSource};
        use photocraft_geom::Size;
        use photocraft_raster::Interrupt;
        let b = Rect::new(0, 0, 4, 3);
        let c = MappingContext { mode: ColorMode::Grayscale, owner_bounds: b, native_filter_bounds: b, ..context() };
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mapping = normal_map(&StoredEffect::test("gegl:normal-map", Vec::new()), &c).unwrap();
            assert!(mapping.working_rgb);
            let mut doc = Document::new("Gray normals", Size::new(4, 3), ColorMode::Grayscale, depth);
            doc.icc_profile = Some(photocraft_cms::Builtin::GrayGamma22.profile().to_bytes());
            let mut input = Layer::raster("Editable Gray height", doc.pixel_format());
            input.surface_mut().unwrap().fill_rect(b, &[0.25, 0.5]);
            let mut budget = 1 << 30;
            let mut owner = crate::xcf::native_effects::NativeOwner::new(&doc, &input, b, &mut budget, &Interrupt::NONE).unwrap();
            owner
                .push(
                    mapping,
                    crate::xcf::native_effects::NativeEnvelope { opacity: 1., blend: BlendMode::Normal, visible: true, mask: None },
                    &mut budget,
                    &Interrupt::NONE,
                )
                .unwrap();
            let content = owner.content();
            let LayerContent::Smart(smart) = &content else { panic!() };
            let SmartSource::Embedded { bytes, .. } = &smart.source else { panic!() };
            let rgb = photocraft_format::load_from_bytes(bytes).unwrap();
            assert_eq!(rgb.mode, ColorMode::Rgb);
            assert_eq!(rgb.depth, depth);
            assert_eq!(rgb.icc_profile, Some(photocraft_cms::Builtin::Srgb.profile().to_bytes()));
            doc.layers.push(Layer::new("Native Gray-projected normals", content));
            let saved = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
            assert_eq!(reopened, doc);
            assert_eq!(reopened.mode, ColorMode::Grayscale);
            assert_eq!(reopened.icc_profile, Some(photocraft_cms::Builtin::GrayGamma22.profile().to_bytes()));
        }
    }
    #[test]
    fn normal_gain_extremes_are_finite_native_lut_pixels_at_every_depth() {
        use photocraft_color::{ColorMode, PixelFormat, SampleType};
        use photocraft_raster::Surface;
        for scale in [0., 0.0001, 1000., f32::MAX] {
            let mapping = result("gegl:normal-map", vec![float("scale", scale)]);
            let lut = photocraft_cms::lutfile::parse("normal.cube", mapping.filters[0].params["data"].as_str().unwrap().as_bytes()).unwrap();
            assert!(lut.data.iter().all(|n| n.is_finite()));
            let adj = photocraft_doc::Adjustment::ColorLookup {
                name: "Native vector gain".into(),
                lut: Some(std::sync::Arc::new(lut.data)),
                size: 17,
                tetrahedral: true,
                dither: false,
            };
            let mut buffer = photocraft_compose::Buffer { rect: photocraft_geom::Rect::new(0, 0, 2, 1), px: vec![[1., 0.5, 1., 0.25], [0.5, 0.5, 1., 0.75]] };
            photocraft_compose::adjust::apply(&adj, &mut buffer);
            assert_eq!(buffer.px[0][3], 0.25);
            assert_eq!(buffer.px[1], [0.5, 0.5, 1., 0.75]);
            assert!(buffer.px.iter().flatten().all(|n| n.is_finite()));
            for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
                let mut surface = Surface::new(PixelFormat::new(ColorMode::Rgb, sample, true));
                let data: Vec<_> = buffer.px.iter().flatten().copied().collect();
                surface.write_region(buffer.rect, &data);
                assert!(surface.read_region(buffer.rect).iter().all(|n| n.is_finite()));
            }
        }
    }
    #[test]
    fn normal_component_collision_translates_observed_cyclic_fallback_without_rewriting_source() {
        for x in 0..3 {
            let collision = result("gegl:normal-map", vec![enm("x-component", x), enm("y-component", x)]);
            let explicit = result("gegl:normal-map", vec![enm("x-component", x), enm("y-component", (x + 1) % 3)]);
            let Some(NativeStructure::Composite { branches: a }) = collision.structure else { panic!() };
            let Some(NativeStructure::Composite { branches: b }) = explicit.structure else { panic!() };
            assert_eq!(a.len(), b.len());
            for (a, b) in a.iter().zip(&b) {
                assert_eq!(a.filters, b.filters);
            }
            assert!(collision.notes[0].contains("cyclic Y fallback"));
        }
    }
    #[test]
    fn actual_native_units_and_capability_gaps_are_explicit() {
        let tone = result("gegl:mantiuk06", vec![float("detail", 99.)]);
        assert_eq!(tone.filters[0].params["pencil"], json!(true));
        assert_eq!(tone.filters[0].params["points"].as_array().unwrap().len(), 256);
        assert!(tone.filters[2].params["amount"].as_f64().unwrap() <= 500.);
        let cropped = result("gegl:mirrors", vec![float("trim-x", 0.2)]);
        let Some(NativeStructure::Composite { branches }) = cropped.structure else { panic!("native mirror crop") };
        assert_eq!(branches[0].filter_frame, Some(Rect::new(12, 0, 64, 48)));
    }
    #[test]
    fn normal_components_and_post_sum_sinusoid_controls_stay_native() {
        let mapped =
            result("gegl:normal-map", vec![enm("x-component", 2), enm("y-component", 0), Argument { name: "flip-x", value: ArgumentValue::Bool(true) }]);
        let Some(NativeStructure::Composite { branches }) = mapped.structure else { panic!("normal branches") };
        assert_eq!(branches.len(), 3);
        assert_eq!(branches[0].filters[1].params["kernel"][11], json!(1.));
        assert_eq!(branches[0].filters[2].params["blue"], json!([100., 0., 0., 0.]));
        assert_eq!(branches[2].filters[0].params["green"], json!([0., 0., 0., 100.]));
        let post = result("gegl:linear-sinusoid", vec![float("offset", 0.5), float("exponent", 1.)]);
        assert!(post.structure.is_some());
        assert_eq!(post.filters.len(), 2);
        assert_eq!(post.filters[0].params["red"][3], json!(50.));
        assert_eq!(post.filters[1].params["points"][255], json!(255.));
        let screen = result("gegl:newsprint", Vec::new());
        let Some(NativeStructure::Composite { branches }) = screen.structure else { panic!("native screen branches") };
        let Effect::PatternOverlay { angle, scale, .. } = branches[1].layer_effects[0] else { panic!("native screen") };
        assert_eq!(angle, 75.);
        assert_eq!(scale, 12. / 64.);
    }
    #[test]
    fn hostile_geometry_and_typed_values_fail_without_allocating_or_panicking() {
        let mut c = context();
        c.native_filter_bounds = Rect::from_xywh(0, 0, 1_000_000, 1_000_000);
        assert!(grid(&StoredEffect::test("gegl:grid", vec![int("x", 2), int("line-width", 1)]), &c).is_err());
        assert!(maze(&StoredEffect::test("gegl:maze", vec![int("x", 1), int("y", 1)]), &c).is_err());
        for args in [vec![float("x-period", f32::NAN)], vec![float("x-amplitude", f32::MAX)], vec![float("x-period", 0.000001)]] {
            assert!(sinusoid(&StoredEffect::test("gegl:linear-sinusoid", args), &context()).is_err());
        }
        assert_eq!(
            flare(&StoredEffect::test("gegl:lens-flare", vec![float("pos-x", f32::NAN)]), &context()).unwrap_err().reason,
            photocraft_doc::import_conversion::ConversionReason::InvalidOptionalData
        );
    }
    #[test]
    fn editable_geometry_obeys_actual_budget_and_cancellation() {
        let mut metadata = context();
        metadata.remaining_metadata_bytes = 1;
        assert!(map(&StoredEffect::test("gegl:grid", Vec::new()), &metadata).unwrap().is_err());
        assert!(map(&StoredEffect::test("gegl:long-shadow", Vec::new()), &metadata).unwrap().is_err());
        let tiny = MappingContext { remaining_generation_bytes: 128, ..context() };
        for operation in ["gegl:grid", "gegl:maze", "gegl:linear-sinusoid", "gegl:mirrors", "gegl:long-shadow"] {
            assert!(map(&StoredEffect::test(operation, Vec::new()), &tiny).unwrap().is_err(), "{operation}");
        }
        let cancel = || true;
        let interrupted = MappingContext { interrupt: photocraft_raster::Interrupt::cancel_only(&cancel), ..context() };
        for operation in ["gegl:grid", "gegl:maze", "gegl:linear-sinusoid"] {
            assert!(map(&StoredEffect::test(operation, Vec::new()), &interrupted).unwrap().is_err(), "{operation}");
        }
    }
    #[test]
    fn lens_distortion_native_radial_map_background_vignette_and_native_reopen_are_editable() {
        use photocraft_color::{PixelFormat, SampleType};
        use photocraft_raster::{Interrupt, Surface};
        let b = Rect::new(0, 0, 64, 32);
        let c = MappingContext {
            linear: true,
            owner_bounds: b,
            native_filter_bounds: b,
            target_profile: Some(photocraft_cms::Builtin::LinearSrgb.profile().to_bytes()),
            resolution_dpi: 72.0,
            ..context()
        };
        let effect = StoredEffect::test("gegl:lens-distortion", vec![float("main", 25.), float("edge", 10.), float("brighten", 50.), paint("background")]);
        let converted = map(&effect, &c).unwrap().unwrap();
        let Some(NativeStructure::Composite { branches }) = converted.structure else { panic!("native optical branches") };
        let filter = &branches[1].filters[0];
        assert_eq!(filter.command, "filter.distort.displace");
        let map = crate::smart_map::displace_embedded_map(&filter.params).unwrap();
        let mut source = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::F32, true));
        let pixels: Vec<_> = (0..32).flat_map(|y| (0..64).flat_map(move |x| [x as f32 / 63., y as f32 / 31., 0.25, 0.5])).collect();
        source.write_region(b, &pixels);
        let options = photocraft_algo::FilterParams::Displace {
            horizontal: filter.params["horizontal"].as_f64().unwrap() as f32,
            vertical: filter.params["vertical"].as_f64().unwrap() as f32,
            stretch: true,
            undefined: photocraft_algo::UndefinedAreas::Wrap,
            map: Some(map),
        };
        let geometry = photocraft_algo::apply_in_with(&source, &options, b, b, None, b, &Interrupt::NONE).unwrap();
        // Independent standard native radial counterpart: positive k1/k2 makes
        // this right-hand sample read farther out, and native coverage stays .5.
        assert!(geometry.pixel(48, 16)[0] > source.pixel(48, 16)[0]);
        assert_eq!(geometry.pixel(48, 16)[3], 0.5);
        let vignette = &branches[1].filters[1];
        assert_eq!(vignette.command, "filter.lensCorrection");
        assert_eq!(vignette.params["vignetteAmount"], -8.75);
        let filtered = photocraft_algo::lens::correct(
            &geometry,
            b,
            &photocraft_algo::lens::LensCorrection {
                vignette_amount: -8.75,
                vignette_midpoint: 100.,
                edge: photocraft_algo::lens::EdgeMode::Extension,
                ..Default::default()
            },
        );
        assert!(filtered.pixel(48, 16)[2] < geometry.pixel(48, 16)[2]);
        assert_eq!(filtered.pixel(48, 16)[3], 0.5);
        assert_eq!(branches[1].mask.as_ref().unwrap().value(0, 16), 0.);
        assert_eq!(branches[1].mask.as_ref().unwrap().value(32, 16), 1.);
        let mut doc = photocraft_doc::Document::new("Native optical data", photocraft_geom::Size::new(64, 32), ColorMode::Rgb, SampleType::F32);
        doc.icc_profile = c.target_profile.clone();
        for branch in &branches {
            let content = branch.fill.clone().map(LayerContent::Fill).unwrap_or_else(|| LayerContent::Raster(filtered.clone()));
            let mut layer = Layer::new("Native optics", content);
            layer.mask = branch.mask.clone();
            layer.vector_mask = branch.vector_mask.clone();
            doc.layers.push(layer);
        }
        let before = photocraft_compose::render(&doc, b);
        assert!((before.px[16 * 64][3] - 128. / 255.).abs() < 1e-5);
        assert!((before.px[16 * 64][0] - photocraft_color::convert::srgb_to_linear(128. / 255.)).abs() < 1e-5);
        let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
        assert_eq!(photocraft_compose::render(&reopened, b).px, before.px);
        // Persist the editable ordinary filters as native SmartObject data as well.
        let mut smart = photocraft_doc::SmartObject::new(
            photocraft_doc::SmartSource::Embedded { file_name: "contents.pcraft".into(), bytes: std::sync::Arc::new(bytes) },
            Affine::IDENTITY,
            None,
        );
        smart.smart_filters = branches[1].filters.clone();
        doc.layers.clear();
        doc.layers.push(Layer::new("Native optical filters", LayerContent::Smart(smart)));
        let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
        let LayerContent::Smart(smart) = &reopened.layers[0].content else { panic!("native smart") };
        assert_eq!(smart.smart_filters[0].params, branches[1].filters[0].params);
        assert_eq!(smart.smart_filters[1].params, branches[1].filters[1].params);
    }
    #[test]
    fn lens_distortion_all_scalar_controls_negative_folds_and_work_guards_use_native_data() {
        let neutral = result("gegl:lens-distortion", vec![float("brighten", 50.), float("x-shift", 25.)]);
        assert!(neutral.filters.is_empty() && neutral.structure.is_none());
        let base = result("gegl:lens-distortion", vec![float("main", 25.)]);
        let Some(NativeStructure::Composite { branches: base }) = base.structure else { panic!("native radial") };
        for args in [
            vec![float("main", -100.), float("edge", -100.)],
            vec![float("main", 25.), float("zoom", 25.)],
            vec![float("main", 25.), float("x-shift", 25.)],
            vec![float("main", 25.), float("y-shift", 25.)],
            vec![float("edge", 25.)],
        ] {
            let converted = result("gegl:lens-distortion", args);
            let Some(NativeStructure::Composite { branches }) = converted.structure else { panic!("native radial") };
            assert_ne!(branches[1].filters[0].params, base[1].filters[0].params);
            assert!(branches[1].mask.is_some());
        }
        let e = StoredEffect::test("gegl:lens-distortion", vec![float("edge", 25.)]);
        for c in [
            MappingContext { remaining_generation_bytes: 1, ..context() },
            MappingContext { remaining_metadata_bytes: 1, ..context() },
            MappingContext { native_filter_bounds: Rect::new(0, 0, 30000, 30000), ..context() },
        ] {
            assert!(map(&e, &c).unwrap().is_err());
        }
        let cancel = || true;
        let c = MappingContext { interrupt: photocraft_raster::Interrupt::cancel_only(&cancel), ..context() };
        assert!(map(&e, &c).unwrap().is_err());
        assert!(map(&StoredEffect::test("gegl:lens-distortion", vec![float("main", f32::NAN)]), &context()).unwrap().is_err());
    }
    #[test]
    fn solid_noise_uses_all_geometry_detail_seed_and_turbulence_controls_as_native_resources() {
        let convert = |extra: Vec<Argument<'static>>| {
            let mut args = vec![int("width", 17), int("height", 13)];
            args.extend(extra);
            result("gegl:noise-solid", args)
        };
        let original = convert(Vec::new());
        let Some(NativeStructure::Replace { patterns, .. }) = original.structure else { panic!("native solid noise") };
        let original = patterns[0].id.clone();
        for fields in [
            vec![float("x-size", 2.)],
            vec![float("y-size", 8.)],
            vec![int("detail", 3)],
            vec![Argument { name: "seed", value: ArgumentValue::UInt(1) }],
            vec![Argument { name: "turbulent", value: ArgumentValue::Bool(true) }],
        ] {
            let converted = convert(fields);
            let Some(NativeStructure::Replace { patterns, .. }) = converted.structure else { panic!("native solid noise") };
            assert_ne!(patterns[0].id, original);
        }
        let c = MappingContext { linear: true, target_profile: Some(photocraft_cms::Builtin::LinearSrgb.profile().to_bytes()), ..context() };
        let source = StoredEffect::test(
            "gegl:noise-solid",
            vec![
                int("width", 17),
                int("height", 13),
                float("x-size", 3.5),
                float("y-size", 2.25),
                int("detail", 3),
                Argument { name: "tileable", value: ArgumentValue::Bool(true) },
            ],
        );
        let converted = map(&source, &c).unwrap().unwrap();
        let Some(NativeStructure::Replace { layer, patterns }) = converted.structure else { panic!("native periodic solid noise") };
        assert_eq!((patterns[0].width, patterns[0].height), (17, 13));
        for y in 0..13 {
            assert_eq!(patterns[0].surface.pixel(0, y), patterns[0].surface.pixel(16, y));
        }
        for x in 0..17 {
            assert_eq!(patterns[0].surface.pixel(x, 0), patterns[0].surface.pixel(x, 12));
        }
        let pixel = patterns[0].surface.pixel(3, 4);
        assert_eq!(pixel[0], pixel[1]);
        assert_eq!(pixel[1], pixel[2]);
        assert_eq!(pixel[3], 1.);
        let mut doc =
            photocraft_doc::Document::new("Native solid pattern", photocraft_geom::Size::new(20, 15), ColorMode::Rgb, photocraft_color::SampleType::F32);
        doc.icc_profile = c.target_profile;
        doc.patterns = patterns;
        doc.layers.push(*layer);
        let before = photocraft_compose::render(&doc, doc.bounds());
        assert_eq!(before.px[19][3], 0.);
        assert_eq!(before.px[0][3], 1.);
        let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
        assert_eq!(photocraft_compose::render(&reopened, reopened.bounds()).px, before.px);
    }
    #[test]
    fn solid_noise_empty_extents_and_hostile_work_budget_or_cancel_never_allocate_large_patterns() {
        let empty = result("gegl:noise-solid", vec![int("width", 0)]);
        let Some(NativeStructure::Replace { patterns, .. }) = empty.structure else { panic!("native transparent fill") };
        assert!(patterns.is_empty());
        let huge = StoredEffect::test("gegl:noise-solid", vec![int("width", i32::MAX), int("height", i32::MAX), int("detail", 15)]);
        assert!(map(&huge, &context()).unwrap().is_err());
        let small = StoredEffect::test("gegl:noise-solid", vec![int("width", 17), int("height", 13)]);
        let tiny = MappingContext { remaining_generation_bytes: 1, ..context() };
        assert!(map(&small, &tiny).unwrap().is_err());
        let cancel = || true;
        let cancelled = MappingContext { interrupt: photocraft_raster::Interrupt::cancel_only(&cancel), ..context() };
        assert!(map(&small, &cancelled).unwrap().is_err());
        assert!(map(&StoredEffect::test("gegl:noise-solid", vec![float("x-size", f32::NAN)]), &context()).unwrap().is_err());
    }
    #[test]
    fn gradient_direction_native_lut_cardinal_units_depth_alpha_and_persistence() {
        use photocraft_color::{PixelFormat, SampleType};
        use photocraft_doc::{Adjustment, Document, SmartObject, SmartSource};
        use photocraft_geom::Size;
        use photocraft_raster::{Interrupt, Surface};
        let mapped = result("gegl:image-gradient", vec![enm("output-mode", 1)]);
        let cube = photocraft_cms::lutfile::parse("gradient.cube", mapped.filters[0].params["data"].as_str().unwrap().as_bytes()).unwrap();
        let adjustment =
            Adjustment::ColorLookup { name: "Native direction".into(), lut: Some(std::sync::Arc::new(cube.data)), size: 17, tetrahedral: true, dither: false };
        let mut interpolated = photocraft_compose::Buffer { rect: Rect::new(0, 0, 1, 1), px: vec![[0.73, 0.61, 0., 0.375]] };
        photocraft_compose::adjust::apply(&adjustment, &mut interpolated);
        let angle = interpolated.px[0][0] * std::f32::consts::TAU - std::f32::consts::PI;
        let analytic = (-0.11_f32).atan2(-0.23_f32);
        assert!((angle - analytic).abs() < 0.03, "native finite LUT approximation: {angle} vs {analytic}");
        assert_eq!(interpolated.px[0][3], 0.375);
        let b = Rect::new(0, 0, 5, 1);
        let expected = [std::f32::consts::PI, 0., -std::f32::consts::FRAC_PI_2, std::f32::consts::FRAC_PI_2, 0.];
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut buffer = photocraft_compose::Buffer {
                rect: b,
                px: vec![[0.75, 0.5, 0., 0.25], [0.25, 0.5, 0., 0.5], [0.5, 0.75, 0., 0.75], [0.5, 0.25, 0., 0.5], [0.5, 0.5, 0., 0.25]],
            };
            photocraft_compose::adjust::apply(&adjustment, &mut buffer);
            let mut source = Surface::new(PixelFormat::new(ColorMode::Rgb, depth, true));
            source.write_region(b, &buffer.px.iter().flatten().copied().collect::<Vec<_>>());
            let filter = &mapped.filters[1].params;
            let options = photocraft_algo::convolution::Params {
                kernel: filter["kernel"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect(),
                scale: 1.,
                offset: -std::f64::consts::PI,
                channels: vec![true, true, true, false],
                alpha_weighted: false,
                edge: photocraft_algo::convolution::Edge::Repeat,
            };
            let native = photocraft_algo::convolution::apply_with(&source, &options, b, b, None, &Interrupt::NONE).unwrap();
            let tolerance = match depth {
                SampleType::U8 => 0.03,
                SampleType::U16 => 0.001,
                SampleType::F32 => 0.00001,
            };
            for (index, angle) in expected.iter().enumerate() {
                let sample = native.pixel(index as i32, 0);
                let angle = if depth == SampleType::F32 { *angle } else { angle.clamp(0., 1.) };
                assert!((sample[0] - angle).abs() < tolerance, "{depth:?}: {:?} != {angle}", sample);
                assert_eq!(sample[0], sample[1]);
                assert_eq!(sample[1], sample[2]);
                assert_eq!(sample[3], source.pixel(index as i32, 0)[3]);
            }
            let mut doc = Document::new("Native angle data", Size::new(5, 1), ColorMode::Rgb, depth);
            doc.icc_profile = Some(photocraft_cms::Builtin::LinearSrgb.profile().to_bytes());
            doc.layers.push(Layer::new("Native pixels", LayerContent::Raster(native)));
            let before = photocraft_compose::render(&doc, b).px;
            let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
            assert_eq!(photocraft_compose::render(&reopened, b).px, before);
            let mut smart =
                SmartObject::new(SmartSource::Embedded { file_name: "gradient.pcraft".into(), bytes: std::sync::Arc::new(bytes) }, Affine::IDENTITY, None);
            smart.smart_filters = mapped.filters.clone();
            doc.layers.clear();
            doc.layers.push(Layer::new("Editable native angle", LayerContent::Smart(smart)));
            let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
            let LayerContent::Smart(smart) = &reopened.layers[0].content else { panic!("native smart") };
            assert_eq!(smart.smart_filters, mapped.filters);
        }
        let both = result("gegl:image-gradient", vec![enm("output-mode", 2)]);
        assert_eq!(both.filters, result("gegl:image-gradient", Vec::new()).filters, "observed drawable Both is magnitude, not invented RGB vector data");
    }
    #[test]
    fn gradient_direction_native_derivative_basis_clipped_alpha_and_bounded_domains() {
        use photocraft_color::{PixelFormat, SampleType};
        use photocraft_doc::Document;
        use photocraft_geom::Size;
        use photocraft_raster::{Interrupt, Surface};
        let mapped = result("gegl:image-gradient", vec![enm("output-mode", 1)]);
        let Some(NativeStructure::Composite { branches }) = mapped.structure else { panic!("native derivative branches") };
        assert_eq!(branches.len(), 2);
        assert!(branches[1].clipped);
        let b = Rect::new(0, 0, 5, 5);
        let mut source = Surface::new(PixelFormat::RGBA32F);
        let pixels: Vec<_> = (0..5).flat_map(|_| (0..5).flat_map(|x| [x as f32 / 4., x as f32 / 4., x as f32 / 4., 0.5])).collect();
        source.write_region(b, &pixels);
        let mut monochrome = photocraft_compose::Buffer { rect: b, px: pixels.as_chunks::<4>().0.to_vec() };
        photocraft_compose::adjust::apply_depth(
            &photocraft_doc::Adjustment::ChannelMixer { matrix: [[0.2126, 0.7152, 0.0722, 0.]; 3], monochrome: true },
            &mut monochrome,
            photocraft_compose::adjust::Transfer::Gamma(1.),
            Some(SampleType::F32),
        );
        source.write_region(b, &monochrome.px.iter().flatten().copied().collect::<Vec<_>>());
        let mut doc = Document::new("Native derivative axes", Size::new(5, 5), ColorMode::Rgb, SampleType::F32);
        doc.icc_profile = Some(photocraft_cms::Builtin::LinearSrgb.profile().to_bytes());
        for (axis, branch) in branches.iter().enumerate() {
            let custom = &branch.filters[1].params;
            let kernel = custom["kernel"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect();
            let mut surface = photocraft_algo::apply_in_with(
                &source,
                &photocraft_algo::FilterParams::Custom { kernel, scale: 2., offset: 127.5 },
                b,
                b,
                None,
                b,
                &Interrupt::NONE,
            )
            .unwrap();
            assert!((surface.pixel(2, 2)[0] - if axis == 0 { 0.75 } else { 0.5 }).abs() < 0.002);
            let routed: Vec<_> =
                surface.read_region(b).as_chunks::<4>().0.iter().flat_map(|p| if axis == 0 { [p[0], 0., 0., p[3]] } else { [0., p[0], 0., p[3]] }).collect();
            surface.write_region(b, &routed);
            if axis != 0 {
                assert_eq!(branch.filters[3].command, "filter.other.setAlpha");
                surface =
                    photocraft_algo::coverage::apply_with(&surface, photocraft_algo::coverage::Operation::SetAlpha(1.), b, b, None, &Interrupt::NONE).unwrap();
            }
            let mut layer = Layer::new("Editable native axis", LayerContent::Raster(surface));
            layer.blend = branch.blend;
            layer.clipped = branch.clipped;
            doc.layers.push(layer);
        }
        let rendered = photocraft_compose::render(&doc, b);
        let center = rendered.px[12];
        assert!((center[0] - 0.75).abs() < 0.002);
        assert!((center[1] - 0.5).abs() < 1e-6, "{center:?}");
        assert_eq!(center[3], 0.5);
        for c in [MappingContext { remaining_generation_bytes: 1, ..context() }, MappingContext { remaining_metadata_bytes: 1, ..context() }] {
            assert!(map(&StoredEffect::test("gegl:image-gradient", vec![enm("output-mode", 1)]), &c).unwrap().is_err());
        }
        let cancel = || true;
        let cancelled = MappingContext { interrupt: Interrupt::cancel_only(&cancel), ..context() };
        assert!(map(&StoredEffect::test("gegl:image-gradient", vec![enm("output-mode", 1)]), &cancelled).unwrap().is_err());
        let gray = MappingContext { mode: ColorMode::Grayscale, ..context() };
        let gray_native = map(&StoredEffect::test("gegl:image-gradient", vec![enm("output-mode", 1)]), &gray).unwrap().unwrap();
        assert!(gray_native.working_rgb);
        assert_eq!(gray_native.filters, mapped.filters);
        assert!(matches!(gray_native.structure, Some(NativeStructure::Composite { .. })));
        assert!(map(&StoredEffect::test("gegl:image-gradient", vec![enm("output-mode", 99)]), &context()).unwrap().is_err());
    }
    fn screen_native_document(mapping: NativeMapping, depth: photocraft_color::SampleType) -> photocraft_doc::Document {
        use photocraft_color::PixelFormat;
        use photocraft_doc::adjust::Adjustment;
        use photocraft_raster::{Interrupt, Surface};
        let b = Rect::new(0, 0, 8, 8);
        let mut source = Surface::new(PixelFormat::new(ColorMode::Rgb, depth, true));
        for y in 0..8 {
            for x in 0..8 {
                source.write_pixel(x, y, &[0.5, 0.5, 0.5, [0., 0.25, 0.5, 1.][x as usize % 4]]);
            }
        }
        let mut doc = photocraft_doc::Document::new("Native screen pipeline", photocraft_geom::Size::new(8, 8), ColorMode::Rgb, depth);
        doc.icc_profile = Some(photocraft_cms::Builtin::Srgb.profile().to_bytes());
        doc.patterns = mapping.patterns;
        let Some(NativeStructure::Composite { branches }) = mapping.structure else { panic!("native screen structure") };
        for branch in branches {
            let mut pixels = source.clone();
            for filter in branch.filters {
                if filter.command == "filter.other.setAlpha" {
                    pixels = photocraft_algo::coverage::apply_with(&pixels, photocraft_algo::coverage::Operation::SetAlpha(1.), b, b, None, &Interrupt::NONE)
                        .unwrap();
                    continue;
                }
                let adj = if filter.command == "image.adjustments.channelMixer" {
                    let mono = filter.params["monochrome"].as_bool().unwrap_or(false);
                    let matrix = std::array::from_fn(|row| {
                        std::array::from_fn(|column| {
                            filter.params[if mono { "gray" } else { ["red", "green", "blue"][row] }][column].as_f64().unwrap_or(0.) as f32 / 100.
                        })
                    });
                    Adjustment::ChannelMixer { matrix, monochrome: mono }
                } else if filter.command == "image.adjustments.colorLookup" {
                    let table = photocraft_cms::lutfile::parse("native.cube", filter.params["data"].as_str().unwrap().as_bytes()).unwrap();
                    Adjustment::ColorLookup {
                        name: "Native ink".into(),
                        size: table.size as u32,
                        lut: Some(std::sync::Arc::new(table.data)),
                        tetrahedral: true,
                        dither: false,
                    }
                } else if filter.command == "image.adjustments.curves" {
                    crate::xcf::coverage::adjustment(&filter.command, &filter.params).unwrap()
                } else {
                    assert_eq!(filter.command, "image.adjustments.invert");
                    Adjustment::Invert
                };
                let mut buffer =
                    photocraft_compose::Buffer { rect: b, px: (0..8).flat_map(|y| (0..8).map(move |x| (x, y))).map(|(x, y)| pixels.rgba(x, y)).collect() };
                photocraft_compose::adjust::apply_in(
                    &adj,
                    &mut buffer,
                    photocraft_compose::adjust::Transfer::for_document(ColorMode::Rgb, depth),
                    Some(depth),
                    ColorMode::Rgb,
                );
                pixels = Surface::new(source.format());
                pixels.write_region(b, &buffer.px.into_iter().flatten().collect::<Vec<_>>());
            }
            let mut layer = Layer::new("Ordinary native screened pixels", LayerContent::Raster(pixels));
            layer.blend = branch.blend;
            layer.clipped = branch.clipped;
            layer.effects.items = branch.layer_effects;
            doc.layers.push(layer);
        }
        doc
    }
    #[test]
    fn mirrors_crop_is_inside_each_native_instance_before_fold_and_survives_native_storage() {
        use crate::xcf::native_effects::{NativeEnvelope, NativeOwner};
        use photocraft_color::{PixelFormat, SampleType};
        use photocraft_doc::{SmartObject, SmartSource};
        use photocraft_raster::{Interrupt, Surface};
        let converted = result("gegl:mirrors", vec![float("trim-x", 0.25), float("trim-y", 0.5), int("n-segs", 4)]);
        let Some(NativeStructure::Composite { branches }) = &converted.structure else { panic!("native mirror branches") };
        let frame = Rect::new(16, 24, 64, 48);
        for branch in branches {
            assert_eq!(branch.filter_frame, Some(frame));
            assert!(branch.transform.inverse().is_some());
        }
        let transforms: Vec<_> = branches.iter().map(|branch| branch.transform.mul(&Affine::translate(16., 24.))).collect();
        let mut doc = photocraft_doc::Document::new("Native mirror source", photocraft_geom::Size::new(64, 48), ColorMode::Rgb, SampleType::F32);
        let mut pixels = Surface::new(PixelFormat::RGBA32F);
        pixels.write_pixel(20, 30, &[0.25, 0.5, 0.75, 0.5]);
        let layer = Layer::new("Original editable pixels", LayerContent::Raster(pixels));
        doc.layers.push(layer.clone());
        let mut budget = crate::xcf::MAX_PIXEL_BYTES;
        let mut owner = NativeOwner::new(&doc, &layer, doc.bounds(), &mut budget, &Interrupt::NONE).unwrap();
        owner.push(converted, NativeEnvelope { opacity: 1., blend: BlendMode::Normal, visible: true, mask: None }, &mut budget, &Interrupt::NONE).unwrap();
        let LayerContent::Smart(SmartObject { source: SmartSource::Embedded { bytes, .. }, .. }) = owner.content() else { panic!("computed native mirror") };
        let computed = photocraft_format::load_from_bytes(&bytes).unwrap();
        for (layer, transform) in computed.layers.iter().skip(1).zip(transforms) {
            let LayerContent::Smart(smart) = &layer.content else { panic!("cropped native frame") };
            assert_eq!(smart.transform, transform);
            let SmartSource::Embedded { bytes, .. } = &smart.source else { panic!("embedded crop") };
            let cropped = photocraft_format::load_from_bytes(bytes).unwrap();
            assert_eq!(cropped.size, photocraft_geom::Size::new(48, 24));
            let LayerContent::Smart(input) = &cropped.layers[0].content else { panic!("editable crop input") };
            assert_eq!(input.transform, Affine::translate(-16., -24.));
        }
        let empty = result("gegl:mirrors", vec![float("output-scale", 0.)]);
        let Some(NativeStructure::Replace { layer, .. }) = empty.structure else { panic!("native empty result") };
        assert!(photocraft_compose::render_layer(&layer, Rect::new(0, 0, 4, 4)).px.iter().all(|p| p[3] == 0.));
        assert!(converted_native_mirror_policy().contains("fixed native owner canvas"));
    }
    fn converted_native_mirror_policy() -> String {
        result("gegl:mirrors", vec![Argument { name: "clip", value: ArgumentValue::Bool(false) }, Argument { name: "warp", value: ArgumentValue::Bool(false) }])
            .notes
            .join(" ")
    }

    #[test]
    fn newsprint_native_hardmix_counterpart_preserves_coverage_at_all_depths_and_reopens() {
        use photocraft_color::{PixelFormat, SampleType};
        use photocraft_raster::Surface;
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let converted = result("gegl:newsprint", vec![float("period", 8.), float("angle", 0.)]);
            let actual = screen_native_document(converted, depth);
            // Independently author the ordinary native counterpart: black base
            // preserving original alpha plus opaque .5 image clipped through a
            // native Hard Mix Pattern Overlay with a linear threshold ramp.
            let mut expected = photocraft_doc::Document::new("Manual native line screen", photocraft_geom::Size::new(8, 8), ColorMode::Rgb, depth);
            expected.icc_profile = actual.icc_profile.clone();
            let mut base = Surface::new(PixelFormat::new(ColorMode::Rgb, depth, true));
            let mut opaque = Surface::new(base.format());
            for y in 0..8 {
                for x in 0..8 {
                    base.write_pixel(x, y, &[0., 0., 0., [0., 0.25, 0.5, 1.][x as usize % 4]]);
                    opaque.write_pixel(x, y, &[0.5, 0.5, 0.5, 1.]);
                }
            }
            expected.layers.push(Layer::new("Manual original alpha", LayerContent::Raster(base.clone())));
            let mut ramp = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
            for y in 0..64 {
                ramp.fill_rect(Rect::new(0, y, 64, y + 1), &[(y as f32 + 0.5) / 64.]);
            }
            let pattern = photocraft_doc::Pattern::new("Independent native threshold ramp", ramp, 64, 64);
            let mut screen = Layer::new("Manual opaque native screen", LayerContent::Raster(opaque));
            screen.clipped = true;
            screen.effects.items.push(Effect::PatternOverlay {
                common: FxCommon::new(BlendMode::HardMix, 1.),
                name: pattern.name.clone(),
                id: pattern.id.clone(),
                scale: 0.125,
                angle: 0.,
                link: false,
                phase: (0., 0.),
            });
            expected.patterns.push(pattern);
            expected.layers.push(screen);
            let rendered = photocraft_compose::render(&actual, actual.bounds());
            let reference = photocraft_compose::render(&expected, expected.bounds());
            for (index, (pixel, wanted)) in rendered.px.iter().zip(&reference.px).enumerate() {
                assert_eq!(
                    pixel,
                    wanted,
                    "{depth:?}/{index}: source={:?}, tile={:?}, style={:?}",
                    actual.layers[1].surface().unwrap().rgba(1, 5),
                    actual.patterns[0].surface.pixel(0, 40),
                    actual.layers[1].effects.items
                );
            }
            for y in 0..8 {
                for x in 0..8 {
                    let p = rendered.get(x, y);
                    let alpha = base.rgba(x, y)[3];
                    assert_eq!(p[3], alpha, "{depth:?}/{x}/{y}");
                    let value = if y >= 4 && alpha > 0. { 1. } else { 0. };
                    assert_eq!(p[..3], [value; 3], "{depth:?}/{x}/{y}");
                }
            }
            let bytes = photocraft_format::save_to_bytes(&actual, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
            assert_eq!(photocraft_compose::render(&reopened, reopened.bounds()).px, rendered.px);
            assert!(matches!(reopened.layers[1].effects.items[0], Effect::PatternOverlay { .. }));
            assert_eq!(reopened.patterns.len(), 1);
        }
    }
    #[test]
    fn newsprint_independent_plates_patterns_cmyk_profile_and_actual_budgets_are_native() {
        use photocraft_color::SampleType;
        let rgb = result(
            "gegl:newsprint",
            vec![
                enm("color-model", 2),
                float("period2", 4.),
                float("period3", 8.),
                float("period4", 16.),
                enm("pattern2", 2),
                enm("pattern3", 3),
                enm("pattern4", 4),
                float("angle2", 0.),
                float("angle3", 30.),
                float("angle4", 60.),
            ],
        );
        let Some(NativeStructure::Composite { branches }) = &rgb.structure else { panic!("native RGB plates") };
        assert_eq!(branches.len(), 4);
        assert_eq!(rgb.patterns.len(), 3);
        for ((branch, period), angle) in branches.iter().skip(1).zip([4., 8., 16.]).zip([0., 30., 60.]) {
            assert!(branch.clipped);
            assert_eq!(branch.blend, BlendMode::LinearDodge);
            let Effect::PatternOverlay { scale, angle: a, ref common, .. } = branch.layer_effects[0] else { panic!("native plate screen") };
            assert_eq!(scale, period / 64.);
            assert_eq!(a, angle);
            assert_eq!(common.blend, BlendMode::HardMix);
        }
        let output = screen_native_document(rgb, SampleType::F32);
        assert!(photocraft_compose::render(&output, output.bounds()).px.iter().all(|p| p[3] <= 1.));
        let make = |pullout| result("gegl:newsprint", vec![enm("color-model", 3), float("black-pullout", pullout)]);
        let cmyk = make(1.);
        let none = make(0.);
        let (Some(NativeStructure::Composite { branches: full }), Some(NativeStructure::Composite { branches: zero })) = (&cmyk.structure, &none.structure)
        else {
            panic!("native CMYK plates")
        };
        assert_eq!(full.len(), 5);
        assert!(full.iter().skip(1).all(|p| p.clipped && p.blend == BlendMode::Multiply));
        assert_ne!(full[4].filters[0].params["data"], zero[4].filters[0].params["data"]);
        let rendered = photocraft_compose::render(&screen_native_document(cmyk, SampleType::F32), Rect::new(0, 0, 8, 8));
        for x in 0..8 {
            assert_eq!(rendered.get(x, 0)[3], [0., 0.25, 0.5, 1.][x as usize % 4]);
        }
        let small = MappingContext { remaining_generation_bytes: 1, ..context() };
        assert!(map(&StoredEffect::test("gegl:newsprint", Vec::new()), &small).unwrap().is_err());
        let small = MappingContext { remaining_metadata_bytes: 1, ..context() };
        assert!(map(&StoredEffect::test("gegl:newsprint", Vec::new()), &small).unwrap().is_err());
        let cancel = || true;
        let c = MappingContext { interrupt: photocraft_raster::Interrupt::cancel_only(&cancel), ..context() };
        assert!(map(&StoredEffect::test("gegl:newsprint", Vec::new()), &c).unwrap().is_err());
        let zero = result("gegl:newsprint", vec![float("period", 0.), enm("pattern", 3)]);
        assert!(zero.patterns.is_empty());
        let gray = MappingContext { mode: ColorMode::Grayscale, ..context() };
        let gray = map(&StoredEffect::test("gegl:newsprint", vec![enm("color-model", 3)]), &gray).unwrap().unwrap();
        assert!(gray.working_rgb);
        assert!(gray.notes[0].contains("ordinary RGB/sRGB Smart working contents"));
    }

    #[test]
    fn oil_histogram_resolution_is_superseded_native_policy_and_controls_have_native_purpose() {
        let base = result("gegl:oilify", Vec::new());
        for bins in [8, 128, 256] {
            let mapping = result("gegl:oilify", vec![int("intensities", bins)]);
            assert_eq!(mapping.filters, base.filters);
            assert!(mapping.notes[0].contains("superseded estimator-resolution provenance"));
            assert!(mapping.notes[0].contains(&format!("histogram size{bins}")));
            let wide = result("gegl:oilify", vec![int("intensities", bins), int("mask-radius", 100)]);
            assert_eq!(wide.filters[0].command, "filter.blur.surfaceBlur");
            assert_eq!(wide.filters[0].params["radius"], 100);
            assert_eq!(wide.filters[0].params["threshold"], 15.);
            assert_eq!(wide.filters[1].params["stylization"], 10.);
        }
        let smooth = result("gegl:oilify", vec![int("exponent", 20)]);
        assert_eq!(smooth.filters[0].params["cleanliness"], 10.);
        assert_eq!(result("gegl:oilify", vec![int("exponent", 1)]).filters[0].params["cleanliness"], 0.);
        let independent = result("gegl:oilify", vec![Argument { name: "use-inten", value: ArgumentValue::Bool(false) }]);
        let Some(NativeStructure::Composite { branches }) = independent.structure else { panic!("native component paint") };
        assert_eq!(branches.len(), 3);
        for (axis, branch) in branches.iter().enumerate() {
            assert_eq!(branch.filters[0].params["gray"][axis], 100.);
            assert_eq!(branch.filters[1], base.filters[0]);
            assert_eq!(branch.clipped, axis != 0);
            if axis != 0 {
                assert_eq!(branch.filters[3].command, "filter.other.setAlpha");
            }
        }
        let gray = MappingContext { mode: photocraft_color::ColorMode::Grayscale, ..context() };
        let gray_independent =
            map(&StoredEffect::test("gegl:oilify", vec![Argument { name: "use-inten", value: ArgumentValue::Bool(false) }]), &gray).unwrap().unwrap();
        assert_eq!(gray_independent.filters, base.filters);
        assert!(gray_independent.notes.iter().any(|note| note.contains("one ordinary native color component")));
        let tiny = MappingContext { remaining_metadata_bytes: 1, ..context() };
        assert!(map(&StoredEffect::test("gegl:oilify", vec![Argument { name: "use-inten", value: ArgumentValue::Bool(false) }]), &tiny).unwrap().is_err());
        let cancel = || true;
        let cancelled = MappingContext { interrupt: photocraft_raster::Interrupt::cancel_only(&cancel), ..context() };
        assert!(map(&StoredEffect::test("gegl:oilify", Vec::new()), &cancelled).unwrap().is_err());
        for args in [vec![int("intensities", 7)], vec![int("mask-radius", 101)], vec![int("exponent", 0)], vec![float("intensities", 8.5)]] {
            assert!(map(&StoredEffect::test("gegl:oilify", args), &context()).unwrap().is_err());
        }
    }
    #[test]
    fn oil_joint_and_independent_native_fields_differ_preserve_alpha_and_reopen_at_each_depth() {
        use photocraft_color::{PixelFormat, SampleType};
        use photocraft_doc::{Document, SmartObject, SmartSource};
        use photocraft_geom::Size;
        use photocraft_raster::{Interrupt, Surface};
        let joint = result("gegl:oilify", Vec::new());
        let separate = result("gegl:oilify", vec![Argument { name: "use-inten", value: ArgumentValue::Bool(false) }]);
        let Some(NativeStructure::Composite { branches }) = separate.structure else { panic!("native independent fields") };
        let b = Rect::new(0, 0, 9, 7);
        let p = &joint.filters[0].params;
        let options = photocraft_algo::FilterParams::OilPaint {
            stylization: p["stylization"].as_f64().unwrap() as f32,
            cleanliness: p["cleanliness"].as_f64().unwrap() as f32,
            scale: 1.,
            bristle_detail: 0.,
            lighting: false,
            angle: 0.,
            shine: 0.,
        };
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut source = Surface::new(PixelFormat::new(ColorMode::Rgb, depth, true));
            let pixels: Vec<_> = (0..7)
                .flat_map(|y| (0..9).flat_map(move |x| [x as f32 / 8., y as f32 / 6., if (x + y) % 3 == 0 { 1. } else { 0. }, [0.25, 0.5, 0.75][(x + y) % 3]]))
                .collect();
            source.write_region(b, &pixels);
            let painted = photocraft_algo::apply_in_with(&source, &options, b, b, None, b, &Interrupt::NONE).unwrap();
            let mut doc = Document::new("Native independent component strokes", Size::new(9, 7), ColorMode::Rgb, depth);
            doc.icc_profile = Some(photocraft_cms::Builtin::LinearSrgb.profile().to_bytes());
            for (axis, branch) in branches.iter().enumerate() {
                let mut isolated = source.clone();
                let gray: Vec<_> =
                    source.read_region(b).as_chunks::<4>().0.iter().flat_map(|pixel| [pixel[axis], pixel[axis], pixel[axis], pixel[3]]).collect();
                isolated.write_region(b, &gray);
                let mut isolated = photocraft_algo::apply_in_with(&isolated, &options, b, b, None, b, &Interrupt::NONE).unwrap();
                let routed: Vec<_> = isolated
                    .read_region(b)
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .flat_map(|pixel| {
                        let mut out = [0., 0., 0., pixel[3]];
                        out[axis] = pixel[0];
                        out
                    })
                    .collect();
                isolated.write_region(b, &routed);
                if axis != 0 {
                    isolated =
                        photocraft_algo::coverage::apply_with(&isolated, photocraft_algo::coverage::Operation::SetAlpha(1.), b, b, None, &Interrupt::NONE)
                            .unwrap();
                }
                let mut layer = Layer::new("Native painted component", LayerContent::Raster(isolated));
                layer.blend = branch.blend;
                layer.clipped = branch.clipped;
                doc.layers.push(layer);
            }
            let before = photocraft_compose::render(&doc, b).px;
            assert!(
                before.iter().zip(painted.read_region(b).as_chunks::<4>().0).any(|(a, b)| a[..3].iter().zip(&b[..3]).any(|(a, b)| (a - b).abs() > 0.005)),
                "native common vs independent fields must distinguish use-inten at{depth:?}"
            );
            for (index, pixel) in before.iter().enumerate() {
                assert!((pixel[3] - source.pixel((index % 9) as i32, (index / 9) as i32)[3]).abs() < 1e-6);
                assert!(pixel.iter().all(|v| v.is_finite()));
            }
            let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
            assert_eq!(photocraft_compose::render(&reopened, b).px, before);
            let mut smart =
                SmartObject::new(SmartSource::Embedded { file_name: "paint.pcraft".into(), bytes: std::sync::Arc::new(bytes) }, Affine::IDENTITY, None);
            smart.smart_filters = branches[1].filters.clone();
            doc.layers.clear();
            doc.layers.push(Layer::new("Editable native component paint", LayerContent::Smart(smart)));
            let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
            let LayerContent::Smart(smart) = &reopened.layers[0].content else { panic!("native smart") };
            assert_eq!(smart.smart_filters, branches[1].filters);
        }
    }
}
