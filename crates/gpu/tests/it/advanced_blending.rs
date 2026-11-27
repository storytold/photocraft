//! Advanced Blending on the GPU: shallow and deep knockouts (top level, isolated and pass-through
//! groups, clipping groups), Blend Clipped Layers as Group off, Transparency Shapes Layer off and
//! Layer / Vector Mask Hides Effects (below the knockout tests) must match the CPU compositor
//! within the parity tolerance, and the document must render on the GPU (a refusal fails the
//! check).

use photocraft_color::{BlendMode, Color, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Adjustment, BlendRange, Document, Effect, Fill, GradientStyle, Knockout, Layer, LayerContent, LayerMask, Path, Subpath, VectorMask};
use photocraft_geom::{Rect, Size};
use photocraft_raster::Surface;

use crate::parity::{Gpu, base_doc, check, check_with, checker_pattern, diff_rect, gpu, half_float_compositor, mask, noise_layer};

const KNOCKOUTS: [Knockout; 2] = [Knockout::Shallow, Knockout::Deep];

/// A 48 × 40 RGB document; with `background`, its bottom layer is a Background (opaque noise
/// with transparency and position locked).
fn doc(depth: SampleType, background: bool) -> Document {
    let mut d = Document::new("knockout", Size::new(48, 40), ColorMode::Rgb, depth);
    if background {
        let mut bg = noise_layer("Background", d.pixel_format(), d.bounds(), 1, 1.0);
        bg.locks.transparency = true;
        bg.locks.position = true;
        d.layers.push(bg);
    }
    d
}

fn underlay(d: &Document, seed: u32) -> Layer {
    noise_layer("under", d.pixel_format(), d.bounds(), seed, 0.4)
}

fn ko_layer(d: &Document, seed: u32, knockout: Knockout) -> Layer {
    let mut l = noise_layer("knockout", d.pixel_format(), Rect::new(6, 5, 42, 35), seed, 0.3);
    l.advanced.knockout = knockout;
    l
}

fn group(name: &str, pass_through: bool, children: Vec<Layer>) -> Layer {
    let mut g = Layer::group(name, children);
    g.blend = if pass_through { BlendMode::PassThrough } else { BlendMode::Normal };
    g
}

/// `check` that also names the case.
fn run(g: &mut Gpu, d: &Document, what: &str, knockout: Knockout) {
    check(g, d, &format!("{what}, {knockout:?}"));
}

/// A clipping group on `base` with Blend Clipped Layers as Group off: a Normal clipped layer at
/// opacity 0.7 and a Multiply one above it.
fn clipped_pair(d: &Document, mut base: Layer, base_opacity: f32) -> Vec<Layer> {
    base.advanced.blend_clipped = false;
    base.opacity = base_opacity;
    let mut c1 = noise_layer("clipped 1", d.pixel_format(), Rect::new(8, 8, 44, 34), 7, 0.5);
    c1.clipped = true;
    c1.opacity = 0.7;
    let mut c2 = noise_layer("clipped 2", d.pixel_format(), Rect::new(2, 10, 38, 30), 8, 0.4);
    c2.clipped = true;
    c2.blend = BlendMode::Multiply;
    vec![base, c1, c2]
}

fn clip_base(d: &Document) -> Layer {
    noise_layer("base", d.pixel_format(), Rect::new(4, 4, 40, 34), 6, 0.5)
}

#[test]
fn top_level_knockout_matches_the_cpu() {
    let Some(mut g) = gpu() else { return };
    for background in [false, true] {
        for knockout in KNOCKOUTS {
            for (opacity, fill) in [(1.0, 1.0), (0.5, 1.0), (1.0, 0.5), (0.5, 0.5)] {
                let mut d = doc(SampleType::U8, background);
                d.layers.push(underlay(&d, 2));
                let mut l = ko_layer(&d, 3, knockout);
                l.opacity = opacity;
                l.fill_opacity = fill;
                d.layers.push(l);
                d.layers.push(noise_layer("above", d.pixel_format(), Rect::new(10, 8, 30, 28), 4, 0.5));
                run(&mut g, &d, &format!("background {background}, opacity {opacity}, fill {fill}"), knockout);
            }
        }
    }
}

/// Outside the shape of a partial-opacity knockout the drawn pixel is untouched, so 32-bit values
/// above 1 survive as on the CPU.
#[test]
fn partial_opacity_knockout_keeps_hdr_pixels_outside_its_shape() {
    let Some(mut g) = gpu() else { return };
    for knockout in KNOCKOUTS {
        let mut d = doc(SampleType::F32, false);
        let mut bottom = Layer::raster("hdr", d.pixel_format());
        let patterns = [[2.0, 1.5, 0.25, 1.0], [1.25, 3.0, 0.5, 1.0], [0.4, 0.6, 0.8, 1.0]];
        let px: Vec<f32> = (0..48 * 40).flat_map(|i| patterns[i % patterns.len()]).collect();
        bottom.surface_mut().unwrap().write_region(Rect::from_xywh(0, 0, 48, 40), &px);
        d.layers.push(bottom);
        let mut l = ko_layer(&d, 3, knockout);
        l.opacity = 0.5;
        d.layers.push(l);
        let cpu = photocraft_compose::render(&d, d.bounds());
        assert!(cpu.px[0][..3].iter().any(|&v| v > 1.0), "the CPU keeps a value above 1 outside the shape ({knockout:?})");
        run(&mut g, &d, "HDR outside a partial-opacity knockout", knockout);
    }
}

#[test]
fn knockout_in_every_depth() {
    let Some(mut g) = gpu() else { return };
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for knockout in KNOCKOUTS {
            let mut d = doc(depth, true);
            d.layers.push(underlay(&d, 2));
            let mut l = ko_layer(&d, 3, knockout);
            l.opacity = 0.5;
            d.layers.push(l);
            run(&mut g, &d, &format!("{depth:?}"), knockout);
        }
    }
}

#[test]
fn knockout_in_groups_matches_the_cpu() {
    let Some(mut g) = gpu() else { return };
    for background in [false, true] {
        for knockout in KNOCKOUTS {
            for opacity in [1.0, 0.5] {
                // Isolated, pass-through, and each nested in the other.
                for (name, outer_pass, inner_pass) in [
                    ("isolated", false, None),
                    ("pass-through", true, None),
                    ("pass-through in isolated", false, Some(true)),
                    ("isolated in pass-through", true, Some(false)),
                ] {
                    let mut d = doc(SampleType::U8, background);
                    d.layers.push(underlay(&d, 2));
                    let mut l = ko_layer(&d, 3, knockout);
                    l.opacity = opacity;
                    let mut children = vec![noise_layer("inner", d.pixel_format(), Rect::new(0, 0, 36, 30), 5, 0.4), l];
                    if let Some(pass) = inner_pass {
                        children = vec![group("inner group", pass, children)];
                    }
                    d.layers.push(group("group", outer_pass, children));
                    run(&mut g, &d, &format!("{name}, background {background}, opacity {opacity}"), knockout);
                }
            }
        }
    }
}

#[test]
fn knockout_in_a_partial_rectangle_matches_the_cpu() {
    let Some(mut g) = gpu() else { return };
    for knockout in KNOCKOUTS {
        let mut d = doc(SampleType::U8, true);
        d.layers.push(underlay(&d, 2));
        let mut l = ko_layer(&d, 3, knockout);
        l.opacity = 0.6;
        d.layers.push(group("group", true, vec![noise_layer("inner", d.pixel_format(), d.bounds(), 5, 0.4), l]));
        diff_rect(&mut g, &d, Rect::new(10, 8, 40, 30), &format!("partial rectangle, {knockout:?}")).unwrap_or_else(|e| panic!("{e}"));
    }
}

#[test]
fn clipped_knockout_matches_the_cpu() {
    let Some(mut g) = gpu() else { return };
    for knockout in KNOCKOUTS {
        for opacity in [1.0, 0.5] {
            let build = |base_group: Option<bool>| {
                let mut d = doc(SampleType::U8, true);
                d.layers.push(underlay(&d, 2));
                let base = noise_layer("base", d.pixel_format(), Rect::new(4, 4, 40, 34), 6, 0.5);
                let mut c = ko_layer(&d, 7, knockout);
                c.clipped = true;
                c.opacity = opacity;
                let mut c2 = noise_layer("second clipped", d.pixel_format(), Rect::new(8, 8, 38, 30), 8, 0.4);
                c2.clipped = true;
                c2.blend = BlendMode::Multiply;
                match base_group {
                    None => d.layers.extend([base, c, c2]),
                    Some(pass) => {
                        let inner = group("clip base", pass, vec![base]);
                        d.layers.extend([inner, c, c2]);
                    }
                }
                d
            };
            run(&mut g, &build(None), &format!("clipped to a pixel layer, opacity {opacity}"), knockout);
            // A group as the clipping base, pass-through (its isolated rendering carries the clip)
            // and isolated.
            for pass in [true, false] {
                run(&mut g, &build(Some(pass)), &format!("clipped to a group (pass-through {pass}), opacity {opacity}"), knockout);
            }
        }
    }
}

#[test]
fn clipped_knockout_inside_a_group_matches_the_cpu() {
    let Some(mut g) = gpu() else { return };
    for knockout in KNOCKOUTS {
        for pass in [false, true] {
            let mut d = doc(SampleType::U8, true);
            d.layers.push(underlay(&d, 2));
            let base = noise_layer("base", d.pixel_format(), Rect::new(4, 4, 40, 34), 6, 0.5);
            let mut c = ko_layer(&d, 7, knockout);
            c.clipped = true;
            c.opacity = 0.7;
            d.layers.push(group("group", pass, vec![base, c]));
            run(&mut g, &d, &format!("clipped inside a group (pass-through {pass})"), knockout);
        }
    }
}

#[test]
fn knockout_with_masks_and_transparency_shapes_matches_the_cpu() {
    let Some(mut g) = gpu() else { return };
    for knockout in KNOCKOUTS {
        for (masked, shapes, opacity) in
            [(true, true, 1.0), (true, true, 0.5), (false, false, 1.0), (false, false, 0.5), (true, false, 1.0), (true, false, 0.5)]
        {
            let mut d = doc(SampleType::U8, true);
            d.layers.push(underlay(&d, 2));
            let mut l = ko_layer(&d, 3, knockout);
            if masked {
                l.mask = Some(mask(Rect::new(0, 0, 48, 40), 9, 0.4));
            }
            l.advanced.transparency_shapes = shapes;
            l.opacity = opacity;
            l.fill_opacity = 0.8;
            d.layers.push(l);
            run(&mut g, &d, &format!("mask {masked}, transparency shapes {shapes}, opacity {opacity}"), knockout);
        }
    }
}

#[test]
fn knockout_adjustment_layer_uses_its_masks_as_the_shape() {
    let Some(mut g) = gpu() else { return };
    for knockout in KNOCKOUTS {
        for opacity in [1.0, 0.5] {
            let mut d = doc(SampleType::U8, true);
            d.layers.push(underlay(&d, 2));
            let mut adj = Layer::new("invert", LayerContent::Adjustment(Adjustment::Invert));
            adj.mask = Some(mask(Rect::new(0, 0, 48, 40), 9, 0.2));
            adj.advanced.knockout = knockout;
            adj.opacity = opacity;
            d.layers.push(adj);
            run(&mut g, &d, &format!("adjustment, opacity {opacity}"), knockout);
        }
    }
}

/// A Background under an opaque fill layer is left out of the plan, but a deep knockout above
/// the fill still reveals it.
#[test]
fn deep_knockout_reaches_a_background_hidden_by_occlusion() {
    let Some(mut g) = gpu() else { return };
    for knockout in KNOCKOUTS {
        let mut d = doc(SampleType::U8, true);
        d.layers.push(Layer::new("cover", LayerContent::Fill(Fill::Solid(Color::rgba(0.2, 0.7, 0.4, 1.0)))));
        let mut l = ko_layer(&d, 3, knockout);
        l.opacity = 0.6;
        d.layers.push(l);
        run(&mut g, &d, "background under an opaque fill", knockout);
        // And inside a pass-through group, which lets the knockout through to it.
        let mut d2 = doc(SampleType::U8, true);
        d2.layers.push(Layer::new("cover", LayerContent::Fill(Fill::Solid(Color::rgba(0.2, 0.7, 0.4, 1.0)))));
        let l = ko_layer(&d2, 3, knockout);
        d2.layers.push(group("group", true, vec![noise_layer("inner", d2.pixel_format(), d2.bounds(), 5, 0.4), l]));
        run(&mut g, &d2, "background under an opaque fill, pass-through group", knockout);
    }
}

#[test]
fn knockout_with_blend_if_and_channel_restrictions_matches_the_cpu() {
    let Some(mut g) = gpu() else { return };
    for knockout in KNOCKOUTS {
        for opacity in [1.0, 0.5] {
            let mut blend_if = doc(SampleType::U8, true);
            blend_if.layers.push(underlay(&blend_if, 2));
            let mut l = ko_layer(&blend_if, 3, knockout);
            l.opacity = opacity;
            l.blend_if.set(0, [BlendRange { black: [60, 120], white: [255, 255] }, BlendRange::FULL]);
            blend_if.layers.push(l);
            run(&mut g, &blend_if, &format!("Blend If, opacity {opacity}"), knockout);

            let mut channels = doc(SampleType::U8, true);
            channels.layers.push(underlay(&channels, 2));
            let mut l = ko_layer(&channels, 3, knockout);
            l.opacity = opacity;
            l.excluded_channels = 0b010;
            channels.layers.push(l);
            run(&mut g, &channels, &format!("channel restrictions, opacity {opacity}"), knockout);
        }
    }
}

#[test]
fn knockout_layer_with_a_drop_shadow_matches_the_cpu() {
    let Some(mut g) = gpu() else { return };
    for knockout in KNOCKOUTS {
        for opacity in [1.0, 0.5] {
            let mut d = doc(SampleType::U8, true);
            d.layers.push(underlay(&d, 2));
            let mut l = ko_layer(&d, 3, knockout);
            l.opacity = opacity;
            let Effect::DropShadow(mut shadow) = Effect::default_drop_shadow() else { unreachable!() };
            shadow.distance = 4.0;
            shadow.size = 3.0;
            l.effects.items.push(Effect::DropShadow(shadow));
            d.layers.push(l);
            run(&mut g, &d, &format!("drop shadow, opacity {opacity}"), knockout);
        }
    }
}

/// The resident cache keeps a stroked shape's fill apart from its full content: a knockout shape
/// (the full content) and the split fill under a visible clipped layer live in one plan.
#[test]
fn knockout_on_a_stroked_shape_with_a_clipped_layer_matches_the_cpu() {
    let Some(mut g) = gpu() else { return };
    use photocraft_doc::vector::{Path, ShapeLayer, ShapeStroke, Subpath};
    for knockout in KNOCKOUTS {
        for fill in [0.0, 0.5] {
            let mut d = doc(SampleType::U8, true);
            d.layers.push(underlay(&d, 2));
            let path = Path::new(vec![Subpath::polygon(&[(8.3, 6.6), (40.2, 9.1), (36.7, 33.4), (11.9, 30.2)])]);
            let stroke = ShapeStroke { width: 4.0, paint: Fill::Solid(Color::rgb(0.9, 0.9, 0.1)), ..ShapeStroke::default() };
            let mut sh = ShapeLayer { path, fill: Some(Fill::Solid(Color::rgb(0.2, 0.3, 0.8))), stroke: Some(stroke), ..ShapeLayer::default() };
            sh.cache = Some(photocraft_vector::render_shape(&sh, d.pixel_format(), d.bounds()));
            let mut l = Layer::new("shape", LayerContent::Shape(sh));
            l.advanced.knockout = knockout;
            l.fill_opacity = fill;
            let mut c = noise_layer("clip", d.pixel_format(), Rect::new(4, 4, 44, 36), 42, 0.5);
            c.clipped = true;
            d.layers.extend([l, c]);
            run(&mut g, &d, &format!("stroked shape, fill opacity {fill}, clipped layer"), knockout);
        }
    }
}

/// Interior effects (and one stroke) of every kind Blend Interior Effects as Group groups under
/// Fill, plus a set mixing them with exterior effects.
fn interior_effects() -> Vec<(&'static str, Vec<Effect>)> {
    use photocraft_doc::{Contour, FxCommon, FxPaint, Glow, GlowSource, GlowTechnique, Gradient, GradientStyle, Satin, Shadow, StrokeFx, StrokePosition};
    let overlay = |blend, opacity| Effect::ColorOverlay { common: FxCommon::new(blend, opacity), color: Color::rgb(0.9, 0.2, 0.3) };
    let gradient = Effect::GradientOverlay {
        common: FxCommon::new(BlendMode::Normal, 0.9),
        gradient: Gradient {
            stops: vec![(0.0, Color::rgb(1.0, 0.2, 0.0)), (0.5, Color::rgb(0.1, 0.9, 0.3)), (1.0, Color::rgb(0.2, 0.1, 1.0))],
            opacity_stops: vec![(0.0, 1.0), (1.0, 0.4)],
            style: GradientStyle::Linear,
            angle: 30.0,
            scale: 0.9,
            reverse: false,
            align: true,
            offset: (0.1, -0.05),
        },
        dither: false,
        interpolation: None,
    };
    let satin = Effect::Satin(Satin {
        common: FxCommon::new(BlendMode::Multiply, 0.7),
        color: Color::rgb(0.9, 0.4, 0.1),
        angle: 60.0,
        distance: 5.0,
        size: 6.0,
        contour: Contour::Linear,
        anti_alias: false,
        invert: false,
    });
    let glow = |blend| Glow {
        common: FxCommon::new(blend, 0.8),
        paint: FxPaint::Color(Color::rgb(1.0, 0.9, 0.2)),
        technique: GlowTechnique::Softer,
        spread: 0.1,
        size: 5.0,
        contour: Contour::Linear,
        anti_alias: false,
        range: 0.5,
        jitter: 0.0,
        noise: 0.0,
        source: GlowSource::Edge,
    };
    let inner_shadow = Effect::InnerShadow(Shadow {
        common: FxCommon::new(BlendMode::Multiply, 0.8),
        color: Color::rgb(0.1, 0.05, 0.3),
        angle: 120.0,
        use_global_light: false,
        distance: 4.0,
        spread: 0.0,
        size: 6.0,
        contour: Contour::Linear,
        anti_alias: false,
        noise: 0.0,
        knocks_out: true,
    });
    let stroke = Effect::Stroke(StrokeFx {
        common: FxCommon::new(BlendMode::Normal, 0.9),
        size: 3.0,
        position: StrokePosition::Inside,
        paint: FxPaint::Color(Color::rgb(0.1, 0.7, 0.3)),
        overprint: false,
    });
    let Effect::DropShadow(mut drop) = Effect::default_drop_shadow() else { unreachable!() };
    drop.distance = 4.0;
    drop.size = 3.0;
    vec![
        ("color overlay", vec![overlay(BlendMode::Normal, 0.7)]),
        ("soft light overlay", vec![overlay(BlendMode::SoftLight, 0.8)]),
        ("gradient overlay", vec![gradient.clone()]),
        ("satin", vec![satin.clone()]),
        ("inner glow", vec![Effect::InnerGlow(glow(BlendMode::Screen))]),
        ("inner shadow", vec![inner_shadow.clone()]),
        ("inside stroke", vec![stroke.clone()]),
        (
            "several with a drop shadow and an outer glow",
            vec![
                overlay(BlendMode::Overlay, 0.6),
                gradient,
                satin,
                Effect::InnerGlow(glow(BlendMode::Screen)),
                inner_shadow,
                stroke,
                Effect::DropShadow(drop),
                Effect::OuterGlow(glow(BlendMode::Screen)),
            ],
        ),
    ]
}

/// A layer with `items` under Blend Interior Effects as Group.
fn interior_layer(d: &Document, items: &[Effect], blend: BlendMode, opacity: f32, fill: f32) -> Layer {
    let mut l = noise_layer("interior", d.pixel_format(), Rect::new(8, 7, 40, 33), 3, 0.3);
    l.effects.items.extend(items.iter().cloned());
    l.blend = blend;
    l.opacity = opacity;
    l.fill_opacity = fill;
    l.advanced.blend_interior = true;
    l
}

/// Blend Interior Effects as Group applies Fill to the layer together with its interior effects
/// (after them, before the inner shadow) instead of to the layer alone.
#[test]
fn interior_effects_as_group_match_the_cpu() {
    let Some(mut g) = gpu() else { return };
    let modes = [BlendMode::Normal, BlendMode::Multiply, BlendMode::Color];
    for (name, items) in interior_effects() {
        // Every Fill and Opacity in Normal; the other modes at one combination.
        let mut cases: Vec<(BlendMode, f32, f32)> = [(1.0, 0.5), (1.0, 0.0), (0.6, 0.5), (0.6, 0.0)].iter().map(|&(o, f)| (BlendMode::Normal, o, f)).collect();
        cases.extend(modes[1..].iter().map(|&m| (m, 0.6, 0.5)));
        cases.push((BlendMode::Normal, 1.0, 1.0));
        for (blend, opacity, fill) in cases {
            let mut d = doc(SampleType::U8, true);
            d.layers.push(underlay(&d, 2));
            d.layers.push(interior_layer(&d, &items, blend, opacity, fill));
            check(&mut g, &d, &format!("{name}, {blend:?}, opacity {opacity}, fill {fill}"));
        }
    }
}

/// The CPU composites each individually blended clipped layer as an unclipped layer, so
/// Transparency Shapes Layer off on it (the special eight's blend, the shape effects take) runs on
/// the GPU too.
#[test]
fn individually_blended_clipped_layers_without_transparency_shapes_match_the_cpu() {
    let Some(mut g) = gpu() else { return };
    for (blend, fill, fx) in [(BlendMode::ColorDodge, 0.6, false), (BlendMode::Normal, 1.0, true)] {
        let mut d = doc(SampleType::U8, true);
        d.layers.push(underlay(&d, 2));
        let mut layers = clipped_pair(&d, clip_base(&d), 1.0);
        let mut c = layers.remove(1);
        c.blend = blend;
        c.fill_opacity = fill;
        c.advanced.transparency_shapes = false;
        if fx {
            let Effect::DropShadow(mut shadow) = Effect::default_drop_shadow() else { unreachable!() };
            shadow.distance = 4.0;
            shadow.size = 3.0;
            c.effects.items.push(Effect::DropShadow(shadow));
        }
        layers.insert(1, c);
        d.layers.extend(layers);
        check(&mut g, &d, &format!("individually clipped, Transparency Shapes Layer off, {blend:?}, fill {fill}, effects {fx}"));
    }
}

#[test]
fn interior_effects_as_group_in_every_depth() {
    let Some(mut g) = gpu() else { return };
    let effects = interior_effects();
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for (name, items) in [&effects[1], &effects[7]] {
            let mut d = doc(depth, true);
            d.layers.push(underlay(&d, 2));
            d.layers.push(interior_layer(&d, items, BlendMode::Normal, 0.9, 0.5));
            check(&mut g, &d, &format!("{name}, {depth:?}"));
        }
    }
}

#[test]
fn interior_effects_as_group_combined_with_other_options_match_the_cpu() {
    let Some(mut g) = gpu() else { return };
    let effects = interior_effects();
    let items = &effects[7].1;

    // Transparency Shapes Layer off: the effects take the whole layer as their shape.
    let mut d = doc(SampleType::U8, true);
    d.layers.push(underlay(&d, 2));
    let mut l = interior_layer(&d, items, BlendMode::Normal, 0.8, 0.5);
    l.advanced.transparency_shapes = false;
    d.layers.push(l);
    check(&mut g, &d, "Transparency Shapes Layer off");

    // A knockout.
    for knockout in KNOCKOUTS {
        let mut d = doc(SampleType::U8, true);
        d.layers.push(underlay(&d, 2));
        let mut l = ko_layer(&d, 3, knockout);
        l.effects.items.extend(items.iter().cloned());
        l.fill_opacity = 0.5;
        l.opacity = 0.8;
        l.advanced.blend_interior = true;
        d.layers.push(l);
        run(&mut g, &d, "interior group with a knockout", knockout);
    }

    // Masks hiding the effects.
    let mut d = doc(SampleType::U8, true);
    d.layers.push(underlay(&d, 2));
    let mut l = interior_layer(&d, items, BlendMode::Normal, 0.8, 0.5);
    hide_with(&mut l, Hides::Both);
    d.layers.push(l);
    check(&mut g, &d, "masks hiding the effects");

    // Inside isolated and pass-through groups.
    for pass in [false, true] {
        let mut d = doc(SampleType::U8, true);
        d.layers.push(underlay(&d, 2));
        let below = noise_layer("below", d.pixel_format(), Rect::new(0, 0, 30, 30), 6, 0.5);
        let inner = interior_layer(&d, items, BlendMode::Normal, 0.9, 0.5);
        d.layers.push(group("group", pass, vec![below, inner]));
        check(&mut g, &d, &format!("inside a group, pass-through {pass}"));
    }

    // A clipping base with a clipped layer.
    let mut d = doc(SampleType::U8, true);
    d.layers.push(underlay(&d, 2));
    d.layers.push(interior_layer(&d, items, BlendMode::Normal, 0.8, 0.5));
    let mut c = noise_layer("clipped", d.pixel_format(), Rect::new(10, 9, 36, 30), 7, 0.4);
    c.clipped = true;
    c.opacity = 0.7;
    d.layers.push(c);
    check(&mut g, &d, "clipping base");
}

/// Transparency Shapes Layer off on a clipped layer still makes the GPU refuse the document.
#[test]
fn the_remaining_advanced_blending_options_are_refused() {
    let Some(mut g) = gpu() else { return };
    // Transparency Shapes Layer off on a layer clipped to a base stays on the CPU (its behaviour
    // there is changing separately), for effects and for the special eight.
    for (blend, fx, what) in [(BlendMode::ColorDodge, false, "Color Dodge"), (BlendMode::Normal, true, "layer effects")] {
        let mut d = base_doc(48, 40);
        d.layers.push(noise_layer("base", d.pixel_format(), Rect::new(4, 4, 44, 36), 3, 0.6));
        let mut c = noise_layer("clipped", d.pixel_format(), Rect::new(8, 8, 40, 32), 4, 0.5);
        c.clipped = true;
        c.blend = blend;
        c.advanced.transparency_shapes = false;
        if fx {
            c.effects.items.push(Effect::default_drop_shadow());
        }
        d.layers.push(c);
        let e = photocraft_gpu::render_to_vec_stats(&mut g.comp, &g.device, &g.queue, &d, d.bounds())
            .expect_err("the GPU must refuse a clipped layer without transparency shapes");
        assert!(e.to_string().contains("Transparency Shapes Layer off") && e.to_string().contains("clipped layer"), "{what}: {e}");
    }
}

/// The special eight with Transparency Shapes Layer off (the layer's own alpha joins Fill inside
/// the blend, coverage is Opacity × masks), at every depth, with and without a mask.
#[test]
fn shapeless_special_blends_match_the_cpu() {
    let Some(mut g) = gpu() else { return };
    let modes = [BlendMode::ColorDodge, BlendMode::LinearBurn, BlendMode::VividLight, BlendMode::HardMix];
    // Hard Mix runs on the GPU at 8 bits only (see `hard_mix_shapeless_at_16_and_32_bits_stays_on_the_cpu`).
    let combos = [(1.0, 1.0), (0.5, 1.0), (1.0, 0.5), (0.6, 0.4)];
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for mode in modes {
            if mode == BlendMode::HardMix && depth != SampleType::U8 {
                continue;
            }
            for (opacity, fill) in combos {
                for masked in [false, true] {
                    let mut d = doc(depth, true);
                    d.layers.push(underlay(&d, 2));
                    let mut l = noise_layer("shapeless", d.pixel_format(), Rect::new(6, 5, 42, 35), 3, 0.2);
                    l.blend = mode;
                    l.opacity = opacity;
                    l.fill_opacity = fill;
                    l.advanced.transparency_shapes = false;
                    if mode == BlendMode::HardMix {
                        // Hard Mix below 100 % Fill divides by 1 − alpha × Fill: on noise, an alpha
                        // just under 1 turns rounding into visible differences on either compositor.
                        // `hard_mix_shapeless_near_opaque_matches_the_cpu` pins near-opaque alpha.
                        let (r, ch) = (Rect::new(6, 5, 42, 35), d.pixel_format().channels());
                        let surf = l.surface_mut().unwrap();
                        let mut px = surf.read_region(r);
                        px.iter_mut().skip(ch - 1).step_by(ch).for_each(|a| *a = a.min(0.85));
                        surf.write_region(r, &px);
                    }
                    if masked {
                        l.mask = Some(mask(Rect::new(0, 0, 48, 40), 9, 0.4));
                    }
                    d.layers.push(l);
                    check(&mut g, &d, &format!("{mode:?} {depth:?} opacity {opacity} fill {fill} mask {masked}"));
                }
            }
        }
    }
    // A mode outside the special eight looks the same either way.
    let mut d = doc(SampleType::U8, true);
    d.layers.push(underlay(&d, 2));
    let mut l = noise_layer("multiply", d.pixel_format(), Rect::new(6, 5, 42, 35), 3, 0.2);
    l.blend = BlendMode::Multiply;
    l.opacity = 0.7;
    l.fill_opacity = 0.6;
    l.advanced.transparency_shapes = false;
    d.layers.push(l);
    check(&mut g, &d, "Multiply without transparency shapes");
}

/// Hard Mix without Transparency Shapes Layer around an opaque alpha at 8 bits: where source and
/// backdrop sit on the threshold (source = 1 − backdrop), an opaque pixel at full Fill takes the
/// binary branch and gives 1, while any lower alpha (or Fill) divides its way back to the
/// backdrop. Half-float accumulation leaves the document to the CPU.
#[test]
fn hard_mix_shapeless_near_opaque_matches_the_cpu() {
    let Some(mut g) = gpu() else { return };
    let mut half = half_float_compositor(&g);
    // Mid-range backdrop levels, where neither side of the threshold is near a corner case.
    let level = |x: i32, y: i32, c: i32| 64 + (x * 7 + y * 13 + c * 31) % 128;
    let thin = {
        let mut m = LayerMask::reveal_all();
        m.surface = Surface::with_default(PixelFormat::GRAY8, &[254.0 / 255.0]);
        m.density = 0.01;
        m
    };
    let masks = [("none", None), ("near-opaque", Some(thin)), ("noise", Some(mask(Rect::new(0, 0, 48, 40), 9, 0.4)))];
    for fill in [1.0, 0.5] {
        for opacity in [1.0, 0.7] {
            for (mask_name, layer_mask) in &masks {
                let what = format!("near-opaque Hard Mix, fill {fill}, opacity {opacity}, mask {mask_name}");
                let mut d = doc(SampleType::U8, false);
                let r = d.bounds();
                let pixels = || (r.y0..r.y1).flat_map(move |y| (r.x0..r.x1).map(move |x| (x, y)));
                let mut bg = Layer::raster("Background", d.pixel_format());
                bg.locks.transparency = true;
                bg.locks.position = true;
                let under: Vec<f32> = pixels().flat_map(|(x, y)| [level(x, y, 0), level(x, y, 1), level(x, y, 2), 255].map(|v| v as f32 / 255.0)).collect();
                bg.surface_mut().unwrap().write_region(r, &under);
                d.layers.push(bg);
                // Alpha cycles 255, 254, 253 along a row. Even rows sit on the threshold; the
                // others are noise.
                let mut l = Layer::raster("hard mix", d.pixel_format());
                let source: Vec<f32> = pixels()
                    .flat_map(|(x, y)| {
                        let colour = |c: i32| if y % 2 == 0 { 255 - level(x, y, c) } else { (x * 53 + y * 29 + c * 97) % 256 };
                        [colour(0), colour(1), colour(2), 255 - x % 3].map(|v| v as f32 / 255.0)
                    })
                    .collect();
                l.surface_mut().unwrap().write_region(r, &source);
                l.blend = BlendMode::HardMix;
                l.fill_opacity = fill;
                l.opacity = opacity;
                l.advanced.transparency_shapes = false;
                l.mask = layer_mask.clone();
                d.layers.push(l);
                let cpu = photocraft_compose::render(&d, r);
                for ((x, y), (out, back)) in pixels().zip(cpu.px.iter().zip(under.as_chunks::<4>().0)) {
                    if y % 2 != 0 {
                        continue;
                    }
                    let moved = out[0] - back[0];
                    if fill >= 1.0 && x % 3 == 0 {
                        assert!(moved > 4.0 / 255.0, "{what}: an opaque pixel at ({x},{y}) goes to 1: {moved}");
                    } else {
                        assert!(moved.abs() < 1.0 / 255.0, "{what}: the pixel at ({x},{y}) keeps the backdrop: {moved}");
                    }
                }
                check(&mut g, &d, &what);
                let e = photocraft_gpu::render_to_vec_stats(&mut half, &g.device, &g.queue, &d, r)
                    .expect_err("half-float accumulation must refuse Hard Mix without transparency shapes");
                assert!(e.to_string().contains("Hard Mix"), "{what}: {e}");
                assert!(half.supports(&d).is_err(), "{what}");
            }
        }
    }
}

/// Half floats can't keep a near-opaque alpha, and Hard Mix's Fill formula divides by
/// 1 − alpha × Fill: a 16/32-bit document stays on the CPU.
#[test]
fn hard_mix_shapeless_at_16_and_32_bits_stays_on_the_cpu() {
    let Some(mut g) = gpu() else { return };
    for (depth, near) in [(SampleType::U16, 65534.0 / 65535.0), (SampleType::F32, 0.9999)] {
        let mut d = doc(depth, false);
        let mut under = Layer::raster("under", d.pixel_format());
        under.surface_mut().unwrap().fill_rect(d.bounds(), &[0.25, 0.25, 0.25, 1.0]);
        // On the threshold: opaque on the left, near-opaque on the right.
        let mut l = Layer::raster("shapeless", d.pixel_format());
        l.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 24, 40), &[0.75, 0.75, 0.75, 1.0]);
        l.surface_mut().unwrap().fill_rect(Rect::new(24, 0, 48, 40), &[0.75, 0.75, 0.75, near]);
        l.blend = BlendMode::HardMix;
        l.advanced.transparency_shapes = false;
        d.layers.extend([under, l]);
        let cpu = photocraft_compose::render(&d, d.bounds());
        let (opaque, nearly) = (cpu.px[0][0], cpu.px[47][0]);
        assert!(opaque - nearly > 0.5, "{depth:?}: opaque {opaque}, near-opaque {nearly}");
        let e = photocraft_gpu::render_to_vec_stats(&mut g.comp, &g.device, &g.queue, &d, d.bounds())
            .expect_err("the GPU must refuse Hard Mix without transparency shapes at 16/32 bits");
        assert!(e.to_string().contains("Hard Mix"), "{depth:?}: {e}");
        let e = g.comp.supports(&d).expect_err("supports() agrees");
        assert!(e.to_string().contains("Hard Mix"), "{depth:?}: {e}");
    }
}

#[test]
fn shapeless_special_blends_in_groups_and_clipping_groups_match_the_cpu() {
    let Some(mut g) = gpu() else { return };
    for pass in [false, true] {
        for (opacity, fill) in [(1.0, 1.0), (0.6, 0.4)] {
            // A shapeless layer inside the group.
            let mut d = doc(SampleType::U8, true);
            d.layers.push(underlay(&d, 2));
            let mut inner = noise_layer("inner", d.pixel_format(), Rect::new(4, 4, 44, 36), 5, 0.3);
            inner.blend = BlendMode::VividLight;
            inner.opacity = opacity;
            inner.fill_opacity = fill;
            inner.advanced.transparency_shapes = false;
            let below = noise_layer("below", d.pixel_format(), Rect::new(0, 0, 30, 30), 6, 0.5);
            d.layers.push(group("group", pass, vec![below, inner]));
            check(&mut g, &d, &format!("inside a group, pass-through {pass}, opacity {opacity}, fill {fill}"));

            // A shapeless group.
            let mut d = doc(SampleType::U8, true);
            d.layers.push(underlay(&d, 2));
            let inner = noise_layer("inner", d.pixel_format(), Rect::new(4, 4, 44, 36), 5, 0.3);
            let mut grp = group("group", pass, vec![inner]);
            grp.blend = BlendMode::ColorDodge;
            grp.opacity = opacity;
            grp.fill_opacity = fill;
            grp.advanced.transparency_shapes = false;
            grp.mask = Some(mask(Rect::new(0, 0, 48, 40), 9, 0.4));
            d.layers.push(grp);
            check(&mut g, &d, &format!("shapeless group, pass-through {pass}, opacity {opacity}, fill {fill}"));
        }
    }
    // A shapeless base with layers clipped to it: the clips composite onto the base's content,
    // which the mask then splits.
    for masked in [false, true] {
        for (opacity, fill) in [(1.0, 1.0), (0.6, 0.4)] {
            let mut d = doc(SampleType::U8, true);
            d.layers.push(underlay(&d, 2));
            let mut base = noise_layer("base", d.pixel_format(), Rect::new(4, 4, 44, 36), 3, 0.4);
            base.blend = BlendMode::LinearBurn;
            base.opacity = opacity;
            base.fill_opacity = fill;
            base.advanced.transparency_shapes = false;
            if masked {
                base.mask = Some(mask(Rect::new(0, 0, 48, 40), 9, 0.4));
            }
            let mut c1 = noise_layer("clipped 1", d.pixel_format(), Rect::new(8, 8, 44, 34), 7, 0.5);
            c1.clipped = true;
            c1.opacity = 0.7;
            let mut c2 = noise_layer("clipped 2", d.pixel_format(), Rect::new(2, 10, 38, 30), 8, 0.4);
            c2.clipped = true;
            c2.blend = BlendMode::Multiply;
            d.layers.extend([base, c1, c2]);
            check(&mut g, &d, &format!("shapeless base with clips, mask {masked}, opacity {opacity}, fill {fill}"));
        }
    }
    // With a knockout too.
    for knockout in KNOCKOUTS {
        let mut d = doc(SampleType::U8, true);
        d.layers.push(underlay(&d, 2));
        let mut l = ko_layer(&d, 3, knockout);
        l.blend = BlendMode::ColorDodge;
        l.opacity = 0.8;
        l.fill_opacity = 0.5;
        l.advanced.transparency_shapes = false;
        d.layers.push(l);
        run(&mut g, &d, "shapeless special blend with a knockout", knockout);
    }
}

/// Transparency Shapes Layer off with effects: the effects take the whole layer as their shape
/// and the layer's own transparency acts like fill opacity within it.
#[test]
fn shapeless_effects_match_the_cpu() {
    use photocraft_doc::{FxCommon, FxPaint, Glow, GlowSource, GlowTechnique, StrokeFx, StrokePosition};
    let Some(mut g) = gpu() else { return };
    let glow = |source| Glow {
        common: FxCommon::new(BlendMode::Screen, 0.8),
        paint: FxPaint::Color(Color::rgb(1.0, 0.9, 0.2)),
        technique: GlowTechnique::Softer,
        spread: 0.1,
        size: 5.0,
        contour: photocraft_doc::Contour::Linear,
        anti_alias: false,
        range: 0.5,
        jitter: 0.0,
        noise: 0.0,
        source,
    };
    let Effect::DropShadow(mut shadow) = Effect::default_drop_shadow() else { unreachable!() };
    shadow.distance = 4.0;
    shadow.size = 3.0;
    let stroke = |position| {
        Effect::Stroke(StrokeFx {
            common: FxCommon::new(BlendMode::Normal, 0.9),
            size: 3.0,
            position,
            paint: FxPaint::Color(Color::rgb(0.1, 0.7, 0.3)),
            overprint: false,
        })
    };
    let cases: Vec<(&str, Vec<Effect>)> = vec![
        ("drop shadow", vec![Effect::DropShadow(shadow)]),
        ("outer glow", vec![Effect::OuterGlow(glow(GlowSource::Edge))]),
        ("inner glow", vec![Effect::InnerGlow(glow(GlowSource::Edge))]),
        ("outside stroke", vec![stroke(StrokePosition::Outside)]),
        ("inside stroke", vec![stroke(StrokePosition::Inside)]),
        ("color overlay", vec![Effect::ColorOverlay { common: FxCommon::new(BlendMode::Normal, 0.7), color: Color::rgb(0.9, 0.2, 0.3) }]),
        (
            "overlay and glow",
            vec![
                Effect::ColorOverlay { common: FxCommon::new(BlendMode::Multiply, 0.8), color: Color::rgb(0.9, 0.5, 0.3) },
                Effect::OuterGlow(glow(GlowSource::Edge)),
            ],
        ),
    ];
    for (name, items) in cases {
        for (opacity, fill, masked) in [(1.0, 1.0, false), (0.7, 0.5, false), (1.0, 0.4, true)] {
            for blend in [BlendMode::Normal, BlendMode::Multiply] {
                let mut d = doc(SampleType::U8, true);
                d.layers.push(underlay(&d, 2));
                let mut l = noise_layer("fx", d.pixel_format(), Rect::new(8, 7, 40, 33), 3, 0.3);
                l.effects.items.clone_from(&items);
                l.advanced.transparency_shapes = false;
                l.blend = blend;
                l.opacity = opacity;
                l.fill_opacity = fill;
                if masked {
                    l.mask = Some(mask(Rect::new(0, 0, 48, 40), 9, 0.4));
                }
                d.layers.push(l);
                check(&mut g, &d, &format!("{name}, {blend:?}, opacity {opacity}, fill {fill}, mask {masked}"));
            }
        }
    }
    // The layer's clipping group joins the content the effects take their colour from.
    let mut d = doc(SampleType::U8, true);
    d.layers.push(underlay(&d, 2));
    let mut base = noise_layer("base", d.pixel_format(), Rect::new(8, 7, 40, 33), 3, 0.3);
    base.effects.items.push(Effect::default_drop_shadow());
    base.advanced.transparency_shapes = false;
    base.fill_opacity = 0.6;
    let mut c = noise_layer("clipped", d.pixel_format(), Rect::new(10, 9, 36, 30), 4, 0.5);
    c.clipped = true;
    d.layers.extend([base, c]);
    check(&mut g, &d, "shapeless effects with a clipped layer");
}

#[test]
fn clipped_layers_blended_individually_match_the_cpu() {
    let Some(mut g) = gpu() else { return };
    for (opacity, fill) in [(1.0, 1.0), (0.6, 1.0), (0.6, 0.5)] {
        let mut d = doc(SampleType::U8, true);
        d.layers.push(underlay(&d, 2));
        let mut base = clip_base(&d);
        base.fill_opacity = fill;
        let layers = clipped_pair(&d, base, opacity);
        d.layers.extend(layers);
        check(&mut g, &d, &format!("pixel base, opacity {opacity}, fill {fill}"));
    }
    // A clipped layer in Screen mode, and a hidden clipped layer next to a visible one.
    let mut d = doc(SampleType::U8, true);
    d.layers.push(underlay(&d, 2));
    let mut layers = clipped_pair(&d, clip_base(&d), 0.8);
    layers[2].blend = BlendMode::Screen;
    let mut hidden = noise_layer("hidden", d.pixel_format(), d.bounds(), 11, 0.9);
    hidden.clipped = true;
    hidden.visible = false;
    layers.insert(2, hidden);
    d.layers.extend(layers);
    check(&mut g, &d, "screen clipped layer and a hidden one");
}

#[test]
fn clipped_layers_blended_individually_on_groups_match_the_cpu() {
    let Some(mut g) = gpu() else { return };
    for pass in [false, true] {
        for opacity in [1.0, 0.6] {
            let mut d = doc(SampleType::U8, true);
            d.layers.push(underlay(&d, 2));
            let inner = noise_layer("inner", d.pixel_format(), Rect::new(0, 0, 40, 34), 5, 0.4);
            let layers = clipped_pair(&d, group("base group", pass, vec![inner]), opacity);
            d.layers.extend(layers);
            check(&mut g, &d, &format!("group base (pass-through {pass}), opacity {opacity}"));
        }
    }
}

#[test]
fn individually_clipped_knockouts_match_the_cpu() {
    let Some(mut g) = gpu() else { return };
    for knockout in KNOCKOUTS {
        for base_knockout in [Knockout::None, Knockout::Shallow, Knockout::Deep] {
            for opacity in [1.0, 0.6] {
                let mut d = doc(SampleType::U8, true);
                d.layers.push(underlay(&d, 2));
                let mut base = clip_base(&d);
                base.advanced.knockout = base_knockout;
                let mut layers = clipped_pair(&d, base, opacity);
                layers[1].advanced.knockout = knockout;
                d.layers.extend(layers);
                run(&mut g, &d, &format!("base knockout {base_knockout:?}, opacity {opacity}"), knockout);
            }
        }
    }
}

/// An opaque fill base is not skipped over: a clipped shallow knockout reveals the layers
/// beneath it.
#[test]
fn individually_clipped_shallow_knockout_reaches_beneath_an_opaque_fill() {
    let Some(mut g) = gpu() else { return };
    let mut d = doc(SampleType::U8, true);
    d.layers.push(underlay(&d, 2));
    let mut fill = Layer::new("fill", LayerContent::Fill(Fill::Solid(Color::rgba(0.2, 0.5, 0.8, 1.0))));
    fill.advanced.blend_clipped = false;
    let mut c = ko_layer(&d, 3, Knockout::Shallow);
    c.clipped = true;
    d.layers.extend([fill, c]);
    check(&mut g, &d, "shallow knockout under an opaque fill base");
}

/// Where the base covers a pixel fully an individually blended clipped layer keeps its 32-bit
/// values above 1; where the base is partly transparent, however slightly, the clamped mix
/// applies, as on the CPU. Half-float accumulation tells the two apart as well.
#[test]
fn individually_clipped_hdr_values_match_the_cpu() {
    let Some(mut g) = gpu() else { return };
    let mut half = half_float_compositor(&g);
    // The base's alpha at every third pixel, and its mask there (sample depth, value, density).
    let cases = [(1.0, None), (0.4, None), (0.9999, None), (1.0, Some((SampleType::U8, 254.0 / 255.0, 0.01))), (1.0, Some((SampleType::F32, 0.9999, 1.0)))];
    for (base_alpha, masked) in cases {
        for opacity in [1.0, 0.7] {
            let what = format!("HDR clipped layer, base alpha {base_alpha}, mask {masked:?}, opacity {opacity}");
            let mut d = doc(SampleType::F32, true);
            d.layers.push(underlay(&d, 2));
            let mut base = Layer::raster("base", d.pixel_format());
            let rect = Rect::new(4, 4, 40, 34);
            let px: Vec<f32> = (0..rect.width() * rect.height()).flat_map(|i| [0.3, 0.5, 0.2, if i % 3 == 0 { base_alpha } else { 1.0 }]).collect();
            base.surface_mut().unwrap().write_region(rect, &px);
            if let Some((sample, value, density)) = masked {
                let mut m = LayerMask::reveal_all();
                m.surface = Surface::with_default(PixelFormat { sample, ..PixelFormat::GRAY8 }, &[1.0]);
                let values: Vec<f32> = (0..rect.width() * rect.height()).map(|i| if i % 3 == 0 { value } else { 1.0 }).collect();
                m.surface.write_region(rect, &values);
                m.density = density;
                base.mask = Some(m);
            }
            base.opacity = opacity;
            base.advanced.blend_clipped = false;
            let mut c = Layer::raster("hdr clip", d.pixel_format());
            let crect = Rect::new(8, 8, 38, 30);
            let cpx: Vec<f32> = (0..crect.width() * crect.height()).flat_map(|_| [2.0, 0.4, 0.6, 1.0]).collect();
            c.surface_mut().unwrap().write_region(crect, &cpx);
            c.clipped = true;
            d.layers.extend([base, c]);
            // Where the base covers fully (opaque pixels at opacity 1) the CPU keeps the clipped
            // value; below full coverage it mixes, which clamps.
            let partial = base_alpha < 1.0 || masked.is_some();
            let full = |x: i32, y: i32| opacity >= 1.0 && !(partial && ((y - rect.y0) * rect.width() as i32 + x - rect.x0) % 3 == 0);
            let classify = |px: &[[f32; 4]], backend: &str| {
                for y in crect.y0..crect.y1 {
                    for x in crect.x0..crect.x1 {
                        let red = px[(y * d.bounds().width() as i32 + x) as usize][0];
                        assert_eq!(red > 1.0, full(x, y), "{what}, {backend} at ({x},{y}): red {red}");
                    }
                }
            };
            classify(&photocraft_compose::render(&d, d.bounds()).px, "CPU");
            check(&mut g, &d, &what);
            classify(&check_with(&g, &mut half, &d, 3.0 / 255.0, &what), "half-float GPU");
        }
    }
}

/// A 32-bit base that takes several passes (a group) keeps Blend Clipped Layers as Group off on
/// the CPU; at 8 bits it renders on the GPU.
#[test]
fn individually_clipped_multi_pass_bases_stay_on_the_cpu_at_32_bits() {
    let Some(mut g) = gpu() else { return };
    for depth in [SampleType::U8, SampleType::F32] {
        let mut d = doc(depth, true);
        d.layers.push(underlay(&d, 2));
        let inner = noise_layer("inner", d.pixel_format(), Rect::new(0, 0, 40, 34), 5, 0.4);
        let layers = clipped_pair(&d, group("base group", false, vec![inner]), 1.0);
        d.layers.extend(layers);
        if depth == SampleType::F32 {
            let e = photocraft_gpu::render_to_vec_stats(&mut g.comp, &g.device, &g.queue, &d, d.bounds())
                .expect_err("the GPU must refuse a multi-pass 32-bit base");
            assert!(e.to_string().contains("Blend Clipped Layers as Group off"), "{e}");
            assert!(g.comp.supports(&d).is_err());
        } else {
            check(&mut g, &d, "group base at 8 bits");
        }
    }
}

#[test]
fn individually_clipped_layers_with_masks_restrictions_and_blend_if_match_the_cpu() {
    let Some(mut g) = gpu() else { return };
    for opacity in [1.0, 0.6] {
        let mut d = doc(SampleType::U8, true);
        d.layers.push(underlay(&d, 2));
        let mut base = clip_base(&d);
        base.mask = Some(mask(Rect::new(0, 0, 48, 40), 9, 0.4));
        let layers = clipped_pair(&d, base, opacity);
        d.layers.extend(layers);
        check(&mut g, &d, &format!("masked base, opacity {opacity}"));

        let mut d = doc(SampleType::U8, true);
        d.layers.push(underlay(&d, 2));
        let mut layers = clipped_pair(&d, clip_base(&d), opacity);
        layers[1].excluded_channels = 0b010;
        layers[2].blend_if.set(0, [BlendRange { black: [60, 120], white: [255, 255] }, BlendRange::FULL]);
        d.layers.extend(layers);
        check(&mut g, &d, &format!("clipped channel restrictions and Blend If, opacity {opacity}"));
    }
}

#[test]
fn individually_clipped_layers_in_groups_and_depths_match_the_cpu() {
    let Some(mut g) = gpu() else { return };
    for pass in [false, true] {
        for knockout in KNOCKOUTS {
            let mut d = doc(SampleType::U8, true);
            d.layers.push(underlay(&d, 2));
            let mut layers = clipped_pair(&d, clip_base(&d), 0.7);
            layers[1].advanced.knockout = knockout;
            d.layers.push(group("group", pass, layers));
            run(&mut g, &d, &format!("inside a group (pass-through {pass})"), knockout);
        }
    }
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut d = doc(depth, true);
        d.layers.push(underlay(&d, 2));
        let layers = clipped_pair(&d, clip_base(&d), 0.6);
        d.layers.extend(layers);
        check(&mut g, &d, &format!("{depth:?}"));
    }
}

/// How a layer's two masks hide its effects: the one that doesn't still shapes the layer.
#[derive(Clone, Copy, Debug)]
enum Hides {
    LayerMask,
    VectorMask,
    Both,
}

const HIDES: [Hides; 3] = [Hides::LayerMask, Hides::VectorMask, Hides::Both];

fn tri_mask() -> VectorMask {
    VectorMask::new(Path::new(vec![Subpath::polygon(&[(6.5, 5.2), (40.3, 9.1), (30.2, 36.4), (9.7, 30.8)])]))
}

/// `l` with a layer mask and a vector mask, `hides` of which hide its effects.
fn hide_with(l: &mut Layer, hides: Hides) {
    l.mask = Some(mask(Rect::new(0, 0, 48, 40), 9, 0.4));
    l.vector_mask = Some(tri_mask());
    l.advanced.layer_mask_hides_effects = matches!(hides, Hides::LayerMask | Hides::Both);
    l.advanced.vector_mask_hides_effects = matches!(hides, Hides::VectorMask | Hides::Both);
}

/// Effects that reach outside the layer, inside it and across it.
fn hidden_effects() -> Vec<(&'static str, Effect)> {
    use photocraft_doc::{FxCommon, FxPaint, Glow, GlowSource, GlowTechnique, StrokeFx, StrokePosition};
    let Effect::DropShadow(mut shadow) = Effect::default_drop_shadow() else { unreachable!() };
    shadow.distance = 4.0;
    shadow.size = 3.0;
    let glow = Glow {
        common: FxCommon::new(BlendMode::Screen, 0.8),
        paint: FxPaint::Color(Color::rgb(1.0, 0.9, 0.2)),
        technique: GlowTechnique::Softer,
        spread: 0.1,
        size: 5.0,
        contour: photocraft_doc::Contour::Linear,
        anti_alias: false,
        range: 0.5,
        jitter: 0.0,
        noise: 0.0,
        source: GlowSource::Edge,
    };
    let stroke = |position| {
        Effect::Stroke(StrokeFx {
            common: FxCommon::new(BlendMode::Normal, 0.9),
            size: 3.0,
            position,
            paint: FxPaint::Color(Color::rgb(0.1, 0.7, 0.3)),
            overprint: false,
        })
    };
    vec![
        ("drop shadow", Effect::DropShadow(shadow)),
        ("outer glow", Effect::OuterGlow(glow)),
        ("outside stroke", stroke(StrokePosition::Outside)),
        ("inside stroke", stroke(StrokePosition::Inside)),
        ("color overlay", Effect::ColorOverlay { common: FxCommon::new(BlendMode::Normal, 0.7), color: Color::rgb(0.9, 0.2, 0.3) }),
    ]
}

/// Layer / Vector Mask Hides Effects: the layer and its effects are built without the hiding
/// mask(s), which then mix the finished result back towards the backdrop.
#[test]
fn masks_hiding_effects_match_the_cpu() {
    let Some(mut g) = gpu() else { return };
    for (name, effect) in hidden_effects() {
        for hides in HIDES {
            for (opacity, fill) in [(1.0, 1.0), (0.6, 0.4)] {
                let mut d = doc(SampleType::U8, true);
                d.layers.push(underlay(&d, 2));
                let mut l = noise_layer("fx", d.pixel_format(), Rect::new(8, 7, 40, 33), 3, 0.3);
                l.effects.items.push(effect.clone());
                l.opacity = opacity;
                l.fill_opacity = fill;
                hide_with(&mut l, hides);
                d.layers.push(l);
                check(&mut g, &d, &format!("{name}, {hides:?} hiding, opacity {opacity}, fill {fill}"));
            }
        }
    }
    // Several effects, a single mask, a mask that is switched off, and a hiding setting without
    // a mask to act on.
    let items: Vec<Effect> = hidden_effects().into_iter().map(|(_, e)| e).collect();
    for what in ["several effects and only a layer mask", "a disabled hiding mask", "a hiding setting without its mask"] {
        let mut d = doc(SampleType::U8, true);
        d.layers.push(underlay(&d, 2));
        let mut l = noise_layer("fx", d.pixel_format(), Rect::new(8, 7, 40, 33), 3, 0.3);
        l.effects.items.extend(items.iter().cloned());
        l.opacity = 0.8;
        match what {
            "several effects and only a layer mask" => {
                l.mask = Some(mask(Rect::new(0, 0, 48, 40), 9, 0.4));
                l.advanced.layer_mask_hides_effects = true;
            }
            "a disabled hiding mask" => {
                hide_with(&mut l, Hides::Both);
                l.mask.as_mut().unwrap().enabled = false;
                l.advanced.vector_mask_hides_effects = false;
            }
            _ => {
                l.vector_mask = Some(tri_mask());
                l.advanced.layer_mask_hides_effects = true;
            }
        }
        d.layers.push(l);
        check(&mut g, &d, what);
    }
}

#[test]
fn masks_hiding_effects_in_every_depth() {
    let Some(mut g) = gpu() else { return };
    let effects = hidden_effects();
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for hides in HIDES {
            let mut d = doc(depth, true);
            d.layers.push(underlay(&d, 2));
            let mut l = noise_layer("fx", d.pixel_format(), Rect::new(8, 7, 40, 33), 3, 0.3);
            l.effects.items.extend([effects[0].1.clone(), effects[3].1.clone()]);
            l.opacity = 0.9;
            l.fill_opacity = 0.7;
            hide_with(&mut l, hides);
            d.layers.push(l);
            check(&mut g, &d, &format!("{depth:?}, {hides:?} hiding"));
        }
    }
}

/// With a knockout the hiding mask mixes towards the knocked-out backdrop (the layer's own
/// `before`), not the one beneath the knockout.
#[test]
fn masks_hiding_effects_with_a_knockout_match_the_cpu() {
    let Some(mut g) = gpu() else { return };
    let effect = hidden_effects().remove(0).1;
    for knockout in KNOCKOUTS {
        for opacity in [1.0, 0.6] {
            let mut d = doc(SampleType::U8, true);
            d.layers.push(underlay(&d, 2));
            let mut l = ko_layer(&d, 3, knockout);
            l.effects.items.push(effect.clone());
            l.opacity = opacity;
            hide_with(&mut l, Hides::Both);
            d.layers.push(l);
            run(&mut g, &d, &format!("hiding mask, opacity {opacity}"), knockout);
        }
    }
}

#[test]
fn masks_hiding_effects_in_groups_match_the_cpu() {
    let Some(mut g) = gpu() else { return };
    let effects = hidden_effects();
    for pass in [false, true] {
        for hides in HIDES {
            for (opacity, fill) in [(1.0, 1.0), (0.6, 0.4)] {
                // A layer with hiding masks inside the group.
                let mut d = doc(SampleType::U8, true);
                d.layers.push(underlay(&d, 2));
                let below = noise_layer("below", d.pixel_format(), Rect::new(0, 0, 30, 30), 6, 0.5);
                let mut inner = noise_layer("inner", d.pixel_format(), Rect::new(8, 7, 40, 33), 5, 0.3);
                inner.effects.items.push(effects[0].1.clone());
                inner.opacity = opacity;
                inner.fill_opacity = fill;
                hide_with(&mut inner, hides);
                d.layers.push(group("group", pass, vec![below, inner]));
                check(&mut g, &d, &format!("inside a group, pass-through {pass}, {hides:?} hiding, opacity {opacity}, fill {fill}"));

                // A group whose masks hide its effects.
                let mut d = doc(SampleType::U8, true);
                d.layers.push(underlay(&d, 2));
                let a = noise_layer("a", d.pixel_format(), Rect::new(8, 7, 30, 30), 5, 0.3);
                let b = noise_layer("b", d.pixel_format(), Rect::new(16, 12, 40, 33), 6, 0.5);
                let mut grp = group("group", pass, vec![a, b]);
                grp.effects.items.extend([effects[0].1.clone(), effects[2].1.clone()]);
                grp.opacity = opacity;
                grp.fill_opacity = fill;
                hide_with(&mut grp, hides);
                d.layers.push(grp);
                check(&mut g, &d, &format!("group hiding its effects, pass-through {pass}, {hides:?} hiding, opacity {opacity}, fill {fill}"));
            }
        }
    }
}

#[test]
fn masks_hiding_effects_with_clipped_layers_match_the_cpu() {
    let Some(mut g) = gpu() else { return };
    let effects = hidden_effects();
    for hides in HIDES {
        for (opacity, fill) in [(1.0, 1.0), (0.6, 0.4)] {
            // A base whose masks hide its effects, with layers clipped to it (grouped and
            // blended individually).
            for individually in [false, true] {
                let mut d = doc(SampleType::U8, true);
                d.layers.push(underlay(&d, 2));
                let mut base = noise_layer("base", d.pixel_format(), Rect::new(6, 6, 42, 34), 3, 0.4);
                base.effects.items.extend([effects[0].1.clone(), effects[2].1.clone()]);
                base.opacity = opacity;
                base.fill_opacity = fill;
                hide_with(&mut base, hides);
                let mut layers = clipped_pair(&d, base, opacity);
                layers[0].advanced.blend_clipped = !individually;
                d.layers.extend(layers);
                check(&mut g, &d, &format!("hiding base with clips, individually {individually}, {hides:?} hiding, opacity {opacity}, fill {fill}"));
            }
            // A clipped layer whose masks hide its effects.
            for effect in [&effects[0].1, &effects[3].1, &effects[4].1] {
                let mut d = doc(SampleType::U8, true);
                d.layers.push(underlay(&d, 2));
                d.layers.push(clip_base(&d));
                let mut c = noise_layer("clipped", d.pixel_format(), Rect::new(8, 8, 40, 32), 4, 0.5);
                c.clipped = true;
                c.effects.items.push(effect.clone());
                c.opacity = opacity;
                c.fill_opacity = fill;
                hide_with(&mut c, hides);
                d.layers.push(c);
                check(&mut g, &d, &format!("clipped layer hiding its effects, {hides:?} hiding, opacity {opacity}, fill {fill}"));
            }
        }
    }
}

/// Where the hiding mask lets everything through the result is the layer with its effects
/// untouched (the CPU only mixes where the mask is below 1), so 32-bit values above 1 survive
/// there; elsewhere the mix clamps them.
#[test]
fn masks_hiding_effects_keep_hdr_values_where_they_let_everything_through() {
    let Some(mut g) = gpu() else { return };
    for effect in [hidden_effects().remove(0).1, hidden_effects().remove(4).1] {
        let mut d = doc(SampleType::F32, false);
        let mut bottom = Layer::raster("hdr", d.pixel_format());
        let patterns = [[2.0, 1.5, 0.25, 1.0], [1.25, 3.0, 0.5, 1.0], [0.4, 0.6, 0.8, 1.0]];
        let px: Vec<f32> = (0..48 * 40).flat_map(|i| patterns[i % patterns.len()]).collect();
        bottom.surface_mut().unwrap().write_region(Rect::from_xywh(0, 0, 48, 40), &px);
        d.layers.push(bottom);
        let mut l = noise_layer("fx", d.pixel_format(), Rect::new(6, 5, 42, 35), 3, 0.2);
        l.effects.items.push(effect);
        l.opacity = 0.5;
        let mut m = LayerMask::reveal_all();
        m.surface.fill_rect(Rect::new(24, 0, 48, 40), &[0.5]);
        l.mask = Some(m);
        l.advanced.layer_mask_hides_effects = true;
        d.layers.push(l);
        let cpu = photocraft_compose::render(&d, d.bounds());
        let open = |x: i32, y: i32| cpu.px[(y * 48 + x) as usize][..3].iter().any(|&v| v > 1.0);
        assert!((5..35).any(|y| (6..24).any(|x| open(x, y))), "the CPU keeps a value above 1 where the mask lets everything through");
        check(&mut g, &d, "HDR under a fully open hiding mask");
    }
}

/// Under a mask that hides the effects, a 32-bit layer keeps its values above 1 only where the
/// mask covers fully, the backdrop is copied where it covers nothing, and every coverage between,
/// however near either end, takes the clamped mix. Half-float accumulation tells them apart too.
#[test]
fn masks_hiding_effects_tell_near_full_coverage_from_full() {
    let Some(mut g) = gpu() else { return };
    let mut half = half_float_compositor(&g);
    let backdrop = 3.0;
    // The mask over the left half (sample depth, value, density); the right half reveals fully.
    let cases = [
        (SampleType::U8, 1.0, 1.0),
        (SampleType::U8, 254.0 / 255.0, 0.01),
        (SampleType::U16, 65534.0 / 65535.0, 1.0),
        (SampleType::U16, 1.0 / 65535.0, 1.0),
        (SampleType::U8, 0.0, 1.0),
        (SampleType::U16, 0.0, 1.0),
    ];
    for (sample, value, density) in cases {
        let what = format!("hiding mask {sample:?} {value} at density {density}");
        let mut d = doc(SampleType::F32, false);
        let mut bottom = Layer::raster("hdr", d.pixel_format());
        bottom.surface_mut().unwrap().fill_rect(d.bounds(), &[backdrop, 1.5, 0.25, 1.0]);
        let mut l = Layer::raster("fx", d.pixel_format());
        l.surface_mut().unwrap().fill_rect(Rect::new(6, 5, 42, 35), &[2.5, 0.4, 0.6, 1.0]);
        let Effect::DropShadow(mut shadow) = Effect::default_drop_shadow() else { unreachable!() };
        shadow.common.opacity = 0.5;
        l.effects.items.push(Effect::DropShadow(shadow));
        d.layers.extend([bottom, l]);
        // The layer with its effects, which the mask reveals, is above 1 everywhere.
        let revealed = photocraft_compose::render(&d, d.bounds());
        assert!(revealed.px.iter().all(|p| p[0] > 1.0), "{what}: the revealed result is above 1");
        let mut m = LayerMask::reveal_all();
        m.surface = Surface::with_default(PixelFormat { sample, ..PixelFormat::GRAY8 }, &[1.0]);
        m.surface.fill_rect(Rect::new(0, 0, 24, 40), &[value]);
        m.density = density;
        let top = d.layers.last_mut().unwrap();
        top.mask = Some(m);
        top.advanced.layer_mask_hides_effects = true;
        let coverage = 1.0 - density * (1.0 - value);
        let classify = |px: &[[f32; 4]], backend: &str| {
            for y in 0..40 {
                for x in 0..48 {
                    let red = px[(y * 48 + x) as usize][0];
                    let k = if x < 24 { coverage } else { 1.0 };
                    let ok = if k >= 1.0 {
                        red > 1.0
                    } else if k <= 0.0 {
                        (red - backdrop).abs() < 1e-3
                    } else {
                        red <= 1.0
                    };
                    assert!(ok, "{what}, {backend} at ({x},{y}), coverage {k}: red {red}");
                }
            }
        };
        classify(&photocraft_compose::render(&d, d.bounds()).px, "CPU");
        check(&mut g, &d, &what);
        classify(&check_with(&g, &mut half, &d, 3.0 / 255.0, &what), "half-float GPU");
    }
}

/// A mask that hides a layer's effects completely leaves the backdrop as it was, even for 32-bit
/// values above 1, over a canvas wider than one CPU render tile: the GPU matches the CPU, which
/// is tile-independent there.
#[test]
fn fully_hiding_mask_keeps_hdr_values_across_cpu_render_tiles() {
    let Some(mut g) = gpu() else { return };
    let w = photocraft_compose::RENDER_TILE * 2 + 88;
    let mut d = Document::new("wide", Size::new(w as u32, 64), ColorMode::Rgb, SampleType::F32);
    let mut bottom = Layer::raster("hdr", d.pixel_format());
    let px: Vec<f32> = (0..w * 64).flat_map(|_| [2.0, 1.5, 1.25, 1.0]).collect();
    bottom.surface_mut().unwrap().write_region(Rect::from_xywh(0, 0, w as u32, 64), &px);
    d.layers.push(bottom);
    let mut l = noise_layer("fx", d.pixel_format(), Rect::new(100, 16, 140, 48), 3, 0.5);
    l.effects.items.push(Effect::default_drop_shadow());
    l.mask = Some(LayerMask::hide_all());
    l.advanced.layer_mask_hides_effects = true;
    d.layers.push(l);
    check(&mut g, &d, "HDR under a fully hiding mask");
    let out = photocraft_gpu::render_to_vec(&mut g.comp, &g.device, &g.queue, &d, d.bounds()).expect("render");
    let at = |x: i32, y: i32| out[(y * w + x) as usize];
    for (x, y, what) in [(w - 20, 30, "far from the layer"), (120, 30, "under the hidden layer")] {
        assert!(at(x, y)[..3].iter().all(|&v| v > 1.0), "{what}: HDR value clamped, got {:?}", at(x, y));
    }
}

/// A fill layer whose vector mask hides its effects is laid out as the CPU lays it out, from the
/// layer without that mask: the canvas, not the revealed middle half the mask would otherwise
/// give as its frame.
#[test]
fn fills_under_a_hiding_vector_mask_are_laid_out_over_the_canvas() {
    let Some(mut g) = gpu() else { return };
    let (w, h) = (64u32, 48u32);
    let pat = checker_pattern();
    let gradient = || {
        let stops = vec![(0.0, Color::rgb(1.0, 0.0, 0.0)), (1.0, Color::rgb(0.0, 0.0, 1.0))];
        Fill::gradient(stops, 0.0, 1.0, GradientStyle::Linear, false)
    };
    let pattern = || Fill::Pattern { name: pat.name.clone(), id: pat.id.clone(), scale: 1.0, angle: 0.0, link: true, phase: (0.0, 0.0) };
    for (what, fill) in [("gradient", gradient()), ("linked pattern", pattern())] {
        let mut d = base_doc(w, h);
        d.patterns.push(pat.clone());
        let mut l = Layer::new("fill", LayerContent::Fill(fill));
        l.effects.items.push(Effect::default_drop_shadow());
        let half = [(16.0, 12.0), (48.0, 12.0), (48.0, 36.0), (16.0, 36.0)];
        l.vector_mask = Some(VectorMask::new(Path::new(vec![Subpath::polygon(&half)])));
        l.advanced.vector_mask_hides_effects = true;
        d.layers.push(l);
        check(&mut g, &d, &format!("{what} under a hiding vector mask"));
        if what == "gradient" {
            let out = photocraft_gpu::render_to_vec(&mut g.comp, &g.device, &g.queue, &d, d.bounds()).expect("render");
            let red = |x: u32| out[(24 * w + x) as usize][0];
            // Laid out over the whole canvas the revealed half spans about half the gradient; fitted
            // to it, it would span all of it.
            assert!(red(18) - red(45) < 0.75, "the gradient is fitted to the revealed region: {} to {}", red(18), red(45));
        }
    }
}
