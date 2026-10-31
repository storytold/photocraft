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

/// algo's description of a CPU filter as the GPU operator's.
fn gpu_blur(s: Separable) -> SeparableBlur {
    match s {
        Separable::Kernel(k) => SeparableBlur::Kernel(k),
        Separable::Boxes(b) => SeparableBlur::Boxes(b),
        Separable::Extreme { radius, max } => SeparableBlur::Extreme { radius, max },
        Separable::Disc { half_widths, max } => SeparableBlur::Disc { half_widths, max },
        Separable::Convolve { taps, scale, offset } => SeparableBlur::Convolve { taps, scale, offset },
    }
}

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
    let blur = gpu_blur(Separable::of(&FilterParams::GaussianBlur { radius }).unwrap());
    let selection = doc.selection.clone().map(Arc::new);
    let program = GaussianBlur::new(blur, extent, area, selection, keep_alpha, doc.depth).expect("a runnable blur");
    LayerOperator { layer, target: photocraft_gpu::OperatorTarget::Content, program: Arc::new(program) }
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

/// The operators' scratch textures go when the last operator does (they grew to the largest
/// window and stayed for the rest of the session).
#[test]
fn scratch_textures_go_with_the_last_operator() {
    let Some(mut g) = gpu() else { return };
    if !g.comp.operators_supported(&g.device) {
        return;
    }
    let (doc, id) = blur_doc(SampleType::U8);
    g.comp.set_operators(doc.id, vec![operator(&doc, id, 6.0, false)]);
    photocraft_gpu::render_to_vec_stats(&mut g.comp, &g.device, &g.queue, &doc, doc.bounds()).unwrap();
    assert!(g.comp.operator_scratch_bytes() > 0, "the passes made their scratch");
    g.comp.set_operators(doc.id, Vec::new());
    assert_eq!(g.comp.operator_scratch_bytes(), 0, "no operator left: no scratch");
}

/// Two operators on one layer would share its pages (the second taking the first's output): the
/// render refuses them, and the caller previews on the CPU.
#[test]
fn two_operators_on_one_layer_fail_the_render() {
    let Some(mut g) = gpu() else { return };
    if !g.comp.operators_supported(&g.device) {
        return;
    }
    let (doc, id) = blur_doc(SampleType::U8);
    g.comp.set_operators(doc.id, vec![operator(&doc, id, 4.0, false), operator(&doc, id, 12.0, false)]);
    let e = photocraft_gpu::render_to_vec(&mut g.comp, &g.device, &g.queue, &doc, doc.bounds()).err().map(|e| e.to_string()).unwrap_or_default();
    assert!(e.contains("two layer operators on one layer"), "{e:?}");
    g.comp.set_operators(doc.id, vec![operator(&doc, id, 4.0, false)]);
    assert!(photocraft_gpu::render_to_vec(&mut g.comp, &g.device, &g.queue, &doc, doc.bounds()).is_ok(), "one is fine");
}

/// A whole-extent operator (a distort) uploads its window once: a slider step reruns the passes
/// without uploading anything. Its window was different at the canvas's edge cells (the
/// 300 × 200 canvas doesn't end on a page), and a non-Displace remap made a new map stand-in each
/// call: both uploaded again on every step.
#[test]
fn a_whole_extent_window_is_uploaded_once() {
    let Some(mut g) = gpu() else { return };
    if !g.comp.operators_supported(&g.device) {
        return;
    }
    let (doc, id) = blur_doc(SampleType::U8);
    let content = doc.layer(id).unwrap().surface().unwrap().content_bounds();
    let twirl = |angle: f32| {
        let params = FilterParams::Twirl { angle };
        let (warp, _) = photocraft_algo::separable::Warp::of(&params, doc.bounds()).unwrap();
        let photocraft_algo::separable::Warp::Twirl { angle } = warp else { panic!("twirl") };
        let d = photocraft_gpu::RemapParams {
            mapping: photocraft_gpu::Mapping::Twirl { angle },
            bounds: doc.bounds(),
            undefined: photocraft_gpu::Undefined::Repeat,
        };
        let area = photocraft_algo::output_area(&params, content, doc.bounds(), None);
        let program = GaussianBlur::new(SeparableBlur::Remap(Arc::new(d)), edge_extent(doc.bounds(), content), area, None, false, doc.depth).unwrap();
        LayerOperator { layer: id, target: photocraft_gpu::OperatorTarget::Content, program: Arc::new(program) }
    };
    for (label, comp) in [("", &mut g.comp), (" (paged)", &mut g.paged)] {
        comp.set_operators(doc.id, vec![twirl(120.0)]);
        photocraft_gpu::render_to_vec_stats(comp, &g.device, &g.queue, &doc, doc.bounds()).unwrap();
        for angle in [121.0, 122.0] {
            comp.set_operators(doc.id, vec![twirl(angle)]);
            let step = photocraft_gpu::render_to_vec_stats(comp, &g.device, &g.queue, &doc, doc.bounds()).unwrap().1;
            assert!(step.operator_cells > 0, "{label} {angle}°: the passes run");
            assert_eq!(step.bytes_uploaded, 0, "{label} {angle}°: nothing uploaded");
        }
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
    // Per-pixel work past what a refresh should take (`MAX_AXIS_TAPS`): long kernels and wide squares.
    let taps = photocraft_gpu::MAX_AXIS_TAPS;
    assert!(GaussianBlur::new(SeparableBlur::Kernel(vec![1.0 / taps as f32; taps]), r, r, None, false, SampleType::U8).is_some(), "at the limit");
    assert!(GaussianBlur::new(SeparableBlur::Kernel(vec![0.001; taps + 2]), r, r, None, false, SampleType::U8).is_none(), "a longer kernel");
    let square = |radius: u32| GaussianBlur::new(SeparableBlur::Extreme { radius, max: true }, r, r, None, false, SampleType::U8);
    assert!(square((taps / 2) as u32).is_some() && square((taps / 2) as u32 + 1).is_none(), "squares past radius 300");
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

/// Gaussian Blur of `radius` on the mask of `layer`, as the engine applies a filter with the mask
/// targeted (`channel_surface_for_filter`): the content is the canvas and the mask's pixels, and
/// the result rounds to the mask's own depth.
fn mask_filtered(doc: &Document, layer: LayerId, radius: f32) -> (Document, Rect, Rect) {
    let mut out = doc.clone();
    let params = FilterParams::GaussianBlur { radius };
    let mask = &doc.layer(layer).unwrap().mask.as_ref().unwrap().surface;
    let content = doc.bounds().union(&mask.content_bounds());
    let sel_bounds = doc.selection.as_ref().map(Surface::content_bounds);
    let bounds = sel_bounds.filter(|b| !b.is_empty()).unwrap_or(doc.bounds());
    let area = photocraft_algo::output_area(&params, content, bounds, sel_bounds);
    let extent = edge_extent(doc.bounds(), content);
    let m = &mut out.layer_mut(layer).unwrap().mask.as_mut().unwrap().surface;
    *m = photocraft_algo::apply_in_with(m, &params, area, bounds, doc.selection.as_ref(), extent, &Interrupt::NONE).unwrap();
    (out, area, extent)
}

/// Phase 3 of #5772: a filter with the layer mask targeted runs as an operator on the mask, and
/// matches the CPU filter of the mask composited by the CPU.
#[test]
fn a_mask_blur_operator_matches_the_cpu_filter() {
    let Some(mut g) = gpu() else { return };
    if !g.comp.operators_supported(&g.device) {
        eprintln!("skipping mask operator parity: no compute");
        return;
    }
    let mut failures = Vec::new();
    for depth in [SampleType::U8, SampleType::U16] {
        for radius in [0.8, 4.0, 12.0, 40.0] {
            for (selection, hide) in [(false, true), (true, true), (false, false)] {
                let (mut doc, id) = blur_doc(depth);
                // A hard-edged mask: a revealed rectangle on a hidden canvas, or a hidden one on a
                // revealed canvas (whose default is white).
                let mut mask = if hide { photocraft_doc::LayerMask::hide_all() } else { photocraft_doc::LayerMask::reveal_all() };
                mask.surface = Surface::with_default(PixelFormat { sample: depth, ..PixelFormat::GRAY8 }, &[if hide { 0.0 } else { 1.0 }]);
                let r = Rect::new(90, 60, 210, 140);
                mask.surface.write_region(r, &vec![if hide { 1.0 } else { 0.0 }; (r.width() * r.height()) as usize]);
                doc.layer_mut(id).unwrap().mask = Some(mask);
                if selection {
                    doc.selection = Some(feathered_selection());
                }
                let what =
                    format!("mask, {depth:?} radius {radius}{}{}", if selection { ", selection" } else { "" }, if hide { "" } else { ", reveal-all mask" });
                let (expected, area, extent) = mask_filtered(&doc, id, radius);
                let blur = gpu_blur(Separable::of(&FilterParams::GaussianBlur { radius }).unwrap());
                let program = GaussianBlur::new(blur, extent, area, doc.selection.clone().map(Arc::new), false, depth).expect("a runnable blur");
                let op = LayerOperator { layer: id, target: photocraft_gpu::OperatorTarget::Mask, program: Arc::new(program) };
                if let Err(e) = compare(&mut g, &doc, &op, &expected, tolerance(depth), &what) {
                    failures.push(e);
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A mask the compositor combines with vector masks first (or a disabled one) is refused, so the
/// preview stays on the CPU rather than show the unfiltered mask.
#[test]
fn a_mask_operator_on_a_combined_or_disabled_mask_is_refused() {
    let Some(mut g) = gpu() else { return };
    if !g.comp.operators_supported(&g.device) {
        return;
    }
    let (mut doc, id) = blur_doc(SampleType::U8);
    let mut mask = photocraft_doc::LayerMask::hide_all();
    mask.enabled = false;
    doc.layer_mut(id).unwrap().mask = Some(mask);
    let (_, area, extent) = (0, doc.bounds(), doc.bounds());
    let program = GaussianBlur::new(SeparableBlur::Kernel(vec![0.25, 0.5, 0.25]), extent, area, None, false, SampleType::U8).unwrap();
    let op = LayerOperator { layer: id, target: photocraft_gpu::OperatorTarget::Mask, program: Arc::new(program) };
    g.comp.set_operators(doc.id, vec![op]);
    let e = photocraft_gpu::render_to_vec_stats(&mut g.comp, &g.device, &g.queue, &doc, doc.bounds()).unwrap_err();
    assert!(e.to_string().contains("mask operator"), "{e}");
}

/// Phase 4 of #5772: Unsharp Mask as a layer operator (the exact blur, then its combine) against
/// the filter applied on the CPU as the engine applies it, composited by the CPU. At a positive
/// threshold a last-bit difference in the blur sum can flip whether a pixel sharpens at all, so
/// a few such pixels (at most 0.1 %) may differ there; nothing else may.
#[test]
fn an_unsharp_mask_operator_matches_the_cpu_filter() {
    let Some(mut g) = gpu() else { return };
    if !g.comp.operators_supported(&g.device) {
        eprintln!("skipping unsharp mask parity: no compute");
        return;
    }
    let mut failures = Vec::new();
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for (amount, radius, threshold) in [(50.0, 1.0, 0.0), (150.0, 4.0, 0.0), (300.0, 12.0, 0.0), (80.0, 2.5, 6.0), (500.0, 40.0, 20.0)] {
            for selection in [false, true] {
                let (mut doc, id) = blur_doc(depth);
                if selection {
                    doc.selection = Some(feathered_selection());
                }
                let what = format!("{depth:?} amount {amount} radius {radius} threshold {threshold}{}", if selection { ", selection" } else { "" });
                let params = FilterParams::UnsharpMask { amount, radius, threshold };
                // As the engine applies it (`filters.rs`).
                let content = doc.layer(id).unwrap().surface().unwrap().content_bounds();
                let sel_bounds = doc.selection.as_ref().map(Surface::content_bounds);
                let bounds = sel_bounds.filter(|b| !b.is_empty()).unwrap_or(doc.bounds());
                let area = photocraft_algo::output_area(&params, content, bounds, sel_bounds);
                let extent = edge_extent(doc.bounds(), content);
                let mut expected = doc.clone();
                let surf = expected.layer_mut(id).unwrap().surface_mut().unwrap();
                *surf = photocraft_algo::apply_in_with(surf, &params, area, bounds, doc.selection.as_ref(), extent, &Interrupt::NONE).unwrap();
                let Some(Separable::Kernel(k)) = Separable::of(&params) else { panic!("{what}: the exact kernel") };
                let program = GaussianBlur::new(SeparableBlur::Kernel(k), extent, area, doc.selection.clone().map(Arc::new), false, depth)
                    .and_then(|b| b.unsharp(amount, threshold))
                    .expect("a runnable unsharp mask");
                let op = LayerOperator { layer: id, target: photocraft_gpu::OperatorTarget::Content, program: Arc::new(program) };
                let rect = doc.bounds();
                let cpu = photocraft_compose::render(&expected, rect);
                for (label, comp) in [("", &mut g.comp), (" (paged)", &mut g.paged)] {
                    comp.set_operators(doc.id, vec![op.clone()]);
                    let out = match photocraft_gpu::render_to_vec_stats(comp, &g.device, &g.queue, &doc, rect) {
                        Ok((out, _)) => out,
                        Err(e) => {
                            failures.push(format!("{what}{label}: {e}"));
                            continue;
                        }
                    };
                    let tol = tolerance(depth);
                    let off: Vec<usize> = cpu
                        .px
                        .iter()
                        .zip(&out)
                        .enumerate()
                        .filter(|(_, (c, o))| (0..4).any(|k| (c[k] * c[3] - o[k] * o[3]).abs() > tol))
                        .map(|(i, _)| i)
                        .collect();
                    let allowed = if threshold > 0.0 { cpu.px.len() / 1000 } else { 0 };
                    if off.len() > allowed {
                        let i = off[0];
                        let w = rect.width() as usize;
                        failures.push(format!(
                            "{what}{label}: {} pixels off (allowed {allowed}), first at ({}, {}): cpu {:?} gpu {:?}",
                            off.len(),
                            i % w,
                            i / w,
                            cpu.px[i],
                            out[i]
                        ));
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Phase 4: Box Blur (a uniform kernel up to radius 4, one box pass beyond) and High Pass (the
/// exact blur as `0.5 + source - blurred`) as layer operators, against the CPU filters applied
/// as the engine applies them.
#[test]
fn box_blur_and_high_pass_operators_match_the_cpu_filters() {
    let Some(mut g) = gpu() else { return };
    if !g.comp.operators_supported(&g.device) {
        eprintln!("skipping box blur / high pass parity: no compute");
        return;
    }
    let mut failures = Vec::new();
    let cases = [
        FilterParams::BoxBlur { radius: 2.0 },
        FilterParams::BoxBlur { radius: 9.0 },
        FilterParams::BoxBlur { radius: 40.0 },
        FilterParams::HighPass { radius: 1.5 },
        FilterParams::HighPass { radius: 10.0 },
        FilterParams::HighPass { radius: 60.0 },
    ];
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for params in &cases {
            for selection in [false, true] {
                let (mut doc, id) = blur_doc(depth);
                if selection {
                    doc.selection = Some(feathered_selection());
                }
                let what = format!("{depth:?} {params:?}{}", if selection { ", selection" } else { "" });
                let content = doc.layer(id).unwrap().surface().unwrap().content_bounds();
                let sel_bounds = doc.selection.as_ref().map(Surface::content_bounds);
                let bounds = sel_bounds.filter(|b| !b.is_empty()).unwrap_or(doc.bounds());
                let area = photocraft_algo::output_area(params, content, bounds, sel_bounds);
                let extent = edge_extent(doc.bounds(), content);
                let mut expected = doc.clone();
                let surf = expected.layer_mut(id).unwrap().surface_mut().unwrap();
                *surf = photocraft_algo::apply_in_with(surf, params, area, bounds, doc.selection.as_ref(), extent, &Interrupt::NONE).unwrap();
                let blur = gpu_blur(Separable::of(params).expect("a separable blur"));
                let program = GaussianBlur::new(blur, extent, area, doc.selection.clone().map(Arc::new), false, depth).expect("a runnable blur");
                let program = if matches!(params, FilterParams::HighPass { .. }) { program.high_pass() } else { program };
                let op = LayerOperator { layer: id, target: photocraft_gpu::OperatorTarget::Content, program: Arc::new(program) };
                if let Err(e) = compare(&mut g, &doc, &op, &expected, tolerance(depth), &what) {
                    failures.push(e);
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Phase 4: Minimum and Maximum (over a square and over a disc) and Custom as layer operators,
/// against the CPU filters applied as the engine applies them. They work on the straight values,
/// so a layer's transparent margin and Lock Transparency are both covered.
#[test]
fn minimum_maximum_and_custom_operators_match_the_cpu_filters() {
    use photocraft_algo::Preserve;
    let Some(mut g) = gpu() else { return };
    if !g.comp.operators_supported(&g.device) {
        eprintln!("skipping minimum / maximum / custom parity: no compute");
        return;
    }
    let mut failures = Vec::new();
    // Blur More's kernel, an edge kernel with a scale and offset, and one with a single tap.
    let blur_more = vec![0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 1.0, 0.0, 0.0, 2.0, 2.0, 2.0, 0.0, 0.0, 1.0, 2.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    let mut edges = vec![0.0; 25];
    for (i, w) in [(6, -1.0), (7, -1.0), (8, -1.0), (11, -1.0), (12, 9.0), (13, -1.0), (16, -1.0), (17, -1.0), (18, -1.0), (2, 1.0), (22, -1.0)] {
        edges[i] = w;
    }
    let mut corner = vec![0.0; 25];
    corner[24] = 1.0;
    let cases = [
        FilterParams::Minimum { radius: 1.0, preserve: Preserve::Squareness },
        FilterParams::Maximum { radius: 6.4, preserve: Preserve::Squareness },
        FilterParams::Minimum { radius: 3.0, preserve: Preserve::Roundness },
        FilterParams::Maximum { radius: 7.5, preserve: Preserve::Roundness },
        FilterParams::Maximum { radius: 0.4, preserve: Preserve::Roundness },
        // Discs whose steps run many half-widths, wider than the layer's margin.
        FilterParams::Minimum { radius: 25.0, preserve: Preserve::Roundness },
        FilterParams::Maximum { radius: 60.0, preserve: Preserve::Roundness },
        FilterParams::Custom { kernel: blur_more, scale: 14.0, offset: 0.0 },
        FilterParams::Custom { kernel: edges, scale: 2.0, offset: 30.0 },
        FilterParams::Custom { kernel: corner, scale: 0.0, offset: -10.0 },
    ];
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for params in &cases {
            for (selection, keep_alpha) in [(false, false), (true, false), (false, true)] {
                let (mut doc, id) = blur_doc(depth);
                if selection {
                    doc.selection = Some(feathered_selection());
                }
                let what = format!("{depth:?} {params:?}{}{}", if selection { ", selection" } else { "" }, if keep_alpha { ", lock transparency" } else { "" });
                let content = doc.layer(id).unwrap().surface().unwrap().content_bounds();
                let sel_bounds = doc.selection.as_ref().map(Surface::content_bounds);
                let bounds = sel_bounds.filter(|b| !b.is_empty()).unwrap_or(doc.bounds());
                let area = photocraft_algo::output_area(params, content, bounds, sel_bounds);
                let extent = edge_extent(doc.bounds(), content);
                let mut expected = doc.clone();
                let surf = expected.layer_mut(id).unwrap().surface_mut().unwrap();
                let before = surf.clone();
                let mut result = photocraft_algo::apply_in_with(surf, params, area, bounds, doc.selection.as_ref(), extent, &Interrupt::NONE).unwrap();
                if keep_alpha {
                    // As the engine keeps the old alpha (`filters.rs`).
                    let r = result.tile_bounds().union(&before.tile_bounds());
                    let (mut new, old) = (result.read_region(r), before.read_region(r));
                    let ch = before.channels();
                    for (n, o) in new.chunks_exact_mut(ch).zip(old.chunks_exact(ch)) {
                        n[ch - 1] = o[ch - 1];
                    }
                    result.write_region(r, &new);
                }
                *surf = result;
                let blur = gpu_blur(Separable::of(params).expect("a window filter"));
                let program = GaussianBlur::new(blur, extent, area, doc.selection.clone().map(Arc::new), keep_alpha, depth).expect("a runnable filter");
                let op = LayerOperator { layer: id, target: photocraft_gpu::OperatorTarget::Content, program: Arc::new(program) };
                if let Err(e) = compare(&mut g, &doc, &op, &expected, tolerance(depth), &what) {
                    failures.push(e);
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A disc past the GPU's limit, or a Custom kernel with nothing finite to divide by, is refused:
/// the preview stays on the CPU.
#[test]
fn window_filters_the_gpu_cant_run_are_refused() {
    let area = Rect::new(0, 0, 10, 10);
    let disc = |r: usize| SeparableBlur::Disc { half_widths: vec![1; 2 * r + 1], max: true };
    assert!(GaussianBlur::new(disc(photocraft_gpu::MAX_DISC_RADIUS), area, area, None, false, SampleType::U8).is_some());
    assert!(GaussianBlur::new(disc(photocraft_gpu::MAX_DISC_RADIUS + 1), area, area, None, false, SampleType::U8).is_none());
    let convolve = |scale: f32, offset: f32| SeparableBlur::Convolve { taps: vec![(0, 0, 1.0)], scale, offset };
    assert!(GaussianBlur::new(convolve(1.0, 0.0), area, area, None, false, SampleType::U8).is_some());
    for (scale, offset) in [(0.0, 0.0), (f32::NAN, 0.0), (1.0, f32::INFINITY)] {
        assert!(GaussianBlur::new(convolve(scale, offset), area, area, None, false, SampleType::U8).is_none(), "{scale} {offset}");
    }
}

/// A 37×23 straight-RGBA displacement map: smooth red and green ramps with a few hard steps, so
/// shifts vary across the layer and both the bilinear map read and the source sample matter.
fn displace_map() -> photocraft_algo::Image {
    let (w, h) = (37usize, 23usize);
    let mut data = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let r = (x as f32 / (w - 1) as f32 * 0.9 + 0.05 + if x % 9 == 4 { 0.3 } else { 0.0 }).min(1.0);
            let g = (1.0 - y as f32 / (h - 1) as f32) * 0.8 + 0.1;
            data.extend_from_slice(&[r, g, 0.5, 1.0]);
        }
    }
    photocraft_algo::Image { rect: Rect::new(0, 0, w as i32, h as i32), ch: 4, data }
}

/// Phase 4, multi-input: Displace as a layer operator (the map a second input, the source the
/// layer's whole extent) against the CPU filter applied as the engine applies it.
#[test]
fn a_displace_operator_matches_the_cpu_filter() {
    use photocraft_algo::UndefinedAreas;
    let Some(mut g) = gpu() else { return };
    if !g.comp.operators_supported(&g.device) {
        eprintln!("skipping displace parity: no compute");
        return;
    }
    let map = Arc::new(displace_map());
    let texels: Arc<Vec<[f32; 4]>> = Arc::new(map.data.as_chunks::<4>().0.to_vec());
    let mut failures = Vec::new();
    let cases = [
        (10.0, 10.0, true, UndefinedAreas::Wrap),
        (-25.0, 40.0, false, UndefinedAreas::Wrap),
        (30.0, -15.0, true, UndefinedAreas::Repeat),
        (60.0, 60.0, false, UndefinedAreas::Transparent),
    ];
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for &(horizontal, vertical, stretch, undefined) in &cases {
            for (selection, keep_alpha) in [(false, false), (true, false), (false, true)] {
                let (mut doc, id) = blur_doc(depth);
                if selection {
                    doc.selection = Some(feathered_selection());
                }
                let what = format!(
                    "{depth:?} {horizontal}/{vertical} stretch {stretch} {undefined:?}{}{}",
                    if selection { ", selection" } else { "" },
                    if keep_alpha { ", lock transparency" } else { "" }
                );
                let params = FilterParams::Displace { horizontal, vertical, stretch, undefined, map: Some(map.clone()) };
                let content = doc.layer(id).unwrap().surface().unwrap().content_bounds();
                let sel_bounds = doc.selection.as_ref().map(Surface::content_bounds);
                let bounds = sel_bounds.filter(|b| !b.is_empty()).unwrap_or(doc.bounds());
                let area = photocraft_algo::output_area(&params, content, bounds, sel_bounds);
                let extent = edge_extent(doc.bounds(), content);
                let mut expected = doc.clone();
                let surf = expected.layer_mut(id).unwrap().surface_mut().unwrap();
                let before = surf.clone();
                let mut result = photocraft_algo::apply_in_with(surf, &params, area, bounds, doc.selection.as_ref(), extent, &Interrupt::NONE).unwrap();
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
                let d = photocraft_gpu::RemapParams {
                    mapping: photocraft_gpu::Mapping::Displace {
                        map: texels.clone(),
                        map_size: [37, 23],
                        scale: [horizontal / 100.0 * 128.0, vertical / 100.0 * 128.0],
                        stretch,
                    },
                    bounds,
                    undefined: match undefined {
                        UndefinedAreas::Wrap => photocraft_gpu::Undefined::Wrap,
                        UndefinedAreas::Repeat => photocraft_gpu::Undefined::Repeat,
                        UndefinedAreas::Transparent => photocraft_gpu::Undefined::Transparent,
                    },
                };
                let program = GaussianBlur::new(SeparableBlur::Remap(Arc::new(d)), extent, area, doc.selection.clone().map(Arc::new), keep_alpha, depth)
                    .expect("a runnable displace");
                let op = LayerOperator { layer: id, target: photocraft_gpu::OperatorTarget::Content, program: Arc::new(program) };
                if let Err(e) = compare(&mut g, &doc, &op, &expected, tolerance(depth), &what) {
                    failures.push(e);
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// algo's distort mapping as the GPU operator's (as the filter preview converts it).
fn gpu_mapping(w: photocraft_algo::separable::Warp) -> photocraft_gpu::Mapping {
    use photocraft_algo::separable::Warp;
    use photocraft_gpu::Mapping;
    match w {
        Warp::Twirl { angle } => Mapping::Twirl { angle },
        Warp::Pinch { amount } => Mapping::Pinch { amount },
        Warp::Spherize { amount, mode } => Mapping::Spherize { amount, mode },
        Warp::Wave { generators, wave_type } => Mapping::Wave { generators, wave_type },
        Warp::Ripple { amplitude, wavelength } => Mapping::Ripple { amplitude, wavelength },
        Warp::Polar { to_polar } => Mapping::Polar { to_polar },
        Warp::ZigZag { amount, ridges, style } => Mapping::ZigZag { amount, ridges, style },
        Warp::Shear { rows } => Mapping::Shear { rows },
    }
}

/// Phase 4: the distort filters (Twirl, Pinch, Spherize, Wave, Ripple, Polar Coordinates, ZigZag,
/// Shear) as layer operators, against the CPU filters applied as the engine applies them. Where
/// a mapping is discontinuous (Wave's Square and Triangle, Polar Coordinates' 12 o'clock seam) a
/// last-bit difference in a sine can put a pixel on the other side, so at most 0.1 % of pixels
/// may differ there; nothing else may.
#[test]
fn distort_operators_match_the_cpu_filters() {
    use photocraft_algo::{PolarMode, RippleSize, SpherizeMode, UndefinedAreas, WaveType, ZigZagStyle};
    let Some(mut g) = gpu() else { return };
    if !g.comp.operators_supported(&g.device) {
        eprintln!("skipping distort parity: no compute");
        return;
    }
    let wave = |wave_type, undefined| FilterParams::Wave {
        generators: 5,
        wavelength_min: 10.0,
        wavelength_max: 120.0,
        amplitude_min: 5.0,
        amplitude_max: 35.0,
        wave_type,
        undefined,
        seed: 7,
    };
    let cases: Vec<(FilterParams, bool)> = vec![
        (FilterParams::Twirl { angle: 50.0 }, false),
        (FilterParams::Twirl { angle: -300.0 }, false),
        (FilterParams::Pinch { amount: 50.0 }, false),
        (FilterParams::Pinch { amount: -80.0 }, false),
        (FilterParams::Spherize { amount: 60.0, mode: SpherizeMode::Normal }, false),
        (FilterParams::Spherize { amount: -50.0, mode: SpherizeMode::Normal }, false),
        (FilterParams::Spherize { amount: 70.0, mode: SpherizeMode::HorizontalOnly }, false),
        (FilterParams::Spherize { amount: -40.0, mode: SpherizeMode::VerticalOnly }, false),
        (wave(WaveType::Sine, UndefinedAreas::Wrap), false),
        (wave(WaveType::Triangle, UndefinedAreas::Repeat), true),
        (wave(WaveType::Square, UndefinedAreas::Wrap), true),
        (FilterParams::Ripple { amount: 200.0, size: RippleSize::Medium }, false),
        (FilterParams::Ripple { amount: -500.0, size: RippleSize::Large }, false),
        (FilterParams::PolarCoordinates { mode: PolarMode::RectangularToPolar }, true),
        (FilterParams::PolarCoordinates { mode: PolarMode::PolarToRectangular }, true),
        (FilterParams::ZigZag { amount: 50.0, ridges: 5.0, style: ZigZagStyle::AroundCenter }, false),
        (FilterParams::ZigZag { amount: -70.0, ridges: 9.0, style: ZigZagStyle::OutFromCenter }, false),
        (FilterParams::ZigZag { amount: 40.0, ridges: 3.0, style: ZigZagStyle::PondRipples }, false),
        (FilterParams::Shear { points: vec![[0.0, 0.0], [0.5, 0.3], [1.0, -0.2]], undefined: UndefinedAreas::Wrap }, false),
        (FilterParams::Shear { points: vec![[0.0, -0.4], [1.0, 0.4]], undefined: UndefinedAreas::Transparent }, false),
    ];
    let mut failures = Vec::new();
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for (params, discontinuous) in &cases {
            for selection in [false, true] {
                let (mut doc, id) = blur_doc(depth);
                if selection {
                    doc.selection = Some(feathered_selection());
                }
                let what = format!("{depth:?} {params:?}{}", if selection { ", selection" } else { "" });
                let content = doc.layer(id).unwrap().surface().unwrap().content_bounds();
                let sel_bounds = doc.selection.as_ref().map(Surface::content_bounds);
                let bounds = sel_bounds.filter(|b| !b.is_empty()).unwrap_or(doc.bounds());
                let area = photocraft_algo::output_area(params, content, bounds, sel_bounds);
                let extent = edge_extent(doc.bounds(), content);
                let mut expected = doc.clone();
                let surf = expected.layer_mut(id).unwrap().surface_mut().unwrap();
                *surf = photocraft_algo::apply_in_with(surf, params, area, bounds, doc.selection.as_ref(), extent, &Interrupt::NONE).unwrap();
                let (warp, edge) = photocraft_algo::separable::Warp::of(params, bounds).expect("a distort mapping");
                let undefined = match edge {
                    photocraft_algo::Edge::Transparent => photocraft_gpu::Undefined::Transparent,
                    photocraft_algo::Edge::Repeat => photocraft_gpu::Undefined::Repeat,
                    photocraft_algo::Edge::Wrap => photocraft_gpu::Undefined::Wrap,
                };
                let d = photocraft_gpu::RemapParams { mapping: gpu_mapping(warp), bounds, undefined };
                let program = GaussianBlur::new(SeparableBlur::Remap(Arc::new(d)), extent, area, doc.selection.clone().map(Arc::new), false, depth)
                    .expect("a runnable distort");
                let op = LayerOperator { layer: id, target: photocraft_gpu::OperatorTarget::Content, program: Arc::new(program) };
                let rect = doc.bounds();
                let cpu = photocraft_compose::render(&expected, rect);
                for (label, comp) in [("", &mut g.comp), (" (paged)", &mut g.paged)] {
                    comp.set_operators(doc.id, vec![op.clone()]);
                    let out = match photocraft_gpu::render_to_vec_stats(comp, &g.device, &g.queue, &doc, rect) {
                        Ok((out, _)) => out,
                        Err(e) => {
                            failures.push(format!("{what}{label}: {e}"));
                            continue;
                        }
                    };
                    let tol = tolerance(depth);
                    let off: Vec<usize> = cpu
                        .px
                        .iter()
                        .zip(&out)
                        .enumerate()
                        .filter(|(_, (c, o))| (0..4).any(|k| (c[k] * c[3] - o[k] * o[3]).abs() > tol))
                        .map(|(i, _)| i)
                        .collect();
                    let allowed = if *discontinuous { cpu.px.len() / 1000 } else { 0 };
                    if off.len() > allowed {
                        let i = off[0];
                        let w = rect.width() as usize;
                        failures.push(format!(
                            "{what}{label}: {} pixels off (allowed {allowed}), first at ({}, {}): cpu {:?} gpu {:?}",
                            off.len(),
                            i % w,
                            i / w,
                            cpu.px[i],
                            out[i]
                        ));
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// algo's Lens Blur as the GPU operator's (as the filter preview converts it), its depth map
/// `map` for a Layer Mask or Channel depth.
fn gpu_lens(l: photocraft_algo::separable::Lens, map: Option<Arc<Surface>>) -> photocraft_gpu::LensParams {
    use photocraft_algo::DepthSource;
    use photocraft_gpu::LensDepth;
    let depth = match (l.depth, map) {
        (DepthSource::None, _) => LensDepth::None,
        (DepthSource::Transparency, _) => LensDepth::Transparency,
        (DepthSource::LayerMask | DepthSource::Channel, Some(m)) => LensDepth::Map(m),
        // No map: the CPU reads no depth either.
        (DepthSource::LayerMask | DepthSource::Channel, None) => LensDepth::None,
    };
    photocraft_gpu::LensParams {
        levels: l.levels.into_iter().map(|(rows, count)| (rows, count as f32)).collect(),
        boost: l.boost,
        threshold: l.threshold,
        depth,
        invert: l.invert,
        focal: l.focal,
        noise: l.noise,
        gaussian: l.gaussian,
        mono: l.mono,
        seed: l.seed,
    }
}

/// A depth map over the document: a diagonal ramp.
fn depth_ramp() -> Surface {
    let mut s = Surface::new(PixelFormat::GRAY8);
    let data: Vec<f32> = (0..300 * 200).map(|i| ((i % 300) as f32 / 300.0 + (i / 300) as f32 / 400.0).fract()).collect();
    s.write_region(Rect::new(0, 0, 300, 200), &data);
    s
}

/// Phase 4: Lens Blur as a layer operator, against the CPU filter applied as the engine applies
/// it: each iris shape, each depth source (the depth map as its second input), Specular
/// Highlights, and both noise distributions.
#[test]
fn lens_blur_operators_match_the_cpu_filter() {
    use photocraft_algo::{DepthSource, Distribution};
    let Some(mut g) = gpu() else { return };
    if !g.comp.operators_supported(&g.device) {
        eprintln!("skipping lens parity: no compute");
        return;
    }
    let lens = |radius: f32, blades: u32, curvature: f32, depth: DepthSource, brightness: f32, noise: f32, distribution: Distribution, mono: bool| {
        FilterParams::LensBlur {
            radius,
            blades,
            curvature,
            rotation: 20.0,
            depth,
            focal_distance: 100.0,
            invert_depth: false,
            brightness,
            threshold: 180.0,
            noise,
            distribution,
            monochromatic: mono,
            seed: 11,
            depth_map: None,
        }
    };
    let cases = vec![
        lens(15.0, 6, 0.0, DepthSource::None, 0.0, 0.0, Distribution::Uniform, false),
        lens(9.0, 3, 50.0, DepthSource::None, 0.0, 0.0, Distribution::Uniform, false),
        lens(30.0, 8, 100.0, DepthSource::None, 60.0, 0.0, Distribution::Uniform, false),
        lens(20.0, 5, 0.0, DepthSource::Transparency, 0.0, 0.0, Distribution::Uniform, false),
        lens(20.0, 6, 30.0, DepthSource::LayerMask, 30.0, 0.0, Distribution::Uniform, false),
        lens(12.0, 6, 0.0, DepthSource::Channel, 0.0, 40.0, Distribution::Uniform, false),
        lens(12.0, 4, 0.0, DepthSource::None, 0.0, 60.0, Distribution::Gaussian, true),
        lens(0.0, 6, 0.0, DepthSource::None, 0.0, 20.0, Distribution::Gaussian, false),
    ];
    let map = Arc::new(depth_ramp());
    let map_image = Arc::new(photocraft_algo::Image::read(&map, Rect::new(0, 0, 300, 200)));
    let mut failures = Vec::new();
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for case in &cases {
            for (selection, invert) in [(false, false), (true, true)] {
                let mut params = case.clone();
                if let FilterParams::LensBlur { depth_map, depth: source, invert_depth, .. } = &mut params {
                    *invert_depth = invert;
                    if matches!(source, DepthSource::LayerMask | DepthSource::Channel) {
                        *depth_map = Some(Arc::clone(&map_image));
                    }
                }
                let (mut doc, id) = blur_doc(depth);
                if selection {
                    doc.selection = Some(feathered_selection());
                }
                let what = format!("{depth:?} {params:?}{}", if selection { ", selection" } else { "" });
                let content = doc.layer(id).unwrap().surface().unwrap().content_bounds();
                let sel_bounds = doc.selection.as_ref().map(Surface::content_bounds);
                let bounds = sel_bounds.filter(|b| !b.is_empty()).unwrap_or(doc.bounds());
                let area = photocraft_algo::output_area(&params, content, bounds, sel_bounds);
                let extent = edge_extent(doc.bounds(), content);
                let mut expected = doc.clone();
                let surf = expected.layer_mut(id).unwrap().surface_mut().unwrap();
                *surf = photocraft_algo::apply_in_with(surf, &params, area, bounds, doc.selection.as_ref(), extent, &Interrupt::NONE).unwrap();
                let l = photocraft_algo::separable::Lens::of(&params).expect("lens blur");
                let program = GaussianBlur::new(
                    SeparableBlur::Lens(Arc::new(gpu_lens(l, Some(Arc::clone(&map))))),
                    extent,
                    area,
                    doc.selection.clone().map(Arc::new),
                    false,
                    depth,
                )
                .expect("a runnable lens blur");
                let op = LayerOperator { layer: id, target: photocraft_gpu::OperatorTarget::Content, program: Arc::new(program) };
                if let Err(e) = compare(&mut g, &doc, &op, &expected, tolerance(depth), &what) {
                    failures.push(e);
                }
            }
        }
    }
    // A Grayscale layer: its one channel is the luma, and takes one noise sample.
    for depth in [SampleType::U8, SampleType::F32] {
        for case in [
            lens(25.0, 6, 0.0, DepthSource::None, 70.0, 0.0, Distribution::Uniform, false),
            lens(10.0, 5, 0.0, DepthSource::Transparency, 0.0, 50.0, Distribution::Uniform, false),
            lens(10.0, 5, 0.0, DepthSource::None, 0.0, 50.0, Distribution::Gaussian, false),
        ] {
            let fmt = PixelFormat::new(ColorMode::Grayscale, depth, true);
            let mut doc = Document::new("gray", Size::new(300, 200), ColorMode::Grayscale, depth);
            doc.layers.push(noise_layer("bg", fmt, Rect::new(0, 0, 300, 200), 1, 1.0));
            let target = noise_layer("target", fmt, Rect::new(40, 30, 260, 170), 2, 0.2);
            let id = target.id;
            doc.layers.push(target);
            let what = format!("Gray {depth:?} {case:?}");
            let content = doc.layer(id).unwrap().surface().unwrap().content_bounds();
            let area = photocraft_algo::output_area(&case, content, doc.bounds(), None);
            let extent = edge_extent(doc.bounds(), content);
            let mut expected = doc.clone();
            let surf = expected.layer_mut(id).unwrap().surface_mut().unwrap();
            *surf = photocraft_algo::apply_in_with(surf, &case, area, doc.bounds(), None, extent, &Interrupt::NONE).unwrap();
            let l = photocraft_algo::separable::Lens::of(&case).expect("lens blur");
            let program = GaussianBlur::new(SeparableBlur::Lens(Arc::new(gpu_lens(l, None))), extent, area, None, false, depth).expect("a runnable lens blur");
            let op = LayerOperator { layer: id, target: photocraft_gpu::OperatorTarget::Content, program: Arc::new(program) };
            if let Err(e) = compare(&mut g, &doc, &op, &expected, tolerance(depth), &what) {
                failures.push(e);
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Phase 5: a program needing a texture past the device's size limit (a whole-extent warp on a
/// layer wider than it) falls back to the CPU without a validation error. On the web, where
/// errors arrive asynchronously, an error would take the device with it.
#[test]
fn an_operator_past_the_texture_limit_falls_back_without_an_error() {
    let _lock = crate::GPU_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let Ok(adapter) = pollster::block_on(wgpu::Instance::default().request_adapter(&wgpu::RequestAdapterOptions::default())) else { return };
    let limits = wgpu::Limits { max_texture_dimension_2d: 256, ..adapter.limits() };
    let descriptor = wgpu::DeviceDescriptor { required_limits: limits, ..Default::default() };
    let Ok((device, queue)) = pollster::block_on(adapter.request_device(&descriptor)) else { return };
    assert_eq!(device.limits().max_texture_dimension_2d, 256);
    let Ok(mut comp) = Compositor::try_new_with_format(&device, wgpu::TextureFormat::Rgba32Float) else { return };
    if !comp.operators_supported(&device) {
        return;
    }
    // 300 px wide: Twirl's window (the layer's whole extent) can't be one texture.
    let (doc, id) = blur_doc(SampleType::U8);
    let content = doc.layer(id).unwrap().surface().unwrap().content_bounds();
    let params = FilterParams::Twirl { angle: 90.0 };
    let (warp, _) = photocraft_algo::separable::Warp::of(&params, doc.bounds()).unwrap();
    let mapping = match warp {
        photocraft_algo::separable::Warp::Twirl { angle } => photocraft_gpu::Mapping::Twirl { angle },
        _ => unreachable!(),
    };
    let d = photocraft_gpu::RemapParams { mapping, bounds: doc.bounds(), undefined: photocraft_gpu::Undefined::Repeat };
    let area = photocraft_algo::output_area(&params, content, doc.bounds(), None);
    let program = GaussianBlur::new(SeparableBlur::Remap(Arc::new(d)), edge_extent(doc.bounds(), content), area, None, false, doc.depth).unwrap();
    comp.set_operators(doc.id, vec![LayerOperator { layer: id, target: photocraft_gpu::OperatorTarget::Content, program: Arc::new(program) }]);
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let result = photocraft_gpu::render_to_vec_stats(&mut comp, &device, &queue, &doc, doc.bounds());
    let error = pollster::block_on(scope.pop());
    assert!(error.is_none(), "no validation error: {error:?}");
    let message = result.err().map(|e| e.to_string()).unwrap_or_default();
    assert!(message.contains("256 px limit"), "the refresh falls back to the CPU, saying why: {message:?}");
}

/// Closing the last document with operators frees their scratch textures (they were kept for the
/// rest of the session).
#[test]
fn scratch_textures_go_with_the_last_operator_document() {
    let Some(mut g) = gpu() else { return };
    if !g.comp.operators_supported(&g.device) {
        return;
    }
    let (doc, id) = blur_doc(SampleType::U8);
    g.comp.set_operators(doc.id, vec![operator(&doc, id, 6.0, false)]);
    photocraft_gpu::render_to_vec_stats(&mut g.comp, &g.device, &g.queue, &doc, doc.bounds()).unwrap();
    assert!(g.comp.operator_scratch_bytes() > 0, "the passes made their scratch");
    g.comp.forget_doc(doc.id);
    assert_eq!(g.comp.operator_scratch_bytes(), 0, "the last operator document closed");
}
