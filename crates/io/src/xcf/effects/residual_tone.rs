//! Frozen B06-01/O001-007,O009 and B06-02/O012,014,015,017,018,019,024,027.
//! Parameter-derived ordinary native curves, patterns, filters and styles; no source evaluator.
use super::{ArgumentValue, MappingContext, MappingFailure, MappingResult, NativeBranch, NativeMapping, NativeStructure, StoredEffect, native_filter};
use photocraft_color::{BlendMode, Color, ColorMode, PixelFormat, SampleType};
use photocraft_doc::effects::{Bevel, BevelStyle, BevelTechnique, Contour, Effect, FxCommon};
use photocraft_doc::{Fill, Layer, LayerContent, Pattern};
use photocraft_geom::{Rect, Size};
use photocraft_raster::Surface;
use serde_json::{Value, json};
type Result<T> = std::result::Result<T, MappingFailure>;
fn num(e: &StoredEffect<'_>, name: &str, default: f64, lo: f64, hi: f64) -> Result<f64> {
    let value = e.number(name)?.unwrap_or(default);
    if !value.is_finite() || value < f64::from(lo as f32) || value > f64::from(hi as f32) {
        return Err(format!("{} {name} outside declared/meaningful native range", e.operation).into());
    }
    Ok(value)
}
fn integer(e: &StoredEffect<'_>, name: &str, default: u32, lo: u32, hi: u32) -> Result<u32> {
    let v = num(e, name, f64::from(default), f64::from(lo), f64::from(hi))?;
    if v.fract() != 0.0 {
        return Err(format!("{name} must be integral").into());
    }
    Ok(v as u32)
}
fn choice(e: &StoredEffect<'_>, name: &str, default: i32, options: &[i32]) -> Result<i32> {
    let v = e.enum_value(name)?.unwrap_or(default);
    if !options.contains(&v) {
        return Err(format!("unknown {name} enum {v}").into());
    }
    Ok(v)
}
fn allowed(e: &StoredEffect<'_>, names: &[&str]) -> Result<()> {
    for a in &e.arguments {
        if !names.contains(&a.name) {
            return Err(format!("unmapped {} argument {}", e.operation, a.name).into());
        }
    }
    Ok(())
}
fn one(command: &str, params: Value, note: &str) -> NativeMapping {
    NativeMapping { filters: vec![native_filter(command, params)], notes: vec![note.into()], ..Default::default() }
}
pub(super) fn map(e: &StoredEffect<'_>, c: &MappingContext) -> MappingResult {
    Some(match e.canonical_operation(){
 "gegl:absolute"=>{allowed(e,&[]).map(|()|NativeMapping{layer_effects:vec![Effect::ColorOverlay{common:FxCommon::new(BlendMode::Difference,1.),color:Color::BLACK}],notes:vec!["Signed source absolute becomes an ordinary native black Color Overlay in Difference mode: native abs(color - black). Native layer coverage, transparent hidden colors and compositing rules apply; no source evaluator or signed-domain LUT is required".into()],..Default::default()})},
 "gegl:alien-map"=>alien(e),"gegl:alpha-clip"=>alpha_clip(e),
 "gegl:antialias"=>{allowed(e,&[]).map(|()|one("filter.blur.surfaceBlur",json!({"radius":1.0,"threshold":2.0}),"Single-pixel source antialias becomes native minimum-radius edge-preserving smoothing; native neighborhood/threshold rules differ"))},
 "gegl:apply-lens"=>lens(e,c),"gegl:bayer-matrix"=>bayer(e,c),"gegl:bevel"=>bevel(e,c),"gegl:bloom"=>bloom(e),
 "gegl:bump-map"=>Err(MappingFailure::resource("Bump-map auxiliary source pixels are not present in the stored scalar configuration; no external resource or owner luminance was substituted")),
 "gegl:cartoon"=>cartoon(e),
 "gegl:cell-noise"=>cell_noise(e,c),
 "gegl:checkerboard"=>checker(e,c),
 "gegl:color-assimilation-grid"=>assimilation(e,c),
 "gegl:color-enhance"=>allowed(e,&[]).map(|()|one("image.autoTone",json!({}),"Source automatic color enhancement becomes native per-channel Auto Tone analysis of the actual prepared input; fixed ordinary Levels replace analysis before first display/save. Native 0.1% display-bin clipping and RGB channel ranges differ from source chroma/color enhancement; this is native adaptive intent, not a fixed saturation preset")),
 "gegl:color-to-alpha"=>color_to_alpha(e,c),
 "gegl:convolution-matrix"=>convolution(e,c),_=>return None})
}
fn saturation_filters(factor: f64) -> Vec<photocraft_doc::SmartFilter> {
    let mut remaining = factor;
    let mut filters = Vec::new();
    loop {
        let gain = remaining.min(2.);
        let rows: Vec<Vec<f64>> = (0..3)
            .map(|row| {
                let mut coefficients: Vec<f64> = [0.2126, 0.7152, 0.0722]
                    .into_iter()
                    .enumerate()
                    .map(|(column, weight)| 100. * ((1. - gain) * weight + if column == row { gain } else { 0. }))
                    .collect();
                coefficients.push(0.);
                coefficients
            })
            .collect();
        filters.push(native_filter("image.adjustments.channelMixer", json!({"red":rows.first(),"green":rows.get(1),"blue":rows.get(2),"monochrome":false})));
        if remaining <= 2. {
            break;
        }
        remaining /= 2.;
    }
    filters
}
fn assimilation(e: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping> {
    allowed(e, &["grid-size", "saturation", "angle", "line-thickness"])?;
    let period = num(e, "grid-size", 23., 0., 150.)?;
    let saturation = num(e, "saturation", 2.5, 0., 30.)?;
    let angle = num(e, "angle", 45., -180., 180.)?;
    let thickness = num(e, "line-thickness", 0.4, 0., 1.)?;
    let gray = saturation_filters(0.);
    if period == 0. || thickness == 0. {
        return Ok(NativeMapping{filters:gray,notes:vec!["Zero grid size/thickness becomes native grayscale, matching the observed empty colored-grid intent; native luma coefficients/color-space behavior differ".into()],..Default::default()});
    }
    let rotation = photocraft_geom::Affine::rotate(angle.to_radians());
    let inverse = photocraft_geom::Affine::rotate(-angle.to_radians());
    let bounds = context.native_filter_bounds;
    let corners = [(bounds.x0, bounds.y0), (bounds.x1, bounds.y0), (bounds.x0, bounds.y1), (bounds.x1, bounds.y1)]
        .map(|(x, y)| inverse.apply(photocraft_geom::Point::new(f64::from(x), f64::from(y))));
    let min_x = corners.iter().map(|p| p.x).fold(f64::INFINITY, f64::min);
    let max_x = corners.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max);
    let min_y = corners.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
    let max_y = corners.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max);
    let first = [(min_x / period).floor(), (min_y / period).floor()];
    let last = [(max_x / period).ceil(), (max_y / period).ceil()];
    if first.iter().chain(last.iter()).any(|v| !v.is_finite() || v.abs() > 1e12) {
        return Err("Native assimilation grid coordinate count exceeds bounded geometry".into());
    }
    let starts = first.map(|v| (v as i64).saturating_sub(1));
    let ends = last.map(|v| (v as i64).saturating_add(1));
    let count = starts
        .into_iter()
        .zip(ends)
        .try_fold(0usize, |count, (start, end)| usize::try_from(end.saturating_sub(start).saturating_add(1)).ok().and_then(|n| count.checked_add(n)))
        .ok_or("native assimilation grid count overflow")?;
    if count > 8192 || count.checked_mul(512).is_none_or(|bytes| bytes > context.remaining_generation_bytes) {
        return Err("Native assimilation vector grid exceeds bounded geometry/generation allowance".into());
    }
    if count.checked_mul(1024).is_none_or(|bytes| bytes > context.remaining_metadata_bytes) {
        return Err(MappingFailure {
            reason: photocraft_doc::import_conversion::ConversionReason::RetentionLimit,
            message: "Native assimilation grid exceeds remaining metadata allowance".into(),
        });
    }
    let mut paths = Vec::new();
    paths.try_reserve_exact(count).map_err(|_| "native assimilation grid allocation unavailable")?;
    let half = period * thickness * 0.5;
    for axis in 0..2 {
        let start = starts.get(axis).copied().ok_or("native grid axis")?;
        let end = ends.get(axis).copied().ok_or("native grid axis")?;
        for index in start..=end {
            context.interrupt.check().map_err(|error| error.to_string())?;
            let center = index as f64 * period;
            let points = if axis == 0 {
                [(center - half, min_y - period), (center + half, min_y - period), (center + half, max_y + period), (center - half, max_y + period)]
            } else {
                [(min_x - period, center - half), (max_x + period, center - half), (max_x + period, center + half), (min_x - period, center + half)]
            };
            let points = points.map(|(x, y)| {
                let p = rotation.apply(photocraft_geom::Point::new(x, y));
                (p.x, p.y)
            });
            paths.push(photocraft_doc::Subpath::polygon(&points));
        }
    }
    let mask = photocraft_doc::VectorMask::new(photocraft_doc::Path::new(paths));
    Ok(NativeMapping{structure:Some(NativeStructure::Composite{branches:vec![NativeBranch{filters:gray,..Default::default()},NativeBranch{filters:saturation_filters(saturation),vector_mask:Some(mask),clipped:true,..Default::default()}]}),notes:vec!["Observed gray-base/colored-grid intent becomes ordinary native gray Channel Mixer plus a saturated source branch clipped to the gray base and editable vector rectangles. Grid period/angle/fractional line thickness control native geometry; saturation becomes bounded native luminance-preserving matrix gains (multiple native passes above 2). Native clipping preserves base alpha; native luma weights, 1/1024 coefficient quantization, working color, grid phase, vector antialias and per-layer alpha mixing differ from source rendering. All pixels/colors derive from the actual input, with no source graph or cached source render".into()],..Default::default()})
}

fn cell_noise(e: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping> {
    allowed(e, &["scale", "shape", "rank", "iterations", "palettize", "seed"])?;
    let scale = num(e, "scale", 1., 0., 20.)?;
    let shape = num(e, "shape", 2., 1., 2.)?;
    let rank = integer(e, "rank", 1, 1, 3)?;
    let iterations = integer(e, "iterations", 1, 1, 20)?;
    let palette = e.boolean("palettize")?.unwrap_or(false);
    let seed = integer(e, "seed", 0, 0, u32::MAX)?;
    let bounds = context.native_filter_bounds;
    let width = bounds.width();
    let height = bounds.height();
    let work = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|n| n.checked_mul(u64::from(iterations)))
        .and_then(|n| n.checked_mul(25))
        .ok_or("native cellular pattern work count overflow")?;
    if work > 32_000_000 {
        return Err("Native cellular pattern exceeds bounded resource-authoring work allowance".into());
    }
    pattern_result(
        "Imported native cellular pattern",
        Size::new(width, height),
        (bounds.x0 as f32, bounds.y0 as f32),
        0.,
        context,
        |x, y| {
            let mut sum = 0.;
            let mut weight = 0.;
            let mut amplitude = 1.;
            let mut frequency = scale / 64.;
            for octave in 0..iterations {
                let px = (f64::from(x) + f64::from(bounds.x0)) * frequency;
                let py = (f64::from(y) + f64::from(bounds.y0)) * frequency;
                let ix = px.floor() as i64;
                let iy = py.floor() as i64;
                let mut candidates = [(f64::INFINITY, 0.); 25];
                let mut index = 0usize;
                // Original public Voronoi primitive: one jittered feature point in
                // each nearby lattice cell, then a ranked Minkowski distance.
                for dy in -2..=2 {
                    for dx in -2..=2 {
                        let cx = ix.saturating_add(dx);
                        let cy = iy.saturating_add(dy);
                        let random = |axis| super::residual_transform::lattice(cx, cy, i64::from(octave) * 3 + axis, seed);
                        let delta_x = (px - cx as f64 - random(0)).abs();
                        let delta_y = (py - cy as f64 - random(1)).abs();
                        let distance = (delta_x.powf(shape) + delta_y.powf(shape)).powf(1. / shape);
                        if let Some(candidate) = candidates.get_mut(index) {
                            *candidate = (distance, random(2));
                        }
                        index += 1;
                    }
                }
                candidates.sort_by(|a, b| a.0.total_cmp(&b.0));
                let chosen = candidates.get(rank.saturating_sub(1) as usize).ok_or("native cellular rank")?;
                let value = if palette { chosen.1 } else { (chosen.0 / f64::from(rank).sqrt()).clamp(0., 1.) };
                sum += amplitude * value;
                weight += amplitude;
                amplitude *= 0.5;
                frequency *= 2.;
            }
            let value = (sum / weight).clamp(0., 1.) as f32;
            Ok([value, value, value, 1.])
        },
        "Original native Voronoi/Minkowski ranked-cell math becomes ordinary native Pattern pixels, not a source-node evaluator. Source scale controls approximate cell frequency, shape the native distance norm, rank the nearest feature, iterations a normalized octave sum, palettize a deterministic cell shade and seed feature placement. One native feature per cell, finite neighbor search, amplitude/frequency policy, normalization and random sequence approximate the source statistics; generated pixels use the actual native target ICC",
    )
}

fn color_to_alpha(e: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping> {
    allowed(e, &["color", "transparency-threshold", "opacity-threshold"])?;
    let transparency = num(e, "transparency-threshold", 0., 0., 1.)?;
    let opacity = num(e, "opacity-threshold", 1., 0., 1.)?;
    let color = if e.argument("color")?.is_some() { super::resources::native_color(e, "color", context)? } else { Color::WHITE };
    let rgb = color.to_rgb();
    if rgb.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
        return Err("Color-to-alpha key lies outside native bounded key gamut after target ICC conversion".into());
    }
    Ok(one(
        "filter.other.colorToAlpha",
        json!({"color":[rgb[0],rgb[1],rgb[2],1.],"transparencyThreshold":transparency,"opacityThreshold":opacity}),
        "Typed source color is converted through its declared ICC into the native target model; source thresholds become ordinary editable native Color to Alpha parameters. Native stored-channel unmixing and alpha multiplication preserve PhotoCraft compositing and editing; source color-space/threshold rules may differ. Key alpha is intentionally ignored as in the native command; original source configuration remains retained",
    ))
}

fn alien(e: &StoredEffect<'_>) -> Result<NativeMapping> {
    allowed(
        e,
        &[
            "color-model",
            "cpn-1-frequency",
            "cpn-2-frequency",
            "cpn-3-frequency",
            "cpn-1-phaseshift",
            "cpn-2-phaseshift",
            "cpn-3-phaseshift",
            "cpn-1-keep",
            "cpn-2-keep",
            "cpn-3-keep",
        ],
    )?;
    let model = choice(e, "color-model", 0, &[0, 1])?;
    let mut p = serde_json::Map::new();
    p.insert("pencil".into(), json!(true));
    for (index, key) in ["red", "green", "blue"].iter().enumerate() {
        let channel = index + 1;
        let f = num(e, &format!("cpn-{channel}-frequency"), 1., 0., 20.)?;
        let phase = num(e, &format!("cpn-{channel}-phaseshift"), 0., 0., 360.)? / 360.;
        let keep = e.boolean(&format!("cpn-{channel}-keep"))?.unwrap_or(false);
        let table: Vec<f64> = (0..256)
            .map(|v| {
                let x = f64::from(v) / 255.;
                if keep {
                    x * 255.
                } else {
                    let t = (x * f + phase + 0.25).rem_euclid(1.);
                    (1. - (2. * t - 1.).abs()) * 255.
                }
            })
            .collect();
        p.insert((*key).into(), json!(table));
    }
    let mut filters = Vec::new();
    if model == 1 {
        filters.push(native_filter("filter.other.hsbHsl", json!({"inputMode":"rgb","rowOrder":"hsl"})));
    }
    filters.push(native_filter("image.adjustments.curves", Value::Object(p)));
    if model == 1 {
        filters.push(native_filter("filter.other.hsbHsl", json!({"inputMode":"hsl","rowOrder":"rgb"})));
    }
    Ok(NativeMapping{filters,notes:vec!["Source frequency/phase/keep controls become bounded editable native piecewise-linear periodic Curves; native triangular waveform, finite curve sampling, HSL conversion, document encoding and HDR gamut differ from source remapping".into()],..Default::default()})
}
fn alpha_clip(e: &StoredEffect<'_>) -> Result<NativeMapping> {
    allowed(e, &["clip-low", "low-limit", "clip-high", "high-limit"])?;
    let low = num(e, "low-limit", 0., -f64::from(f32::MAX), f64::from(f32::MAX))?;
    let high = num(e, "high-limit", 1., -f64::from(f32::MAX), f64::from(f32::MAX))?;
    let cliplo = e.boolean("clip-low")?.unwrap_or(true);
    let cliphi = e.boolean("clip-high")?.unwrap_or(true);
    if (!cliplo || low <= 0.) && (!cliphi || high >= 1.) {
        Ok(NativeMapping {
            notes: vec!["Alpha clip bounds include the full validated native coverage range0..1; native coverage is unchanged".into()],
            ..Default::default()
        })
    } else if cliplo && cliphi && low == high && (0.0..=1.0).contains(&low) {
        Ok(one(
            "filter.other.setAlpha",
            json!({"value":low}),
            "Equal enabled source alpha clip bounds become ordinary native constant Set Alpha coverage; native stored colors/depth and editability are preserved",
        ))
    } else {
        let effective_low = if cliplo { low.max(0.) } else { 0. };
        let effective_high = if cliphi { high.min(1.) } else { 1. };
        if effective_low > effective_high || !(0.0..=1.0).contains(&effective_low) || !(0.0..=1.0).contains(&effective_high) {
            return Err("Source alpha clip produces crossed or out-of-native-coverage bounds; configuration retained without silent range clamping".into());
        }
        Ok(one(
            "filter.other.clampAlpha",
            json!({"low":effective_low,"high":effective_high}),
            "Enabled source alpha bounds become ordinary editable native Clamp Alpha parameters; outward or disabled bounds reduce to the existing native coverage domain 0..1. Native color/HDR samples and depth are preserved; only native coverage changes",
        ))
    }
}
fn lens(e: &StoredEffect<'_>, context: &MappingContext<'_>) -> Result<NativeMapping> {
    use photocraft_doc::{Path, Subpath, VectorMask};
    allowed(e, &["refraction-index", "keep-surroundings", "background-color"])?;
    let index = num(e, "refraction-index", 1.7, 1.0, 100.0)?;
    let keep = e.boolean("keep-surroundings")?.unwrap_or(false);
    if let Some(value) = e.argument("background-color")?
        && !matches!(value, ArgumentValue::Color(_) | ArgumentValue::LiteralColor { .. })
    {
        return Err("invalid lens background color".into());
    }
    let bounds = context.native_filter_bounds;
    if bounds.is_empty() {
        return Err("native lens requires nonempty owner bounds".into());
    }
    if context.remaining_generation_bytes < 4096 || context.remaining_metadata_bytes < 4096 {
        return Err("native lens geometry/parameters exceed remaining allowance".into());
    }
    context.interrupt.check().map_err(|e| e.to_string())?;
    let (x, y, w, h) = (f64::from(bounds.x0), f64::from(bounds.y0), f64::from(bounds.width()), f64::from(bounds.height()));
    let ellipse = VectorMask::new(photocraft_vector::shapes::ellipse(x, y, w, h));
    let underlay = if keep {
        NativeBranch::default()
    } else {
        let color = match e.argument("background-color")? {
            None => super::resources::native_rgba([0.0, 0.0, 0.0, 0.0], context)?,
            Some(ArgumentValue::Color(Some(_)) | ArgumentValue::LiteralColor { .. }) => super::resources::native_color(e, "background-color", context)?,
            Some(ArgumentValue::Color(None)) => {
                return Err(MappingFailure::resource("Stored lens background color is absent; cannot resolve actual source paint"));
            }
            _ => return Err("invalid lens background color".into()),
        };
        let extent = VectorMask::new(Path::new(vec![Subpath::polygon(&[(x, y), (x + w, y), (x + w, y + h), (x, y + h)])]));
        NativeBranch { fill: Some(Fill::Solid(color)), vector_mask: Some(extent), ..Default::default() }
    };
    let sphere = NativeBranch {
        filters: vec![native_filter("filter.distort.spherize", json!({"amount":100.0*(1.0-index.recip()),"mode":"normal"}))],
        vector_mask: Some(ellipse),
        ..Default::default()
    };
    Ok(NativeMapping {
        structure: Some(NativeStructure::Composite { branches: vec![underlay, sphere] }),
        notes: vec![format!(
            "Source refraction index {index} becomes native Spherize optical power 1 - 1/index inside an ordinary editable ellipse vector mask. Native sphere geometry, interpolation and antialiasing differ. Keep surroundings {keep} selects the original native input underlay; otherwise actual stored background paint is converted through its explicit ICC to native document color and placed as a finite native Solid Fill (absent default is transparent, as declared by source). Native compositing alpha applies across the whole outside-lens region; background hidden beneath translucent lens pixels also follows native source-over rules"
        )],
        ..Default::default()
    })
}

fn bevel(e: &StoredEffect<'_>, c: &MappingContext) -> Result<NativeMapping> {
    allowed(e, &["type", "blendmode", "metric", "radius", "elevation", "depth", "azimuth"])?;
    let kind = choice(e, "type", 0, &[0, 1])?;
    let metric = choice(e, "metric", 2, &[0, 1, 2])?;
    let blend = match choice(e, "blendmode", 1, &[0, 1, 2, 3, 4, 5, 6])? {
        0 => BlendMode::Normal,
        1 => BlendMode::HardLight,
        2 => BlendMode::Multiply,
        3 => BlendMode::ColorDodge,
        4 => BlendMode::Darken,
        5 => BlendMode::Lighten,
        _ => BlendMode::LinearDodge,
    };
    let size = num(e, "radius", 3., 1., 8.)? as f32;
    let elevation = num(e, "elevation", 25., 0., 180.)?;
    let angle = num(e, "azimuth", 68., 0., 360.)? as f32;
    let depth = integer(e, "depth", 40, 1, 100)? as f32 / 100.;
    let b = Bevel {
        enabled: true,
        style: if kind == 0 { BevelStyle::InnerBevel } else { BevelStyle::Emboss },
        technique: match metric {
            0 => BevelTechnique::Smooth,
            1 => BevelTechnique::ChiselSoft,
            _ => BevelTechnique::ChiselHard,
        },
        depth,
        up: elevation <= 90.,
        size,
        soften: 0.,
        angle,
        altitude: if elevation <= 90. { elevation } else { 180. - elevation } as f32,
        use_global_light: false,
        gloss_contour: Contour::Linear,
        gloss_anti_alias: false,
        highlight: FxCommon::new(blend, 1.),
        highlight_color: Color::WHITE.in_mode(c.mode),
        shadow: FxCommon::new(BlendMode::Multiply, 1.),
        shadow_color: Color::BLACK.in_mode(c.mode),
        contour: None,
        texture: None,
    };
    Ok(NativeMapping{layer_effects:vec![Effect::BevelEmboss(b)],notes:vec!["Chamfer/bump and distance metric become native bevel style/technique approximations; radius,light angles and depth percentage remain editable. Native bevel shading, blend placement and native contour differ".into()],..Default::default()})
}
fn bloom(e: &StoredEffect<'_>) -> Result<NativeMapping> {
    allowed(e, &["threshold", "softness", "radius", "strength", "limit-exposure"])?;
    let threshold = num(e, "threshold", 50., 0., 100.)? / 100.;
    let softness = num(e, "softness", 25., 0., 100.)? / 100.;
    let radius = num(e, "radius", 10., 0., 1500.)?;
    let strength = num(e, "strength", 50., 0., 100.)? / 100.;
    let limit = e.boolean("limit-exposure")?.unwrap_or(false);
    let start = (threshold - softness * 0.5).max(0.);
    let end = (threshold + softness * 0.5).min(1.).max(start + 1. / 255.);
    let curve: Vec<f64> = (0..256)
        .map(|v| {
            let x = f64::from(v) / 255.;
            let weight = ((x - start) / (end - start)).clamp(0., 1.);
            x * weight * 255.
        })
        .collect();
    let mut filters = vec![native_filter("image.adjustments.curves", json!({"pencil":true,"points":curve}))];
    if radius > 0. {
        let passes = ((radius / 1000.).powi(2).ceil() as u32).max(1);
        for _ in 0..passes {
            filters.push(native_filter("filter.blur.gaussianBlur", json!({"radius":(radius/f64::from(passes).sqrt()).max(0.1)})));
        }
    }
    Ok(NativeMapping{structure:Some(NativeStructure::Composite{branches:vec![NativeBranch::default(),NativeBranch{filters,blend:if limit{BlendMode::Screen}else{BlendMode::LinearDodge},opacity:strength as f32,..Default::default()}]}),notes:vec!["Native highlight Curves→Gaussian branch recombines over ordinary native original pixels. Source threshold/softness percentages use a native linear highlight ramp; native per-channel highlight choice,blur kernel,screen/add and HDR clipping differ".into()],..Default::default()})
}
fn cartoon(e: &StoredEffect<'_>) -> Result<NativeMapping> {
    allowed(e, &["mask-radius", "pct-black"])?;
    let radius = num(e, "mask-radius", 7., 0., 50.)?;
    let black = num(e, "pct-black", 0.2, 0., 1.)?;
    let mut filters = Vec::new();
    if radius > 0. {
        filters.push(native_filter("filter.blur.surfaceBlur", json!({"radius":radius.max(1.),"threshold":15.})));
    }
    let mut edge = native_filter("filter.stylize.findEdges", json!({}));
    edge.blend = BlendMode::Multiply;
    filters.push(edge);
    filters.push(native_filter("image.adjustments.levels", json!({"inBlack":black*253.,"inWhite":255.,"gamma":1.,"outBlack":0.,"outWhite":255.})));
    Ok(NativeMapping{filters,notes:vec!["Source mask radius becomes native edge-preserving preprocessing; native edge Multiply and black-point Levels approximate cartoon outlines. pct-black is a native black-point proportion, not the source region-count algorithm; native edges/colors/clipping differ".into()],..Default::default()})
}
pub(super) fn pattern_result(
    name: &str,
    size: Size,
    phase: (f32, f32),
    angle: f32,
    c: &MappingContext,
    mut sample: impl FnMut(u32, u32) -> Result<[f32; 4]>,
    note: &str,
) -> Result<NativeMapping> {
    use photocraft_cms::{Builtin, ColorSpace, Intent, Profile, Transform};
    let (width, height) = (size.width, size.height);
    let count = u64::from(width).checked_mul(u64::from(height)).ok_or("native pattern area overflow")?;
    if width == 0
        || height == 0
        || width > photocraft_doc::pattern::MAX_EDGE
        || height > photocraft_doc::pattern::MAX_EDGE
        || count > photocraft_doc::pattern::MAX_PAINT_PIXELS
    {
        return Err("source pattern exceeds ordinary native pattern edge/paint limits".into());
    }
    c.interrupt.check().map_err(|error| error.to_string())?;
    if !matches!(c.mode, ColorMode::Rgb | ColorMode::Grayscale) {
        return Err("native generated patterns require an RGB or Gray target".into());
    }
    let format = PixelFormat::new(c.mode, SampleType::F32, true);
    let channels = format.channels();
    let row_width = usize::try_from(width).map_err(|_| "native pattern row width overflow")?;
    let source_len = row_width.checked_mul(4).ok_or("native source pattern row overflow")?;
    let converted_len = row_width.checked_mul(channels).ok_or("native converted pattern row overflow")?;
    // The surface allocates complete native tiles, even for a one-pixel image.
    // Count their actual padded storage before sampling or allocating anything.
    let edge = u32::try_from(photocraft_geom::TILE_SIZE).map_err(|_| "native tile edge")?;
    let tiles = usize::try_from(width.div_ceil(edge))
        .ok()
        .and_then(|x| usize::try_from(height.div_ceil(edge)).ok().and_then(|y| x.checked_mul(y)))
        .ok_or("native pattern tile count overflow")?;
    let tile_pixels = usize::try_from(edge).ok().and_then(|v| v.checked_mul(v)).ok_or("native tile area overflow")?;
    let surface_bytes = tiles.checked_mul(tile_pixels).and_then(|v| v.checked_mul(format.bytes_per_pixel())).ok_or("native pattern tile storage overflow")?;
    let scratch_bytes =
        source_len.checked_add(converted_len).and_then(|v| v.checked_mul(std::mem::size_of::<f32>())).ok_or("native pattern row scratch overflow")?;
    let peak = surface_bytes.checked_add(scratch_bytes).ok_or("native pattern working storage overflow")?;
    if peak > c.remaining_generation_bytes {
        return Err("native pattern padded tiles and row scratch exceed remaining generation allowance".into());
    }
    let fallback = if c.mode == ColorMode::Grayscale {
        if c.linear { photocraft_cms::builtin::linear_gray().clone() } else { Builtin::SGray.profile().clone() }
    } else {
        if c.linear { Builtin::LinearSrgb } else { Builtin::Srgb }.profile().clone()
    };
    let target = match &c.target_profile {
        Some(bytes) => Profile::parse(bytes).map_err(|e| e.to_string())?,
        None => fallback,
    };
    let expected = if c.mode == ColorMode::Grayscale { ColorSpace::Gray } else { ColorSpace::Rgb };
    if target.color_space != expected {
        return Err("native pattern target ICC model mismatch".into());
    }
    let mut rgba = Vec::new();
    rgba.try_reserve_exact(source_len).map_err(|_| "native source pattern row allocation unavailable")?;
    let mut pixels = Vec::new();
    pixels.try_reserve_exact(converted_len).map_err(|_| "native converted pattern allocation unavailable")?;
    pixels.resize(converted_len, 0.);
    let transform = Transform::new(Builtin::Srgb.profile(), &target, Intent::RelativeColorimetric, false).map_err(|e| e.to_string())?;
    let mut surface = Surface::new(format);
    for y in 0..height {
        c.interrupt.check().map_err(|error| error.to_string())?;
        rgba.clear();
        for x in 0..width {
            if x % 256 == 0 {
                c.interrupt.check().map_err(|error| error.to_string())?;
            }
            let px = sample(x, y)?;
            if px.iter().any(|v| !v.is_finite()) || !(0.0..=1.0).contains(&px[3]) {
                return Err("generated native pattern has nonfinite colors or invalid coverage".into());
            }
            rgba.extend_from_slice(&px);
        }
        c.interrupt.check().map_err(|error| error.to_string())?;
        transform.convert_f32(&rgba, 4, &mut pixels, channels, true);
        if pixels.iter().any(|v| !v.is_finite()) {
            return Err("native pattern CMS conversion produced nonfinite samples".into());
        }
        c.interrupt.check().map_err(|error| error.to_string())?;
        surface.try_write_region(Rect::new(0, y as i32, width as i32, y as i32 + 1), &pixels).map_err(|error| error.to_string())?;
    }
    // Hashing needs only one converted row after both CMS scratch rows are gone.
    drop(rgba);
    drop(pixels);
    let hash_allowance = c.remaining_generation_bytes.checked_sub(surface_bytes).ok_or("native pattern hash allowance exhausted")?;
    let pattern = Pattern::try_new_in(name, surface, width, height, hash_allowance, &c.interrupt).map_err(|error| error.to_string())?;
    let layer = Layer::new(name, LayerContent::Fill(Fill::Pattern { name: pattern.name.clone(), id: pattern.id.clone(), scale: 1., angle, link: true, phase }));
    Ok(NativeMapping {
        structure: Some(NativeStructure::Replace { layer: Box::new(layer), patterns: vec![pattern] }),
        notes: vec![note.into()],
        ..Default::default()
    })
}
fn offset(e: &StoredEffect<'_>, name: &str) -> Result<f32> {
    let value = num(e, name, 0., f64::from(i32::MIN), f64::from(i32::MAX))?;
    if value.fract() != 0. {
        return Err(format!("{name} must be integral").into());
    }
    Ok(value as f32)
}
fn rotation(e: &StoredEffect<'_>) -> Result<f32> {
    match e.argument("rotation")? {
        None => Ok(0.),
        Some(ArgumentValue::String("0")) => Ok(0.),
        Some(ArgumentValue::String("90")) => Ok(90.),
        Some(ArgumentValue::String("180")) => Ok(180.),
        Some(ArgumentValue::String("270")) => Ok(270.),
        Some(ArgumentValue::Enum(v)) if (0..=3).contains(v) => Ok(*v as f32 * 90.),
        _ => Err("unknown Bayer quarter-turn rotation".into()),
    }
}
fn bayer(e: &StoredEffect<'_>, c: &MappingContext) -> Result<NativeMapping> {
    allowed(e, &["subdivisions", "x-scale", "y-scale", "rotation", "reflect", "amplitude", "offset", "exponent", "x-offset", "y-offset"])?;
    let subdivisions = integer(e, "subdivisions", 1, 0, 15)?;
    let edge = 1u32.checked_shl(subdivisions).ok_or("Bayer dimension overflow")?;
    let xs = integer(e, "x-scale", 1, 1, i32::MAX as u32)?;
    let ys = integer(e, "y-scale", 1, 1, i32::MAX as u32)?;
    let width = edge.checked_mul(xs).ok_or("Bayer X scale overflow")?;
    let height = edge.checked_mul(ys).ok_or("Bayer Y scale overflow")?;
    let angle = rotation(e)?;
    let reflect = e.boolean("reflect")?.unwrap_or(false);
    let amplitude = num(e, "amplitude", 0., -f64::from(f32::MAX), f64::from(f32::MAX))?;
    let exponent = num(e, "exponent", 0., -f64::from(f32::MAX), f64::from(f32::MAX))?;
    let bias = num(e, "offset", 0., -f64::from(f32::MAX), f64::from(f32::MAX))?;
    let gain = amplitude.exp2();
    let power = exponent.exp2();
    if !gain.is_finite() || !power.is_finite() || power == 0. {
        return Err("Bayer logarithmic amplitude/exponent cannot produce finite native pattern values".into());
    }
    let phase = (offset(e, "x-offset")?, offset(e, "y-offset")?);
    let denominator = f64::from(edge) * f64::from(edge);
    pattern_result(
        "Imported ordered threshold pattern",
        Size::new(width, height),
        phase,
        angle,
        c,
        |x, y| {
            let x = if reflect { edge - 1 - x / xs } else { x / xs };
            let y = y / ys;
            let mut rank = 0u32;
            for bit in 0..subdivisions {
                let xb = (x >> bit) & 1;
                let yb = (y >> bit) & 1;
                let quadrant = ((xb ^ yb) << 1) | yb;
                rank = rank.checked_mul(4).and_then(|n| n.checked_add(quadrant)).ok_or("Bayer rank overflow")?;
            }
            let value = (f64::from(rank) + 0.5) / denominator * gain + bias;
            let tone = value.signum() * value.abs().powf(power);
            if !tone.is_finite() || tone.abs() > f64::from(f32::MAX) {
                return Err("Bayer tone exceeds native finite F32 range".into());
            }
            Ok([tone as f32, tone as f32, tone as f32, 1.])
        },
        "Standard Bayer ordered threshold data becomes an ordinary editable native Pattern Fill. Logarithmic amplitude/exponent and source offsets are translated; native pattern rotation/linking, finite storage/CMS and document compositing apply; source procedural controls remain retained historical data",
    )
}
fn checker(e: &StoredEffect<'_>, c: &MappingContext) -> Result<NativeMapping> {
    allowed(e, &["x", "y", "x-offset", "y-offset", "color1", "color2", "format"])?;
    if e.argument("format")?.is_some() {
        return Err("unknown explicitly stored checker pixel format cannot change native pattern interpretation".into());
    }
    let x = integer(e, "x", 16, 1, i32::MAX as u32)?;
    let y = integer(e, "y", 16, 1, i32::MAX as u32)?;
    let width = x.checked_mul(2).ok_or("checker width overflow")?;
    let height = y.checked_mul(2).ok_or("checker height overflow")?;
    let color = |name: &str, default: [f32; 4]| -> Result<[f32; 4]> {
        match e.argument(name)? {
            None => Ok(default),
            Some(ArgumentValue::Color(Some(_)) | ArgumentValue::LiteralColor { .. }) => super::resources::color(e, name).map_err(Into::into),
            _ => Err(format!("checker {name} is not a usable typed source color").into()),
        }
    };
    let first = color("color1", [0., 0., 0., 1.])?;
    let second = color("color2", [1.; 4])?;
    let phase = (offset(e, "x-offset")?, offset(e, "y-offset")?);
    pattern_result(
        "Imported checker pattern",
        Size::new(width, height),
        phase,
        0.,
        c,
        |px, py| Ok(if (px / x + py / y) % 2 == 0 { first } else { second }),
        "Typed source checker colors and cell sizes become a native Pattern Fill, converted through their own source spaces and actual target ICC. Native pattern phase/linking and document compositing are authoritative",
    )
}
fn convolution(e: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping> {
    let mut names: Vec<String> =
        ["divisor", "offset", "red", "green", "blue", "alpha", "normalize", "alpha-weight", "border"].iter().map(|v| (*v).to_string()).collect();
    for column in ['a', 'b', 'c', 'd', 'e'] {
        for row in 1..=5 {
            names.push(format!("{column}{row}"));
        }
    }
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    allowed(e, &refs)?;
    let mut kernel = Vec::with_capacity(25);
    // Source letters identify X columns and numerals Y rows; native Custom stores row-major.
    for row in 1..=5 {
        for column in ['a', 'b', 'c', 'd', 'e'] {
            let key = format!("{column}{row}");
            kernel.push(num(e, &key, if key == "c3" { 1. } else { 0. }, -f64::MAX, f64::MAX)?);
        }
    }
    let rgb = [e.boolean("red")?.unwrap_or(true), e.boolean("green")?.unwrap_or(true), e.boolean("blue")?.unwrap_or(true)];
    let alpha = e.boolean("alpha")?.unwrap_or(true);
    let weighted = e.boolean("alpha-weight")?.unwrap_or(true);
    let border = choice(e, "border", 1, &[0, 1, 2, 3, 4])?;
    let normalize = e.boolean("normalize")?.unwrap_or(true);
    let requested = num(e, "divisor", 1., -f64::MAX, f64::MAX)?;
    let offset = num(e, "offset", 0., -1., 1.)?;
    let sum: f64 = kernel.iter().sum();
    let divisor = if normalize { if sum == 0. { 1. } else { sum } } else { requested };
    if divisor == 0. || !divisor.is_finite() || !kernel.iter().map(|v| v.abs()).sum::<f64>().is_finite() {
        return Err(MappingFailure {
            reason: photocraft_doc::import_conversion::ConversionReason::NumericalConversion,
            message: "Native convolution requires finite nonzero divisor and finite absolute coefficient support".into(),
        });
    }
    let mut channels = if context.mode == ColorMode::Rgb { rgb.to_vec() } else { vec![rgb.into_iter().any(|v| v); context.mode.color_channels()] };
    channels.push(alpha);
    Ok(one(
        "filter.other.convolution",
        json!({"kernel":kernel,"scale":divisor,"offset":offset,"channels":channels,"alphaWeighted":weighted,"edge":match border{0=>"transparent",1=>"repeat",2=>"wrap",3=>"black",_=>"white"}}),
        "Source row-major coefficients, divisor/normalization, normalized offset, selected native channels, alpha enable and all five boundary choices become an ordinary native Convolution filter. Native alphaWeighted uses absolute-support normalization sum(k*C*A)*sum(abs(k))/sum(abs(k)*A), not division by signed convolved alpha; support below1e-12 of total preserves hidden finite HDR colors. This is an explicit native normalized-convolution approximation, not a source alpha evaluator. Native alpha clamps0..1; CMYK uses native brightness polarity and boundary solids use stored neutral axes. Scalar Gray/non-RGB owners enable their native color axes when any source RGB flag is enabled (source separate RGB channel choices collapse explicitly to the native document model)",
    ))
}
#[cfg(test)]
mod tests {
    use super::super::{Argument, ColorLiteral};
    use super::*;
    fn ctx() -> MappingContext<'static> {
        MappingContext {
            mode: ColorMode::Rgb,
            linear: true,
            owner_bounds: Rect::new(-2, 3, 7, 8),
            native_filter_bounds: Rect::new(0, 0, 9, 5),
            target_profile: Some(photocraft_cms::Builtin::LinearSrgb.profile().to_bytes()),
            resolution_dpi: 72.0,
            remaining_generation_bytes: crate::xcf::MAX_PIXEL_BYTES,
            remaining_metadata_bytes: crate::xcf::MAX_METADATA_BYTES,
            interrupt: photocraft_raster::Interrupt::NONE,
        }
    }
    fn arg(name: &'static str, value: f32) -> Argument<'static> {
        Argument { name, value: ArgumentValue::Float(value) }
    }
    fn int(name: &'static str, value: i32) -> Argument<'static> {
        Argument { name, value: ArgumentValue::Int(value) }
    }
    fn flag(name: &'static str, value: bool) -> Argument<'static> {
        Argument { name, value: ArgumentValue::Bool(value) }
    }
    #[test]
    fn lens_native_ellipse_preserves_or_replaces_the_entire_surrounding_region() {
        use photocraft_doc::Document;
        use photocraft_raster::Interrupt;
        let bounds = Rect::new(0, 0, 24, 16);
        let mut context = ctx();
        context.native_filter_bounds = bounds;
        context.owner_bounds = bounds;
        let mut original = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::F32, true));
        original.write_region(bounds, &(0..384).flat_map(|i| [i as f32 / 512.0, 0.25, 0.75, 0.25]).collect::<Vec<_>>());
        for (keep, paint) in [(true, false), (false, false), (false, true)] {
            let mut arguments = vec![flag("keep-surroundings", keep), arg("refraction-index", 2.0)];
            if paint {
                arguments.push(Argument {
                    name: "background-color",
                    value: ArgumentValue::Color(Some(ColorLiteral { encoding: "R'G'B'A u8", pixels: &[128, 64, 32, 128], profile: &[] })),
                });
            }
            let mapped = map(&StoredEffect::test("gegl:apply-lens", arguments), &context).unwrap().unwrap();
            let Some(NativeStructure::Composite { branches }) = mapped.structure else { panic!("native lens branches") };
            assert_eq!(branches[1].filters[0].params["amount"], 50.0);
            let mut native = Document::new("Native ellipse lens", Size::new(24, 16), ColorMode::Rgb, SampleType::F32);
            native.icc_profile = context.target_profile.clone();
            for branch in branches {
                let mut layer = if let Some(fill) = branch.fill {
                    Layer::new("Editable background", LayerContent::Fill(fill))
                } else {
                    let surface = if branch.filters.is_empty() {
                        original.clone()
                    } else {
                        photocraft_algo::apply_in_with(
                            &original,
                            &photocraft_algo::FilterParams::Spherize { amount: 50.0, mode: photocraft_algo::SpherizeMode::Normal },
                            bounds,
                            bounds,
                            None,
                            bounds,
                            &Interrupt::NONE,
                        )
                        .unwrap()
                    };
                    Layer::new("Editable lens input", LayerContent::Raster(surface))
                };
                layer.vector_mask = branch.vector_mask;
                native.layers.push(layer);
            }
            let rendered = photocraft_compose::render(&native, bounds);
            let outside = rendered.px[0];
            if keep {
                assert_eq!(outside.as_slice(), original.pixel(0, 0).as_slice());
            } else if !paint {
                assert_eq!(outside, [0.0; 4]);
            } else {
                // Independent standard sRGB transfer counterpart in actual
                // Linear-sRGB document paint, including typed source alpha.
                let expected = [128.0 / 255.0, 64.0 / 255.0, 32.0 / 255.0].map(photocraft_color::convert::srgb_to_linear);
                for (actual, expected) in outside[..3].iter().zip(expected) {
                    assert!((*actual - expected).abs() < 0.0001);
                }
                assert!((outside[3] - 128.0 / 255.0).abs() < 0.0001);
            }
            // Whole top/bottom row corners are outside the native ellipse;
            // this checks both ends rather than a single crop's alpha.
            for index in [0, 23, 360, 383] {
                assert_eq!(rendered.px[index][3], outside[3]);
            }
            assert!(rendered.px[8 * 24 + 12][3] > 0.24);
            let bytes = photocraft_format::save_to_bytes(&native, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
            assert_eq!(photocraft_compose::render(&reopened, bounds).px, rendered.px);
            assert!(reopened.layers[1].vector_mask.is_some());
        }
        context.remaining_metadata_bytes = 0;
        assert!(map(&StoredEffect::test("gegl:apply-lens", vec![]), &context).unwrap().is_err());
    }
    #[test]
    fn standard_bayer_pattern_is_native_color_data_and_survives_native_reopen() {
        let converted = map(&StoredEffect::test("gegl:bayer-matrix", vec![]), &ctx()).unwrap().unwrap();
        let Some(NativeStructure::Replace { layer, patterns }) = converted.structure else { panic!("native replacement") };
        assert!(matches!(layer.content, LayerContent::Fill(Fill::Pattern { .. })));
        assert_eq!((patterns[0].width, patterns[0].height), (2, 2));
        for ((x, y), expected) in [(0, 0), (1, 0), (0, 1), (1, 1)].into_iter().zip([0.01434987, 0.34851024, 0.73884493, 0.11601616]) {
            let p = patterns[0].surface.pixel(x, y);
            for v in p.iter().take(3) {
                assert!((*v - expected).abs() < 0.00001);
            }
            assert_eq!(p[3], 1.);
        }
        let mut doc = photocraft_doc::Document::new("Native pattern", photocraft_geom::Size::new(6, 4), ColorMode::Rgb, SampleType::F32);
        doc.icc_profile = ctx().target_profile;
        doc.layers.push(*layer);
        doc.patterns = patterns;
        let first = photocraft_compose::render(&doc, doc.bounds());
        let saved = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
        assert_eq!(photocraft_compose::render(&reopened, reopened.bounds()).px, first.px);
        assert!(first.px.iter().all(|p| p[3] == 1.));
    }
    #[test]
    fn bayer_reflection_scale_phase_and_quarter_turn_stay_editable() {
        let source = StoredEffect::test(
            "gegl:bayer-matrix",
            vec![
                int("x-scale", 2),
                int("y-scale", 3),
                int("x-offset", -3),
                int("y-offset", 7),
                flag("reflect", true),
                Argument { name: "rotation", value: ArgumentValue::String("90") },
            ],
        );
        let result = map(&source, &ctx()).unwrap().unwrap();
        let Some(NativeStructure::Replace { layer, patterns }) = result.structure else { panic!("replace") };
        assert_eq!((patterns[0].width, patterns[0].height), (4, 6));
        assert!((patterns[0].surface.pixel(0, 0)[0] - 0.34851024).abs() < 0.00001);
        let LayerContent::Fill(Fill::Pattern { angle, phase, .. }) = layer.content else { panic!("fill") };
        assert_eq!((angle, phase), (90., (-3., 7.)));
    }
    #[test]
    fn checker_uses_actual_target_icc_instead_of_relabeling_srgb_colors() {
        let mut context = ctx();
        context.linear = false;
        context.target_profile = Some(photocraft_cms::Builtin::DisplayP3.profile().to_bytes());
        let source = StoredEffect::test(
            "gegl:checkerboard",
            vec![
                int("x", 1),
                int("y", 1),
                Argument {
                    name: "color1",
                    value: ArgumentValue::Color(Some(ColorLiteral { encoding: "R'G'B'A u8", pixels: &[255, 0, 0, 255], profile: &[] })),
                },
            ],
        );
        let converted = map(&source, &context).unwrap().unwrap();
        let Some(NativeStructure::Replace { patterns, .. }) = converted.structure else { panic!("pattern") };
        let red = patterns[0].surface.pixel(0, 0);
        for (v, expected) in red.iter().take(3).zip([0.91749, 0.20029, 0.13856]) {
            assert!((*v - expected).abs() < 0.001, "{red:?}");
        }
        assert_eq!(patterns[0].surface.pixel(1, 0), [1., 1., 1., 1.]);
    }
    #[test]
    fn bevel_uses_actual_native_editable_fields() {
        let source = StoredEffect::test("gegl:bevel", vec![arg("radius", 4.), arg("elevation", 150.), int("depth", 75), arg("azimuth", 120.)]);
        let converted = map(&source, &ctx()).unwrap().unwrap();
        let Effect::BevelEmboss(bevel) = &converted.layer_effects[0] else { panic!("bevel") };
        assert_eq!((bevel.size, bevel.altitude, bevel.depth, bevel.angle), (4., 30., 0.75, 120.));
        assert!(!bevel.up);
        assert_eq!(bevel.technique, BevelTechnique::ChiselHard);
    }
    #[test]
    fn bloom_preserves_original_and_builds_native_highlight_blur_branch() {
        let converted =
            map(&StoredEffect::test("gegl:bloom", vec![arg("threshold", 50.), arg("softness", 20.), arg("radius", 4.), arg("strength", 25.)]), &ctx())
                .unwrap()
                .unwrap();
        let Some(NativeStructure::Composite { branches }) = converted.structure else { panic!("branches") };
        assert_eq!(branches.len(), 2);
        assert!(branches[0].filters.is_empty());
        assert_eq!((branches[0].blend, branches[0].opacity), (BlendMode::Normal, 1.));
        assert_eq!((branches[1].blend, branches[1].opacity), (BlendMode::LinearDodge, 0.25));
        assert_eq!(branches[1].filters[0].command, "image.adjustments.curves");
        assert_eq!(branches[1].filters[1].params["radius"], 4.0);
    }
    #[test]
    fn native_convolution_retains_selected_alpha_weighting_boundaries_and_scalar_gray_policy() {
        for border in 0..=4 {
            let source = StoredEffect::test(
                "gegl:convolution-matrix",
                vec![
                    flag("red", false),
                    flag("blue", false),
                    flag("alpha", true),
                    flag("alpha-weight", true),
                    Argument { name: "border", value: ArgumentValue::Enum(border) },
                ],
            );
            let m = map(&source, &ctx()).unwrap().unwrap();
            assert_eq!(m.filters[0].params["channels"], json!([false, true, false, true]));
            assert_eq!(m.filters[0].params["alphaWeighted"], json!(true));
            assert_eq!(m.filters[0].params["edge"], json!(["transparent", "repeat", "wrap", "black", "white"][border as usize]));
            let mut c = ctx();
            c.mode = ColorMode::Grayscale;
            let m = map(&source, &c).unwrap().unwrap();
            assert_eq!(m.filters[0].params["channels"], json!([true, true]));
        }
        let source = StoredEffect::test(
            "gegl:convolution-matrix",
            vec![arg("c3", 0.), arg("offset", 0.5), flag("red", false), flag("green", false), flag("blue", false)],
        );
        let mut c = ctx();
        c.mode = ColorMode::Grayscale;
        let m = map(&source, &c).unwrap().unwrap();
        assert_eq!(m.filters[0].params["channels"], json!([false, true]));
        assert!(m.filters[0].params["kernel"].as_array().unwrap().iter().all(|v| v == &json!(0.0)));
        assert_eq!(m.filters[0].params["offset"], json!(0.5));
    }

    #[test]
    fn custom_kernel_is_row_major_and_offset_is_in_native_units() {
        let source = StoredEffect::test(
            "gegl:convolution-matrix",
            vec![
                arg("c3", 0.),
                arg("a2", 2.),
                arg("divisor", 2.),
                arg("offset", 0.25),
                flag("normalize", false),
                flag("alpha", false),
                flag("alpha-weight", false),
            ],
        );
        let converted = map(&source, &ctx()).unwrap().unwrap();
        assert_eq!(converted.filters[0].command, "filter.other.convolution");
        let p = &converted.filters[0].params;
        assert_eq!(p["kernel"][5], 2.0);
        assert_eq!(p["kernel"].as_array().unwrap().iter().filter(|v| v.as_f64() != Some(0.)).count(), 1);
        assert_eq!(p["offset"], 0.25);
        assert_eq!(p["scale"], 2.0);
    }
    #[test]
    fn alpha_clip_identity_constant_and_range_are_native_coverage_operations() {
        assert!(map(&StoredEffect::test("gegl:alpha-clip", vec![]), &ctx()).unwrap().unwrap().filters.is_empty());
        let clipped = map(&StoredEffect::test("gegl:alpha-clip", vec![arg("low-limit", 0.25)]), &ctx()).unwrap().unwrap();
        assert_eq!(clipped.filters[0].command, "filter.other.clampAlpha");
        assert_eq!(clipped.filters[0].params, json!({"low":0.25,"high":1.}));
        let disabled = map(&StoredEffect::test("gegl:alpha-clip", vec![arg("low-limit", 0.75), flag("clip-low", false), arg("high-limit", 0.5)]), &ctx())
            .unwrap()
            .unwrap();
        assert_eq!(disabled.filters[0].params, json!({"low":0.,"high":0.5}));
        assert!(map(&StoredEffect::test("gegl:alpha-clip", vec![arg("low-limit", 0.75), arg("high-limit", 0.25)]), &ctx()).unwrap().is_err());
        let constant = map(&StoredEffect::test("gegl:alpha-clip", vec![arg("low-limit", 0.25), arg("high-limit", 0.25)]), &ctx()).unwrap().unwrap();
        assert_eq!(constant.filters[0].command, "filter.other.setAlpha");
        assert_eq!(constant.filters[0].params, json!({"value":0.25}));
        assert_eq!(map(&StoredEffect::test("gegl:color-enhance", vec![]), &ctx()).unwrap().unwrap().filters[0].command, "image.autoTone");
    }
    #[test]
    fn hostile_pattern_sizes_and_nonfinite_controls_fail_before_allocation() {
        for source in [
            StoredEffect::test("gegl:bayer-matrix", vec![int("subdivisions", 15)]),
            StoredEffect::test("gegl:checkerboard", vec![int("x", i32::MAX)]),
            StoredEffect::test("gegl:bayer-matrix", vec![arg("amplitude", f32::NAN)]),
            StoredEffect::test("gegl:convolution-matrix", vec![arg("a1", f32::INFINITY)]),
            StoredEffect::test("gegl:bevel", vec![arg("elevation", 181.)]),
        ] {
            assert!(map(&source, &ctx()).unwrap().is_err());
        }
    }
    #[test]
    fn generated_pattern_checks_padded_tiles_and_live_remaining_budget_before_sampling() {
        let sampled = std::cell::Cell::new(false);
        // Even a single pixel requires a full native tile, unlike its tiny
        // logical area. Include both concurrently live CMS row buffers.
        let edge = photocraft_geom::TILE_SIZE as usize;
        let peak = edge * edge * 16 + 8 * std::mem::size_of::<f32>();
        let mut context = ctx();
        context.remaining_generation_bytes = peak - 1;
        let result = pattern_result(
            "Budget probe",
            Size::new(1, 1),
            (0., 0.),
            0.,
            &context,
            |_, _| {
                sampled.set(true);
                Ok([1., 0., 0., 1.])
            },
            "budget regression",
        );
        assert!(result.unwrap_err().message.contains("remaining generation allowance"));
        assert!(!sampled.get());
        // A maximum-paint-area Bayer pattern used to allocate several full-image
        // buffers before global accounting. This stays a preflight-only test.
        context.remaining_generation_bytes = crate::xcf::MAX_PIXEL_BYTES;
        let result = pattern_result(
            "Large preflight",
            Size::new(8192, 8192),
            (0., 0.),
            0.,
            &context,
            |_, _| {
                sampled.set(true);
                Ok([1.; 4])
            },
            "large allocation regression",
        );
        assert!(result.is_err());
        assert!(!sampled.get());
        // An exactly sufficient peak budget succeeds with only one tile and rows.
        context.remaining_generation_bytes = peak;
        assert!(pattern_result("Exact peak", Size::new(1, 1), (0., 0.), 0., &context, |_, _| Ok([1.; 4]), "exact budget").is_ok());
        context.mode = ColorMode::Grayscale;
        context.target_profile = Some(photocraft_cms::builtin::linear_gray().to_bytes());
        context.remaining_generation_bytes = edge * edge * 8 + 6 * std::mem::size_of::<f32>();
        let gray = pattern_result("Exact Gray peak", Size::new(1, 1), (0., 0.), 0., &context, |_, _| Ok([0.25, 0.5, 0.75, 0.5]), "Gray budget").unwrap();
        let Some(NativeStructure::Replace { patterns, .. }) = gray.structure else { panic!("native Gray pattern") };
        assert_eq!(patterns[0].surface.format().mode, ColorMode::Grayscale);
        assert_eq!(patterns[0].surface.channels(), 2);
        assert_eq!(patterns[0].surface.read_region(Rect::new(0, 0, 1, 1))[1], 0.5);
    }
    #[test]
    fn generated_pattern_honors_preexisting_and_mid_row_cancellation() {
        use photocraft_raster::Interrupt;
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        let flag = AtomicBool::new(true);
        let sampled = AtomicUsize::new(0);
        let cancel = || flag.load(Ordering::Relaxed);
        let context = MappingContext { interrupt: Interrupt::cancel_only(&cancel), ..ctx() };
        let result = pattern_result(
            "Cancelled",
            Size::new(512, 2),
            (0., 0.),
            0.,
            &context,
            |_, _| {
                sampled.fetch_add(1, Ordering::Relaxed);
                Ok([1.; 4])
            },
            "cancel regression",
        );
        assert!(result.unwrap_err().message.contains("cancelled"));
        assert_eq!(sampled.load(Ordering::Relaxed), 0);
        flag.store(false, Ordering::Relaxed);
        let result = pattern_result(
            "Midrow cancellation",
            Size::new(512, 2),
            (0., 0.),
            0.,
            &context,
            |_, _| {
                let count = sampled.fetch_add(1, Ordering::Relaxed) + 1;
                if count == 4 {
                    flag.store(true, Ordering::Relaxed);
                }
                Ok([1.; 4])
            },
            "cancel regression",
        );
        assert!(result.unwrap_err().message.contains("cancelled"));
        assert!((4..=256).contains(&sampled.load(Ordering::Relaxed)));
    }
    #[test]
    fn signed_absolute_is_native_difference_style_with_hdr_and_fractional_coverage() {
        let converted = map(&StoredEffect::test("gegl:absolute", vec![]), &ctx()).unwrap().unwrap();
        let mut surface = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::F32, true));
        surface.write_region(Rect::new(0, 0, 2, 1), &[-2., 0.5, -0.75, 0.25, 4., -8., 16., 1.]);
        let mut layer = Layer::new("Signed native data", LayerContent::Raster(surface));
        layer.effects.items = converted.layer_effects;
        let mut document = photocraft_doc::Document::new("Native absolute", Size::new(2, 1), ColorMode::Rgb, SampleType::F32);
        document.layers.push(layer);
        let before = photocraft_compose::render(&document, document.bounds());
        assert_eq!(before.px, vec![[2., 0.5, 0.75, 0.25], [4., 8., 16., 1.]]);
        let saved = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
        assert_eq!(photocraft_compose::render(&reopened, reopened.bounds()).px, before.px);
        let alias = StoredEffect::test("gegl:abs", vec![]);
        let mapping = map(&alias, &ctx()).unwrap().unwrap();
        assert!(matches!(mapping.layer_effects[0], Effect::ColorOverlay { .. }));
        assert_eq!(alias.operation, "gegl:abs");
    }
    #[test]
    fn typed_color_to_alpha_uses_native_icc_key_and_actual_native_unmix_coverage() {
        let source = StoredEffect::test(
            "gegl:color-to-alpha",
            vec![Argument {
                name: "color",
                value: ArgumentValue::Color(Some(ColorLiteral { encoding: "R'G'B'A u8", pixels: &[128, 64, 32, 64], profile: &[] })),
            }],
        );
        let converted = map(&source, &ctx()).unwrap().unwrap();
        let filter = &converted.filters[0];
        assert_eq!(filter.command, "filter.other.colorToAlpha");
        let array = filter.params["color"].as_array().unwrap();
        let key: [f32; 4] = std::array::from_fn(|i| array[i].as_f64().unwrap() as f32);
        for (actual, expected) in key.into_iter().zip([0.2158605, 0.05126946, 0.01444384, 1.]) {
            assert!((actual - expected).abs() < 0.00001);
        }
        let bounds = Rect::new(0, 0, 2, 1);
        let mut surface = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::F32, true));
        surface.write_region(bounds, &[key[0], key[1], key[2], 0.75, (1. + key[0]) * 0.5, key[1] * 0.5, key[2] * 0.5, 0.8]);
        let params = photocraft_algo::FilterParams::ColorToAlpha { color: key, transparency_threshold: 0., opacity_threshold: 1. };
        let result = photocraft_algo::apply_in_with(&surface, &params, bounds, bounds, None, bounds, &photocraft_raster::Interrupt::NONE).unwrap();
        assert_eq!(result.pixel(0, 0)[3], 0.);
        let foreground = result.pixel(1, 0);
        for (actual, expected) in foreground.into_iter().zip([1., 0., 0., 0.4]) {
            assert!((actual - expected).abs() < 0.00001);
        }
        let mut document = photocraft_doc::Document::new("Native unmix", Size::new(2, 1), ColorMode::Rgb, SampleType::F32);
        document.layers.push(Layer::new("Native output", LayerContent::Raster(result)));
        let before = photocraft_compose::render(&document, bounds);
        let saved = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
        assert_eq!(photocraft_compose::render(&reopened, bounds).px, before.px);
        assert!(map(&StoredEffect::test("gegl:color-to-alpha", vec![arg("transparency-threshold", -0.01)]), &ctx()).unwrap().is_err());
    }
    #[test]
    fn cellular_pattern_translates_rank_norm_octaves_palette_seed_and_native_reopen() {
        let converted = map(&StoredEffect::test("gegl:cell-noise", vec![]), &ctx()).unwrap().unwrap();
        let Some(NativeStructure::Replace { layer, patterns }) = converted.structure else { panic!("native cellular pattern") };
        let first = patterns[0].surface.pixel(0, 0);
        assert_eq!(first[3], 1.);
        assert!((0.0..=1.0).contains(&first[0]));
        assert!((first[0] - first[1]).abs() < 0.00001 && (first[1] - first[2]).abs() < 0.00001);
        let baseline = patterns[0].id.clone();
        for arguments in [
            vec![arg("scale", 2.)],
            vec![arg("shape", 1.)],
            vec![int("rank", 3)],
            vec![int("iterations", 2)],
            vec![flag("palettize", true)],
            vec![Argument { name: "seed", value: ArgumentValue::UInt(1) }],
        ] {
            let changed = map(&StoredEffect::test("gegl:cell-noise", arguments), &ctx()).unwrap().unwrap();
            let Some(NativeStructure::Replace { patterns, .. }) = changed.structure else { panic!("pattern") };
            assert_ne!(patterns[0].id, baseline);
        }
        let mut document = photocraft_doc::Document::new("Native cellular", Size::new(9, 5), ColorMode::Rgb, SampleType::F32);
        document.icc_profile = ctx().target_profile;
        document.patterns = patterns;
        document.layers.push(*layer);
        let before = photocraft_compose::render(&document, document.bounds());
        let bytes = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
        assert_eq!(photocraft_compose::render(&reopened, reopened.bounds()).px, before.px);
        let tiny = MappingContext { remaining_generation_bytes: 1, ..ctx() };
        assert!(map(&StoredEffect::test("gegl:cell-noise", vec![]), &tiny).unwrap().is_err());
        let oversized = MappingContext { native_filter_bounds: Rect::new(0, 0, 8192, 8192), ..ctx() };
        assert!(map(&StoredEffect::test("gegl:cell-noise", vec![int("iterations", 20)]), &oversized).unwrap().is_err());
    }
    #[test]
    fn assimilation_is_editable_gray_and_clipped_color_grid_with_native_hdr_and_alpha() {
        use photocraft_doc::adjust::Adjustment;
        let bounds = Rect::new(0, 0, 16, 16);
        let context = MappingContext { native_filter_bounds: bounds, ..ctx() };
        let converted =
            map(&StoredEffect::test("gegl:color-assimilation-grid", vec![arg("grid-size", 8.), arg("angle", 0.), arg("line-thickness", 0.25)]), &context)
                .unwrap()
                .unwrap();
        let Some(NativeStructure::Composite { branches }) = converted.structure else { panic!("native branches") };
        assert_eq!(branches.len(), 2);
        assert!(branches[1].clipped);
        assert!(branches[1].vector_mask.is_some());
        let mut document = photocraft_doc::Document::new("Native assimilation", Size::new(16, 16), ColorMode::Rgb, SampleType::F32);
        document.icc_profile = context.target_profile.clone();
        for branch in branches {
            let mut buffer = photocraft_compose::Buffer::filled(bounds, [1., 0., 0., 0.5]);
            for filter in branch.filters {
                assert_eq!(filter.command, "image.adjustments.channelMixer");
                let matrix = std::array::from_fn(|row| {
                    std::array::from_fn(|column| filter.params[["red", "green", "blue"][row]][column].as_f64().unwrap() as f32 / 100.)
                });
                photocraft_compose::adjust::apply_depth(
                    &Adjustment::ChannelMixer { matrix, monochrome: false },
                    &mut buffer,
                    photocraft_compose::adjust::Transfer::Gamma(1.),
                    Some(SampleType::F32),
                );
            }
            let mut surface = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::F32, true));
            surface.write_region(bounds, &buffer.px.into_iter().flatten().collect::<Vec<_>>());
            let mut layer = Layer::new("Native grid branch", LayerContent::Raster(surface));
            layer.vector_mask = branch.vector_mask;
            layer.clipped = branch.clipped;
            document.layers.push(layer);
        }
        let before = photocraft_compose::render(&document, bounds);
        assert!(before.px.iter().all(|p| (p[3] - 0.5).abs() < 0.00001));
        let gray = before.get(4, 4);
        assert!((gray[0] - gray[1]).abs() < 0.001 && (gray[1] - gray[2]).abs() < 0.001);
        let color = before.get(0, 0);
        assert!(color[0] > gray[0] + 0.1 && color[1] < gray[1] - 0.05);
        let bytes = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
        assert_eq!(photocraft_compose::render(&reopened, bounds).px, before.px);
        assert_eq!(
            map(&StoredEffect::test("gegl:color-assimilation-grid", vec![arg("grid-size", 0.)]), &ctx()).unwrap().unwrap().filters[0].command,
            "image.adjustments.channelMixer"
        );
        for arguments in [vec![arg("grid-size", f32::from_bits(1))], vec![arg("grid-size", 0.0001)], vec![arg("saturation", f32::NAN)]] {
            assert!(map(&StoredEffect::test("gegl:color-assimilation-grid", arguments), &context).unwrap().is_err());
        }
    }
}
