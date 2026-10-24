//! Frozen B06-09/10: native rendering, native geometry and explicit missing resources.
use super::residual_spatial::{
    boolean, bounds, choice, color, composite, generation_bytes, generation_check, geometry_bytes, integer, invalid, mapped, num, one, random, rect, replace,
    seed, shape,
};
use super::{MappingContext, MappingFailure, MappingResult, NativeBranch, NativeMapping, StoredEffect, native_filter};
use photocraft_color::BlendMode;
use photocraft_doc::effects::{
    Bevel, BevelStyle, BevelTechnique, Contour, Effect, FxCommon, FxPaint, Glow, GlowSource, GlowTechnique, Shadow, StrokeFx, StrokePosition,
};
use photocraft_doc::{Fill, Group, Layer, LayerContent, Path, Subpath, VectorMask};
use photocraft_geom::Affine;
use serde_json::json;
type Result<T> = std::result::Result<T, MappingFailure>;
/// Peak generated F32 tile storage plus explicit temporary bytes, checked before allocation.
fn surface_generation(c: &MappingContext, b: photocraft_geom::Rect, channels: usize, surfaces: usize, scratch: usize) -> Result<()> {
    let tile = i64::from(photocraft_geom::TILE_SIZE);
    let last_x = i64::from(b.x1) - 1;
    let last_y = i64::from(b.y1) - 1;
    let nx = usize::try_from(last_x.div_euclid(tile) - i64::from(b.x0).div_euclid(tile) + 1).map_err(|_| invalid("native tile count"))?;
    let ny = usize::try_from(last_y.div_euclid(tile) - i64::from(b.y0).div_euclid(tile) + 1).map_err(|_| invalid("native tile count"))?;
    let tile_pixels = usize::try_from(tile * tile).map_err(|_| invalid("native tile storage"))?;
    let bytes = nx
        .checked_mul(ny)
        .and_then(|n| n.checked_mul(surfaces))
        .and_then(|n| tile_pixels.checked_mul(channels).and_then(|p| p.checked_mul(4)).and_then(|p| p.checked_add(128)).and_then(|p| n.checked_mul(p)))
        .and_then(|n| n.checked_add(scratch))
        .and_then(|n| n.checked_add(8192))
        .ok_or_else(|| MappingFailure::resource("generated native tile storage overflows"))?;
    generation_bytes(c, bytes)
}
fn stress(e: &StoredEffect<'_>, c: &MappingContext) -> Result<NativeMapping> {
    let r = integer(e, "radius", 300, 2, 6000)?;
    let samples = integer(e, "samples", 5, 2, 500)?;
    let iterations = integer(e, "iterations", 5, 1, 1000)?;
    let shadows = boolean(e, "enhance-shadows", false)?;
    if r > 2500 {
        let sigma = f64::from(r) / 3.;
        let count = (sigma / 1000.).powi(2).ceil().max(1.) as usize;
        super::spatial::gaussian_workspace(c, sigma / (count as f64).sqrt(), 10)?;
        let bytes = (count * 2 + 8) * 2048 + 16384;
        generation_bytes(c, bytes)?;
        if bytes > c.remaining_metadata_bytes {
            return Err(MappingFailure::resource("native local base/detail stages exceed actual remaining metadata allowance"));
        }
        let gaussian: Vec<_> = (0..count).map(|_| native_filter("filter.blur.gaussianBlur", json!({"radius":sigma/(count as f64).sqrt()}))).collect();
        let half = || native_filter("image.adjustments.channelMixer", json!({"red":[50.,0.,0.,0.],"green":[0.,50.,0.,0.],"blue":[0.,0.,50.,0.]}));
        let mut tone = gaussian.clone();
        tone.push(native_filter("image.adjustments.shadowsHighlights",json!({"shadowAmount":if shadows{50}else{35},"shadowTone":50,"shadowRadius":0,"highlightAmount":35,"highlightTone":50,"highlightRadius":0,"color":0,"midtone":0,"blackClip":0,"whiteClip":0})));
        tone.push(half());
        let mut base = gaussian;
        base.push(half());
        let black = super::resources::native_rgba([0., 0., 0., 1.], c).map_err(invalid)?;
        let mut result = composite(
            vec![
                NativeBranch { layer_effects: vec![Effect::ColorOverlay { common: FxCommon::new(BlendMode::Normal, 1.), color: black }], ..Default::default() },
                NativeBranch { filters: tone, blend: BlendMode::LinearDodge, clipped: true, ..Default::default() },
                NativeBranch { filters: vec![half()], blend: BlendMode::LinearDodge, clipped: true, ..Default::default() },
                NativeBranch { filters: base, blend: BlendMode::Subtract, clipped: true, ..Default::default() },
            ],
            format!(
                "Large local contrast support becomes a native Gaussian base/detail document: source radius{r} uses an explicit three-sigma support policy, native Shadows/Highlights tones the base, and clipped Add/Subtract recombines original detail with half-strength Channel Mixer headroom. Source samples{samples}/iterations{iterations} remain disclosed algorithm differences. Native clipping, working-color/HDR, alpha-weighted Gaussian edges and ordinary native coverage apply; no dialog radius cap or source evaluator"
            ),
        );
        result.filters.push(native_filter("image.adjustments.channelMixer", json!({"red":[200.,0.,0.,0.],"green":[0.,200.,0.,0.],"blue":[0.,0.,200.,0.]})));
        return Ok(result);
    }
    Ok(one(
        "image.adjustments.shadowsHighlights",
        json!({"shadowAmount":if shadows{50}else{35},"shadowTone":50,"shadowRadius":r,"highlightAmount":35,"highlightTone":50,"highlightRadius":r,"color":0,"midtone":0,"blackClip":0,"whiteClip":0}),
        format!(
            "Source local radius becomes native Shadows/Highlights support. Explicit native tone-balancing policy uses35% both ways or50% shadow lifting when requested. Source stochastic samples{samples}/iterations{iterations} remain disclosed and never become native strength; native tone metric/ICC and alpha apply"
        ),
    ))
}
fn water_cells(size: i32, smooth: f64, regularity: i32, fill: i32, c: &MappingContext) -> Result<NativeMapping> {
    let b = bounds(c)?;
    let nx = (b.width() as usize).div_ceil(size as usize);
    let ny = (b.height() as usize).div_ceil(size as usize);
    let count = nx.checked_mul(ny).ok_or_else(|| invalid("native cell count overflows"))?;
    if count > 64 {
        return Err(MappingFailure::resource("native editable water cells exceed64 branches"));
    }
    geometry_bytes(c, count, count * 4)?;
    let metadata = count * 8192;
    generation_bytes(c, metadata)?;
    if metadata > c.remaining_metadata_bytes {
        return Err(MappingFailure::resource("native water cells exceed actual metadata allowance"));
    }
    // Shared cut coordinates partition the native frame without gaps or overlap.
    // Only interior boundaries are jittered; increasing regularity reduces jitter.
    let mut state = 0u32;
    let mut cuts = |origin: i32, end: i32, n: usize| -> Result<Vec<i32>> {
        let mut out = Vec::new();
        out.try_reserve_exact(n + 1).map_err(|_| MappingFailure::resource("native cell cuts unavailable"))?;
        out.push(origin);
        for i in 1..n {
            generation_check(c)?;
            let jitter = (f64::from(random(&mut state)) / f64::from(u32::MAX) - 0.5) * f64::from(size) * 0.5 * (1. - f64::from(regularity) / 50.);
            let position = i64::from(origin) + (i as i64) * i64::from(size) + jitter.round() as i64;
            out.push(position.clamp(i64::from(origin) + 1, i64::from(end) - 1) as i32);
        }
        out.push(end);
        Ok(out)
    };
    let xs = cuts(b.x0, b.x1, nx)?;
    let ys = cuts(b.y0, b.y1, ny)?;
    let mut branches = Vec::new();
    branches.try_reserve_exact(count).map_err(|_| MappingFailure::resource("native water cell branches unavailable"))?;
    let mut frames = 0usize;
    for y in ys.windows(2) {
        for x in xs.windows(2) {
            generation_check(c)?;
            let cell = photocraft_geom::Rect::new(x[0], y[0], x[1], y[1]);
            let (frame, transform, filters) = if fill == 1 {
                let sx = x[0] + (random(&mut state) % cell.width()) as i32;
                let sy = y[0] + (random(&mut state) % cell.height()) as i32;
                let transform = Affine::translate(f64::from(x[0]), f64::from(y[0]))
                    .mul(&Affine { m: [f64::from(cell.width()), 0., 0., f64::from(cell.height()), 0., 0.] })
                    .mul(&Affine::translate(-f64::from(sx), -f64::from(sy)));
                (photocraft_geom::Rect::from_xywh(sx, sy, 1, 1), transform, Vec::new())
            } else {
                let mut filters = Vec::new();
                if smooth > 0. {
                    super::spatial::gaussian_workspace(c, smooth.max(0.1), 3)?;
                    let mut blur = native_filter("filter.blur.gaussianBlur", json!({"radius":smooth.max(0.1)}));
                    if smooth < 0.1 {
                        blur.opacity = (smooth / 0.1).powi(2) as f32;
                    }
                    filters.push(blur);
                }
                filters.push(native_filter("filter.blur.average", json!({})));
                (cell, Affine::IDENTITY, filters)
            };
            let tile = photocraft_geom::TILE_SIZE as usize;
            let frame_bytes = (frame.width() as usize)
                .div_ceil(tile)
                .checked_mul((frame.height() as usize).div_ceil(tile))
                .and_then(|n| n.checked_mul(tile * tile * (c.mode.color_channels() + 1) * 4 + 128))
                .and_then(|n| n.checked_mul(3))
                .and_then(|n| n.checked_add(65536))
                .ok_or_else(|| MappingFailure::resource("native cell cache storage overflows"))?;
            frames = frames.checked_add(frame_bytes).ok_or_else(|| MappingFailure::resource("native water frame storage overflows"))?;
            branches.push(NativeBranch {
                filter_frame: Some(frame),
                transform,
                filters,
                vector_mask: Some(VectorMask::new(Path::new(vec![rect(f64::from(x[0]), f64::from(y[0]), f64::from(cell.width()), f64::from(cell.height()))]))),
                ..Default::default()
            });
        }
    }
    generation_bytes(c, frames.checked_add(metadata).ok_or_else(|| invalid("native cell storage overflows"))?)?;
    Ok(composite(
        branches,
        format!(
            "Large source water size{size} becomes at most64 native rectangular cells with shared jittered boundaries; regularization{regularity} controls native boundary regularity. Fill{fill} uses either framed native Gaussian(smoothness{smooth}) then native alpha-weighted Average, or deterministic random1x1 samples expanded by ordinary editable affine placements. Native rectangular cells replace watershed regions and native randomness replaces the source sequence. For random fill, source smoothing remains a disclosed sampling difference because the point frame has no neighboring pixels; no source prefilter evaluator persists. Regional native mean alpha or selected-point alpha, vector-mask antialiasing and native transparent-source behavior apply"
        ),
    ))
}
fn water(e: &StoredEffect<'_>, c: &MappingContext) -> Result<NativeMapping> {
    let size = integer(e, "size", 32, 8, i32::MAX)?;
    let smooth = num(e, "smoothness", 1., 0., 1000.)?;
    let regularity = integer(e, "regularization", 0, 0, 50)?;
    let fill = choice(e, "fill", 0, &[0, 1])?;
    if size > 300 {
        return water_cells(size, smooth, regularity, fill, c);
    }
    let mut filters = Vec::new();
    if smooth > 0. {
        let mut blur = native_filter("filter.blur.gaussianBlur", json!({"radius":smooth.max(0.1)}));
        if smooth < 0.1 {
            blur.opacity = (smooth / 0.1).powi(2) as f32;
        }
        filters.push(blur);
    }
    filters.push(native_filter("filter.pixelate.crystallize", json!({"cellSize":size,"seed":0})));
    Ok(mapped(
        filters,
        format!(
            "Source segmentation scale and smoothing become native Gaussian then Crystallize. Below native Gaussian0.1, (sigma/.1)^2 fractional filter strength is an explicit weak-kernel approximation. Native deterministic polygon cells replace watershed regularization{regularity} and fill-mode{fill}; native fixed seed0 is an explicit reproducibility policy because source has no seed property. Native center sampling approximates regional average/random fill, retaining source controls as disclosed data"
        ),
    ))
}
fn transparent_paper(c: &MappingContext, (width, height): (i32, i32), rate: f64, wrap: bool, partial: i32, center: bool, seed: u32) -> Result<NativeMapping> {
    let b = bounds(c)?;
    let nx = (f64::from(b.width()) / f64::from(width)).ceil() as usize;
    let ny = (f64::from(b.height()) / f64::from(height)).ceil() as usize;
    let cells = nx.checked_mul(ny).ok_or_else(|| invalid("paper tile count overflow"))?;
    if cells > 64 {
        return Err("native editable paper copies exceed64 source tiles; a denser native map representation is required".into());
    }
    let start_x = f64::from(b.x0) + if center { (f64::from(b.width()) - nx as f64 * f64::from(width)) * 0.5 } else { 0. };
    let start_y = f64::from(b.y0) + if center { (f64::from(b.height()) - ny as f64 * f64::from(height)) * 0.5 } else { 0. };
    // Two bounded passes determine wrapped copy count before reserving any branch data.
    let generate = |emit: &mut dyn FnMut(f64, f64, f64, f64, f64, f64) -> Result<()>| -> Result<()> {
        let mut state = seed;
        for y in 0..ny {
            for x in 0..nx {
                generation_check(c)?;
                let x0 = (start_x + x as f64 * f64::from(width)).max(f64::from(b.x0));
                let y0 = (start_y + y as f64 * f64::from(height)).max(f64::from(b.y0));
                let x1 = (start_x + (x + 1) as f64 * f64::from(width)).min(f64::from(b.x1));
                let y1 = (start_y + (y + 1) as f64 * f64::from(height)).min(f64::from(b.y1));
                let fractional = x1 - x0 < f64::from(width) || y1 - y0 < f64::from(height);
                let rx = f64::from(random(&mut state)) / f64::from(u32::MAX) * 2. - 1.;
                let ry = f64::from(random(&mut state)) / f64::from(u32::MAX) * 2. - 1.;
                if fractional && partial == 0 {
                    continue;
                }
                let dx = if fractional && partial == 1 { 0. } else { (rx * f64::from(width) * rate / 100.).round() };
                let dy = if fractional && partial == 1 { 0. } else { (ry * f64::from(height) * rate / 100.).round() };
                let dx = if wrap { (dx + f64::from(b.width()) * 0.5).rem_euclid(f64::from(b.width())) - f64::from(b.width()) * 0.5 } else { dx };
                let dy = if wrap { (dy + f64::from(b.height()) * 0.5).rem_euclid(f64::from(b.height())) - f64::from(b.height()) * 0.5 } else { dy };
                for wy in -1..=1 {
                    for wx in -1..=1 {
                        if !wrap && (wx != 0 || wy != 0) {
                            continue;
                        }
                        let tx = dx + f64::from(wx) * f64::from(b.width());
                        let ty = dy + f64::from(wy) * f64::from(b.height());
                        let left = (x0 + tx).max(f64::from(b.x0));
                        let top = (y0 + ty).max(f64::from(b.y0));
                        let right = (x1 + tx).min(f64::from(b.x1));
                        let bottom = (y1 + ty).min(f64::from(b.y1));
                        if right > left && bottom > top {
                            emit(tx, ty, left, top, right - left, bottom - top)?
                        }
                    }
                }
            }
        }
        Ok(())
    };
    let mut count = 0usize;
    generate(&mut |_, _, _, _, _, _| {
        count = count.checked_add(1).ok_or_else(|| invalid("paper branch count overflow"))?;
        if count > 64 {
            return Err("wrapped native paper copies exceed64 branches".into());
        }
        Ok(())
    })?;
    let bytes = count.max(1).checked_mul(4096).ok_or_else(|| invalid("paper metadata overflow"))?;
    if bytes > c.remaining_metadata_bytes {
        return Err(MappingFailure::resource("native paper copies exceed actual remaining metadata allowance"));
    }
    generation_bytes(c, bytes)?;
    let mut branches = Vec::new();
    branches.try_reserve_exact(count.max(1)).map_err(|_| MappingFailure::resource("native paper copy allocation unavailable"))?;
    generate(&mut |tx, ty, x, y, w, h| {
        branches.push(NativeBranch {
            transform: Affine::translate(tx, ty),
            vector_mask: Some(VectorMask::new(Path::new(vec![rect(x, y, w, h)]))),
            ..Default::default()
        });
        Ok(())
    })?;
    if branches.is_empty() {
        branches.push(NativeBranch { fill: Some(Fill::Solid(photocraft_color::Color::rgba(0., 0., 0., 0.))), ..Default::default() })
    }
    Ok(composite(
        branches,
        "Transparent paper gaps are ordinary native editable source-copy Smart layers, translated with destination-local rectangular VectorMasks. Source rectangular dimensions, centered partition, fractional background/unchanged/moved policy and wrap coverage are retained. Native seeded uniform integer placements and row-major overlap order replace source random sequence/order; native mask antialiasing and Normal alpha composition govern overlaps. No opaque background or original-image base is inserted",
    ))
}
fn tile_paper(e: &StoredEffect<'_>, c: &MappingContext) -> Result<NativeMapping> {
    let b = bounds(c)?;
    let w = integer(e, "tile-width", 155, 1, i32::MAX)?;
    let h = integer(e, "tile-height", 56, 1, i32::MAX)?;
    let rate = num(e, "move-rate", 25., 1., 100.)?;
    let wrap = boolean(e, "wrap-around", false)?;
    let partial = choice(e, "fractional-type", 2, &[0, 1, 2])?;
    let center = boolean(e, "centering", true)?;
    let bg = choice(e, "background-type", 1, &[0, 1, 2, 3])?;
    let seed = seed(e)?;
    let paint = color(e, "bg-color", [0., 0., 0., 1.], c)?;
    if bg == 0 {
        return transparent_paper(c, (w, h), rate, wrap, partial, center, seed);
    }
    let nx = (f64::from(b.width()) / f64::from(w)).ceil();
    let ny = (f64::from(b.height()) / f64::from(h)).ceil();
    let count = (nx * ny).sqrt();
    if !(1.0..=99.0).contains(&count) || rate > 99. {
        return Err("source paper tile count/movement exceeds native Tiles count1..99 or maxOffset1..99 dialog domain".into());
    }
    Ok(one(
        "filter.stylize.tiles",
        json!({"count":count.round(),"maxOffset":rate,"fill":match bg{1=>"inverse",2=>"unaltered",_=>"background"},"background":[paint.c[0],paint.c[1],paint.c[2],paint.alpha],"seed":seed}),
        format!(
            "Source rectangular tile count becomes native square Tiles count by geometric mean of x/y counts; movement remains percent. Native square partition, randomness and boundary rules replace source {w}x{h} rectangles, wrap{wrap}, fractional-mode{partial}, centering{center}; selected native fill/color preserve background intent"
        ),
    ))
}
fn propagation(e: &StoredEffect<'_>, c: &MappingContext) -> Result<NativeMapping> {
    let mode = match e.argument("mode")? {
        None => 0,
        Some(super::ArgumentValue::Enum(v)) if (0..=6).contains(v) => *v,
        Some(super::ArgumentValue::String(nick)) => ["white", "black", "middle", "color-peak", "color", "opaque", "transparent"]
            .iter()
            .position(|v| v == nick)
            .map(|v| v as i32)
            .ok_or_else(|| invalid("unknown source propagation mode"))?,
        _ => return Err(invalid("unknown or malformed source propagation mode")),
    };
    let low = num(e, "lower-threshold", 0., 0., 1.)?;
    let high = num(e, "upper-threshold", 1., 0., 1.)?;
    let rate = num(e, "rate", 1., 0., 1.)?;
    let directions = [boolean(e, "top", true)?, boolean(e, "left", true)?, boolean(e, "right", true)?, boolean(e, "bottom", true)?];
    let value = boolean(e, "value", true)?;
    let alpha = boolean(e, "alpha", true)?;
    if rate == 0. || directions.iter().all(|v| !*v) || (!value && !alpha) {
        return Ok(mapped(Vec::new(), "Zero propagation amount/directions/channels preserves native pixels"));
    }
    if low > high {
        return Ok(mapped(Vec::new(), "Crossed source relative-threshold interval selects no neighbor differences; native pixels remain unchanged"));
    }
    let limitation = |message: &str| MappingFailure { reason: photocraft_doc::import_conversion::ConversionReason::NativeLimitation, message: message.into() };
    if low != 0. || high != 1. {
        return Err(limitation(
            "Native propagation construction has no conditional neighbor-minus-center value gate. Native Blend If tests absolute current/underlying channel ranges, while ordinary Min/Max/Percentile/AlphaCurve do not select neighbors by relative differences. Source lower/upper thresholds remain retained unapplied; an absolute Blend If surrogate is not claimed equivalent",
        ));
    }
    let all = directions.iter().all(|v| *v);
    let mut channels = vec![value; c.mode.color_channels()];
    channels.push(alpha);
    let rank = |maximum: bool| {
        let q = if maximum { 100. } else { 0. };
        native_filter(
            "filter.other.percentile",
            json!({"radius":1,"colorPercentile":if c.mode==photocraft_color::ColorMode::Cmyk{100.-q}else{q},"alphaPercentile":q,"shape":"square","channels":channels}),
        )
    };
    let offsets: Vec<(i32, i32)> =
        [(0, -1), (-1, 0), (1, 0), (0, 1)].into_iter().zip(directions).filter_map(|(offset, enabled)| enabled.then_some(offset)).collect();
    if matches!(mode, 0 | 1 | 5 | 6) && all {
        let mut filter = rank(matches!(mode, 0 | 5));
        filter.opacity = rate as f32;
        return Ok(mapped(
            vec![filter],
            "Source bright/dark or opaque/transparent expansion becomes ordinary native independent color/coverage extrema over a1px square. Native selected-channel mask preserves unselected stored colors/HDR/alpha; CMYK follows native brightness polarity. Native per-channel extrema replace source value/opacity-selected neighbor color vectors; rate is native filter opacity, with native footprint/coverage/compositing rules",
        ));
    }
    if mode == 4 {
        // ColorToAlpha loses the distinction between absent pixels and opaque exact-key
        // pixels. Inverting that alpha before growth would invent hidden-pixel key hits,
        // even if final clipping prevented drawing outside original coverage.
        color(e, "color", [0., 0., 1., 1.], c)?;
        return Err(limitation(
            "Native key-growth construction cannot intersect derived key coverage with original alpha before neighborhood growth and recombine the original in this bounded native stage. Color to Alpha maps both transparent input and opaque exact-key input to alpha0; inverse alpha therefore invents transparent/hidden-pixel key hits, which can grow inside final clipping coverage. Source key/color configuration and original pixels remain retained unapplied; no inverted-alpha approximation is emitted",
        ));
    }
    if !value && alpha && mode != 2 {
        return Err(limitation(
            "Native asymmetric or painted propagation has no existing alpha-only neighbor stencil/recombination that preserves original stored colors. Symmetric opacity extrema are supported by native Percentile channel selection; translated layer blends and colored edge overlays would change colors or accumulate coverage instead of the requested alpha-only selection. Original pixels/configuration remain retained unapplied",
        ));
    }
    if mode == 2 {
        if all {
            return Ok(composite(
                vec![
                    NativeBranch::default(),
                    NativeBranch { filters: vec![rank(false)], opacity: (rate / (2. - rate)) as f32, clipped: !alpha, ..Default::default() },
                    NativeBranch { filters: vec![rank(true)], opacity: (rate / 2.) as f32, clipped: !alpha, ..Default::default() },
                ],
                "Middle-to-peaks intent becomes ordinary native minimum/maximum branches. For opaque samples, native weights original(1-rate)+minimum*rate/2+maximum*rate/2 give the native neighborhood midrange; partial coverage uses native layer/clipping compositing. Source peak-only neighbor choice is replaced explicitly by native per-channel midrange; channel flags remain native rank masks",
            ));
        }
        let mut kernel = vec![0.; 25];
        if let Some(v) = kernel.get_mut(12) {
            *v = 1.;
        }
        for &(x, y) in &offsets {
            let index = ((2 - y) * 5 + 2 - x) as usize;
            if let Some(v) = kernel.get_mut(index) {
                *v = 1.;
            }
        }
        let mut filter = native_filter(
            "filter.other.convolution",
            json!({"kernel":kernel,"scale":1+offsets.len(),"offset":0,"channels":channels,"alphaWeighted":false,"edge":"repeat"}),
        );
        filter.opacity = rate as f32;
        return Ok(mapped(
            vec![filter],
            "Directional middle propagation becomes ordinary native selected-channel Convolution averaging the current sample with enabled1px cardinal source neighbors. Direction selects opposite sampling taps so native paint propagates toward the requested direction; native linear mean replaces conditional border-pair averaging. Native boundary/alpha/depth rules apply, and rate is native filter opacity",
        ));
    }
    if mode == 3 {
        let mut paint = color(e, "color", [0., 0., 1., 1.], c)?;
        let paint_alpha = paint.alpha;
        paint.alpha = 1.;
        let filters = vec![
            native_filter("filter.stylize.findEdges", json!({})),
            native_filter("filter.other.colorToAlpha", json!({"color":[1.,1.,1.,1.],"transparencyThreshold":0,"opacityThreshold":1})),
        ];
        let placements = if all { vec![(0, 0)] } else { offsets };
        let mut branches = vec![NativeBranch::default()];
        for (x, y) in placements {
            branches.push(NativeBranch {
                filters: filters.clone(),
                transform: Affine::translate(f64::from(x), f64::from(y)),
                opacity: rate as f32 * paint_alpha,
                clipped: !alpha,
                layer_effects: vec![Effect::ColorOverlay { common: FxCommon::new(BlendMode::Normal, 1.), color: paint }],
                ..Default::default()
            });
        }
        return Ok(composite(
            branches,
            "Color-to-peaks intent becomes native Find Edges, native white Color to Alpha and an assigned-profile native Color Overlay. It never inverts alpha: originally transparent pixels remain transparent before native placement/clipping. Enabled directions place native edge paint1px toward checked cardinal directions; all directions use the native symmetric edge map. Native edge detection/key unmixing and layer compositing replace source peak selection; value-only clips to original coverage, alpha-enabled paint uses native coverage, and rate/paint alpha become native layer opacity",
        ));
    }
    let mut branches = vec![NativeBranch::default()];
    for (x, y) in offsets {
        branches.push(NativeBranch {
            transform: Affine::translate(f64::from(x), f64::from(y)),
            blend: if matches!(mode, 0 | 5) { BlendMode::Lighten } else { BlendMode::Darken },
            opacity: rate as f32,
            clipped: !alpha,
            ..Default::default()
        });
    }
    Ok(composite(
        branches,
        "Directional bright/dark or opacity propagation becomes ordinary native1px cardinal source copies in Lighten/Darken blend. Checked directions are native placements, rate is per-copy native opacity, and value-only uses native clipping to preserve base coverage. Native per-channel blend selection and alpha-over replace source neighbor vector/diagonal selection; no asymmetric source morphology evaluator is embedded",
    ))
}
fn texturize(e: &StoredEffect<'_>) -> Result<NativeMapping> {
    let direction = choice(e, "direction", 0, &[0, 1, 2, 3])?;
    let depth = integer(e, "depth", 4, 1, 50)?;
    let light = match direction {
        0 => "topRight",
        1 => "topLeft",
        2 => "bottomLeft",
        _ => "bottomRight",
    };
    Ok(one(
        "filter.gallery.texturizer",
        json!({"texture":"canvas","scaling":100,"relief":depth,"light":light,"invert":false}),
        "Canvas-weave intent uses native procedural Gallery Texturizer canvas, with source depth as native relief and matching diagonal light direction. Native 100% texture scale is an explicit policy because source has no scale control. The unavailable source built-in texture table is not copied or claimed equivalent: native weave geometry, relief metric, shading and color rules govern the editable result",
    ))
}

fn unpremultiply() -> Result<NativeMapping> {
    Ok(one(
        "layer.matting.removeBlackMatte",
        json!({}),
        "Alpha-unmixing intent becomes the existing native Photoshop Remove Black Matte action, kept editable as an ordinary Smart Filter. The native operation divides color by partial alpha, retains coverage and opaque/transparent samples, and applies native color/depth/clipping rules. Source linear-light unpremultiplication differs from native document-channel/RGBA conversion policy; no second XCF tile decoding or source representation interpreter is introduced",
    ))
}

struct StereoCamera {
    pan: f64,
    tilt: f64,
    spin: f64,
    zoom: f64,
    frame: (f64, f64),
    inverse: bool,
    sampler: i32,
}
fn stereographic_map(c: &MappingContext, camera: StereoCamera) -> Result<NativeMapping> {
    let b = bounds(c)?;
    let w = f64::from(b.width());
    let h = f64::from(b.height());
    let (fw, fh) = camera.frame;
    if fw == 0. || fh == 0. {
        return Ok(composite(
            vec![NativeBranch { fill: Some(Fill::Solid(photocraft_color::Color::rgba(0., 0., 0., 0.))), ..Default::default() }],
            "Zero native virtual projection-frame extent produces transparent output; no native canvas mutation",
        ));
    }
    let (st, ct) = camera.tilt.rem_euclid(360.).to_radians().sin_cos();
    let (ss, cs) = camera.spin.rem_euclid(360.).to_radians().sin_cos();
    let pan = camera.pan.rem_euclid(360.).to_radians();
    let factor = 100. / camera.zoom;
    let mut mapped = super::residual_transform::coordinate_map(
        c,
        |x, y| {
            let x = f64::from(x) + 0.5;
            let y = f64::from(y) + 0.5;
            if camera.inverse {
                let longitude = x / fw * std::f64::consts::TAU - pan;
                let latitude = (y / fh - 0.5) * std::f64::consts::PI;
                let vx = longitude.sin() * latitude.cos();
                let vy = latitude.sin();
                let vz = longitude.cos() * latitude.cos();
                let ry = vy * ct - vz * st;
                let rz = vy * st + vz * ct;
                // The antipodal singularity is a documented finite native edge-clamp policy.
                let divisor = (1. + rz).max(1e-6);
                let px = vx / divisor;
                let py = ry / divisor;
                let nx = (px * cs + py * ss) / factor;
                let ny = (-px * ss + py * cs) / factor;
                ((w * 0.5 + nx * w * 0.5).clamp(0.5, w - 0.5), (h * 0.5 + ny * w * 0.5).clamp(0.5, h - 0.5))
            } else {
                let nx = (x - fw * 0.5) / (fw * 0.5) * factor;
                let ny = (y - fh * 0.5) / (fw * 0.5) * factor;
                let px = nx * cs - ny * ss;
                let py = nx * ss + ny * cs;
                let r2 = px * px + py * py;
                let vx = 2. * px / (1. + r2);
                let vy = 2. * py / (1. + r2);
                let vz = (1. - r2) / (1. + r2);
                let vertical = vy * ct + vz * st;
                let forward = vz * ct - vy * st;
                let longitude = (vx.atan2(forward) + pan).rem_euclid(std::f64::consts::TAU);
                let latitude = vertical.atan2(vx.hypot(forward));
                ((longitude / std::f64::consts::TAU * w + 0.5).rem_euclid(w), ((0.5 + latitude / std::f64::consts::PI) * h + 0.5).clamp(0.5, h - 0.5))
            }
        },
        &format!(
            "Standard analytic stereographic camera geometry is authored once into ordinary editable native Displace data, preserving pan/tilt/spin direction, reciprocal percentage zoom and inverse-camera intent. Requested output extent becomes native virtual frame{fw}x{fh} plus destination VectorMask inside the existing XCF canvas; no native canvas resize is implied. Native pixel-centred rays use a hemisphere at normalized plane radius1, antipodal singularities clamp to finite source edges, and source sampler{} is replaced by native bilinear interpolation/U8 UV quantization/wrapped boundaries. Native aperture, frame cropping and pole behavior explicitly approximate source projection; no source evaluator remains",
            camera.sampler
        ),
    )?;
    let filters = std::mem::take(&mut mapped.filters);
    let mask = VectorMask::new(Path::new(vec![rect(f64::from(b.x0), f64::from(b.y0), fw, fh)]));
    mapped.structure = Some(super::NativeStructure::Composite { branches: vec![NativeBranch { filters, vector_mask: Some(mask), ..Default::default() }] });
    Ok(mapped)
}
fn stereographic(e: &StoredEffect<'_>, c: &MappingContext) -> Result<NativeMapping> {
    let pan = num(e, "pan", 0., -360., 360.)?;
    let tilt = num(e, "tilt", 90., -180., 180.)?;
    let spin = num(e, "spin", 0., -360., 360.)?;
    let zoom = num(e, "zoom", 100., 0.01, 1000.)?;
    let width = integer(e, "width", -1, -1, 10000)?;
    let height = integer(e, "height", -1, -1, 10000)?;
    let inverse = boolean(e, "inverse", false)?;
    let sampler = choice(e, "sampler-type", 0, &[0, 1, 2, 3, 4])?;
    let b = bounds(c)?;
    stereographic_map(
        c,
        StereoCamera {
            pan,
            tilt,
            spin,
            zoom,
            inverse,
            sampler,
            frame: (if width == -1 { f64::from(b.width()) } else { f64::from(width) }, if height == -1 { f64::from(b.height()) } else { f64::from(height) }),
        },
    )
}

fn glass(e: &StoredEffect<'_>, context: &MappingContext) -> Result<NativeMapping> {
    let width = f64::from(integer(e, "tile-width", 25, 5, 500)?);
    let height = f64::from(integer(e, "tile-height", 25, 5, 500)?);
    let mut mapping = super::residual_transform::coordinate_map(
        context,
        |x, y| {
            let u = f64::from(x) + 0.5;
            let v = f64::from(y) + 0.5;
            let center_x = (u / width).floor() * width + width / 2.;
            let center_y = (v / height).floor() * height + height / 2.;
            (u + 0.5 * (center_x - u), v + 0.5 * (center_y - v))
        },
        "Source rectangular glass-tile intent becomes an independently authored standard rectangular-cell lens coordinate resource feeding ordinary native Displace. Each source tile-width/height independently sets its native cell period across the full declared5..500 pixel domain; local source coordinates move half way toward the cell center (fixed native half-strength refraction). Native owner-origin alignment, U8 coordinate quantization, bilinear reconstruction, cell-edge discontinuities, alpha and native depth/color rules replace source refraction; no source glass processor remains. Displacement is at most125px per axis, inside native range; actual native map/hex/cache metadata and cancellation bound work rather than the Glass dialog period9..36",
    )?;
    for filter in &mut mapping.filters {
        filter.params["undefinedAreas"] = json!("repeat");
    }
    mapping.notes.push(
        "Native repeated-edge sampling is the explicit boundary policy for tile refraction; source expansion/boundary response is not asserted identical"
            .into(),
    );
    Ok(mapping)
}

fn vignette(e: &StoredEffect<'_>, c: &MappingContext) -> Result<NativeMapping> {
    let b = bounds(c)?;
    let kind = choice(e, "shape", 0, &[0, 1, 2, 3, 4])?;
    let paint = color(e, "color", [0., 0., 0., 1.], c)?;
    let radius = num(e, "radius", 1.2, 0., f64::MAX)?;
    let soft = num(e, "softness", 0.8, 0., 1.)?;
    let gamma = num(e, "gamma", 2., 0., f64::MAX)?;
    let proportion = num(e, "proportion", 1., 0., 1.)?;
    let squeeze = num(e, "squeeze", 0., -1., 1.)?;
    let x = num(e, "x", 0.5, -f64::MAX, f64::MAX)?;
    let y = num(e, "y", 0.5, -f64::MAX, f64::MAX)?;
    let angle = num(e, "rotation", 0., 0., 360.)?.to_radians();
    let cx = f64::from(b.x0) + x * f64::from(b.width());
    let cy = f64::from(b.y0) + y * f64::from(b.height());
    let min = f64::from(b.width().min(b.height()));
    let rx = radius * (proportion * f64::from(b.width()) + (1. - proportion) * min) / 2. * (1. + squeeze);
    let ry = radius * (proportion * f64::from(b.height()) + (1. - proportion) * min) / 2. * (1. - squeeze);
    if ![cx, cy, rx, ry].iter().all(|v| v.is_finite() && v.abs() <= 1_000_000.) {
        return Err("vignette geometry exceeds bounded native path coordinates".into());
    }
    geometry_bytes(c, 1, 64)?;
    let mut path = if radius == 0. || rx == 0. || ry == 0. {
        Path::new(Vec::new())
    } else {
        let points: Vec<_> = match kind {
            1 => vec![(-rx, -ry), (rx, -ry), (rx, ry), (-rx, ry)],
            2 => vec![(0., -ry), (rx, 0.), (0., ry), (-rx, 0.)],
            3 => vec![(-1_000_000., -ry), (1_000_000., -ry), (1_000_000., ry), (-1_000_000., ry)],
            4 => vec![(-rx, -1_000_000.), (rx, -1_000_000.), (rx, 1_000_000.), (-rx, 1_000_000.)],
            _ => (0..64)
                .map(|n| {
                    let a = f64::from(n) * std::f64::consts::TAU / 64.;
                    (rx * a.cos(), ry * a.sin())
                })
                .collect(),
        };
        Path::new(vec![Subpath::polygon(&points)]).transform(&Affine::translate(cx, cy).mul(&Affine::rotate(angle)))
    };
    // An empty native vector mask reveals all, so radius0 is an intentional full paint.
    let mask = if path.is_empty() {
        None
    } else {
        path.inverted = true;
        let mut mask = VectorMask::new(path);
        mask.feather = (soft * rx.min(ry) / 3. / gamma.max(0.01).sqrt()) as f32;
        Some(mask)
    };
    Ok(composite(
        vec![NativeBranch::default(), NativeBranch { fill: Some(Fill::Solid(paint)), vector_mask: mask, ..Default::default() }],
        "Native solid paint with editable inverted circle/rectangle/diamond/band vector mask preserves vignette center, relative radius, aspect, squeeze and rotation. Native Gaussian feather uses softness*minor-radius/(3*sqrt(gamma)); this disclosed falloff policy replaces source gamma curve and shape metric. Zero radius paints the whole native owner; native alpha/ICC/compositing govern",
    ))
}
fn nova(e: &StoredEffect<'_>, c: &MappingContext) -> Result<NativeMapping> {
    let b = bounds(c)?;
    let x = num(e, "center-x", 0.5, -f64::MAX, f64::MAX)?;
    let y = num(e, "center-y", 0.5, -f64::MAX, f64::MAX)?;
    let radius = integer(e, "radius", 20, 1, 20000)?;
    let spokes = integer(e, "spokes-count", 100, 1, 1024)?;
    let hue = integer(e, "random-hue", 0, 0, 360)?;
    let mut rng = seed(e)?;
    let paint = color(e, "color", [0., 0., 1., 1.], c)?;
    let cx = f64::from(b.x0) + x * f64::from(b.width());
    let cy = f64::from(b.y0) + y * f64::from(b.height());
    if !cx.is_finite() || !cy.is_finite() || cx.abs() > 1_000_000. || cy.abs() > 1_000_000. {
        return Err("supernova center exceeds bounded native geometry".into());
    }
    let bytes = (spokes as usize).checked_mul(4096).ok_or_else(|| invalid("native starburst geometry bytes"))?;
    if bytes > c.remaining_metadata_bytes {
        return Err(MappingFailure::resource("native starburst layers exceed actual remaining metadata allowance"));
    }
    generation_bytes(c, bytes)?;
    let mut children = Vec::new();
    children.try_reserve_exact(spokes as usize).map_err(|_| MappingFailure::resource("native starburst layers allocation"))?;
    for n in 0..spokes {
        generation_check(c)?;
        let jitter = f64::from(random(&mut rng)) / f64::from(u32::MAX);
        let a = std::f64::consts::TAU * (f64::from(n) + jitter * 0.3) / f64::from(spokes);
        let r = f64::from(radius) * (0.6 + 0.4 * jitter);
        let half = std::f64::consts::PI / f64::from(spokes.max(3)) * 0.2;
        let mut p = paint;
        if p.mode == photocraft_color::ColorMode::Rgb && hue != 0 {
            let t = (f64::from(hue) * (jitter - 0.5)).to_radians();
            let (sin, cos) = t.sin_cos();
            let [r, g, b, _] = p.c;
            let mean = (r + g + b) / 3.;
            let k = sin as f32 / 3f32.sqrt();
            let cc = cos as f32;
            p.c = [mean + (r - mean) * cc + (b - g) * k, mean + (g - mean) * cc + (r - b) * k, mean + (b - mean) * cc + (g - r) * k, 0.];
        }
        let mut edge = p;
        edge.alpha = 0.;
        let mut layer = shape(
            "Supernova ray",
            vec![Subpath::polygon(&[(cx, cy), (cx + r * (a - half).cos(), cy + r * (a - half).sin()), (cx + r * (a + half).cos(), cy + r * (a + half).sin())])],
            p,
        );
        if let LayerContent::Shape(ref mut shape) = layer.content {
            let mut gradient =
                Fill::gradient(vec![(0., p), (1., edge)], 0., 2. * radius as f32 / b.width() as f32, photocraft_doc::GradientStyle::Radial, false);
            if let Fill::Gradient { align, offset, .. } = &mut gradient {
                *align = false;
                *offset = ((x - 0.5) as f32, (y - 0.5) as f32);
            }
            shape.fill = Some(gradient);
        }
        layer.blend = BlendMode::Screen;
        children.push(layer);
    }
    Ok(replace(
        Layer::new("Imported native starburst", LayerContent::Group(Group { children, expanded: true, artboard: None })),
        format!(
            "Source center/radius/spoke count/seed/color become finite editable native gradient-filled ray shapes. Native seeded angular/length variation and RGB-axis color rotation approximate random-hue{hue}; native finite ray extent, Screen overlap, gradient framing and attenuation differ from source light scattering. No imaginary source brightness field or baked starburst"
        ),
    ))
}
fn spiral(e: &StoredEffect<'_>, c: &MappingContext) -> Result<NativeMapping> {
    let b = bounds(c)?;
    let kind = choice(e, "type", 0, &[0, 1])?;
    let direction = choice(e, "direction", 0, &[0, 1])?;
    let x = num(e, "x", 0.5, -f64::MAX, f64::MAX)?;
    let y = num(e, "y", 0.5, -f64::MAX, f64::MAX)?;
    let radius = num(e, "radius", 100., 1., f64::MAX)?;
    let base = num(e, "base", 2., 1., f64::MAX)?;
    let balance = num(e, "balance", 0., -1., 1.)?;
    let rotation = num(e, "rotation", 0., 0., 360.)?.to_radians();
    let width = integer(e, "width", 1024, 0, i32::MAX)?;
    let height = integer(e, "height", 768, 0, i32::MAX)?;
    let a = color(e, "color1", [0., 0., 0., 1.], c)?;
    let z = color(e, "color2", [1., 1., 1., 1.], c)?;
    if kind == 1 && base == 1. {
        geometry_bytes(c, 1, 4)?;
        let background = Layer::new("Uniform spiral color1", LayerContent::Fill(Fill::Solid(a)));
        let mut foreground = Layer::new("Uniform spiral color2", LayerContent::Fill(Fill::Solid(z)));
        foreground.opacity = ((1. - balance) / 2.) as f32;
        let mut layer =
            Layer::new("Imported native uniform spiral", LayerContent::Group(Group { children: vec![background, foreground], expanded: true, artboard: None }));
        layer.vector_mask =
            Some(VectorMask::new(Path::new(vec![rect(0., 0., f64::from((width as u32).min(b.width())), f64::from((height as u32).min(b.height())))])));
        return Ok(replace(
            layer,
            "Source logarithmic spiral base1 has independently observed spatially uniform output. Preserve both typed colors as editable native Solid Fill layers with explicit native source-over composition: color1 under color2 at opacity(1-balance)/2. Native ICC, color transfer, straight/premultiplied alpha and source-over semantics replace the source degenerate blend; the stock typed-color/alpha output is not asserted to be a raw midpoint. Center, radius, rotation and direction do not deform this uniform native boundary result. Source width/height define the editable finite extent; no source spiral evaluator remains",
        ));
    }
    let cx = f64::from(b.x0) + x * f64::from(b.width());
    let cy = f64::from(b.y0) + y * f64::from(b.height());
    if ![cx, cy, radius, base].iter().all(|v| v.is_finite() && v.abs() <= 1_000_000.) {
        return Err("spiral geometry exceeds bounded native coordinate domain".into());
    }
    let max_radius =
        [(b.x0, b.y0), (b.x0, b.y1), (b.x1, b.y0), (b.x1, b.y1)].into_iter().map(|(x, y)| (f64::from(x) - cx).hypot(f64::from(y) - cy)).fold(0f64, f64::max);
    let turns = if kind == 0 { max_radius / radius + 1. } else { (max_radius / radius).max(1.).ln() / base.ln() + 1. };
    let samples = (turns * 64.).ceil().max(64.);
    if samples > 32_000. {
        return Err("spiral sampled native outline exceeds bounded geometry work allowance".into());
    }
    let count = samples as u32;
    geometry_bytes(c, 1, (count as usize + 1).checked_mul(2).ok_or_else(|| invalid("spiral knot bytes"))?)?;
    let mut points = Vec::new();
    points.try_reserve_exact((count as usize + 1) * 2).map_err(|_| MappingFailure::resource("native spiral outline allocation"))?;
    let span = turns * std::f64::consts::TAU;
    let fraction = (balance + 1.) / 2.;
    for reverse in [false, true] {
        for n in 0..=count {
            generation_check(c)?;
            let t = f64::from(if reverse { count - n } else { n }) / f64::from(count) * span;
            let r = if kind == 0 { radius * t / std::f64::consts::TAU } else { radius * base.powf(t / std::f64::consts::TAU) };
            let r = if reverse { r + radius * fraction } else { r };
            let angle = rotation + if direction == 0 { t } else { -t };
            points.push((cx + r * angle.cos(), cy + r * angle.sin()));
        }
    }
    if points.iter().any(|(x, y)| !x.is_finite() || !y.is_finite() || x.abs() > 1_000_000. || y.abs() > 1_000_000.) {
        return Err("spiral native outline coordinate exceeds finite bound".into());
    }
    let background = Layer::new("Spiral background", LayerContent::Fill(Fill::Solid(a)));
    let band = shape("Spiral band", vec![Subpath::polygon(&points)], z);
    let mut layer = Layer::new("Imported native spiral", LayerContent::Group(Group { children: vec![background, band], expanded: true, artboard: None }));
    layer.vector_mask = Some(VectorMask::new(Path::new(vec![rect(0., 0., f64::from(width.min(b.width() as i32)), f64::from(height.min(b.height() as i32)))])));
    Ok(replace(
        layer,
        "Source linear/logarithmic spiral intent becomes a bounded editable native two-color ribbon: radius supplies turn spacing/initial logarithmic radius, base growth, balance band width, center/rotation/direction and extent are explicit. This independently specified native ribbon geometry and vector antialiasing approximate source alternating spiral paint; no source evaluator is embedded",
    ))
}

fn style_bevel(e: &StoredEffect<'_>, c: &MappingContext) -> Result<Bevel> {
    let mode = choice(e, "bevel-blend", 0, &[0, 1, 2, 3, 4])?;
    let kind = choice(e, "bevel-type", 1, &[0, 1])?;
    let depth = integer(e, "bevel-depth", 65, 1, 100)?;
    let elevation = num(e, "bevel-elevation", 55., 55., 125.)?;
    let angle = num(e, "bevel-azimuth", 75., 0., 360.)?;
    let radius = num(e, "bevel-radius", 5., 1., 8.)?;
    let low = num(e, "bevel-outlow", 0., 0., 0.2)?;
    let high = num(e, "bevel-outhigh", 1., 1., 1.2)?;
    let dark = num(e, "bevel-dark", 0., 0., 1.)?;
    let white = super::resources::native_rgba([high as f32, high as f32, high as f32, 1.], c).map_err(invalid)?;
    let black = super::resources::native_rgba([0., 0., 0., 1.], c).map_err(invalid)?;
    let blend = match mode {
        1 => BlendMode::LinearDodge,
        2 => BlendMode::HardLight,
        3 => BlendMode::Darken,
        4 => BlendMode::ColorDodge,
        _ => BlendMode::Multiply,
    };
    Ok(Bevel {
        enabled: true,
        style: BevelStyle::InnerBevel,
        technique: if kind == 0 { BevelTechnique::ChiselHard } else { BevelTechnique::Smooth },
        depth: depth as f32 / 100.,
        up: true,
        size: radius as f32,
        soften: 0.,
        angle: angle as f32,
        altitude: (elevation - 90.).abs() as f32,
        use_global_light: false,
        gloss_contour: if low == 0. && high == 1. {
            Contour::Linear
        } else {
            Contour::Custom {
                name: "Native imported bevel tone".into(),
                points: vec![
                    photocraft_doc::adjust::CurvePoint { input: 0., output: (low / high) as f32 },
                    photocraft_doc::adjust::CurvePoint { input: 1., output: 1. },
                ],
                corners: vec![false, false],
            }
        },
        gloss_anti_alias: true,
        highlight: FxCommon::new(BlendMode::Screen, 1.),
        highlight_color: white,
        shadow: FxCommon::new(blend, (1. - dark) as f32),
        shadow_color: black,
        contour: None,
        texture: None,
    })
}
fn styles(e: &StoredEffect<'_>, c: &MappingContext) -> Result<NativeMapping> {
    choice(e, "guichange", 0, &[0, 1, 2])?;
    if boolean(e, "enableimage", false)? || boolean(e, "enableimageoutline", false)? {
        return Err(MappingFailure::resource(
            "Style image/outline-image is enabled but only its source locator/configuration is serialized; no authorized actual image pixels are available, and source image-src/os-src never authorizes file reads",
        ));
    }
    let special = boolean(e, "enablespecialoutline", false)?;
    let special_bevel = special && boolean(e, "enableoutlinebevel", true)?;
    let mut effects = Vec::new();
    let policy = choice(e, "color-policy", 1, &[0, 1, 2])?;
    if policy != 0 {
        let mut paint = color(e, "color-fill", [1., 1., 1., 1.], c)?;
        let opacity = paint.alpha;
        paint.alpha = 1.;
        effects.push(Effect::ColorOverlay { common: FxCommon::new(if policy == 1 { BlendMode::Multiply } else { BlendMode::Normal }, opacity), color: paint });
    }
    let outline_start = effects.len();
    let mut morphological_outline = None;
    if boolean(e, "enableoutline", false)? || special {
        let width = num(e, "outline", 12., 0., f64::MAX)?;
        let x = num(e, "outline-x", 0., -f64::MAX, f64::MAX)?;
        let y = num(e, "outline-y", 0., -f64::MAX, f64::MAX)?;
        let grow_shape = choice(e, "outline-grow-shape", 1, &[0, 1, 2])?;
        let blur = num(e, "outline-blur", 0., 0., 3.)?;
        let opacity = num(e, "outline-opacity", 1., 0., 1.)?;
        let mut paint = color(e, "outline-color", [0., 0., 0., 1.], c)?;
        let alpha = paint.alpha;
        paint.alpha = 1.;
        if width > 1_000_000. || x.hypot(y) > 1_000_000. {
            return Err("style outline exceeds bounded native style geometry".into());
        }
        if width == 0. {
            // A zero-width outline is absent, even when an offset/blur is configured.
        } else if grow_shape != 1 {
            let radius = width.ceil() as usize;
            let count = radius.div_ceil(500);
            if count + usize::from(blur > 0.) > 64 {
                return Err(MappingFailure::resource("native outline morphology exceeds64 bounded filter passes"));
            }
            generation_bytes(c, count * 2048 + 8192)?;
            let mut filters = Vec::new();
            filters.try_reserve_exact(count + usize::from(blur > 0.)).map_err(|_| MappingFailure::resource("native outline filters unavailable"))?;
            let mut remaining = radius;
            while remaining > 0 {
                let step = remaining.min(500);
                filters.push(if grow_shape == 0 {
                    native_filter("filter.other.maximum", json!({"radius":step,"preserve":"squareness"}))
                } else {
                    native_filter("filter.other.percentile", json!({"radius":step,"colorPercentile":100,"alphaPercentile":100,"shape":"diamond"}))
                });
                remaining -= step;
            }
            if blur > 0. {
                filters.push(native_filter("filter.blur.gaussianBlur", json!({"radius":blur.max(0.1)})));
            }
            morphological_outline = Some(NativeBranch {
                filters,
                opacity: opacity as f32 * alpha,
                transform: Affine::translate(x, y),
                layer_effects: vec![Effect::ColorOverlay { common: FxCommon::new(BlendMode::Normal, 1.), color: paint }],
                ..Default::default()
            });
        } else if x == 0. && y == 0. && blur == 0. {
            effects.push(Effect::Stroke(StrokeFx {
                common: FxCommon::new(BlendMode::Normal, opacity as f32 * alpha),
                size: width as f32,
                position: StrokePosition::Outside,
                paint: FxPaint::Color(paint),
                overprint: false,
            }));
        } else {
            effects.push(Effect::DropShadow(Shadow {
                common: FxCommon::new(BlendMode::Normal, opacity as f32 * alpha),
                color: paint,
                angle: (-y.atan2(x).to_degrees() + 180.) as f32,
                use_global_light: false,
                distance: x.hypot(y) as f32,
                spread: 1.,
                size: (width + blur) as f32,
                contour: Contour::Linear,
                anti_alias: true,
                noise: 0.,
                knocks_out: true,
            }));
        }
    }
    let outline_effects = if special_bevel { effects.split_off(outline_start) } else { Vec::new() };
    let opacity = num(e, "shadow-opacity", 0., 0., 1.)?;
    if opacity > 0. {
        let x = num(e, "shadow-x", 10., -f64::MAX, f64::MAX)?;
        let y = num(e, "shadow-y", 10., -f64::MAX, f64::MAX)?;
        let size = num(e, "shadow-radius", 12., 0., f64::MAX)?;
        let grow = num(e, "shadow-grow-radius", 0., 0., 100.)?;
        let mut paint = color(e, "shadow-color", [0., 0., 0., 1.], c)?;
        let alpha = paint.alpha;
        paint.alpha = 1.;
        if x.hypot(y) > 1_000_000. || size > 1_000_000. {
            return Err("source style shadow exceeds native bounded geometry".into());
        }
        effects.push(Effect::DropShadow(Shadow {
            common: FxCommon::new(BlendMode::Normal, opacity as f32 * alpha),
            color: paint,
            angle: (-y.atan2(x).to_degrees() + 180.) as f32,
            use_global_light: false,
            distance: x.hypot(y) as f32,
            spread: (grow / (grow + size.max(0.001))) as f32,
            size: size as f32,
            contour: Contour::Linear,
            anti_alias: true,
            noise: 0.,
            knocks_out: true,
        }));
    }
    if boolean(e, "enableinnerglow", false)? {
        let mode = choice(e, "ig-blend", 0, &[0, 1, 2, 3, 4, 5])?;
        let radius = num(e, "ig-radius", 6., 0., 30.)?;
        let grow = num(e, "ig-grow-radius", 5., 1., 30.)?;
        let opacity = num(e, "ig-opacity", 1., 0., 1.)?;
        let treatment = num(e, "ig-treatment", 60., 50., 85.)?;
        let mut paint = color(e, "ig-value", [1., 143. / 255., 0., 1.], c)?;
        let alpha = paint.alpha;
        paint.alpha = 1.;
        effects.push(Effect::InnerGlow(Glow {
            common: FxCommon::new(
                match mode {
                    1 => BlendMode::Overlay,
                    2 => BlendMode::Screen,
                    3 => BlendMode::HardLight,
                    4 => BlendMode::ColorDodge,
                    5 => BlendMode::LinearDodge,
                    _ => BlendMode::Normal,
                },
                opacity as f32 * alpha,
            ),
            paint: FxPaint::Color(paint),
            technique: GlowTechnique::Precise,
            spread: (grow / (grow + radius.max(0.001))) as f32,
            size: radius as f32,
            contour: Contour::Linear,
            anti_alias: true,
            range: (treatment / 100.) as f32,
            jitter: 0.,
            noise: 0.,
            source: GlowSource::Edge,
        }));
    }
    if boolean(e, "enablebevel", false)? {
        effects.push(Effect::BevelEmboss(style_bevel(e, c)?));
    }
    if let Some(mut outline) = morphological_outline {
        if special_bevel {
            outline.post_layer_effects.push(Effect::BevelEmboss(style_bevel(e, c)?));
        }
        return Ok(composite(
            vec![
                outline,
                NativeBranch {
                    fill_opacity: 0.,
                    advanced: photocraft_doc::AdvancedBlending { knockout: photocraft_doc::Knockout::Shallow, ..Default::default() },
                    ..Default::default()
                },
                NativeBranch { layer_effects: effects, ..Default::default() },
            ],
            "Square/diamond outline intent becomes ordinary native Maximum-square/Percentile-diamond morphology, native Color Overlay and placement, followed by original-input fill0 Shallow Knockout and the styled original. The native stage is isolated, so cutout cannot erase unrelated document layers; partial coverage follows native knockout/compositing. Fractional width rounds outward to native pixel growth; blur uses the native Gaussian minimum0.1px. Nondefault special-outline bevel acts on grown outer coverage before cutout, so its inner-edge lighting differs from the circle Stroke/post-Bevel construction. Inner-glow treatment percent maps to native Glow Range/contour. No source style renderer or baked source appearance",
        ));
    }
    if special_bevel {
        let post = Effect::BevelEmboss(style_bevel(e, c)?);
        return Ok(composite(
            vec![
                NativeBranch { layer_effects: effects, ..Default::default() },
                NativeBranch { layer_effects: outline_effects, post_layer_effects: vec![post], fill_opacity: 0., ..Default::default() },
            ],
            "Special outline bevel uses an ordinary fill0 Stroke/shadow style source layer, embedded as native outline pixels/data, then native Bevel on that outline alpha beside the styled original. Source bevel output levels become native custom gloss endpoints and CMS-converted HDR highlight paint; native contour/highlight response and profile clipping differ from source graph levels. Inner-glow treatment percent becomes native Glow Range with its native contour transfer. Native style order, outline morphology, offset/coverage and bevel sampling apply; no source style interpreter remains",
        ));
    }
    Ok(NativeMapping{layer_effects:effects,notes:vec!["Enabled source styles become ordinary native overlay, stroke/offset shadow, drop shadow, inner glow and bevel. Bevel output levels use native custom gloss endpoints and CMS-converted HDR highlight paint, with native contour/color/profile behavior replacing source graph levels. Inner-glow treatment percent becomes native Glow Range with its native contour transfer. Native style order, morphology, lighting and alpha apply; GUI/source resource locators remain inert".into()],..Default::default()})
}

fn video(e: &StoredEffect<'_>, c: &MappingContext) -> Result<NativeMapping> {
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_doc::Pattern;
    use photocraft_raster::Surface;
    let pattern = choice(e, "pattern", 2, &[0, 1, 2, 3, 4, 5, 6, 7, 8])?;
    let additive = boolean(e, "additive", true)?;
    let rotated = boolean(e, "rotated", false)?;
    let (width, height, cw, ch) = match pattern {
        0 => (3, 2, 1, 1),
        1 => (6, 4, 2, 2),
        3 => (6, 1, 2, 1),
        4 => (3, 6, 1, 3),
        5 => (3, 3, 1, 1),
        6 => (9, 9, 3, 3),
        7 => (6, 4, 2, 2),
        8 => (6, 4, 1, 2),
        _ => (3, 1, 1, 1),
    };
    let channels = if c.mode == ColorMode::Grayscale { 2 } else { 4 };
    let paint_bytes = width as usize * height as usize * channels * 4;
    let hash_bytes = width as usize * channels * 4;
    surface_generation(c, photocraft_geom::Rect::from_xywh(0, 0, width, height), channels, 1, paint_bytes + hash_bytes)?;
    let mut pixels = Vec::new();
    pixels.try_reserve_exact(paint_bytes / 4).map_err(|_| MappingFailure::resource("video pattern scratch allocation"))?;
    for row in 0..height {
        generation_check(c)?;
        for col in 0..width {
            let (x, y) = (col / cw, row / ch);
            let index = match pattern {
                0 | 1 | 4 => (x + y % 2) % 3,
                5 | 6 => (x + y) % 3,
                7 => (x + 2 * y) % 3,
                8 => (x / 2 + y) % 3,
                _ => x % 3,
            };
            let low = if additive { 1. } else { 0. };
            let mut gain = [low, low, low, 1.];
            if let Some(v) = gain.get_mut(index as usize) {
                *v = if additive { 2. } else { 1. };
            }
            if c.mode == ColorMode::Grayscale {
                pixels.extend_from_slice(&[(gain[0] + gain[1] + gain[2]) / 3., 1.]);
            } else {
                pixels.extend_from_slice(&gain);
            }
        }
    }
    if pixels.len() != width as usize * height as usize * channels {
        return Err(invalid("native video pattern size"));
    }
    let mut surface = Surface::new(PixelFormat::new(c.mode, SampleType::F32, true));
    surface.try_write_region(photocraft_geom::Rect::from_xywh(0, 0, width, height), &pixels).map_err(|e| MappingFailure::resource(e.to_string()))?;
    let pattern_data = Pattern::try_new_in("Imported video channel gains", surface, width, height, hash_bytes, &c.interrupt)
        .map_err(|e| MappingFailure::resource(e.to_string()))?;
    let effect = Effect::PatternOverlay {
        common: FxCommon::new(BlendMode::Multiply, 1.),
        name: pattern_data.name.clone(),
        id: pattern_data.id.clone(),
        scale: 1.,
        angle: if rotated { 90. } else { 0. },
        link: true,
        phase: (0., 0.),
    };
    Ok(NativeMapping {
        patterns: vec![pattern_data],
        layer_effects: vec![effect],
        notes: vec![format!(
            "Native reusable F32 channel-gain pattern and ordinary Pattern Overlay preserve owner alpha and RGB video stripe/stagger intent, pattern selection{pattern}, rotation and additive flag. Additive uses selected native channel gain2/others1; replacement uses1/0. Independently specified finite cell layouts approximate source hardware patterns; Gray uses mean channel gain and native ICC/alpha/clipping govern. No source image, GIMP renderer or per-pixel runtime operation survives"
        )],
        ..Default::default()
    })
}

fn seamless(c: &MappingContext) -> Result<NativeMapping> {
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_doc::LayerMask;
    use photocraft_raster::Surface;
    let b = bounds(c)?;
    let row_bytes = (b.width() as usize).checked_mul(4).ok_or_else(|| invalid("seamless row allocation overflow"))?;
    surface_generation(c, b, 1, 3, row_bytes)?;
    let mut row = Vec::new();
    row.try_reserve_exact(b.width() as usize).map_err(|_| MappingFailure::resource("native seamless row scratch unavailable"))?;
    let mut branches = vec![NativeBranch::default()];
    for (offset_x, offset_y) in [(true, false), (false, true), (true, true)] {
        generation_check(c)?;
        let mut surface = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
        for y in b.y0..b.y1 {
            generation_check(c)?;
            row.clear();
            let t = f64::from(y - b.y0) / f64::from(b.height());
            let wy = (std::f64::consts::PI * t).sin().powi(2);
            for x in b.x0..b.x1 {
                let t = f64::from(x - b.x0) / f64::from(b.width());
                let wx = (std::f64::consts::PI * t).sin().powi(2);
                let weight = match (offset_x, offset_y) {
                    (true, false) => 1. - wx,
                    (false, true) => {
                        let top = wx * (1. - wy);
                        let denominator = wy + top;
                        if denominator == 0. { 0. } else { top / denominator }
                    }
                    _ => (1. - wx) * (1. - wy),
                };
                row.push(weight as f32);
            }
            surface.try_write_region(photocraft_geom::Rect::new(b.x0, y, b.x1, y + 1), &row).map_err(|e| MappingFailure::resource(e.to_string()))?;
        }
        let mut mask = LayerMask::hide_all();
        mask.surface = surface;
        branches.push(NativeBranch {
            filters: vec![native_filter(
                "filter.other.offset",
                json!({"horizontal":if offset_x{b.width()/2}else{0},"vertical":if offset_y{b.height()/2}else{0},"undefinedAreas":"wrap"}),
            )],
            mask: Some(mask),
            blend: BlendMode::Normal,
            ..Default::default()
        });
    }
    generation_check(c)?;
    Ok(composite(
        branches,
        "Source seamless intent becomes four ordinary half-wrap native Offset instances with complementary two-axis sin-squared native crossfade weights converted to conditional Normal-layer opacities, preserving opaque-source opacity. Three F32 mask planes use one scratch row, with actual tile+scratch preflight and per-row cancellation. Native integer offsets, alpha composition and original gradients differ from source seamless synthesis; no source width/radius/seed is invented",
    ))
}

fn stretch(e: &StoredEffect<'_>, hsv: bool) -> Result<NativeMapping> {
    let keep = if hsv { true } else { boolean(e, "keep-colors", true)? };
    let perceptual = if hsv { false } else { boolean(e, "perceptual", false)? };
    Ok(one(
        if keep { "image.autoContrast" } else { "image.autoTone" },
        json!({}),
        format!(
            "Contrast-stretch intent uses existing native Auto {}, analyzed once from actual prepared native pixels and rewritten to fixed editable Levels before first display/save. Native 0.1% histogram clipping, display-bin analysis and ICC/color policy replace source {} component/transfer behavior (perceptual{perceptual}); no source operation or automatic action remains after native preparation",
            if keep { "Contrast with shared channel endpoints" } else { "Tone with per-channel endpoints" },
            if hsv { "HSV value" } else { "RGB/Gray" }
        ),
    ))
}

pub(super) fn map(e: &StoredEffect<'_>, c: &MappingContext) -> MappingResult {
    if !matches!(
        e.canonical_operation(),
        "gegl:stress"
            | "gegl:vignette"
            | "gegl:supernova"
            | "gegl:spiral"
            | "gegl:styles"
            | "gegl:stretch-contrast"
            | "gegl:stretch-contrast-hsv"
            | "gegl:video-degradation"
            | "gegl:tile-seamless"
            | "gegl:waterpixels"
            | "gegl:tile-paper"
            | "gegl:value-propagate"
            | "gegl:texturize-canvas"
            | "gegl:unpremultiply"
            | "gegl:stereographic-projection"
            | "gegl:tile-glass"
    ) {
        return None;
    }
    if let Err(error) = super::residual_spatial::generation_bytes(c, 4096) {
        return Some(Err(error));
    }
    Some(match e.canonical_operation() {
        "gegl:stress" => stress(e, c),
        "gegl:vignette" => vignette(e, c),
        "gegl:supernova" => nova(e, c),
        "gegl:spiral" => spiral(e, c),
        "gegl:styles" => styles(e, c),
        "gegl:stretch-contrast" => stretch(e, false),
        "gegl:stretch-contrast-hsv" => stretch(e, true),
        "gegl:video-degradation" => video(e, c),
        "gegl:tile-seamless" => seamless(c),
        "gegl:waterpixels" => water(e, c),
        "gegl:tile-paper" => tile_paper(e, c),
        "gegl:value-propagate" => propagation(e, c),
        "gegl:texturize-canvas" => texturize(e),
        "gegl:unpremultiply" => unpremultiply(),
        "gegl:stereographic-projection" => stereographic(e, c),
        "gegl:tile-glass" => glass(e, c),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::super::{Argument, ArgumentValue};
    use super::*;
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_doc::Document;
    use photocraft_geom::{Rect, Size};
    use photocraft_raster::Surface;
    fn context() -> MappingContext<'static> {
        MappingContext {
            mode: ColorMode::Rgb,
            linear: false,
            owner_bounds: Rect::from_xywh(0, 0, 32, 24),
            native_filter_bounds: Rect::from_xywh(0, 0, 32, 24),
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
    fn flag(name: &'static str, value: bool) -> Argument<'static> {
        Argument { name, value: ArgumentValue::Bool(value) }
    }
    fn mapped_effect(op: &'static str, args: Vec<Argument<'static>>) -> NativeMapping {
        map(&StoredEffect::test(op, args), &context()).unwrap().unwrap()
    }
    #[test]
    fn all_sixteen_frozen_identities_are_accounted_for_without_fake_defaults() {
        let mut native = 0;
        for op in [
            "gegl:spiral",
            "gegl:stereographic-projection",
            "gegl:stress",
            "gegl:stretch-contrast",
            "gegl:stretch-contrast-hsv",
            "gegl:styles",
            "gegl:supernova",
            "gegl:texturize-canvas",
            "gegl:tile-glass",
            "gegl:tile-paper",
            "gegl:tile-seamless",
            "gegl:unpremultiply",
            "gegl:value-propagate",
            "gegl:video-degradation",
            "gegl:vignette",
            "gegl:waterpixels",
        ] {
            if let Ok(mapping) = map(&StoredEffect::test(op, Vec::new()), &context()).unwrap() {
                native += 1;
                assert!(!mapping.notes.is_empty(), "{op}");
                assert!(mapping.filters.iter().all(|f| !f.command.starts_with("gegl:")));
            }
        }
        assert_eq!(native, 16);
        assert!(map(&StoredEffect::test("other-operation", Vec::new()), &context()).is_none());
    }
    #[test]
    fn transparent_paper_keeps_editable_masks_gaps_native_alpha_and_partial_underlay() {
        use photocraft_doc::{SmartObject, SmartSource};
        use std::sync::Arc;
        let c = context();
        let result = transparent_paper(&c, (8, 8), 100., false, 2, false, 3).unwrap();
        let Some(super::super::NativeStructure::Composite { branches }) = result.structure else { panic!("native paper copies") };
        assert!(!branches.is_empty() && branches.len() <= 64);
        let mut original = Document::new("source", Size::new(32, 24), ColorMode::Rgb, SampleType::F32);
        let mut pixels = Surface::new(original.pixel_format());
        pixels.fill_rect(original.bounds(), &[1., 0., 0., 1.]);
        original.layers.push(Layer::new("original", LayerContent::Raster(pixels.clone())));
        let source = SmartSource::Embedded {
            file_name: "source.pcraft".into(),
            bytes: Arc::new(photocraft_format::save_to_bytes(&original, &Default::default()).unwrap()),
        };
        let mut layers = Vec::new();
        for branch in branches {
            assert!(branch.fill.is_none());
            assert!(branch.vector_mask.is_some());
            let placed = photocraft_algo::warp::place_source(&pixels, original.bounds(), &branch.transform, None).unwrap();
            let mut layer = Layer::new("editable paper tile", LayerContent::Smart(SmartObject::new(source.clone(), branch.transform, Some(placed))));
            layer.vector_mask = branch.vector_mask;
            layers.push(layer);
        }
        let mut doc = Document::new("native transparent paper", original.size, original.mode, original.depth);
        doc.layers.push(Layer::group("native paper", layers));
        let full = photocraft_compose::render(&doc, doc.bounds()).px;
        assert!(full.iter().any(|p| p[3] == 0.));
        assert!(full.iter().any(|p| p[3] == 1.));
        let saved = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
        assert_eq!(full, photocraft_compose::render(&reopened, reopened.bounds()).px);
        let mut blue = Surface::new(doc.pixel_format());
        blue.fill_rect(doc.bounds(), &[0., 0., 1., 1.]);
        doc.layers[0].opacity = 0.4;
        doc.layers.insert(0, Layer::new("ordinary native underlay", LayerContent::Raster(blue)));
        let mixed = photocraft_compose::render(&doc, doc.bounds()).px;
        for (a, b) in full.iter().zip(&mixed) {
            assert_eq!(b[3], 1.);
            if a[3] == 0. {
                assert_eq!(*b, [0., 0., 1., 1.]);
            }
        }
        let empty = transparent_paper(&c, (100, 100), 100., false, 0, true, 3).unwrap();
        let Some(super::super::NativeStructure::Composite { branches }) = empty.structure else { panic!() };
        assert_eq!(branches.len(), 1);
        assert!(matches!(branches[0].fill,Some(Fill::Solid(paint)) if paint.alpha==0.));
        assert!(transparent_paper(&c, (1, 1), 100., false, 2, false, 3).is_err());
        let tiny = MappingContext { remaining_metadata_bytes: 1, ..context() };
        assert!(transparent_paper(&tiny, (8, 8), 100., false, 2, false, 3).is_err());
    }
    #[test]
    fn alpha_unmixing_maps_to_existing_editable_native_matte_action() {
        let result = mapped_effect("gegl:unpremultiply", Vec::new());
        assert_eq!(result.filters[0].command, "layer.matting.removeBlackMatte");
        assert_eq!(result.filters[0].params, json!({}));
        assert!(result.notes[0].contains("Source linear-light"));
        assert!(result.notes[0].contains("retains coverage"));
    }
    #[test]
    fn canvas_intent_uses_native_procedural_weave_and_exact_diagonal_light_enum() {
        for (value, light) in [(0, "topRight"), (1, "topLeft"), (2, "bottomLeft"), (3, "bottomRight")] {
            let result = mapped_effect("gegl:texturize-canvas", vec![Argument { name: "direction", value: ArgumentValue::Enum(value) }, int("depth", 50)]);
            assert_eq!(result.filters[0].command, "filter.gallery.texturizer");
            assert_eq!(result.filters[0].params, json!({"texture":"canvas","scaling":100,"relief":50,"light":light,"invert":false}));
            assert!(result.notes[0].contains("not copied"));
        }
    }
    #[test]
    fn stretch_selects_native_auto_intent_without_inventing_histogram_endpoints() {
        let keep = mapped_effect("gegl:stretch-contrast", Vec::new());
        assert_eq!(keep.filters[0].command, "image.autoContrast");
        assert_eq!(keep.filters[0].params, json!({}));
        let channels = mapped_effect("gegl:stretch-contrast", vec![flag("keep-colors", false), flag("perceptual", true)]);
        assert_eq!(channels.filters[0].command, "image.autoTone");
        let hsv = mapped_effect("gegl:stretch-contrast-hsv", Vec::new());
        assert_eq!(hsv.filters[0].command, "image.autoContrast");
        assert!(hsv.notes[0].contains("HSV value"));
        assert!(hsv.notes[0].contains("0.1%"));
    }
    #[test]
    fn stereographic_map_keeps_observed_longitude_and_pole_orientation() {
        let result = mapped_effect("gegl:stereographic-projection", Vec::new());
        let Some(super::super::NativeStructure::Composite { branches }) = &result.structure else { panic!("native camera source") };
        let alias = mapped_effect("gegl:little-planet", Vec::new());
        let Some(super::super::NativeStructure::Composite { branches: alias_branches }) = &alias.structure else { panic!("native alias source") };
        assert_eq!(branches[0].filters, alias_branches[0].filters);
        let filter = &branches[0].filters[0];
        assert_eq!(filter.command, "filter.distort.displace");
        let image = crate::smart_map::displace_embedded_map(&filter.params).unwrap();
        let uv = |x: usize, y: usize| {
            let i = (y * 32 + x) * 3;
            let rx = filter.params["horizontal"].as_f64().unwrap() * 128. / 100.;
            let ry = filter.params["vertical"].as_f64().unwrap() * 128. / 100.;
            (x as f64 + 0.5 + (f64::from(image.data[i]) - 0.5) * 2. * rx, y as f64 + 0.5 + (f64::from(image.data[i + 1]) - 0.5) * 2. * ry)
        };
        assert!(uv(0, 12).0 > uv(31, 12).0, "observed longitude orientation");
        assert!(uv(16, 12).1 > uv(0, 12).1, "observed pole-facing radial latitude");
        assert!(result.notes[0].contains("Native aperture"));
    }
    #[test]
    fn stereographic_camera_controls_author_finite_native_maps_and_virtual_output_masks() {
        let baseline = mapped_effect("gegl:stereographic-projection", vec![float("tilt", 0.)]);
        let Some(super::super::NativeStructure::Composite { branches }) = &baseline.structure else { panic!() };
        let base = &branches[0].filters[0];
        assert_eq!(base.command, "filter.distort.displace");
        for args in [
            vec![float("tilt", 30.)],
            vec![float("tilt", 0.), float("pan", 40.)],
            vec![float("tilt", 0.), float("spin", 75.)],
            vec![float("tilt", 0.), float("zoom", 200.)],
            vec![float("tilt", 0.), int("width", 17)],
            vec![flag("inverse", true)],
        ] {
            let changed = mapped_effect("gegl:stereographic-projection", args);
            let Some(super::super::NativeStructure::Composite { branches }) = changed.structure else { panic!() };
            assert_ne!(branches[0].filters[0].params, base.params);
            assert!(branches[0].vector_mask.is_some());
            let map = crate::smart_map::displace_embedded_map(&branches[0].filters[0].params).unwrap();
            assert!(map.data.iter().all(|v| v.is_finite()));
        }
        let bounded = MappingContext { remaining_metadata_bytes: 1, ..context() };
        assert!(map(&StoredEffect::test("gegl:stereographic-projection", vec![float("tilt", 0.)]), &bounded).unwrap().is_err());
        let empty = mapped_effect("gegl:stereographic-projection", vec![int("width", 0)]);
        let Some(super::super::NativeStructure::Composite { branches }) = empty.structure else { panic!() };
        assert!(matches!(branches[0].fill,Some(Fill::Solid(paint)) if paint.alpha==0.));
        for args in [
            vec![float("pan", 360.), float("tilt", 180.), float("spin", 360.), float("zoom", 0.01), flag("inverse", true)],
            vec![float("pan", -360.), float("tilt", -180.), float("spin", -360.), float("zoom", 1000.)],
        ] {
            assert!(map(&StoredEffect::test("gegl:stereographic-projection", args), &context()).unwrap().is_ok());
        }
    }
    #[test]
    fn native_video_pattern_is_small_reusable_and_preserves_owner_alpha() {
        let mapping = mapped_effect("gegl:video-degradation", Vec::new());
        assert_eq!(mapping.patterns.len(), 1);
        assert_eq!((mapping.patterns[0].width, mapping.patterns[0].height), (3, 1));
        assert_eq!(mapping.patterns[0].surface.pixel(0, 0), vec![2., 1., 1., 1.]);
        assert_eq!(mapping.patterns[0].surface.pixel(1, 0), vec![1., 2., 1., 1.]);
        let mut source = Surface::new(PixelFormat::RGBA32F);
        source.write_region(Rect::from_xywh(0, 0, 3, 1), &[0.2, 0.3, 0.4, 0.25, 0.2, 0.3, 0.4, 0.5, 0.2, 0.3, 0.4, 0.75]);
        let mut layer = Layer::new("Original", LayerContent::Raster(source));
        layer.effects.items = mapping.layer_effects;
        let mut doc = Document::new("Native pattern", Size::new(3, 1), ColorMode::Rgb, SampleType::F32);
        doc.layers.push(layer);
        doc.patterns = mapping.patterns;
        let pixels = photocraft_compose::render(&doc, doc.bounds()).px;
        for (p, a) in pixels.iter().zip([0.25, 0.5, 0.75]) {
            assert!((p[3] - a).abs() < 1e-6);
        }
        let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
        assert_eq!(reopened.patterns, doc.patterns);
        assert_eq!(photocraft_compose::render(&reopened, reopened.bounds()).px, pixels);
    }
    #[test]
    fn logarithmic_spiral_unit_base_is_native_uniform_color_composition_with_finite_extent() {
        use photocraft_color::{Color, SampleType};
        use photocraft_doc::{Document, Layer, LayerContent};
        use photocraft_geom::Size;
        let arguments = || {
            vec![
                Argument { name: "type", value: ArgumentValue::Enum(1) },
                float("base", 1.),
                float("balance", 0.75),
                int("width", 4),
                int("height", 3),
                Argument {
                    name: "color1",
                    value: ArgumentValue::Color(Some(super::super::ColorLiteral { encoding: "R'G'B'A u8", pixels: &[51, 102, 204, 64], profile: &[] })),
                },
                Argument {
                    name: "color2",
                    value: ArgumentValue::Color(Some(super::super::ColorLiteral { encoding: "R'G'B'A u8", pixels: &[204, 51, 102, 192], profile: &[] })),
                },
            ]
        };
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mapping = spiral(&StoredEffect::test("gegl:spiral", arguments()), &context()).unwrap();
            let Some(super::super::NativeStructure::Replace { layer, .. }) = mapping.structure else { panic!("native uniform spiral") };
            let LayerContent::Group(group) = &layer.content else { panic!("native Fill group") };
            assert_eq!(group.children.len(), 2);
            assert_eq!(group.children[1].opacity, 0.125);
            assert!(group.children.iter().all(|layer| matches!(layer.content, LayerContent::Fill(Fill::Solid(_)))));
            let mut actual = Document::new("Native uniform spiral", Size::new(8, 6), ColorMode::Rgb, depth);
            actual.layers.push(*layer);
            let mut expected = Document::new("Independent native source-over", Size::new(8, 6), ColorMode::Rgb, depth);
            expected.layers.push(Layer::new("Color1", LayerContent::Fill(Fill::Solid(Color::rgba(51. / 255., 102. / 255., 204. / 255., 64. / 255.)))));
            let mut top = Layer::new("Color2", LayerContent::Fill(Fill::Solid(Color::rgba(204. / 255., 51. / 255., 102. / 255., 192. / 255.))));
            top.opacity = 0.125;
            expected.layers.push(top);
            let pixels = photocraft_compose::render(&actual, actual.bounds());
            let reference = photocraft_compose::render(&expected, expected.bounds());
            assert_eq!(pixels.get(1, 1), reference.get(1, 1));
            assert_eq!(pixels.get(6, 4)[3], 0.0);
            assert_ne!(pixels.get(1, 1)[3], 0.875); // Independent stock alpha is intentionally not emulated.
            let bytes = photocraft_format::save_to_bytes(&actual, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
            assert_eq!(photocraft_compose::render(&reopened, reopened.bounds()).px, pixels.px);
        }
        for c in [MappingContext { remaining_metadata_bytes: 1, ..context() }, MappingContext { remaining_generation_bytes: 1, ..context() }] {
            assert!(spiral(&StoredEffect::test("gegl:spiral", arguments()), &c).is_err());
        }
        let cancel = || true;
        assert!(
            spiral(
                &StoredEffect::test("gegl:spiral", arguments()),
                &MappingContext { interrupt: photocraft_raster::Interrupt::cancel_only(&cancel), ..context() }
            )
            .is_err()
        );
    }
    #[test]
    fn glass_cell_periods_have_native_refraction_counterparts_across_full_domain() {
        use photocraft_color::{PixelFormat, SampleType};
        use photocraft_doc::{Document, Layer, LayerContent, SmartObject, SmartSource};
        use photocraft_geom::{Affine, Size};
        use photocraft_raster::{Interrupt, Surface};
        let bounds = Rect::new(0, 0, 5, 2);
        let c = MappingContext { owner_bounds: bounds, native_filter_bounds: bounds, ..context() };
        let mapping = glass(&StoredEffect::test("gegl:tile-glass", vec![int("tile-width", 5), int("tile-height", 10)]), &c).unwrap();
        let filter = &mapping.filters[0];
        let decoded = crate::smart_map::displace_embedded_map(&filter.params).unwrap();
        // Independent half-strength cell5x10: dx=[1,.5,0,-.5,-1],
        // dy=[2.25,1.75]; native U8 plane encodings follow from those extrema.
        let independent = std::sync::Arc::new(photocraft_algo::Image {
            rect: bounds,
            ch: 3,
            data: [255., 227.].into_iter().flat_map(|y| [255., 191., 128., 64., 0.].into_iter().flat_map(move |x| [x / 255., y / 255., 0.5])).collect(),
        });
        assert_eq!(filter.params["undefinedAreas"], "repeat");
        assert!((filter.params["horizontal"].as_f64().unwrap() - 100. / 128.).abs() < 1e-8);
        assert!((filter.params["vertical"].as_f64().unwrap() - 2.25 * 100. / 128.).abs() < 1e-8);
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut source = Surface::new(PixelFormat::new(ColorMode::Rgb, depth, true));
            source.write_region(bounds, &(0..10).flat_map(|i| [i as f32 / 9., 0.25, 0.75, 0.25 + i as f32 / 16.]).collect::<Vec<_>>());
            let apply = |horizontal, vertical, map| {
                photocraft_algo::apply_in_with(
                    &source,
                    &photocraft_algo::FilterParams::Displace {
                        horizontal,
                        vertical,
                        stretch: true,
                        undefined: photocraft_algo::UndefinedAreas::Repeat,
                        map: Some(map),
                    },
                    bounds,
                    bounds,
                    None,
                    bounds,
                    &Interrupt::NONE,
                )
                .unwrap()
                .read_region(bounds)
            };
            let horizontal = filter.params["horizontal"].as_f64().unwrap() as f32;
            let vertical = filter.params["vertical"].as_f64().unwrap() as f32;
            let actual = apply(horizontal, vertical, decoded.clone());
            assert_eq!(actual, apply(100. / 128., 2.25 * 100. / 128., independent.clone()));
            // Native interpolation's premultiply/unpremultiply may round one F32 ULP.
            assert!(apply(0., 0., decoded.clone()).iter().zip(source.read_region(bounds)).all(|(actual, original)| (actual - original).abs() < 1e-6));
            let mut input = Document::new("Native glass input", Size::new(5, 2), ColorMode::Rgb, depth);
            input.layers.push(Layer::new("Pixels", LayerContent::Raster(source.clone())));
            let bytes = photocraft_format::save_to_bytes(&input, &Default::default()).unwrap();
            let mut smart =
                SmartObject::new(SmartSource::Embedded { file_name: "input.pcraft".into(), bytes: std::sync::Arc::new(bytes) }, Affine::IDENTITY, None);
            smart.smart_filters = mapping.filters.clone();
            input.layers = vec![Layer::new("Editable native refraction", LayerContent::Smart(smart))];
            let bytes = photocraft_format::save_to_bytes(&input, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
            let LayerContent::Smart(restored) = &reopened.layers[0].content else { panic!("native displacement") };
            assert_eq!(restored.smart_filters, mapping.filters);
            assert_eq!(apply(horizontal, vertical, crate::smart_map::displace_embedded_map(&restored.smart_filters[0].params).unwrap()), actual);
        }
        for (width, height) in [(5, 5), (500, 500), (5, 500), (500, 5)] {
            assert!(glass(&StoredEffect::test("gegl:tile-glass", vec![int("tile-width", width), int("tile-height", height)]), &c).is_ok());
        }
        let wide = glass(&StoredEffect::test("gegl:tile-glass", vec![int("tile-width", 500), int("tile-height", 10)]), &c).unwrap();
        assert_ne!(wide.filters[0].params, filter.params);
        for c in [MappingContext { remaining_generation_bytes: 1, ..context() }, MappingContext { remaining_metadata_bytes: 1, ..context() }] {
            assert!(glass(&StoredEffect::test("gegl:tile-glass", vec![]), &c).is_err());
        }
        let cancel = || true;
        assert!(glass(&StoredEffect::test("gegl:tile-glass", vec![]), &MappingContext { interrupt: Interrupt::cancel_only(&cancel), ..context() }).is_err());
    }
    #[test]
    fn generators_are_editable_geometry_with_source_control_counts() {
        let nova = mapped_effect("gegl:supernova", vec![int("spokes-count", 7), int("random-hue", 90)]);
        let Some(super::super::NativeStructure::Replace { layer, .. }) = nova.structure else { panic!("native nova") };
        let LayerContent::Group(g) = layer.content else { panic!("nova group") };
        assert_eq!(g.children.len(), 7);
        assert!(g.children.iter().all(|l| matches!(l.content, LayerContent::Shape(_))));
        let spiral = mapped_effect("gegl:spiral", Vec::new());
        let Some(super::super::NativeStructure::Replace { layer, .. }) = spiral.structure else { panic!("native spiral") };
        let LayerContent::Group(g) = layer.content else { panic!("spiral group") };
        assert_eq!(g.children.len(), 2);
        assert!(matches!(g.children[1].content, LayerContent::Shape(_)));
    }
    #[test]
    fn seamless_conditional_masks_form_convex_opaque_crossfade() {
        let mapping = seamless(&context()).unwrap();
        let Some(super::super::NativeStructure::Composite { branches }) = mapping.structure else { panic!("seamless branches") };
        assert_eq!(branches.len(), 4);
        assert!(branches[0].mask.is_none());
        assert!(branches.iter().all(|b| b.blend == BlendMode::Normal));
        for (x, y) in [(0, 0), (8, 6), (16, 12), (31, 23)] {
            let mb = branches[1].mask.as_ref().unwrap().value(x, y);
            let mc = branches[2].mask.as_ref().unwrap().value(x, y);
            let md = branches[3].mask.as_ref().unwrap().value(x, y);
            let a = (1. - mb) * (1. - mc) * (1. - md);
            let b = mb * (1. - mc) * (1. - md);
            let cc = mc * (1. - md);
            assert!((a + b + cc + md - 1.).abs() < 1e-6);
            assert_eq!(branches[3].filters[0].params["horizontal"], json!(16));
        }
    }
    #[test]
    fn special_outline_bevel_has_separate_native_outline_stage_and_custom_gloss_controls() {
        let mapping = mapped_effect(
            "gegl:styles",
            vec![flag("enablespecialoutline", true), flag("enableoutlinebevel", true), float("bevel-outlow", 0.2), float("bevel-outhigh", 1.2)],
        );
        let Some(super::super::NativeStructure::Composite { branches }) = mapping.structure else { panic!("native outline stage") };
        assert_eq!(branches.len(), 2);
        assert_eq!(branches[1].fill_opacity, 0.);
        assert!(branches[1].layer_effects.iter().any(|fx| matches!(fx, Effect::Stroke(_))));
        let Effect::BevelEmboss(bevel) = &branches[1].post_layer_effects[0] else { panic!("native post-outline bevel") };
        let Contour::Custom { points, .. } = &bevel.gloss_contour else { panic!("native editable gloss curve") };
        assert!((points[0].output - 1. / 6.).abs() < 1e-6);
        assert_eq!(points[1].output, 1.);
        assert!(mapping.notes[0].contains("HDR highlight"));
        let plain = mapped_effect("gegl:styles", vec![flag("enablespecialoutline", true), flag("enableoutlinebevel", false)]);
        assert!(plain.structure.is_none());
        assert!(plain.layer_effects.iter().any(|fx| matches!(fx, Effect::Stroke(_))));
        let mut source = Surface::new(PixelFormat::RGBA32F);
        source.fill_rect(Rect::new(12, 8, 20, 16), &[1., 0., 0., 1.]);
        let mut stroke = Layer::new("editable native source", LayerContent::Raster(source));
        stroke.fill_opacity = 0.;
        stroke.effects.items = branches[1].layer_effects.clone();
        let mut inside = Document::new("native outline", Size::new(32, 24), ColorMode::Rgb, SampleType::F32);
        inside.layers.push(stroke);
        let outline = photocraft_compose::render(&inside, inside.bounds()).px;
        assert_eq!(outline[12 * 32 + 16][3], 0.);
        assert!(outline.iter().any(|p| p[3] > 0.));
        let mut cache = Surface::new(PixelFormat::RGBA32F);
        let data: Vec<f32> = outline.iter().flatten().copied().collect();
        cache.write_region(inside.bounds(), &data);
        let mut outside = Document::new("native bevel on outline", inside.size, inside.mode, inside.depth);
        let mut layer = Layer::new("native outline result", LayerContent::Raster(cache));
        layer.effects.items = branches[1].post_layer_effects.clone();
        outside.layers.push(layer);
        let result = photocraft_compose::render(&outside, outside.bounds()).px;
        assert_eq!(result[12 * 32 + 16][3], 0.);
        assert!(result.iter().any(|p| p[3] > 0.));
        let bytes = photocraft_format::save_to_bytes(&outside, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
        assert_eq!(result, photocraft_compose::render(&reopened, reopened.bounds()).px);
    }
    #[test]
    fn shaped_outline_uses_native_growth_scoped_knockout_and_bevel_before_cutout() {
        let r = Rect::new(0, 0, 32, 24);
        let mut renders = Vec::new();
        for grow_shape in [0, 2] {
            for bevel in [false, true] {
                let mapping = mapped_effect(
                    "gegl:styles",
                    vec![
                        flag("enableoutline", true),
                        flag("enablespecialoutline", bevel),
                        flag("enableoutlinebevel", bevel),
                        float("outline", 2.),
                        Argument { name: "outline-grow-shape", value: ArgumentValue::Enum(grow_shape) },
                        Argument { name: "color-policy", value: ArgumentValue::Enum(0) },
                    ],
                );
                let Some(super::super::NativeStructure::Composite { branches }) = mapping.structure else { panic!("native shaped outline") };
                assert_eq!(branches.len(), 3);
                assert_eq!(branches[1].advanced.knockout, photocraft_doc::Knockout::Shallow);
                assert_eq!(branches[1].fill_opacity, 0.);
                assert_eq!(branches[0].post_layer_effects.len(), usize::from(bevel));
                assert_eq!(branches[0].filters[0].command, if grow_shape == 0 { "filter.other.maximum" } else { "filter.other.percentile" });
                let mut source = Surface::new(PixelFormat::RGBA32F);
                source.write_pixel(16, 12, &[1., 0., 0., 1.]);
                let mut native = Document::new("Isolated native outline", Size::new(32, 24), ColorMode::Rgb, SampleType::F32);
                for branch in branches {
                    assert_eq!(branch.transform, Affine::IDENTITY);
                    let mut filtered = source.clone();
                    for f in branch.filters {
                        let radius = f.params["radius"].as_f64().unwrap() as f32;
                        let params = if f.command == "filter.other.maximum" {
                            photocraft_algo::FilterParams::Maximum { radius, preserve: photocraft_algo::Preserve::Squareness }
                        } else {
                            photocraft_algo::FilterParams::Percentile(photocraft_algo::rank::Params {
                                radius: radius as u16,
                                color_percentile: 100.,
                                alpha_percentile: 100.,
                                shape: photocraft_algo::rank::Neighborhood::Diamond,
                                channels: None,
                            })
                        };
                        filtered = photocraft_algo::apply_in_with(&filtered, &params, r, r, None, r, &photocraft_raster::Interrupt::NONE).unwrap();
                    }
                    let mut layer = Layer::new("Ordinary native branch", LayerContent::Raster(filtered));
                    layer.effects.items = branch.layer_effects;
                    layer.fill_opacity = branch.fill_opacity;
                    if !branch.post_layer_effects.is_empty() {
                        let mut inner = Document::new("Native pre-bevel", Size::new(32, 24), ColorMode::Rgb, SampleType::F32);
                        inner.layers.push(layer);
                        let pixels = photocraft_compose::render(&inner, r).px;
                        let mut cache = Surface::new(PixelFormat::RGBA32F);
                        cache.write_region(r, &pixels.iter().flatten().copied().collect::<Vec<_>>());
                        layer = Layer::new("Native outer bevel", LayerContent::Raster(cache));
                        layer.effects.items = branch.post_layer_effects;
                    }
                    layer.opacity = branch.opacity;
                    layer.advanced = branch.advanced;
                    layer.blend = branch.blend;
                    layer.clipped = branch.clipped;
                    native.layers.push(layer);
                }
                let result = photocraft_compose::render(&native, r).px;
                assert_eq!(result[12 * 32 + 16], [1., 0., 0., 1.]);
                assert!(result[12 * 32 + 18][3] > 0.);
                if !bevel {
                    renders.push(result.clone());
                }
                let mut cache = Surface::new(PixelFormat::RGBA32F);
                cache.write_region(r, &result.iter().flatten().copied().collect::<Vec<_>>());
                let mut background = Surface::new(PixelFormat::RGBA32F);
                background.fill_rect(r, &[0., 0., 1., 1.]);
                let mut outside = Document::new("Native outer scope", Size::new(32, 24), ColorMode::Rgb, SampleType::F32);
                outside.layers.push(Layer::new("Unrelated blue", LayerContent::Raster(background)));
                outside.layers.push(Layer::new("Isolated outline", LayerContent::Raster(cache)));
                let before = photocraft_compose::render(&outside, r).px;
                assert_eq!(before[0], [0., 0., 1., 1.]);
                let bytes = photocraft_format::save_to_bytes(&outside, &Default::default()).unwrap();
                let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
                assert_eq!(before, photocraft_compose::render(&reopened, r).px);
            }
        }
        assert_eq!(renders[0][14 * 32 + 18][3], 1.);
        assert_eq!(renders[1][14 * 32 + 18][3], 0.);
        let zero = mapped_effect(
            "gegl:styles",
            vec![
                flag("enableoutline", true),
                float("outline", 0.),
                float("outline-x", 5.),
                float("outline-blur", 2.),
                Argument { name: "outline-grow-shape", value: ArgumentValue::Enum(0) },
                Argument { name: "color-policy", value: ArgumentValue::Enum(0) },
            ],
        );
        assert!(zero.structure.is_none());
        assert!(zero.layer_effects.is_empty());
    }

    #[test]
    fn inner_glow_treatment_controls_native_range_and_survives_native_reopen() {
        let mut results = Vec::new();
        for treatment in [50., 60., 85.] {
            let mapping = mapped_effect(
                "gegl:styles",
                vec![flag("enableinnerglow", true), float("ig-treatment", treatment), Argument { name: "color-policy", value: ArgumentValue::Enum(0) }],
            );
            let Effect::InnerGlow(glow) = &mapping.layer_effects[0] else { panic!("native glow") };
            assert!((glow.range - treatment / 100.).abs() < 1e-6);
            assert!(matches!(glow.contour, Contour::Linear));
            let mut source = Surface::new(PixelFormat::RGBA32F);
            source.fill_rect(Rect::new(8, 6, 24, 18), &[0.1, 0.2, 0.8, 0.75]);
            let mut layer = Layer::new("Native inner glow", LayerContent::Raster(source));
            layer.effects.items = mapping.layer_effects;
            let mut native = Document::new("Native range", Size::new(32, 24), ColorMode::Rgb, SampleType::F32);
            native.layers.push(layer);
            let before = photocraft_compose::render(&native, native.bounds()).px;
            let saved = photocraft_format::save_to_bytes(&native, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
            assert_eq!(before, photocraft_compose::render(&reopened, reopened.bounds()).px);
            results.push(before);
        }
        assert_ne!(results[0], results[2]);
        for treatment in [49., 86., f32::NAN] {
            assert!(
                map(&StoredEffect::test("gegl:styles", vec![flag("enableinnerglow", true), float("ig-treatment", treatment)]), &context()).unwrap().is_err()
            );
        }
    }

    #[test]
    fn native_style_flags_control_styles_and_resource_locators_stay_inert() {
        let mapping = mapped_effect(
            "gegl:styles",
            vec![flag("enableoutline", true), flag("enableinnerglow", true), flag("enablebevel", true), float("shadow-opacity", 0.5)],
        );
        assert_eq!(mapping.layer_effects.len(), 5);
        assert!(mapping.layer_effects.iter().any(|fx| matches!(fx, Effect::BevelEmboss(_))));
        let source = StoredEffect::test(
            "gegl:styles",
            vec![flag("enableimage", true), Argument { name: "image-src", value: ArgumentValue::String("C:/never-read-this-resource.png") }],
        );
        assert_eq!(map(&source, &context()).unwrap().unwrap_err().reason, photocraft_doc::import_conversion::ConversionReason::ResourceUnavailable);
    }
    #[test]
    fn large_stress_and_random_water_cells_use_bounded_native_structures() {
        let c = MappingContext { native_filter_bounds: Rect::from_xywh(0, 0, 7, 5), ..context() };
        let stress = stress(&StoredEffect::test("gegl:stress", vec![int("radius", 6000)]), &c).unwrap();
        let Some(super::super::NativeStructure::Composite { branches }) = stress.structure else { panic!("native base/detail") };
        assert_eq!(branches.len(), 4);
        let variance: f64 =
            branches[1].filters.iter().filter(|f| f.command == "filter.blur.gaussianBlur").map(|f| f.params["radius"].as_f64().unwrap().powi(2)).sum();
        assert!((variance - 2000f64.powi(2)).abs() < 1e-6);
        let water = water_cells(i32::MAX, 0., 50, 1, &c).unwrap();
        let Some(super::super::NativeStructure::Composite { branches }) = water.structure else { panic!("native cells") };
        assert_eq!(branches.len(), 1);
        let frame = branches[0].filter_frame.unwrap();
        assert_eq!((frame.width(), frame.height()), (1, 1));
        let transform = branches[0].transform.mul(&Affine::translate(f64::from(frame.x0), f64::from(frame.y0)));
        assert_eq!(transform.m, [7., 0., 0., 5., 0., 0.]);
        let average = water_cells(i32::MAX, 0.01, 50, 0, &c).unwrap();
        let Some(super::super::NativeStructure::Composite { branches }) = average.structure else { panic!("native averaged cells") };
        assert_eq!(branches[0].filter_frame, Some(c.native_filter_bounds));
        assert_eq!(branches[0].filters.last().unwrap().command, "filter.blur.average");
        assert!((branches[0].filters[0].opacity - 0.01).abs() < 1e-6);
        let big = MappingContext { native_filter_bounds: Rect::from_xywh(0, 0, 6400, 6400), ..context() };
        assert!(water_cells(301, 0., 0, 1, &big).is_err());
        let tiny = MappingContext { remaining_generation_bytes: 1, ..c };
        assert!(water_cells(301, 0., 0, 1, &tiny).is_err());
    }
    #[test]
    fn invalid_or_budget_hostile_inputs_are_bounded() {
        let mut c = context();
        c.native_filter_bounds = Rect::from_xywh(0, 0, 100_000, 100_000);
        assert!(seamless(&c).is_err());
        for (op, args) in [
            ("gegl:supernova", vec![int("spokes-count", i32::MAX)]),
            ("gegl:vignette", vec![float("radius", f32::MAX)]),
            ("gegl:spiral", vec![float("x", f32::NAN)]),
            ("gegl:value-propagate", vec![float("lower-threshold", 0.2)]),
        ] {
            assert!(map(&StoredEffect::test(op, args), &context()).unwrap().is_err(), "{op}");
        }
    }
    #[test]
    fn propagation_native_modes_preserve_flags_directions_paint_and_named_limits() {
        for (mode, maximum) in [(0, true), (1, false), (5, true), (6, false)] {
            for (value, alpha) in [(true, false), (false, true), (true, true)] {
                let m = mapped_effect(
                    "gegl:value-propagate",
                    vec![Argument { name: "mode", value: ArgumentValue::Enum(mode) }, flag("value", value), flag("alpha", alpha)],
                );
                assert_eq!(m.filters[0].command, "filter.other.percentile");
                assert_eq!(m.filters[0].params["channels"], json!([value, value, value, alpha]));
                assert_eq!(m.filters[0].params["alphaPercentile"], json!(if maximum { 100. } else { 0. }));
            }
        }
        let middle = mapped_effect("gegl:value-propagate", vec![Argument { name: "mode", value: ArgumentValue::String("middle") }, float("rate", 0.5)]);
        let Some(super::super::NativeStructure::Composite { branches }) = middle.structure else { panic!("native middle branches") };
        assert_eq!(branches.len(), 3);
        assert!((branches[1].opacity - 1. / 3.).abs() < 1e-6);
        assert_eq!(branches[2].opacity, 0.25);
        let direction = mapped_effect("gegl:value-propagate", vec![flag("left", false), flag("right", false), flag("bottom", false), flag("alpha", false)]);
        let Some(super::super::NativeStructure::Composite { branches }) = direction.structure else { panic!("native direction") };
        assert_eq!(branches.len(), 2);
        assert_eq!(branches[1].transform, Affine::translate(0., -1.));
        assert_eq!(branches[1].blend, BlendMode::Lighten);
        assert!(branches[1].clipped);
        let middle = mapped_effect(
            "gegl:value-propagate",
            vec![
                Argument { name: "mode", value: ArgumentValue::Enum(2) },
                flag("left", false),
                flag("right", false),
                flag("bottom", false),
                flag("value", false),
            ],
        );
        assert_eq!(middle.filters[0].command, "filter.other.convolution");
        assert_eq!(middle.filters[0].params["channels"], json!([false, false, false, true]));
        assert_eq!(middle.filters[0].params["kernel"][17], json!(1.0));
        assert_eq!(middle.filters[0].params["scale"], json!(2));
        let peak = mapped_effect("gegl:value-propagate", vec![Argument { name: "mode", value: ArgumentValue::Enum(3) }, flag("alpha", false)]);
        let Some(super::super::NativeStructure::Composite { branches }) = peak.structure else { panic!("native colored edges") };
        assert_eq!(branches[1].filters[0].command, "filter.stylize.findEdges");
        assert_eq!(branches[1].filters[1].command, "filter.other.colorToAlpha");
        assert!(branches[1].clipped);
        let Effect::ColorOverlay { color, .. } = &branches[1].layer_effects[0] else { panic!("native replacement paint") };
        for (actual, expected) in color.to_rgb().into_iter().zip([0., 0., 1.]) {
            assert!((actual - expected).abs() < 1e-6);
        }
        for args in [
            vec![float("lower-threshold", 0.2)],
            vec![Argument { name: "mode", value: ArgumentValue::Enum(4) }],
            vec![flag("top", false), flag("value", false)],
            vec![Argument { name: "mode", value: ArgumentValue::Enum(3) }, flag("value", false)],
        ] {
            let effect = StoredEffect::test("gegl:value-propagate", args);
            let failure = map(&effect, &context()).unwrap().unwrap_err();
            assert_eq!(failure.reason, photocraft_doc::import_conversion::ConversionReason::NativeLimitation);
            assert!(!failure.message.is_empty());
        }
        assert!(
            map(&StoredEffect::test("gegl:value-propagate", vec![Argument { name: "mode", value: ArgumentValue::String("unknown") }]), &context())
                .unwrap()
                .is_err()
        );
    }
    #[test]
    fn key_growth_inverse_counterexample_retains_source_pixels_and_report_after_reopen() {
        use crate::xcf::tests::{W, hierarchy};
        use photocraft_doc::import_conversion::{ConversionDisposition, ConversionReason, ImportConversion};
        use photocraft_raster::Interrupt;
        let r = Rect::new(0, 0, 5, 1);
        let bytes = [0, 0, 255, 0, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 0, 0, 255, 255];
        let mut input = Surface::new(PixelFormat::RGBA32F);
        input.write_region(r, &bytes.iter().map(|v| f32::from(*v) / 255.).collect::<Vec<_>>());
        let removed = photocraft_algo::apply_in_with(
            &input,
            &photocraft_algo::FilterParams::ColorToAlpha { color: [0., 0., 1., 1.], transparency_threshold: 0., opacity_threshold: 1. },
            r,
            r,
            None,
            r,
            &Interrupt::NONE,
        )
        .unwrap();
        assert_eq!(removed.pixel(0, 0)[3], 0.);
        assert_eq!(removed.pixel(4, 0)[3], 0.);
        let inverted = photocraft_algo::coverage::apply_with(
            &removed,
            photocraft_algo::coverage::Operation::AlphaCurve(&(0..256).map(|i| 1. - i as f32 / 255.).collect::<Vec<_>>()),
            r,
            r,
            None,
            &Interrupt::NONE,
        )
        .unwrap();
        let grown = photocraft_algo::apply_in_with(
            &inverted,
            &photocraft_algo::FilterParams::Maximum { radius: 1., preserve: photocraft_algo::Preserve::Squareness },
            r,
            r,
            None,
            r,
            &Interrupt::NONE,
        )
        .unwrap();
        assert_eq!(grown.pixel(1, 0)[3], 1.);
        assert_eq!(input.pixel(1, 0), vec![1., 1., 1., 1.]);
        let mut w = W::new(26);
        for value in [5, 1, 0, 150] {
            w.u32(value);
        }
        w.prop(17, &[0]);
        w.end();
        let layer = w.slot();
        w.ptr(0);
        w.ptr(0);
        w.ptr(0);
        let at = w.out.len();
        w.patch(layer, at);
        for value in [5, 1, 1] {
            w.u32(value);
        }
        w.string("Original key source");
        w.end();
        let pixels = w.slot();
        w.ptr(0);
        let effect = w.slot();
        w.ptr(0);
        let at = hierarchy(&mut w, 5, 1, 4, &bytes, crate::xcf::Compression::None);
        w.patch(pixels, at);
        let at = w.out.len();
        w.patch(effect, at);
        w.string("Original key propagation");
        w.string("");
        w.string("gegl:value-propagate");
        w.string("0:0");
        let mut argument = W { out: Vec::new(), wide: true };
        argument.string("mode");
        argument.u32(5);
        argument.u32(4);
        w.prop(45, &argument.out);
        w.end();
        w.ptr(0);
        let imported = crate::xcf::import("key-limit.xcf", &w.out, &Interrupt::NONE).unwrap().document;
        assert_eq!(imported.layers[0].surface().unwrap().read_region(r), input.read_region(r));
        let ImportConversion::Report(report) = imported.metadata.import_conversion.as_ref().unwrap() else { panic!("native report") };
        let entry = report.entries.iter().find(|entry| entry.feature_code == "xcf.layer_effect").unwrap();
        assert_eq!(entry.disposition, ConversionDisposition::RetainedUnapplied);
        assert_eq!(entry.reason, ConversionReason::NativeLimitation);
        assert!(entry.native_description.contains("transparent/hidden-pixel key hits"));
        assert!(
            report.attachments.iter().any(|attachment| attachment.bytes.windows(b"gegl:value-propagate".len()).any(|window| window == b"gegl:value-propagate"))
        );
        let saved = photocraft_format::save_to_bytes(&imported, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
        assert_eq!(reopened.layers[0].surface().unwrap().read_region(r), input.read_region(r));
        assert_eq!(reopened.metadata.import_conversion, imported.metadata.import_conversion);
    }
    #[test]
    fn directional_native_layer_blends_do_not_select_alpha_extrema_or_preserve_stored_color() {
        for blend in [BlendMode::Normal, BlendMode::Lighten, BlendMode::Darken] {
            let mut doc = Document::new("native alpha-over counterexample", Size::new(1, 1), ColorMode::Rgb, SampleType::F32);
            let mut center = Surface::new(doc.pixel_format());
            center.fill_rect(doc.bounds(), &[1., 0., 0., 0.25]);
            let mut neighbor = Surface::new(doc.pixel_format());
            neighbor.fill_rect(doc.bounds(), &[0., 0., 1., 0.5]);
            let base = Layer::new("original center", LayerContent::Raster(center));
            let mut over = Layer::new("translated native neighbor", LayerContent::Raster(neighbor));
            over.blend = blend;
            doc.layers = vec![base, over];
            let actual = photocraft_compose::render(&doc, doc.bounds()).px[0];
            // Native layer opacity accumulates coverage; a direction-selected extrema
            // operation would choose either .25 or .5, and alpha-only retains red.
            assert!((actual[3] - 0.625).abs() < 1e-6, "{blend:?}");
            assert!(actual[2] > 0., "{blend:?}");
        }
    }
    #[test]
    fn propagation_relative_and_directional_alpha_limits_retain_exact_source_and_reopen() {
        use crate::xcf::tests::{W, hierarchy};
        use photocraft_doc::import_conversion::{ConversionDisposition, ConversionReason, ImportConversion};
        use photocraft_raster::Interrupt;
        let bounds = Rect::new(0, 0, 3, 1);
        // Hidden key-colored data, partial coverage and opaque non-key data must survive.
        let pixels = [0, 0, 255, 0, 32, 128, 224, 96, 240, 64, 16, 255];
        let cases = [
            ("relative threshold", vec![("lower-threshold", 3, 0.2f32.to_bits())], "neighbor-minus-center"),
            ("directional opaque", vec![("mode", 5, 5), ("value", 2, 0), ("left", 2, 0)], "alpha-only neighbor"),
            ("directional transparent", vec![("mode", 5, 6), ("value", 2, 0), ("top", 2, 0)], "alpha-only neighbor"),
        ];
        for (name, arguments, description) in cases {
            let mut w = W::new(26);
            for value in [3, 1, 0, 150] {
                w.u32(value);
            }
            w.prop(17, &[0]);
            w.end();
            let layer = w.slot();
            w.ptr(0);
            w.ptr(0);
            w.ptr(0);
            let at = w.out.len();
            w.patch(layer, at);
            for value in [3, 1, 1] {
                w.u32(value);
            }
            w.string(name);
            w.end();
            let image = w.slot();
            w.ptr(0);
            let effect = w.slot();
            w.ptr(0);
            let at = hierarchy(&mut w, 3, 1, 4, &pixels, crate::xcf::Compression::None);
            w.patch(image, at);
            let at = w.out.len();
            w.patch(effect, at);
            w.string(name);
            w.string("");
            w.string("gegl:value-propagate");
            w.string("0:0");
            let mut exact_arguments = Vec::new();
            for (key, kind, value) in arguments {
                let mut argument = W { out: Vec::new(), wide: true };
                argument.string(key);
                argument.u32(kind);
                argument.u32(value);
                exact_arguments.push(argument.out.clone());
                w.prop(45, &argument.out);
            }
            w.end();
            w.ptr(0);
            let imported = crate::xcf::import(name, &w.out, &Interrupt::NONE).unwrap().document;
            let original = imported.layers[0].surface().unwrap().read_region(bounds);
            for (actual, expected) in original.iter().zip(pixels) {
                assert!((*actual - f32::from(expected) / 255.).abs() < 1e-6, "{name}");
            }
            let ImportConversion::Report(report) = imported.metadata.import_conversion.as_ref().unwrap() else { panic!("report") };
            let entry = report.entries.iter().find(|entry| entry.feature_code == "xcf.layer_effect").unwrap();
            assert_eq!(entry.disposition, ConversionDisposition::RetainedUnapplied, "{name}");
            assert_eq!(entry.reason, ConversionReason::NativeLimitation, "{name}");
            assert!(entry.native_description.contains(description), "{name}: {}", entry.native_description);
            for raw in exact_arguments {
                assert!(report.attachments.iter().any(|a| a.bytes.windows(raw.len()).any(|window| window == raw)), "{name}: exact original argument");
            }
            let saved = photocraft_format::save_to_bytes(&imported, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
            assert_eq!(reopened.layers[0].surface().unwrap().read_region(bounds), original, "{name}");
            assert_eq!(reopened.metadata.import_conversion, imported.metadata.import_conversion, "{name}");
        }
    }
    #[test]
    fn actual_remaining_budget_and_midrow_cancellation_precede_native_generation() {
        let tiny = MappingContext { remaining_generation_bytes: 128, ..context() };
        for operation in ["gegl:tile-seamless", "gegl:video-degradation", "gegl:supernova", "gegl:spiral", "gegl:vignette"] {
            assert!(map(&StoredEffect::test(operation, Vec::new()), &tiny).unwrap().is_err(), "{operation}");
        }
        use std::sync::atomic::{AtomicUsize, Ordering};
        let checks = AtomicUsize::new(0);
        let cancel = || checks.fetch_add(1, Ordering::Relaxed) >= 3;
        let interrupted = MappingContext { interrupt: photocraft_raster::Interrupt::cancel_only(&cancel), ..context() };
        assert!(seamless(&interrupted).is_err());
        assert!(checks.load(Ordering::Relaxed) >= 4);
    }
}
