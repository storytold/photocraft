//! Layer operators (#5772): a Gaussian Blur run by the compositor on a layer's pixels matches
//! the filter applied on the CPU (`photocraft_algo::apply_in_with`, as the engine applies it),
//! composited by the CPU compositor.

use std::sync::Arc;

use photocraft_algo::FilterParams;
use photocraft_algo::separable::{Separable, edge_extent};
use photocraft_doc::LayerId;
use photocraft_gpu::{GaussianBlur, LayerOperator, SeparableBlur};
use photocraft_raster::{Interrupt, Surface};

use super::*;

/// Gaussian Blur of `radius` on `layer` of `doc`, as the engine applies it (`filters.rs`): the
/// output area, the edge extent, the selection's mix and Lock Transparency.
fn filtered(doc: &Document, layer: LayerId, radius: f32, keep_alpha: bool) -> Document {
    let mut out = doc.clone();
    let params = FilterParams::GaussianBlur { radius };
    let (area, extent, bounds) = geometry(doc, layer, radius);
    let selection = doc.selection.clone();
    let l = out.layer_mut(layer).unwrap();
    let surf = l.surface_mut().unwrap();
    let before = surf.clone();
    let mut result = photocraft_algo::apply_in_with(surf, &params, area, bounds, selection.as_ref(), extent, &Interrupt::NONE).unwrap();
    if keep_alpha {
        let r = result.tile_bounds().union(&before.tile_bounds());
        let (mut new, old) = (result.read_region(r), before.read_region(r));
        let ch = before.channels();
        for (n, o) in new.chunks_exact_mut(ch).zip(old.chunks_exact(ch)) {
            n[ch - 1] = o[ch - 1];
        }
        result.write_region(r, &new);
    }
    *surf = result;
    out
}

/// The filter's output area, edge extent and reference bounds for `layer` of `doc`.
fn geometry(doc: &Document, layer: LayerId, radius: f32) -> (Rect, Rect, Rect) {
    let params = FilterParams::GaussianBlur { radius };
    let content = doc.layer(layer).unwrap().surface().unwrap().content_bounds();
    let sel_bounds = doc.selection.as_ref().map(Surface::content_bounds);
    let bounds = sel_bounds.filter(|b| !b.is_empty()).unwrap_or(doc.bounds());
    let area = photocraft_algo::output_area(&params, content, bounds, sel_bounds);
    (area, edge_extent(doc.bounds(), content), bounds)
}

/// The same blur as a layer operator.
fn operator(doc: &Document, layer: LayerId, radius: f32, keep_alpha: bool) -> LayerOperator {
    let (area, extent, _) = geometry(doc, layer, radius);
    let blur = match Separable::of(&FilterParams::GaussianBlur { radius }).unwrap() {
        Separable::Kernel(k) => SeparableBlur::Kernel(k),
        Separable::Boxes(b) => SeparableBlur::Boxes(b),
    };
    let selection = doc.selection.clone().map(Arc::new);
    let program = GaussianBlur::new(blur, extent, area, selection, keep_alpha, doc.depth).expect("a runnable blur");
    LayerOperator { layer, program: Arc::new(program) }
}

/// The GPU render of `doc` with `op` (whole textures, then pages) against the CPU render of
/// `expected`: Err with the worst pixel past `tol`.
fn compare(g: &mut Gpu, doc: &Document, op: &LayerOperator, expected: &Document, tol: f32, what: &str) -> Result<(), String> {
    let rect = doc.bounds();
    let cpu = photocraft_compose::render(expected, rect);
    for (label, comp) in [("", &mut g.comp), (" (paged)", &mut g.paged)] {
        comp.set_operators(doc.id, vec![op.clone()]);
        let (out, stats) = photocraft_gpu::render_to_vec_stats(comp, &g.device, &g.queue, doc, rect).map_err(|e| format!("{what}{label}: {e}"))?;
        let worst = worst_diff(&cpu.px, &out);
        let w = rect.width() as usize;
        let (x, y) = (rect.x0 + (worst.1 % w) as i32, rect.y0 + (worst.1 / w) as i32);
        if worst.0 > tol {
            return Err(format!("{what}{label}: max diff {:.3}/255 at ({x},{y}): cpu {:?} gpu {:?}", worst.0 * 255.0, cpu.px[worst.1], out[worst.1]));
        }
        assert!(stats.cells > 0, "{what}{label}: rendered");
    }
    Ok(())
}

/// A 300×200 document: a noise background, a translucent noise layer (the target) that leaves
/// a margin of transparency, and a Multiply layer above part of it.
fn blur_doc(depth: SampleType) -> (Document, LayerId) {
    let fmt = PixelFormat { sample: depth, ..PixelFormat::RGBA8 };
    let mut d = Document::new("blur", Size::new(300, 200), ColorMode::Rgb, depth);
    d.layers.push(noise_layer("bg", fmt, Rect::new(0, 0, 300, 200), 1, 1.0));
    let target = noise_layer("target", fmt, Rect::new(40, 30, 260, 170), 2, 0.2);
    let id = target.id;
    d.layers.push(target);
    let mut above = noise_layer("above", fmt, Rect::new(150, 0, 300, 90), 3, 0.5);
    above.blend = photocraft_color::BlendMode::Multiply;
    d.layers.push(alone(above));
    (d, id)
}

/// A feathered selection: a soft-edged ellipse over the right half of the target.
fn feathered_selection() -> Surface {
    let mut s = Surface::new(PixelFormat::GRAY8);
    let data: Vec<f32> = (0..300 * 200)
        .map(|i| {
            let (x, y) = ((i % 300) as f32, (i / 300) as f32);
            let r = (((x - 200.0) / 70.0).powi(2) + ((y - 100.0) / 60.0).powi(2)).sqrt();
            (1.5 - r).clamp(0.0, 1.0)
        })
        .collect();
    s.write_region(Rect::new(0, 0, 300, 200), &data);
    s
}

fn tolerance(depth: SampleType) -> f32 {
    match depth {
        // One level, plus the float slack of the other parity tests.
        SampleType::U8 => TOL,
        // Results round to 16 bits on both sides; pages of the layers around are half floats.
        SampleType::U16 => 1.0 / 255.0,
        SampleType::F32 => 1.0 / 255.0,
    }
}

#[test]
fn a_blur_operator_matches_the_cpu_filter() {
    let Some(mut g) = gpu() else { return };
    if !g.comp.operators_supported(&g.device) {
        eprintln!("skipping operator parity: no compute");
        return;
    }
    let mut failures = Vec::new();
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for radius in [0.3, 0.8, 2.0, 4.0, 4.5, 12.0, 40.0, 250.0] {
            for (selection, keep_alpha) in [(false, false), (true, false), (false, true)] {
                let (mut doc, id) = blur_doc(depth);
                if selection {
                    doc.selection = Some(feathered_selection());
                }
                let what =
                    format!("{depth:?} radius {radius}{}{}", if selection { ", selection" } else { "" }, if keep_alpha { ", lock transparency" } else { "" });
                let expected = filtered(&doc, id, radius, keep_alpha);
                if let Err(e) = compare(&mut g, &doc, &operator(&doc, id, radius, keep_alpha), &expected, tolerance(depth), &what) {
                    failures.push(e);
                }
            }
        }
    }
    assert!(failures.is_empty(), "{} failures:\n{}", failures.len(), failures.join("\n"));
}

/// The target inside a group: the group composites over the blur's halo, not the layer's own
/// bounds.
#[test]
fn a_blur_inside_a_group_reaches_past_the_layers_pixels() {
    let Some(mut g) = gpu() else { return };
    if !g.comp.operators_supported(&g.device) {
        return;
    }
    let (mut doc, id) = blur_doc(SampleType::U8);
    // A small target in a group, far from the canvas edges.
    let small = noise_layer("small", PixelFormat::RGBA8, Rect::new(120, 80, 150, 110), 5, 1.0);
    let id_small = small.id;
    doc.layers.push(Layer::group("group", vec![small]));
    let _ = id;
    let expected = filtered(&doc, id_small, 12.0, false);
    compare(&mut g, &doc, &operator(&doc, id_small, 12.0, false), &expected, TOL, "group").unwrap_or_else(|e| panic!("{e}"));
}

/// Changing the operator (a dialog's slider), clearing it, and the stack-prefix cache: every
/// render shows the current settings, never pixels cached with others.
#[test]
fn renders_follow_the_operator_and_never_reuse_stale_prefixes() {
    let Some(mut g) = gpu() else { return };
    if !g.comp.operators_supported(&g.device) {
        return;
    }
    let (doc, id) = blur_doc(SampleType::U8);
    for radius in [2.0, 12.0, 4.5, 2.0] {
        let expected = filtered(&doc, id, radius, false);
        compare(&mut g, &doc, &operator(&doc, id, radius, false), &expected, TOL, &format!("radius {radius}")).unwrap_or_else(|e| panic!("{e}"));
    }
    for comp in [&mut g.comp, &mut g.paged] {
        comp.clear_operators(doc.id);
    }
    check(&mut g, &doc, "cleared");
}

/// The output is kept while nothing changes: a second render of the same view runs no passes,
/// a changed slider does; and the blur really changes the image (the parity isn't vacuous).
#[test]
fn outputs_are_reused_until_the_operator_changes() {
    let Some(mut g) = gpu() else { return };
    if !g.comp.operators_supported(&g.device) {
        return;
    }
    let (doc, id) = blur_doc(SampleType::U8);
    let unfiltered = photocraft_compose::render(&doc, doc.bounds());
    let blurred = photocraft_compose::render(&filtered(&doc, id, 4.0, false), doc.bounds());
    assert!(worst_diff(&unfiltered.px, &blurred.px).0 > 0.1, "the blur changes the image");
    let render = |g: &mut Gpu, radius: f32| {
        g.comp.set_operators(doc.id, vec![operator(&doc, id, radius, false)]);
        photocraft_gpu::render_to_vec_stats(&mut g.comp, &g.device, &g.queue, &doc, doc.bounds()).unwrap().1
    };
    assert!(render(&mut g, 4.0).operator_cells > 0, "first render runs the passes");
    assert_eq!(render(&mut g, 4.0).operator_cells, 0, "the same operator again: kept");
    assert!(render(&mut g, 12.0).operator_cells > 0, "a new radius runs them again");
    // A drag whose reach crosses a margin step keeps the windows: no upload, only the passes. (On
    // the other compositor, whose windows are first made at radius 40.)
    let full = |g: &mut Gpu, radius: f32| {
        g.paged.set_operators(doc.id, vec![operator(&doc, id, radius, false)]);
        photocraft_gpu::render_to_vec_stats(&mut g.paged, &g.device, &g.queue, &doc, doc.bounds()).unwrap().1
    };
    full(&mut g, 40.0);
    for radius in [44.0, 48.0, 36.0, 40.0] {
        let stats = full(&mut g, radius);
        assert!(stats.operator_cells > 0, "radius {radius}: the passes run");
        assert_eq!(stats.bytes_uploaded, 0, "radius {radius}: the windows are kept");
    }
}

#[test]
fn blur_descriptions_the_passes_cant_run_are_refused() {
    let r = Rect::new(0, 0, 10, 10);
    assert!(GaussianBlur::new(SeparableBlur::Kernel(vec![0.5, 0.5]), r, r, None, false, SampleType::U8).is_none(), "even kernel");
    assert!(GaussianBlur::new(SeparableBlur::Kernel(vec![f32::NAN]), r, r, None, false, SampleType::U8).is_none(), "NaN weight");
    assert!(GaussianBlur::new(SeparableBlur::Boxes(vec![]), r, r, None, false, SampleType::U8).is_none(), "no boxes");
    assert!(GaussianBlur::new(SeparableBlur::Boxes(vec![4]), r, r, None, false, SampleType::U8).is_none(), "even box");
    assert!(GaussianBlur::new(SeparableBlur::Boxes(vec![4001]), r, r, None, false, SampleType::U8).is_none(), "reach past the limit");
    assert!(GaussianBlur::new(SeparableBlur::Kernel(vec![1.0]), Rect::EMPTY, r, None, false, SampleType::U8).is_none(), "no extent");
    assert!(GaussianBlur::new(SeparableBlur::Boxes(vec![5, 5, 7]), r, r, None, false, SampleType::U8).is_some());
}

/// What operators can't draw exactly fails the render (the caller previews on the CPU): a
/// document whose filter runs on other channels than the page texels' RGB, and an operator layer
/// under layer effects (its own, or a group's), whose shapes come from the unblurred pixels.
#[test]
fn operators_the_gpu_cant_draw_exactly_fail_the_render() {
    let Some(mut g) = gpu() else { return };
    if !g.comp.operators_supported(&g.device) {
        return;
    }
    let render = |g: &mut Gpu, doc: &Document, op: LayerOperator| {
        g.comp.set_operators(doc.id, vec![op]);
        photocraft_gpu::render_to_vec(&mut g.comp, &g.device, &g.queue, doc, doc.bounds())
    };
    // Lab.
    let (mut lab, id) = blur_doc(SampleType::U8);
    lab.mode = ColorMode::Lab;
    let op = operator(&lab, id, 4.0, false);
    assert!(render(&mut g, &lab, op).is_err(), "Lab");
    // Effects on the layer.
    let (mut doc, id) = blur_doc(SampleType::U8);
    let op = operator(&doc, id, 4.0, false);
    doc.layer_mut(id).unwrap().effects.items =
        vec![photocraft_doc::Effect::Stroke(stroke(3.0, photocraft_doc::StrokePosition::Outside, photocraft_doc::FxPaint::Color(Color::rgb(1.0, 0.0, 0.0))))];
    assert!(render(&mut g, &doc, op.clone()).is_err(), "effects on the layer");
    // Effects on a group holding it.
    let (doc, _) = blur_doc(SampleType::U8);
    let small = noise_layer("small", PixelFormat::RGBA8, Rect::new(120, 80, 150, 110), 5, 1.0);
    let id = small.id;
    let mut group = Layer::group("group", vec![small]);
    group.effects.items =
        vec![photocraft_doc::Effect::Stroke(stroke(3.0, photocraft_doc::StrokePosition::Outside, photocraft_doc::FxPaint::Color(Color::rgb(1.0, 0.0, 0.0))))];
    let mut doc = doc;
    doc.layers.push(group);
    let op = operator(&doc, id, 4.0, false);
    assert!(render(&mut g, &doc, op).is_err(), "effects on a group");
    // Without the operator the same document draws.
    g.comp.clear_operators(doc.id);
    assert!(photocraft_gpu::render_to_vec(&mut g.comp, &g.device, &g.queue, &doc, doc.bounds()).is_ok());
}

/// Refreshes of part of the canvas (a view, then the view panned and back) blur only what they
/// draw, and each matches the CPU filter there, on whole textures and on 256 px pages.
#[test]
fn partial_refreshes_match_as_the_view_moves() {
    let Some(mut g) = gpu() else { return };
    if !g.comp.operators_supported(&g.device) {
        return;
    }
    let (mut doc, id) = blur_doc(SampleType::U8);
    doc.selection = Some(feathered_selection());
    for radius in [2.0, 12.0, 40.0] {
        let expected = filtered(&doc, id, radius, false);
        let op = operator(&doc, id, radius, false);
        for (label, comp) in [("", &mut g.comp), (" (paged)", &mut g.paged)] {
            comp.set_operators(doc.id, vec![op.clone()]);
            for rect in [Rect::new(60, 40, 180, 120), Rect::new(170, 90, 300, 200), Rect::new(0, 0, 90, 70), Rect::new(60, 40, 180, 120)] {
                let cpu = photocraft_compose::render(&expected, rect);
                let out = photocraft_gpu::render_to_vec(comp, &g.device, &g.queue, &doc, rect).unwrap();
                let worst = worst_diff(&cpu.px, &out);
                assert!(worst.0 <= TOL, "radius {radius}{label} over {rect:?}: max diff {:.3}/255", worst.0 * 255.0);
            }
        }
    }
}

/// A document nested past the model's group limit can't hide an operator from the bounded
/// lookups: the render fails (the CPU previews it).
#[test]
fn operators_in_over_deep_documents_fail_the_render() {
    let Some(mut g) = gpu() else { return };
    if !g.comp.operators_supported(&g.device) {
        return;
    }
    let (mut doc, id) = blur_doc(SampleType::U8);
    let op = operator(&doc, id, 2.0, false);
    let mut nest = Layer::group("deep", Vec::new());
    for _ in 0..photocraft_doc::MAX_GROUP_DEPTH + 1 {
        nest = Layer::group("deep", vec![nest]);
    }
    doc.layers.push(nest);
    g.comp.set_operators(doc.id, vec![op]);
    assert!(photocraft_gpu::render_to_vec(&mut g.comp, &g.device, &g.queue, &doc, doc.bounds()).is_err());
}
