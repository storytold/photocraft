//! Finite B06-03/04 conversions. Only ordinary native commands/styles are emitted.
//! Unfinished procedural constructions retain source configuration and a named gap.

use super::{ArgumentValue, MappingContext, MappingFailure, MappingResult, NativeBranch, NativeMapping, NativeStructure, StoredEffect, native_filter};
use photocraft_color::{BlendMode, ColorMode};
use photocraft_doc::{
    SmartFilter,
    effects::{Contour, Effect, FxCommon, Shadow},
    import_conversion::ConversionReason,
};
use serde_json::{Value, json};

type Result<T> = std::result::Result<T, MappingFailure>;

fn invalid(message: impl Into<String>) -> MappingFailure {
    MappingFailure { reason: ConversionReason::InvalidOptionalData, message: message.into() }
}

fn number(e: &StoredEffect<'_>, key: &str, default: f64, low: f64, high: f64) -> Result<f64> {
    let value = e.number(key).map_err(invalid)?.unwrap_or(default);
    if !value.is_finite() || !(low..=high).contains(&value) {
        return Err(invalid(format!("{}: {key} is outside its finite source domain", e.operation)));
    }
    Ok(value)
}

fn integer(e: &StoredEffect<'_>, key: &str, default: u32, low: u32, high: u32) -> Result<u32> {
    let v = number(e, key, f64::from(default), f64::from(low), f64::from(high))?;
    if v.fract() != 0.0 {
        return Err(invalid(format!("{key} must be integral")));
    }
    Ok(v as u32)
}

fn choice(e: &StoredEffect<'_>, key: &str, default: usize, names: &[&str]) -> Result<usize> {
    match e.argument(key).map_err(invalid)? {
        None => Ok(default),
        Some(ArgumentValue::Enum(v)) => usize::try_from(*v).ok().filter(|v| *v < names.len()).ok_or_else(|| invalid(format!("unknown {key} enum"))),
        Some(ArgumentValue::String(v)) => names.iter().position(|name| name == v).ok_or_else(|| invalid(format!("unknown {key} nick"))),
        _ => Err(invalid(format!("{key} is not a stored enum or public nick"))),
    }
}

fn allowed(e: &StoredEffect<'_>, names: &[&str]) -> Result<()> {
    for arg in &e.arguments {
        if !names.contains(&arg.name) {
            return Err(format!("unmapped {} argument {}", e.operation, arg.name).into());
        }
        e.argument(arg.name).map_err(invalid)?;
    }
    Ok(())
}

fn mapped(filters: Vec<SmartFilter>, note: impl Into<String>) -> NativeMapping {
    NativeMapping { filters, notes: vec![note.into()], ..Default::default() }
}

fn one(command: &str, params: Value, note: impl Into<String>) -> NativeMapping {
    mapped(vec![native_filter(command, params)], note)
}

fn deinterlace(e: &StoredEffect<'_>, context: &MappingContext<'_>) -> Result<NativeMapping> {
    allowed(e, &["keep", "orientation", "size"])?;
    let keep = choice(e, "keep", 0, &["even", "odd"])?;
    let orientation = choice(e, "orientation", 0, &["horizontal", "vertical"])?;
    let size = integer(e, "size", 1, 0, 100)?;
    if size == 0 {
        return Err("Zero deinterlace support needs independently established source boundary behavior".into());
    }
    if size > 1 {
        return deinterlace_support(keep, orientation, size, context);
    }
    let mut native = one(
        "filter.video.deInterlace",
        json!({"eliminate":if keep == 0 {"oddFields"} else {"evenFields"}, "createBy":"interpolation"}),
        "Stock-source keep-even retains owner-local zero-based odd rows; native one-based odd fields are removed. Native neighboring-field interpolation, alpha and boundary rules replace source interpolation; vertical fields use native quarter-turn placement around the filter",
    );
    native.quarter_turns = orientation as u8;
    Ok(native)
}

fn deinterlace_support(keep: usize, orientation: usize, size: u32, context: &MappingContext<'_>) -> Result<NativeMapping> {
    use photocraft_doc::vector::{Knot, Path, Subpath, VectorMask};
    context.interrupt.check().map_err(|_| MappingFailure::from("Native field interpolation cancelled"))?;
    let bounds = context.native_filter_bounds;
    if bounds.is_empty() {
        return Err(invalid("Deinterlace owner bounds are empty"));
    }
    let extent = if orientation == 0 { bounds.height() } else { bounds.width() };
    let fields = if keep == 0 { extent.div_ceil(2) } else { extent / 2 };
    let count = usize::try_from(fields).map_err(|_| invalid("Native field count overflow"))?;
    if count == 0 {
        return Ok(mapped(Vec::new(), "The owner has no missing field at the selected parity; preserve native pixels"));
    }
    // Two independent vector masks, four knots per missing row/column.
    if count.checked_mul(8).is_none_or(|n| n > 65_536) {
        return Err("Native deinterlace field masks exceed65,536 bounded geometry knots".into());
    }
    let bytes = count
        .checked_mul(std::mem::size_of::<Subpath>() + 4 * std::mem::size_of::<Knot>())
        .and_then(|n| n.checked_mul(2))
        .and_then(|n| n.checked_add(4096))
        .ok_or_else(|| invalid("Native deinterlace geometry allocation overflow"))?;
    if bytes > context.remaining_generation_bytes {
        return Err(MappingFailure::resource("Native deinterlace vector masks exceed remaining generation allowance"));
    }
    let mut paths = Vec::new();
    paths.try_reserve_exact(count).map_err(|_| MappingFailure::resource("Native field geometry allocation unavailable"))?;
    for row in (keep as u32..extent).step_by(2) {
        context.interrupt.check().map_err(|_| MappingFailure::from("Native field interpolation cancelled"))?;
        let x = f64::from(bounds.x0) + if orientation == 1 { f64::from(row) } else { 0.0 };
        let y = f64::from(bounds.y0) + if orientation == 0 { f64::from(row) } else { 0.0 };
        let right = if orientation == 0 { f64::from(bounds.x1) } else { x + 1.0 };
        let bottom = if orientation == 1 { f64::from(bounds.y1) } else { y + 1.0 };
        paths.push(Subpath::polygon(&[(x, y), (right, y), (right, bottom), (x, bottom)]));
    }
    let mask = VectorMask::new(Path::new(paths));
    let offset = |delta: i32| {
        native_filter(
            "filter.other.offset",
            json!({"horizontal":if orientation==1 {delta} else {0},"vertical":if orientation==0 {delta} else {0},"undefinedAreas":"repeat"}),
        )
    };
    Ok(NativeMapping {
        structure: Some(NativeStructure::Composite {
            branches: vec![
                NativeBranch::default(),
                NativeBranch { filters: vec![offset(size as i32)], vector_mask: Some(mask.clone()), ..Default::default() },
                NativeBranch { filters: vec![offset(-(size as i32))], vector_mask: Some(mask), opacity: 0.5, ..Default::default() },
            ],
        }),
        notes: vec![format!(
            "Independently observed source block sizes2/3 retain alternating single fields; block size changes support, not row grouping. Source support{size}px becomes editable native Offset copies at+/-{size}px, restricted to missing single-row/column VectorMasks and averaged by native50% Normal blending. Native repeated-edge boundaries, interpolation and mask antialiasing apply. Native branch compositing approximates source premultiplied interpolation on semitransparent pixels; this is a native support-scale approximation, not source kernel emulation"
        )],
        ..Default::default()
    })
}

fn dither(e: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping> {
    allowed(e, &["red-levels", "green-levels", "blue-levels", "alpha-levels", "dither-method", "seed"])?;
    let levels = [integer(e, "red-levels", 6, 2, 65536)?, integer(e, "green-levels", 6, 2, 65536)?, integer(e, "blue-levels", 6, 2, 65536)?];
    let alpha = integer(e, "alpha-levels", 256, 2, 65536)?;
    let method = choice(
        e,
        "dither-method",
        1,
        &[
            "none",
            "floyd-steinberg",
            "bayer",
            "random",
            "random-covariant",
            "arithmetic-add",
            "arithmetic-add-covariant",
            "arithmetic-xor",
            "arithmetic-xor-covariant",
            "blue-noise",
            "blue-noise-covariant",
        ],
    )?;
    let seed = match e.argument("seed").map_err(invalid)? {
        None => 0,
        Some(ArgumentValue::UInt(seed)) => *seed,
        _ => return Err(invalid("dither seed must be an unsigned stored integer")),
    };
    if !matches!(context.mode, ColorMode::Rgb | ColorMode::Grayscale) {
        return Err("Dither quantization requires a native RGB/Gray channel mapping".into());
    }
    if context.remaining_generation_bytes < 65536 || context.remaining_metadata_bytes < 65536 {
        return Err("native dither transfer parameters exceed remaining allowance".into());
    }
    context.interrupt.check().map_err(|e| e.to_string())?;
    let curves =
        levels.map(|n| (0..256).map(|i| ((f64::from(i) / 255.0 * f64::from(n - 1)).round() / f64::from(n - 1) * 255.0).clamp(0.0, 255.0)).collect::<Vec<_>>());
    let mut filters = Vec::new();
    let noise = 100.0 * (levels.iter().map(|level| (1.0 / f64::from(level - 1)).powi(2)).sum::<f64>() / 3.0).sqrt();
    if method != 0 && noise >= 0.1 {
        filters.push(native_filter(
            "filter.noise.addNoise",
            json!({"amount":noise,"distribution":"uniform","monochromatic":matches!(method,4|6|8|10),"seed":seed}),
        ));
    }
    let [red, green, blue] = curves;
    let parameters = if context.mode == ColorMode::Grayscale {
        let points = red.iter().zip(&green).zip(&blue).map(|((r, g), b)| r * 0.2126 + g * 0.7152 + b * 0.0722).collect::<Vec<_>>();
        json!({"pencil":true,"points":points})
    } else {
        json!({"pencil":true,"red":red,"green":green,"blue":blue})
    };
    filters.push(native_filter("image.adjustments.curves", parameters));
    if alpha != 256 {
        let values = (0..256).map(|i| ((f64::from(i) / 255.0 * f64::from(alpha - 1)).round() / f64::from(alpha - 1)) as f32).collect::<Vec<_>>();
        filters.push(native_filter("filter.other.alphaCurve", json!({"values":values})));
    }
    Ok(mapped(
        filters,
        format!(
            "Source RGB level counts become ordinary native256-sample pencil Curves; Gray uses a native Rec.709-weighted combination of channel transfer maps. Native piecewise-linear curve interpolation, document encoding and depth/HDR clipping apply; counts above256 are sampled approximations. Source dither method{method} uses {}. Native uniform pre-dither amount{noise}% derives from RMS quantizer steps: amount100 spans+/-0.5 and each desired half-step is0.5/(levels-1). Stored seed{seed} is used (native0 if absent); covariant nicks request native monochromatic noise. Floyd error propagation, Bayer/arithmetic threshold layouts and blue-noise spectra remain source metadata and are explicitly approximated by native stochastic dither. Source alpha-levels {alpha} becomes an ordinary native alphaCurve quantizer with256samples (default256 is native identity). Native piecewise-linear interpolation replaces hard source steps, with sampled approximations above256levels; alpha has no native stochastic pre-dither. Native depth and coverage clipping rules apply, while color/HDR channels remain independent",
            if method == 0 {
                "no pre-dither"
            } else if noise < 0.1 {
                "no forced noise because the derived amplitude is below native0.1% minimum"
            } else {
                "editable native seeded uniform Add Noise"
            }
        ),
    ))
}

fn drop_shadow(e: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping> {
    allowed(e, &["x", "y", "radius", "grow-shape", "grow-radius", "color", "opacity"])?;
    let x = number(e, "x", 20.0, -f64::MAX, f64::MAX)?;
    let y = number(e, "y", 20.0, -f64::MAX, f64::MAX)?;
    let radius = number(e, "radius", 10.0, 0.0, f64::MAX)?;
    let grow = number(e, "grow-radius", 0.0, -100.0, 100.0)?;
    let shape = choice(e, "grow-shape", 1, &["square", "circle", "diamond"])?;
    let opacity = number(e, "opacity", 0.5, 0.0, 1.0)?;
    // Two continuous boxes of width w have variance w²/6. This matches the
    // native tent's physical spread to source Gaussian sigma without copying a renderer.
    let blur = radius * 6.0_f64.sqrt();
    let size = blur + grow;
    let distance = x.hypot(y);
    let mut color = if e.argument("color").map_err(invalid)?.is_none() {
        super::resources::native_rgba([0.0, 0.0, 0.0, 1.0], context)?
    } else {
        super::resources::native_color(e, "color", context)?
    };
    let angle = if distance == 0.0 { 0.0 } else { y.atan2(-x).to_degrees() };
    // Native shadows use style opacity, rather than Color.alpha, for paint.
    let paint_opacity = opacity as f32 * color.alpha;
    color.alpha = 1.0;
    if paint_opacity == 0.0 {
        return Ok(NativeMapping {
            notes: vec!["Zero native shadow coverage preserves the editable original without generating invisible geometry".into()],
            ..Default::default()
        });
    }
    if grow < 0.0 || !size.is_finite() || size > 250.0 || distance > 30000.0 {
        use super::{NativeBranch, NativeStructure};
        if x.abs() > 1.0e10 || y.abs() > 1.0e10 {
            return Err("Shadow translation exceeds ordinary native placement bounds".into());
        }
        let count = if radius == 0.0 { 0.0 } else { (radius / 1000.0).powi(2).ceil().max(1.0) };
        if !count.is_finite() || count > (64 - usize::from(grow != 0.0)) as f64 {
            return Err("Native shadow blur exceeds the64-filter variance-decomposition resource limit".into());
        }
        let bytes = count as usize * 2048 + 12288;
        super::residual_spatial::generation_bytes(context, bytes)?;
        if bytes > context.remaining_metadata_bytes {
            return Err(MappingFailure {
                reason: photocraft_doc::import_conversion::ConversionReason::RetentionLimit,
                message: "Native shadow stages exceed actual remaining metadata allowance".into(),
            });
        }
        let mut filters = Vec::new();
        if grow != 0.0 {
            filters.push(native_filter(
                if grow < 0.0 { "filter.other.minimum" } else { "filter.other.maximum" },
                json!({"radius":grow.abs(),"preserve":if shape==0{"squareness"}else{"roundness"}}),
            ));
        }
        if count > 0.0 {
            let sigma = radius / count.sqrt();
            super::spatial::gaussian_workspace(context, sigma.max(0.1), 6)?;
            for _ in 0..count as usize {
                super::residual_spatial::generation_check(context)?;
                let mut filter = native_filter("filter.blur.gaussianBlur", json!({"radius":sigma.max(0.1)}));
                if sigma < 0.1 {
                    filter.opacity = (sigma / 0.1).powi(2) as f32;
                }
                filters.push(filter);
            }
        }
        let shadow = NativeBranch {
            transform: photocraft_geom::Affine::translate(x, y),
            filters,
            layer_effects: vec![Effect::ColorOverlay { common: FxCommon::new(BlendMode::Normal, 1.0), color }],
            opacity: paint_opacity,
            ..Default::default()
        };
        return Ok(NativeMapping {
            structure: Some(NativeStructure::Composite { branches: vec![shadow, NativeBranch::default()] }),
            notes: vec!["Independent editable native shadow copy uses translated owner data, Minimum/Maximum coverage morphology, variance-decomposed Gaussian and Color Overlay beneath the unchanged original. Negative growth is native erosion; large blur support is represented rather than capped to style250. Native round/square morphology replaces source circle/square/diamond, with diamond approximated by native roundness; native Free Transform, Gaussian/alpha boundaries, weak tiny-kernel strength and Normal overlap composition apply".into()],
            ..Default::default()
        });
    }
    let shadow = Shadow {
        common: FxCommon::new(BlendMode::Normal, paint_opacity),
        color,
        angle: angle as f32,
        use_global_light: false,
        distance: distance as f32,
        spread: if size == 0.0 { 0.0 } else { (grow / size) as f32 },
        size: size as f32,
        contour: Contour::Linear,
        anti_alias: false,
        noise: 0.0,
        knocks_out: true,
    };
    Ok(NativeMapping { layer_effects:vec![Effect::DropShadow(shadow)],notes:vec![
        "Source x/y becomes native shadow distance/light angle; native integer offset rounding applies. Gaussian sigma becomes equal-continuous-variance native tent width sigma√6; growth becomes native dilation/spread. Native chamfer growth replaces source square/circle/diamond metrics, and native knockout, color management, alpha and compositing apply".into()],..Default::default() })
}

/// Standard independent finite-difference kernels placed in native Custom's 5x5 grid.
fn custom(k: [f64; 9], scale: f64, offset: f64) -> SmartFilter {
    let mut kernel = [0.0; 25];
    for (i, value) in k.into_iter().enumerate() {
        if let Some(slot) = kernel.get_mut((i / 3 + 1) * 5 + i % 3 + 1) {
            *slot = value;
        }
    }
    native_filter("filter.other.custom", json!({"kernel":kernel.to_vec(),"scale":scale,"offset":offset}))
}

fn laplace(e: &StoredEffect<'_>) -> Result<NativeMapping> {
    allowed(e, &[])?;
    Ok(mapped(
        vec![custom([0.0, -1.0, 0.0, -1.0, 4.0, -1.0, 0.0, -1.0, 0.0], 1.0, 0.0)],
        "Source Laplace becomes an editable native four-neighbor Laplacian Custom kernel. Native signed range, channel encoding, edge repetition and alpha preservation replace source zero-crossing/normalization rules",
    ))
}

fn sobel(e: &StoredEffect<'_>, context: &MappingContext<'_>) -> Result<NativeMapping> {
    allowed(e, &["horizontal", "vertical", "keep-sign"])?;
    let horizontal = e.boolean("horizontal").map_err(invalid)?.unwrap_or(true);
    let vertical = e.boolean("vertical").map_err(invalid)?.unwrap_or(true);
    let signed = e.boolean("keep-sign").map_err(invalid)?.unwrap_or(true);
    let kernels = [[-1.0, 0.0, 1.0, -2.0, 0.0, 2.0, -1.0, 0.0, 1.0], [-1.0, -2.0, -1.0, 0.0, 0.0, 0.0, 1.0, 2.0, 1.0]];
    if horizontal && vertical {
        return Ok(mapped(
            vec![native_filter("filter.stylize.findEdges", json!({})), native_filter("image.adjustments.invert", json!({}))],
            "Both source Sobel directions become native Find Edges plus Invert, producing native positive vector magnitude. Native clamping, color encoding, alpha and boundary behavior apply; signed directional source values are retained as source metadata",
        ));
    }
    if !horizontal && !vertical {
        let neutral = if signed { photocraft_color::convert::linear_to_srgb(0.5) } else { 0.0 };
        let color = super::resources::native_rgba([neutral, neutral, neutral, 1.0], context)?;
        return Ok(NativeMapping {
            structure: Some(NativeStructure::Composite {
                branches: vec![NativeBranch {
                    layer_effects: vec![Effect::ColorOverlay { common: FxCommon::new(BlendMode::Normal, 1.0), color }],
                    ..Default::default()
                }],
            }),
            notes: vec![format!(
                "Independent stock Sobel observations with both directions disabled produce {}. This becomes an ordinary native Color Overlay converted through actual target ICC. Native owner alpha is preserved; the source border-alpha clearing quirk is replaced by native style coverage and editing rules",
                if signed { "half-linear-light signed neutral" } else { "black unsigned magnitude" }
            )],
            ..Default::default()
        });
    }
    let kernel = if horizontal { kernels[0] } else { kernels[1] };
    if signed {
        return Ok(mapped(
            vec![custom(kernel, 8.0, 127.5)],
            "One source Sobel direction becomes a native signed Custom derivative centered at50% and normalized to the native 0..1 editing range; source negative/HDR output is explicitly remapped, not clipped silently",
        ));
    }
    let outputs = (0..256).map(|i| ((f64::from(i) / 255.0 - 0.5) * 8.0).abs().min(1.0) * 255.0).collect::<Vec<_>>();
    Ok(mapped(
        vec![custom(kernel, 8.0, 127.5), native_filter("image.adjustments.curves", json!({"pencil":true,"points":outputs}))],
        "Unsigned directional Sobel becomes a native normalized Custom derivative and editable absolute-response pencil Curve; native 256-sample curve interpolation and channel/alpha rules apply",
    ))
}

fn edge(e: &StoredEffect<'_>) -> Result<NativeMapping> {
    allowed(e, &["algorithm", "amount", "border-behavior"])?;
    let algorithm = choice(e, "algorithm", 0, &["sobel", "prewitt", "gradient", "roberts", "differential", "laplace"])?;
    let amount = number(e, "amount", 2.0, 1.0, 10.0)?;
    choice(e, "border-behavior", 1, &["none", "clamp", "loop", "black", "white"])?;
    if algorithm == 0 {
        let outputs = (0..256).map(|i| ((1.0 - f64::from(i) / 255.0) * amount).min(1.0) * 255.0).collect::<Vec<_>>();
        return Ok(mapped(
            vec![native_filter("filter.stylize.findEdges", json!({})), native_filter("image.adjustments.curves", json!({"pencil":true,"points":outputs}))],
            "Sobel edge strength becomes native Find Edges and an editable inverted gain Curve using source amount. Native magnitude saturation, document encoding, alpha and repeated-edge boundaries replace the source border policy",
        ));
    }
    let pair = match algorithm {
        1 => [[-1.0, 0.0, 1.0, -1.0, 0.0, 1.0, -1.0, 0.0, 1.0], [-1.0, -1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0]],
        2 | 4 => [[0.0, 0.0, 0.0, -1.0, 0.0, 1.0, 0.0, 0.0, 0.0], [0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0]],
        3 => [[0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, -1.0], [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, -1.0, 0.0]],
        _ => [[0.0, -1.0, 0.0, -1.0, 4.0, -1.0, 0.0, -1.0, 0.0], [0.0; 9]],
    };
    let outputs = (0..256).map(|i| ((f64::from(i) / 255.0 - 0.5) * 8.0 * amount).abs().min(1.0) * 255.0).collect::<Vec<_>>();
    let count = if algorithm == 5 { 1 } else { 2 };
    let branches = pair
        .into_iter()
        .take(count)
        .map(|kernel| NativeBranch {
            filters: vec![custom(kernel, 8.0, 127.5), native_filter("image.adjustments.curves", json!({"pencil":true,"points":outputs}))],
            blend: BlendMode::LinearDodge,
            ..Default::default()
        })
        .collect();
    Ok(NativeMapping {structure:Some(NativeStructure::Composite {branches}),notes:vec![
        "Source edge algorithm becomes native standard finite-difference Custom kernels and gain/absolute pencil Curves. Two directions combine by native Add (L1 magnitude approximation); Gradient/Differential use centered differences. Source amount controls gain; native channel encoding, curve sampling, alpha, clamping and edge repetition apply".into()],..Default::default() })
}

fn distance(e: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping> {
    allowed(e, &["metric", "edge-handling", "threshold-lo", "threshold-hi", "averaging", "normalize"])?;
    let metric = choice(e, "metric", 0, &["euclidean", "manhattan", "chebyshev"])?;
    let edge = choice(e, "edge-handling", 1, &["above", "below"])?;
    let low = number(e, "threshold-lo", 0.0001, 0.0, 1.0)?;
    let high = number(e, "threshold-hi", 1.0, 0.0, 1.0)?;
    if high <= low {
        return Err(invalid("distance threshold-hi must exceed threshold-lo"));
    }
    let averaging = integer(e, "averaging", 0, 0, 1000)?;
    let normalize = e.boolean("normalize").map_err(invalid)?.unwrap_or(true);
    if !matches!(context.mode, ColorMode::Rgb | ColorMode::Grayscale) {
        return Err("Native distance morphology conversion requires RGB or Gray channels".into());
    }
    let width = f64::from(context.owner_bounds.width());
    let height = f64::from(context.owner_bounds.height());
    if width <= 0.0 || height <= 0.0 {
        return Err(invalid("distance owner bounds are empty"));
    }
    let radius = width.hypot(height).ceil().clamp(1.0, 31.0) as u32;
    // Bound the complete generated parameter set before constructing/cloning it.
    // JSON numbers and two-element point arrays have appreciable bookkeeping.
    let parameter_bytes = (radius as usize + 1).checked_mul(256 * 128 + 4096).ok_or_else(|| invalid("distance parameter allowance overflows"))?;
    if parameter_bytes > context.remaining_generation_bytes || parameter_bytes > context.remaining_metadata_bytes {
        return Err("Native distance branch parameters exceed the remaining generation allowance".into());
    }
    context.interrupt.check().map_err(|_| invalid("distance conversion cancelled"))?;
    let outputs = (0..256)
        .map(|i| {
            let ramp = ((f64::from(i) / 255.0 - low) / (high - low)).clamp(0.0, 1.0);
            let averaged = if averaging == 0 { ramp } else { (ramp * averaging as f64).round() / averaging as f64 };
            averaged * 255.0
        })
        .collect::<Vec<_>>();
    let weight = if normalize { 1.0 / f64::from(radius + 1) } else { 1.0 };
    let preserve = if metric == 0 { "roundness" } else { "squareness" };
    let mut branches = Vec::new();
    branches.try_reserve_exact(radius as usize + 1).map_err(|_| invalid("distance branch allocation failed"))?;
    for r in 0..=radius {
        context.interrupt.check().map_err(|_| invalid("distance conversion cancelled"))?;
        let mut filters = Vec::new();
        if context.mode == ColorMode::Rgb {
            filters.push(native_filter("image.adjustments.channelMixer", json!({"monochrome":true,"gray":[21.26,71.52,7.22,0]})));
        }
        filters.push(native_filter("image.adjustments.curves", json!({"pencil":true,"points":outputs})));
        if r != 0 {
            filters.push(native_filter("filter.other.minimum", json!({"radius":r,"preserve":preserve})));
        }
        // Scale color before Add: integer Add clips before opacity mixing.
        // Opaque branches also preserve opaque owner coverage.
        if normalize {
            filters.push(native_filter("image.adjustments.curves", json!({"points":[[0.0,0.0],[255.0,255.0*weight]]})));
        }
        branches.push(NativeBranch { filters, blend: BlendMode::LinearDodge, opacity: 1.0, ..Default::default() });
    }
    Ok(NativeMapping {
        structure: Some(NativeStructure::Composite { branches }),
        notes: vec![format!(
            "Source threshold interval becomes a native 256-sample pencil ramp after native Rec.709-weighted monochrome Channel Mixer where RGB, followed by erosion radii0..{radius} and native Add branches. Averaging{averaging} uses a monotone midpoint-threshold average quantized to that count (zero uses a continuous ramp); source threshold sampling is approximated. Normalize{normalize} uses fixed weight{weight}, with every opaque shell color-scaled by native Curves; raw distance units retain HDR sums only at native float depth, while integer documents clip by native Add rules. Opaque binary channels sum nested surviving erosion shells into a bounded distance-like response. Native roundness approximates Euclidean; square support represents Chebyshev and explicitly approximates Manhattan diamond support. Distances beyond radius{radius} saturate; normalization uses this fixed bound rather than a source-content maximum. Source edge-handling{edge} becomes native repeated-edge boundaries; signed/HDR inputs follow native Curves endpoint extrapolation in32-bit RGB and normalized limits in other modes/depths and native document-encoding luminance replaces source transfer/profile weights. Native Add branch alpha compositing approximates semitransparent source coverage"
        )],
        ..Default::default()
    })
}

/// Ordinary native Pattern resources, clipped to a finite editable vector extent.
fn generated_pattern(
    name: &str,
    width: u32,
    height: u32,
    context: &MappingContext<'_>,
    sample: impl FnMut(u32, u32) -> Result<[f32; 4]>,
    note: &str,
) -> Result<NativeMapping> {
    use photocraft_doc::{Path, Subpath, VectorMask};
    if context.remaining_metadata_bytes < 4096 {
        return Err("native pattern extent and fill metadata exceed remaining allowance".into());
    }
    let remaining = context.remaining_generation_bytes.checked_sub(4096).ok_or("native generator extent exceeds remaining allowance")?;
    let bounded = MappingContext {
        mode: context.mode,
        linear: context.linear,
        owner_bounds: context.owner_bounds,
        native_filter_bounds: context.native_filter_bounds,
        target_profile: context.target_profile.clone(),
        resolution_dpi: 72.0,
        remaining_generation_bytes: remaining,
        remaining_metadata_bytes: context.remaining_metadata_bytes,
        interrupt: context.interrupt,
    };
    let mut mapped = super::residual_tone::pattern_result(name, photocraft_geom::Size::new(width, height), (0.0, 0.0), 0.0, &bounded, sample, note)?;
    if let Some(NativeStructure::Replace { layer, .. }) = &mut mapped.structure {
        layer.vector_mask = Some(VectorMask::new(Path::new(vec![Subpath::polygon(&[
            (0.0, 0.0),
            (f64::from(width), 0.0),
            (f64::from(width), f64::from(height)),
            (0.0, f64::from(height)),
        ])])));
    }
    Ok(mapped)
}

fn diffraction(e: &StoredEffect<'_>, context: &MappingContext<'_>) -> Result<NativeMapping> {
    let frequency =
        [number(e, "red-frequency", 0.815, 0.0, 20.0)?, number(e, "green-frequency", 1.221, 0.0, 20.0)?, number(e, "blue-frequency", 1.123, 0.0, 20.0)?];
    let contours =
        [number(e, "red-contours", 0.821, 0.0, 10.0)?, number(e, "green-contours", 0.821, 0.0, 10.0)?, number(e, "blue-contours", 0.974, 0.0, 10.0)?];
    let edges = [number(e, "red-sedges", 0.610, 0.0, 1.0)?, number(e, "green-sedges", 0.677, 0.0, 1.0)?, number(e, "blue-sedges", 0.636, 0.0, 1.0)?];
    let brightness = number(e, "brightness", 0.066, 0.0, 1.0)?;
    let scattering = number(e, "scattering", 37.126, 0.0, 100.0)?;
    let polarization = number(e, "polarization", -0.473, -1.0, 1.0)?;
    let width = integer(e, "width", 200, 0, i32::MAX as u32)?;
    let height = integer(e, "height", 200, 0, i32::MAX as u32)?;
    let work = u64::from(width).checked_mul(u64::from(height)).and_then(|n| n.checked_mul(16)).ok_or("native interference work overflows")?;
    if work > 64_000_000 {
        return Err("native interference pattern exceeds 4 million pixels of fixed bounded wave work".into());
    }
    generated_pattern(
        "Native interference pattern",
        width,
        height,
        context,
        move |x, y| {
            let nx = (f64::from(x) + 0.5) / f64::from(width) - 0.5;
            let ny = (f64::from(y) + 0.5) / f64::from(height) - 0.5;
            let radial = nx.hypot(ny) * scattering / 10.0;
            let rgb = std::array::from_fn::<_, 3, _>(|i| {
                // Original two-plane-wave interference primitive, not a source renderer.
                let phase = std::f64::consts::TAU * frequency[i];
                let a = (phase * (nx + radial)).cos();
                let b = (phase * (ny - radial)).cos();
                let intensity = (0.5 + 0.25 * ((1.0 + polarization) * a + (1.0 - polarization) * b)).clamp(0.0, 1.0);
                let contour = if contours[i] == 0.0 { 1.0 } else { intensity.powf(contours[i]) };
                let smooth = contour * contour * (3.0 - 2.0 * contour);
                (brightness * ((1.0 - edges[i]) * contour + edges[i] * smooth)) as f32
            });
            Ok([rgb[0], rgb[1], rgb[2], 1.0])
        },
        "Source diffraction controls become an original two-plane-wave native interference Pattern, with independent RGB phase frequencies, contour exponents, smooth edge interpolation, radial scattering and polarization axis weights. Brightness scales nonlinear sRGB before native target ICC conversion. The finite stored extent is a native vector mask. This standard mathematical primitive approximates optical intent; source wave integration, scale, highlights, scattering law and exact contour response are not reproduced, and no source processing operator remains",
    )
}

fn fractal(e: &StoredEffect<'_>, context: &MappingContext<'_>) -> Result<NativeMapping> {
    let family =
        choice(e, "fractaltype", 0, &["mandelbrot", "julia", "barnsley-1", "barnsley-2", "barnsley-3", "spider", "man-o-war", "lambda", "sierpinski"])?;
    let iterations = integer(e, "iter", 50, 1, 1000)?;
    let zoom = number(e, "zoom", 300.0, 0.0000001, 10_000_000.0)?;
    let shiftx = number(e, "shiftx", 0.0, -f64::MAX, f64::MAX)?;
    let shifty = number(e, "shifty", 0.0, -f64::MAX, f64::MAX)?;
    let cx = number(e, "cx", -0.75, -2.5, 2.5)?;
    let cy = number(e, "cy", -0.2, -2.5, 2.5)?;
    let stretch = [number(e, "redstretch", 1.0, 0.0, 1.0)?, number(e, "greenstretch", 1.0, 0.0, 1.0)?, number(e, "bluestretch", 1.0, 0.0, 1.0)?];
    let modes = [
        choice(e, "redmode", 1, &["sine", "cosine", "none"])?,
        choice(e, "greenmode", 1, &["sine", "cosine", "none"])?,
        choice(e, "bluemode", 0, &["sine", "cosine", "none"])?,
    ];
    let inverse = [
        e.boolean("redinvert").map_err(invalid)?.unwrap_or(false),
        e.boolean("greeninvert").map_err(invalid)?.unwrap_or(false),
        e.boolean("blueinvert").map_err(invalid)?.unwrap_or(false),
    ];
    let colors = integer(e, "ncolors", 256, 2, 8192)?;
    let logarithmic = e.boolean("useloglog").map_err(invalid)?.unwrap_or(false);
    let width = context.native_filter_bounds.width();
    let height = context.native_filter_bounds.height();
    if u64::from(width).checked_mul(u64::from(height)).and_then(|n| n.checked_mul(u64::from(iterations))).is_none_or(|work| work > 64_000_000) {
        return Err("native fractal pixels times iterations exceeds 64 million bounded orbit steps".into());
    }
    // Validate complete domain endpoints before allocating the native resource.
    if [shiftx / zoom, shifty / zoom, (shiftx + f64::from(width)) / zoom, (shifty + f64::from(height)) / zoom].iter().any(|v| !v.is_finite()) {
        return Err(invalid("native fractal coordinate domain overflows"));
    }
    generated_pattern(
        "Native mathematical fractal",
        width,
        height,
        context,
        move |x, y| {
            let px = (f64::from(x) + 0.5 - f64::from(width) / 2.0 + shiftx) / zoom - 0.5;
            let py = (f64::from(y) + 0.5 - f64::from(height) / 2.0 + shifty) / zoom;
            if !px.is_finite() || !py.is_finite() {
                return Err(invalid("native fractal coordinate is nonfinite"));
            }
            let (mut zx, mut zy, mut cr, mut ci) = match family {
                0 => (0.0, 0.0, px, py),
                5 | 6 => (px + cx, py + cy, px, py),
                8 => (px + 0.5, py + 0.5, 0.0, 0.0),
                _ => (px, py, cx, cy),
            };
            let (mut previous_x, mut previous_y) = (px, py);
            let mut escaped = None;
            for k in 0..iterations {
                if k.is_multiple_of(32) {
                    context.interrupt.check().map_err(|_| invalid("native fractal generation cancelled"))?;
                }
                if family == 8 {
                    // Standard triangular inverse-IFS: retain the three corner
                    // half-scale triangles, reject the middle open triangle.
                    let child = if zx < 0.0 || zy < 0.0 || zx + zy > 1.0 {
                        None
                    } else if zx + zy <= 0.5 {
                        Some((2.0 * zx, 2.0 * zy))
                    } else if zx >= 0.5 {
                        Some((2.0 * zx - 1.0, 2.0 * zy))
                    } else if zy >= 0.5 {
                        Some((2.0 * zx, 2.0 * zy - 1.0))
                    } else {
                        None
                    };
                    let Some((next_x, next_y)) = child else {
                        escaped = Some((k + 1, 2.0));
                        break;
                    };
                    (zx, zy) = (next_x, next_y);
                    continue;
                }
                let (old_x, old_y) = (zx, zy);
                let (square_x, square_y) = (zx * zx - zy * zy, 2.0 * zx * zy);
                (zx, zy) = match family {
                    2 | 3 => {
                        let positive = if family == 2 { old_x >= 0.0 } else { old_x * ci + cr * old_y >= 0.0 };
                        let a = old_x - if positive { 1.0 } else { -1.0 };
                        (a * cr - old_y * ci, a * ci + old_y * cr)
                    }
                    4 => {
                        if old_x > 0.0 {
                            (square_x - 1.0, square_y)
                        } else {
                            (square_x - 1.0 + cr * old_x, square_y + ci * old_x)
                        }
                    }
                    7 => {
                        let (a, b) = (old_x - square_x, old_y - square_y);
                        (cr * a - ci * b, cr * b + ci * a)
                    }
                    6 => (square_x + previous_x + cr, square_y + previous_y + ci),
                    _ => (square_x + cr, square_y + ci),
                };
                if family == 5 {
                    (cr, ci) = (cr / 2.0 + zx, ci / 2.0 + zy);
                }
                (previous_x, previous_y) = (old_x, old_y);
                let magnitude = zx.hypot(zy);
                if !magnitude.is_finite() || magnitude > 2.0 {
                    escaped = Some((k + 1, magnitude));
                    break;
                }
            }
            let Some((count, magnitude)) = escaped else {
                return Ok([0.0, 0.0, 0.0, 1.0]);
            };
            let mut value = f64::from(count) / f64::from(iterations);
            if logarithmic && family == 8 {
                // Original native IFS palette policy: logarithmic escape depth.
                // No source-specific log-log orbit smoothing algorithm is retained.
                value = f64::from(count).ln_1p() / f64::from(iterations).ln_1p();
            } else if logarithmic && magnitude.is_finite() && magnitude > 1.0 {
                value = ((f64::from(count) + 1.0 - magnitude.ln().ln() / std::f64::consts::LN_2) / f64::from(iterations)).clamp(0.0, 1.0);
            }
            value = (value * f64::from(colors - 1)).round() / f64::from(colors - 1);
            let rgb = std::array::from_fn::<_, 3, _>(|i| {
                let v = match modes[i] {
                    0 => 0.5 + 0.5 * (std::f64::consts::TAU * value).sin(),
                    1 => 0.5 + 0.5 * (std::f64::consts::TAU * value).cos(),
                    _ => value,
                };
                (if inverse[i] { 1.0 - v * stretch[i] } else { v * stretch[i] }) as f32
            });
            Ok([rgb[0], rgb[1], rgb[2], 1.0])
        },
        "Source fractal intent becomes an independently authored standard Mandelbrot, Julia, Barnsley J1/J2/J3, Spider, Man-o-War, Lambda or deterministic Sierpinski inverse-IFS native Pattern. Native zoom means pixels per complex unit, shift is pixels, and the native domain is centered at (-0.5, 0). Julia uses actual cx/cy; Mandelbrot uses each pixel as c. Spider uses c0=pixel and c-next=c/2+z-next; Man-o-War adds the previous orbit value. For these two families cx/cy become native initial-orbit perturbations; Lambda uses cx/cy as its complex multiplier in lambda*z*(1-z). Barnsley families use actual cx/cy as the Julia-map c. Sierpinski uses a right triangle in native domain [-0.5, 0.5] with deterministic corner inverse maps; source CX/CY are declared inactive for this family and remain inactive. Its source log-log smoothing flag selects the explicit native logarithmic escape-depth palette ln(1+count)/ln(1+iteration budget), rather than source orbit-magnitude smoothing. These are documented native parameter policies, not assumed source equivalences. Escape radius 2, iteration budget, original sine/cosine/linear band palette, RGB stretch/inversion and optional smooth logarithmic count are converted once to ordinary native pixels and a finite vector extent. Source coordinate convention, family-specific coloring, interior palette and iteration response differ. No source fractal operator/runtime remains",
    )
}

fn fractal_trace(e: &StoredEffect<'_>, context: &MappingContext<'_>) -> Result<NativeMapping> {
    let family = choice(e, "fractal", 0, &["mandelbrot", "julia"])?;
    let x1 = number(e, "X1", -1.0, -50.0, 50.0)?;
    let x2 = number(e, "X2", 0.5, -50.0, 50.0)?;
    let y1 = number(e, "Y1", -1.0, -50.0, 50.0)?;
    let y2 = number(e, "Y2", 1.0, -50.0, 50.0)?;
    let jx = number(e, "JX", 0.5, -50.0, 50.0)?;
    let jy = number(e, "JY", 0.5, -50.0, 50.0)?;
    let depth = integer(e, "depth", 3, 1, 65536)?;
    let bailout = number(e, "bailout", 10000.0, 0.0, f64::MAX)?.sqrt();
    let edge = choice(e, "abyss-policy", 2, &["none", "clamp", "loop", "black", "white"])?;
    if x1 == x2 || y1 == y2 {
        return Err("Native fractal trace cannot normalize a zero-width complex domain".into());
    }
    let (width, height) = (context.native_filter_bounds.width(), context.native_filter_bounds.height());
    if width == 0 || height == 0 || width > crate::xcf::MAX_SIDE || height > crate::xcf::MAX_SIDE {
        return Err(invalid("native fractal trace owner bounds are invalid"));
    }
    let (mw, mh) = (width.min(256), height.min(256));
    let count =
        usize::try_from(mw).ok().and_then(|w| usize::try_from(mh).ok().and_then(|h| w.checked_mul(h))).ok_or("native fractal trace map size overflow")?;
    if (count as u64).checked_mul(u64::from(depth)).is_none_or(|work| work > 64_000_000) {
        return Err("native fractal trace map pixels times depth exceeds 64 million orbit steps".into());
    }
    // float RGB map, two byte planes, descriptor/raw/hex serialization copies
    // and editable JSON data coexist in the legacy native embed helper.
    let peak = count.checked_mul(64).and_then(|n| n.checked_add(65536)).ok_or("native fractal trace map allowance overflow")?;
    let serialized = count.checked_mul(4).and_then(|n| n.checked_add(65536)).ok_or("native fractal trace metadata allowance overflow")?;
    if peak > context.remaining_generation_bytes || serialized > context.remaining_metadata_bytes {
        return Err("native fractal trace raw/serialized map exceeds remaining generation or metadata allowance".into());
    }
    context.interrupt.check().map_err(|_| invalid("native fractal trace cancelled"))?;
    let mut data = Vec::new();
    data.try_reserve_exact(count.checked_mul(3).ok_or("native fractal trace RGB size overflow")?)
        .map_err(|_| "native fractal trace map allocation unavailable")?;
    // One full owner span is enough for in-domain samples, within native scale.
    let sx = f64::from(width).min(1278.72);
    let sy = f64::from(height).min(1278.72);
    for y in 0..mh {
        context.interrupt.check().map_err(|_| invalid("native fractal trace cancelled"))?;
        for x in 0..mw {
            let u = (f64::from(x) + 0.5) / f64::from(mw);
            let v = (f64::from(y) + 0.5) / f64::from(mh);
            let px = x1 + u * (x2 - x1);
            let py = y1 + v * (y2 - y1);
            let (mut zx, mut zy, cr, ci) = if family == 0 { (0.0, 0.0, px, py) } else { (px, py, jx, jy) };
            for k in 0..depth {
                if k.is_multiple_of(32) {
                    context.interrupt.check().map_err(|_| invalid("native fractal trace cancelled"))?;
                }
                let next = zx * zx - zy * zy + cr;
                let next_y = 2.0 * zx * zy + ci;
                if !next.is_finite() || !next_y.is_finite() {
                    return Err(invalid("native fractal trace orbit exceeds finite coordinate range"));
                }
                zx = next;
                zy = next_y;
                if zx.hypot(zy) > bailout {
                    break;
                }
            }
            let tx = (zx - x1) / (x2 - x1) * f64::from(width);
            let ty = (zy - y1) / (y2 - y1) * f64::from(height);
            let dx = tx - u * f64::from(width);
            let dy = ty - v * f64::from(height);
            if !dx.is_finite() || !dy.is_finite() {
                return Err(invalid("native fractal trace displacement overflows"));
            }
            data.extend_from_slice(&[(0.5 + 0.5 * (dx / sx).clamp(-1.0, 1.0)) as f32, (0.5 + 0.5 * (dy / sy).clamp(-1.0, 1.0)) as f32, 0.5]);
        }
    }
    let image = photocraft_algo::Image { rect: photocraft_geom::Rect::new(0, 0, mw as i32, mh as i32), ch: 3, data };
    let settings = json!({"horizontal":(sx*100.0/128.0).min(999.0),"vertical":(sy*100.0/128.0).min(999.0),"fit":"stretch","undefinedAreas":if edge==2 {"wrap"} else {"repeat"}});
    context.interrupt.check().map_err(|_| invalid("native fractal trace cancelled"))?;
    let params = crate::smart_map::displace_embed_map(&settings, &image).ok_or("native fractal coordinate map could not be embedded")?;
    context.interrupt.check().map_err(|_| invalid("native fractal trace cancelled"))?;
    Ok(one(
        "filter.distort.displace",
        params,
        format!(
            "Source quadratic trace intent becomes an independently authored standard Mandelbrot/Julia coordinate iteration converted once to an ordinary native Displace map, preserving the actual editable input. Actual domain, Julia constant, iteration depth and squared-radius bailout control the native coordinate map. Native map grid is at most 256 by 256, two 8-bit displacement planes with bilinear sampling, scales bounded to 999 percent (1278.72 pixels) and clamped offsets. Source abyss {edge} becomes {}; transparent/black/white edges are approximated by repeat. Native domain normalization, escape stopping, quantization, coordinate range, sampling and output bounds differ from source. The generated native map/settings survive native save and remain editable; no source runtime or source-rendered bitmap is used",
            if edge == 2 { "wrap" } else { "repeat" }
        ),
    ))
}

fn displace(e: &StoredEffect<'_>) -> Result<NativeMapping> {
    allowed(e, &["displace-mode", "sampler-type", "abyss-policy", "amount-x", "amount-y", "center", "center-x", "center-y"])?;
    let x = number(e, "amount-x", 0.0, -500.0, 500.0)?;
    let y = number(e, "amount-y", 0.0, -500.0, 500.0)?;
    choice(e, "displace-mode", 0, &["cartesian", "polar"])?;
    choice(e, "sampler-type", 2, &["nearest", "linear", "cubic", "nohalo", "lohalo"])?;
    choice(e, "abyss-policy", 1, &["none", "clamp", "loop", "black", "white"])?;
    e.boolean("center").map_err(invalid)?;
    number(e, "center-x", 0.5, -f64::MAX, f64::MAX)?;
    number(e, "center-y", 0.5, -f64::MAX, f64::MAX)?;
    if x == 0.0 && y == 0.0 {
        return Ok(mapped(vec![], "Both source displacement amounts are zero; preserve native pixels without looking up any map resource"));
    }
    Err(MappingFailure::resource(
        "Displace needs the actual serialized auxiliary map pixels and coordinate interpretation; a source locator cannot authorize file access",
    ))
}

fn cubism(e: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping> {
    allowed(e, &["tile-size", "tile-saturation", "bg-color", "seed"])?;
    let size = number(e, "tile-size", 10.0, 0.0, 256.0)?;
    let saturation = number(e, "tile-saturation", 2.5, 0.0, 10.0)?;
    let seed = match e.argument("seed").map_err(invalid)? {
        None => 0,
        Some(ArgumentValue::UInt(value)) => *value,
        _ => return Err(invalid("Cubism seed must be an unsigned stored integer")),
    };
    let background = if e.argument("bg-color").map_err(invalid)?.is_some() {
        super::resources::native_color(e, "bg-color", context)?
    } else {
        super::resources::native_rgba([0.0, 0.0, 0.0, 0.0], context)?
    };
    if size == 0.0 || saturation == 0.0 {
        use photocraft_doc::{Fill, Layer, LayerContent, Path, Subpath, VectorMask};
        let bounds = context.native_filter_bounds;
        if bounds.is_empty() || context.remaining_generation_bytes < 4096 || context.remaining_metadata_bytes < 4096 {
            return Err("native Cubism background extent exceeds owner/remaining allowance".into());
        }
        context.interrupt.check().map_err(|e| e.to_string())?;
        let mut layer = Layer::new("Native Cubism background", LayerContent::Fill(Fill::Solid(background)));
        layer.vector_mask = Some(VectorMask::new(Path::new(vec![Subpath::polygon(&[
            (f64::from(bounds.x0), f64::from(bounds.y0)),
            (f64::from(bounds.x1), f64::from(bounds.y0)),
            (f64::from(bounds.x1), f64::from(bounds.y1)),
            (f64::from(bounds.x0), f64::from(bounds.y1)),
        ])])));
        return Ok(NativeMapping{structure:Some(NativeStructure::Replace{layer:Box::new(layer),patterns:vec![]}),notes:vec!["Independent stock Cubism tile-size0 observations produce the actual background alone. Native expansion0 also chooses background alone because its editable tiles have zero area. Preserve these degenerate cases as an editable finite native Solid Fill, including its actual converted ICC color/alpha; absent default is transparent. No unrelated tile count or source pixel cache is substituted".into()],..Default::default()});
    }
    use photocraft_doc::{Fill, Path, Subpath, VectorMask};
    use photocraft_geom::{Affine, Rect};
    let bounds = context.native_filter_bounds;
    if bounds.is_empty() {
        return Err(invalid("Cubism owner bounds are empty"));
    }
    let nx = (f64::from(bounds.width()) / size).ceil();
    let ny = (f64::from(bounds.height()) / size).ceil();
    if !nx.is_finite() || !ny.is_finite() || nx > 63. || ny > 63. || nx * ny > 63. {
        return Err(MappingFailure::resource(
            "Native editable Cubism collage exceeds63 cells plus background; tiny/dense tiles remain retained rather than collapsing to a dialog tile count",
        ));
    }
    let count = (nx * ny) as usize;
    let metadata = count.checked_mul(12288).and_then(|n| n.checked_add(8192)).ok_or_else(|| MappingFailure::resource("Native Cubism metadata overflows"))?;
    if metadata > context.remaining_metadata_bytes {
        return Err(MappingFailure::resource("Native Cubism collage exceeds actual metadata allowance"));
    }
    super::residual_spatial::generation_bytes(context, metadata)?;
    let cache_bytes = |rect: Rect, caches: usize| -> Result<usize> {
        let tile = i64::from(photocraft_geom::TILE_SIZE);
        let columns = usize::try_from((i64::from(rect.x1) - 1).div_euclid(tile) - i64::from(rect.x0).div_euclid(tile) + 1)
            .map_err(|_| MappingFailure::resource("Native Cubism tile columns"))?;
        let rows = usize::try_from((i64::from(rect.y1) - 1).div_euclid(tile) - i64::from(rect.y0).div_euclid(tile) + 1)
            .map_err(|_| MappingFailure::resource("Native Cubism tile rows"))?;
        columns
            .checked_mul(rows)
            .and_then(|n| n.checked_mul(photocraft_geom::TILE_SIZE as usize * photocraft_geom::TILE_SIZE as usize))
            .and_then(|n| n.checked_mul((context.mode.color_channels() + 1) * 4))
            .and_then(|n| n.checked_mul(caches))
            .ok_or_else(|| MappingFailure::resource("Native Cubism cache storage overflows"))
    };
    let mut work = cache_bytes(bounds, count)?.checked_add(metadata).ok_or_else(|| MappingFailure::resource("Native Cubism owner cache storage overflows"))?;
    super::residual_spatial::generation_bytes(context, work)?;
    let mut branches = Vec::new();
    branches.try_reserve_exact(count + 1).map_err(|_| MappingFailure::resource("Native Cubism branches unavailable"))?;
    branches.push(NativeBranch { fill: Some(Fill::Solid(background)), ..Default::default() });
    let mut state = seed;
    for row in 0..ny as usize {
        for column in 0..nx as usize {
            context.interrupt.check().map_err(|e| e.to_string())?;
            let left = f64::from(bounds.x0) + column as f64 * size;
            let top = f64::from(bounds.y0) + row as f64 * size;
            let right = (left + size).min(f64::from(bounds.x1));
            let bottom = (top + size).min(f64::from(bounds.y1));
            let frame = Rect::new(left.floor() as i32, top.floor() as i32, right.ceil() as i32, bottom.ceil() as i32);
            if frame.is_empty() {
                return Err(invalid("Native Cubism cell has no representable sampling frame"));
            }
            let unit = |value: u32| f64::from(value) / f64::from(u32::MAX) - 0.5;
            let angle = unit(super::residual_spatial::random(&mut state)) * std::f64::consts::FRAC_PI_2;
            let jitter_x = unit(super::residual_spatial::random(&mut state)) * size * 0.5;
            let jitter_y = unit(super::residual_spatial::random(&mut state)) * size * 0.5;
            let center = (left + size / 2. + jitter_x, top + size / 2. + jitter_y);
            let scale_x = size * saturation / f64::from(frame.width());
            let scale_y = size * saturation / f64::from(frame.height());
            let transform = Affine::translate(center.0, center.1)
                .mul(&Affine::rotate(angle))
                .mul(&Affine { m: [scale_x, 0., 0., scale_y, 0., 0.] })
                .mul(&Affine::translate(-(f64::from(frame.x0) + f64::from(frame.width()) / 2.), -(f64::from(frame.y0) + f64::from(frame.height()) / 2.)));
            let corners = [(frame.x0, frame.y0), (frame.x1, frame.y0), (frame.x1, frame.y1), (frame.x0, frame.y1)]
                .map(|(x, y)| transform.apply(photocraft_geom::Point::new(f64::from(x), f64::from(y))));
            if corners.iter().any(|p| {
                !p.x.is_finite()
                    || !p.y.is_finite()
                    || p.x < f64::from(i32::MIN + 1)
                    || p.y < f64::from(i32::MIN + 1)
                    || p.x > f64::from(i32::MAX - 1)
                    || p.y > f64::from(i32::MAX - 1)
            }) {
                return Err(invalid("Native Cubism expanded tile exceeds finite native coordinates"));
            }
            let output = Rect::new(
                corners.iter().map(|p| p.x).fold(f64::INFINITY, f64::min).floor() as i32,
                corners.iter().map(|p| p.y).fold(f64::INFINITY, f64::min).floor() as i32,
                corners.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max).ceil() as i32,
                corners.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max).ceil() as i32,
            );
            let frame_bytes = cache_bytes(frame, 3)?;
            let output_bytes = cache_bytes(output, 2)?;
            work = work
                .checked_add(frame_bytes)
                .and_then(|n| n.checked_add(output_bytes))
                .and_then(|n| n.checked_add(65536))
                .ok_or_else(|| MappingFailure::resource("Native Cubism aggregate frame/cache storage overflows"))?;
            super::residual_spatial::generation_bytes(context, work)?;
            let path = Path::new(vec![Subpath::polygon(&corners.map(|p| (p.x, p.y)))]);
            branches.push(NativeBranch {
                filter_frame: Some(frame),
                filters: vec![native_filter("filter.blur.average", json!({}))],
                transform,
                vector_mask: Some(VectorMask::new(path)),
                ..Default::default()
            });
        }
    }
    Ok(NativeMapping {
        structure: Some(NativeStructure::Composite { branches }),
        notes: vec![format!(
            "Source Cubism becomes an ordinary editable native cell collage over the actual CMS-converted background. Exact floating tile-size {size}px supplies grid spacing with no native1..99 tile-count collapse; native integer sampling frames average each visible input cell. The public source property calls tile-saturation expansion: {saturation} directly scales tile side lengths, creating real native gaps/overlap rather than color saturation. Actual seed {seed} selects independently authored deterministic ±45degree rotations and ±quarter-cell jitter. Native rectangular topology, visible-cell alpha-weighted means, bilinear affine placement, source-over overlap/alpha, antialiasing and owner-canvas clipping replace source tessellation. Actual63-cell+background, aggregate native source/frame/paint cache, metadata and cancellation allowances bound work; no source Cubism evaluator or saved source pixels are substituted"
        )],
        ..Default::default()
    })
}

fn emboss(e: &StoredEffect<'_>, context: &MappingContext<'_>) -> Result<NativeMapping> {
    allowed(e, &["type", "azimuth", "elevation", "depth"])?;
    let kind = choice(e, "type", 0, &["emboss", "bumpmap"])?;
    let azimuth = number(e, "azimuth", 30.0, 0.0, 360.0)?;
    let elevation = number(e, "elevation", 45.0, 0.0, 180.0)?;
    let depth = integer(e, "depth", 20, 1, 100)?;
    if context.remaining_generation_bytes < 65536 || context.remaining_metadata_bytes < 65536 {
        return Err("native emboss curve parameters exceed remaining allowance".into());
    }
    context.interrupt.check().map_err(|e| e.to_string())?;
    let projected = elevation.to_radians().cos();
    let amount = (100.0 * projected.abs()).max(1.0);
    let residual = 100.0 * projected.abs() / amount;
    let angle = (azimuth + if projected < 0.0 { 180.0 } else { 0.0 } + 180.0).rem_euclid(360.0) - 180.0;
    // Standard flat Lambertian illumination is sin(elevation) in linear light.
    // Convert that original light baseline through the actual native target ICC.
    let illumination = photocraft_color::convert::linear_to_srgb(elevation.to_radians().sin().max(0.0) as f32);
    let color = super::resources::native_rgba([illumination, illumination, illumination, 1.0], context)?;
    let [red, green, blue] = color.to_rgb();
    let response = |base: f32| (0..256).map(|i| ((f64::from(i) / 255.0 - 0.5) * residual + f64::from(base)).clamp(0.0, 1.0) * 255.0).collect::<Vec<_>>();
    let curve = if context.mode == ColorMode::Grayscale {
        json!({"pencil":true,"points":response(red)})
    } else {
        json!({"pencil":true,"red":response(red),"green":response(green),"blue":response(blue)})
    };
    let relief =
        vec![native_filter("filter.stylize.emboss", json!({"angle":angle,"height":depth,"amount":amount})), native_filter("image.adjustments.curves", curve)];
    let note = format!(
        "Source azimuth and declared depth become native Emboss direction and height {depth}px. Elevation {elevation} degrees becomes an original Lambertian light projection: native relief amount 100*abs(cos elevation), azimuth reversed for negative cosine, and an editable native Curve shifts the flat 50% relief baseline to sin elevation through actual native target ICC. Native amount minimum1 is compensated by residual Curve gain near overhead light. Source flat-light observations establish the zero/peak orientation; native derivative, saturation, illumination response, working-space curves and quantization still approximate source lighting"
    );
    if kind == 0 {
        return Ok(mapped(relief, note));
    }
    Ok(NativeMapping {
        structure: Some(NativeStructure::Composite {
            branches: vec![NativeBranch::default(), NativeBranch { filters: relief, blend: BlendMode::Overlay, ..Default::default() }],
        }),
        notes: vec![format!("{note}. Bumpmap keeps editable original color below native Overlay relief; native alpha/overlay replace source surface lighting")],
        ..Default::default()
    })
}

fn neon(e: &StoredEffect<'_>) -> Result<NativeMapping> {
    allowed(e, &["radius", "amount"])?;
    let radius = number(e, "radius", 5.0, 0.0, 1500.0)?;
    let amount = number(e, "amount", 0.0, 0.0, 100.0)?;
    let mut filters = Vec::new();
    if radius > 0.0 {
        let passes = ((radius / 1000.0).powi(2).ceil() as u32).max(1);
        let sigma = (radius / f64::from(passes).sqrt()).max(0.1);
        filters.extend((0..passes).map(|_| native_filter("filter.blur.gaussianBlur", json!({"radius":sigma}))));
    }
    filters.push(native_filter("filter.stylize.findEdges", json!({})));
    filters.push(native_filter("image.adjustments.invert", json!({})));
    let mut remaining = 1.0 + 9.0 * amount;
    while remaining > 1.0 + f64::EPSILON {
        let gain = remaining.min(2.0);
        filters.push(native_filter("image.adjustments.channelMixer", json!({"red":[100.0*gain,0,0,0],"green":[0,100.0*gain,0,0],"blue":[0,0,100.0*gain,0]})));
        remaining /= gain;
    }
    Ok(mapped(
        filters,
        format!(
            "Source radius{radius}px becomes native Gaussian edge scale followed by native Find Edges and Invert, producing a black-background editable edge response. Native gradient, transfer, alpha and boundary rules apply. Source intensity {amount} becomes observed relative gain 1 + 9*amount through at most10 ordinary native Channel Mixer passes, each coefficient at most200percent. Independent stock step probes establish that relative strength, not identical edge magnitudes. Native integer clipping, float coefficient quantization, alpha and working-channel rules replace source response"
        ),
    ))
}

fn engrave(e: &StoredEffect<'_>) -> Result<NativeMapping> {
    allowed(e, &["row-height", "limit"])?;
    let height = integer(e, "row-height", 10, 2, 16)?;
    let limit = e.boolean("limit").map_err(invalid)?.unwrap_or(false);
    let size = ((f64::from(height) - 1.5) / 2.0).clamp(1.0, 12.0);
    let mut filters = Vec::new();
    if limit {
        filters.push(native_filter("image.adjustments.levels", json!({"outBlack":255.0/f64::from(height),"outWhite":255.0*(1.0-1.0/f64::from(height))})));
    }
    filters.push(native_filter(
        "filter.gallery.halftonePattern",
        json!({"size":size,"contrast":50,"patternType":"line","foreground":[0,0,0,1],"background":[1,1,1,1]}),
    ));
    Ok(mapped(
        filters,
        format!(
            "Source Engrave becomes native Halftone Pattern horizontal tone-adaptive ink lines, source row-height {height}px mapped by native spacing1.5+2*size (minimum3.5px). Limit {limit} selects native pre-Levels output endpoints 1/height and 1-1/height, retaining at least one light/dark stripe fraction; independent flat-source observations establish that endpoint policy. Source heights below2 are invalid; at height2 the limiting endpoints coincide at half tone. Native crisp ink contrast50, prefilter, phase, line antialias and alpha rules approximate the source line screen"
        ),
    ))
}

fn fattal(e: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping> {
    allowed(e, &["alpha", "beta", "saturation", "noise"])?;
    let alpha = number(e, "alpha", 1.0, 0.0, 2.0)?;
    let beta = number(e, "beta", 0.9, 0.1, 2.0)?;
    let saturation = number(e, "saturation", 0.8, 0.0, 1.0)?;
    let noise = number(e, "noise", 0.0, 0.0, 1.0)?;
    if !matches!(context.mode, ColorMode::Rgb | ColorMode::Grayscale) {
        return Err("Native global tone approximation requires RGB or Gray channels".into());
    }
    if context.remaining_generation_bytes < 32768 || context.remaining_metadata_bytes < 32768 {
        return Err("native Fattal tone parameters exceed remaining allowance".into());
    }
    // An independent global power curve has constant logarithmic slope beta.
    // It preserves the source contrast-compression intent, not its spatial solver.
    let outputs = (0..256).map(|i| (f64::from(i) / 255.0).powf(beta) * 255.0).collect::<Vec<_>>();
    let contrast = |gradient: f64| 255.0 * (1.0 - (-gradient).exp());
    let mut filters = Vec::new();
    if noise > 0.0 {
        filters.push(native_filter("filter.blur.surfaceBlur", json!({"radius":1.0,"threshold":contrast(noise).max(2.0)})));
    }
    if beta < 1.0 {
        let desired = 100.0 * (1.0 - beta);
        let amount = desired.max(1.0);
        let mut detail = native_filter("filter.sharpen.unsharpMask", json!({"amount":amount,"radius":1.0,"threshold":contrast(alpha)}));
        detail.opacity = (desired / amount) as f32;
        filters.push(detail);
    } else if beta > 1.0 {
        let mut detail = native_filter("filter.blur.surfaceBlur", json!({"radius":1.0,"threshold":contrast(alpha).max(2.0)}));
        detail.opacity = (beta - 1.0).min(1.0) as f32;
        filters.push(detail);
    }
    filters.push(native_filter("image.adjustments.curves", json!({"pencil":true,"points":outputs})));
    if context.mode == ColorMode::Rgb {
        filters.push(native_filter("image.adjustments.hueSaturation", json!({"saturation":(saturation-1.0)*100.0})));
    }
    Ok(mapped(
        filters,
        format!(
            "Source beta{beta} contrast compression becomes an ordinary native global power pencil Curve with logarithmic slope beta; source saturation{saturation} becomes native Hue/Saturation where RGB. Native bounded curve range, encoding, interpolation and channel rules apply. Source alpha {alpha} and noise {noise} log-gradient thresholds use an explicit unit-reference contrast policy: fraction 1-exp(-g), scaled to native 0..255 contrast levels, not an assertion of equal units. Beta below1 adds native radius1 Unsharp detail with alpha threshold and amount100*(1-beta); native minimum amount1 is weighted by fractional filter opacity. Beta above1 selects native radius1 Surface Blur with alpha threshold and opacitymin(beta-1,1). Positive noise adds native radius1 edge-preserving Surface Blur before detail; noise0 adds no protection filter. Native Surface Blur minimum threshold2/255 is explicit. The finest-native-neighborhood and reference-contrast choices approximate local detail/noise protection; this does not reproduce source gradient-domain reconstruction or HDR range"
        ),
    ))
}

pub(super) fn map(e: &StoredEffect<'_>, context: &MappingContext) -> MappingResult {
    let result = match e.canonical_operation() {
        "gegl:deinterlace" => deinterlace(e, context),
        "gegl:dither" => dither(e, context),
        "gegl:dropshadow" => drop_shadow(e, context),
        "gegl:edge" => edge(e),
        "gegl:edge-laplace" => laplace(e),
        "gegl:edge-sobel" => sobel(e, context),
        "gegl:displace" => displace(e),
        "gegl:cubism" => cubism(e, context),
        "gegl:diffraction-patterns" => diffraction(e, context),
        "gegl:distance-transform" => distance(e, context),
        "gegl:edge-neon" => neon(e),
        "gegl:emboss" => emboss(e, context),
        "gegl:engrave" => engrave(e),
        "gegl:fattal02" => fattal(e, context),
        "gegl:fractal-explorer" => fractal(e, context),
        "gegl:fractal-trace" => fractal_trace(e, context),
        _ => return None,
    };
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::super::Argument;
    use super::*;
    use photocraft_geom::Rect;
    fn context() -> MappingContext<'static> {
        MappingContext {
            mode: ColorMode::Rgb,
            linear: false,
            owner_bounds: Rect::new(-2, 3, 10, 12),
            native_filter_bounds: Rect::new(0, 0, 12, 9),
            target_profile: None,
            resolution_dpi: 72.0,
            remaining_generation_bytes: crate::xcf::MAX_PIXEL_BYTES,
            remaining_metadata_bytes: crate::xcf::MAX_METADATA_BYTES,
            interrupt: photocraft_raster::Interrupt::NONE,
        }
    }
    fn arg(name: &'static str, value: ArgumentValue<'static>) -> Argument<'static> {
        Argument { name, value }
    }
    fn run(operation: &'static str, args: Vec<Argument<'static>>) -> Result<NativeMapping> {
        map(&StoredEffect::test(operation, args), &context()).unwrap()
    }

    #[test]
    fn deinterlace_keeps_source_local_fields_and_rotates_vertical() {
        let a = run("gegl:deinterlace", vec![]).unwrap();
        assert_eq!(a.filters[0].params["eliminate"], "oddFields");
        let b = run("gegl:deinterlace", vec![arg("keep", ArgumentValue::Enum(1)), arg("orientation", ArgumentValue::String("vertical"))]).unwrap();
        assert_eq!(b.filters[0].params["eliminate"], "evenFields");
        assert_eq!(b.quarter_turns, 1);
        assert!(run("gegl:deinterlace", vec![arg("size", ArgumentValue::Int(0))]).is_err());
    }
    #[test]
    fn extended_field_support_is_native_offset_and_single_field_masks_with_bounds() {
        let a = run("gegl:deinterlace", vec![arg("size", ArgumentValue::Int(3))]).unwrap();
        let Some(NativeStructure::Composite { branches }) = a.structure else { panic!("native support interpolation") };
        assert_eq!(branches.len(), 3);
        assert_eq!(branches[1].filters[0].params["vertical"], 3);
        assert_eq!(branches[2].filters[0].params["vertical"], -3);
        assert_eq!(branches[2].opacity, 0.5);
        let mask = branches[1].vector_mask.as_ref().unwrap();
        assert_eq!(mask.path.subpaths.len(), 5);
        for (index, subpath) in mask.path.subpaths.iter().enumerate() {
            assert_eq!(subpath.knots[0].anchor.y, (index * 2) as f64);
            assert_eq!(subpath.knots[2].anchor.y, (index * 2 + 1) as f64);
        }
        let a =
            run("gegl:deinterlace", vec![arg("size", ArgumentValue::Int(2)), arg("orientation", ArgumentValue::Enum(1)), arg("keep", ArgumentValue::Enum(1))])
                .unwrap();
        let Some(NativeStructure::Composite { branches }) = a.structure else { panic!("native vertical support") };
        assert_eq!(branches[1].filters[0].params["horizontal"], 2);
        assert_eq!(branches[1].vector_mask.as_ref().unwrap().path.subpaths[0].knots[0].anchor.x, 1.0);
        let effect = StoredEffect::test("gegl:deinterlace", vec![arg("size", ArgumentValue::Int(2))]);
        let mut c = context();
        c.remaining_generation_bytes = 0;
        assert!(map(&effect, &c).unwrap().is_err());
        c = context();
        c.native_filter_bounds = Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX);
        assert!(map(&effect, &c).unwrap().is_err());
        let cancelled = || true;
        c = context();
        c.interrupt = photocraft_raster::Interrupt::cancel_only(&cancelled);
        assert!(map(&effect, &c).unwrap().is_err());
    }
    #[test]
    fn quantization_preserves_unequal_channel_counts_and_endpoints() {
        let alias = StoredEffect::test("gegl:color-reduction", vec![arg("dither-method", ArgumentValue::Enum(0))]);
        assert_eq!(map(&alias, &context()).unwrap().unwrap().filters[0].command, "image.adjustments.curves");
        assert_eq!(alias.operation, "gegl:color-reduction");
        let a = run(
            "gegl:dither",
            vec![arg("dither-method", ArgumentValue::Enum(0)), arg("red-levels", ArgumentValue::Int(2)), arg("green-levels", ArgumentValue::Int(3))],
        )
        .unwrap();
        assert_eq!(a.filters[0].params["red"][100], 0.0);
        assert_eq!(a.filters[0].params["green"][100], 127.5);
        assert_eq!(a.filters[0].params["red"][255], 255.0);
        assert_eq!(run("gegl:dither", vec![]).unwrap().filters[0].command, "filter.noise.addNoise");
        assert!(
            run("gegl:dither", vec![arg("dither-method", ArgumentValue::Enum(0)), arg("alpha-levels", ArgumentValue::Int(2))]).unwrap().notes[0]
                .contains("alpha-levels 2 becomes an ordinary native alphaCurve")
        );
    }

    #[test]
    fn alpha_quantization_uses_native_coverage_at_each_depth_without_touching_color() {
        use photocraft_color::{PixelFormat, SampleType};
        use photocraft_raster::{Interrupt, Surface};
        let mapping = run("gegl:dither", vec![arg("dither-method", ArgumentValue::Enum(0)), arg("alpha-levels", ArgumentValue::Int(2))]).unwrap();
        let alpha = mapping.filters.iter().find(|f| f.command == "filter.other.alphaCurve").unwrap();
        let actual_values = alpha.params["values"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect::<Vec<_>>();
        let expected_values = (0..256).map(|i| if i < 128 { 0.0 } else { 1.0 }).collect::<Vec<_>>();
        let bounds = Rect::new(0, 0, 9, 1);
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut source = Surface::new(PixelFormat::new(ColorMode::Rgb, depth, true));
            source.write_region(bounds, &(0..9).flat_map(|i| [0.25, 0.5, 1.5, i as f32 / 8.0]).collect::<Vec<_>>());
            let apply = |values| {
                photocraft_algo::apply_in_with(&source, &photocraft_algo::FilterParams::AlphaCurve { values }, bounds, bounds, None, bounds, &Interrupt::NONE)
                    .unwrap()
            };
            let actual = apply(actual_values.clone());
            let expected = apply(expected_values.clone());
            assert_eq!(actual.read_region(bounds), expected.read_region(bounds));
            for x in 0..9 {
                assert_eq!(&actual.pixel(x, 0)[..3], &source.pixel(x, 0)[..3]);
            }
            assert_eq!(actual.pixel(2, 0)[3], 0.0);
            assert_eq!(actual.pixel(6, 0)[3], 1.0);
            if depth == SampleType::F32 {
                assert!((actual.pixel(4, 0)[3] - 0.5).abs() < 1e-6);
                assert_eq!(actual.pixel(4, 0)[2], 1.5);
            }
        }
        let high = run("gegl:dither", vec![arg("alpha-levels", ArgumentValue::Int(65536))]).unwrap();
        assert_eq!(high.filters.last().unwrap().params["values"].as_array().unwrap().len(), 256);
    }
    #[test]
    fn seeded_native_dither_matches_an_independent_native_counterpart_at_all_depths() {
        use photocraft_color::{PixelFormat, SampleType};
        use photocraft_doc::Adjustment;
        use photocraft_doc::adjust::{CurvePoint, ToneSpace};
        use photocraft_raster::Surface;
        let mapping = run(
            "gegl:dither",
            vec![
                arg("dither-method", ArgumentValue::Enum(3)),
                arg("seed", ArgumentValue::UInt(7)),
                arg("red-levels", ArgumentValue::Int(2)),
                arg("green-levels", ArgumentValue::Int(2)),
                arg("blue-levels", ArgumentValue::Int(2)),
            ],
        )
        .unwrap();
        assert_eq!(mapping.filters[0].params["amount"], 100.0);
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let bounds = Rect::new(0, 0, 9, 1);
            let format = PixelFormat::new(ColorMode::Rgb, depth, true);
            let mut original = Surface::new(format);
            original.write_region(bounds, &(0..9).flat_map(|i| [i as f32 / 8.0, 0.375, 0.625, if i % 2 == 0 { 0.5 } else { 1.0 }]).collect::<Vec<_>>());
            let apply = |amount: f32, seed: u32, curves: [Vec<CurvePoint>; 3]| {
                let noisy = photocraft_algo::apply(
                    &original,
                    &photocraft_algo::FilterParams::AddNoise { amount, distribution: photocraft_algo::Distribution::Uniform, monochromatic: false, seed },
                    bounds,
                    bounds,
                    None,
                );
                let samples = noisy.read_region(bounds);
                let mut buffer = photocraft_compose::Buffer { rect: bounds, px: samples.as_chunks::<4>().0.iter().map(|p| [p[0], p[1], p[2], p[3]]).collect() };
                photocraft_compose::adjust::apply_in(
                    &Adjustment::Curves { master: Vec::new(), per_channel: curves, space: ToneSpace::Rgb, black: Vec::new(), pencil: true },
                    &mut buffer,
                    photocraft_compose::adjust::Transfer::for_document(ColorMode::Rgb, depth),
                    Some(depth),
                    ColorMode::Rgb,
                );
                let mut output = Surface::new(format);
                output.write_region(bounds, &buffer.px.iter().flatten().copied().collect::<Vec<_>>());
                output
            };
            // Independent native two-level pencil counterpart: first128 entries
            // black, last128 white. Native interpolation across the midpoint remains.
            let counterpart =
                std::array::from_fn(|_| (0..256).map(|i| CurvePoint { input: i as f32 / 255.0, output: if i < 128 { 0.0 } else { 1.0 } }).collect());
            let expected = apply(100.0, 7, counterpart);
            let actual_curves = ["red", "green", "blue"].map(|name| {
                mapping.filters[1].params[name]
                    .as_array()
                    .unwrap()
                    .iter()
                    .enumerate()
                    .map(|(i, value)| CurvePoint { input: i as f32 / 255.0, output: value.as_f64().unwrap() as f32 / 255.0 })
                    .collect()
            });
            let actual =
                apply(mapping.filters[0].params["amount"].as_f64().unwrap() as f32, mapping.filters[0].params["seed"].as_u64().unwrap() as u32, actual_curves);
            assert_eq!(actual, expected, "ordinary native seeded noise/quantization at {depth:?}");
            for x in 0..9 {
                assert_eq!(actual.pixel(x, 0)[3], original.pixel(x, 0)[3]);
            }
        }
        let low = run(
            "gegl:dither",
            vec![arg("red-levels", ArgumentValue::Int(65536)), arg("green-levels", ArgumentValue::Int(65536)), arg("blue-levels", ArgumentValue::Int(65536))],
        )
        .unwrap();
        assert_eq!(low.filters.len(), 1);
        assert!(low.notes[0].contains("below native0.1% minimum"));
        let covariant = run("gegl:dither", vec![arg("dither-method", ArgumentValue::Enum(4))]).unwrap();
        assert_eq!(covariant.filters[0].params["monochromatic"], true);
    }
    #[test]
    fn shadow_geometry_uses_light_angle_and_variance_not_defaults() {
        let a = run(
            "gegl:dropshadow",
            vec![
                arg("x", ArgumentValue::Float(-3.0)),
                arg("y", ArgumentValue::Float(4.0)),
                arg("radius", ArgumentValue::Float(2.0)),
                arg("grow-radius", ArgumentValue::Float(1.0)),
            ],
        )
        .unwrap();
        let Effect::DropShadow(s) = &a.layer_effects[0] else { panic!("expected native shadow") };
        assert!((s.distance - 5.0).abs() < 1e-5);
        assert!((s.angle - 53.1301).abs() < 1e-3);
        assert!((f64::from(s.size) * (1.0 - f64::from(s.spread)) - 2.0 * 6.0_f64.sqrt()).abs() < 1e-5);
        assert_eq!(s.common.blend, BlendMode::Normal);
        let negative = run("gegl:dropshadow", vec![arg("grow-radius", ArgumentValue::Float(-1.0))]).unwrap();
        let Some(super::super::NativeStructure::Composite { branches }) = negative.structure else { panic!("native erosion shadow") };
        assert_eq!(branches[0].filters[0].command, "filter.other.minimum");
        assert_eq!(branches[0].filters[0].params["radius"], json!(1.0));
        assert_eq!(branches.len(), 2);
        let large = run("gegl:dropshadow", vec![arg("radius", ArgumentValue::Float(1500.0)), arg("x", ArgumentValue::Float(50000.0))]).unwrap();
        let Some(super::super::NativeStructure::Composite { branches }) = large.structure else { panic!("native broad shadow") };
        assert_eq!(branches[0].transform, photocraft_geom::Affine::translate(50000.0, 20.0));
        assert_eq!(branches[0].filters.len(), 3);
        let variance: f64 = branches[0].filters.iter().map(|f| f.params["radius"].as_f64().unwrap().powi(2)).sum();
        assert!((variance - 2250000.0).abs() < 1e-6);
    }
    #[test]
    fn standard_kernel_is_centered_row_major_and_zero_sum() {
        let a = run("gegl:edge-laplace", vec![]).unwrap();
        let k = a.filters[0].params["kernel"].as_array().unwrap();
        assert_eq!(k.len(), 25);
        assert_eq!(k[12], 4.0);
        assert_eq!(k[7], -1.0);
        assert_eq!(k[0], 0.0);
        assert_eq!(k.iter().map(|v| v.as_f64().unwrap()).sum::<f64>(), 0.0);
    }
    #[test]
    fn directional_sobel_and_prewitt_have_editable_native_constructions() {
        let a = run("gegl:edge-sobel", vec![arg("vertical", ArgumentValue::Bool(false)), arg("keep-sign", ArgumentValue::Bool(false))]).unwrap();
        assert_eq!(a.filters.len(), 2);
        assert_eq!(a.filters[1].params["points"].as_array().unwrap().len(), 256);
        let a = run("gegl:edge", vec![arg("algorithm", ArgumentValue::Enum(1)), arg("amount", ArgumentValue::Float(3.0))]).unwrap();
        let Some(NativeStructure::Composite { branches }) = a.structure else { panic!("native branches") };
        assert_eq!(branches.len(), 2);
        assert_eq!(branches[0].blend, BlendMode::LinearDodge);
    }
    #[test]
    fn distance_shells_render_native_depth_alpha_and_hdr_rules() {
        use photocraft_color::{PixelFormat, SampleType};
        use photocraft_doc::adjust::{CurvePoint, ToneSpace};
        use photocraft_doc::{Adjustment, Document, Layer, LayerContent};
        use photocraft_geom::Size;
        use photocraft_raster::Surface;
        let bounds = Rect::new(0, 0, 5, 5);
        let mut ctx = context();
        ctx.owner_bounds = bounds;
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for normalize in [false, true] {
                let a = map(
                    &StoredEffect::test(
                        "gegl:distance-transform",
                        vec![arg("metric", ArgumentValue::Enum(2)), arg("normalize", ArgumentValue::Bool(normalize)), arg("averaging", ArgumentValue::Int(4))],
                    ),
                    &ctx,
                )
                .unwrap()
                .unwrap();
                let Some(NativeStructure::Composite { branches }) = a.structure else { panic!("erosion branches") };
                assert_eq!(branches.len(), 9);
                assert_eq!(branches[0].opacity, 1.0);
                assert_eq!(branches[0].filters[1].params["points"][100], 127.5);
                let format = PixelFormat::new(ColorMode::Rgb, depth, true);
                let mut source = Surface::new(format);
                source.write_region(
                    bounds,
                    &(0..5)
                        .flat_map(|y| {
                            (0..5).flat_map(move |x| {
                                // Signed/HDR source endpoints pass through ordinary native
                                // Curves; integer storage already clips them by native rules.
                                let v = if (1..=3).contains(&x) && (1..=3).contains(&y) { 2.0 } else { -1.0 };
                                [v, v, v, 1.0]
                            })
                        })
                        .collect::<Vec<_>>(),
                );
                let mut native = Document::new("Native distance counterpart", Size::new(5, 5), ColorMode::Rgb, depth);
                for branch in branches {
                    let mut filtered = source.clone();
                    for filter in branch.filters {
                        if filter.command == "image.adjustments.curves" {
                            let points = filter.params["points"].as_array().unwrap();
                            let pencil = filter.params["pencil"].as_bool().unwrap_or(false);
                            let master = points
                                .iter()
                                .enumerate()
                                .map(|(i, p)| {
                                    if let Some(pair) = p.as_array() {
                                        CurvePoint { input: pair[0].as_f64().unwrap() as f32 / 255.0, output: pair[1].as_f64().unwrap() as f32 / 255.0 }
                                    } else {
                                        CurvePoint { input: i as f32 / 255.0, output: p.as_f64().unwrap() as f32 / 255.0 }
                                    }
                                })
                                .collect();
                            let mut buffer = photocraft_compose::Buffer {
                                rect: bounds,
                                px: filtered.read_region(bounds).as_chunks::<4>().0.iter().map(|p| [p[0], p[1], p[2], p[3]]).collect(),
                            };
                            photocraft_compose::adjust::apply_in(
                                &Adjustment::Curves {
                                    master,
                                    per_channel: std::array::from_fn(|_| Vec::new()),
                                    space: ToneSpace::Rgb,
                                    black: Vec::new(),
                                    pencil,
                                },
                                &mut buffer,
                                photocraft_compose::adjust::Transfer::for_document(ColorMode::Rgb, depth),
                                Some(depth),
                                ColorMode::Rgb,
                            );
                            filtered.write_region(bounds, &buffer.px.iter().flatten().copied().collect::<Vec<_>>());
                        } else if filter.command == "filter.other.minimum" {
                            filtered = photocraft_algo::apply(
                                &filtered,
                                &photocraft_algo::FilterParams::Minimum {
                                    radius: filter.params["radius"].as_f64().unwrap() as f32,
                                    preserve: photocraft_algo::Preserve::Squareness,
                                },
                                bounds,
                                bounds,
                                None,
                            );
                        } else {
                            assert_eq!(filter.command, "image.adjustments.channelMixer");
                        } // Equal RGB already monochrome.
                    }
                    let mut layer = Layer::new("Native erosion shell", LayerContent::Raster(filtered));
                    layer.blend = branch.blend;
                    layer.opacity = branch.opacity;
                    native.layers.push(layer);
                }
                let actual = photocraft_compose::flatten(&native);
                let expected_center = if normalize {
                    2.0 / 9.0
                } else if depth == SampleType::F32 {
                    2.0
                } else {
                    1.0
                };
                let expected_edge = if normalize { 1.0 / 9.0 } else { 1.0 };
                let tolerance = if depth == SampleType::U8 { 0.01 } else { 0.0001 };
                assert!((actual.px[12][0] - expected_center).abs() < tolerance, "{depth:?} normalized={normalize}: {:?}", actual.px[12]);
                assert!((actual.px[11][0] - expected_edge).abs() < tolerance);
                assert!(actual.px.iter().all(|p| p[3] == 1.0));
                assert_eq!(actual.px[0][0], 0.0);
            }
        }
        let effect = StoredEffect::test("gegl:distance-transform", vec![arg("averaging", ArgumentValue::Int(1000))]);
        let a = map(&effect, &ctx).unwrap().unwrap();
        let Some(NativeStructure::Composite { branches }) = a.structure else { panic!("bounded branches") };
        assert!(branches.len() <= 32);
        ctx.remaining_generation_bytes = 0;
        assert!(map(&effect, &ctx).unwrap().is_err());
        ctx = context();
        let cancel = || true;
        ctx.interrupt = photocraft_raster::Interrupt::cancel_only(&cancel);
        assert!(map(&effect, &ctx).unwrap().is_err());
    }
    #[test]
    fn original_native_generators_have_closed_form_counterparts_and_finite_extents() {
        let m = run(
            "gegl:diffraction-patterns",
            vec![
                arg("width", ArgumentValue::Int(2)),
                arg("height", ArgumentValue::Int(1)),
                arg("red-frequency", ArgumentValue::Float(0.0)),
                arg("green-frequency", ArgumentValue::Float(0.0)),
                arg("blue-frequency", ArgumentValue::Float(0.0)),
                arg("brightness", ArgumentValue::Float(0.25)),
            ],
        )
        .unwrap();
        let Some(NativeStructure::Replace { layer, patterns }) = m.structure else { panic!("native pattern") };
        // Two constant zero-phase waves interfere constructively: original
        // primitive intensity1, so the independently expected RGB is brightness.
        for pixel in patterns[0].surface.read_region(Rect::new(0, 0, 2, 1)).as_chunks::<4>().0.iter() {
            for value in &pixel[..3] {
                assert!((*value - 0.25).abs() < 1e-5);
            }
            assert_eq!(pixel[3], 1.0);
        }
        assert!(matches!(layer.content, photocraft_doc::LayerContent::Fill(photocraft_doc::Fill::Pattern { .. })));
        let extent = layer.vector_mask.unwrap();
        assert_eq!(extent.path.subpaths[0].knots[2].anchor.x, 2.0);
        assert_eq!(extent.path.subpaths[0].knots[2].anchor.y, 1.0);
        let mut c = context();
        c.native_filter_bounds = Rect::new(0, 0, 1, 1);
        for family in [0, 1, 2, 3, 4, 5, 6, 7, 8] {
            let effect = StoredEffect::test(
                "gegl:fractal-explorer",
                vec![
                    arg("fractaltype", ArgumentValue::Enum(family)),
                    arg("zoom", ArgumentValue::Float(1.0)),
                    arg("shiftx", ArgumentValue::Float(3.5)),
                    arg("cx", ArgumentValue::Float(if matches!(family, 2 | 3 | 7) { 2.0 } else { 0.0 })),
                    arg("cy", ArgumentValue::Float(0.0)),
                    arg("iter", ArgumentValue::Int(2)),
                    arg("ncolors", ArgumentValue::Int(3)),
                    arg("redmode", ArgumentValue::Enum(2)),
                    arg("greenmode", ArgumentValue::Enum(2)),
                    arg("bluemode", ArgumentValue::Enum(2)),
                ],
            );
            let mapped = map(&effect, &c).unwrap().unwrap();
            let Some(NativeStructure::Replace { patterns, .. }) = mapped.structure else { panic!("quadratic pattern") };
            // c=3 escapes Mandelbrot on step1. Julia z=3,c=0 also
            // escapes on step1. Linear three-band palette gives1/2.
            assert!((patterns[0].surface.pixel(0, 0)[0] - 0.5).abs() < 1e-5);
        }
        for (family, shiftx, shifty, expected) in [(2, -2.5, 0.0, 0.5), (3, -2.5, 0.0, 0.5), (4, -2.5, 0.0, 1.0), (8, 0.1, -0.4, 0.0), (8, 0.3, -0.2, 0.5)] {
            let mapped = map(
                &StoredEffect::test(
                    "gegl:fractal-explorer",
                    vec![
                        arg("fractaltype", ArgumentValue::Enum(family)),
                        arg("zoom", ArgumentValue::Float(1.0)),
                        arg("shiftx", ArgumentValue::Float(shiftx)),
                        arg("shifty", ArgumentValue::Float(shifty)),
                        arg("cx", ArgumentValue::Float(2.0)),
                        arg("cy", ArgumentValue::Float(0.0)),
                        arg("iter", ArgumentValue::Int(2)),
                        arg("ncolors", ArgumentValue::Int(3)),
                        arg("redmode", ArgumentValue::Enum(2)),
                    ],
                ),
                &c,
            )
            .unwrap()
            .unwrap();
            let Some(NativeStructure::Replace { patterns, .. }) = mapped.structure else { panic!("standard native family") };
            // Independently: BarnsleyJ1(-3,c2)=-4,J2=-8; J3=-3→2→3.
            // Sierpinski corner(.1,.1) survives twice;(.3,.3) is a first-level hole.
            assert!((patterns[0].surface.pixel(0, 0)[0] - expected).abs() < 1e-5, "family{family},shift({shiftx},{shifty})");
        }
        assert!(run("gegl:fractal-explorer", vec![arg("fractaltype", ArgumentValue::Enum(9))]).is_err());
        c.remaining_generation_bytes = 0;
        assert!(map(&StoredEffect::test("gegl:fractal-explorer", vec![]), &c).unwrap().is_err());
        c = context();
        c.native_filter_bounds = Rect::new(0, 0, 1000, 1000);
        assert!(map(&StoredEffect::test("gegl:fractal-explorer", vec![arg("iter", ArgumentValue::Int(1000))]), &c).unwrap().is_err());
        assert!(run("gegl:diffraction-patterns", vec![arg("width", ArgumentValue::Int(i32::MAX)), arg("height", ArgumentValue::Int(i32::MAX))]).is_err());
        let cancel = || true;
        c = context();
        c.interrupt = photocraft_raster::Interrupt::cancel_only(&cancel);
        assert!(map(&StoredEffect::test("gegl:fractal-explorer", vec![]), &c).unwrap().is_err());
        let checks = std::sync::atomic::AtomicU32::new(0);
        let mid_orbit = || checks.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 5;
        c = context();
        c.native_filter_bounds = Rect::new(0, 0, 1, 1);
        c.interrupt = photocraft_raster::Interrupt::cancel_only(&mid_orbit);
        assert!(map(&StoredEffect::test("gegl:fractal-explorer", vec![arg("iter", ArgumentValue::Int(1000))]), &c).unwrap().is_err());
        assert_eq!(checks.load(std::sync::atomic::Ordering::Relaxed), 6);
    }
    #[test]
    fn sierpinski_inactive_constants_and_active_log_palette_have_native_counterparts() {
        let c = MappingContext { native_filter_bounds: Rect::new(0, 0, 1, 1), ..context() };
        let make = |smooth, cx, cy| {
            let mapped = map(
                &StoredEffect::test(
                    "gegl:fractal-explorer",
                    vec![
                        arg("fractaltype", ArgumentValue::Enum(8)),
                        arg("zoom", ArgumentValue::Float(1.0)),
                        arg("shiftx", ArgumentValue::Float(0.3)),
                        arg("shifty", ArgumentValue::Float(-0.2)),
                        arg("iter", ArgumentValue::Int(4)),
                        arg("ncolors", ArgumentValue::Int(8192)),
                        arg("redmode", ArgumentValue::Enum(2)),
                        arg("cx", ArgumentValue::Float(cx)),
                        arg("cy", ArgumentValue::Float(cy)),
                        arg("useloglog", ArgumentValue::Bool(smooth)),
                    ],
                ),
                &c,
            )
            .unwrap()
            .unwrap();
            let Some(NativeStructure::Replace { patterns, .. }) = mapped.structure else { panic!("native IFS Pattern") };
            patterns[0].surface.pixel(0, 0)
        };
        // The native point(.3,.3) belongs to the central first-level hole:
        // escape depth1, max depth4. Linear palette=1/4; log palette=ln2/ln5.
        let linear = make(false, -0.75, -0.2);
        let logarithmic = make(true, -0.75, -0.2);
        let quantize = |v: f64| ((v * 8191.0).round() / 8191.0) as f32;
        assert!((linear[0] - quantize(0.25)).abs() < 1e-6);
        assert!((logarithmic[0] - quantize(2.0_f64.ln() / 5.0_f64.ln())).abs() < 1e-6);
        assert_ne!(linear[0], logarithmic[0]);
        assert_eq!(make(false, 1.25, 1.25), linear);
        assert_eq!(make(true, 1.25, 1.25), logarithmic);
    }
    #[test]
    fn fractal_trace_preserves_input_with_editable_native_map_and_reopen() {
        use photocraft_color::{PixelFormat, SampleType};
        use photocraft_doc::{Affine, Document, Layer, LayerContent, SmartObject, SmartSource};
        use photocraft_geom::Size;
        use photocraft_raster::{Interrupt, Surface};
        let bounds = Rect::new(0, 0, 4, 2);
        let mut c = context();
        c.native_filter_bounds = bounds;
        c.owner_bounds = bounds;
        let effect = StoredEffect::test("gegl:fractal-trace", vec![arg("depth", ArgumentValue::Int(1))]);
        let mapped = map(&effect, &c).unwrap().unwrap();
        assert!(mapped.structure.is_none());
        let filter = mapped.filters[0].clone();
        let embedded = crate::smart_map::displace_embedded_map(&filter.params).unwrap();
        // One Mandelbrot step z=c maps each destination to itself before
        // ordinary native8-bit displacement quantization. Midgray becomes128.
        let independent = photocraft_algo::Image { rect: bounds, ch: 3, data: (0..8).flat_map(|_| [128.0 / 255.0, 128.0 / 255.0, 0.5]).collect() };
        let mut source = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::F32, true));
        source.write_region(bounds, &(0..8).flat_map(|i| [i as f32 / 8.0, 0.25, 0.75, 0.5 + i as f32 / 16.0]).collect::<Vec<_>>());
        let original = source.clone();
        let apply = |horizontal, vertical, map| {
            photocraft_algo::apply_in_with(
                &source,
                &photocraft_algo::FilterParams::Displace {
                    horizontal,
                    vertical,
                    stretch: true,
                    undefined: photocraft_algo::UndefinedAreas::Wrap,
                    map: Some(map),
                },
                bounds,
                bounds,
                None,
                bounds,
                &Interrupt::NONE,
            )
            .unwrap()
        };
        let sx = filter.params["horizontal"].as_f64().unwrap() as f32;
        let sy = filter.params["vertical"].as_f64().unwrap() as f32;
        let actual = apply(sx, sy, embedded.clone());
        let expected = apply(4.0 * 100.0 / 128.0, 2.0 * 100.0 / 128.0, std::sync::Arc::new(independent));
        assert_eq!(actual.read_region(bounds), expected.read_region(bounds));
        assert_eq!(source.read_region(bounds), original.read_region(bounds));
        assert_eq!(apply(0.0, 0.0, embedded.clone()).read_region(bounds), source.read_region(bounds));
        let mut input = Document::new("Original native input", Size::new(4, 2), ColorMode::Rgb, SampleType::F32);
        input.layers.push(Layer::new("Input", LayerContent::Raster(source.clone())));
        let contents = photocraft_format::save_to_bytes(&input, &Default::default()).unwrap();
        let mut smart =
            SmartObject::new(SmartSource::Embedded { file_name: "input.pcraft".into(), bytes: std::sync::Arc::new(contents) }, Affine::IDENTITY, None);
        smart.smart_filters = mapped.filters;
        let mut document = Document::new("Native fractal displacement", Size::new(4, 2), ColorMode::Rgb, SampleType::F32);
        document.layers.push(Layer::new("Editable native warp", LayerContent::Smart(smart)));
        let bytes = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
        let LayerContent::Smart(restored) = &reopened.layers[0].content else { panic!("native smart") };
        let restored_map = crate::smart_map::displace_embedded_map(&restored.smart_filters[0].params).unwrap();
        assert_eq!(apply(sx, sy, restored_map).read_region(bounds), actual.read_region(bounds));
        assert_eq!(restored.smart_filters[0].params["horizontal"], filter.params["horizontal"]);
        let mut edited = reopened;
        let LayerContent::Smart(editable) = &mut edited.layers[0].content else { panic!("editable native smart") };
        editable.smart_filters[0].params["horizontal"] = json!(0.0);
        editable.smart_filters[0].params["vertical"] = json!(0.0);
        let edited_bytes = photocraft_format::save_to_bytes(&edited, &Default::default()).unwrap();
        let edited_reopened = photocraft_format::load_from_bytes(&edited_bytes).unwrap();
        let LayerContent::Smart(editable) = &edited_reopened.layers[0].content else { panic!("edited native smart") };
        let settings = &editable.smart_filters[0].params;
        assert_eq!(
            apply(
                settings["horizontal"].as_f64().unwrap() as f32,
                settings["vertical"].as_f64().unwrap() as f32,
                crate::smart_map::displace_embedded_map(settings).unwrap()
            )
            .read_region(bounds),
            source.read_region(bounds)
        );
        c.remaining_metadata_bytes = 0;
        assert!(map(&effect, &c).unwrap().is_err());
        c = context();
        c.remaining_generation_bytes = 0;
        assert!(map(&effect, &c).unwrap().is_err());
        c = context();
        c.native_filter_bounds = Rect::new(0, 0, 1000, 1000);
        assert!(map(&StoredEffect::test("gegl:fractal-trace", vec![arg("depth", ArgumentValue::Int(65536))]), &c).unwrap().is_err());
        let cancel = || true;
        c = context();
        c.interrupt = Interrupt::cancel_only(&cancel);
        assert!(map(&effect, &c).unwrap().is_err());
        assert!(run("gegl:fractal-trace", vec![arg("X1", ArgumentValue::Float(0.5))]).is_err());
    }
    #[test]
    fn zero_tile_background_and_disabled_sobel_axes_translate_to_native_paint() {
        use photocraft_color::{PixelFormat, SampleType};
        use photocraft_doc::{Document, Layer, LayerContent};
        use photocraft_geom::Size;
        use photocraft_raster::Surface;
        for paint in [false, true] {
            let mut arguments = vec![arg("tile-size", ArgumentValue::Float(0.0))];
            if paint {
                arguments.push(arg(
                    "bg-color",
                    ArgumentValue::Color(Some(super::super::ColorLiteral { encoding: "R'G'B'A u8", pixels: &[128, 64, 32, 128], profile: &[] })),
                ));
            }
            let mapped = run("gegl:cubism", arguments).unwrap();
            let Some(NativeStructure::Replace { layer, .. }) = mapped.structure else { panic!("native constant background") };
            assert!(matches!(layer.content, LayerContent::Fill(_)));
            let mut doc = Document::new("Native zero tile background", Size::new(12, 9), ColorMode::Rgb, SampleType::F32);
            doc.layers.push(*layer);
            let before = photocraft_compose::flatten(&doc);
            for pixel in &before.px {
                if paint {
                    assert!((pixel[0] - 128.0 / 255.0).abs() < 1e-5);
                    assert!((pixel[3] - 128.0 / 255.0).abs() < 1e-5);
                } else {
                    assert_eq!(*pixel, [0.0; 4]);
                }
            }
            let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
            assert_eq!(photocraft_compose::flatten(&photocraft_format::load_from_bytes(&bytes).unwrap()).px, before.px);
        }
        for signed in [false, true] {
            let mapped = run(
                "gegl:edge-sobel",
                vec![arg("horizontal", ArgumentValue::Bool(false)), arg("vertical", ArgumentValue::Bool(false)), arg("keep-sign", ArgumentValue::Bool(signed))],
            )
            .unwrap();
            let Some(NativeStructure::Composite { mut branches }) = mapped.structure else { panic!("native neutral style") };
            let mut source = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::F32, true));
            source.write_region(Rect::new(0, 0, 3, 2), &(0..6).flat_map(|_| [0.1, 0.4, 0.8, 0.4]).collect::<Vec<_>>());
            let mut layer = Layer::new("Editable native neutral", LayerContent::Raster(source));
            layer.effects.items = branches.remove(0).layer_effects;
            let mut doc = Document::new("Native disabled Sobel axes", Size::new(3, 2), ColorMode::Rgb, SampleType::F32);
            doc.layers.push(layer);
            let before = photocraft_compose::flatten(&doc);
            let expected = if signed { photocraft_color::convert::linear_to_srgb(0.5) } else { 0.0 };
            for pixel in &before.px {
                assert!((pixel[0] - expected).abs() < 1e-5);
                assert!((pixel[3] - 0.4).abs() < 1e-5);
            }
            let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
            assert_eq!(photocraft_compose::flatten(&photocraft_format::load_from_bytes(&bytes).unwrap()).px, before.px);
        }
    }
    #[test]
    fn cubism_expansion_and_exact_spacing_are_native_geometry_not_dialog_counts() {
        use photocraft_geom::{Point, Rect};
        let c = MappingContext { owner_bounds: Rect::new(0, 0, 8, 4), native_filter_bounds: Rect::new(0, 0, 8, 4), ..context() };
        let make = |size, saturation, seed| {
            cubism(
                &StoredEffect::test(
                    "gegl:cubism",
                    vec![
                        arg("tile-size", ArgumentValue::Float(size)),
                        arg("tile-saturation", ArgumentValue::Float(saturation)),
                        arg("seed", ArgumentValue::UInt(seed)),
                    ],
                ),
                &c,
            )
            .unwrap()
        };
        let mapping = make(4., 0.5, 0);
        let Some(NativeStructure::Composite { branches }) = mapping.structure else { panic!("native collage") };
        assert_eq!(branches.len(), 3);
        assert_eq!(branches[1].filter_frame, Some(Rect::new(0, 0, 4, 4)));
        assert_eq!(branches[2].filter_frame, Some(Rect::new(4, 0, 8, 4)));
        assert_eq!(branches[1].filters[0].command, "filter.blur.average");
        let transform = branches[1].transform;
        let side_start = transform.apply(Point::new(0., 0.));
        let side_end = transform.apply(Point::new(4., 0.));
        assert!(((side_end.x - side_start.x).hypot(side_end.y - side_start.y) - 2.).abs() < 1e-8);
        // Independent first three terms of the published native LCG, seed0.
        let unit = |value: u32| f64::from(value) / f64::from(u32::MAX) - 0.5;
        let angle = unit(1_013_904_223) * std::f64::consts::FRAC_PI_2;
        assert!((transform.m[0] - 0.5 * angle.cos()).abs() < 1e-10);
        let center = transform.apply(Point::new(2., 2.));
        assert!((center.x - (2. + unit(1_196_435_762) * 2.)).abs() < 1e-10);
        assert!((center.y - (2. + unit(3_519_870_697) * 2.)).abs() < 1e-10);
        let Some(NativeStructure::Composite { branches: expanded }) = make(4., 2., 0).structure else { panic!("expanded collage") };
        assert!((expanded[1].transform.m[0] / transform.m[0] - 4.).abs() < 1e-10);
        let Some(NativeStructure::Composite { branches: seeded }) = make(4., 0.5, 1).structure else { panic!("seeded collage") };
        assert_ne!(seeded[1].transform, transform);
        let Some(NativeStructure::Composite { branches: large }) = make(128., 1., 0).structure else { panic!("large collage") };
        let Some(NativeStructure::Composite { branches: larger }) = make(256., 1., 0).structure else { panic!("larger collage") };
        assert_eq!(large.len(), 2);
        assert_eq!(larger.len(), 2);
        assert_ne!(large[1].transform, larger[1].transform);
        assert!(matches!(make(4., 0., 0).structure, Some(NativeStructure::Replace { .. })));
    }
    #[test]
    fn cubism_native_average_placement_overlap_and_persistence_have_independent_counterparts() {
        use photocraft_color::{PixelFormat, SampleType};
        use photocraft_doc::{Document, Layer, LayerContent, SmartObject, SmartSource};
        use photocraft_geom::{Affine, Rect, Size};
        use photocraft_raster::{Interrupt, Surface};
        let bounds = Rect::new(0, 0, 8, 4);
        let c = MappingContext { owner_bounds: bounds, native_filter_bounds: bounds, ..context() };
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let format = PixelFormat::new(ColorMode::Rgb, depth, true);
            let mut source = Surface::new(format);
            source.write_region(bounds, &(0..32).flat_map(|i| if i % 8 < 4 { [0.2, 0.4, 0.6, 0.5] } else { [0.8, 0.6, 0.2, 0.25] }).collect::<Vec<_>>());
            let original = source.read_region(bounds);
            let render = |saturation: f32, independent: bool| {
                let mapping = cubism(
                    &StoredEffect::test(
                        "gegl:cubism",
                        vec![arg("tile-size", ArgumentValue::Float(4.)), arg("tile-saturation", ArgumentValue::Float(saturation))],
                    ),
                    &c,
                )
                .unwrap();
                let Some(NativeStructure::Composite { branches }) = mapping.structure else { panic!("native collage") };
                let mut doc = Document::new("Native Cubism collage", Size::new(8, 4), ColorMode::Rgb, depth);
                for branch in branches {
                    let mut layer = if let Some(fill) = branch.fill {
                        Layer::new("Native background", LayerContent::Fill(fill))
                    } else {
                        let frame = branch.filter_frame.unwrap();
                        let local = Rect::new(0, 0, frame.width() as i32, frame.height() as i32);
                        let mut cropped = Surface::new(format);
                        cropped.write_region(local, &source.read_region(frame));
                        let cache = if independent {
                            let mut expected = Surface::new(format);
                            let color = source.pixel(frame.x0, frame.y0);
                            expected.write_region(local, &color.repeat((frame.width() * frame.height()) as usize));
                            expected
                        } else {
                            photocraft_algo::apply_in_with(&cropped, &photocraft_algo::FilterParams::Average, local, local, None, local, &Interrupt::NONE)
                                .unwrap()
                        };
                        let mut input = Document::new("Native source cell", Size::new(frame.width(), frame.height()), ColorMode::Rgb, depth);
                        input.layers.push(Layer::new("Original input cell", LayerContent::Raster(cropped)));
                        let bytes = photocraft_format::save_to_bytes(&input, &Default::default()).unwrap();
                        let mut smart = SmartObject::new(
                            SmartSource::Embedded { file_name: "cell.pcraft".into(), bytes: std::sync::Arc::new(bytes) },
                            branch.transform.mul(&Affine::translate(f64::from(frame.x0), f64::from(frame.y0))),
                            Some(cache),
                        );
                        smart.smart_filters = branch.filters;
                        Layer::new("Editable native averaged tile", LayerContent::Smart(smart))
                    };
                    layer.vector_mask = branch.vector_mask;
                    doc.layers.push(layer);
                }
                doc
            };
            for saturation in [0.5, 2.] {
                let actual = render(saturation, false);
                let expected = render(saturation, true);
                let pixels = photocraft_compose::render(&actual, bounds).px;
                let expected_pixels = photocraft_compose::render(&expected, bounds).px;
                assert!(pixels.iter().flatten().zip(expected_pixels.iter().flatten()).all(|(a, b)| (a - b).abs() < 1e-6));
                let bytes = photocraft_format::save_to_bytes(&actual, &Default::default()).unwrap();
                let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
                assert_eq!(photocraft_compose::render(&reopened, bounds).px, pixels);
            }
            let small = photocraft_compose::render(&render(0.5, false), bounds).px;
            let expanded = photocraft_compose::render(&render(2., false), bounds).px;
            assert_ne!(small, expanded);
            assert!(expanded.iter().map(|p| p[3]).sum::<f32>() > small.iter().map(|p| p[3]).sum::<f32>());
            assert_eq!(source.read_region(bounds), original);
        }
    }
    #[test]
    fn cubism_dense_geometry_and_aggregate_caches_reject_atomically_before_native_generation() {
        let effect = StoredEffect::test("gegl:cubism", vec![arg("tile-size", ArgumentValue::Float(0.0001))]);
        assert!(cubism(&effect, &context()).unwrap_err().message.contains("63 cells"));
        let effect = StoredEffect::test("gegl:cubism", vec![]);
        for c in [MappingContext { remaining_generation_bytes: 1, ..context() }, MappingContext { remaining_metadata_bytes: 1, ..context() }] {
            assert!(cubism(&effect, &c).is_err());
        }
        let checks = std::sync::atomic::AtomicUsize::new(0);
        let cancel = || checks.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 3;
        let c = MappingContext {
            native_filter_bounds: photocraft_geom::Rect::new(0, 0, 40, 40),
            interrupt: photocraft_raster::Interrupt::cancel_only(&cancel),
            ..context()
        };
        assert!(cubism(&effect, &c).is_err());
        assert!(checks.load(std::sync::atomic::Ordering::Relaxed) >= 4);
        let c = MappingContext { native_filter_bounds: photocraft_geom::Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX), ..context() };
        assert!(cubism(&effect, &c).is_err());
    }
    #[test]
    fn light_intensity_endpoint_and_detail_controls_use_actual_native_math() {
        use photocraft_color::{PixelFormat, SampleType};
        use photocraft_doc::adjust::{CurvePoint, ToneSpace};
        use photocraft_doc::{Adjustment, Affine, Document, Layer, LayerContent, SmartObject, SmartSource};
        use photocraft_raster::{Interrupt, Surface};
        let bounds = Rect::new(0, 0, 7, 7);
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut flat = Surface::new(PixelFormat::new(ColorMode::Rgb, depth, true));
            flat.write_region(bounds, &(0..49).flat_map(|_| [0.5, 0.5, 0.5, 0.4]).collect::<Vec<_>>());
            for elevation in [0.0_f32, 30.0, 90.0, 120.0, 180.0] {
                let mapping = run("gegl:emboss", vec![arg("elevation", ArgumentValue::Float(elevation)), arg("depth", ArgumentValue::Int(1))]).unwrap();
                let f = &mapping.filters[0];
                let native = photocraft_algo::apply_in_with(
                    &flat,
                    &photocraft_algo::FilterParams::Emboss {
                        angle: f.params["angle"].as_f64().unwrap() as f32,
                        height: 1.0,
                        amount: f.params["amount"].as_f64().unwrap() as f32,
                    },
                    bounds,
                    bounds,
                    None,
                    bounds,
                    &Interrupt::NONE,
                )
                .unwrap();
                let mut buffer = photocraft_compose::Buffer { rect: bounds, px: native.read_region(bounds).as_chunks::<4>().0.to_vec() };
                let curves = std::array::from_fn(|i| {
                    mapping.filters[1].params[["red", "green", "blue"][i]]
                        .as_array()
                        .unwrap()
                        .iter()
                        .enumerate()
                        .map(|(n, v)| CurvePoint { input: n as f32 / 255.0, output: v.as_f64().unwrap() as f32 / 255.0 })
                        .collect()
                });
                photocraft_compose::adjust::apply_in(
                    &Adjustment::Curves { master: vec![], per_channel: curves, space: ToneSpace::Rgb, black: vec![], pencil: true },
                    &mut buffer,
                    photocraft_compose::adjust::Transfer::for_document(ColorMode::Rgb, depth),
                    Some(depth),
                    ColorMode::Rgb,
                );
                let expected = photocraft_color::convert::linear_to_srgb(elevation.to_radians().sin().max(0.0));
                assert!((buffer.px[24][0] - expected).abs() < 0.01, "{depth:?},elevation{elevation}");
                assert_eq!(buffer.px[24][3], flat.pixel(3, 3)[3]);
            }
        }
        let neon = run("gegl:edge-neon", vec![arg("radius", ArgumentValue::Float(0.0)), arg("amount", ArgumentValue::Float(1.0))]).unwrap();
        let mut buffer = photocraft_compose::Buffer { rect: Rect::new(0, 0, 1, 1), px: vec![[0.01, 0.02, 0.03, 0.4]] };
        for filter in neon.filters.iter().filter(|f| f.command == "image.adjustments.channelMixer") {
            let matrix = std::array::from_fn(|i| {
                std::array::from_fn(|j| {
                    let value = filter.params[["red", "green", "blue"][i]][j].as_f64().unwrap();
                    assert!(value.abs() <= 200.0);
                    value as f32 / 100.0
                })
            });
            photocraft_compose::adjust::apply_in(
                &Adjustment::ChannelMixer { matrix, monochrome: false },
                &mut buffer,
                photocraft_compose::adjust::Transfer::for_document(ColorMode::Rgb, SampleType::F32),
                Some(SampleType::F32),
                ColorMode::Rgb,
            );
        }
        assert!((buffer.px[0][0] - 0.1).abs() < 1e-6);
        assert!((buffer.px[0][2] - 0.3).abs() < 1e-6);
        assert_eq!(buffer.px[0][3], 0.4);
        let limited = run("gegl:engrave", vec![arg("limit", ArgumentValue::Bool(true))]).unwrap();
        assert_eq!(limited.filters[0].params["outBlack"], 25.5);
        assert_eq!(limited.filters[0].params["outWhite"], 229.5);
        let half = run("gegl:engrave", vec![arg("limit", ArgumentValue::Bool(true)), arg("row-height", ArgumentValue::Int(2))]).unwrap();
        assert_eq!(half.filters[0].params["outBlack"], half.filters[0].params["outWhite"]);
        assert!(run("gegl:engrave", vec![arg("row-height", ArgumentValue::Int(1))]).is_err());
        let mapping = run(
            "gegl:fattal02",
            vec![arg("beta", ArgumentValue::Float(1.0)), arg("noise", ArgumentValue::Float(0.1)), arg("saturation", ArgumentValue::Float(1.0))],
        )
        .unwrap();
        let f = &mapping.filters[0];
        assert_eq!(f.command, "filter.blur.surfaceBlur");
        let area = Rect::new(0, 0, 9, 1);
        let mut source = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::F32, true));
        source.write_region(area, &[0.2, 0.21, 0.2, 0.2, 0.9, 0.9, 0.89, 0.9, 0.9].into_iter().flat_map(|v| [v, v, v, 0.6]).collect::<Vec<_>>());
        let apply = |threshold| {
            photocraft_algo::apply_in_with(
                &source,
                &photocraft_algo::FilterParams::SurfaceBlur { radius: 1.0, threshold },
                area,
                area,
                None,
                area,
                &Interrupt::NONE,
            )
            .unwrap()
        };
        let actual = apply(f.params["threshold"].as_f64().unwrap() as f32);
        let expected = apply(255.0 * (1.0 - (-0.1_f32).exp()));
        assert_eq!(actual.read_region(area), expected.read_region(area));
        assert!(actual.pixel(1, 0)[0] < source.pixel(1, 0)[0]);
        assert!(actual.pixel(4, 0)[0] > 0.89);
        assert_eq!(actual.pixel(1, 0)[3], source.pixel(1, 0)[3]);
        let fractional = run("gegl:fattal02", vec![arg("beta", ArgumentValue::Float(0.995)), arg("alpha", ArgumentValue::Float(0.0))]).unwrap();
        assert!((fractional.filters[0].opacity - 0.5).abs() < 1e-5);
        assert_eq!(fractional.filters[0].params["amount"], 1.0);
        let smooth = run("gegl:fattal02", vec![arg("beta", ArgumentValue::Float(1.5)), arg("alpha", ArgumentValue::Float(0.0))]).unwrap();
        assert_eq!(smooth.filters[0].opacity, 0.5);
        assert_eq!(smooth.filters[0].params["threshold"], 2.0);
        let mut contents = Document::new("Native detail input", photocraft_geom::Size::new(9, 1), ColorMode::Rgb, SampleType::F32);
        contents.layers.push(Layer::new("Input", LayerContent::Raster(source)));
        let bytes = photocraft_format::save_to_bytes(&contents, &Default::default()).unwrap();
        let mut smart = SmartObject::new(SmartSource::Embedded { file_name: "input.pcraft".into(), bytes: std::sync::Arc::new(bytes) }, Affine::IDENTITY, None);
        smart.smart_filters = mapping.filters.clone();
        let mut document = Document::new("Native detail controls", photocraft_geom::Size::new(9, 1), ColorMode::Rgb, SampleType::F32);
        document.layers.push(Layer::new("Detail", LayerContent::Smart(smart)));
        let bytes = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
        let LayerContent::Smart(smart) = &reopened.layers[0].content else { panic!("native detail") };
        assert_eq!(smart.smart_filters, mapping.filters);
    }
    #[test]
    fn partial_native_intent_keeps_unmapped_lighting_coverage_and_intensity_explicit() {
        let a = run(
            "gegl:emboss",
            vec![arg("azimuth", ArgumentValue::Float(300.0)), arg("depth", ArgumentValue::Int(7)), arg("elevation", ArgumentValue::Float(60.0))],
        )
        .unwrap();
        assert_eq!(a.filters[0].params["angle"], -60.0);
        assert_eq!(a.filters[0].params["height"], 7);
        assert!(a.notes[0].contains("Elevation 60 degrees"));
        assert!((a.filters[0].params["amount"].as_f64().unwrap() - 50.0).abs() < 1e-6);
        let a = run("gegl:emboss", vec![arg("type", ArgumentValue::String("bumpmap"))]).unwrap();
        let Some(NativeStructure::Composite { branches }) = a.structure else { panic!("native color relief") };
        assert_eq!(branches.len(), 2);
        assert_eq!(branches[1].blend, BlendMode::Overlay);
        let a = run("gegl:cubism", vec![arg("tile-size", ArgumentValue::Float(3.0)), arg("seed", ArgumentValue::UInt(42))]).unwrap();
        assert!(a.notes[0].contains("Actual seed 42"));
        let Some(NativeStructure::Composite { branches }) = a.structure else { panic!("native cell collage") };
        assert!(branches.len() > 1);
        assert_eq!(branches[1].filter_frame.unwrap().width(), 3);
        let Some(photocraft_doc::Fill::Solid(background)) = &branches[0].fill else { panic!("native background") };
        assert_eq!(background.alpha, 0.0);
        let a = run("gegl:edge-neon", vec![arg("radius", ArgumentValue::Float(1500.0)), arg("amount", ArgumentValue::Float(40.0))]).unwrap();
        assert_eq!(a.filters.len(), 14);
        assert!(a.notes[0].contains("intensity 40 becomes observed relative gain"));
        let total_variance =
            a.filters.iter().filter(|f| f.command == "filter.blur.gaussianBlur").map(|f| f.params["radius"].as_f64().unwrap().powi(2)).sum::<f64>();
        assert!((total_variance - 1500.0_f64.powi(2)).abs() < 1e-6);
        let a = run("gegl:engrave", vec![arg("row-height", ArgumentValue::Int(10))]).unwrap();
        assert_eq!(a.filters[0].command, "filter.gallery.halftonePattern");
        assert_eq!(a.filters[0].params["size"], 4.25);
        assert_eq!(a.filters[0].params["patternType"], "line");
        let a = run("gegl:fattal02", vec![arg("beta", ArgumentValue::Float(0.5)), arg("saturation", ArgumentValue::Float(0.5))]).unwrap();
        assert!(
            (a.filters.iter().find(|f| f.command == "image.adjustments.curves").unwrap().params["points"][64].as_f64().unwrap()
                - (64.0 / 255.0_f64).sqrt() * 255.0)
                .abs()
                < 1e-9
        );
        assert_eq!(a.filters.last().unwrap().params["saturation"], -50.0);
    }
    #[test]
    fn hostile_numbers_unknown_controls_and_unfinished_generators_stay_inert() {
        assert_eq!(run("gegl:dropshadow", vec![arg("radius", ArgumentValue::Float(f32::NAN))]).unwrap_err().reason, ConversionReason::InvalidOptionalData);
        assert!(run("gegl:edge-laplace", vec![arg("surprise", ArgumentValue::Float(1.0))]).is_err());
        assert!(run("gegl:fractal-explorer", vec![arg("fractaltype", ArgumentValue::Enum(9))]).is_err());
        assert!(map(&StoredEffect::test("not:owned", vec![]), &context()).is_none());
    }
}
