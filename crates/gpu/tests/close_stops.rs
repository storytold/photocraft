//! Exercise close, strictly increasing stops through the document planner. These use the
//! ordinary LUT branch; bounded coincident stops have a separate exact-ramp path.

use photocraft_color::{BlendMode, Color, ColorMode, SampleType};
use photocraft_doc::{
    Adjustment, Contour, Document, Effect, Fill, FillCache, FxCommon, FxPaint, Glow, GlowSource, GlowTechnique, Gradient, GradientStyle, Layer, LayerContent,
    StrokeFx, StrokePosition,
};
use photocraft_geom::Size;
use photocraft_gpu::plan;
use photocraft_raster::Surface;

#[derive(Clone, Copy, Debug)]
enum Ramp {
    Wide,
    CloseColor,
    CloseOpacity,
    ExactColor,
}

fn gradient(ramp: Ramp) -> Gradient {
    let mut gradient = Gradient { angle: 0.0, ..Gradient::default() };
    match ramp {
        Ramp::Wide => {}
        Ramp::CloseColor => {
            gradient.stops = vec![(0.0, Color::BLACK), (0.4999, Color::BLACK), (0.5, Color::WHITE), (1.0, Color::WHITE)];
        }
        Ramp::CloseOpacity => {
            gradient.opacity_stops = vec![(0.0, 1.0), (0.4999, 1.0), (0.5, 0.0), (1.0, 0.0)];
        }
        Ramp::ExactColor => {
            gradient.stops = vec![(0.0, Color::BLACK), (0.5, Color::BLACK), (0.5, Color::WHITE), (1.0, Color::WHITE)];
        }
    }
    gradient
}

fn fill(ramp: Ramp) -> Fill {
    let gradient = gradient(ramp);
    let mut fill = Fill::gradient(gradient.stops, 0.0, 1.0, GradientStyle::Linear, false);
    if let Fill::Gradient { opacity_stops, .. } = &mut fill {
        *opacity_stops = gradient.opacity_stops;
    }
    fill
}

fn document(depth: SampleType) -> Document {
    Document::new("close stops", Size::new(8, 2), ColorMode::Rgb, depth)
}

fn fill_document(depth: SampleType, ramp: Ramp) -> Document {
    let mut document = document(depth);
    document.layers.push(Layer::new("gradient", LayerContent::Fill(fill(ramp))));
    document
}

fn map_document(depth: SampleType, ramp: Ramp) -> Document {
    let mut document = document(depth);
    document.layers.push(Layer::new("pixels", LayerContent::Fill(Fill::Solid(Color::WHITE))));
    document.layers.push(Layer::new(
        "map",
        LayerContent::Adjustment(Adjustment::GradientMap {
            stops: gradient(ramp).stops.iter().map(|(position, color)| (*position, color.to_rgb())).collect(),
            reverse: false,
            dither: false,
        }),
    ));
    document
}

fn glow(paint: FxPaint) -> Glow {
    Glow {
        common: FxCommon::new(BlendMode::Screen, 0.8),
        paint,
        technique: GlowTechnique::Softer,
        spread: 0.0,
        size: 1.0,
        contour: Contour::Linear,
        anti_alias: false,
        range: 0.5,
        jitter: 0.0,
        noise: 0.0,
        source: GlowSource::Edge,
    }
}

#[derive(Clone, Copy, Debug)]
enum Route {
    Overlay,
    Stroke,
    InnerGlow,
    OuterGlow,
}

impl Route {
    const ALL: [Self; 4] = [Self::Overlay, Self::Stroke, Self::InnerGlow, Self::OuterGlow];

    fn effect(self, ramp: Ramp) -> Effect {
        let gradient = gradient(ramp);
        let paint = FxPaint::Gradient(gradient.clone());
        match self {
            Self::Overlay => Effect::GradientOverlay { common: FxCommon::new(BlendMode::Normal, 1.0), gradient, dither: false },
            Self::Stroke => Effect::Stroke(StrokeFx { common: FxCommon::new(BlendMode::Normal, 1.0), size: 1.0, position: StrokePosition::Inside, paint }),
            Self::InnerGlow => Effect::InnerGlow(glow(paint)),
            Self::OuterGlow => Effect::OuterGlow(glow(paint)),
        }
    }
}

fn effect_document(depth: SampleType, route: Route, ramp: Ramp) -> Document {
    let mut document = document(depth);
    let mut layer = Layer::new("pixels", LayerContent::Fill(Fill::Solid(Color::WHITE)));
    layer.effects.items.push(route.effect(ramp));
    document.layers.push(layer);
    document
}

// Collect every depth/channel failure so the initial RED identifies each actual planner
// route, rather than stopping at the first 8-bit document.
fn expect_close_fallback(document: &Document, context: &str, failures: &mut Vec<String>) {
    match plan(document) {
        Err(error) if error.0.contains("coincident gradient stops") => {}
        Err(error) => failures.push(format!("{context}: unexpected fallback: {error}")),
        Ok(_) => failures.push(format!("{context}: close increasing stops incorrectly planned on the GPU")),
    }
}

fn check_fill(ramp: Ramp) {
    let mut failures = Vec::new();
    for depth in SampleType::ALL {
        expect_close_fallback(&fill_document(depth, ramp), &format!("fill {depth:?} {ramp:?}"), &mut failures);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn check_effect(route: Route) {
    let mut failures = Vec::new();
    for depth in SampleType::ALL {
        for ramp in [Ramp::CloseColor, Ramp::CloseOpacity] {
            expect_close_fallback(&effect_document(depth, route, ramp), &format!("{route:?} {depth:?} {ramp:?}"), &mut failures);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn close_color_fill_requests_the_cpu() {
    check_fill(Ramp::CloseColor);
}

#[test]
fn close_opacity_fill_requests_the_cpu() {
    check_fill(Ramp::CloseOpacity);
}

#[test]
fn close_gradient_map_requests_the_cpu() {
    let mut failures = Vec::new();
    for depth in SampleType::ALL {
        expect_close_fallback(&map_document(depth, Ramp::CloseColor), &format!("Gradient Map {depth:?}"), &mut failures);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn close_gradient_overlay_requests_the_cpu() {
    check_effect(Route::Overlay);
}

#[test]
fn close_stroke_requests_the_cpu() {
    check_effect(Route::Stroke);
}

#[test]
fn close_inner_glow_requests_the_cpu() {
    check_effect(Route::InnerGlow);
}

#[test]
fn close_outer_glow_requests_the_cpu() {
    check_effect(Route::OuterGlow);
}

#[test]
fn wide_ordinary_ramps_remain_eligible_at_every_depth() {
    for depth in SampleType::ALL {
        assert!(plan(&fill_document(depth, Ramp::Wide)).is_ok(), "wide fill {depth:?}");
        assert!(plan(&map_document(depth, Ramp::Wide)).is_ok(), "wide Gradient Map {depth:?}");
        for route in Route::ALL {
            assert!(plan(&effect_document(depth, route, Ramp::Wide)).is_ok(), "wide {route:?} {depth:?}");
        }
    }
}

#[test]
fn matching_fill_caches_and_hidden_layers_bypass_close_ramps() {
    for depth in SampleType::ALL {
        for ramp in [Ramp::CloseColor, Ramp::CloseOpacity] {
            let mut document = fill_document(depth, ramp);
            let LayerContent::Fill(fill) = &document.layers[0].content else { panic!("the cache fixture must contain a fill layer") };
            document.layers[0].fill_cache = Some(FillCache { fill: fill.clone(), surface: Surface::with_default(document.pixel_format(), &[1.0; 4]) });
            assert!(plan(&document).is_ok(), "matching cached fill {depth:?} {ramp:?}");
            document.layers[0].fill_cache = None;
            document.layers[0].visible = false;
            assert!(plan(&document).is_ok(), "hidden live fill {depth:?} {ramp:?}");
        }
        let mut document = map_document(depth, Ramp::CloseColor);
        document.layers[1].visible = false;
        assert!(plan(&document).is_ok(), "hidden Gradient Map {depth:?}");
    }
}

#[test]
fn stale_fill_caches_do_not_bypass_close_ramps() {
    let mut failures = Vec::new();
    for depth in SampleType::ALL {
        for ramp in [Ramp::CloseColor, Ramp::CloseOpacity] {
            let mut document = fill_document(depth, ramp);
            document.layers[0].fill_cache =
                Some(FillCache { fill: Fill::Solid(Color::WHITE), surface: Surface::with_default(document.pixel_format(), &[1.0; 4]) });
            expect_close_fallback(&document, &format!("stale fill cache {depth:?} {ramp:?}"), &mut failures);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn disabled_and_hidden_effects_bypass_close_ramps() {
    for depth in SampleType::ALL {
        for route in Route::ALL {
            for ramp in [Ramp::CloseColor, Ramp::CloseOpacity] {
                let mut document = effect_document(depth, route, ramp);
                document.layers[0].effects.enabled = false;
                assert!(plan(&document).is_ok(), "disabled effect group {route:?} {depth:?} {ramp:?}");
                document.layers[0].effects.enabled = true;
                document.layers[0].effects.items[0].set_enabled(false);
                assert!(plan(&document).is_ok(), "disabled item {route:?} {depth:?} {ramp:?}");
                document.layers[0].effects.items[0].set_enabled(true);
                document.layers[0].visible = false;
                assert!(plan(&document).is_ok(), "hidden effect layer {route:?} {depth:?} {ramp:?}");
            }
        }
    }
}

#[test]
fn bounded_exact_duplicates_keep_their_separate_depth_and_geometry_policy() {
    for depth in [SampleType::U8, SampleType::U16] {
        assert!(plan(&fill_document(depth, Ramp::ExactColor)).is_ok(), "bounded exact fill {depth:?}");
        for route in [Route::Overlay, Route::Stroke] {
            assert!(plan(&effect_document(depth, route, Ramp::ExactColor)).is_ok(), "bounded exact {route:?} {depth:?}");
        }
        assert!(plan(&map_document(depth, Ramp::ExactColor)).unwrap_err().0.contains("exact gradient geometry"));
        for route in [Route::InnerGlow, Route::OuterGlow] {
            assert!(plan(&effect_document(depth, route, Ramp::ExactColor)).unwrap_err().0.contains("exact gradient geometry"));
        }
    }
    assert!(plan(&fill_document(SampleType::F32, Ramp::ExactColor)).unwrap_err().0.contains("32-bit"));
    for route in [Route::Overlay, Route::Stroke] {
        assert!(plan(&effect_document(SampleType::F32, route, Ramp::ExactColor)).unwrap_err().0.contains("32-bit"));
    }
}
