//! The Select and Mask preview document: how a refined selection is shown in the view a
//! [`MaskViewRecipe`] names.
//!
//! The preview runs `select.refineEdge` on the proxy document with `output: selection` (or builds
//! the Show Edge band or the original selection), then builds the view from that proxy: veil fill
//! layers, a layer mask, and the coverage as the selection for the ants. The view state and its
//! keys belong to the framework's `MaskViewState`; this file only turns its recipe into pixels.

use std::sync::Arc;

use craft::imaging::ui::mask_view::{MaskCoverage, MaskViewRecipe, VeilMask};
use photocraft_color::{Color, PixelFormat};
use photocraft_doc::{Document, Fill, Layer, LayerContent, LayerId, LayerMask};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Value, json};

/// The engine command behind Select › Select and Mask….
pub const COMMAND: &str = "select.refineEdge";

/// Sparse float coverage; quantize only when building a display mask.
struct Coverage {
    values: Surface,
    outside: f32,
}

impl Coverage {
    /// Creates an empty coverage buffer with zero outside default.
    fn empty() -> Coverage {
        Coverage { values: Surface::new(PixelFormat::GRAY8.with_sample(photocraft_color::SampleType::F32)), outside: 0.0 }
    }

    /// Reads sparse mask coverage from `s` within `canvas`, preserving shared tile buffers.
    fn read(s: Option<&Surface>, canvas: Rect) -> Coverage {
        let Some(s) = s else { return Coverage::empty() };
        let outside = s.default_pixel().first().copied().unwrap_or(0.0).clamp(0.0, 1.0);
        let mut out = Coverage { values: Surface::with_default(PixelFormat::GRAY8.with_sample(photocraft_color::SampleType::F32), &[outside]), outside };
        let mut converted = std::collections::HashMap::new();
        for (tc, tile) in s.tiles() {
            let area = tc.rect().intersect(&canvas);
            if area.is_empty() {
                continue;
            }
            let key = Arc::as_ptr(tile) as usize;
            if area == tc.rect()
                && let Some(shared) = converted.get(&key)
            {
                out.values.put_tiles([(*tc, Arc::clone(shared))]);
                continue;
            }
            let data = photocraft_algo::selection::mask_from_surface(Some(s), area);
            out.values.write_region(area, &data);
            if area == tc.rect()
                && let Some(shared) = out.values.tile(*tc)
            {
                converted.insert(key, shared.clone());
            }
        }
        out
    }

    /// Computes the morphological edge band of `s` within `canvas` with radius `r`.
    fn edge_band(s: Option<&Surface>, canvas: Rect, r: usize) -> Coverage {
        let Some(s) = s else { return Coverage::empty() };
        let reach = i32::try_from(r).unwrap_or(i32::MAX).saturating_add(1);
        let outside = s.default_pixel().first().copied().unwrap_or(0.0);
        let area = if outside == 0.0 { s.content_bounds().inflate(reach).intersect(&canvas) } else { canvas };
        let mut candidates = std::collections::BTreeSet::new();
        for (tc, _) in s.tiles() {
            candidates.extend(tc.rect().inflate(reach).intersect(&area).tiles());
        }
        let mut out = Coverage::empty();
        for tc in candidates {
            let core = tc.rect().intersect(&area);
            let input = core.inflate(reach).intersect(&area);
            let (w, h) = (input.width() as usize, input.height() as usize);
            let values = photocraft_algo::selection::mask_from_surface(Some(s), input);
            let grown = photocraft_algo::matting::morph(&values, w, h, r, true);
            let shrunk = photocraft_algo::matting::morph(&values, w, h, r, false);
            for y in core.y0..core.y1 {
                for x in core.x0..core.x1 {
                    let i = (y - input.y0) as usize * w + (x - input.x0) as usize;
                    if let (Some(g), Some(s)) = (grown.get(i), shrunk.get(i)) {
                        let v = (g - s).clamp(0.0, 1.0);
                        if v != 0.0 {
                            out.values.write_pixel(x, y, &[v]);
                        }
                    }
                }
            }
        }
        out
    }

    /// Returns the coverage value at canvas coordinate `(x, y)`.
    fn at(&self, x: i32, y: i32) -> f32 {
        self.values.sample_channel(x, y, 0)
    }

    /// Blends painted brush additions and restorations into the coverage buffer.
    fn with_brush(mut self, brush: Option<&photocraft_algo::matting::EdgeBrush>, canvas: Rect) -> Coverage {
        let Some(brush) = brush else { return self };
        for tc in brush.tile_coords() {
            let area = tc.rect().intersect(&canvas);
            for y in area.y0..area.y1 {
                for x in area.x0..area.x1 {
                    let b = brush.at(x, y);
                    if b == 0.0 {
                        continue;
                    }
                    let v = self.at(x, y);
                    let value = if b >= 0.0 { v.max(b) } else { v * (1.0 + b) };
                    self.values.write_pixel(x, y, &[value]);
                }
            }
        }
        self
    }

    /// Converts float coverage to an 8-bit GRAY8 surface using mapping function `f`.
    fn surface(&self, f: impl Fn(f32) -> f32) -> Surface {
        let g = |v: f32| f(v).clamp(0.0, 1.0);
        let mut s = Surface::with_default(PixelFormat::GRAY8, &[g(self.outside)]);
        let default = s.default_bytes().first().copied().unwrap_or(0);
        let mut converted: std::collections::HashMap<usize, Option<Arc<photocraft_raster::Tile>>> = std::collections::HashMap::new();
        for (tc, tile) in self.values.tiles() {
            let key = Arc::as_ptr(tile) as usize;
            if let Some(shared) = converted.get(&key) {
                if let Some(shared) = shared {
                    s.put_tiles([(*tc, shared.clone())]);
                }
                continue;
            }
            let bytes: Vec<u8> = self.values.read_region(tc.rect()).iter().map(|v| (g(*v) * 255.0).round() as u8).collect();
            if bytes.iter().all(|v| *v == default) {
                converted.insert(key, None);
            } else {
                s.write_interleaved(tc.rect(), &bytes);
                converted.insert(key, s.tile(*tc).cloned());
            }
        }
        s
    }
}

/// A pixel length param (`radius`, `feather`) of `params`, reduced by `k`.
fn length(params: &Value, key: &str, k: u32) -> f64 {
    params.get(key).and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(0.0).max(0.0) / f64::from(k.max(1))
}

/// The preview of Select and Mask on `proxy`, the document already reduced by `k` (the workspace
/// builds it itself, its selection already carrying the workspace's tool edits), shown as `recipe`
/// says. `params` are the `select.refineEdge` parameters. `None` when the refinement fails or is
/// cancelled.
pub fn preview_proxy(
    mut proxy: Document,
    active: Option<LayerId>,
    params: &Value,
    recipe: &MaskViewRecipe,
    k: u32,
    cancel: Option<&photocraft_engine::jobs::JobCtx>,
) -> Option<Document> {
    // X held: the image without a view.
    if recipe.hidden {
        return Some(proxy);
    }
    let k = k.max(1);
    let active = active.filter(|id| proxy.layer(*id).is_some());
    // What Select and Mask starts from: the selection, else the active layer's mask.
    let original = proxy.selection.clone().or_else(|| active.and_then(|id| proxy.layer(id)).and_then(|l| l.mask.as_ref()).map(|m| m.surface.clone()));
    let canvas = Rect::new(0, 0, i32::try_from(proxy.size.width).unwrap_or(i32::MAX), i32::try_from(proxy.size.height).unwrap_or(i32::MAX));
    let coverage = match recipe.coverage {
        MaskCoverage::EdgeBand => {
            let band = Coverage::edge_band(original.as_ref(), canvas, length(params, "radius", k).round().clamp(1.0, 250.0) as usize);
            // With nothing selected OK refines nothing (`matting::refine_area`), so the brush shows nothing either.
            let selects = original.as_ref().is_some_and(|s| !s.content_bounds().intersect(&canvas).is_empty());
            let brush = if selects { scaled_strokes(params, k).and_then(|s| photocraft_algo::matting::EdgeBrush::new(&s, canvas)) } else { None };
            band.with_brush(brush.as_ref(), canvas)
        }
        MaskCoverage::Original => Coverage::read(original.as_ref(), canvas),
        // The refined selection; a coverage the framework adds later shows it too.
        _ => Coverage::read(refine(&proxy, active, params, k, cancel)?.as_ref(), canvas),
    };
    show(&mut proxy, active, recipe, &coverage);
    Some(proxy)
}

/// `select.refineEdge` on the proxy, its output forced to the selection: the refined coverage.
fn refine(proxy: &Document, active: Option<LayerId>, params: &Value, k: u32, cancel: Option<&photocraft_engine::jobs::JobCtx>) -> Option<Option<Surface>> {
    let mut s = photocraft_engine::Session::new();
    s.set_inline_job_ctx(cancel.cloned());
    s.add_document(proxy.clone(), None);
    if let Some(id) = active {
        s.select_layer(id).ok()?;
    }
    let mut p = params.as_object().cloned().unwrap_or_default();
    p.insert("output".into(), json!("selection"));
    p.insert("decontaminate".into(), json!(false));
    for key in ["radius", "feather"] {
        if p.contains_key(key) {
            p.insert(key.into(), json!(length(params, key, k)));
        }
    }
    if let Some(strokes) = p.get("refineStrokes").and_then(Value::as_array).cloned() {
        let k = f64::from(k.max(1));
        let scale = |v: &Value| v.as_f64().map_or(v.clone(), |n| json!(n / k));
        let scaled: Vec<Value> = strokes
            .into_iter()
            .map(|mut s| {
                if let Some(o) = s.as_object_mut() {
                    if let Some(size) = o.get("size").and_then(Value::as_f64) {
                        o.insert("size".into(), json!((size / k).max(1.0)));
                    }
                    if let Some(pts) = o.get_mut("points").and_then(Value::as_array_mut) {
                        for pt in pts.iter_mut() {
                            if let Some(xy) = pt.as_array_mut() {
                                xy.iter_mut().for_each(|v| *v = scale(v));
                            }
                        }
                    }
                }
                s
            })
            .collect();
        p.insert("refineStrokes".into(), Value::Array(scaled));
    }
    s.execute(COMMAND, Value::Object(p)).ok()?;
    Some(s.active()?.doc.selection.clone())
}

/// The Refine Edge Brush strokes of `params`, reduced by `k`, for Show Edge; `None` when there
/// are none (or they are malformed: the refinement itself reports those).
fn scaled_strokes(params: &Value, k: u32) -> Option<Vec<photocraft_algo::matting::EdgeStroke>> {
    let k = k.max(1) as f32;
    params
        .get("refineStrokes")?
        .as_array()?
        .iter()
        .map(|s| {
            let points = s
                .get("points")?
                .as_array()?
                .iter()
                .map(|p| Some((p.get(0)?.as_f64()? as f32 / k, p.get(1)?.as_f64()? as f32 / k)))
                .collect::<Option<Vec<_>>>()?;
            let size = (s.get("size").and_then(Value::as_f64).unwrap_or(30.0) as f32 / k).max(1.0);
            Some(photocraft_algo::matting::EdgeStroke { points, size, add: s.get("mode").and_then(Value::as_str) != Some("subtract") })
        })
        .collect()
}

/// A solid fill layer named for the view.
fn veil(name: &str, color: Color, opacity: f32, mask: Option<Surface>) -> Layer {
    let mut l = Layer::new(name, LayerContent::Fill(Fill::Solid(color)));
    l.opacity = opacity;
    l.mask = mask.map(|surface| LayerMask { surface, ..LayerMask::reveal_all() });
    l
}

/// Turn `doc` into the view `recipe` names of `coverage`.
fn show(doc: &mut Document, active: Option<LayerId>, recipe: &MaskViewRecipe, coverage: &Coverage) {
    doc.selection = Some(coverage.surface(|v| v));
    for v in &recipe.veils {
        let mask = match v.mask {
            VeilMask::Everywhere => None,
            VeilMask::Coverage => Some(coverage.surface(|c| c)),
            // The unselected area, and any mask the framework adds later: the veil covers the masked part.
            _ => Some(coverage.surface(|c| 1.0 - c)),
        };
        doc.layers.push(veil("Select and Mask view", Color::rgb(v.color.r, v.color.g, v.color.b), v.opacity * v.color.a, mask));
    }
    if recipe.layer_floor.is_none() && !recipe.hide_other_layers {
        return;
    }
    let Some(id) = active else { return };
    if recipe.hide_other_layers
        && let Some(path) = doc.path_of(id)
    {
        // Only the layer being masked (with its groups and contents) stays visible.
        let others: Vec<LayerId> = doc.walk().into_iter().filter(|(p, _, _)| !path.starts_with(p) && !p.starts_with(&path)).map(|(_, _, l)| l.id).collect();
        for other in others {
            if let Some(l) = doc.layer_mut(other) {
                l.visible = false;
            }
        }
    }
    if let Some(floor) = recipe.layer_floor {
        let surface = coverage.surface(|v| v + (1.0 - v) * floor);
        if let Some(l) = doc.layer_mut(id) {
            l.mask = Some(LayerMask { surface, ..LayerMask::reveal_all() });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft::imaging::selection::ColorIndicates;
    use craft::imaging::ui::mask_view::{MaskViewChange, MaskViewMode, MaskViewState};
    use photocraft_color::{ColorMode, SampleType};
    use photocraft_doc::Size;

    /// A white 40×20 document with its left half selected.
    fn half_selected() -> Document {
        let mut doc = Document::with_background("d", Size::new(40, 20), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        let mut sel = Surface::new(PixelFormat::GRAY8);
        sel.fill_rect(Rect::new(0, 0, 20, 20), &[1.0]);
        doc.selection = Some(sel);
        doc
    }

    /// The recipe of `mode` at 50 % opacity (the views without an opacity ignore it).
    fn recipe(mode: MaskViewMode) -> MaskViewRecipe {
        let mut state = MaskViewState::default();
        state.apply(MaskViewChange::Mode(mode));
        state.apply(MaskViewChange::Opacity(mode, 50.0));
        state.recipe()
    }

    fn pixel(doc: &Document, x: i32, y: i32) -> Vec<f32> {
        let s = photocraft_compose::flatten_to_surface(doc, PixelFormat::RGBA8, None);
        s.pixel(x, y)
    }

    /// Every view shows the selected half as the image and marks the rest its own way.
    #[test]
    fn each_view_marks_the_unselected_area() {
        let doc = half_selected();
        let active = doc.layers.first().map(|l| l.id);
        let near = |a: &[f32], b: &[f32]| a.iter().zip(b).all(|(a, b)| (a - b).abs() < 0.02);
        let white = [1.0, 1.0, 1.0, 1.0];
        // (view, the selected pixel, the unselected pixel)
        let mut selected_overlay = MaskViewState::default();
        selected_overlay.apply(MaskViewChange::Mode(MaskViewMode::Overlay));
        selected_overlay.apply(MaskViewChange::Opacity(MaskViewMode::Overlay, 50.0));
        selected_overlay.overlay_indicates = ColorIndicates::SelectedAreas;
        let cases = [
            (recipe(MaskViewMode::Overlay), white, [1.0, 0.5, 0.5, 1.0]),
            (selected_overlay.recipe(), [1.0, 0.5, 0.5, 1.0], white),
            (recipe(MaskViewMode::OnBlack), white, [0.5, 0.5, 0.5, 1.0]),
            (recipe(MaskViewMode::OnWhite), white, white),
            (recipe(MaskViewMode::BlackAndWhite), white, [0.0, 0.0, 0.0, 1.0]),
            (recipe(MaskViewMode::OnLayers), white, [0.0, 0.0, 0.0, 0.0]),
            (recipe(MaskViewMode::OnionSkin), white, [1.0, 1.0, 1.0, 0.5]),
            (recipe(MaskViewMode::MarchingAnts), white, white),
        ];
        for (recipe, selected, unselected) in cases {
            let shown = preview_proxy(doc.clone(), active, &json!({"radius": 0}), &recipe, 1, None).expect("a preview");
            let inside = pixel(&shown, 5, 10);
            assert!(near(&inside, &selected), "{:?}: selected area {inside:?}, want {selected:?}", recipe.mode);
            let outside = pixel(&shown, 35, 10);
            assert!(near(&outside, &unselected), "{:?}: unselected area {outside:?}, want {unselected:?}", recipe.mode);
            // The refined selection rides along for the canvas's marching ants.
            assert!(shown.selection.as_ref().is_some_and(|s| s.pixel(5, 10)[0] > 0.99 && s.pixel(35, 10)[0] < 0.01), "{:?}", recipe.mode);
        }
    }

    /// Refine Edge Brush strokes in the params reach the (reduced) preview: strands beyond the
    /// band join the selection, and Show Edge includes the painted area.
    #[test]
    fn brush_strokes_reach_the_preview() {
        // Warm subject (x < 40) with warm strands every 4th column from x = 60, on blue.
        let strand = |x: u32| x < 40 || ((60..84).contains(&x) && x.is_multiple_of(4));
        let mut doc = Document::with_background("d", Size::new(120, 40), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        if let Some(bg) = doc.layers.first_mut().and_then(|l| l.surface_mut()) {
            let n = bg.channels();
            let data: Vec<f32> = (0..40u32)
                .flat_map(|_| (0..120u32).flat_map(move |x| if strand(x) { [0.9, 0.7, 0.2, 1.0] } else { [0.1, 0.3, 0.8, 1.0] }.into_iter().take(n)))
                .collect();
            bg.write_region(Rect::new(0, 0, 120, 40), &data);
        }
        let mut sel = Surface::new(PixelFormat::GRAY8);
        sel.fill_rect(Rect::new(0, 0, 40, 40), &[1.0]);
        doc.selection = Some(sel);
        let active = doc.layers.first().map(|l| l.id);
        let params = json!({"radius": 4, "refineStrokes": [{"points": [[70.0, 2.0], [70.0, 38.0]], "size": 30.0, "mode": "add"}]});
        let proxy = || crate::proxy::proxy_document(&doc, 2);

        let shown = preview_proxy(proxy(), active, &params, &recipe(MaskViewMode::BlackAndWhite), 2, None).expect("a preview");
        assert!(pixel(&shown, 32, 10)[0] > 0.8, "a strand at full size x = 64 joins the selection");
        assert!(pixel(&shown, 50, 10)[0] < 0.01, "the background past the strands stays out");

        let mut state = MaskViewState::default();
        state.apply(MaskViewChange::Mode(MaskViewMode::BlackAndWhite));
        state.apply(MaskViewChange::ShowEdge(true));
        let edge = preview_proxy(proxy(), active, &params, &state.recipe(), 2, None).expect("a preview");
        assert!(pixel(&edge, 35, 10)[0] > 0.99 && pixel(&edge, 55, 10)[0] < 0.01, "Show Edge includes the painted area");
    }

    /// Show Edge shows the band around the selection's edge; Show Original the unrefined selection.
    #[test]
    fn show_edge_and_show_original() {
        let doc = half_selected();
        let active = doc.layers.first().map(|l| l.id);
        let mut state = MaskViewState::default();
        state.apply(MaskViewChange::Mode(MaskViewMode::BlackAndWhite));
        state.apply(MaskViewChange::ShowEdge(true));
        let edge = json!({"radius": 3, "smooth": 100});
        let shown = preview_proxy(doc.clone(), active, &edge, &state.recipe(), 1, None).expect("a preview");
        assert!(pixel(&shown, 20, 10)[0] > 0.99, "on the edge");
        assert!(pixel(&shown, 5, 10)[0] < 0.01 && pixel(&shown, 35, 10)[0] < 0.01, "away from it");

        state.apply(MaskViewChange::ShowEdge(false));
        state.apply(MaskViewChange::ShowOriginal(true));
        let original = json!({"radius": 3, "shiftEdge": 100});
        let shown = preview_proxy(doc, active, &original, &state.recipe(), 1, None).expect("a preview");
        assert!(pixel(&shown, 19, 10)[0] > 0.99 && pixel(&shown, 21, 10)[0] < 0.01, "the selection as it was");
    }
}

#[test]
fn sparse_edge_band_matches_dense_reference_at_canvas_edges_and_default_holes() {
    let canvas = Rect::new(0, 0, 540, 300);
    let mut mask = Surface::with_default(PixelFormat::GRAY8, &[1.0]);
    mask.fill_rect(Rect::new(0, 0, 7, 18), &[0.0]);
    mask.fill_rect(Rect::new(240, 100, 270, 280), &[0.0]);
    mask.fill_rect(Rect::new(20, 0, 24, 5), &[0.0]);
    let sparse = Coverage::edge_band(Some(&mask), canvas, 2);
    let input = photocraft_algo::selection::mask_from_surface(Some(&mask), canvas);
    let w = canvas.width() as usize;
    let h = canvas.height() as usize;
    let grown = photocraft_algo::matting::morph(&input, w, h, 2, true);
    let shrunk = photocraft_algo::matting::morph(&input, w, h, 2, false);
    for y in canvas.y0..canvas.y1 {
        for x in canvas.x0..canvas.x1 {
            let i = y as usize * w + x as usize;
            let want = (grown[i] - shrunk[i]).clamp(0.0, 1.0);
            assert!((sparse.at(x, y) - want).abs() < 1e-6, "({x},{y})");
        }
    }
    assert_eq!(sparse.values.sample_channel(0, 0, 0), sparse.at(0, 0));
    let display = sparse.surface(|v| v);
    assert_eq!(display.sample_channel(0, 0, 0), (sparse.at(0, 0) * 255.0).round() / 255.0);
    assert_eq!(sparse.values.tile_count(), display.tile_count());
}

#[test]
fn sparse_brush_dabs_bound_tiles_and_apply_add_restore_formula() {
    let canvas = Rect::new(0, 0, 100_000, 32);
    let mut base = Surface::with_default(PixelFormat::GRAY8.with_sample(photocraft_color::SampleType::F32), &[0.25]);
    base.fill_rect(Rect::new(100, 10, 104, 14), &[0.6]);
    let coverage = Coverage::read(Some(&base), canvas);
    let strokes = [
        photocraft_algo::matting::EdgeStroke { points: vec![(101.5, 11.5)], size: 4.0, add: true },
        photocraft_algo::matting::EdgeStroke { points: vec![(99_001.5, 11.5)], size: 4.0, add: false },
    ];
    let brush = photocraft_algo::matting::EdgeBrush::new(&strokes, canvas).expect("dabs paint");
    let brush_tiles = brush.tile_coords().count();
    assert!(brush_tiles <= 4, "distant dabs allocated {brush_tiles} tiles");
    let original = Coverage::read(Some(&base), canvas);
    let result = coverage.with_brush(Some(&brush), canvas);
    assert!(result.values.tile_count() <= original.values.tile_count() + brush_tiles);
    for (x, b) in [(101, brush.at(101, 11)), (99_001, brush.at(99_001, 11))] {
        let v = original.at(x, 11);
        let want = if b >= 0.0 { v.max(b) } else { v * (1.0 + b) };
        assert!((result.at(x, 11) - want).abs() < 1e-6, "x={x}, brush={b}");
    }
}

#[test]
fn preview_conversion_preserves_shared_solid_tiles() {
    let canvas = Rect::new(0, 0, 2048, 1024);
    let mut selection = Surface::new(PixelFormat::GRAY8);
    selection.fill_rect(canvas, &[1.0]);
    let coverage = Coverage::read(Some(&selection), canvas);
    let display = coverage.surface(|v| v);
    let a = photocraft_geom::TileCoord::new(0, 0);
    let b = photocraft_geom::TileCoord::new(3, 2);
    assert!(Arc::ptr_eq(coverage.values.tile(a).unwrap(), coverage.values.tile(b).unwrap()));
    assert!(Arc::ptr_eq(display.tile(a).unwrap(), display.tile(b).unwrap()));
}
