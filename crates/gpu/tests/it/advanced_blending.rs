//! Advanced Blending knockout on the GPU: shallow and deep knockouts (top level, isolated and
//! pass-through groups, clipping groups) must match the CPU compositor within the parity
//! tolerance, and the document must render on the GPU (a refusal fails the check).

use photocraft_color::{BlendMode, Color, ColorMode, SampleType};
use photocraft_doc::{Adjustment, BlendRange, Document, Effect, Fill, Knockout, Layer, LayerContent};
use photocraft_geom::{Rect, Size};

use crate::parity::{Gpu, base_doc, check, diff_rect, gpu, mask, noise_layer};

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
            let st = photocraft_gpu::render_to_vec_stats(&mut g.comp, &g.device, &g.queue, &d, d.bounds());
            assert!(st.is_ok(), "planned on the GPU");
            run(&mut g, &d, &format!("stroked shape, fill opacity {fill}, clipped layer"), knockout);
        }
    }
}

/// The options that stay on the CPU still make the GPU refuse the document.
#[test]
fn the_remaining_advanced_blending_options_are_refused() {
    let Some(mut g) = gpu() else { return };
    let mut d = base_doc(48, 40);
    let mut l = ko_layer(&d, 3, Knockout::Deep);
    l.advanced.transparency_shapes = false;
    l.blend = BlendMode::ColorDodge;
    d.layers.push(l);
    let e = photocraft_gpu::render_to_vec_stats(&mut g.comp, &g.device, &g.queue, &d, d.bounds())
        .expect_err("the GPU must refuse a special-eight blend without transparency shapes");
    assert!(e.to_string().contains("Transparency Shapes Layer off with a Color Dodge"), "{e}");
}
