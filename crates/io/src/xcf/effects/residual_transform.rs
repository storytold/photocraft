//! Frozen B06-07/08: native transforms, image adjustments and editable composites.
//! Approximation policies are explicit; no source graph or expression is executable.
use super::{ArgumentValue, MappingContext, MappingFailure, MappingResult, NativeBranch, NativeMapping, NativeStructure, StoredEffect, native_filter};
use photocraft_algo::transform::Homography;
use photocraft_color::BlendMode;
use photocraft_doc::effects::{Effect, FxCommon};
use photocraft_geom::{Affine, Rect};
use serde_json::{Value, json};
type Result<T> = std::result::Result<T, MappingFailure>;

fn number(e: &StoredEffect<'_>, key: &str, default: f64, low: f64, high: f64) -> Result<f64> {
    let value = e.number(key)?.unwrap_or(default);
    if !value.is_finite() || value < f64::from(low as f32) || value > f64::from(high as f32) {
        return Err(format!("{} {key} outside bounded conversion range", e.operation).into());
    }
    Ok(value)
}
fn integer(e: &StoredEffect<'_>, key: &str, default: u32, low: u32, high: u32) -> Result<u32> {
    let value = number(e, key, f64::from(default), f64::from(low), f64::from(high))?;
    if value.fract() != 0.0 || value > f64::from(high) {
        return Err(format!("{key} must be a bounded integer").into());
    }
    Ok(value as u32)
}
fn text<'a>(e: &'a StoredEffect<'a>, key: &str, default: &'a str) -> Result<&'a str> {
    match e.argument(key)? {
        None => Ok(default),
        Some(ArgumentValue::String(value)) => Ok(value),
        _ => Err(format!("{key} must be text").into()),
    }
}
fn fields(e: &StoredEffect<'_>, keys: &[&str]) -> Result<()> {
    for argument in &e.arguments {
        if !keys.contains(&argument.name) {
            return Err(format!("unmapped {} field {}", e.operation, argument.name).into());
        }
    }
    Ok(())
}
fn mapped(command: &str, params: Value, note: &str) -> NativeMapping {
    NativeMapping { filters: vec![native_filter(command, params)], notes: vec![note.into()], ..Default::default() }
}
pub(super) fn map(e: &StoredEffect<'_>, context: &MappingContext) -> MappingResult {
    Some(match e.operation {
        "gegl:photocopy" => photocopy(e),
        "gegl:pixelize" => pixelize(e, context),
        "gegl:recursive-transform" => recursive(e, context),
        "gegl:rgb-clip" => clip(e, context),
        "gegl:ripple" => ripple(e, context),
        "gegl:slic" => slic(e, context),
        "gegl:softglow" => softglow(e),
        "gegl:spherize" => spherize(e),
        "gegl:panorama-projection" => panorama(e, context),
        "gegl:perlin-noise" | "gegl:plasma" | "gegl:simplex-noise" => noise(e, context),
        "gegl:red-eye-removal" => red_eye(e),
        "gegl:reinhard05" => reinhard(e),
        "gegl:shift" => shift(e, context),
        "gegl:sinus" => sinus(e, context),
        _ => return None,
    })
}
fn photocopy(e: &StoredEffect<'_>) -> Result<NativeMapping> {
    fields(e, &["mask-radius", "sharpness", "black", "white"])?;
    let radius = number(e, "mask-radius", 10., 0., 50.)?;
    let sharpness = number(e, "sharpness", 0.5, 0., 1.)?;
    let black = number(e, "black", 0.2, 0., 1.)?;
    let white = number(e, "white", 0.2, 0., 1.)?;
    let mut filters = Vec::new();
    if radius > 0. {
        filters.push(native_filter("filter.blur.gaussianBlur", json!({"radius":radius.max(0.1)})));
    }
    filters.push(native_filter("filter.stylize.findEdges", json!({})));
    if sharpness > 0. {
        filters.push(native_filter("filter.sharpen.unsharpMask", json!({"amount":sharpness*200.,"radius":1.,"threshold":0.})));
    }
    // Independent bounded black/white point controls cannot cross at their extremes.
    filters.push(native_filter("image.adjustments.levels", json!({"inBlack":black*126.,"inWhite":255.-white*127.,"gamma":1.,"outBlack":0.,"outWhite":255.})));
    Ok(NativeMapping { filters, notes: vec!["Native Gaussian→Find Edges→Unsharp Mask→Levels approximates photocopy structure. Radius and sharpness stay editable; black/white become bounded native input-point proportions, not source image-percentile thresholds. Native kernels, color and alpha rules apply".into()], ..Default::default() })
}
fn pixelize(e: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping> {
    fields(e, &["norm", "size-x", "size-y", "offset-x", "offset-y", "ratio-x", "ratio-y", "background"])?;
    let norm = e.enum_value("norm")?.unwrap_or(2);
    if ![0, 1, 2].contains(&norm) {
        return Err("unknown pixelize norm".into());
    }
    let x = integer(e, "size-x", 16, 1, i32::MAX as u32)?;
    let y = integer(e, "size-y", 16, 1, i32::MAX as u32)?;
    let offset_x = i64::from(super::residual_spatial::integer(e, "offset-x", 0, i32::MIN, i32::MAX)?);
    let offset_y = i64::from(super::residual_spatial::integer(e, "offset-y", 0, i32::MIN, i32::MAX)?);
    let rx = number(e, "ratio-x", 1., 0., 1.)?;
    let ry = number(e, "ratio-y", 1., 0., 1.)?;
    if x == 1 && y == 1 && norm == 2 && rx == 1. && ry == 1. {
        return Ok(NativeMapping { notes: vec!["Full-coverage one-pixel square cells are native identity".into()], ..Default::default() });
    }
    let complete_square = norm == 2 && rx == 1. && ry == 1.;
    if complete_square && x == y && (2..=200).contains(&x) && offset_x.rem_euclid(i64::from(x)) == 0 && offset_y.rem_euclid(i64::from(y)) == 0 {
        return Ok(mapped(
            "filter.pixelate.mosaic",
            json!({"cellSize":x}),
            "Full equal square cells at the native origin use PhotoCraft's existing editable Mosaic command directly. Native Mosaic sampling, color/HDR and alpha rules are authoritative. Source background is geometrically unused and remains inert historical data",
        ));
    }
    let background = if complete_square {
        // Every position belongs to a cell: source background is inert and
        // need not acquire an invented interpretation of raw color bytes.
        super::resources::native_rgba([0.; 4], context).map_err(MappingFailure::from)?
    } else {
        super::residual_spatial::color(e, "background", [1., 1., 1., 1.], context)?
    };
    native_pixel_cells((x, y), (offset_x, offset_y), norm, (rx, ry), background, context)
}
fn native_pixel_cells(
    size: (u32, u32),
    offset: (i64, i64),
    norm: i32,
    ratio: (f64, f64),
    background: photocraft_color::Color,
    context: &MappingContext,
) -> Result<NativeMapping> {
    use super::residual_spatial::{bounds, generation_bytes, generation_check, geometry_bytes, rect};
    use photocraft_doc::{Fill, Path, PathOp, Subpath, VectorMask};
    let (x, y) = size;
    let (offset_x, offset_y) = offset;
    let (rx, ry) = ratio;
    let b = bounds(context)?;
    let sx = i64::from(x);
    let sy = i64::from(y);
    let first_x = offset_x + (i64::from(b.x0) - offset_x).div_euclid(sx) * sx;
    let first_y = offset_y + (i64::from(b.y0) - offset_y).div_euclid(sy) * sy;
    let nx = usize::try_from((i64::from(b.x1) - first_x + sx - 1) / sx).map_err(|_| MappingFailure::resource("native pixel grid width"))?;
    let ny = usize::try_from((i64::from(b.y1) - first_y + sy - 1) / sy).map_err(|_| MappingFailure::resource("native pixel grid height"))?;
    let count = nx.checked_mul(ny).ok_or_else(|| MappingFailure::resource("native pixel cell count overflow"))?;
    if count > 63 {
        return Err(MappingFailure::resource("native editable pixel grid exceeds63 cells plus background"));
    }
    let metadata = (count + 1) * 8192;
    geometry_bytes(context, count, count * 4)?;
    generation_bytes(context, metadata)?;
    if metadata > context.remaining_metadata_bytes {
        return Err(MappingFailure::resource("native pixel cells exceed remaining metadata allowance"));
    }
    let mut branches = Vec::new();
    branches.try_reserve_exact(count + 1).map_err(|_| MappingFailure::resource("native pixel cell branches unavailable"))?;
    branches.push(NativeBranch { fill: Some(Fill::Solid(background)), ..Default::default() });
    let mut uncovered = vec![rect(f64::from(b.x0), f64::from(b.y0), f64::from(b.width()), f64::from(b.height()))];
    let mut storage = metadata;
    if rx > 0. && ry > 0. {
        for row in 0..ny {
            for column in 0..nx {
                generation_check(context)?;
                let left = first_x + column as i64 * sx;
                let top = first_y + row as i64 * sy;
                let right = (left + sx).min(i64::from(b.x1));
                let bottom = (top + sy).min(i64::from(b.y1));
                let frame = Rect::new(left.max(i64::from(b.x0)) as i32, top.max(i64::from(b.y0)) as i32, right as i32, bottom as i32);
                let tile = photocraft_geom::TILE_SIZE as usize;
                let bytes = (frame.width() as usize)
                    .div_ceil(tile)
                    .checked_mul((frame.height() as usize).div_ceil(tile))
                    .and_then(|n| n.checked_mul(tile * tile * (context.mode.color_channels() + 1) * 4 + 128))
                    .and_then(|n| n.checked_mul(3))
                    .and_then(|n| n.checked_add(65536))
                    .ok_or_else(|| MappingFailure::resource("native pixel frame storage overflows"))?;
                storage = storage.checked_add(bytes).ok_or_else(|| MappingFailure::resource("native pixel grid storage overflows"))?;
                generation_bytes(context, storage)?;
                let cx = left as f64 + f64::from(x) * 0.5;
                let cy = top as f64 + f64::from(y) * 0.5;
                let half_x = f64::from(x) * rx * 0.5;
                let half_y = f64::from(y) * ry * 0.5;
                let path = match norm {
                    0 => Path::new(vec![Subpath::polygon(&[(cx, cy - half_y), (cx + half_x, cy), (cx, cy + half_y), (cx - half_x, cy)])]),
                    1 => photocraft_vector::shapes::ellipse(cx - half_x, cy - half_y, half_x * 2., half_y * 2.),
                    _ => Path::new(vec![rect(cx - half_x, cy - half_y, half_x * 2., half_y * 2.)]),
                };
                uncovered.extend(path.subpaths.iter().cloned().map(|p| p.with_op(PathOp::Subtract)));
                branches.push(NativeBranch {
                    filter_frame: Some(frame),
                    filters: vec![native_filter("filter.blur.average", json!({}))],
                    vector_mask: Some(VectorMask::new(path)),
                    ..Default::default()
                });
            }
        }
    }
    if let Some(background) = branches.first_mut() {
        background.vector_mask = Some(VectorMask::new(Path::new(uncovered)));
    }
    Ok(NativeMapping {
        structure: Some(NativeStructure::Composite { branches }),
        notes: vec!["Source rectangular grid sizes and signed offsets become editable framed native Average cells, with native diamond/ellipse/rectangle vector masks sized by the two coverage ratios and actual CMS-converted source background paint confined to uncovered geometry. Cell alpha is preserved rather than composited over an opaque underlay. Full stored cell sizes are accepted within bounded cell/cache work; partial edge cells average their visible native frame. Native alpha-weighted means and vector antialiasing replace source premultiplied accumulation and hard-edge coverage rules. No source pixel evaluator persists".into()],
        ..Default::default()
    })
}
fn clip(e: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping> {
    use photocraft_color::{BlendMode, Color, ColorMode};
    fields(e, &["clip-low", "low-limit", "clip-high", "high-limit"])?;
    let low = number(e, "low-limit", 0., -f64::MAX, f64::MAX)?;
    let high = number(e, "high-limit", 1., -f64::MAX, f64::MAX)?;
    let clip_low = e.boolean("clip-low")?.unwrap_or(true);
    let clip_high = e.boolean("clip-high")?.unwrap_or(true);
    if !clip_low && !clip_high {
        return Ok(NativeMapping::default());
    }
    // These are scalar component bounds, not serialized sRGB paint. Store them
    // in the document's actual encoded model, just like native channel values;
    // an sRGB-to-document color conversion would change a linear/HDR threshold.
    let paint = |value: f64| -> Result<Color> {
        let value = value as f32;
        if !value.is_finite() {
            return Err("Native clip bound exceeds finite F32 color storage".into());
        }
        match context.mode {
            ColorMode::Rgb => Ok(Color::rgb(value, value, value)),
            ColorMode::Grayscale => Ok(Color::gray(value)),
            _ => Err("Native scalar RGB clip requires an RGB or Gray owner".into()),
        }
    };
    let mut branches = vec![NativeBranch::default()];
    for (enabled, value, blend) in [(clip_low, low, BlendMode::Lighten), (clip_high, high, BlendMode::Darken)] {
        if enabled {
            branches.push(NativeBranch { fill: Some(photocraft_doc::Fill::Solid(paint(value)?)), blend, clipped: true, ..Default::default() });
        }
    }
    let mut notes = vec![
        "Scalar component bounds become ordinary editable native Solid Fill clipping layers: lower Lighten, then upper Darken, above the original owner. Bounds use actual native document-encoded RGB/Gray channel values and its persisted ICC, not an invented sRGB paint conversion. Native F32 blending supports signed/HDR bounds and preserves the base alpha; zero-coverage hidden color follows native transparency rules. Native depth quantization, color encoding, masks and compositing remain authoritative".into(),
    ];
    if clip_low && clip_high && low > high {
        notes.push("Crossed source bounds use the explicit native lower-then-upper ordering, collapsing covered channels to the upper bound. Stock source conditional behavior differs; original limits remain retained. No invalid reversed-range clamp is evaluated".into());
    }
    Ok(NativeMapping { structure: Some(NativeStructure::Composite { branches }), notes, ..Default::default() })
}

fn ripple(e: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping> {
    fields(e, &["amplitude", "period", "phi", "angle", "sampler-type", "wave-type", "abyss-policy", "tileable"])?;
    let amplitude = number(e, "amplitude", 25., 0., 1000.)?;
    let period = number(e, "period", 200., 0., 1000.)?;
    let phase = number(e, "phi", 0., -1., 1.)?;
    let angle = number(e, "angle", 0., -180., 180.)?;
    let sampler = text(e, "sampler-type", "cubic")?;
    let wave = text(e, "wave-type", "sine")?;
    let abyss = text(e, "abyss-policy", "none")?;
    let tileable = e.boolean("tileable")?.unwrap_or(false);
    if !["sine", "triangle", "sawtooth"].contains(&wave)
        || !["nearest", "linear", "cubic", "nohalo", "lohalo"].contains(&sampler)
        || !["none", "clamp", "loop"].contains(&abyss)
    {
        return Err("Unknown ripple wave/sampler/abyss policy retained".into());
    }
    if amplitude == 0. {
        return Ok(NativeMapping::default());
    }
    let bounds = context.native_filter_bounds;
    if bounds.is_empty() {
        return Err("Native ripple requires nonempty bounds".into());
    }
    // Standard plane waves: phase advances along the wave normal and the
    // displacement is perpendicular. Only the finite native map is persisted.
    let period = if period < 0.0001 { 1.0 } else { period };
    let radians = angle.to_radians();
    let mut fx = radians.cos() / period;
    let mut fy = -radians.sin() / period;
    if tileable {
        let mut nx = (fx * f64::from(bounds.width())).round();
        let mut ny = (fy * f64::from(bounds.height())).round();
        if nx == 0. && ny == 0. {
            if fx.abs() >= fy.abs() {
                nx = if fx < 0. { -1. } else { 1. };
            } else {
                ny = if fy < 0. { -1. } else { 1. };
            }
        }
        fx = nx / f64::from(bounds.width());
        fy = ny / f64::from(bounds.height());
    }
    let length = fx.hypot(fy);
    let direction = (-fy / length, fx / length);
    let coordinate = |x: f64, y: f64| {
        let cycles = x * fx + y * fy;
        let value = match wave {
            "triangle" => (std::f64::consts::TAU * (cycles - phase)).sin().asin() * 2. / std::f64::consts::PI,
            "sawtooth" => 2. * (cycles - phase + 0.5).rem_euclid(1.) - 1.,
            _ => (std::f64::consts::TAU * (cycles + phase)).sin(),
        };
        (x + direction.0 * amplitude * value, y + direction.1 * amplitude * value)
    };
    let mask_bytes = if abyss == "none" { coordinate_mask_work_bytes(bounds)? } else { 0 };
    let reduced = MappingContext {
        remaining_generation_bytes: context
            .remaining_generation_bytes
            .checked_sub(mask_bytes)
            .ok_or_else(|| MappingFailure::resource("native ripple mask/map workspace"))?,
        remaining_metadata_bytes: context
            .remaining_metadata_bytes
            .checked_sub(mask_bytes)
            .ok_or_else(|| MappingFailure::resource("native ripple mask/map metadata"))?,
        mode: context.mode,
        linear: context.linear,
        owner_bounds: context.owner_bounds,
        native_filter_bounds: bounds,
        target_profile: context.target_profile.clone(),
        resolution_dpi: 72.0,
        interrupt: context.interrupt,
    };
    let mut mapped = coordinate_map(
        &reduced,
        |x, y| coordinate(f64::from(x) + 0.5, f64::from(y) + 0.5),
        &format!(
            "Source amplitude/period, phase cycles, counterclockwise wave angle and sine/triangle/sawtooth geometry author one editable ordinary native Displace map. Native U8 quantization and bilinear reconstruction replace requested {sampler}; source period below.0001 uses a finite one-pixel native phase period. Tileable={tileable} snaps normal frequency to whole canvas cycles under an explicit native periodic policy; source tiling heuristics may differ. Original source waveform parameters do not survive as executable behavior"
        ),
    )?;
    if let Some(filter) = mapped.filters.first_mut() {
        filter.params["undefinedAreas"] = json!(if abyss == "clamp" { "repeat" } else { "wrap" });
    }
    if abyss == "none" {
        let mask = coordinate_mask(context, bounds, |u, v| {
            let (x, y) = coordinate(u * f64::from(bounds.width()), v * f64::from(bounds.height()));
            x >= 0. && y >= 0. && x < f64::from(bounds.width()) && y < f64::from(bounds.height())
        })?;
        mapped.structure = Some(NativeStructure::Composite {
            branches: vec![NativeBranch { filters: std::mem::take(&mut mapped.filters), mask: Some(mask), ..Default::default() }],
        });
        mapped.notes.push(
            "Transparent source abyss becomes an editable native destination validity mask; native hard coverage and subsequent native compositing apply"
                .into(),
        );
    }
    Ok(mapped)
}
fn slic(e: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping> {
    fields(e, &["cluster-size", "compactness", "iterations"])?;
    let size = integer(e, "cluster-size", 32, 2, i32::MAX as u32)?;
    let compactness = integer(e, "compactness", 20, 1, 40)?;
    let iterations = integer(e, "iterations", 1, 1, 30)?;
    let irregular = (3..=300).contains(&size);
    let regularity = f64::from(compactness - 1) / 39.;
    if irregular && regularity == 0. {
        return Ok(mapped(
            "filter.pixelate.crystallize",
            json!({"cellSize":size,"seed":0}),
            &format!(
                "Native deterministic Crystallize approximates minimum-compactness regional cells. Fixed native seed0 is an import policy, not a decoded source seed. Source iterations{iterations} are superseded by the native deterministic partition rather than becoming fake random or strength controls; native cell/color/alpha rules apply"
            ),
        ));
    }
    let transparent = super::resources::native_rgba([0.; 4], context).map_err(MappingFailure::from)?;
    let mut regular = match native_pixel_cells((size, size), (0, 0), 2, (1., 1.), transparent, context) {
        Ok(mapping) => mapping,
        Err(error)
            if irregular
                && !context.interrupt.cancelled()
                && matches!(
                    error.reason,
                    photocraft_doc::import_conversion::ConversionReason::RetentionLimit
                        | photocraft_doc::import_conversion::ConversionReason::ResourceUnavailable
                ) =>
        {
            return Ok(mapped(
                "filter.pixelate.crystallize",
                json!({"cellSize":size,"seed":0}),
                &format!(
                    "Native deterministic Crystallize preserves regional cell size. Requested compactness regular-grid candidate exceeded actual bounded native cell/cache work: {}; compactness{compactness} remains historical and iterations{iterations} are superseded by deterministic native partition convergence. Fixed native seed0 is an import policy, not a decoded source seed",
                    error.message
                ),
            ));
        }
        Err(error) => return Err(error),
    };
    let Some(NativeStructure::Composite { branches }) = &mut regular.structure else {
        return Err("Native regular cell structure unavailable".into());
    };
    if irregular {
        if let Some(first) = branches.first_mut() {
            *first = NativeBranch { filters: vec![native_filter("filter.pixelate.crystallize", json!({"cellSize":size,"seed":0}))], ..Default::default() };
        }
        for branch in branches.iter_mut().skip(1) {
            branch.opacity = regularity as f32;
        }
    }
    regular.notes = vec![format!(
        "Source cluster side{size} becomes editable native regional cell geometry. Compactness{compactness} maps linearly from1..40 to irregular native Crystallize versus regular square framed native Average composition (regular fraction{regularity}); native source-over/alpha rules apply rather than source k-means distances. Fixed native seed0 is a reproducibility policy, not a decoded source seed. Source iterations{iterations} are superseded by deterministic native convergence, not fake perturbations. For cluster sides outside native Crystallize3..300, explicit regular native cell geometry supersedes irregular segmentation and compactness; actual cell/cache budgets bound work without a dialog-size clamp. Native partitions differ from source content-adaptive SLIC"
    )];
    Ok(regular)
}
fn softglow(e: &StoredEffect<'_>) -> Result<NativeMapping> {
    fields(e, &["glow-radius", "brightness", "sharpness"])?;
    let radius = number(e, "glow-radius", 10., 1., 50.)?;
    let brightness = number(e, "brightness", 0.3, 0., 1.)?;
    let sharpness = number(e, "sharpness", 0.85, 0., 1.)?;
    let curve: Vec<f64> = (0..256).map(|i| (f64::from(i) / 255.).powf(1. + sharpness * 7.) * 255.).collect();
    let filters = vec![
        native_filter("image.adjustments.curves", json!({"pencil":true,"points":curve})),
        native_filter("filter.blur.gaussianBlur", json!({"radius":radius})),
    ];
    Ok(NativeMapping { structure: Some(NativeStructure::Composite { branches: vec![NativeBranch::default(), NativeBranch { filters, blend: BlendMode::Screen, opacity: brightness as f32, ..Default::default() }] }), notes: vec!["Native highlight power Curves→Gaussian branch screens over ordinary original pixels. Source radius becomes native sigma; brightness becomes branch opacity and sharpness a native highlight exponent. This is an editable approximation with native color, kernel and compositing rules".into()], ..Default::default() })
}
fn spherize(e: &StoredEffect<'_>) -> Result<NativeMapping> {
    fields(e, &["mode", "angle-of-view", "curvature", "amount", "sampler-type"])?;
    let mode = e.enum_value("mode")?.unwrap_or(0);
    let native_mode = match mode {
        0 => "normal",
        1 => "horizontalOnly",
        2 => "verticalOnly",
        _ => return Err("Unknown spherize mode".into()),
    };
    let view = number(e, "angle-of-view", 0., 0., 180.)?;
    let curvature = number(e, "curvature", 1., 0., 1.)?;
    let amount = number(e, "amount", 1., -1., 1.)?;
    let sampler = e.enum_value("sampler-type")?.unwrap_or(1);
    if !(0..=4).contains(&sampler) {
        return Err("Unknown spherize sampler".into());
    }
    // A cap's sagitta / radius is 1-cos(apex/2). The source property defines
    // apex as curvature times the co-angle of view; normalize the default
    // hemisphere to native amount100, without introducing another projector.
    let apex = curvature * (180. - view);
    let cap_height = if apex == 180. { 1. } else { 1. - (apex.to_radians() * 0.5).cos() };
    let strength = amount * cap_height * 100.;
    if strength == 0. {
        return Ok(NativeMapping {
            notes: vec![
                "Zero spherical-cap apex (curvature0 or camera view180), or amount0, becomes native identity; no source projection evaluator is retained"
                    .into(),
            ],
            ..Default::default()
        });
    }
    Ok(mapped(
        "filter.distort.spherize",
        json!({"mode":native_mode,"amount":strength}),
        &format!(
            "Source cap apex is curvature{curvature} times co-angle of camera view{view}; normalized cap sagitta (1−cos(apex/2)) scales signed amount{amount} into editable native Spherize strength. Default hemispherical cap keeps native amount100; curvature0/view180 becomes identity. Native Spherize radial/axis geometry and interpolation replace the source camera projection and sampler{sampler}; this geometric strength conversion is an explicit approximation, not another source projector"
        ),
    ))
}

/// A strict numeric matrix only. This does not parse transform expressions or graphs.
fn matrix(raw: &str) -> Result<Homography> {
    if raw.len() > 1024 {
        return Err("Transform matrix text exceeds bounded parser size".into());
    }
    let value = raw.trim().strip_prefix("matrix").ok_or("Only a numeric matrix is supported")?.trim();
    let inner = value.strip_prefix('(').and_then(|s| s.strip_suffix(')')).ok_or("Malformed numeric matrix")?;
    let values = inner
        .split(',')
        .map(|v| v.trim().parse::<f64>().map_err(|_| MappingFailure::from("Invalid numeric matrix coefficient")))
        .collect::<Result<Vec<_>>>()?;
    if values.len() != 9 || values.iter().any(|v| !v.is_finite() || v.abs() > 1_000_000.) {
        return Err("Matrix requires nine bounded finite coefficients".into());
    }
    let coefficients: [f64; 9] = values.as_slice().try_into().map_err(|_| "Malformed numeric matrix")?;
    normalize_matrix(Homography(coefficients))
}
fn normalize_matrix(matrix: Homography) -> Result<Homography> {
    let scale = if matrix.0[8].abs() >= 1e-12 { matrix.0[8] } else { matrix.0.iter().map(|v| v.abs()).fold(0., f64::max) };
    if !scale.is_finite() || scale == 0. {
        return Err("Singular or nonfinite source numeric matrix retained".into());
    }
    let matrix = Homography(matrix.0.map(|v| v / scale));
    if matrix.0.iter().any(|v| !v.is_finite() || v.abs() > 16_777_216.) || matrix.inverse().is_none_or(|inverse| inverse.0.iter().any(|v| !v.is_finite())) {
        return Err("Singular or excessive native projective matrix retained".into());
    }
    Ok(matrix)
}

fn recursive(e: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping> {
    fields(e, &["transform", "first-iteration", "iterations", "fade-color", "fade-opacity", "paste-below", "sampler-type"])?;
    let transform = matrix(text(e, "transform", "matrix (1, 0, 0, 0, 1, 0, 0, 0, 1)")?)?;
    let first = integer(e, "first-iteration", 0, 0, 20)?;
    let iterations = integer(e, "iterations", 3, 0, 20)?;
    let fade = number(e, "fade-opacity", 1., 0., 1.)?;
    let below = e.boolean("paste-below")?.unwrap_or(false);
    let sampler = e.enum_value("sampler-type")?.unwrap_or(1);
    if !(0..=4).contains(&sampler) {
        return Err("Unknown recursive transform sampler".into());
    }
    let fade_color = if e.argument("fade-color")?.is_some() { Some(super::resources::native_color(e, "fade-color", context)?) } else { None };
    context.interrupt.check().map_err(|error| error.to_string())?;
    let end = first.checked_add(iterations).ok_or("Recursive iteration range overflow")?;
    let count = usize::try_from(iterations + 1).map_err(|_| "Recursive branch count overflow")?;
    let allowance = count.checked_mul(4096).ok_or("Recursive branch metadata overflow")?;
    if allowance > context.remaining_metadata_bytes || allowance > context.remaining_generation_bytes {
        return Err(MappingFailure {
            reason: photocraft_doc::import_conversion::ConversionReason::RetentionLimit,
            message: "Recursive native branch data exceeds the remaining metadata or generation allowance".into(),
        });
    }
    let mut power = Homography::IDENTITY;
    let mut branches = Vec::new();
    branches.try_reserve_exact(count).map_err(|_| "Recursive native branches allocation unavailable")?;
    for index in 0..=end {
        context.interrupt.check().map_err(|error| error.to_string())?;
        if index >= first {
            let bounds = context.native_filter_bounds;
            let points = [(bounds.x0, bounds.y0), (bounds.x1, bounds.y0), (bounds.x0, bounds.y1), (bounds.x1, bounds.y1)];
            let mut side = None;
            for (x, y) in points {
                let (x, y) = (f64::from(x), f64::from(y));
                let w = power.w(x, y);
                if !w.is_finite() || w.abs() < 1e-10 || side.is_some_and(|side| side != w.is_sign_positive()) {
                    return Err("Recursive projective footprint crosses or touches its horizon; bounded native clipping conversion remains pending".into());
                }
                side = Some(w.is_sign_positive());
                let (x, y) = power.apply(x, y);
                if !x.is_finite() || !y.is_finite() || x.abs() > 16_777_216. || y.abs() > 16_777_216. {
                    return Err("Repeated projective footprint exceeds bounded native coordinates".into());
                }
            }
            let [a, c, x, b, d, y, p, q, w] = power.0;
            let (placement, perspective) = if p == 0. && q == 0. && w != 0. {
                (Affine { m: [a / w, b / w, c / w, d / w, x / w, y / w] }, None)
            } else {
                (Affine::IDENTITY, Some(power.0))
            };
            let mut layer_effects = Vec::new();
            if let Some(mut color) = fade_color {
                // Native ColorOverlay stores alpha in its common opacity, not Color.alpha.
                let opacity = 1. - (1. - f64::from(color.alpha)).powi(index as i32);
                color.alpha = 1.;
                layer_effects.push(Effect::ColorOverlay { common: FxCommon { enabled: true, blend: BlendMode::Normal, opacity: opacity as f32 }, color });
            }
            branches.push(NativeBranch { transform: placement, perspective, opacity: fade.powi(index as i32) as f32, layer_effects, ..Default::default() });
        }
        if index < end {
            power = normalize_matrix(transform.mul(&power))?;
        }
    }
    if below {
        branches.reverse();
    }
    Ok(NativeMapping { structure: Some(NativeStructure::Composite { branches }), notes: vec!["Inclusive first..first+iterations source copies become ordinary native affine or numeric projective Smart Object branches; homogeneous scale is normalized and every requested power has a finite invertible bounded footprint. Native front-side/horizon clipping and interpolation differ from source resampling; fade-opacity is exponentiated per iteration and paste-below reverses native stack order. Typed fade color becomes native ColorOverlay with effective alpha in effect opacity; repeated color fading is an explicit native approximation. Native interpolation, edge coverage, transformed style placement and working-space compositing apply".into()], ..Default::default() })
}

fn reinhard(e: &StoredEffect<'_>) -> Result<NativeMapping> {
    fields(e, &["brightness", "chromatic", "light"])?;
    let brightness = number(e, "brightness", 0., -100., 100.)?;
    let chromatic = number(e, "chromatic", 0., 0., 1.)?;
    let light = number(e, "light", 1., 0., 1.)?;
    Ok(mapped(
        "filter.other.hdrToning",
        json!({"radius":30.,"strength":0.1+0.4*light,"gamma":1.,"exposure":0.,"detail":30.,"shadow":brightness,"highlight":0.,"vibrance":0.,"saturation":-50.*chromatic,"curve":[]}),
        "Native local HDR adaptation approximates source tone-compression intent using PhotoCraft's ordinary editable HDR Toning. Source brightness drives the native signed shadow slider, not exposure stops; source light drives native compression strength0.1..0.5 and chromatic drives native saturation0..-50; source chromatic adaptation uses image/channel statistics whereas native saturation scales distance from native luminance. These are disclosed native response policies, not source-equivalent units or Reinhard statistics. Native radius30/detail30 and bounded RGB gamut follow PhotoCraft rules; native alpha is preserved rather than reproducing source alpha changes. Original source settings remain historical, and subsequent edits use native adaptation only",
    ))
}

fn panorama(e: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping> {
    fields(e, &["pan", "tilt", "spin", "zoom", "width", "height", "inverse", "sampler-type"])?;
    let pan = number(e, "pan", 0., -360., 360.)?.to_radians();
    let tilt = number(e, "tilt", 0., -180., 180.)?.to_radians();
    let spin = number(e, "spin", 0., -360., 360.)?.to_radians();
    let zoom = number(e, "zoom", 100., 0.01, 1000.)?;
    let bounds = context.native_filter_bounds;
    let width = bounds.width();
    let height = bounds.height();
    if width == 0 || height == 0 {
        return Err("Panorama requires a nonempty native frame".into());
    }
    let declared_width = number(e, "width", -1., -1., 10000.)?;
    let declared_height = number(e, "height", -1., -1., 10000.)?;
    if declared_width.fract() != 0. || declared_height.fract() != 0. {
        return Err("Panorama dimensions must be integers".into());
    }
    // Independent stock observations: a new frame is used only when both
    // dimensions are positive. A lone dimension or zero pair preserves extent.
    let (frame_width, frame_height) =
        if declared_width > 0. && declared_height > 0. { (declared_width as u32, declared_height as u32) } else { (width, height) };
    let inverse = e.boolean("inverse")?.unwrap_or(false);
    let frame = Rect::new(
        bounds.x0,
        bounds.y0,
        bounds.x0.checked_add(i32::try_from(frame_width).map_err(|_| "Panorama frame width exceeds native coordinates")?).ok_or("Panorama frame x overflow")?,
        bounds
            .y0
            .checked_add(i32::try_from(frame_height).map_err(|_| "Panorama frame height exceeds native coordinates")?)
            .ok_or("Panorama frame y overflow")?,
    );
    if inverse {
        let mask_bytes = coordinate_mask_work_bytes(frame)?;
        let map_bytes = usize::try_from(u64::from(width) * u64::from(height))
            .ok()
            .and_then(|n| n.checked_mul(64))
            .and_then(|n| n.checked_add(16384))
            .ok_or("Panorama map work overflow")?;
        let map_metadata = usize::try_from(u64::from(width) * u64::from(height))
            .ok()
            .and_then(|n| n.checked_mul(8))
            .and_then(|n| n.checked_add(16384))
            .ok_or("Panorama metadata overflow")?;
        if map_bytes.checked_add(mask_bytes).is_none_or(|n| n > context.remaining_generation_bytes)
            || map_metadata.checked_add(mask_bytes).is_none_or(|n| n > context.remaining_metadata_bytes)
        {
            return Err(MappingFailure {
                reason: photocraft_doc::import_conversion::ConversionReason::RetentionLimit,
                message: "Inverse panorama native mask and map exceed actual working/metadata allowances before allocation".into(),
            });
        }
    }
    let sampler = match e.argument("sampler-type")? {
        None => "nearest",
        Some(ArgumentValue::String(value)) if ["nearest", "linear", "cubic", "nohalo", "lohalo"].contains(value) => value,
        Some(ArgumentValue::Enum(value)) => match value {
            0 => "nearest",
            1 => "linear",
            2 => "cubic",
            3 => "nohalo",
            4 => "lohalo",
            _ => return Err("Unknown panorama sampler".into()),
        },
        _ => return Err("Unknown panorama sampler".into()),
    };
    let (sin_spin, cos_spin) = spin.sin_cos();
    let (sin_tilt, cos_tilt) = tilt.sin_cos();
    let inverse_uv = |u: f64, v: f64| {
        let longitude = u * std::f64::consts::TAU - pan;
        let latitude = (v - 0.5) * std::f64::consts::PI;
        let (sin_longitude, cos_longitude) = longitude.sin_cos();
        let (sin_latitude, cos_latitude) = latitude.sin_cos();
        let rx = sin_longitude * cos_latitude;
        let vertical = sin_latitude;
        let forward = cos_longitude * cos_latitude;
        let ry = vertical * cos_tilt - forward * sin_tilt;
        let depth = vertical * sin_tilt + forward * cos_tilt;
        if depth <= 1e-8 {
            return None;
        }
        let nx = (rx * cos_spin + ry * sin_spin) / depth;
        let ny = (-rx * sin_spin + ry * cos_spin) / depth;
        let u = f64::from(width) * 0.5 + nx * f64::from(width) * 0.5 * zoom / 100.;
        let v = f64::from(height) * 0.5 + ny * f64::from(width) * 0.5 * zoom / 100.;
        if u.is_finite() && v.is_finite() && u >= 0.5 && u <= f64::from(width) - 0.5 && v >= 0.5 && v <= f64::from(height) - 0.5 { Some((u, v)) } else { None }
    };
    let source_coordinate = |x: u32, y: u32| {
        // Standard pinhole camera with horizontal half-width one at 100% zoom.
        // Pixel centres and finite centre rays are native authoring policies.
        if inverse {
            return inverse_uv((f64::from(x) + 0.5) / f64::from(width), (f64::from(y) + 0.5) / f64::from(height))
                .unwrap_or((f64::from(x) + 0.5, f64::from(y) + 0.5));
        }
        let factor = 100. / zoom;
        let nx = (f64::from(x) + 0.5 - f64::from(width) * 0.5) / (f64::from(width) * 0.5) * factor;
        let ny = (f64::from(y) + 0.5 - f64::from(height) * 0.5) / (f64::from(width) * 0.5) * factor * (f64::from(frame_height) / f64::from(height))
            / (f64::from(frame_width) / f64::from(width));
        let rx = nx * cos_spin - ny * sin_spin;
        let ry = nx * sin_spin + ny * cos_spin;
        let vertical = ry * cos_tilt + sin_tilt;
        let forward = cos_tilt - ry * sin_tilt;
        let longitude = (rx.atan2(forward) + pan).rem_euclid(std::f64::consts::TAU);
        let latitude = vertical.atan2(rx.hypot(forward));
        let u = (longitude / std::f64::consts::TAU * f64::from(width) + 0.5).rem_euclid(f64::from(width));
        let v = ((0.5 + latitude / std::f64::consts::PI) * f64::from(height) + 0.5).clamp(0.5, f64::from(height) - 0.5);
        (u, v)
    };
    let mut mapping = coordinate_map(
        context,
        source_coordinate,
        &format!(
            "One-time standard pinhole/equirectangular camera geometry becomes an ordinary editable native Displace map. Observed source pan zero uses the panorama longitude seam; pan/tilt/spin orientation and reciprocal percentage zoom control native UV authoring at unchanged extent. Native pixel-centred rays use a 90° horizontal camera at zoom100; source centre singularities are not reproduced. Native U8 map quantization, bilinear sampling (source requested {sampler}), longitude wrapping and clamped pole coordinates approximate source sampling. Original source parameters remain historical; no projection evaluator runs after import"
        ),
    )?;
    let mask = if inverse { Some(coordinate_mask(context, frame, |u, v| inverse_uv(u, v).is_some())?) } else { None };
    if inverse || frame_width != width || frame_height != height {
        let sx = f64::from(frame_width) / f64::from(width);
        let sy = f64::from(frame_height) / f64::from(height);
        let transform = Affine { m: [sx, 0., 0., sy, f64::from(bounds.x0) * (1. - sx), f64::from(bounds.y0) * (1. - sy)] };
        mapping.structure = Some(NativeStructure::Composite {
            branches: vec![NativeBranch { filters: std::mem::take(&mut mapping.filters), transform, mask, ..Default::default() }],
        });
        mapping.notes.push(format!("Native frame {frame_width}x{frame_height} becomes an ordinary editable Smart Object affine placement anchored at the native owner origin; both positive dimensions select a frame, lone/zero dimensions preserve source extent. Source output samples are approximated by native input-grid map authoring then native affine resampling. Inverse={inverse} uses an original standard sphere-to-pinhole camera and an editable native raster mask for back-facing/out-of-view rays; mask coverage is evaluated in the final output frame, with native hard-edge coverage/placement/resampling rather than source inverse singularities; normalized native inverse reframing differs from observed source resized inverse-view behavior. No camera evaluator survives import"));
    }
    Ok(mapping)
}
pub(super) fn coordinate_mask_work_bytes(frame: Rect) -> Result<usize> {
    if frame.is_empty() {
        return Err("Panorama mask requires a nonempty frame".into());
    }
    let tile = i64::from(photocraft_geom::TILE_SIZE);
    let columns = (i64::from(frame.x1) - 1).div_euclid(tile) - i64::from(frame.x0).div_euclid(tile) + 1;
    let rows = (i64::from(frame.y1) - 1).div_euclid(tile) - i64::from(frame.y0).div_euclid(tile) + 1;
    let tiles = columns
        .checked_mul(rows)
        .and_then(|n| n.checked_mul(tile * tile * 4))
        .and_then(|n| usize::try_from(n).ok())
        .ok_or("Panorama mask tile size overflow")?;
    let row = usize::try_from(frame.width()).ok().and_then(|width| width.checked_mul(4)).ok_or("Panorama mask row size overflow")?;
    tiles.checked_add(row).ok_or_else(|| "Panorama mask scratch overflow".into())
}
pub(super) fn coordinate_mask(context: &MappingContext, frame: Rect, visible: impl Fn(f64, f64) -> bool) -> Result<photocraft_doc::LayerMask> {
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    let bytes = coordinate_mask_work_bytes(frame)?;
    if bytes > context.remaining_generation_bytes || bytes > context.remaining_metadata_bytes {
        return Err(MappingFailure::resource("Native coordinate mask exceeds remaining working/metadata allowance"));
    }
    context.interrupt.check().map_err(|e| e.to_string())?;
    let mut surface = photocraft_raster::Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
    let mut row = Vec::new();
    row.try_reserve_exact(frame.width() as usize).map_err(|_| "Panorama mask row allocation unavailable")?;
    row.resize(frame.width() as usize, 0.);
    for y in 0..frame.height() {
        context.interrupt.check().map_err(|e| e.to_string())?;
        for (x, value) in row.iter_mut().enumerate() {
            if x % 256 == 0 {
                context.interrupt.check().map_err(|e| e.to_string())?;
            }
            *value = if visible((x as f64 + 0.5) / f64::from(frame.width()), (f64::from(y) + 0.5) / f64::from(frame.height())) { 1. } else { 0. };
        }
        let y = frame.y0 + y as i32;
        surface.try_write_region(Rect::new(frame.x0, y, frame.x1, y + 1), &row).map_err(|e| e.to_string())?;
    }
    Ok(photocraft_doc::LayerMask { surface, ..photocraft_doc::LayerMask::reveal_all() })
}

/// Author an ordinary native Displace resource from a deterministic coordinate map.
/// Destination indices are local to native_filter_bounds. Returned source coordinates
/// use the same local pixel-centre convention: the first source centre is (0.5,0.5).
/// No callback or source operation survives import; only native U8 map data is stored.
pub(super) fn coordinate_map(context: &MappingContext, coordinate: impl Fn(u32, u32) -> (f64, f64), note: &str) -> Result<NativeMapping> {
    let bounds = context.native_filter_bounds;
    let width = bounds.width();
    let height = bounds.height();
    context.interrupt.check().map_err(|error| error.to_string())?;
    let count =
        usize::try_from(width).ok().and_then(|w| usize::try_from(height).ok().and_then(|h| w.checked_mul(h))).ok_or("native coordinate map area overflow")?;
    if count == 0 || count > 1 << 24 {
        return Err("Coordinate map exceeds native embedded displacement dimensions".into());
    }
    // Native params encode two U8 planes in a descriptor hex string, then native
    // contents/JSON may wrap those bytes again. Reserve conservatively for those
    // concurrent copies before creating either the float map or its descriptor.
    let metadata = count.checked_mul(8).and_then(|n| n.checked_add(16384)).ok_or("native coordinate parameter size overflow")?;
    let work = count.checked_mul(64).and_then(|n| n.checked_add(16384)).ok_or("native coordinate work size overflow")?;
    if metadata > (32 << 20).min(context.remaining_metadata_bytes) || work > context.remaining_generation_bytes {
        return Err(MappingFailure {
            reason: photocraft_doc::import_conversion::ConversionReason::RetentionLimit,
            message: "Native coordinate map/hex/base64 scratch or parameters exceed the current generation/32MiB JSON allowance".into(),
        });
    }
    let displacement = |x, y| {
        let (u, v) = coordinate(x, y);
        (u - (f64::from(x) + 0.5), v - (f64::from(y) + 0.5))
    };
    let mut range_x = 0f64;
    let mut range_y = 0f64;
    for y in 0..height {
        context.interrupt.check().map_err(|error| error.to_string())?;
        for x in 0..width {
            if x % 256 == 0 {
                context.interrupt.check().map_err(|error| error.to_string())?;
            }
            let (dx, dy) = displacement(x, y);
            if !dx.is_finite() || !dy.is_finite() {
                return Err("Native coordinate callback produced nonfinite UV offsets".into());
            }
            range_x = range_x.max(dx.abs());
            range_y = range_y.max(dy.abs());
        }
    }
    let max_displacement = 999. * 128. / 100.;
    if !range_x.is_finite() || !range_y.is_finite() || range_x > max_displacement || range_y > max_displacement {
        return Err(
            "Coordinate UV offsets exceed this native Displace approximation's ±1278.72 pixel range; wider native warp conversion remains unfinished".into()
        );
    }
    let samples = count.checked_mul(3).ok_or("native coordinate sample count overflow")?;
    let mut data = Vec::new();
    data.try_reserve_exact(samples).map_err(|_| "native coordinate map allocation unavailable")?;
    for y in 0..height {
        context.interrupt.check().map_err(|error| error.to_string())?;
        for x in 0..width {
            if x % 256 == 0 {
                context.interrupt.check().map_err(|error| error.to_string())?;
            }
            let (dx, dy) = displacement(x, y);
            if !dx.is_finite() || !dy.is_finite() {
                return Err("Native coordinate callback produced nonfinite UV offsets".into());
            }
            if dx.abs() > range_x || dy.abs() > range_y {
                return Err("Native coordinate callback changed between bounded authoring passes".into());
            }
            data.extend_from_slice(&[
                if range_x == 0. { 0.5 } else { (0.5 + dx / (2. * range_x)) as f32 },
                if range_y == 0. { 0.5 } else { (0.5 + dy / (2. * range_y)) as f32 },
                0.5,
            ]);
        }
    }
    let image = photocraft_algo::Image { rect: Rect::new(0, 0, width as i32, height as i32), ch: 3, data };
    let settings = json!({"horizontal":range_x*100./128.,"vertical":range_y*100./128.,"fit":"stretch","undefinedAreas":"wrap"});
    let params = crate::smart_map::displace_embed_map(&settings, &image).ok_or("native coordinate map could not be embedded")?;
    context.interrupt.check().map_err(|error| error.to_string())?;
    Ok(mapped("filter.distort.displace", params, note))
}

fn shift(e: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping> {
    fields(e, &["shift", "direction", "seed"])?;
    let amount = integer(e, "shift", 5, 0, 200)?;
    let direction = text(e, "direction", "horizontal")?;
    if !["horizontal", "vertical"].contains(&direction) {
        return Err("Unknown shift direction".into());
    }
    let seed = integer(e, "seed", 0, 0, u32::MAX)?;
    context.interrupt.check().map_err(|error| error.to_string())?;
    if amount == 0 {
        return Ok(NativeMapping::default());
    }
    let horizontal = direction == "horizontal";
    let length = if horizontal { context.native_filter_bounds.height() } else { context.native_filter_bounds.width() };
    if length == 0 || length > crate::xcf::MAX_SIDE {
        return Err("Native shift map geometry is outside the import canvas limits".into());
    }
    // One map sample per source row/column, not a full image. Include a
    // conservative allowance for the native descriptor/hex serialization copies.
    let work = usize::try_from(length).ok().and_then(|n| n.checked_mul(64)).and_then(|n| n.checked_add(4096)).ok_or("native shift map allowance overflow")?;
    if work > context.remaining_generation_bytes {
        return Err("Native shift map exceeds remaining generation allowance".into());
    }
    let metadata =
        usize::try_from(length).ok().and_then(|n| n.checked_mul(8)).and_then(|n| n.checked_add(4096)).ok_or("native shift parameter size overflow")?;
    if metadata > context.remaining_metadata_bytes {
        return Err(MappingFailure {
            reason: photocraft_doc::import_conversion::ConversionReason::RetentionLimit,
            message: "Native shift map encoding exceeds remaining metadata allowance".into(),
        });
    }
    let count = usize::try_from(length).ok().and_then(|n| n.checked_mul(3)).ok_or("native shift map sample overflow")?;
    let mut data = Vec::new();
    data.try_reserve_exact(count).map_err(|_| "native shift map allocation unavailable")?;
    for index in 0..length {
        if index % 256 == 0 {
            context.interrupt.check().map_err(|error| error.to_string())?;
        }
        let value = lattice(0, i64::from(index), 0, seed) as f32;
        let pixel = if horizontal { [value, 0.5, 0.5] } else { [0.5, value, 0.5] };
        data.extend_from_slice(&pixel);
    }
    let rect = if horizontal { photocraft_geom::Rect::new(0, 0, 1, length as i32) } else { photocraft_geom::Rect::new(0, 0, length as i32, 1) };
    let map = photocraft_algo::Image { rect, ch: 3, data };
    // Native Displace full contrast moves 128 pixels at 100% scale.
    let scale = f64::from(amount) * 100. / 128.;
    let settings = json!({"horizontal":if horizontal{scale}else{0.},"vertical":if horizontal{0.}else{scale},"fit":"stretch","undefinedAreas":"repeat"});
    let params = crate::smart_map::displace_embed_map(&settings, &map).ok_or("native shift displacement map could not be embedded")?;
    context.interrupt.check().map_err(|error| error.to_string())?;
    Ok(mapped(
        "filter.distort.displace",
        params,
        "Source stochastic row/column shift becomes ordinary native Displace with an embedded one-dimensional seeded map. Source shift controls maximum native displacement in pixels; direction selects the map axis. Original native lattice hashing replaces the source random sequence; native 8-bit map quantization, bilinear sampling and repeated boundary pixels differ. The native embedded map and displacement scale remain editable; no external map or source evaluator is needed",
    ))
}

fn red_eye(e: &StoredEffect<'_>) -> Result<NativeMapping> {
    fields(e, &["threshold"])?;
    let threshold = number(e, "threshold", 0.4, 0., 0.8)?;
    super::resources::lookup(17,"Imported native red eye correction",vec!["Threshold-directed native RGB/HSB red-dominance correction becomes ordinary editable Color Lookup data. A native hue falloff and red-dominance ramp select reddish pixels; selected saturation/brightness approach neutral green/blue brightness. This is an explicit native approximation, with different detection thresholds and no spatial eye recognition. Native working RGB, LUT interpolation, 0..1 gamut and alpha/compositing rules apply; original source threshold remains retained".into()],|input|{
        let rgb=input.map(|v|v as f32);
        let [h,s,v]=photocraft_color::convert::rgb_to_hsb(rgb);
        let hue_weight=(1.-f64::from(h.min(360.-h))/45.).clamp(0.,1.);
        let dominance=(input[0]-input[1].max(input[2])).max(0.);
        let weight=hue_weight*((dominance-(0.8-threshold))/0.2).clamp(0.,1.);
        let corrected=photocraft_color::convert::hsb_to_rgb([h,s*(1.-weight as f32),v*(1.-weight as f32)+(rgb[1]+rgb[2])*0.5*weight as f32]);
        corrected.map(f64::from)
    }).map_err(Into::into)
}

fn extent_mask(x: f64, y: f64, width: u32, height: u32) -> photocraft_doc::VectorMask {
    photocraft_doc::VectorMask::new(photocraft_doc::Path::new(vec![photocraft_doc::Subpath::polygon(&[
        (x, y),
        (x + f64::from(width), y),
        (x + f64::from(width), y + f64::from(height)),
        (x, y + f64::from(height)),
    ])]))
}
fn empty_pattern() -> NativeMapping {
    NativeMapping {
        structure: Some(NativeStructure::Replace {
            layer: Box::new(photocraft_doc::Layer::new(
                "Empty source generator",
                photocraft_doc::LayerContent::Fill(photocraft_doc::Fill::Solid(photocraft_color::Color::TRANSPARENT)),
            )),
            patterns: Vec::new(),
        }),
        notes: vec!["Zero-sized source generator becomes ordinary transparent native fill".into()],
        ..Default::default()
    }
}
fn source_color(e: &StoredEffect<'_>, name: &str, default: [f32; 4]) -> Result<[f32; 4]> {
    match e.argument(name)? {
        None => Ok(default),
        Some(ArgumentValue::Color(Some(_)) | ArgumentValue::LiteralColor { .. }) => super::resources::color(e, name).map_err(Into::into),
        _ => Err(format!("{name} has no portable source color literal").into()),
    }
}
fn sinus(e: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping> {
    fields(e, &["x-scale", "y-scale", "complexity", "seed", "tiling", "perturbation", "color1", "color2", "blend-mode", "blend-power", "width", "height"])?;
    let xs = number(e, "x-scale", 15., 0.0001, 1_000_000.)?;
    let ys = number(e, "y-scale", 15., 0.0001, 1_000_000.)?;
    let complexity = number(e, "complexity", 1., 0., 15.)?;
    let seed = integer(e, "seed", 0, 0, u32::MAX)?;
    let tile = e.boolean("tiling")?.unwrap_or(true);
    let perturb = e.boolean("perturbation")?.unwrap_or(true);
    let first = source_color(e, "color1", [1., 1., 0., 1.])?;
    let second = source_color(e, "color2", [0., 0., 1., 1.])?;
    let blend = text(e, "blend-mode", "sinusoidal")?;
    if !["linear", "bilinear", "sinusoidal"].contains(&blend) {
        return Err("Unknown sinus blend mode".into());
    }
    let power = number(e, "blend-power", 0., -7.5, 7.5)?.exp2();
    let width = integer(e, "width", 1024, 0, i32::MAX as u32)?;
    let height = integer(e, "height", 768, 0, i32::MAX as u32)?;
    if width == 0 || height == 0 {
        return Ok(empty_pattern());
    }
    let cycles_x = if tile { (f64::from(width) / xs).round().max(1.) } else { f64::from(width) / xs };
    let cycles_y = if tile { (f64::from(height) / ys).round().max(1.) } else { f64::from(height) / ys };
    let phase = f64::from(seed) / f64::from(u32::MAX) * std::f64::consts::TAU;
    let mut converted = super::residual_tone::pattern_result(
        "Imported native sine pattern",
        photocraft_geom::Size::new(width, height),
        (0., 0.),
        0.,
        context,
        |x, y| {
            let a = f64::from(x) / f64::from(width) * cycles_x * std::f64::consts::TAU + phase;
            let b = f64::from(y) / f64::from(height) * cycles_y * std::f64::consts::TAU - phase;
            let distortion = if perturb { complexity * 0.25 * b.sin() } else { 0. };
            // Original standard sine superposition, not the source generator algorithm.
            let mut mix = ((a + distortion).sin() + b.sin() + 2.) * 0.25;
            mix = match blend {
                "bilinear" => 1. - (2. * mix - 1.).abs(),
                "sinusoidal" => (1. - (mix * std::f64::consts::PI).cos()) * 0.5,
                _ => mix,
            };
            mix = mix.clamp(0., 1.).powf(power);
            Ok(std::array::from_fn(|channel| ((1. - mix) * f64::from(first[channel]) + mix * f64::from(second[channel])) as f32))
        },
        "Original public-math sine superposition becomes ordinary native Pattern data with source colors converted through their own ICC into the native target ICC. Native wavelength policy uses x/y scale as approximate pixel periods; seed controls phase, complexity controls cross-axis distortion, tiling quantizes cycles, blend mode/power shape native color interpolation. This is an editable native resource approximation, not the source stochastic generator algorithm",
    )?;
    if let Some(NativeStructure::Replace { layer, .. }) = &mut converted.structure {
        layer.vector_mask = Some(extent_mask(0., 0., width, height));
    }
    Ok(converted)
}

/// Original smooth value noise: integer lattice hash and cubic smoothstep.
/// This import-only native pattern authoring primitive does not evaluate a source node.
pub(super) fn lattice(x: i64, y: i64, z: i64, seed: u32) -> f64 {
    let mut value = (x as u64).wrapping_mul(0x9e3779b185ebca87)
        ^ (y as u64).wrapping_mul(0xc2b2ae3d27d4eb4f)
        ^ (z as u64).wrapping_mul(0x165667b19e3779f9)
        ^ u64::from(seed);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
    (value ^ (value >> 31)) as f64 / u64::MAX as f64
}
fn value_noise(x: f64, y: f64, z: f64, seed: u32) -> f64 {
    let (ix, iy, iz) = (x.floor() as i64, y.floor() as i64, z.floor() as i64);
    let smooth = |v: f64| v * v * (3. - 2. * v);
    let (fx, fy, fz) = (smooth(x - x.floor()), smooth(y - y.floor()), smooth(z - z.floor()));
    let mut value = 0.;
    for dz in 0..2 {
        for dy in 0..2 {
            for dx in 0..2 {
                value += lattice(ix.saturating_add(dx), iy.saturating_add(dy), iz.saturating_add(dz), seed)
                    * if dx == 0 { 1. - fx } else { fx }
                    * if dy == 0 { 1. - fy } else { fy }
                    * if dz == 0 { 1. - fz } else { fz };
            }
        }
    }
    value
}
fn noise(e: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping> {
    let (width, height, x, y, seed, octaves, frequency, multiplier, decay, z, colored) = match e.operation {
        "gegl:plasma" => {
            fields(e, &["turbulence", "x", "y", "width", "height", "seed"])?;
            let turbulence = number(e, "turbulence", 1., 0., 7.)?;
            (
                integer(e, "width", 1024, 0, i32::MAX as u32)?,
                integer(e, "height", 768, 0, i32::MAX as u32)?,
                number(e, "x", 0., f64::from(i32::MIN), f64::from(i32::MAX))?,
                number(e, "y", 0., f64::from(i32::MIN), f64::from(i32::MAX))?,
                integer(e, "seed", 0, 0, u32::MAX)?,
                6,
                1. / 128.,
                2.,
                0.25 + turbulence * 0.075,
                0.,
                true,
            )
        }
        "gegl:perlin-noise" => {
            fields(e, &["alpha", "scale", "zoff", "n"])?;
            let alpha = number(e, "alpha", 1.2, 0.01, 100.)?;
            let scale = number(e, "scale", 1.8, 0.01, 4.)?;
            (
                context.native_filter_bounds.width(),
                context.native_filter_bounds.height(),
                f64::from(context.native_filter_bounds.x0),
                f64::from(context.native_filter_bounds.y0),
                0,
                integer(e, "n", 3, 0, 20)?,
                1. / 64.,
                scale,
                1. / alpha,
                number(e, "zoff", -1., -1_000_000., 1_000_000.)?,
                false,
            )
        }
        _ => {
            fields(e, &["scale", "iterations", "seed"])?;
            (
                context.native_filter_bounds.width(),
                context.native_filter_bounds.height(),
                f64::from(context.native_filter_bounds.x0),
                f64::from(context.native_filter_bounds.y0),
                integer(e, "seed", 0, 0, u32::MAX)?,
                integer(e, "iterations", 1, 1, 20)?,
                number(e, "scale", 1., 0., 20.)? / 64.,
                2.,
                0.5,
                0.,
                false,
            )
        }
    };
    if width == 0 || height == 0 {
        return Ok(empty_pattern());
    }
    if x.fract() != 0. || y.fract() != 0. {
        return Err("Source procedural extent coordinates must be integral".into());
    }
    let work = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|n| n.checked_mul(u64::from(octaves.max(1))))
        .and_then(|n| n.checked_mul(if colored { 3 } else { 1 }))
        .ok_or("native noise work count overflow")?;
    if work > 32_000_000 {
        return Err("Native procedural pattern exceeds bounded import work allowance".into());
    }
    let last_frequency = frequency * multiplier.powi(octaves as i32);
    if !last_frequency.is_finite()
        || last_frequency * (x.abs() + y.abs() + f64::from(width) + f64::from(height) + 1.) > 1e14
        || !decay.powi(octaves as i32).is_finite()
    {
        return Err("Native procedural controls exceed finite lattice/amplitude bounds".into());
    }
    let mut converted = super::residual_tone::pattern_result(
        "Imported native fractal pattern",
        photocraft_geom::Size::new(width, height),
        (x as f32, y as f32),
        0.,
        context,
        |px, py| {
            let mut output = [0., 0., 0., 1.];
            for channel in 0..if colored { 3 } else { 1 } {
                let mut sum = 0.;
                let mut weight = 0.;
                let mut amplitude = 1.;
                let mut f = frequency;
                for octave in 0..octaves {
                    sum += amplitude
                        * value_noise((f64::from(px) + x) * f, (f64::from(py) + y) * f, z + f64::from(octave) * 0.37, seed.wrapping_add(channel * 0x10001));
                    weight += amplitude;
                    amplitude *= decay;
                    f *= multiplier;
                }
                if let Some(value) = output.get_mut(channel as usize) {
                    *value = if weight == 0. { 0.5 } else { (sum / weight).clamp(0., 1.) as f32 };
                }
            }
            if !colored {
                output[1] = output[0];
                output[2] = output[0];
            }
            Ok(output)
        },
        "Original smooth lattice noise and normalized octave superposition becomes ordinary native Pattern pixels through target ICC. Source generator intent, seed and bounded geometry are translated; source algorithms are approximated by native cubic value noise. Perlin alpha becomes native inverse octave-amplitude ratio, scale an octave-frequency ratio, zoff a third-coordinate phase; Simplex scale controls approximate pixel frequency and iterations octave count; Plasma turbulence controls native high-frequency detail with independent RGB channels. Source spectral statistics, interpolation, normalization and random sequences differ; fixed Perlin seed 0 is a native policy, not a decoded field",
    )?;
    if let Some(NativeStructure::Replace { layer, .. }) = &mut converted.structure {
        layer.vector_mask = Some(extent_mask(x, y, width, height));
    }
    Ok(converted)
}

#[cfg(test)]
mod tests {
    use super::super::{Argument, ColorLiteral};
    use super::*;
    use photocraft_color::{ColorMode, SampleType};
    use photocraft_geom::Rect;
    fn context() -> MappingContext<'static> {
        MappingContext {
            mode: ColorMode::Rgb,
            linear: true,
            owner_bounds: Rect::new(0, 0, 7, 5),
            native_filter_bounds: Rect::new(0, 0, 7, 5),
            target_profile: Some(photocraft_cms::Builtin::LinearSrgb.profile().to_bytes()),
            resolution_dpi: 72.0,
            remaining_generation_bytes: crate::xcf::MAX_PIXEL_BYTES,
            remaining_metadata_bytes: crate::xcf::MAX_METADATA_BYTES,
            interrupt: photocraft_raster::Interrupt::NONE,
        }
    }
    fn number(name: &'static str, value: f32) -> Argument<'static> {
        Argument { name, value: ArgumentValue::Float(value) }
    }
    fn int(name: &'static str, value: i32) -> Argument<'static> {
        Argument { name, value: ArgumentValue::Int(value) }
    }
    fn boolean(name: &'static str, value: bool) -> Argument<'static> {
        Argument { name, value: ArgumentValue::Bool(value) }
    }
    fn string(name: &'static str, value: &'static str) -> Argument<'static> {
        Argument { name, value: ArgumentValue::String(value) }
    }
    #[test]
    fn recursive_matrix_accepts_numeric_homographies_and_rejects_expressions_and_singular() {
        assert_eq!(matrix("matrix (1, 0, 4, 0, 1, -2, 0, 0, 1)").unwrap(), Homography([1., 0., 4., 0., 1., -2., 0., 0., 1.]));
        assert_eq!(matrix("matrix(2,0,0,0,2,0,0.02,0,2)").unwrap(), Homography([1., 0., 0., 0., 1., 0., 0.01, 0., 1.]));
        for raw in ["translate(1,2)", "matrix(0,0,0,0,0,0,0,0,1)", "matrix(1,0,0,0,1,0,0,0,1);file(x)", "matrix(1,0,0,0,1,0,0,0,NaN)"] {
            assert!(matrix(raw).is_err(), "{raw}");
        }
    }
    #[test]
    fn recursive_inclusive_source_count_transform_power_fade_and_order_are_native_data() {
        let source = StoredEffect::test(
            "gegl:recursive-transform",
            vec![
                string("transform", "matrix (1,0,2,0,1,-1,0,0,1)"),
                int("first-iteration", 1),
                int("iterations", 2),
                number("fade-opacity", 0.5),
                boolean("paste-below", true),
            ],
        );
        let converted = map(&source, &context()).unwrap().unwrap();
        let Some(NativeStructure::Composite { branches }) = converted.structure else { panic!("native branches") };
        assert_eq!(branches.len(), 3);
        for (branch, index) in branches.iter().zip([3, 2, 1]) {
            assert_eq!(branch.transform, Affine::translate(2. * f64::from(index), -f64::from(index)));
            assert_eq!(branch.opacity, 0.5_f32.powi(index));
            assert!(branch.filters.is_empty());
        }
    }
    #[test]
    fn recursive_projective_powers_are_native_data_with_fade_order_and_native_warp_reopen() {
        use photocraft_algo::transform::{Interp, warp_surface};
        use photocraft_color::PixelFormat;
        use photocraft_doc::{Layer, LayerContent, SmartObject, SmartSource};
        use photocraft_raster::Surface;
        let effect = StoredEffect::test(
            "gegl:recursive-transform",
            vec![
                string("transform", "matrix(1,0,0,0,1,0,0.02,0,1)"),
                int("first-iteration", 1),
                int("iterations", 2),
                number("fade-opacity", 0.5),
                boolean("paste-below", true),
            ],
        );
        let result = map(&effect, &context()).unwrap().unwrap();
        let Some(NativeStructure::Composite { branches }) = result.structure else { panic!("native branches") };
        for (branch, index) in branches.iter().zip([3, 2, 1]) {
            assert_eq!(branch.transform, Affine::IDENTITY);
            let matrix = branch.perspective.unwrap();
            assert!((matrix[6] - 0.02 * f64::from(index)).abs() < 1e-12);
            assert_eq!(branch.opacity, 0.5_f32.powi(index));
        }
        let bounds = Rect::new(0, 0, 7, 5);
        let mut source = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::F32, true));
        let pixels: Vec<_> = (0..5).flat_map(|y| (0..7).flat_map(move |x| [x as f32 / 8., y as f32 / 8., 0.25, 0.5])).collect();
        source.write_region(bounds, &pixels);
        let matrix = Homography(branches[2].perspective.unwrap());
        // Independent homogeneous projection: native x=2/(1+.02*2).
        assert!((matrix.apply(2., 1.).0 - 2. / 1.04).abs() < 1e-12);
        let warped = warp_surface(&source, bounds, &matrix, Interp::Bilinear).unwrap();
        assert_eq!(warped.pixel(2, 1)[3], 0.5);
        assert_ne!(warped.pixel(2, 1)[0], source.pixel(2, 1)[0]);
        let mut contents = photocraft_doc::Document::new("Contents", photocraft_geom::Size::new(7, 5), ColorMode::Rgb, SampleType::F32);
        contents.layers.push(Layer::new("Source", LayerContent::Raster(source.clone())));
        let bytes = photocraft_format::save_to_bytes(&contents, &Default::default()).unwrap();
        let mut smart =
            SmartObject::new(SmartSource::Embedded { file_name: "contents.pcraft".into(), bytes: std::sync::Arc::new(bytes) }, Affine::IDENTITY, None);
        smart.perspective = Some(matrix.0);
        let mut document = photocraft_doc::Document::new("Projective native", photocraft_geom::Size::new(7, 5), ColorMode::Rgb, SampleType::F32);
        let mut layer = Layer::new("Projective branch", LayerContent::Smart(smart));
        layer.opacity = branches[2].opacity;
        document.layers.push(layer);
        let bytes = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
        assert_eq!(reopened.layers[0].opacity, 0.5);
        let LayerContent::Smart(smart) = &reopened.layers[0].content else { panic!("native smart") };
        assert_eq!(smart.perspective, Some(matrix.0));
        let again = warp_surface(&source, bounds, &Homography(smart.perspective.unwrap()), Interp::Bilinear).unwrap();
        assert_eq!(again.read_region(bounds), warped.read_region(bounds));
    }
    #[test]
    fn recursive_projective_validation_is_bounded_cancellable_and_does_not_truncate_horizons() {
        for transform in ["matrix(1,0,0,0,1,0,-0.2,0,1)", "matrix(1,0,0,0,1,0,0,0,0)", "matrix(1,0,0,0,1,0,0,0,NaN)"] {
            let effect = StoredEffect::test("gegl:recursive-transform", vec![string("transform", transform), int("first-iteration", 1), int("iterations", 0)]);
            assert!(map(&effect, &context()).unwrap().is_err());
        }
        let effect = StoredEffect::test("gegl:recursive-transform", vec![string("transform", "matrix(1,0,0,0,1,0,0.02,0,1)")]);
        for ctx in [MappingContext { remaining_metadata_bytes: 1, ..context() }, MappingContext { remaining_generation_bytes: 1, ..context() }] {
            assert_eq!(map(&effect, &ctx).unwrap().unwrap_err().reason, photocraft_doc::import_conversion::ConversionReason::RetentionLimit);
        }
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let cancel = || calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 2;
        let ctx = MappingContext { interrupt: photocraft_raster::Interrupt::cancel_only(&cancel), ..context() };
        assert!(map(&effect, &ctx).unwrap().is_err());
    }
    #[test]
    fn recursive_translucent_color_uses_effect_opacity_instead_of_ignored_color_alpha() {
        let source = StoredEffect::test(
            "gegl:recursive-transform",
            vec![
                int("first-iteration", 1),
                int("iterations", 1),
                Argument {
                    name: "fade-color",
                    value: ArgumentValue::Color(Some(ColorLiteral { encoding: "R'G'B'A u8", pixels: &[255, 0, 0, 128], profile: &[] })),
                },
            ],
        );
        let converted = map(&source, &context()).unwrap().unwrap();
        let Some(NativeStructure::Composite { branches }) = converted.structure else { panic!("branches") };
        for (branch, index) in branches.iter().zip([1, 2]) {
            let Effect::ColorOverlay { common, color } = &branch.layer_effects[0] else { panic!("overlay") };
            assert_eq!(color.alpha, 1.);
            assert!((common.opacity - (1. - (1. - 128. / 255.0_f32).powi(index))).abs() < 1e-6);
        }
    }
    fn clip_document(mode: ColorMode, profile: std::sync::Arc<Vec<u8>>) -> photocraft_doc::Document {
        use photocraft_color::PixelFormat;
        use photocraft_doc::{Layer, LayerContent};
        use photocraft_raster::Surface;
        let mut doc = photocraft_doc::Document::new("Native clipping counterpart", photocraft_geom::Size::new(4, 1), mode, SampleType::F32);
        doc.icc_profile = Some(profile);
        let mut source = Surface::new(PixelFormat::new(mode, SampleType::F32, true));
        for (x, (value, alpha)) in [(-2., 0.), (-1., 0.25), (0.5, 0.75), (3., 1.)].into_iter().enumerate() {
            let rgba = [value, value, value, alpha];
            let gray = [value, alpha];
            source.write_pixel(x as i32, 0, if mode == ColorMode::Grayscale { &gray } else { &rgba });
        }
        doc.layers.push(Layer::new("Original native pixels", LayerContent::Raster(source)));
        doc
    }
    #[test]
    fn clip_signed_hdr_fills_match_independent_native_clipping_and_preserve_coverage_after_reopen() {
        use photocraft_color::{BlendMode, Color};
        use photocraft_doc::{Fill, Layer, LayerContent};
        for (mode, linear, profile) in [
            (ColorMode::Rgb, true, photocraft_cms::Builtin::LinearSrgb.profile().to_bytes()),
            (ColorMode::Rgb, false, photocraft_cms::Builtin::Srgb.profile().to_bytes()),
            (ColorMode::Grayscale, true, photocraft_cms::builtin::linear_gray().to_bytes()),
        ] {
            let context = MappingContext { mode, linear, target_profile: Some(profile.clone()), ..context() };
            let converted = map(&StoredEffect::test("gegl:rgb-clip", vec![number("low-limit", -0.5), number("high-limit", 2.)]), &context).unwrap().unwrap();
            assert!(converted.filters.is_empty());
            let Some(NativeStructure::Composite { branches }) = converted.structure else { panic!("native clip branches") };
            assert_eq!(branches.len(), 3);
            let mut actual = clip_document(mode, profile.clone());
            for branch in branches.into_iter().skip(1) {
                assert!(branch.filters.is_empty());
                assert!(branch.clipped);
                let mut layer = Layer::new("Converted editable clipping fill", LayerContent::Fill(branch.fill.unwrap()));
                layer.blend = branch.blend;
                layer.clipped = branch.clipped;
                actual.layers.push(layer);
            }
            // Independent ordinary native document: create two clipping fills by
            // hand, using document-model values instead of converter output.
            let mut expected = clip_document(mode, profile.clone());
            for (value, blend) in [(-0.5, BlendMode::Lighten), (2., BlendMode::Darken)] {
                let color = if mode == ColorMode::Grayscale { Color::gray(value) } else { Color::rgb(value, value, value) };
                let mut layer = Layer::new("Native manual clipping fill", LayerContent::Fill(Fill::Solid(color)));
                layer.clipped = true;
                layer.blend = blend;
                expected.layers.push(layer);
            }
            let rendered = photocraft_compose::render(&actual, actual.bounds());
            assert_eq!(rendered.px, photocraft_compose::render(&expected, expected.bounds()).px);
            for (pixel, expected) in rendered.px.iter().zip([[0., 0., 0., 0.], [-0.5, -0.5, -0.5, 0.25], [0.5, 0.5, 0.5, 0.75], [2., 2., 2., 1.]]) {
                for (got, wanted) in pixel.iter().zip(expected) {
                    assert!((got - wanted).abs() < 1e-5, "{mode:?}/{linear}: {pixel:?} vs {expected:?}");
                }
            }
            let bytes = photocraft_format::save_to_bytes(&actual, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
            assert_eq!(reopened.icc_profile, Some(profile));
            assert_eq!(photocraft_compose::render(&reopened, reopened.bounds()).px, rendered.px);
            assert!(matches!(reopened.layers[1].content, LayerContent::Fill(Fill::Solid(_))));
            assert!(reopened.layers[1].clipped);
        }
    }
    #[test]
    fn clip_crossed_bounds_follow_declared_native_order_without_reversed_clamp() {
        use photocraft_doc::{Layer, LayerContent};
        let profile = photocraft_cms::Builtin::LinearSrgb.profile().to_bytes();
        let mut doc = clip_document(ColorMode::Rgb, profile);
        let converted = map(&StoredEffect::test("gegl:rgb-clip", vec![number("low-limit", 0.75), number("high-limit", 0.25)]), &context()).unwrap().unwrap();
        let Some(NativeStructure::Composite { branches }) = converted.structure else { panic!("native clip branches") };
        for branch in branches.into_iter().skip(1) {
            let mut fill = Layer::new("Native ordered clip", LayerContent::Fill(branch.fill.unwrap()));
            fill.clipped = branch.clipped;
            fill.blend = branch.blend;
            doc.layers.push(fill);
        }
        let pixels = photocraft_compose::render(&doc, doc.bounds()).px;
        assert_eq!(pixels[0], [0.; 4]);
        for (pixel, alpha) in pixels.iter().skip(1).zip([0.25, 0.75, 1.]) {
            assert_eq!(*pixel, [0.25, 0.25, 0.25, alpha]);
        }
    }
    #[test]
    fn clip_flags_crossed_limits_and_invalid_values_are_explicit_native_policies() {
        let effect = |args| map(&StoredEffect::test("gegl:rgb-clip", args), &context()).unwrap().unwrap();
        assert!(effect(vec![boolean("clip-low", false), boolean("clip-high", false)]).structure.is_none());
        for (enabled, command) in [("clip-low", photocraft_color::BlendMode::Darken), ("clip-high", photocraft_color::BlendMode::Lighten)] {
            let mapped = effect(vec![boolean(enabled, false)]);
            let Some(NativeStructure::Composite { branches }) = mapped.structure else { panic!("native structure") };
            assert_eq!(branches.len(), 2);
            assert_eq!(branches[1].blend, command);
        }
        let crossed = effect(vec![number("low-limit", 0.75), number("high-limit", 0.25)]);
        assert!(crossed.notes.iter().any(|note| note.contains("Crossed source bounds")));
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(map(&StoredEffect::test("gegl:rgb-clip", vec![number("low-limit", value)]), &context()).unwrap().is_err());
        }
    }
    #[test]
    fn softglow_keeps_original_and_ordinary_editable_screen_branch() {
        let result = map(&StoredEffect::test("gegl:softglow", vec![number("brightness", 0.25), number("glow-radius", 5.)]), &context()).unwrap().unwrap();
        let Some(NativeStructure::Composite { branches }) = result.structure else { panic!("branches") };
        assert_eq!(branches.len(), 2);
        assert!(branches[0].filters.is_empty());
        assert_eq!(branches[0].opacity, 1.);
        assert_eq!(branches[1].blend, BlendMode::Screen);
        assert_eq!(branches[1].opacity, 0.25);
        assert_eq!(branches[1].filters[0].command, "image.adjustments.curves");
        assert_eq!(branches[1].filters[1].params["radius"], 5.);
    }
    #[test]
    fn pixelize_grid_shapes_background_full_domain_and_native_means_are_editable() {
        use photocraft_doc::{LayerContent, PathOp};
        for norm in 0..=2 {
            let effect = StoredEffect::test(
                "gegl:pixelize",
                vec![
                    int("size-x", 4),
                    int("size-y", 3),
                    int("offset-x", -1),
                    int("offset-y", 1),
                    number("ratio-x", 0.5),
                    Argument { name: "norm", value: ArgumentValue::Enum(norm) },
                ],
            );
            let mapped = map(&effect, &context()).unwrap().unwrap();
            let Some(NativeStructure::Composite { branches }) = mapped.structure else { panic!("cells") };
            assert_eq!(branches.len(), 7);
            assert_eq!(branches[1].filter_frame, Some(Rect::new(0, 0, 3, 1)));
            let background = branches[0].vector_mask.as_ref().unwrap();
            assert_eq!(background.path.subpaths.len(), 7);
            assert!(background.path.subpaths.iter().skip(1).all(|p| p.op == PathOp::Subtract));
            let path = &branches[1].vector_mask.as_ref().unwrap().path;
            assert_eq!(path.subpaths[0].knots.len(), 4);
            assert_eq!(path.subpaths[0].knots.iter().any(|k| k.smooth), norm == 1);
        }
        for args in [vec![int("size-x", i32::MAX), int("size-y", i32::MAX)], vec![int("offset-x", i32::MIN), int("offset-y", i32::MAX)]] {
            assert!(map(&StoredEffect::test("gegl:pixelize", args), &context()).unwrap().is_ok());
        }
        let ambiguous = [0, 0, 128, 63, 0, 0, 128, 63, 0, 0, 128, 63, 0, 0, 128, 63];
        let background =
            || Argument { name: "background", value: ArgumentValue::Color(Some(ColorLiteral { encoding: "RGBA float", pixels: &ambiguous, profile: &[] })) };
        assert!(map(&StoredEffect::test("gegl:pixelize", vec![background()]), &context()).unwrap().is_ok());
        assert!(map(&StoredEffect::test("gegl:pixelize", vec![background(), number("ratio-x", 0.5)]), &context()).unwrap().is_err());
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let bounds = Rect::new(0, 0, 4, 1);
            let format = photocraft_color::PixelFormat::new(ColorMode::Grayscale, sample, false);
            let mut source = photocraft_raster::Surface::new(format);
            source.write_region(bounds, &[0., 0.25, 0.5, 1.]);
            let effect = StoredEffect::test("gegl:pixelize", vec![int("size-x", 2), int("size-y", 1)]);
            let mut workspace = crate::xcf::MAX_PIXEL_BYTES;
            let mut metadata = crate::xcf::MAX_METADATA_BYTES;
            let (actual, _) = super::super::super::coverage::apply(
                &source,
                bounds,
                &effect,
                None,
                false,
                &mut super::super::super::coverage::Budgets { workspace: &mut workspace, metadata: &mut metadata },
                &photocraft_raster::Interrupt::NONE,
            )
            .unwrap()
            .unwrap();
            let raw = source.read_region(bounds);
            let a = (raw[0] + raw[1]) * 0.5;
            let b = (raw[2] + raw[3]) * 0.5;
            let mut expected = photocraft_raster::Surface::new(format);
            expected.write_region(bounds, &[a, a, b, b]);
            for (a, b) in actual.read_region(bounds).iter().zip(expected.read_region(bounds)) {
                assert!((a - b).abs() < 1e-5, "{sample:?}: {a} vs {b}");
            }
            let mut doc = photocraft_doc::Document::new("native geometry", photocraft_geom::Size::new(4, 1), ColorMode::Grayscale, sample);
            let mapping = map(
                &effect,
                &MappingContext { mode: ColorMode::Grayscale, target_profile: None, native_filter_bounds: bounds, owner_bounds: bounds, ..context() },
            )
            .unwrap()
            .unwrap();
            let Some(NativeStructure::Composite { branches }) = mapping.structure else { panic!("cells") };
            for branch in branches {
                let mut layer = photocraft_doc::Layer::raster("editable mask", format);
                layer.vector_mask = branch.vector_mask;
                if let Some(fill) = branch.fill {
                    layer.content = LayerContent::Fill(fill);
                }
                doc.layers.push(layer);
            }
            let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
            assert_eq!(reopened.layers[1].vector_mask, doc.layers[1].vector_mask);
        }
        let small = MappingContext { remaining_generation_bytes: 0, ..context() };
        assert!(map(&StoredEffect::test("gegl:pixelize", vec![number("ratio-x", 0.5)]), &small).unwrap().is_err());
        let dense = MappingContext { native_filter_bounds: Rect::new(0, 0, 200, 200), ..context() };
        let fallback = map(&StoredEffect::test("gegl:pixelize", vec![int("size-x", 2), int("size-y", 2)]), &dense).unwrap().unwrap();
        assert_eq!(fallback.filters[0].command, "filter.pixelate.mosaic");
        assert!(map(&StoredEffect::test("gegl:pixelize", vec![int("size-x", 2), int("size-y", 2), number("ratio-x", 0.5)]), &dense).unwrap().is_err());
    }

    #[test]
    fn pixelize_slic_spherize_preserve_editable_native_domains_and_disclose_partial_controls() {
        let pixel = map(&StoredEffect::test("gegl:pixelize", vec![int("size-x", 8), int("size-y", 32), number("ratio-x", 0.5)]), &context()).unwrap().unwrap();
        let Some(NativeStructure::Composite { branches }) = pixel.structure else { panic!("native cells") };
        assert_eq!(branches.len(), 2);
        assert_eq!(branches[1].filter_frame, Some(Rect::new(0, 0, 7, 5)));
        assert_eq!(branches[1].filters[0].command, "filter.blur.average");
        assert!(pixel.notes.iter().any(|n| n.contains("coverage ratios")));
        let slic = map(&StoredEffect::test("gegl:slic", vec![int("compactness", 5), int("iterations", 3)]), &context()).unwrap().unwrap();
        let Some(NativeStructure::Composite { branches }) = slic.structure else { panic!("regional cells") };
        assert_eq!(branches[0].filters[0].params, json!({"cellSize":32,"seed":0}));
        assert!((branches[1].opacity - 4. / 39.).abs() < 1e-6);
        assert!(slic.notes[0].contains("not a decoded source seed"));
        let sphere =
            map(&StoredEffect::test("gegl:spherize", vec![number("amount", -0.5), Argument { name: "mode", value: ArgumentValue::Enum(2) }]), &context())
                .unwrap()
                .unwrap();
        assert_eq!(sphere.filters[0].params, json!({"mode":"verticalOnly","amount":-50.}));
    }
    #[test]
    fn ripple_standard_wave_geometry_authors_native_phase_angle_and_full_range_maps() {
        let create = |wave, phase, angle| {
            map(
                &StoredEffect::test(
                    "gegl:ripple",
                    vec![
                        number("amplitude", 2.),
                        number("period", 4.),
                        number("phi", phase),
                        number("angle", angle),
                        string("wave-type", wave),
                        string("abyss-policy", "loop"),
                    ],
                ),
                &context(),
            )
            .unwrap()
            .unwrap()
        };
        for (wave, range) in [("sine", 2.0f64.sqrt()), ("triangle", 1.), ("sawtooth", 1.5)] {
            let result = create(wave, 0., 0.);
            let filter = &result.filters[0];
            assert_eq!(filter.command, "filter.distort.displace");
            assert!((filter.params["vertical"].as_f64().unwrap() - range * 100. / 128.).abs() < 1e-6);
            assert_eq!(filter.params["horizontal"], 0.);
            let native = crate::smart_map::displace_embedded_map(&filter.params).unwrap();
            let expected = if wave == "sawtooth" { 2. / 3. } else { 1. };
            assert!((native.data[1] - expected).abs() <= 1. / 255.);
            let restored = reopen_filter(filter.clone());
            assert_eq!(restored.params, filter.params);
            let bounds = context().native_filter_bounds;
            let mut constant = photocraft_raster::Surface::new(photocraft_color::PixelFormat::new(ColorMode::Rgb, SampleType::F32, true));
            constant.write_region(bounds, &[-0.25, 2., 0.75, 0.625].repeat(35));
            let rendered = displace_surface(&restored, &constant, bounds);
            for pixel in rendered.read_region(bounds).as_chunks::<4>().0 {
                for (actual, expected) in pixel.iter().zip([-0.25, 2., 0.75, 0.625]) {
                    assert!((actual - expected).abs() < 1e-6, "native wrap must preserve signed/HDR constant and alpha");
                }
            }
            let rotated = create(wave, 0., 90.);
            assert!(rotated.filters[0].params["horizontal"].as_f64().unwrap() > 0.);
            assert!(rotated.filters[0].params["vertical"].as_f64().unwrap() < 1e-12);
        }
        let phase = create("sine", 0.25, 0.);
        let embedded = crate::smart_map::displace_embedded_map(&phase.filters[0].params).unwrap();
        // Native sine at pixel centre1.5, period4, phase1/4 is -sqrt(1/2).
        assert_eq!(embedded.data[4], 0.);
        for args in [
            vec![number("amplitude", 0.1), number("period", 0.)],
            vec![number("amplitude", 1000.), number("period", 1000.)],
            vec![string("wave-type", "sawtooth"), boolean("tileable", true)],
        ] {
            assert!(map(&StoredEffect::test("gegl:ripple", args), &context()).unwrap().is_ok());
        }
        let transparent = map(&StoredEffect::test("gegl:ripple", vec![number("amplitude", 1000.)]), &context()).unwrap().unwrap();
        let Some(NativeStructure::Composite { branches }) = transparent.structure else { panic!("transparent boundary") };
        assert!(branches[0].mask.as_ref().unwrap().surface.read_region(context().native_filter_bounds).contains(&0.));
        let tiny = MappingContext { remaining_generation_bytes: 1, ..context() };
        assert!(map(&StoredEffect::test("gegl:ripple", vec![]), &tiny).unwrap().is_err());
        let cancel = || true;
        let cancelled = MappingContext { interrupt: photocraft_raster::Interrupt::cancel_only(&cancel), ..context() };
        assert!(map(&StoredEffect::test("gegl:ripple", vec![]), &cancelled).unwrap().is_err());
    }

    #[test]
    fn slic_regularization_and_spherical_cap_policy_are_native_editable_parameters() {
        let minimum = map(&StoredEffect::test("gegl:slic", vec![int("compactness", 1)]), &context()).unwrap().unwrap();
        assert_eq!(minimum.filters[0].params, json!({"cellSize":32,"seed":0}));
        let maximum = map(&StoredEffect::test("gegl:slic", vec![int("compactness", 40), int("iterations", 30)]), &context()).unwrap().unwrap();
        let Some(NativeStructure::Composite { branches }) = maximum.structure else { panic!("regular cells") };
        assert_eq!(branches[1].opacity, 1.);
        assert_eq!(branches[1].filters[0].command, "filter.blur.average");
        for size in [2, 301, i32::MAX] {
            let result = map(&StoredEffect::test("gegl:slic", vec![int("cluster-size", size)]), &context()).unwrap().unwrap();
            assert!(matches!(result.structure, Some(NativeStructure::Composite { .. })));
        }
        let large = MappingContext { native_filter_bounds: Rect::new(0, 0, 512, 512), ..context() };
        let fallback = map(&StoredEffect::test("gegl:slic", vec![]), &large).unwrap().unwrap();
        assert_eq!(fallback.filters[0].command, "filter.pixelate.crystallize");
        assert!(fallback.notes[0].contains("bounded native cell/cache work"));
        let sphere = |args| map(&StoredEffect::test("gegl:spherize", args), &context()).unwrap().unwrap();
        let a = sphere(vec![number("curvature", 0.5), number("amount", -0.5)]);
        let b = sphere(vec![number("angle-of-view", 90.), number("amount", -0.5)]);
        let independent = -(1. - 0.5f64.sqrt()) * 50.;
        assert!((a.filters[0].params["amount"].as_f64().unwrap() - independent).abs() < 1e-12);
        assert_eq!(a.filters[0].params, b.filters[0].params);
        assert_eq!(reopen_filter(a.filters[0].clone()).params, a.filters[0].params);
        for args in [vec![number("curvature", 0.)], vec![number("angle-of-view", 180.)], vec![number("amount", 0.)]] {
            let identity = sphere(args);
            assert!(identity.filters.is_empty());
            assert!(identity.notes[0].contains("identity"));
        }
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let bounds = context().native_filter_bounds;
            let mut source = photocraft_raster::Surface::new(photocraft_color::PixelFormat::new(ColorMode::Rgb, sample, true));
            let pixels: Vec<_> = (0..35).flat_map(|i| [i as f32 / 8. - 0.5, 0.25, 1.5, 0.75]).collect();
            source.write_region(bounds, &pixels);
            let actual = photocraft_algo::FilterParams::Spherize {
                amount: a.filters[0].params["amount"].as_f64().unwrap() as f32,
                mode: photocraft_algo::SpherizeMode::Normal,
            };
            let expected = photocraft_algo::FilterParams::Spherize { amount: independent as f32, mode: photocraft_algo::SpherizeMode::Normal };
            let actual = photocraft_algo::apply_in_with(&source, &actual, bounds, bounds, None, bounds, &photocraft_raster::Interrupt::NONE).unwrap();
            let expected = photocraft_algo::apply_in_with(&source, &expected, bounds, bounds, None, bounds, &photocraft_raster::Interrupt::NONE).unwrap();
            assert_eq!(actual.read_region(bounds), expected.read_region(bounds));
            for pixel in actual.read_region(bounds).as_chunks::<4>().0 {
                assert!((pixel[3] - source.pixel(0, 0)[3]).abs() < 1e-5);
            }
        }
    }

    #[test]
    fn photocopy_and_ripple_are_native_filters_with_named_approximation_policy() {
        let photo = map(&StoredEffect::test("gegl:photocopy", vec![number("black", 1.), number("white", 1.)]), &context()).unwrap().unwrap();
        assert_eq!(photo.filters[0].command, "filter.blur.gaussianBlur");
        assert_eq!(photo.filters[1].command, "filter.stylize.findEdges");
        assert_eq!(photo.filters.last().unwrap().params["inBlack"], 126.);
        assert_eq!(photo.filters.last().unwrap().params["inWhite"], 128.);
        let ripple = map(&StoredEffect::test("gegl:ripple", vec![number("phi", 0.5)]), &context()).unwrap().unwrap();
        let Some(NativeStructure::Composite { branches }) = ripple.structure else { panic!("native masked wave") };
        assert_eq!(branches[0].filters[0].command, "filter.distort.displace");
        assert!(branches[0].mask.is_some());
        assert!(ripple.notes[0].contains("phase cycles"));
    }
    #[test]
    fn malformed_or_excessive_values_retain_source_without_native_clamping_or_large_allocations() {
        for source in [
            StoredEffect::test("gegl:recursive-transform", vec![string("transform", "matrix(4,0,0,0,4,0,0,0,1)"), int("iterations", 20)]),
            StoredEffect::test("gegl:pixelize", vec![int("size-x", 0)]),
            StoredEffect::test("gegl:slic", vec![int("cluster-size", 1)]),
            StoredEffect::test("gegl:spherize", vec![number("amount", f32::NAN)]),
            StoredEffect::test("gegl:softglow", vec![number("brightness", 0.5), number("brightness", 0.7)]),
            StoredEffect::test("gegl:ripple", vec![string("wave-type", "square")]),
        ] {
            assert!(map(&source, &context()).unwrap().is_err(), "{}", source.operation);
        }
    }
    #[test]
    fn sinus_is_editable_native_pattern_with_finite_extent_colors_icc_and_native_reopen() {
        let source = StoredEffect::test(
            "gegl:sinus",
            vec![
                int("width", 4),
                int("height", 3),
                number("x-scale", 4.),
                number("y-scale", 3.),
                boolean("perturbation", false),
                string("blend-mode", "linear"),
            ],
        );
        let converted = map(&source, &context()).unwrap().unwrap();
        let Some(NativeStructure::Replace { layer, patterns }) = converted.structure else { panic!("pattern") };
        assert!(layer.vector_mask.is_some());
        assert_eq!((patterns[0].width, patterns[0].height), (4, 3));
        // At phases (0,0), ordinary two-sine superposition mixes source yellow/blue
        // equally in sRGB. Independent sRGB midpoint converted to linear RGB.
        for component in patterns[0].surface.pixel(0, 0).iter().take(3) {
            assert!((*component - 0.21404114).abs() < 0.00001);
        }
        let mut document = photocraft_doc::Document::new("Native sinus", photocraft_geom::Size::new(8, 6), ColorMode::Rgb, SampleType::F32);
        document.icc_profile = context().target_profile;
        document.patterns = patterns;
        document.layers.push(*layer);
        let before = photocraft_compose::render(&document, document.bounds());
        assert_eq!(before.px[0][3], 1.);
        assert_eq!(before.px[7][3], 0.);
        assert_eq!(before.px[47][3], 0.);
        let bytes = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
        assert_eq!(photocraft_compose::render(&reopened, reopened.bounds()).px, before.px);
        let empty = map(&StoredEffect::test("gegl:sinus", vec![int("width", 0)]), &context()).unwrap().unwrap();
        let Some(NativeStructure::Replace { patterns, .. }) = empty.structure else { panic!("empty fill") };
        assert!(patterns.is_empty());
    }
    #[test]
    fn original_native_noise_patterns_use_seed_geometry_octaves_and_real_color_channels() {
        let first = map(&StoredEffect::test("gegl:simplex-noise", vec![]), &context()).unwrap().unwrap();
        let Some(NativeStructure::Replace { patterns, .. }) = first.structure else { panic!("noise") };
        assert_eq!(patterns[0].surface.pixel(0, 0), [0., 0., 0., 1.]);
        let changed =
            map(&StoredEffect::test("gegl:simplex-noise", vec![Argument { name: "seed", value: ArgumentValue::UInt(1) }]), &context()).unwrap().unwrap();
        let Some(NativeStructure::Replace { patterns: changed, .. }) = changed.structure else { panic!("noise") };
        assert_ne!(patterns[0].id, changed[0].id);
        let plasma = map(&StoredEffect::test("gegl:plasma", vec![int("width", 8), int("height", 6), int("x", -3), int("y", 2)]), &context()).unwrap().unwrap();
        let Some(NativeStructure::Replace { layer, patterns }) = plasma.structure else { panic!("plasma") };
        assert_eq!((patterns[0].width, patterns[0].height), (8, 6));
        let pixel = patterns[0].surface.pixel(0, 0);
        assert_ne!(pixel[0], pixel[1]);
        assert_ne!(pixel[1], pixel[2]);
        assert!(pixel.iter().all(|v| v.is_finite()));
        assert!(layer.vector_mask.is_some());
        let mut document = photocraft_doc::Document::new("Native plasma", photocraft_geom::Size::new(10, 10), ColorMode::Rgb, SampleType::F32);
        document.icc_profile = context().target_profile;
        document.patterns = patterns;
        document.layers.push(*layer);
        let before = photocraft_compose::render(&document, document.bounds());
        assert_eq!(before.px[0][3], 0.);
        assert_eq!(before.px[20][3], 1.);
        assert_eq!(before.px[29][3], 0.);
        let bytes = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
        assert_eq!(photocraft_compose::render(&reopened, reopened.bounds()).px, before.px);
        for source in [
            StoredEffect::test("gegl:plasma", vec![int("width", i32::MAX)]),
            StoredEffect::test("gegl:perlin-noise", vec![number("alpha", 0.)]),
            StoredEffect::test("gegl:sinus", vec![number("x-scale", f32::INFINITY)]),
            StoredEffect::test("gegl:sinus", vec![int("width", i32::MAX)]),
        ] {
            assert!(map(&source, &context()).unwrap().is_err());
        }
    }
    #[test]
    fn red_eye_authors_ordinary_native_lookup_selects_red_and_retains_neutral_green() {
        let converted = map(&StoredEffect::test("gegl:red-eye-removal", vec![]), &context()).unwrap().unwrap();
        let filter = &converted.filters[0];
        assert_eq!(filter.command, "image.adjustments.colorLookup");
        assert!(filter.params.get("file").is_none());
        let table = photocraft_cms::lutfile::parse("native.cube", filter.params["data"].as_str().unwrap().as_bytes()).unwrap();
        assert_eq!(&table.data[16 * 3..16 * 3 + 3], &[0., 0., 0.]);
        let green = 16 * 17 * 3;
        assert_eq!(&table.data[green..green + 3], &[0., 1., 0.]);
        let gray = ((8 * 17 + 8) * 17 + 8) * 3;
        assert_eq!(&table.data[gray..gray + 3], &[0.5, 0.5, 0.5]);
        let more = map(&StoredEffect::test("gegl:red-eye-removal", vec![number("threshold", 0.8)]), &context()).unwrap().unwrap();
        assert_ne!(more.filters[0].params["data"], filter.params["data"]);
        assert!(map(&StoredEffect::test("gegl:red-eye-removal", vec![number("threshold", 0.81)]), &context()).unwrap().is_err());
    }
    fn reopen_filter(filter: photocraft_doc::SmartFilter) -> photocraft_doc::SmartFilter {
        use photocraft_doc::{Layer, LayerContent, SmartObject, SmartSource};
        let contents = photocraft_doc::Document::new("Contents", photocraft_geom::Size::new(64, 32), ColorMode::Rgb, SampleType::F32);
        let contents = photocraft_format::save_to_bytes(&contents, &Default::default()).unwrap();
        let mut smart =
            SmartObject::new(SmartSource::Embedded { file_name: "contents.pcraft".into(), bytes: std::sync::Arc::new(contents) }, Affine::IDENTITY, None);
        smart.smart_filters.push(filter);
        let mut doc = photocraft_doc::Document::new("Native mapping", photocraft_geom::Size::new(64, 32), ColorMode::Rgb, SampleType::F32);
        doc.layers.push(Layer::new("Editable native filter", LayerContent::Smart(smart)));
        let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
        let LayerContent::Smart(smart) = &reopened.layers[0].content else { panic!("native smart") };
        smart.smart_filters[0].clone()
    }
    fn displace_surface(filter: &photocraft_doc::SmartFilter, surface: &photocraft_raster::Surface, bounds: Rect) -> photocraft_raster::Surface {
        let options = photocraft_algo::FilterParams::Displace {
            horizontal: filter.params["horizontal"].as_f64().unwrap() as f32,
            vertical: filter.params["vertical"].as_f64().unwrap() as f32,
            stretch: true,
            undefined: photocraft_algo::UndefinedAreas::Wrap,
            map: crate::smart_map::displace_embedded_map(&filter.params),
        };
        photocraft_algo::apply_in_with(surface, &options, bounds, bounds, None, bounds, &photocraft_raster::Interrupt::NONE).unwrap()
    }
    #[test]
    fn coordinate_resource_preflights_before_callback_and_rejects_nonfinite_or_unstable_maps() {
        let tiny = MappingContext { remaining_metadata_bytes: 1, ..context() };
        let failure = coordinate_map(&tiny, |_, _| panic!("budget must be checked before callback"), "test").unwrap_err();
        assert_eq!(failure.reason, photocraft_doc::import_conversion::ConversionReason::RetentionLimit);
        assert!(coordinate_map(&context(), |_, _| (f64::NAN, 0.5), "test").unwrap_err().message.contains("nonfinite"));
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let changed = coordinate_map(
            &context(),
            |x, y| {
                let count = calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                (f64::from(x) + 0.5 + if count >= 35 { 1. } else { 0. }, f64::from(y) + 0.5)
            },
            "test",
        )
        .unwrap_err();
        assert!(changed.message.contains("changed between"));
    }
    #[test]
    fn panorama_authors_native_camera_map_and_native_render_reopen_preserves_map_and_coverage() {
        use photocraft_color::PixelFormat;
        use photocraft_raster::Surface;
        let bounds = Rect::new(0, 0, 64, 32);
        let ctx = MappingContext { native_filter_bounds: bounds, owner_bounds: bounds, ..context() };
        let result = map(&StoredEffect::test("gegl:panorama-projection", vec![number("pan", 90.)]), &ctx).unwrap().unwrap();
        let filter = &result.filters[0];
        assert_eq!(filter.command, "filter.distort.displace");
        assert!(filter.params.get("mapPath").is_none());
        let decoded = crate::smart_map::displace_embedded_map(&filter.params).unwrap();
        assert_eq!(decoded.rect, bounds);
        let mut surface = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::F32, true));
        let pixels: Vec<_> = (0..32).flat_map(|y| (0..64).flat_map(move |x| [x as f32 / 63., y as f32 / 31., 0.25, 0.5])).collect();
        surface.write_region(bounds, &pixels);
        let before = displace_surface(filter, &surface, bounds);
        // Independent camera direction: yaw90 faces longitude one quarter of
        // the equirectangular source, with the equator halfway down the image.
        let centre = before.pixel(32, 16);
        assert!((centre[0] - 0.25).abs() < 0.03, "{centre:?}");
        assert!((centre[1] - 0.5).abs() < 0.04, "{centre:?}");
        assert_eq!(centre[2], 0.25);
        assert_eq!(centre[3], 0.5);
        let reopened = reopen_filter(filter.clone());
        assert_eq!(crate::smart_map::displace_embedded_map(&reopened.params).unwrap(), decoded);
        assert_eq!(displace_surface(&reopened, &surface, bounds).read_region(bounds), before.read_region(bounds));
        let zoom = map(&StoredEffect::test("gegl:panorama-projection", vec![number("pan", 90.), number("zoom", 200.)]), &ctx).unwrap().unwrap();
        let narrow = displace_surface(&zoom.filters[0], &surface, bounds);
        assert!((narrow.pixel(0, 16)[0] - centre[0]).abs() < (before.pixel(0, 16)[0] - centre[0]).abs());
    }
    #[test]
    fn inverse_panorama_native_map_and_final_frame_mask_preserve_coverage_and_reopen() {
        use photocraft_color::PixelFormat;
        use photocraft_doc::{Layer, LayerContent};
        use photocraft_raster::Surface;
        let bounds = Rect::new(0, 0, 64, 32);
        let ctx = MappingContext { native_filter_bounds: bounds, owner_bounds: bounds, ..context() };
        let result = map(&StoredEffect::test("gegl:panorama-projection", vec![boolean("inverse", true)]), &ctx).unwrap().unwrap();
        let Some(NativeStructure::Composite { mut branches }) = result.structure else { panic!("native inverse") };
        let branch = branches.remove(0);
        assert_eq!(branch.transform, Affine::IDENTITY);
        let mask = branch.mask.unwrap();
        assert_eq!(mask.value(0, 16), 1.);
        assert_eq!(mask.value(32, 16), 0.);
        let mut surface = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::F32, true));
        let pixels: Vec<_> = (0..32).flat_map(|y| (0..64).flat_map(move |x| [x as f32 / 63., y as f32 / 31., 0.25, 0.5])).collect();
        surface.write_region(bounds, &pixels);
        let filtered = displace_surface(&branch.filters[0], &surface, bounds);
        let mut doc = photocraft_doc::Document::new("Native inverse camera", photocraft_geom::Size::new(64, 32), ColorMode::Rgb, SampleType::F32);
        let mut layer = Layer::new("Native inverse map pixels", LayerContent::Raster(filtered));
        layer.mask = Some(mask);
        doc.layers.push(layer);
        let before = photocraft_compose::render(&doc, bounds);
        assert_eq!(before.px[16 * 64][3], 0.5);
        assert_eq!(before.px[16 * 64 + 32][3], 0.);
        let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
        assert_eq!(photocraft_compose::render(&reopened, bounds).px, before.px);
        let rotated = map(&StoredEffect::test("gegl:panorama-projection", vec![boolean("inverse", true), number("pan", 90.)]), &ctx).unwrap().unwrap();
        let Some(NativeStructure::Composite { branches }) = rotated.structure else { panic!("native inverse") };
        assert_eq!(branches[0].mask.as_ref().unwrap().value(16, 16), 1.);
        assert_eq!(branches[0].mask.as_ref().unwrap().value(0, 16), 0.);
        let resized =
            map(&StoredEffect::test("gegl:panorama-projection", vec![boolean("inverse", true), int("width", 32), int("height", 16)]), &ctx).unwrap().unwrap();
        let Some(NativeStructure::Composite { branches }) = resized.structure else { panic!("native frame") };
        assert_eq!(branches[0].transform, Affine::scale(0.5));
        assert_eq!(branches[0].mask.as_ref().unwrap().value(0, 8), 1.);
        assert_eq!(branches[0].mask.as_ref().unwrap().value(32, 8), 0.);
        assert_eq!(branches[0].mask.as_ref().unwrap().value(0, 16), 0.);
    }
    #[test]
    fn panorama_output_frame_uses_both_positive_dimensions_and_preflights_mask_tiles() {
        let bounds = Rect::new(0, 0, 64, 32);
        let ctx = MappingContext { native_filter_bounds: bounds, owner_bounds: bounds, ..context() };
        let default = map(&StoredEffect::test("gegl:panorama-projection", vec![]), &ctx).unwrap().unwrap();
        for args in [vec![int("width", 32)], vec![int("height", 16)], vec![int("width", 0), int("height", 0)]] {
            let result = map(&StoredEffect::test("gegl:panorama-projection", args), &ctx).unwrap().unwrap();
            assert!(result.structure.is_none());
            assert_eq!(result.filters[0].params, default.filters[0].params);
        }
        let resized = map(&StoredEffect::test("gegl:panorama-projection", vec![int("width", 32), int("height", 8)]), &ctx).unwrap().unwrap();
        let Some(NativeStructure::Composite { branches }) = resized.structure else { panic!("native frame") };
        assert_eq!(branches[0].transform, Affine { m: [0.5, 0., 0., 0.25, 0., 0.] });
        assert!(branches[0].mask.is_none());
        assert_ne!(branches[0].filters[0].params, default.filters[0].params);
        let huge = map(&StoredEffect::test("gegl:panorama-projection", vec![boolean("inverse", true), int("width", 10000), int("height", 10000)]), &ctx)
            .unwrap()
            .unwrap_err();
        assert_eq!(huge.reason, photocraft_doc::import_conversion::ConversionReason::RetentionLimit);
        let cancelled = || true;
        let ctx = MappingContext { interrupt: photocraft_raster::Interrupt::cancel_only(&cancelled), ..ctx };
        assert!(map(&StoredEffect::test("gegl:panorama-projection", vec![boolean("inverse", true)]), &ctx).unwrap().is_err());
    }
    #[test]
    fn panorama_rejects_unmapped_domains_and_preflights_metadata_work_and_cancellation() {
        let source = StoredEffect::test("gegl:panorama-projection", vec![]);
        for ctx in [MappingContext { remaining_metadata_bytes: 1, ..context() }, MappingContext { remaining_generation_bytes: 1, ..context() }] {
            let failure = map(&source, &ctx).unwrap().unwrap_err();
            assert_eq!(failure.reason, photocraft_doc::import_conversion::ConversionReason::RetentionLimit);
        }
        for args in [vec![int("width", 10001)], vec![number("height", 0.5)], vec![number("zoom", f32::NAN)], vec![string("sampler-type", "invalid")]] {
            assert!(map(&StoredEffect::test("gegl:panorama-projection", args), &context()).unwrap().is_err());
        }
        let large = MappingContext { native_filter_bounds: Rect::new(0, 0, 3000, 1), ..context() };
        assert!(map(&source, &large).unwrap().unwrap_err().message.contains("1278.72"));
        let cancelled = std::sync::atomic::AtomicBool::new(true);
        let cancel = || cancelled.load(std::sync::atomic::Ordering::Relaxed);
        let ctx = MappingContext { interrupt: photocraft_raster::Interrupt::cancel_only(&cancel), ..context() };
        assert!(map(&source, &ctx).unwrap().is_err());
    }
    #[test]
    fn reinhard_intent_uses_checked_native_hdr_data_preserves_alpha_and_native_reopen() {
        use photocraft_color::PixelFormat;
        use photocraft_raster::{Interrupt, Surface};
        let result = map(&StoredEffect::test("gegl:reinhard05", vec![number("brightness", 25.), number("chromatic", 0.5), number("light", 0.25)]), &context())
            .unwrap()
            .unwrap();
        let filter = &result.filters[0];
        assert_eq!(filter.command, "filter.other.hdrToning");
        assert_eq!(filter.params["shadow"], 25.);
        assert_eq!(filter.params["exposure"], 0.);
        assert_eq!(filter.params["strength"], 0.2);
        assert_eq!(filter.params["saturation"], -25.);
        let options = photocraft_algo::tone::HdrToning { strength: 0.2, shadow: 25., saturation: -25., ..Default::default() };
        let bounds = Rect::new(0, 0, 16, 2);
        let mut source = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::F32, true));
        let pixels: Vec<_> = (0..32)
            .flat_map(|i| {
                let v = (i % 16 + 1) as f32 / 4.;
                [v, v / 2., v / 4., 0.75]
            })
            .collect();
        source.write_region(bounds, &pixels);
        let native = photocraft_algo::tone::checked_hdr::apply_with(&source, &options, bounds, bounds, None, &Interrupt::NONE).unwrap();
        for pixel in native.read_region(bounds).as_chunks::<4>().0 {
            assert!(pixel[..3].iter().all(|v| v.is_finite() && (0. ..=1.).contains(v)));
            assert_eq!(pixel[3], 0.75);
        }
        assert_ne!(native.read_region(bounds), pixels);
        let restored = reopen_filter(filter.clone());
        assert_eq!(restored.params, filter.params);
        assert!(result.notes[0].contains("not exposure stops"));
        for args in [vec![number("brightness", 101.)], vec![number("chromatic", -0.1)], vec![number("light", f32::NAN)]] {
            assert!(map(&StoredEffect::test("gegl:reinhard05", args), &context()).unwrap().is_err());
        }
    }
    #[test]
    fn stochastic_shift_embeds_one_dimensional_native_map_and_replays_after_native_reopen() {
        use photocraft_color::PixelFormat;
        use photocraft_doc::{Layer, LayerContent, SmartObject, SmartSource};
        use photocraft_raster::{Interrupt, Surface};
        let bounds = Rect::new(0, 0, 12, 4);
        let context = MappingContext { native_filter_bounds: bounds, owner_bounds: bounds, ..context() };
        let converted = map(&StoredEffect::test("gegl:shift", vec![int("shift", 2)]), &context).unwrap().unwrap();
        let filter = &converted.filters[0];
        assert_eq!(filter.command, "filter.distort.displace");
        assert_eq!(filter.params["horizontal"], 1.5625);
        assert_eq!(filter.params["vertical"], 0.);
        assert!(filter.params.get("mapPath").is_none());
        let embedded = crate::smart_map::displace_embedded_map(&filter.params).unwrap();
        assert_eq!((embedded.rect.width(), embedded.rect.height()), (1, 4));
        assert_eq!(embedded.data[0], 0.);
        let mut surface = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::F32, true));
        let pixels: Vec<_> = (0..4).flat_map(|y| (0..12).flat_map(move |x| [x as f32 / 16., y as f32 / 8., 0.25, 0.5])).collect();
        surface.write_region(bounds, &pixels);
        let params = photocraft_algo::FilterParams::Displace {
            horizontal: 1.5625,
            vertical: 0.,
            stretch: true,
            undefined: photocraft_algo::UndefinedAreas::Repeat,
            map: Some(embedded.clone()),
        };
        let before = photocraft_algo::apply_in_with(&surface, &params, bounds, bounds, None, bounds, &Interrupt::NONE).unwrap();
        // Seed zero gives lattice zero on row zero, therefore the native inverse
        // map samples exactly two pixels to the left at this interior location.
        assert_eq!(before.pixel(6, 0), surface.pixel(4, 0));
        assert_eq!(before.pixel(6, 0)[3], 0.5);
        let mut original = photocraft_doc::Document::new("Input", photocraft_geom::Size::new(12, 4), ColorMode::Rgb, SampleType::F32);
        original.layers.push(Layer::new("Input", LayerContent::Raster(surface.clone())));
        let source = photocraft_format::save_to_bytes(&original, &Default::default()).unwrap();
        let mut smart =
            SmartObject::new(SmartSource::Embedded { file_name: "contents.pcraft".into(), bytes: std::sync::Arc::new(source) }, Affine::IDENTITY, None);
        smart.smart_filters = converted.filters;
        let mut native = photocraft_doc::Document::new("Native displacement", photocraft_geom::Size::new(12, 4), ColorMode::Rgb, SampleType::F32);
        native.layers.push(Layer::new("Editable map", LayerContent::Smart(smart)));
        let bytes = photocraft_format::save_to_bytes(&native, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
        let LayerContent::Smart(restored) = &reopened.layers[0].content else { panic!("native smart object") };
        let restored_map = crate::smart_map::displace_embedded_map(&restored.smart_filters[0].params).unwrap();
        assert_eq!(restored_map, embedded);
        let after = photocraft_algo::apply_in_with(
            &surface,
            &photocraft_algo::FilterParams::Displace {
                horizontal: 1.5625,
                vertical: 0.,
                stretch: true,
                undefined: photocraft_algo::UndefinedAreas::Repeat,
                map: Some(restored_map),
            },
            bounds,
            bounds,
            None,
            bounds,
            &Interrupt::NONE,
        )
        .unwrap();
        assert_eq!(after.read_region(bounds), before.read_region(bounds));
        let vertical = map(&StoredEffect::test("gegl:shift", vec![string("direction", "vertical")]), &context).unwrap().unwrap();
        let map = crate::smart_map::displace_embedded_map(&vertical.filters[0].params).unwrap();
        assert_eq!((map.rect.width(), map.rect.height()), (12, 1));
        let tiny = MappingContext { remaining_generation_bytes: 1, ..context };
        assert!(super::map(&StoredEffect::test("gegl:shift", vec![]), &tiny).unwrap().is_err());
    }
}
