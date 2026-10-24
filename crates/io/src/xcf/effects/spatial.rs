//! Finite B05 native spatial translations from the frozen 3.2.6 manifest.
//! O008/O010/O030/O031/O036/O046/O049/O050/O052/O059/O068/O069/O073/O074/O075,
//! O078/O079/O080/O081/O082/O083/O084/O086/O106/O121/O124. No source evaluator.

use photocraft_color::{BlendMode, ColorMode};
use photocraft_doc::{SmartFilter, import_conversion::ConversionReason};
use serde_json::{Value, json};

use super::{ArgumentValue, MappingContext, MappingFailure, MappingResult, NativeMapping, StoredEffect, native_filter};

type Result<T> = std::result::Result<T, MappingFailure>;

fn invalid(message: impl Into<String>) -> MappingFailure {
    MappingFailure { reason: ConversionReason::InvalidOptionalData, message: message.into() }
}

fn number(e: &StoredEffect<'_>, key: &str, default: f64, low: f64, high: f64) -> Result<f64> {
    let value = e.number(key).map_err(invalid)?.unwrap_or(default);
    if !value.is_finite() || !(low..=high).contains(&value) {
        return Err(invalid(format!("{}: {key} is outside its source numeric domain", e.operation)));
    }
    Ok(value)
}

fn boolean(e: &StoredEffect<'_>, key: &str) -> Result<Option<bool>> {
    e.boolean(key).map_err(invalid)
}

fn integer(e: &StoredEffect<'_>, key: &str, default: u32, low: u32, high: u32) -> Result<u32> {
    let value = number(e, key, f64::from(default), f64::from(low), f64::from(high))?;
    if value.fract() != 0.0 {
        return Err(invalid(format!("{}: {key} must be integral", e.operation)));
    }
    Ok(value as u32)
}

fn choice(e: &StoredEffect<'_>, key: &str, default: i32, allowed: &[i32]) -> Result<i32> {
    let value = e.enum_value(key).map_err(invalid)?.unwrap_or(default);
    if !allowed.contains(&value) {
        return Err(invalid(format!("{}: unknown {key} enum {value}", e.operation)));
    }
    Ok(value)
}

fn seed(e: &StoredEffect<'_>) -> Result<u32> {
    match e.argument("seed").map_err(invalid)? {
        None => Ok(0),
        Some(ArgumentValue::UInt(value)) => Ok(*value),
        _ => Err(invalid("spatial noise seed must be an unsigned stored integer")),
    }
}

fn native_range(value: f64, key: &str, low: f64, high: f64) -> Result<f64> {
    if !(low..=high).contains(&value) {
        return Err(format!(
            "{key}={value} needs another native structure outside the existing {low}..{high} dialog range; source configuration remains inert"
        )
        .into());
    }
    Ok(value)
}

fn mapped(filters: Vec<SmartFilter>, note: impl Into<String>) -> NativeMapping {
    NativeMapping { filters, notes: vec![note.into()], ..Default::default() }
}

fn one(command: &str, params: Value, note: impl Into<String>) -> NativeMapping {
    mapped(vec![native_filter(command, params)], note)
}

fn identity() -> NativeMapping {
    mapped(Vec::new(), "The source spatial amount is zero; preserve native pixels without forcing a nonzero native dialog minimum")
}

fn gaussian(radius: f64) -> Result<SmartFilter> {
    native_range(radius, "Gaussian standard deviation", 0.1, 1000.0)?;
    Ok(native_filter("filter.blur.gaussianBlur", json!({"radius": radius})))
}

/// Native Gaussian radius is sigma (algo::FilterParams and gaussian_sigma_matches_radius).
/// Trace/2 of the source covariance gives an explicitly isotropic approximation using both axes.
fn gaussian_map(e: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping> {
    let x = number(e, "std-dev-x", 1.5, 0.0, 1500.0)?;
    let y = number(e, "std-dev-y", 1.5, 0.0, 1500.0)?;
    choice(e, "filter", 0, &[0, 1, 2])?;
    choice(e, "abyss-policy", 1, &[0, 1, 2, 3])?;
    boolean(e, "clip-extent")?;
    if x == 0.0 && y == 0.0 {
        return Ok(identity());
    }
    let radius = ((x * x + y * y) / 2.0).sqrt();
    let passes = ((radius / 1000.0).powi(2).ceil() as u32).max(1);
    let native_sigma = radius / f64::from(passes).sqrt();
    super::residual_spatial::generation_bytes(
        context,
        (passes as usize)
            .checked_mul(2048)
            .and_then(|n| n.checked_add(4096))
            .ok_or_else(|| MappingFailure::resource("native filter stack allocation overflow"))?,
    )?;
    let mut filters = (0..passes).map(|_| gaussian(native_sigma.max(0.1))).collect::<Result<Vec<_>>>()?;
    if native_sigma < 0.1 {
        for filter in &mut filters {
            filter.opacity = (native_sigma / 0.1).powi(2) as f32;
        }
    }

    Ok(mapped(
        filters,
        format!(
            "Native Gaussian sigma {radius}px is the root mean square of source sigmas {x}/{y}px; unequal axes become isotropic. {passes} native passes at sigma {native_sigma}px preserve summed variance within dialog bounds. Below native sigma0.1, fractional native filter strength (sigma/.1)^2 is an explicit weak-kernel approximation rather than identity or silently full minimum blur. Native kernel, alpha, boundary extension and output extent apply; source FIR/IIR, abyss and clip choices are retained, not evaluated"
        ),
    ))
}

fn center(e: &StoredEffect<'_>, context: &MappingContext) -> Result<[f64; 2]> {
    let (owner, native) = (context.owner_bounds, context.native_filter_bounds);
    if owner.is_empty() || native.is_empty() {
        return Err(invalid("spatial effect needs nonempty source-owner and native evaluation bounds"));
    }
    let x = number(e, "center-x", 0.5, -10.0, 10.0)?;
    let y = number(e, "center-y", 0.5, -10.0, 10.0)?;
    Ok([
        (f64::from(owner.x0) + f64::from(owner.width()) * x - f64::from(native.x0)) / f64::from(native.width()),
        (f64::from(owner.y0) + f64::from(owner.height()) * y - f64::from(native.y0)) / f64::from(native.height()),
    ])
}

fn native_gain(gain: f64) -> Result<Vec<SmartFilter>> {
    if !gain.is_finite() || gain <= 0. {
        return Err(invalid("native gain must be positive and finite"));
    }
    let stops = gain.log2();
    let count = (stops.abs() / 20.).ceil().max(1.) as usize;
    if count > 64 {
        return Err(MappingFailure::resource("native exposure decomposition exceeds64 filters"));
    }
    Ok((0..count).map(|_| native_filter("image.adjustments.exposure", json!({"exposure":stops/count as f64,"offset":0.,"gamma":1.}))).collect())
}
/// Conservative native box-Gaussian workspace: clipped output, repeated-edge 1-D
/// lines/strips and concurrent branch caches, never a two-dimensional halo.
pub(super) fn gaussian_workspace(c: &MappingContext, sigma: f64, caches: usize) -> Result<()> {
    let b = c.native_filter_bounds;
    if b.is_empty() || !sigma.is_finite() || !(0.0..=1000.).contains(&sigma) {
        return Err(invalid("native Gaussian workspace requires nonempty bounds and radius0..1000"));
    }
    let w = b.width() as usize;
    let h = b.height() as usize;
    let reach = (sigma * 3. + 6.).ceil() as usize;
    let samples = w
        .checked_mul(h)
        .and_then(|area| w.checked_add(2 * reach).and_then(|line| line.checked_mul(h)).and_then(|lines| area.checked_add(lines)))
        .and_then(|area| h.checked_add(2 * reach).and_then(|line| line.checked_mul(w)).and_then(|lines| area.checked_add(lines)))
        .and_then(|n| n.checked_mul(3))
        .and_then(|n| n.checked_mul(c.mode.color_channels() + 1))
        .and_then(|n| n.checked_mul(4))
        .ok_or_else(|| MappingFailure::resource("native Gaussian scratch size overflows"))?;
    let tile = photocraft_geom::TILE_SIZE as usize;
    let axis = |a: i32, z: i32| -> Result<usize> {
        usize::try_from((i64::from(z) - 1).div_euclid(tile as i64) - i64::from(a).div_euclid(tile as i64) + 1)
            .map_err(|_| MappingFailure::resource("native Gaussian tile extent is invalid"))
    };
    let cache = axis(b.x0, b.x1)?
        .checked_mul(axis(b.y0, b.y1)?)
        .and_then(|n| n.checked_mul(tile * tile))
        .and_then(|n| n.checked_mul((c.mode.color_channels() + 1) * 4))
        .and_then(|n| n.checked_mul(caches))
        .and_then(|n| n.checked_add(samples))
        .ok_or_else(|| MappingFailure::resource("native Gaussian cache size overflows"))?;
    super::residual_spatial::generation_bytes(c, cache)
}
fn broad_unsharp(radius: f64, scale: f64, threshold: f64, c: &MappingContext) -> Result<NativeMapping> {
    use super::{NativeBranch, NativeStructure};
    if threshold != 0. {
        // Native repeated Unsharp preserves an editable per-channel threshold,
        // without inventing a two-input dynamic mask or widening its dialog.
        // The radius partition is a support policy, not a convolution identity:
        // thresholded sharpening is nonlinear and its gains do not convolve as
        // Gaussian variances do.
        // Four passes fit the entire validated scale<=300 domain without
        // changing support when either radius or strength crosses a pass boundary.
        let count = 4usize;
        let component_radius = radius / (count as f64).sqrt();
        let amount = ((scale.ln_1p() / count as f64).exp_m1() * 100.).min(500.);
        gaussian_workspace(c, component_radius, count + 3)?;
        let bytes = count * 2048 + 4096;
        if bytes > c.remaining_metadata_bytes {
            return Err(MappingFailure::resource("native threshold sharpening stack exceeds actual remaining metadata allowance"));
        }
        super::residual_spatial::generation_bytes(c, bytes)?;
        let mut filters = Vec::new();
        filters.try_reserve_exact(count).map_err(|_| MappingFailure::resource("native threshold sharpening stack allocation"))?;
        for _ in 0..count {
            super::residual_spatial::generation_check(c)?;
            let mut filter =
                native_filter("filter.sharpen.unsharpMask", json!({"radius": component_radius,"amount": amount.max(1.),"threshold": threshold * 255.}));
            filter.opacity = amount.min(1.) as f32;
            filters.push(filter);
        }
        return Ok(mapped(
            filters,
            format!(
                "Broad threshold sharpening uses a fixed set of {count} ordinary editable native Unsharp passes at radius {component_radius}px, amount {amount}% and threshold {} native levels in every pass. Radius is partitioned by sqrt(pass count) and total high-frequency gain by (1+scale)^(1/pass count)-1; gains below1% use fractional native filter opacity. This is a native multiscale sharpening approximation: thresholded passes are nonlinear, their spatial response is not variance-equivalent to one broad Gaussian, and each pass gates its own per-channel difference rather than one original broad difference. Native alpha-weighted edges, intermediate integer clipping and HDR rules apply. Source radius/strength/threshold remain meaningful controls without expanding the native radius1000 or amount500 dialogs; no source evaluator persists",
                threshold * 255.
            ),
        ));
    }
    let count = (radius / 1000.).powi(2).ceil().max(1.) as usize;
    gaussian_workspace(c, radius / (count as f64).sqrt(), 6)?;
    let bytes = (count + 16) * 2048 + 8192;
    super::residual_spatial::generation_bytes(c, bytes)?;
    if bytes > c.remaining_metadata_bytes {
        return Err(MappingFailure::resource("native sharpening branches exceed actual remaining metadata allowance"));
    }
    let mut blur = (0..count).map(|_| gaussian(radius / (count as f64).sqrt())).collect::<Result<Vec<_>>>()?;
    blur.extend(native_gain(scale)?);
    Ok(NativeMapping{structure:Some(NativeStructure::Composite{branches:vec![NativeBranch{filters:native_gain(1.+scale)?,..Default::default()},NativeBranch{filters:blur,blend:photocraft_color::BlendMode::Subtract,clipped:true,..Default::default()}]}),notes:vec!["Broad sharpening becomes ordinary native Exposure/variance-split Gaussian branches implementing the input/base-detail gain intent. Native Exposure gains follow their own transfer policy rather than source encoded-channel arithmetic. Native clipped Subtract retains original coverage while native negative/HDR/channel clipping and alpha-weighted Gaussian boundaries replace source unsharp behavior. This path currently supports zero difference threshold only; no source kernel or threshold evaluator persists".into()],..Default::default()})
}

fn zoom_copies(factor: f64, centre: [f64; 2], context: &MappingContext) -> Result<NativeMapping> {
    use super::{NativeBranch, NativeStructure};
    let b = context.native_filter_bounds;
    let cx = f64::from(b.x0) + centre[0] * f64::from(b.width());
    let cy = f64::from(b.y0) + centre[1] * f64::from(b.height());
    let bytes = 32 * 4096;
    super::residual_spatial::generation_bytes(context, bytes)?;
    if bytes > context.remaining_metadata_bytes {
        return Err(MappingFailure::resource("native zoom copies exceed actual remaining metadata allowance"));
    }
    let mut branches = Vec::new();
    branches.try_reserve_exact(32).map_err(|_| MappingFailure::resource("native zoom copy allocation unavailable"))?;
    for n in 0..32 {
        super::residual_spatial::generation_check(context)?;
        let t = (f64::from(n) + 0.5) / 32.;
        let scale = 1. / (1. - factor * t);
        let transform =
            photocraft_geom::Affine::translate(cx, cy).mul(&photocraft_geom::Affine::scale(scale)).mul(&photocraft_geom::Affine::translate(-cx, -cy));
        branches.push(NativeBranch { transform, opacity: 1. / (n + 1) as f32, ..Default::default() });
    }
    Ok(NativeMapping{structure:Some(NativeStructure::Composite{branches}),notes:vec!["Native midpoint ray quadrature uses32 editable scaled source copies about the requested center, with conditional Normal opacities giving equal weights where copies are opaque. Positive source zoom samples inward; negative samples outward through native inverse placements. Native Free Transform interpolation, transparent transformed edges and partial-alpha/overlap composition differ from source radial sampling; no source evaluator or silently clamped native slider".into()],..Default::default()})
}
fn radial(e: &StoredEffect<'_>, context: &MappingContext, zoom: bool) -> Result<NativeMapping> {
    let amount = if zoom {
        let factor = number(e, "factor", 0.1, -10.0, 1.0)?;
        if factor == 0.0 {
            return Ok(identity());
        }
        let centre = center(e, context)?;
        if factor < 0.005 || factor > 0.5 || centre.iter().any(|v| !(0.0..=1.0).contains(v)) {
            return zoom_copies(factor, centre, context);
        }
        factor * 200.
    } else {
        let angle = number(e, "angle", 5.0, 0.0, 360.0)?;
        if angle == 0.0 {
            return Ok(identity());
        }
        let centre = center(e, context)?;
        if !(1.0..=100.0).contains(&angle) || centre.iter().any(|v| !(0.0..=1.0).contains(v)) {
            use super::{NativeBranch, NativeStructure};
            let [x, y] = centre;
            let b = context.native_filter_bounds;
            let cx = f64::from(b.x0) + x * f64::from(b.width());
            let cy = f64::from(b.y0) + y * f64::from(b.height());
            let metadata = 32 * 4096;
            super::residual_spatial::generation_bytes(context, metadata)?;
            if metadata > context.remaining_metadata_bytes {
                return Err(MappingFailure::resource("native spin copies exceed actual remaining metadata allowance"));
            }
            let mut branches = Vec::new();
            branches.try_reserve_exact(32).map_err(|_| MappingFailure::resource("native spin copy allocation unavailable"))?;
            for n in 0..32 {
                super::residual_spatial::generation_check(context)?;
                let rotation = (angle * ((f64::from(n) + 0.5) / 32. - 0.5)).to_radians();
                let transform = photocraft_geom::Affine::translate(cx, cy)
                    .mul(&photocraft_geom::Affine::rotate(rotation))
                    .mul(&photocraft_geom::Affine::translate(-cx, -cy));
                branches.push(NativeBranch { transform, opacity: 1. / (n + 1) as f32, ..Default::default() });
            }
            return Ok(NativeMapping{structure:Some(NativeStructure::Composite{branches}),notes:vec!["Native angular midpoint quadrature uses32 editable rotated source copies over the requested symmetric spin arc and actual center. Conditional native Normal opacities give equal weights for opaque copies. Native Free Transform interpolation, transformed transparent edges and partial-alpha overlap differ from source angular sampling. Tiny and full-circle source angles remain meaningful without clamping to native Radial Blur dialog1..100".into()],..Default::default()});
        }
        // Native Radial Blur spin's amount is its arc in degrees (algo::blur::radial).
        native_range(angle, "native spin arc", 1.0, 100.0)?
    };
    let [x, y] = center(e, context)?;
    Ok(one(
        "filter.blur.radialBlur",
        json!({"amount": amount, "method": if zoom { "zoom" } else { "spin" }, "quality": "best", "centerX": x, "centerY": y}),
        "Source center translated from owner coordinates into native evaluation bounds. Native angular/ray quadrature, alpha, sampling and boundaries apply; the source kernel is not reproduced",
    ))
}

fn repeated_denoise(e: &StoredEffect<'_>, iterations: u32, context: &MappingContext) -> Result<NativeMapping> {
    if iterations == 0 {
        return Ok(identity());
    }
    super::residual_spatial::generation_bytes(
        context,
        (iterations as usize)
            .checked_mul(2048)
            .and_then(|n| n.checked_add(4096))
            .ok_or_else(|| MappingFailure::resource("native filter stack allocation overflow"))?,
    )?;
    let filters = (0..iterations)
        .map(|_| {
            native_filter(
                "filter.noise.reduceNoise",
                json!({"strength": 1, "preserveDetails": 60, "reduceColorNoise": 0, "sharpenDetails": 0, "removeJpegArtifact": false}),
            )
        })
        .collect();
    Ok(mapped(
        filters,
        format!(
            "{} becomes {iterations} ordinary native Reduce Noise passes with explicitly chosen mild strength 1, detail 60 and no sharpening/color-noise pass. Iterations remain pass count, not percentage; native smoothing replaces source diffusion",
            e.operation
        ),
    ))
}

fn median(e: &StoredEffect<'_>) -> Result<NativeMapping> {
    let signed = number(e, "radius", 3.0, -400.0, 400.0)?;
    if signed.fract() != 0.0 {
        return Err(invalid("source median radius must be integral"));
    }
    let neighborhood = choice(e, "neighborhood", 1, &[0, 1, 2])?;
    let mut color = number(e, "percentile", 50.0, 0.0, 100.0)?;
    let mut alpha = number(e, "alpha-percentile", 50.0, 0.0, 100.0)?;
    choice(e, "abyss-policy", 1, &[0, 1])?;
    boolean(e, "high-precision")?;
    if signed == 0.0 {
        return Ok(identity());
    }
    if signed < 0.0 {
        color = 100.0 - color;
        alpha = 100.0 - alpha;
    }
    Ok(one(
        "filter.other.percentile",
        json!({"radius": signed.abs() as u16,"colorPercentile":color,"alphaPercentile":alpha,"shape":match neighborhood{0=>"square",1=>"disc",_=>"diamond"}}),
        "Source color/alpha percentiles remain independent ordinary native order statistics; negative radius reverses both ranks. Native rank=floor((N-1)*percentile/100), repeated owner edges and native Median discrete disc footprint apply; source abyss/high-precision and circle-boundary policies are explicit native numerical/geometric approximations",
    ))
}

fn rgb_noise(e: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping> {
    let independent = boolean(e, "independent")?.unwrap_or(true);
    let correlated = boolean(e, "correlated")?.unwrap_or(false);
    let linear = boolean(e, "linear")?.unwrap_or(true);
    let gaussian = boolean(e, "gaussian")?.unwrap_or(true);
    let red = number(e, "red", 0.2, 0.0, 1.0)?;
    let green = number(e, "green", 0.2, 0.0, 1.0)?;
    let blue = number(e, "blue", 0.2, 0.0, 1.0)?;
    let alpha = number(e, "alpha", 0.0, 0.0, 1.0)?;
    let seed = seed(e)?;
    let components = if independent { [red, green, blue] } else { [red; 3] };
    let multiplier = if gaussian { 200.0 } else { 100.0 };
    let mut amounts = if context.mode == ColorMode::Rgb {
        components.into_iter().map(|v| v * multiplier).collect::<Vec<_>>()
    } else {
        // Source RGB amplitudes have no separate color axes in a scalar/native non-RGB owner.
        // Keep expected scalar variance through an explicitly declared native RMS policy.
        let rms = (components.into_iter().map(|v| v * v).sum::<f64>() / 3.).sqrt();
        vec![rms * multiplier; context.mode.color_channels()]
    };
    amounts.push(alpha * multiplier);
    if amounts.iter().all(|v| *v == 0.) {
        return Ok(identity());
    }
    Ok(one(
        "filter.noise.channelNoise",
        json!({"amounts":amounts,"distribution":if gaussian{"gaussian"}else{"uniform"},"monochromatic":!independent,"relative":correlated,"seed":seed}),
        format!(
            "Source RGB and alpha amplitudes become independent native channel amounts (uniform halfwidth=amount/200; Gaussian sigma=amount/400). Source correlated={correlated} becomes declared native signal-relative value+value*noise; source independent={independent} chooses native random-channel correlation. Native coordinate hash/sequence and edited-alpha clamp0..1 apply; finite native HDR colors are retained. Source linear={linear}, native linear={}; native encoding is authoritative. Non-RGB native owners use explicit equal-channel RMS color amplitude rather than invent RGB axes",
            context.linear
        ),
    ))
}

fn neighbor_noise(e: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping> {
    let probability = number(e, "pct-random", 50.0, 0.0, 100.0)? / 100.0;
    let repeat = integer(e, "repeat", 1, 1, 100)?;
    let seed = seed(e)?;
    if probability == 0.0 {
        return Ok(identity());
    }
    super::residual_spatial::generation_bytes(
        context,
        (repeat as usize)
            .checked_mul(2048)
            .and_then(|n| n.checked_add(4096))
            .ok_or_else(|| MappingFailure::resource("native filter stack allocation overflow"))?,
    )?;
    let filters = (0..repeat)
        .map(|pass| {
            let mut filter = native_filter("filter.stylize.diffuse", json!({"mode": "normal", "seed": seed.wrapping_add(pass)}));
            filter.opacity = probability as f32;
            filter
        })
        .collect();
    Ok(mapped(
        filters,
        "Native Diffuse approximates source neighboring-pixel picking. Source replacement probability becomes expected native filter blend weight rather than a Bernoulli selector; source repeat remains pass count and seeds select the native random sequence",
    ))
}

fn slur(e: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping> {
    let probability = number(e, "pct-random", 50.0, 0.0, 100.0)? / 100.0;
    let repeat = integer(e, "repeat", 1, 1, 100)?;
    let seed = seed(e)?;
    if probability == 0.0 {
        return Ok(identity());
    }
    super::residual_spatial::generation_bytes(
        context,
        (repeat as usize)
            .checked_mul(2048)
            .and_then(|n| n.checked_add(4096))
            .ok_or_else(|| MappingFailure::resource("native filter stack allocation overflow"))?,
    )?;
    let filters = (0..repeat)
        .map(|pass| {
            let mut filter = native_filter("filter.stylize.wind", json!({"method": "wind", "direction": "fromRight", "seed": seed.wrapping_add(pass)}));
            filter.opacity = probability as f32;
            filter
        })
        .collect();
    let mut mapping = mapped(
        filters,
        "Ordinary native clockwise quarter-turn placement, leftward Wind and inverse placement preserve downward source intent. Native directional streaks replace one-row random-neighbor copying; source replacement probability becomes expected blend weight and repeat remains pass count. Native hash/edge rules apply",
    );
    mapping.quarter_turns = 1;
    Ok(mapping)
}

fn difference(e: &StoredEffect<'_>) -> Result<NativeMapping> {
    let first = number(e, "radius1", 1.0, 0.0, 1000.0)?;
    let second = number(e, "radius2", 2.0, 0.0, 1000.0)?;
    if first == second {
        return Ok(one(
            "image.adjustments.channelMixer",
            json!({"red": [0,0,0,0], "green": [0,0,0,0], "blue": [0,0,0,0]}),
            "Equal source Gaussian scales cancel to zero color with preserved native alpha; this is not pixel identity",
        ));
    }
    let small = first.min(second);
    let large = first.max(second);
    let extra = (large * large - small * small).sqrt();
    native_range(extra, "additional native Gaussian sigma", 0.1, 1000.0)?;
    let mut filters = Vec::new();
    if small != 0.0 {
        filters.push(gaussian(small)?);
    }
    filters.push(native_filter("filter.other.highPass", json!({"radius": extra})));
    if first > second {
        filters.push(native_filter("image.adjustments.invert", json!({})));
    }
    filters.push(native_filter("image.adjustments.channelMixer", json!({"red": [100,0,0,-50], "green": [0,100,0,-50], "blue": [0,0,100,-50]})));
    Ok(mapped(
        filters,
        "Native Gaussian variances add: smooth at the smaller sigma, then High Pass at sqrt(large²-small²). Reversed scales add native Invert. Native Channel Mixer removes High Pass's 0.5 bias, matching the independently observed zero-color equal-scale source response. Native alpha/HDR/intermediate clipping and edge kernels apply; source normalization remains approximate",
    ))
}

fn high_pass(e: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping> {
    let radius = number(e, "std-dev", 4.0, 0.0, 10000.0)?;
    let contrast = number(e, "contrast", 1.0, 0.0, 5.0)?;
    let mut filters = Vec::new();
    let mut structure = None;
    if radius > 1000. {
        use super::{NativeBranch, NativeStructure};
        let count = (radius / 1000.).powi(2).ceil() as usize;
        // Two more branch filters encode the normalized difference and alpha.
        if count > 62 {
            return Err(MappingFailure::resource(
                "broad native High Pass needs more than62 Gaussian passes plus2 encoding filters; the native branch filter limit is64",
            ));
        }
        let sigma = radius / (count as f64).sqrt();
        gaussian_workspace(context, sigma, count + 6)?;
        let bytes = (count + 8) * 2048 + 8192;
        super::residual_spatial::generation_bytes(context, bytes)?;
        if bytes > context.remaining_metadata_bytes {
            return Err(MappingFailure::resource("broad native High Pass stack exceeds actual metadata allowance"));
        }
        let mut blurred = Vec::new();
        blurred.try_reserve_exact(count + 2).map_err(|_| MappingFailure::resource("native High Pass branch allocation"))?;
        for _ in 0..count {
            super::residual_spatial::generation_check(context)?;
            blurred.push(gaussian(sigma)?);
        }
        blurred.push(high_pass_curve(-0.125, 0.5625)?);
        blurred.push(native_filter("filter.other.setAlpha", json!({"value":1.})));
        structure = Some(NativeStructure::Composite {
            branches: vec![
                NativeBranch { filters: vec![high_pass_curve(0.25, 0.375)?], ..Default::default() },
                NativeBranch { filters: blurred, blend: BlendMode::LinearLight, clipped: true, ..Default::default() },
            ],
        });
        filters.push(high_pass_curve(4., -1.5)?);
    } else if radius != 0.0 {
        native_range(radius, "native high-pass sigma", 0.1, 1000.0)?;
        filters.push(native_filter("filter.other.highPass", json!({"radius": radius})));
    } else {
        // A zero-sigma high-pass subtracts the input from itself; it is not an identity.
        filters.push(native_filter("image.adjustments.channelMixer", json!({"red": [0,0,0,50], "green": [0,0,0,50], "blue": [0,0,0,50]})));
    }
    if contrast != 1.0 {
        // Three ordinary stages keep every coefficient within Channel Mixer's +/-200%.
        let factor = contrast.cbrt();
        for _ in 0..3 {
            let diagonal = factor * 100.0;
            let constant = (1.0 - factor) * 50.0;
            filters.push(native_filter(
                "image.adjustments.channelMixer",
                json!({"red": [diagonal,0,0,constant], "green": [0,diagonal,0,constant], "blue": [0,0,diagonal,constant]}),
            ));
        }
    }
    let mut mapping = mapped(
        filters,
        "Native High Pass sigma is pixels. Source contrast becomes three ordinary native Channel Mixer stages scaling about neutral 0.5, with bounded coefficients. Native integer intermediate clipping and native color/HDR rules can change strong contrast; source controls remain inert historical metadata",
    );
    mapping.structure = structure;
    if radius > 1000. {
        mapping.notes.push("Broad High Pass uses ordinary variance-split Gaussian, pencil Curves and clipped opaque Linear Light branches, then a native pencil decode. Normalized colors are compressed before the blend to avoid premature Linear Light clipping; native LUT interpolation/depth quantization approximate the neutral-centered input-minus-blur intent. Native 32-bit RGB Curves extrapolate endpoint slopes for signed/HDR inputs; other modes/depths use their ordinary normalized limits. Native output bounds, Gaussian alpha weighting and edge rules apply; this is not algebraic source equivalence. Original coverage is retained by the clipped branch; source controls are inert metadata".into());
    }
    Ok(mapping)
}

fn high_pass_curve(gain: f64, bias: f64) -> Result<SmartFilter> {
    let mut outputs = Vec::new();
    outputs.try_reserve_exact(256).map_err(|_| MappingFailure::resource("native High Pass pencil curve allocation"))?;
    for level in 0..256 {
        outputs.push((bias + gain * f64::from(level) / 255.).clamp(0., 1.) * 255.);
    }
    Ok(native_filter("image.adjustments.curves", json!({"pencil":true,"points":outputs})))
}

fn edge_blur(e: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping> {
    let (radius, threshold, repeat, note) = match e.canonical_operation() {
        "gegl:bilateral-filter" => {
            let r = number(e, "blur-radius", 4.0, 0.0, 1000.0)?;
            let edge = number(e, "edge-preservation", 8.0, 0.0, 100.0)?;
            (
                r,
                255.0 / (1.0 + edge),
                1,
                "The source edge index becomes a reciprocal native threshold policy 255/(1+index); this preserves stronger-edge/lower-threshold intent, not bilateral kernel equivalence",
            )
        }
        "gegl:gaussian-blur-selective" => {
            let r = number(e, "blur-radius", 5.0, 1.0, 1000.0)?;
            let delta = number(e, "max-delta", 0.2, 0.0, 1.0)?;
            if delta == 0.0 {
                return Ok(mapped(
                    Vec::new(),
                    "Chosen native zero-similarity policy leaves pixels unchanged instead of forcing the nonzero Surface Blur threshold minimum. Source zero-threshold equivalence has not been independently established",
                ));
            }
            (
                r,
                delta * 255.0,
                1,
                "The normalized source similarity threshold becomes native 8-bit threshold units. Native color-distance metric/weights replace the source selective neighborhood; numerical scaling is not formula equivalence",
            )
        }
        "gegl:snn-mean" => {
            let r = integer(e, "radius", 8, 0, 100)?;
            integer(e, "pairs", 2, 1, 2)?;
            (
                f64::from(r),
                15.0,
                1,
                "Source square-neighborhood radius sets native smoothing support. Source pairs1/2 chooses symmetric-pair comparison, and its public declaration says higher pairs preserves more acute features. This visible estimator/feature response is superseded by deterministic native Surface Blur at the explicitly chosen standard threshold15; pairs is not inactive, a pass count or a strength multiplier. Native color-distance/edge preservation, alpha and neighborhood rules apply",
            )
        }
        _ => {
            let r = number(e, "spatial-factor", 30.0, 0.0, 1000.0)?;
            let edge = number(e, "edge-preservation", 0.8, 0.0, 1.0)?;
            let repeat = integer(e, "n-iterations", 3, 1, 5)?;
            (
                r,
                2.0 + (1.0 - edge) * 253.0,
                repeat,
                "Native repeated Surface Blur preserves source pass count and spatial support. Source normalized edge intent maps monotonically to native threshold 2+(1-edge)*253; native neighborhood replaces domain-transform diffusion",
            )
        }
    };
    super::residual_spatial::generation_check(context)?;
    if radius == 0.0 {
        return Ok(mapped(
            Vec::new(),
            "Chosen native zero-support policy leaves pixels unchanged instead of forcing a nonzero Surface Blur radius. Source zero-support equivalence has not been independently established",
        ));
    }
    // Surface Blur is nonlinear: this partition is an explicit native support
    // policy, not a Gaussian variance identity. Reserve two surrounding stages.
    let partitions = (radius / 100.0).powi(2).ceil().max(1.0) as usize;
    let count = partitions.checked_mul(repeat as usize).ok_or_else(|| MappingFailure::resource("native Surface Blur pass count overflows"))?;
    if count > 62 {
        return Err(MappingFailure::resource(
            "native edge smoothing exceeds62 Surface Blur passes plus2 surrounding filters within the native64 filter stack limit",
        ));
    }
    let native_radius = (radius / (partitions as f64).sqrt()).max(1.0);
    let native_threshold = threshold.max(2.0);
    let bounds = context.native_filter_bounds;
    if bounds.is_empty() {
        return Err(invalid("native Surface Blur requires nonempty evaluation bounds"));
    }
    let tile = photocraft_geom::TILE_SIZE as usize;
    let axis = |a: i32, z: i32| -> Result<usize> {
        usize::try_from((i64::from(z) - 1).div_euclid(tile as i64) - i64::from(a).div_euclid(tile as i64) + 1)
            .map_err(|_| MappingFailure::resource("native Surface Blur tile extent is invalid"))
    };
    let tiles = axis(bounds.x0, bounds.x1)?
        .checked_mul(axis(bounds.y0, bounds.y1)?)
        .ok_or_else(|| MappingFailure::resource("native Surface Blur tile count overflows"))?;
    let channels = context.mode.color_channels() + 1;
    let reach = native_radius.round() as usize;
    let width = bounds.width() as usize;
    let height = bounds.height() as usize;
    // Include every potentially concurrent tile, not an assumed worker count.
    // Ranked native Surface Blur holds values+levels (8B), usize ranks (8B),
    // Fenwick triples (24B), transparency flags and output samples. 48B per
    // color sample conservatively covers those arrays and the histogram path.
    let scratch = width
        .min(tile)
        .checked_add(2 * reach)
        .and_then(|w| height.min(tile).checked_add(2 * reach).and_then(|h| w.checked_mul(h)))
        .and_then(|n| n.checked_mul((channels - 1) * 48 + 8))
        .and_then(|n| n.checked_mul(tiles));
    let caches = tiles.checked_mul(tile * tile).and_then(|n| n.checked_mul(channels * 4)).and_then(|n| n.checked_mul(count + 3));
    let metadata =
        count.checked_mul(2048).and_then(|n| n.checked_add(4096)).ok_or_else(|| MappingFailure::resource("native Surface Blur metadata size overflows"))?;
    let bytes = scratch
        .and_then(|s| caches.and_then(|c| s.checked_add(c)))
        .and_then(|n| n.checked_add(metadata))
        .ok_or_else(|| MappingFailure::resource("native Surface Blur workspace size overflows"))?;
    super::residual_spatial::generation_bytes(context, bytes)?;
    if metadata > context.remaining_metadata_bytes {
        return Err(MappingFailure::resource("native Surface Blur stack exceeds actual remaining metadata allowance"));
    }
    // The native fallback for nonfinite samples is the direct neighborhood
    // sum, so admission uses that worst-case sample-visit count at any depth.
    let work = width
        .checked_mul(height)
        .and_then(|n| n.checked_mul(channels - 1))
        .and_then(|n| n.checked_mul(count))
        .and_then(|n| n.checked_mul((2 * reach + 1).pow(2)));
    if work.is_none_or(|n| n > 1_000_000_000) {
        return Err(MappingFailure::resource("native edge smoothing exceeds one billion worst-case neighborhood sample visits"));
    }
    let mut filters = Vec::new();
    filters.try_reserve_exact(count).map_err(|_| MappingFailure::resource("native Surface Blur stack allocation"))?;
    for _ in 0..count {
        super::residual_spatial::generation_check(context)?;
        filters.push(native_filter("filter.blur.surfaceBlur", json!({"radius": native_radius, "threshold": native_threshold})));
    }
    let mut mapping = mapped(filters, note);
    if partitions > 1 {
        mapping.notes.push(format!("Broad support uses {partitions} ordinary native Surface Blur passes per source iteration, radius {native_radius}px, preserving {repeat} source iterations ({count} native passes). Radius is divided by sqrt(partition count) as a chosen multiscale support policy. Thresholded smoothing is nonlinear: repeated passes are not variance-equivalent to the source kernel or a single broad Surface Blur. Each pass uses native color-distance, intermediate-depth, coverage and boundary rules; original controls remain historical metadata"));
    }
    if radius < 1.0 || threshold < 2.0 {
        mapping.notes.push("Positive source support/similarity below the native dialog minimum uses the explicit native minimum radius1px and threshold2 levels, without a hidden opacity multiplier. The native resolution loses subminimum source distinctions; original values remain historical metadata".into());
    }
    Ok(mapping)
}

fn lens_blur(e: &StoredEffect<'_>) -> Result<NativeMapping> {
    let radius = number(e, "radius", 10.0, 0.0, f64::MAX)?;
    let highlight = number(e, "highlight-factor", 0.0, 0.0, 1.0)?;
    let low = number(e, "highlight-threshold-low", 0.0, -f64::MAX, f64::MAX)?;
    let high = number(e, "highlight-threshold-high", 1.0, -f64::MAX, f64::MAX)?;
    boolean(e, "clip")?;
    boolean(e, "linear-mask")?;
    if radius == 0.0 {
        return Ok(identity());
    }
    native_range(radius, "native Lens Blur radius", 0.0, 100.0)?;
    native_range(low, "native highlight threshold", 0.0, 1.0)?;
    if high < low {
        return Err(invalid("source highlight interval is reversed"));
    }
    Ok(one(
        "filter.blur.lensBlur",
        json!({"radius": radius, "shape": "hexagon", "depthMap": "none", "brightness": highlight * 100.0, "threshold": low * 255.0, "noise": 0}),
        "Native hexagonal Lens Blur radius/highlight boost and lower threshold approximate the source lens kernel. Upper threshold, source clipping and mask encoding remain disclosed. No unavailable auxiliary depth/mask is invented or read; native alpha and highlight behavior apply",
    ))
}

fn focus_blur(e: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping> {
    let blur = number(e, "blur-radius", 25.0, 0.0, 1500.0)?;
    choice(e, "blur-type", 0, &[0, 1])?;
    let shape = choice(e, "shape", 0, &[0, 1, 2, 3, 4])?;
    let x = number(e, "x", 0.5, -10.0, 10.0)?;
    let y = number(e, "y", 0.5, -10.0, 10.0)?;
    let radius = number(e, "radius", 0.75, 0.0, f64::MAX)?;
    let focus = number(e, "focus", 0.25, 0.0, 1.0)?;
    number(e, "midpoint", 0.5, 0.0, 1.0)?;
    let aspect = number(e, "aspect-ratio", 0.0, -1.0, 1.0)?;
    let rotation = number(e, "rotation", 0.0, -180.0, 180.0)?;
    number(e, "highlight-factor", 0.0, 0.0, 1.0)?;
    number(e, "highlight-threshold-low", 0.0, -f64::MAX, f64::MAX)?;
    number(e, "highlight-threshold-high", 1.0, -f64::MAX, f64::MAX)?;
    boolean(e, "high-quality")?;
    if blur == 0.0 {
        return Ok(identity());
    }
    if context.owner_bounds.is_empty() || context.native_filter_bounds.is_empty() {
        return Err(invalid("focus blur needs nonempty owner and native bounds"));
    }
    let owner = context.owner_bounds;
    let native = context.native_filter_bounds;
    let cx = (f64::from(owner.x0) + x * f64::from(owner.width()) - f64::from(native.x0)) / f64::from(native.width());
    let cy = (f64::from(owner.y0) + y * f64::from(owner.height()) - f64::from(native.y0)) / f64::from(native.height());
    let scale = f64::from(owner.width().min(owner.height())) / f64::from(native.width().min(native.height()));
    if radius == 0.0 {
        let passes = (blur / 1000.).powi(2).ceil().max(1.) as usize;
        let sigma = blur / (passes as f64).sqrt();
        gaussian_workspace(context, sigma.max(0.1), passes + 2)?;
        let bytes = passes * 2048 + 4096;
        if bytes > context.remaining_metadata_bytes {
            return Err(MappingFailure::resource("native zero-focus Gaussian stack exceeds actual metadata allowance"));
        }
        super::residual_spatial::generation_bytes(context, bytes)?;
        let mut filters = Vec::new();
        filters.try_reserve_exact(passes).map_err(|_| MappingFailure::resource("native zero-focus Gaussian stack allocation"))?;
        for _ in 0..passes {
            let mut filter = gaussian(sigma.max(0.1))?;
            if sigma < 0.1 {
                filter.opacity = (sigma / 0.1).powi(2) as f32;
            }
            filters.push(filter);
        }
        return Ok(mapped(
            filters,
            "Zero source focus radius leaves no focused region; full-frame ordinary native Gaussian filters preserve blur intent across the allowed0..1500 source sigma domain. Variance-split passes remain at most1000px sigma each; native sigma below0.1 uses fractional filter opacity(sigma/.1)^2. Actual native line/cache workspace, metadata and cancellation checks precede allocation. Native Gaussian, alpha and boundary rules replace source blur-type/lens highlights/midpoint and quality policies, without a minimum-radius artificial focus pin",
        ));
    }
    let passes = (blur / 500.).powi(2).ceil().max(1.) as usize;
    let component_blur = blur / (passes as f64).sqrt();
    if passes > 1 {
        // Gallery reads a two-dimensional expanded window, unlike the Gaussian
        // fast path's strips. Price every possible window concurrently, plus
        // the native tile caches; this deliberately bounds parallel replay too.
        let reach = (component_blur * 0.6 * 3. + 2.).ceil() as usize;
        let tile = (2 * reach).div_ceil(64).saturating_mul(64).clamp(256, 2048);
        let w = native.width() as usize;
        let h = native.height() as usize;
        let channels = context.mode.color_channels() + 1;
        let work = w
            .div_ceil(tile)
            .checked_mul(h.div_ceil(tile))
            .and_then(|n| n.checked_mul(w.min(tile).saturating_add(2 * reach)))
            .and_then(|n| n.checked_mul(h.min(tile).saturating_add(2 * reach)))
            .and_then(|n| n.checked_mul(channels * 4 * 5))
            .and_then(|n| {
                let caches = w.div_ceil(256).checked_mul(h.div_ceil(256))?.checked_mul(256 * 256 * channels * 4)?.checked_mul(passes + 2)?;
                n.checked_add(caches)
            })
            .ok_or_else(|| MappingFailure::resource("native focus Gallery workspace overflow"))?;
        super::residual_spatial::generation_bytes(context, work)?;
        if passes * 2048 + 4096 > context.remaining_metadata_bytes {
            return Err(MappingFailure::resource("native focus Gallery stack exceeds actual metadata allowance"));
        }
    }
    let r = native_range(radius * scale, "native relative focus radius", 0.001, 10.0)?;
    if shape >= 3 {
        let band_focus = native_range(r * focus, "native Tilt-Shift focus width", 0.0, 2.0)?;
        let transition = native_range((r * (1.0 - focus)).max(0.001), "native Tilt-Shift transition width", 0.001, 2.0)?;
        let mut mapping = one(
            "filter.blurGallery.tiltShift",
            json!({"blur": component_blur, "centerX": cx, "centerY": cy, "angle": rotation + if shape == 3 { 0.0 } else { 90.0 }, "focus": band_focus, "transition": transition}),
            "Source horizontal/vertical focus becomes an ordinary native Tilt-Shift band. Native transition, Gaussian kernel and alpha replace source midpoint, lens/bokeh and sampling policies; owner center is translated into native bounds",
        );
        repeat_focus_passes(&mut mapping, passes, context)?;
        return Ok(mapping);
    }
    let rx = (r * (1.0 + aspect)).max(0.001);
    let ry = (r * (1.0 - aspect)).max(0.001);
    let mut mapping = one(
        "filter.blurGallery.irisBlur",
        json!({"pins": [{"x": cx, "y": cy, "radiusX": rx, "radiusY": ry, "angle": rotation + if shape == 2 { 45.0 } else { 0.0 }, "roundness": if shape == 0 { 0 } else { 100 }, "feather": focus.min(0.99), "blur": component_blur}]}),
        "Source focus geometry becomes an editable native Iris Blur pin, with native-shorter-side radius and explicit linear aspect approximation. Diamond uses a rotated native rounded rectangle. Native feather, Gaussian kernel and alpha replace source midpoint, lens/bokeh/highlight and sampling policies",
    );
    repeat_focus_passes(&mut mapping, passes, context)?;
    Ok(mapping)
}

fn repeat_focus_passes(mapping: &mut NativeMapping, passes: usize, context: &MappingContext) -> Result<()> {
    if passes > 1 {
        let filter = mapping.filters.first().cloned().ok_or_else(|| invalid("missing native focus geometry"))?;
        mapping.filters.try_reserve_exact(passes - 1).map_err(|_| MappingFailure::resource("native focus stack allocation"))?;
        for _ in 1..passes {
            super::residual_spatial::generation_check(context)?;
            mapping.filters.push(filter.clone());
        }
        mapping.notes.push(format!("Broad focus uses {passes} ordinary native Gallery passes with identical focus geometry and variance-split maximum blur, each within the native500 slider. Spatially varying blur is nonlinear: repeated transitions and focused-edge leakage differ from a single broad source operation. Native Gaussian/alpha/boundary rules remain authoritative"));
    }
    Ok(())
}

fn spread(e: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping> {
    let x = integer(e, "amount-x", 5, 0, 512)?;
    let y = integer(e, "amount-y", 5, 0, 512)?;
    let seed = seed(e)?;
    let passes = x.max(y);
    if passes == 0 {
        return Ok(identity());
    }
    super::residual_spatial::generation_bytes(
        context,
        (passes as usize)
            .checked_mul(2048)
            .and_then(|n| n.checked_add(4096))
            .ok_or_else(|| MappingFailure::resource("native filter stack allocation overflow"))?,
    )?;
    let filters = (0..passes).map(|pass| native_filter("filter.stylize.diffuse", json!({"mode": "normal", "seed": seed.wrapping_add(pass)}))).collect();
    Ok(mapped(
        filters,
        format!(
            "Source {x}/{y}px random spread becomes {passes} native neighboring-pixel Diffuse passes. Native isotropic random walk replaces independent source axis extents/uniform displacement; maximum requested axis controls the bounded native support, with native seeds and edge behavior"
        ),
    ))
}

fn component_noise(e: &StoredEffect<'_>, context: &MappingContext, lch: bool) -> Result<NativeMapping> {
    let holdness = integer(e, "holdness", 2, 1, 8)?;
    let hue = number(e, "hue-distance", 3.0, 0.0, 180.0)? / 180.0;
    let (luma, chroma) = if lch {
        (number(e, "lightness-distance", 40.0, 0.0, 100.0)? / 100.0, number(e, "chroma-distance", 40.0, 0.0, 100.0)? / 100.0)
    } else {
        (number(e, "value-distance", 0.04, 0.0, 1.0)?, number(e, "saturation-distance", 0.04, 0.0, 1.0)?)
    };
    let seed = seed(e)?;
    if luma == 0.0 && chroma == 0.0 && hue == 0.0 {
        return Ok(identity());
    }
    let mut filters = Vec::new();
    for (index, amplitude, blend) in [(0u32, luma, BlendMode::Luminosity), (1, chroma, BlendMode::Saturation), (2, hue, BlendMode::Hue)] {
        if amplitude == 0.0 {
            continue;
        }
        let amount = 200.0 * amplitude;
        let mut filter = native_filter(
            "filter.noise.addNoise",
            json!({"amount": amount.max(0.1), "distribution": "uniform", "monochromatic": index == 0 || context.mode == ColorMode::Grayscale, "seed": seed.wrapping_add(index)}),
        );
        filter.blend = blend;
        if amount < 0.1 {
            filter.opacity = (amount / 0.1) as f32;
        }
        filters.push(filter);
    }
    Ok(mapped(
        filters,
        format!(
            "Source {} component distances normalize hue/180 and {} into separate native Add Noise stages blended with native Luminosity, Saturation and Hue. This preserves editable native component intent and alpha without replacing source color science: native Photoshop-compatible component formulas/CMS replace source CIE/HSV sampling and holdness {holdness}; all source values remain retained. Native noise sequences and integer clipping differ",
            if lch { "LCh" } else { "HSV" },
            if lch { "L/chroma divided by 100" } else { "saturation/value already normalized" }
        ),
    ))
}

fn hurl(e: &StoredEffect<'_>) -> Result<NativeMapping> {
    let probability = number(e, "pct-random", 50.0, 0.0, 100.0)? / 100.0;
    let repeat = integer(e, "repeat", 1, 1, 100)?;
    let seed = seed(e)?;
    if probability == 0.0 {
        return Ok(identity());
    }
    // Independent replacement passes retain the original with probability(1-p)^n.
    let affected = 1.0 - (1.0 - probability).powf(f64::from(repeat));
    let diagonal = (1.0 - affected) * 100.0;
    let constant = affected * 50.0;
    let color = native_filter(
        "image.adjustments.channelMixer",
        json!({"red": [diagonal,0,0,constant], "green": [0,diagonal,0,constant], "blue": [0,0,diagonal,constant]}),
    );
    let amount = 100.0 * affected.sqrt();
    let mut noise = native_filter("filter.noise.addNoise", json!({"amount": amount.max(0.1), "distribution": "uniform", "monochromatic": false, "seed": seed}));
    if amount < 0.1 {
        noise.opacity = (amount / 0.1) as f32;
    }
    Ok(mapped(
        vec![color, noise],
        format!(
            "Source replacement probability/repeat become affected probability 1-(1-p)^n={affected}. Native Channel Mixer preserves expected color(1-p)*input+p*0.5; native uniform Add Noise supplies replacement variance p/12. The Bernoulli mixture's signal-dependent variance/pixel selector and random sequence are explicitly approximate; source probability is not copied as noise amplitude"
        ),
    ))
}

pub(super) fn map(e: &StoredEffect<'_>, context: &MappingContext) -> MappingResult {
    if !matches!(
        e.canonical_operation(),
        "gegl:gaussian-blur"
            | "gegl:box-blur"
            | "gegl:motion-blur-linear"
            | "gegl:motion-blur-circular"
            | "gegl:motion-blur-zoom"
            | "gegl:median-blur"
            | "gegl:unsharp-mask"
            | "gegl:noise-rgb"
            | "gegl:noise-pick"
            | "gegl:noise-slur"
            | "gegl:noise-reduction"
            | "gegl:mean-curvature-blur"
            | "gegl:variable-blur"
            | "gegl:difference-of-gaussians"
            | "gegl:high-pass"
            | "gegl:bilateral-filter"
            | "gegl:gaussian-blur-selective"
            | "gegl:snn-mean"
            | "gegl:domain-transform"
            | "gegl:lens-blur"
            | "gegl:focus-blur"
            | "gegl:noise-spread"
            | "gegl:denoise-dct"
            | "gegl:noise-cie-lch"
            | "gegl:noise-hsv"
            | "gegl:noise-hurl"
    ) {
        return None;
    }
    if let Err(error) = super::residual_spatial::generation_bytes(context, 4096) {
        return Some(Err(error));
    }
    let result = match e.canonical_operation() {
        "gegl:gaussian-blur" => gaussian_map(e, context),
        "gegl:box-blur" => (|| {
            let radius = integer(e, "radius", 4, 0, 1000)?;
            if radius == 0 {
                return Ok(identity());
            }
            Ok(one(
                "filter.blur.boxBlur",
                json!({"radius": radius}),
                "Source pixel neighborhood radius becomes native box half-width; native kernel, alpha and edges apply",
            ))
        })(),
        "gegl:motion-blur-linear" => (|| {
            let distance = number(e, "length", 10.0, 0.0, 1000.0)?;
            let angle = number(e, "angle", 0.0, -180.0, 180.0)?;
            if distance == 0.0 {
                return Ok(identity());
            }
            native_range(distance, "native motion distance", 1.0, 2000.0)?;
            Ok(one(
                "filter.blur.motionBlur",
                json!({"distance": distance, "angle": -angle}),
                "Source clockwise image-space degrees become negative native mathematical degrees. Independently observed+45degree source impulse runs down-right; native angle sign is reversed. Native centered line sampling, alpha and edges apply",
            ))
        })(),
        "gegl:motion-blur-circular" => radial(e, context, false),
        "gegl:motion-blur-zoom" => radial(e, context, true),
        "gegl:median-blur" => median(e),
        "gegl:unsharp-mask" => (|| {
            let radius = number(e, "std-dev", 3.0, 0.0, 1500.0)?;
            let scale = number(e, "scale", 0.5, 0.0, 300.0)?;
            let threshold = number(e, "threshold", 0.0, 0.0, 1.0)?;
            if radius == 0.0 || scale == 0.0 {
                return Ok(identity());
            }
            if radius > 1000. {
                return broad_unsharp(radius, scale, threshold, context);
            }
            native_range(radius.max(0.1), "native unsharp sigma", 0.1, 1000.0)?;
            let passes = (scale.ln_1p() / 6f64.ln()).ceil().max(1.) as usize;
            super::residual_spatial::generation_bytes(context, passes * 2048 + 4096)?;
            let relative = if passes == 1 { scale } else { (scale.ln_1p() / passes as f64).exp_m1() };
            let amount = relative * 100.;
            let weak = if radius < 0.1 { (radius / 0.1).powi(2) } else { 1. } * (amount / 1.).min(1.);
            let filters = (0..passes)
                .map(|_| {
                    let mut f =
                        native_filter("filter.sharpen.unsharpMask", json!({"radius":radius.max(0.1),"amount":amount.max(1.),"threshold":threshold*255.}));
                    f.opacity = weak as f32;
                    f
                })
                .collect();
            Ok(mapped(
                filters,
                "Native unsharp uses sigma, percent amount and 8-bit threshold. Large source gain uses bounded repeated native passes whose (1+gain) product preserves high-frequency gain; intermediate frequencies, clipping and repeated native thresholding differ from a single source pass. Tiny radius/amount uses explicit fractional minimum-kernel strength, never silent full minimum sharpening. Native color/HDR/alpha apply",
            ))
        })(),
        "gegl:noise-rgb" => rgb_noise(e, context),
        "gegl:noise-pick" => neighbor_noise(e, context),
        "gegl:noise-slur" => slur(e, context),
        "gegl:noise-reduction" => (|| repeated_denoise(e, integer(e, "iterations", 4, 0, 32)?, context))(),
        "gegl:mean-curvature-blur" => (|| repeated_denoise(e, integer(e, "iterations", 20, 0, 500)?, context))(),
        "gegl:variable-blur" => (|| {
            let radius = number(e, "radius", 10.0, 0.0, 1500.0)?;
            boolean(e, "linear-mask")?;
            boolean(e, "high-quality")?;
            if radius == 0.0 {
                return Ok(identity());
            }
            Err(MappingFailure::resource(
                "Variable spatial blur requires the actual auxiliary radius-mask input; scalar radius cannot invent native Field Blur pins or a depth resource",
            ))
        })(),
        "gegl:difference-of-gaussians" => difference(e),
        "gegl:high-pass" => high_pass(e, context),
        "gegl:bilateral-filter" | "gegl:gaussian-blur-selective" | "gegl:snn-mean" | "gegl:domain-transform" => edge_blur(e, context),
        "gegl:lens-blur" => lens_blur(e),
        "gegl:focus-blur" => focus_blur(e, context),
        "gegl:noise-spread" => spread(e, context),
        "gegl:denoise-dct" => (|| {
            let patch = choice(e, "patch-size", 0, &[0, 1])?;
            let sigma = number(e, "sigma", 5.0, 1.0, 100.0)?;
            Ok(one(
                "filter.blur.smartBlur",
                json!({"radius": if patch == 0 { 4 } else { 8 }, "threshold": sigma, "quality": "high", "mode": "normal"}),
                "Source 8x8/16x16 patch support becomes native Smart Blur radius4/8. Source noise sigma supplies the native level-threshold policy, not an asserted equal DCT response. Native spatial edge-aware denoise replaces transform-domain shrinkage; all source configuration remains retained",
            ))
        })(),
        "gegl:noise-cie-lch" => component_noise(e, context, true),
        "gegl:noise-hsv" => component_noise(e, context, false),
        "gegl:noise-hurl" => hurl(e),
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
            owner_bounds: Rect::new(10, 20, 110, 70),
            native_filter_bounds: Rect::new(0, 0, 200, 100),
            target_profile: None,
            resolution_dpi: 72.0,
            remaining_generation_bytes: super::super::super::MAX_PIXEL_BYTES,
            remaining_metadata_bytes: crate::xcf::MAX_METADATA_BYTES,
            interrupt: photocraft_raster::Interrupt::NONE,
        }
    }
    fn float(name: &'static str, value: f32) -> Argument<'static> {
        Argument { name, value: ArgumentValue::Float(value) }
    }
    fn int(name: &'static str, value: i32) -> Argument<'static> {
        Argument { name, value: ArgumentValue::Int(value) }
    }
    fn enm(name: &'static str, value: i32) -> Argument<'static> {
        Argument { name, value: ArgumentValue::Enum(value) }
    }
    fn mapped_effect(operation: &'static str, arguments: Vec<Argument<'static>>) -> NativeMapping {
        map(&StoredEffect::test(operation, arguments), &context()).unwrap().unwrap()
    }

    #[test]
    fn snn_pair_estimator_is_superseded_by_deterministic_native_surface_blur() {
        use photocraft_color::{PixelFormat, SampleType};
        use photocraft_doc::{Document, Layer, LayerContent, SmartObject, SmartSource};
        use photocraft_geom::{Affine, Size};
        use photocraft_raster::{Interrupt, Surface};
        let first = mapped_effect("gegl:snn-mean", vec![int("radius", 2), int("pairs", 1)]);
        let second = mapped_effect("gegl:snn-mean", vec![int("radius", 2), int("pairs", 2)]);
        assert_eq!(first.filters, second.filters);
        assert!(first.notes[0].contains("acute features"));
        assert!(first.notes[0].contains("superseded"));
        let bounds = Rect::new(0, 0, 5, 5);
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut source = Surface::new(PixelFormat::new(ColorMode::Rgb, depth, true));
            source.write_region(
                bounds,
                &(0..25)
                    .flat_map(|i| {
                        [
                            if i == 12 {
                                0.51
                            } else if i % 5 > 2 {
                                0.9
                            } else {
                                0.5
                            },
                            0.25,
                            0.75,
                            i as f32 / 24.,
                        ]
                    })
                    .collect::<Vec<_>>(),
            );
            let params = &first.filters[0].params;
            let apply = |radius, threshold| {
                photocraft_algo::apply_in_with(
                    &source,
                    &photocraft_algo::FilterParams::SurfaceBlur { radius, threshold },
                    bounds,
                    bounds,
                    None,
                    bounds,
                    &Interrupt::NONE,
                )
                .unwrap()
                .read_region(bounds)
            };
            let actual = apply(params["radius"].as_f64().unwrap() as f32, params["threshold"].as_f64().unwrap() as f32);
            assert_eq!(actual, apply(2., 15.));
            for (before, after) in source.read_region(bounds).as_chunks::<4>().0.iter().zip(actual.as_chunks::<4>().0) {
                assert_eq!(before[3], after[3]);
            }
            let mut input = Document::new("Native SNN input", Size::new(5, 5), ColorMode::Rgb, depth);
            input.layers.push(Layer::new("Pixels", LayerContent::Raster(source)));
            let bytes = photocraft_format::save_to_bytes(&input, &Default::default()).unwrap();
            let mut smart =
                SmartObject::new(SmartSource::Embedded { file_name: "input.pcraft".into(), bytes: std::sync::Arc::new(bytes) }, Affine::IDENTITY, None);
            smart.smart_filters = first.filters.clone();
            input.layers = vec![Layer::new("Editable native Surface Blur", LayerContent::Smart(smart))];
            let bytes = photocraft_format::save_to_bytes(&input, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
            let LayerContent::Smart(restored) = &reopened.layers[0].content else { panic!("native Surface Blur") };
            assert_eq!(restored.smart_filters, first.filters);
        }
    }
    #[test]
    fn exact_frozen_batch_has25_default_native_mappings_and_one_explicit_resource_gap() {
        let operations = [
            "bilateral-filter",
            "box-blur",
            "denoise-dct",
            "difference-of-gaussians",
            "domain-transform",
            "focus-blur",
            "gaussian-blur",
            "gaussian-blur-selective",
            "high-pass",
            "lens-blur",
            "mean-curvature-blur",
            "median-blur",
            "motion-blur-circular",
            "motion-blur-linear",
            "motion-blur-zoom",
            "noise-cie-lch",
            "noise-hsv",
            "noise-hurl",
            "noise-pick",
            "noise-reduction",
            "noise-rgb",
            "noise-slur",
            "noise-spread",
            "snn-mean",
            "unsharp-mask",
            "variable-blur",
        ];
        let mut accepted = 0;
        for name in operations {
            let operation = format!("gegl:{name}");
            let effect = StoredEffect::test(&operation, Vec::new());
            let result = map(&effect, &context()).unwrap();
            if name == "variable-blur" {
                assert!(result.is_err(), "{name}");
            } else {
                let result = result.unwrap();
                accepted += 1;
                assert!(!result.filters.is_empty(), "{name}");
                assert!(result.filters.iter().all(|f| !f.command.starts_with("gegl:") && !f.command.starts_with("gimp:")));
                assert!(!result.notes.is_empty());
            }
        }
        assert_eq!(accepted, 25);
        assert!(map(&StoredEffect::test("gegl:not-in-frozen-b05", Vec::new()), &context()).is_none());
    }

    #[test]
    fn registered_motion_alias_uses_same_native_mapping() {
        let canonical = mapped_effect("gegl:motion-blur-linear", vec![float("length", 12.), float("angle", 35.)]);
        let alias = mapped_effect("gegl:motion-blur", vec![float("length", 12.), float("angle", 35.)]);
        assert_eq!(canonical.filters, alias.filters);
        assert_eq!(canonical.notes, alias.notes);
    }
    #[test]
    fn gaussian_uses_both_axis_variances_and_native_sigma_units() {
        let result = mapped_effect("gegl:gaussian-blur", vec![float("std-dev-x", 3.0), float("std-dev-y", 4.0)]);
        assert_eq!(result.filters[0].command, "filter.blur.gaussianBlur");
        assert!((result.filters[0].params["radius"].as_f64().unwrap() - 12.5f64.sqrt()).abs() < 1e-12);
        assert!(result.notes[0].contains("unequal axes become isotropic"));
        let same = mapped_effect("gegl:gaussian-blur", vec![float("std-dev-x", 3.0), float("std-dev-y", 3.0)]);
        assert_eq!(same.filters[0].params, json!({"radius": 3.0}));
        let large = mapped_effect("gegl:gaussian-blur", vec![float("std-dev-x", 1500.0), float("std-dev-y", 1500.0)]);
        assert_eq!(large.filters.len(), 3);
        let variance: f64 = large.filters.iter().map(|f| f.params["radius"].as_f64().unwrap().powi(2)).sum();
        assert!((variance - 2_250_000.0).abs() < 1e-6);
    }

    #[test]
    fn zero_spatial_strengths_do_not_become_native_minimum_radius() {
        for (operation, arguments) in [
            ("gegl:box-blur", vec![int("radius", 0)]),
            ("gegl:gaussian-blur", vec![float("std-dev-x", 0.0), float("std-dev-y", 0.0)]),
            ("gegl:motion-blur-linear", vec![float("length", 0.0)]),
            ("gegl:motion-blur-circular", vec![float("angle", 0.0)]),
            ("gegl:motion-blur-zoom", vec![float("factor", 0.0)]),
            ("gegl:median-blur", vec![int("radius", 0)]),
            ("gegl:unsharp-mask", vec![float("scale", 0.0)]),
            ("gegl:mean-curvature-blur", vec![int("iterations", 0)]),
            ("gegl:focus-blur", vec![float("blur-radius", 0.0)]),
            ("gegl:variable-blur", vec![float("radius", 0.0)]),
        ] {
            assert!(mapped_effect(operation, arguments).filters.is_empty(), "{operation}");
        }
        assert!(!mapped_effect("gegl:high-pass", vec![float("std-dev", 0.0)]).filters.is_empty());
        assert_eq!(
            mapped_effect("gegl:difference-of-gaussians", vec![float("radius1", 2.0), float("radius2", 2.0)]).filters[0].params["red"],
            json!([0, 0, 0, 0])
        );
    }

    #[test]
    fn motion_sign_centers_and_zoom_span_match_independent_native_conventions() {
        let linear = mapped_effect("gegl:motion-blur-linear", vec![float("angle", 45.0), float("length", 6.0)]);
        assert_eq!(linear.filters[0].params, json!({"angle": -45.0,"distance":6.0}));
        let spin = mapped_effect("gegl:motion-blur-circular", vec![float("center-x", 0.25), float("center-y", 0.4), float("angle", 30.0)]);
        assert!((spin.filters[0].params["centerX"].as_f64().unwrap() - 0.175).abs() < 1e-12);
        assert!((spin.filters[0].params["centerY"].as_f64().unwrap() - 0.4).abs() < 1e-7);
        assert_eq!(spin.filters[0].params["amount"], json!(30.0));
        let zoom = mapped_effect("gegl:motion-blur-zoom", vec![float("factor", 0.25)]);
        assert_eq!(zoom.filters[0].params["amount"], json!(50.0));
    }

    #[test]
    fn difference_preserves_order_variances_and_removes_native_highpass_bias() {
        let result = mapped_effect("gegl:difference-of-gaussians", vec![float("radius1", 3.0), float("radius2", 5.0)]);
        assert_eq!(
            result.filters.iter().map(|f| f.command.as_str()).collect::<Vec<_>>(),
            ["filter.blur.gaussianBlur", "filter.other.highPass", "image.adjustments.channelMixer"]
        );
        assert_eq!(result.filters[0].params, json!({"radius":3.0}));
        assert_eq!(result.filters[1].params, json!({"radius":4.0}));
        assert_eq!(result.filters[2].params["red"], json!([100, 0, 0, -50]));
        let reversed = mapped_effect("gegl:difference-of-gaussians", vec![float("radius1", 5.0), float("radius2", 3.0)]);
        assert_eq!(reversed.filters[2].command, "image.adjustments.invert");
    }

    #[test]
    fn unsharp_and_highpass_contrast_use_native_units_and_bounded_ordinary_stages() {
        let result = mapped_effect("gegl:unsharp-mask", vec![float("std-dev", 2.0), float("scale", 1.5), float("threshold", 0.25)]);
        assert_eq!(result.filters[0].params, json!({"radius":2.0,"amount":150.0,"threshold":63.75}));
        let contrast = mapped_effect("gegl:high-pass", vec![float("std-dev", 2.0), float("contrast", 3.375)]);
        assert_eq!(contrast.filters.len(), 4);
        for stage in &contrast.filters[1..] {
            let red = stage.params["red"].as_array().unwrap();
            assert!((red[0].as_f64().unwrap() - 150.0).abs() < 1e-12);
            assert!((red[3].as_f64().unwrap() + 25.0).abs() < 1e-12);
        }
    }

    #[test]
    fn median_inverts_negative_radius_ranks_and_never_discards_alpha_rank() {
        let maximum =
            mapped_effect("gegl:median-blur", vec![int("radius", -2), float("percentile", 0.0), float("alpha-percentile", 0.0), enm("neighborhood", 0)]);
        assert_eq!(maximum.filters[0].command, "filter.other.percentile");
        assert_eq!(maximum.filters[0].params, json!({"radius":2,"colorPercentile":100.0,"alphaPercentile":100.0,"shape":"square"}));
        let effect = StoredEffect::test("gegl:median-blur", vec![float("percentile", 50.0), float("alpha-percentile", 25.0)]);
        let mapped = map(&effect, &context()).unwrap().unwrap();
        assert_eq!(mapped.filters[0].params["alphaPercentile"], json!(25.0));
        assert_eq!(mapped.filters[0].params["colorPercentile"], json!(50.0));
        for neighborhood in 0..=2 {
            let e = StoredEffect::test("gegl:median-blur", vec![enm("neighborhood", neighborhood), float("percentile", 17.0), float("alpha-percentile", 83.0)]);
            let m = map(&e, &context()).unwrap().unwrap();
            assert_eq!(m.filters[0].command, "filter.other.percentile");
            assert_eq!(m.filters[0].params["colorPercentile"], json!(17.0));
            assert_eq!(m.filters[0].params["alphaPercentile"], json!(83.0));
        }
    }

    #[test]
    fn noise_uses_independent_source_amplitude_probe_and_explicit_probability_conversion() {
        let arguments = vec![float("red", 0.25), float("green", 0.25), float("blue", 0.25), Argument { name: "seed", value: ArgumentValue::UInt(u32::MAX) }];
        let noise = mapped_effect("gegl:noise-rgb", arguments);
        assert_eq!(noise.filters[0].command, "filter.noise.channelNoise");
        assert_eq!(noise.filters[0].params["amounts"], json!([50.0, 50.0, 50.0, 0.0]));
        assert_eq!(noise.filters[0].params["seed"], json!(u32::MAX));
        let pick = mapped_effect("gegl:noise-pick", vec![float("pct-random", 25.0), int("repeat", 3)]);
        assert_eq!(pick.filters.len(), 3);
        assert!(pick.filters.iter().all(|f| f.opacity == 0.25));
        let slur = mapped_effect("gegl:noise-slur", vec![float("pct-random", 25.0), int("repeat", 3)]);
        assert_eq!(slur.quarter_turns, 1);
        assert!(slur.filters.iter().all(|f| f.params["direction"] == json!("fromRight") && f.opacity == 0.25));
        let hurl = mapped_effect("gegl:noise-hurl", vec![float("pct-random", 50.0), int("repeat", 3)]);
        assert_eq!(hurl.filters[0].params["red"], json!([12.5, 0, 0, 43.75]));
        assert!((hurl.filters[1].params["amount"].as_f64().unwrap() - 100.0 * 0.875f64.sqrt()).abs() < 1e-12);
    }

    #[test]
    fn channel_noise_preserves_unequal_alpha_and_relative_controls_in_native_data() {
        let e = StoredEffect::test(
            "gegl:noise-rgb",
            vec![
                float("red", 0.1),
                float("green", 0.3),
                float("blue", 0.6),
                float("alpha", 0.2),
                Argument { name: "correlated", value: ArgumentValue::Bool(true) },
            ],
        );
        let m = map(&e, &context()).unwrap().unwrap();
        for (actual, expected) in m.filters[0].params["amounts"].as_array().unwrap().iter().zip([20., 60., 120., 40.]) {
            assert!((actual.as_f64().unwrap() - expected).abs() < 1e-5);
        }
        assert_eq!(m.filters[0].params["relative"], json!(true));
        let mut gray = context();
        gray.mode = ColorMode::Grayscale;
        let m = map(&e, &gray).unwrap().unwrap();
        let amounts = m.filters[0].params["amounts"].as_array().unwrap();
        assert_eq!(amounts.len(), 2);
        assert!((amounts[0].as_f64().unwrap() - ((0.01 + 0.09 + 0.36) / 3.0f64).sqrt() * 200.).abs() < 1e-5);
        assert!((amounts[1].as_f64().unwrap() - 40.).abs() < 1e-5);
    }

    #[test]
    fn bounded_source_repetitions_are_preserved_above_the_old_arbitrary_cap() {
        for (operation, args, count) in [
            ("gegl:mean-curvature-blur", vec![int("iterations", 500)], 500),
            ("gegl:noise-pick", vec![int("repeat", 100)], 100),
            ("gegl:noise-slur", vec![int("repeat", 100)], 100),
            ("gegl:noise-spread", vec![int("amount-x", 512), int("amount-y", 0)], 512),
        ] {
            assert_eq!(mapped_effect(operation, args).filters.len(), count, "{operation}");
        }
    }

    #[test]
    fn focus_zero_region_uses_full_frame_blur_and_component_noise_keeps_native_blends() {
        let zero = mapped_effect("gegl:focus-blur", vec![float("radius", 0.0)]);
        assert_eq!(zero.filters[0].command, "filter.blur.gaussianBlur");
        let luma = mapped_effect("gegl:noise-hsv", vec![float("hue-distance", 0.0), float("saturation-distance", 0.0), float("value-distance", 0.1)]);
        assert_eq!(luma.filters.len(), 1);
        assert_eq!(luma.filters[0].blend, BlendMode::Luminosity);
        let failure = map(&StoredEffect::test("gegl:focus-blur", vec![enm("shape", 3), float("radius", 10.0)]), &context()).unwrap().unwrap_err();
        assert_eq!(failure.reason, ConversionReason::MissingMapping);
    }

    #[test]
    fn zero_focus_full_sigma_domain_uses_bounded_native_variance_and_persists() {
        use photocraft_color::{PixelFormat, SampleType};
        use photocraft_doc::{Document, Layer, LayerContent, SmartObject, SmartSource};
        use photocraft_geom::{Affine, Size};
        use photocraft_raster::{Interrupt, Surface};
        let bounds = Rect::new(0, 0, 3, 2);
        let c = MappingContext { owner_bounds: bounds, native_filter_bounds: bounds, ..context() };
        for requested in [600., 1500.] {
            let effect = StoredEffect::test("gegl:focus-blur", vec![float("radius", 0.), float("blur-radius", requested)]);
            let mapping = focus_blur(&effect, &c).unwrap();
            assert!(mapping.filters.iter().all(|filter| filter.command == "filter.blur.gaussianBlur"));
            let variance: f64 = mapping.filters.iter().map(|filter| filter.params["radius"].as_f64().unwrap().powi(2)).sum();
            assert!((variance - f64::from(requested).powi(2)).abs() < 1e-6);
            let count = if requested == 600. { 1 } else { 3 };
            let sigma = (f64::from(requested) / (count as f64).sqrt()) as f32;
            assert_eq!(mapping.filters.len(), count);
            for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
                let mut source = Surface::new(PixelFormat::new(ColorMode::Rgb, depth, true));
                source.write_region(bounds, &(0..6).flat_map(|i| [i as f32 / 5., 0.25, 0.75, 0.25 + i as f32 / 8.]).collect::<Vec<_>>());
                let apply = |source: &Surface, sigma| {
                    photocraft_algo::apply_in_with(
                        source,
                        &photocraft_algo::FilterParams::GaussianBlur { radius: sigma },
                        bounds,
                        bounds,
                        None,
                        bounds,
                        &Interrupt::NONE,
                    )
                    .unwrap()
                };
                let mut actual = source.clone();
                for filter in &mapping.filters {
                    actual = apply(&actual, filter.params["radius"].as_f64().unwrap() as f32);
                }
                let mut expected = source.clone();
                for _ in 0..count {
                    expected = apply(&expected, sigma);
                }
                assert_eq!(actual.read_region(bounds), expected.read_region(bounds));
                let mut input = Document::new("Native zero-focus input", Size::new(3, 2), ColorMode::Rgb, depth);
                input.layers.push(Layer::new("Pixels", LayerContent::Raster(source)));
                let bytes = photocraft_format::save_to_bytes(&input, &Default::default()).unwrap();
                let mut smart =
                    SmartObject::new(SmartSource::Embedded { file_name: "input.pcraft".into(), bytes: std::sync::Arc::new(bytes) }, Affine::IDENTITY, None);
                smart.smart_filters = mapping.filters.clone();
                input.layers = vec![Layer::new("Native Gaussian stack", LayerContent::Smart(smart))];
                let bytes = photocraft_format::save_to_bytes(&input, &Default::default()).unwrap();
                let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
                let LayerContent::Smart(restored) = &reopened.layers[0].content else { panic!("native Gaussian stack") };
                assert_eq!(restored.smart_filters, mapping.filters);
            }
        }
        let tiny = focus_blur(&StoredEffect::test("gegl:focus-blur", vec![float("radius", 0.), float("blur-radius", 0.01)]), &c).unwrap();
        assert_eq!(tiny.filters[0].params["radius"], 0.1);
        assert!((tiny.filters[0].opacity - 0.01).abs() < 1e-6);
        assert_eq!(focus_blur(&StoredEffect::test("gegl:focus-blur", vec![float("blur-radius", 600.)]), &c).unwrap().filters.len(), 2);
        for c in [MappingContext { remaining_generation_bytes: 1, ..context() }, MappingContext { remaining_metadata_bytes: 1, ..context() }] {
            assert!(focus_blur(&StoredEffect::test("gegl:focus-blur", vec![float("radius", 0.), float("blur-radius", 1500.)]), &c).is_err());
        }
        let cancel = || true;
        assert!(
            focus_blur(
                &StoredEffect::test("gegl:focus-blur", vec![float("radius", 0.), float("blur-radius", 1500.)]),
                &MappingContext { interrupt: Interrupt::cancel_only(&cancel), ..context() }
            )
            .is_err()
        );
    }
    #[test]
    fn broad_focus_preserves_native_geometry_replay_depth_and_persistence() {
        use photocraft_color::{PixelFormat, SampleType};
        use photocraft_doc::{Document, Layer, LayerContent, SmartObject, SmartSource};
        use photocraft_geom::{Affine, Size};
        use photocraft_raster::{Interrupt, Surface};
        let bounds = Rect::new(0, 0, 3, 2);
        let c = MappingContext { owner_bounds: bounds, native_filter_bounds: bounds, ..context() };
        for shape in [0, 3] {
            for requested in [600., 1500.] {
                let effect = StoredEffect::test("gegl:focus-blur", vec![float("blur-radius", requested), enm("shape", shape)]);
                let mapping = focus_blur(&effect, &c).unwrap();
                let count = if requested == 600. { 2 } else { 9 };
                let blur = f64::from(requested) / (count as f64).sqrt();
                assert_eq!(mapping.filters.len(), count);
                assert!(mapping.notes.iter().any(|n| n.contains("nonlinear")));
                let params = if shape == 0 {
                    json!({"pins":[{"x":0.5,"y":0.5,"radiusX":0.75,"radiusY":0.75,"angle":0.,"roundness":0,"feather":0.25,"blur":blur}]})
                } else {
                    json!({"blur":blur,"centerX":0.5,"centerY":0.5,"angle":0.,"focus":0.1875,"transition":0.5625})
                };
                assert!(mapping.filters.iter().all(|f| f.params == params && f.opacity == 1.));
                for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
                    let mut surface = Surface::new(PixelFormat::new(ColorMode::Rgb, depth, true));
                    surface.write_region(
                        bounds,
                        &[0.2, 0.3, 0.4, 0.25, 0.7, 0.3, 0.2, 0.5, 0.4, 0.6, 0.8, 1., 0.8, 0.2, 0.1, 0.75, 0.2, 0.3, 0.4, 0.4, 0.6, 0.7, 0.8, 0.9],
                    );
                    let independent = if shape == 0 {
                        photocraft_algo::FilterParams::IrisBlur {
                            pins: vec![photocraft_algo::IrisPin {
                                x: 0.5,
                                y: 0.5,
                                radius_x: 0.75,
                                radius_y: 0.75,
                                angle: 0.,
                                roundness: 0.,
                                feather: 0.25,
                                blur: blur as f32,
                            }],
                        }
                    } else {
                        photocraft_algo::FilterParams::TiltShift {
                            blur: blur as f32,
                            center_x: 0.5,
                            center_y: 0.5,
                            angle: 0.,
                            focus: 0.1875,
                            transition: 0.5625,
                        }
                    };
                    // Independent native API counterpart; not a source-fidelity oracle.
                    let mut expected = surface.clone();
                    for _ in 0..count {
                        expected = photocraft_algo::apply_in_with(&expected, &independent, bounds, bounds, None, bounds, &Interrupt::NONE).unwrap();
                    }
                    assert!(expected.read_region(bounds).iter().all(|v| v.is_finite()));
                    let mut input = Document::new("Native focused input", Size::new(3, 2), ColorMode::Rgb, depth);
                    input.layers.push(Layer::new("Pixels", LayerContent::Raster(surface)));
                    let bytes = photocraft_format::save_to_bytes(&input, &Default::default()).unwrap();
                    let mut smart =
                        SmartObject::new(SmartSource::Embedded { file_name: "input.pcraft".into(), bytes: std::sync::Arc::new(bytes) }, Affine::IDENTITY, None);
                    smart.smart_filters = mapping.filters.clone();
                    input.layers = vec![Layer::new("Editable native focus", LayerContent::Smart(smart))];
                    let bytes = photocraft_format::save_to_bytes(&input, &Default::default()).unwrap();
                    let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
                    let LayerContent::Smart(restored) = &reopened.layers[0].content else { panic!("native focus") };
                    assert_eq!(restored.smart_filters, mapping.filters);
                }
            }
        }
        let effect = StoredEffect::test("gegl:focus-blur", vec![float("blur-radius", 1500.)]);
        for context in [
            MappingContext { remaining_generation_bytes: 1, ..context() },
            MappingContext { remaining_metadata_bytes: 1, ..context() },
            MappingContext { native_filter_bounds: Rect::new(0, 0, 10000, 10000), ..context() },
        ] {
            assert_eq!(focus_blur(&effect, &context).unwrap_err().reason, ConversionReason::ResourceUnavailable);
        }
        let cancel = || true;
        assert!(focus_blur(&effect, &MappingContext { interrupt: Interrupt::cancel_only(&cancel), ..context() }).is_err());
    }
    #[test]
    fn broad_high_pass_uses_native_normalized_difference_and_retains_real_stack_limit() {
        use super::super::NativeStructure;
        use photocraft_color::{PixelFormat, SampleType};
        use photocraft_doc::{Document, Layer, LayerContent};
        use photocraft_geom::Size;
        use photocraft_raster::{Interrupt, Surface};
        let bounds = Rect::new(0, 0, 3, 1);
        let c = MappingContext { owner_bounds: bounds, native_filter_bounds: bounds, ..context() };
        let effect = StoredEffect::test("gegl:high-pass", vec![float("std-dev", 1500.)]);
        let mapping = high_pass(&effect, &c).unwrap();
        let Some(NativeStructure::Composite { branches }) = &mapping.structure else { panic!("native high-pass branches") };
        assert_eq!(branches.len(), 2);
        let expected_curve =
            |gain: f64, bias: f64| json!({"pencil":true,"points":(0..256).map(|v|(bias+gain*f64::from(v)/255.).clamp(0.,1.)*255.).collect::<Vec<_>>()});
        assert_eq!(branches[0].filters[0].params, expected_curve(0.25, 0.375));
        assert!(branches[1].clipped);
        assert_eq!(branches[1].blend, BlendMode::LinearLight);
        assert_eq!(branches[1].filters.len(), 5);
        assert_eq!(branches[1].filters[3].params, expected_curve(-0.125, 0.5625));
        assert_eq!(branches[1].filters[4].params, json!({"value":1.}));
        assert_eq!(mapping.filters[0].params, expected_curve(4., -1.5));
        let sigma = 1500. / 3f64.sqrt();
        assert!(
            branches[1].filters[..3].iter().all(|f| f.command == "filter.blur.gaussianBlur" && (f.params["radius"].as_f64().unwrap() - sigma).abs() < 1e-10)
        );
        let curve = |source: &Surface, params: &Value| {
            let format = source.format();
            let mut buffer = photocraft_compose::Buffer { rect: bounds, px: source.read_region(bounds).as_chunks::<4>().0.to_vec() };
            let adjustment = super::super::super::coverage::adjustment("image.adjustments.curves", params).unwrap();
            photocraft_compose::adjust::apply_in(
                &adjustment,
                &mut buffer,
                photocraft_compose::adjust::Transfer::for_document(ColorMode::Rgb, format.sample),
                Some(format.sample),
                ColorMode::Rgb,
            );
            let mut result = Surface::new(format);
            result.write_region(bounds, &buffer.px.into_iter().flatten().collect::<Vec<_>>());
            result
        };
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let format = PixelFormat::new(ColorMode::Rgb, depth, true);
            let mut input = Surface::new(format);
            input.write_region(bounds, &[0.2, 0.4, 0.7, 0.25, 0.7, 0.3, 0.2, 0.5, 0.4, 0.8, 0.3, 1.]);
            let mut blurred = input.clone();
            for _ in 0..3 {
                blurred = photocraft_algo::apply_in_with(
                    &blurred,
                    &photocraft_algo::FilterParams::GaussianBlur { radius: sigma as f32 },
                    bounds,
                    bounds,
                    None,
                    bounds,
                    &Interrupt::NONE,
                )
                .unwrap();
            }
            let base = curve(&input, &branches[0].filters[0].params);
            let encoded = curve(&blurred, &branches[1].filters[3].params);
            let opaque = photocraft_algo::apply_in_with(
                &encoded,
                &photocraft_algo::FilterParams::SetAlpha { value: 1. },
                bounds,
                bounds,
                None,
                bounds,
                &Interrupt::NONE,
            )
            .unwrap();
            let mut document = Document::new("Independent native difference", Size::new(3, 1), ColorMode::Rgb, depth);
            document.layers.push(Layer::new("Compressed source", LayerContent::Raster(base)));
            let mut difference = Layer::new("Opaque inverted blur", LayerContent::Raster(opaque));
            difference.blend = BlendMode::LinearLight;
            difference.clipped = true;
            document.layers.push(difference);
            let combined = photocraft_compose::composite_to_surface(&document, bounds, format, None);
            let actual = curve(&combined, &mapping.filters[0].params).read_region(bounds);
            let original = input.read_region(bounds);
            let blurred = blurred.read_region(bounds);
            for ((result, source), low) in actual.as_chunks::<4>().0.iter().zip(original.as_chunks::<4>().0).zip(blurred.as_chunks::<4>().0) {
                for channel in 0..3 {
                    let expected = (0.5 + source[channel] - low[channel]).clamp(0., 1.);
                    let tolerance = if depth == SampleType::U8 { 0.04 } else { 0.003 };
                    assert!((result[channel] - expected).abs() < tolerance, "{depth:?} {result:?} versus {expected}");
                }
                assert_eq!(result[3], source[3]);
            }
            let mut allowance = super::super::super::MAX_PIXEL_BYTES;
            let source = Layer::new("Original", LayerContent::Raster(input));
            let mut owner = super::super::super::native_effects::NativeOwner::new(&document, &source, bounds, &mut allowance, &Interrupt::NONE).unwrap();
            owner
                .push(
                    high_pass(&effect, &c).unwrap(),
                    super::super::super::native_effects::NativeEnvelope { opacity: 1., blend: BlendMode::Normal, visible: true, mask: None },
                    &mut allowance,
                    &Interrupt::NONE,
                )
                .unwrap();
            document.layers = vec![Layer::new("Editable native high-pass", owner.content())];
            let bytes = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
            assert_eq!(reopened.layers[0].content, document.layers[0].content);
        }
        let mut hdr = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::F32, true));
        hdr.write_region(bounds, &[-2., 3., 0.5, 0.75, -1., 2., 0.5, 0.5, -4., 4., 0.5, 1.]);
        let encoded = curve(&hdr, &branches[0].filters[0].params).read_region(bounds);
        let original = hdr.read_region(bounds);
        for (pixel, source) in encoded.as_chunks::<4>().0.iter().zip(original.as_chunks::<4>().0) {
            // Upstream's native 32-bit RGB Curves extend their endpoint slopes.
            for channel in 0..3 {
                let expected = 0.375 + 0.25 * source[channel];
                assert!((pixel[channel] - expected).abs() < 0.001, "{pixel:?} versus {expected}");
            }
            assert_eq!(pixel[3], source[3]);
        }
        assert!(mapping.notes.iter().any(|n| n.contains("32-bit RGB Curves extrapolate")));
        let boundary = high_pass(&StoredEffect::test("gegl:high-pass", vec![float("std-dev", 7874.)]), &c).unwrap();
        let Some(NativeStructure::Composite { branches: boundary }) = boundary.structure else { panic!("bounded broad native branch") };
        assert_eq!(boundary[1].filters.len(), 64);
        assert!(high_pass(&StoredEffect::test("gegl:high-pass", vec![float("std-dev", 1000.)]), &c).unwrap().structure.is_none());
        let too_large = high_pass(&StoredEffect::test("gegl:high-pass", vec![float("std-dev", 10000.)]), &c).unwrap_err();
        assert_eq!(too_large.reason, ConversionReason::ResourceUnavailable);
        assert!(too_large.message.contains("limit is64"));
        for context in [MappingContext { remaining_generation_bytes: 1, ..context() }, MappingContext { remaining_metadata_bytes: 1, ..context() }] {
            assert_eq!(high_pass(&effect, &context).unwrap_err().reason, ConversionReason::ResourceUnavailable);
        }
        let cancel = || true;
        assert!(high_pass(&effect, &MappingContext { interrupt: Interrupt::cancel_only(&cancel), ..context() }).is_err());
    }
    #[test]
    fn weak_gaussian_outward_zoom_and_large_sharpen_gain_have_bounded_native_counterparts() {
        let blur = mapped_effect("gegl:gaussian-blur", vec![float("std-dev-x", 0.01), float("std-dev-y", 0.01)]);
        assert_eq!(blur.filters[0].params["radius"], json!(0.1));
        assert!((blur.filters[0].opacity - 0.01).abs() < 1e-6);
        for factor in [-10., -0.5, 0.0001, 1.] {
            let zoom = mapped_effect("gegl:motion-blur-zoom", vec![float("factor", factor)]);
            let Some(super::super::NativeStructure::Composite { branches }) = zoom.structure else { panic!("native zoom copies") };
            assert_eq!(branches.len(), 32);
            assert!(branches.iter().all(|b| b.transform.m.iter().all(|n| n.is_finite())));
            assert_eq!(branches[0].opacity, 1.);
            let sum: f64 = (0..32)
                .map(|n| {
                    let mut w = f64::from(branches[n].opacity);
                    for next in &branches[n + 1..] {
                        w *= 1. - f64::from(next.opacity);
                    }
                    w
                })
                .sum();
            assert!((sum - 1.).abs() < 1e-6);
        }
        let sharp = mapped_effect("gegl:unsharp-mask", vec![float("scale", 300.)]);
        let gain = sharp.filters.iter().map(|f| 1. + f.params["amount"].as_f64().unwrap() / 100.).product::<f64>();
        assert!((gain - 301.).abs() < 1e-6);
        assert!(sharp.filters.len() <= 4);
        for angle in [0.01, 180., 360.] {
            let m = mapped_effect("gegl:motion-blur-circular", vec![float("angle", angle)]);
            let Some(super::super::NativeStructure::Composite { branches }) = m.structure else { panic!("native spin copies") };
            assert_eq!(branches.len(), 32);
            assert!(branches.iter().all(|b| b.transform.m.iter().all(|n| n.is_finite())));
            for b in branches {
                let [a, d, c, e, _, _] = b.transform.m;
                assert!((a * e - d * c - 1.).abs() < 1e-12);
            }
        }
    }
    #[test]
    fn broad_sharpen_prices_native_line_workspace_and_preserves_variance() {
        let c = MappingContext { native_filter_bounds: Rect::from_xywh(0, 0, 7, 5), ..context() };
        let m = broad_unsharp(1500., 3., 0., &c).unwrap();
        let Some(super::super::NativeStructure::Composite { branches }) = m.structure else { panic!("native detail branches") };
        assert_eq!(branches.len(), 2);
        assert!(branches[1].clipped);
        assert_eq!(branches[1].blend, BlendMode::Subtract);
        let variance: f64 =
            branches[1].filters.iter().filter(|f| f.command == "filter.blur.gaussianBlur").map(|f| f.params["radius"].as_f64().unwrap().powi(2)).sum();
        assert!((variance - 1500f64.powi(2)).abs() < 1e-6);
        let thresholded = broad_unsharp(1500., 3., 0.1, &c).unwrap();
        assert_eq!(thresholded.filters.len(), 4);
        assert!(thresholded.notes[0].contains("not variance-equivalent"));
        let tiny = MappingContext { remaining_generation_bytes: 1, ..c };
        assert!(broad_unsharp(1500., 3., 0., &tiny).is_err());
    }

    #[test]
    fn broad_threshold_sharpening_uses_native_gates_depth_alpha_and_persistence() {
        use photocraft_color::{PixelFormat, SampleType};
        use photocraft_doc::{Document, Layer, LayerContent, SmartObject, SmartSource};
        use photocraft_geom::{Affine, Size};
        use photocraft_raster::{Interrupt, Surface};
        let bounds = Rect::new(0, 0, 3, 1);
        let c = MappingContext { owner_bounds: bounds, native_filter_bounds: bounds, ..context() };
        let mapping = broad_unsharp(1500., 15., 0.1, &c).unwrap();
        assert_eq!(mapping.filters.len(), 4);
        for f in &mapping.filters {
            assert_eq!(f.command, "filter.sharpen.unsharpMask");
            assert!((f.params["radius"].as_f64().unwrap() - 750.).abs() < 1e-10);
            assert!((f.params["amount"].as_f64().unwrap() - 100.).abs() < 1e-10);
            assert_eq!(f.params["threshold"], json!(25.5));
            assert_eq!(f.opacity, 1.);
        }
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
                let mut source = Surface::new(PixelFormat::new(mode, depth, true));
                let colors = mode.color_channels();
                let data: Vec<f32> = [0.2, 0.5, 0.8].into_iter().flat_map(|value| std::iter::repeat_n(value, colors).chain(std::iter::once(0.5))).collect();
                source.write_region(bounds, &data);
                let run = |threshold| {
                    let mut output = source.clone();
                    // Independently chosen native counterpart: gain2 four times,
                    // support750, with the native gate in each pass.
                    for _ in 0..4 {
                        output = photocraft_algo::apply_in_with(
                            &output,
                            &photocraft_algo::FilterParams::UnsharpMask { radius: 750., amount: 100., threshold },
                            bounds,
                            bounds,
                            None,
                            bounds,
                            &Interrupt::NONE,
                        )
                        .unwrap();
                    }
                    output.read_region(bounds)
                };
                let original = source.read_region(bounds);
                let gated = run(255.);
                assert_eq!(gated, original, "all normalized differences below native threshold1");
                let sharpened = run(25.5);
                let mut mapped_output = source.clone();
                for f in &mapping.filters {
                    mapped_output = photocraft_algo::apply_in_with(
                        &mapped_output,
                        &photocraft_algo::FilterParams::UnsharpMask {
                            radius: f.params["radius"].as_f64().unwrap() as f32,
                            amount: f.params["amount"].as_f64().unwrap() as f32,
                            threshold: f.params["threshold"].as_f64().unwrap() as f32,
                        },
                        bounds,
                        bounds,
                        None,
                        bounds,
                        &Interrupt::NONE,
                    )
                    .unwrap();
                }
                for (actual, expected) in mapped_output.read_region(bounds).iter().zip(&sharpened) {
                    assert!((actual - expected).abs() < 1e-5, "independent native four-pass counterpart");
                }
                assert_ne!(sharpened, original);
                for (before, after) in original.chunks_exact(colors + 1).zip(sharpened.chunks_exact(colors + 1)) {
                    assert_eq!(after[colors], before[colors]);
                    assert!(after.iter().all(|v| v.is_finite()));
                }
                let mut input = Document::new("Native threshold input", Size::new(3, 1), mode, depth);
                input.layers.push(Layer::new("Pixels", LayerContent::Raster(source)));
                let bytes = photocraft_format::save_to_bytes(&input, &Default::default()).unwrap();
                let mut smart =
                    SmartObject::new(SmartSource::Embedded { file_name: "input.pcraft".into(), bytes: std::sync::Arc::new(bytes) }, Affine::IDENTITY, None);
                smart.smart_filters = mapping.filters.clone();
                input.layers = vec![Layer::new("Native threshold sharpening", LayerContent::Smart(smart))];
                let bytes = photocraft_format::save_to_bytes(&input, &Default::default()).unwrap();
                let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
                let LayerContent::Smart(smart) = &reopened.layers[0].content else { panic!("native editable filters") };
                assert_eq!(smart.smart_filters, mapping.filters);
            }
        }
        for (radius, strength) in [(1001., 0.00001), (1500., 300.)] {
            let m = broad_unsharp(radius, strength, 1., &c).unwrap();
            assert!(m.filters.len() <= 4);
            for f in m.filters {
                assert!((0.1..=1000.).contains(&f.params["radius"].as_f64().unwrap()));
                assert!((1. ..=500.).contains(&f.params["amount"].as_f64().unwrap()));
                assert!(f.opacity > 0. && f.opacity <= 1.);
            }
        }
        for (radius, strength) in [(0., 7.), (1500., 0.)] {
            assert!(mapped_effect("gegl:unsharp-mask", vec![float("std-dev", radius), float("scale", strength), float("threshold", 0.1)]).filters.is_empty());
        }
        assert!(broad_unsharp(1500., 15., 0.1, &MappingContext { remaining_metadata_bytes: 1, ..context() }).is_err());
        assert!(broad_unsharp(1500., 15., 0.1, &MappingContext { remaining_generation_bytes: 1, ..context() }).is_err());
        let cancel = || true;
        assert!(broad_unsharp(1500., 15., 0.1, &MappingContext { interrupt: Interrupt::cancel_only(&cancel), ..context() }).is_err());
    }

    #[test]
    fn invalid_or_unrepresented_domains_are_retained_without_silent_native_clamping() {
        for (operation, args, reason) in [
            ("gegl:gaussian-blur", vec![float("std-dev-x", f32::NAN)], ConversionReason::InvalidOptionalData),
            ("gegl:variable-blur", Vec::new(), ConversionReason::ResourceUnavailable),
            ("gegl:noise-rgb", vec![float("alpha", 1.1)], ConversionReason::InvalidOptionalData),
        ] {
            assert_eq!(map(&StoredEffect::test(operation, args), &context()).unwrap().unwrap_err().reason, reason, "{operation}");
        }
        let duplicate = StoredEffect::test("gegl:gaussian-blur", vec![float("std-dev-x", 1.0), float("std-dev-x", 2.0)]);
        assert!(map(&duplicate, &context()).unwrap().is_err());
    }
    #[test]
    fn edge_blur_broad_support_and_minimums_are_explicit_native_policies() {
        let mut c = context();
        c.native_filter_bounds = Rect::new(0, 0, 3, 2);
        for (op, args, count, radius, threshold) in [
            ("gegl:bilateral-filter", vec![float("blur-radius", 200.), float("edge-preservation", 8.)], 4, 100., 255. / 9.),
            (
                "gegl:domain-transform",
                vec![float("spatial-factor", 150.), int("n-iterations", 5)],
                15,
                150. / 3f64.sqrt(),
                2. + (1. - f64::from(0.8f32)) * 253.,
            ),
            ("gegl:gaussian-blur-selective", vec![float("blur-radius", 101.), float("max-delta", 0.001)], 2, 101. / 2f64.sqrt(), 2.),
            ("gegl:bilateral-filter", vec![float("blur-radius", 0.25)], 1, 1., 255. / 9.),
        ] {
            let mapping = edge_blur(&StoredEffect::test(op, args), &c).unwrap();
            assert_eq!(mapping.filters.len(), count, "{op}");
            for f in mapping.filters {
                assert_eq!(f.command, "filter.blur.surfaceBlur");
                assert!((f.params["radius"].as_f64().unwrap() - radius).abs() < 1e-9);
                assert!((f.params["threshold"].as_f64().unwrap() - threshold).abs() < 1e-4);
                assert_eq!(f.opacity, 1.);
            }
            assert!(mapping.notes.iter().any(|n| n.contains("native")));
        }
        for (op, args) in [
            ("gegl:bilateral-filter", vec![float("blur-radius", 0.)]),
            ("gegl:domain-transform", vec![float("spatial-factor", 0.)]),
            ("gegl:gaussian-blur-selective", vec![float("max-delta", 0.)]),
        ] {
            let mapping = edge_blur(&StoredEffect::test(op, args), &c).unwrap();
            assert!(mapping.filters.is_empty());
            assert!(mapping.notes[0].contains("not been independently established"));
        }
    }

    #[test]
    fn edge_blur_admits_bounded_native_stacks_before_allocating() {
        let mut c = context();
        c.native_filter_bounds = Rect::new(-1, -1, 1, 1);
        let effect = StoredEffect::test("gegl:bilateral-filter", vec![float("blur-radius", 787.)]);
        assert_eq!(edge_blur(&effect, &c).unwrap().filters.len(), 62);
        let excessive = StoredEffect::test("gegl:bilateral-filter", vec![float("blur-radius", 800.)]);
        assert_eq!(edge_blur(&excessive, &c).unwrap_err().reason, ConversionReason::ResourceUnavailable);
        let ordinary = StoredEffect::test("gegl:bilateral-filter", vec![]);
        c.remaining_generation_bytes = 1;
        assert_eq!(edge_blur(&ordinary, &c).unwrap_err().reason, ConversionReason::ResourceUnavailable);
        c.remaining_generation_bytes = crate::xcf::MAX_PIXEL_BYTES;
        c.remaining_metadata_bytes = 1;
        assert!(edge_blur(&ordinary, &c).unwrap_err().message.contains("metadata"));
        c.remaining_metadata_bytes = crate::xcf::MAX_METADATA_BYTES;
        c.native_filter_bounds = Rect::new(0, 0, 30_000, 30_000);
        assert_eq!(edge_blur(&ordinary, &c).unwrap_err().reason, ConversionReason::ResourceUnavailable);
        c.native_filter_bounds = Rect::new(0, 0, 1, 1);
        let cancelled = || true;
        c.interrupt = photocraft_raster::Interrupt::cancel_only(&cancelled);
        assert!(edge_blur(&ordinary, &c).unwrap_err().message.contains("cancelled"));
    }

    #[test]
    fn edge_blur_broad_native_counterpart_keeps_depth_coverage_and_persistence() {
        use photocraft_color::{PixelFormat, SampleType};
        use photocraft_doc::{Document, Layer, LayerContent, SmartObject, SmartSource};
        use photocraft_geom::{Affine, Size};
        use photocraft_raster::{Interrupt, Surface};
        let bounds = Rect::new(0, 0, 3, 2);
        let mut c = context();
        c.native_filter_bounds = bounds;
        let mapping = edge_blur(&StoredEffect::test("gegl:bilateral-filter", vec![float("blur-radius", 101.), float("edge-preservation", 8.)]), &c).unwrap();
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
                let mut source = Surface::new(PixelFormat::new(mode, depth, true));
                for y in 0..2 {
                    for x in 0..3 {
                        let alpha = (x + y + 1) as f32 / 5.;
                        let rgba = [0.2 + x as f32 * 0.05, 0.5, 0.9, alpha];
                        let gray = [rgba[0], alpha];
                        source.write_pixel(x, y, if mode == ColorMode::Rgb { &rgba } else { &gray });
                    }
                }
                let mut actual = source.clone();
                for f in &mapping.filters {
                    actual = photocraft_algo::apply_in_with(
                        &actual,
                        &photocraft_algo::FilterParams::SurfaceBlur {
                            radius: f.params["radius"].as_f64().unwrap() as f32,
                            threshold: f.params["threshold"].as_f64().unwrap() as f32,
                        },
                        bounds,
                        bounds,
                        None,
                        bounds,
                        &Interrupt::NONE,
                    )
                    .unwrap();
                }
                let mut expected = source.clone();
                for _ in 0..2 {
                    expected = photocraft_algo::apply_in_with(
                        &expected,
                        &photocraft_algo::FilterParams::SurfaceBlur { radius: 101. / 2f32.sqrt(), threshold: 255. / 9. },
                        bounds,
                        bounds,
                        None,
                        bounds,
                        &Interrupt::NONE,
                    )
                    .unwrap();
                }
                assert_eq!(actual.read_region(bounds), expected.read_region(bounds));
                assert_eq!(actual.format(), source.format());
                for (before, after) in
                    source.read_region(bounds).chunks_exact(mode.color_channels() + 1).zip(actual.read_region(bounds).chunks_exact(mode.color_channels() + 1))
                {
                    assert_eq!(before.last(), after.last());
                }
                let mut input = Document::new("Native broad edge smoothing", Size::new(3, 2), mode, depth);
                input.layers.push(Layer::new("Pixels", LayerContent::Raster(source)));
                let bytes = photocraft_format::save_to_bytes(&input, &Default::default()).unwrap();
                let mut smart =
                    SmartObject::new(SmartSource::Embedded { file_name: "pixels.pcraft".into(), bytes: std::sync::Arc::new(bytes) }, Affine::IDENTITY, None);
                smart.smart_filters = mapping.filters.clone();
                input.layers = vec![Layer::new("Native smoothing", LayerContent::Smart(smart))];
                let restored = photocraft_format::load_from_bytes(&photocraft_format::save_to_bytes(&input, &Default::default()).unwrap()).unwrap();
                let LayerContent::Smart(smart) = &restored.layers[0].content else { panic!("native editable object") };
                assert_eq!(smart.smart_filters, mapping.filters);
                assert_eq!(restored.depth, depth);
            }
        }
    }
}
