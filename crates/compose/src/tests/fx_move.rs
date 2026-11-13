//! Effect maps reused across a whole-pixel move (the GPU's move rule, `fx_passes.rs`).

use super::*;
use crate::fx_cache::{FX_TEST_LOCK, fx_builds, fx_entries, fx_evict, fx_layer_bytes};
use photocraft_doc::{
    BevelTechnique, BevelTexture, Effect, FxCommon, FxPaint, Glow, GlowSource, GlowTechnique, Gradient, Satin, Shadow, StrokeFx, StrokePosition,
};

const DEPTHS: [SampleType; 3] = [SampleType::U8, SampleType::U16, SampleType::F32];

fn held() -> std::sync::MutexGuard<'static, ()> {
    FX_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn common() -> FxCommon {
    FxCommon::new(photocraft_color::BlendMode::Normal, 1.0)
}

fn shadow_fx(distance: f32, size: f32) -> Shadow {
    Shadow {
        common: FxCommon::new(photocraft_color::BlendMode::Multiply, 0.8),
        color: Color::BLACK,
        angle: 120.0,
        use_global_light: false,
        distance,
        spread: 0.2,
        size,
        contour: photocraft_doc::Contour::Linear,
        anti_alias: false,
        noise: 0.0,
        knocks_out: true,
    }
}

fn glow_fx(technique: GlowTechnique, source: GlowSource) -> Glow {
    Glow {
        common: common(),
        paint: FxPaint::Color(Color::rgb(0.0, 1.0, 0.0)),
        technique,
        spread: 0.1,
        size: 5.0,
        contour: photocraft_doc::Contour::Linear,
        anti_alias: false,
        range: 0.5,
        jitter: 0.0,
        noise: 0.0,
        source,
    }
}

fn stroke_fx(size: f32, position: StrokePosition) -> Effect {
    Effect::Stroke(StrokeFx { common: common(), size, position, paint: FxPaint::Color(Color::rgb(0.0, 0.0, 1.0)), overprint: false })
}

fn bevel(technique: BevelTechnique, texture: Option<BevelTexture>) -> Effect {
    let mut b = match bevel_fx(photocraft_doc::BevelStyle::InnerBevel, 1.0) {
        Effect::BevelEmboss(b) => b,
        other => panic!("not a bevel: {other:?}"),
    };
    b.technique = technique;
    b.texture = texture;
    Effect::BevelEmboss(b)
}

fn texture(link: bool) -> BevelTexture {
    BevelTexture { name: "stripes".into(), id: "stripes".into(), scale: 1.0, depth: 1.0, invert: false, link, phase: (0.0, 0.0) }
}

fn stripes() -> photocraft_doc::pattern::Pattern {
    let mut sf = photocraft_raster::Surface::new(PixelFormat::RGBA8);
    sf.fill_rect(Rect::new(0, 0, 5, 2), &[0.0, 0.0, 0.0, 1.0]);
    sf.fill_rect(Rect::new(0, 2, 5, 5), &[1.0, 1.0, 1.0, 1.0]);
    let mut p = photocraft_doc::pattern::Pattern::new("stripes", sf, 5, 5);
    p.id = "stripes".into();
    p
}

/// A 128² document with one layer: an opaque block with a half-covered column (so the shape
/// isn't flat), holding `fx`.
fn moving_doc(depth: SampleType, block: Rect, fx: Vec<Effect>) -> Document {
    let mut d = Document::with_background("t", Size::new(128, 128), ColorMode::Rgb, depth, Color::WHITE);
    d.patterns.push(stripes());
    let f = d.pixel_format();
    let mut l = Layer::raster("fx", f);
    l.surface_mut().unwrap().fill_rect(block, &photocraft_raster::from_rgba(&f, [0.9, 0.3, 0.2, 1.0]));
    let col = Rect::new(block.x0, block.y0, block.x0 + 1, block.y1);
    l.surface_mut().unwrap().fill_rect(col, &photocraft_raster::from_rgba(&f, [0.9, 0.3, 0.2, 0.4]));
    l.effects.items = fx;
    d.layers.push(l);
    d
}

/// The engine's move (`shift_shown`): pixels and the effects reference point move together.
fn moved(l: &mut Layer, dx: i32, dy: i32) {
    let s = l.surface().unwrap().clone();
    *l.surface_mut().unwrap() = s.translated(dx, dy, s.content_bounds());
    if let Some(r) = &mut l.effects.reference {
        *r = (r.0 + f64::from(dx), r.1 + f64::from(dy));
    }
}

/// The document rendered with the top layer under a fresh id, so no cached maps apply.
fn cold(d: &Document, rect: Rect) -> Buffer {
    let mut c = d.clone();
    let i = c.layers.len() - 1;
    c.layers[i].id = Layer::raster("cold", c.pixel_format()).id;
    render(&c, rect)
}

fn top_id(d: &Document) -> u64 {
    d.layers[d.layers.len() - 1].id.0
}

/// Renders `d` warm, moves its top layer by `(dx, dy)` and renders again. Returns the moved
/// document, the builds the moved render did, and whether it equals a cold render.
fn warm_then_move(mut d: Document, dx: i32, dy: i32) -> (Document, usize, bool) {
    let id = top_id(&d);
    let _ = flatten(&d);
    let before = fx_builds(id);
    let i = d.layers.len() - 1;
    moved(&mut d.layers[i], dx, dy);
    let warm = flatten(&d);
    let builds = fx_builds(id) - before;
    let same = warm.px == cold(&d, d.bounds()).px;
    (d, builds, same)
}

#[test]
fn a_moved_layer_reuses_its_effect_maps() {
    let _g = held();
    let d = moving_doc(SampleType::U8, Rect::new(30, 30, 70, 60), vec![Effect::DropShadow(shadow_fx(6.0, 4.0)), stroke_fx(3.0, StrokePosition::Outside)]);
    let (_, builds, same) = warm_then_move(d, 10, 0);
    assert_eq!(builds, 0, "the moved layer's maps are reused, not rebuilt");
    assert!(same, "the reused maps render like a cold build");
}

#[test]
fn rebased_maps_equal_a_fresh_build_for_every_local_effect() {
    let _g = held();
    let stacks: Vec<(&str, Vec<Effect>)> = vec![
        ("drop shadow", vec![Effect::DropShadow(shadow_fx(6.0, 4.0))]),
        ("inner shadow", vec![Effect::InnerShadow(shadow_fx(3.0, 3.0))]),
        ("outer glow", vec![Effect::OuterGlow(glow_fx(GlowTechnique::Softer, GlowSource::Edge))]),
        ("inner glow", vec![Effect::InnerGlow(glow_fx(GlowTechnique::Softer, GlowSource::Center))]),
        (
            "satin",
            vec![Effect::Satin(Satin {
                common: common(),
                color: Color::BLACK,
                angle: 19.0,
                distance: 4.0,
                size: 5.0,
                contour: photocraft_doc::Contour::Linear,
                anti_alias: true,
                invert: false,
            })],
        ),
        ("stroke inside", vec![stroke_fx(2.0, StrokePosition::Inside)]),
        ("stroke center", vec![stroke_fx(4.0, StrokePosition::Center)]),
        ("stroke outside", vec![stroke_fx(3.0, StrokePosition::Outside)]),
        ("smooth bevel", vec![bevel(BevelTechnique::Smooth, None)]),
        ("smooth bevel, linked texture", vec![bevel(BevelTechnique::Smooth, Some(texture(true)))]),
        ("stroke emboss", vec![stroke_fx(3.0, StrokePosition::Outside), bevel_fx(photocraft_doc::BevelStyle::StrokeEmboss, 1.0)]),
        ("colour overlay", vec![Effect::ColorOverlay { common: common(), color: Color::rgb(0.0, 1.0, 0.0) }]),
        ("gradient overlay", vec![Effect::GradientOverlay { common: common(), gradient: Gradient::default(), dither: false, interpolation: None }]),
        (
            "pattern overlay",
            vec![Effect::PatternOverlay {
                common: common(),
                name: "stripes".into(),
                id: "stripes".into(),
                scale: 1.0,
                angle: 0.0,
                link: true,
                phase: (0.0, 0.0),
            }],
        ),
    ];
    for depth in DEPTHS {
        for (name, fx) in &stacks {
            let mut d = moving_doc(depth, Rect::new(30, 30, 70, 60), fx.clone());
            let i = d.layers.len() - 1;
            if name.contains("texture") {
                d.layers[i].effects.reference = Some((33.0, 31.0));
            }
            let (d, builds, same) = warm_then_move(d, 10, 7);
            assert_eq!(builds, 0, "{depth:?} {name}: reused");
            assert!(same, "{depth:?} {name}: renders like a cold build");
            // The rebased entry against maps built fresh at the new place.
            let entries = fx_entries(top_id(&d));
            let mut fresh = d.clone();
            fresh.layers[i].id = Layer::raster("fresh", fresh.pixel_format()).id;
            let _ = flatten(&fresh);
            let built = fx_entries(top_id(&fresh));
            assert_eq!(built.len(), 1, "{depth:?} {name}");
            let (f, _) = &built[0];
            let rebased = entries.iter().find(|(m, _)| m.rect() == f.rect()).map(|(m, _)| m.clone());
            let rebased = rebased.unwrap_or_else(|| panic!("{depth:?} {name}: an entry at {:?}", f.rect()));
            assert!(rebased.same_arrays(f), "{depth:?} {name}: rebased maps equal a fresh build");
        }
    }
}

#[test]
fn non_local_effects_rebuild_after_a_move() {
    let _g = held();
    let gradient_stroke = Effect::Stroke(StrokeFx {
        common: common(),
        size: 3.0,
        position: StrokePosition::Outside,
        paint: FxPaint::Gradient(Gradient::default()),
        overprint: false,
    });
    let stacks = [
        ("precise glow", Effect::OuterGlow(glow_fx(GlowTechnique::Precise, GlowSource::Edge))),
        ("chiselled bevel", bevel(BevelTechnique::ChiselHard, None)),
        ("gradient stroke", gradient_stroke),
        ("noisy shadow", Effect::DropShadow(Shadow { noise: 0.3, ..shadow_fx(6.0, 4.0) })),
    ];
    for (name, fx) in stacks {
        let (_, builds, same) = warm_then_move(moving_doc(SampleType::U8, Rect::new(30, 30, 70, 60), vec![fx]), 10, 7);
        assert_eq!(builds, 1, "{name}: rebuilt once");
        assert!(same, "{name}");
    }
}

#[test]
fn a_move_that_changes_the_shape_rebuilds_the_maps() {
    let _g = held();
    let fx = vec![Effect::DropShadow(shadow_fx(6.0, 4.0)), stroke_fx(3.0, StrokePosition::Outside)];
    // Pushed past the canvas edge: the region keeps its size (clipped by the canvas grown by the
    // reach on the far side) only by luck, and the shape within it changes.
    let (_, builds, same) = warm_then_move(moving_doc(SampleType::U8, Rect::new(80, 30, 120, 60), fx.clone()), 20, 0);
    assert_eq!(builds, 1, "clipped by the canvas: rebuilt");
    assert!(same, "clipped by the canvas");
    // An unlinked mask that stays put while the pixels move: same region, another shape.
    let mut d = moving_doc(SampleType::U8, Rect::new(30, 30, 70, 60), fx);
    let mut m = photocraft_raster::Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false));
    m.fill_rect(Rect::new(0, 0, 128, 128), &[1.0]);
    m.fill_rect(Rect::new(50, 0, 56, 128), &[0.0]);
    let i = d.layers.len() - 1;
    d.layers[i].mask = Some(LayerMask { surface: m, enabled: true, linked: false, density: 1.0, feather: 0.0 });
    let (_, builds, same) = warm_then_move(d, 10, 0);
    assert_eq!(builds, 1, "a mask that stays put: rebuilt");
    assert!(same, "a mask that stays put");
}

#[test]
fn an_anchored_texture_reuses_maps_only_when_its_anchor_moved_too() {
    let _g = held();
    let fx = vec![bevel(BevelTechnique::Smooth, Some(texture(true)))];
    // The reference point moves with the layer: reuse.
    let mut d = moving_doc(SampleType::U8, Rect::new(30, 30, 70, 60), fx.clone());
    let i = d.layers.len() - 1;
    d.layers[i].effects.reference = Some((30.0, 30.0));
    let (_, builds, same) = warm_then_move(d, 10, 7);
    assert_eq!(builds, 0, "anchor moved with the layer: reused");
    assert!(same);
    // The reference point left in place: the texture lies differently under the shape.
    let mut d = moving_doc(SampleType::U8, Rect::new(30, 30, 70, 60), fx.clone());
    d.layers[i].effects.reference = Some((30.0, 30.0));
    let id = top_id(&d);
    let _ = flatten(&d);
    let before = fx_builds(id);
    moved(&mut d.layers[i], 10, 7);
    d.layers[i].effects.reference = Some((30.0, 30.0));
    let warm = flatten(&d);
    assert_eq!(fx_builds(id) - before, 1, "anchor left in place: rebuilt");
    assert!(warm.px == cold(&d, d.bounds()).px);
    // No reference point and a layer wider than the canvas and its reach on both sides: the
    // region (clipped by the canvas grown by the reach) and the shape over it stay put, while the
    // anchor (the layer's top-left) moves.
    let (_, builds, same) = warm_then_move(moving_doc(SampleType::U8, Rect::new(-200, 30, 330, 60), fx.clone()), 5, 0);
    assert_eq!(builds, 1, "anchor moved, region didn't: rebuilt");
    assert!(same);
    // Not linked with the layer: tiles from the canvas, so a move always rebuilds.
    let (_, builds, same) =
        warm_then_move(moving_doc(SampleType::U8, Rect::new(30, 30, 70, 60), vec![bevel(BevelTechnique::Smooth, Some(texture(false)))]), 10, 7);
    assert_eq!(builds, 1, "unlinked texture: rebuilt");
    assert!(same);
}

#[test]
fn a_clipped_entry_is_never_reused_after_a_move() {
    let _g = held();
    // Transparency Shapes Layer off: the region is the canvas and a part's render clips it.
    let mut d = moving_doc(SampleType::U8, Rect::new(30, 30, 70, 60), vec![stroke_fx(3.0, StrokePosition::Outside)]);
    let i = d.layers.len() - 1;
    d.layers[i].advanced.transparency_shapes = false;
    let part = Rect::new(20, 20, 60, 50);
    let id = top_id(&d);
    let _ = render(&d, part);
    let before = fx_builds(id);
    moved(&mut d.layers[i], 4, 0);
    let warm = render(&d, part);
    assert_eq!(fx_builds(id) - before, 1, "a clipped entry rebuilds");
    assert!(warm.px == cold(&d, part).px);
}

#[test]
fn a_rebased_entry_is_counted_once_in_the_effect_cache() {
    let _g = held();
    let mut d = moving_doc(SampleType::U8, Rect::new(30, 30, 70, 60), vec![Effect::DropShadow(shadow_fx(6.0, 4.0)), stroke_fx(3.0, StrokePosition::Outside)]);
    let id = top_id(&d);
    let _ = flatten(&d);
    let first = fx_entries(id);
    assert_eq!(first.len(), 1);
    let (orig, orig_bytes) = first[0].clone();
    assert!(orig_bytes < orig.bytes() / 4, "an entry's own bytes are its pins ({orig_bytes} vs {} for the maps)", orig.bytes());
    assert!(fx_layer_bytes(id) >= orig.bytes(), "the built entry's maps are counted");
    let i = d.layers.len() - 1;
    moved(&mut d.layers[i], 10, 7);
    let _ = flatten(&d);
    let after = fx_entries(id);
    let (rebased, bytes) = after.iter().find(|(m, _)| !std::sync::Arc::ptr_eq(m, &orig)).cloned().unwrap_or_else(|| panic!("a rebased entry"));
    assert!(rebased.shares_arrays(&orig), "the rebased entry shares the arrays");
    assert!(bytes < orig.bytes() / 4, "the rebased entry counts its pins only ({bytes} vs {} for the maps)", orig.bytes());
}

#[test]
fn evicting_the_original_keeps_the_moved_entrys_bytes_counted() {
    let _g = held();
    let mut d = moving_doc(SampleType::U8, Rect::new(30, 30, 70, 60), vec![Effect::DropShadow(shadow_fx(6.0, 4.0)), stroke_fx(3.0, StrokePosition::Outside)]);
    let id = top_id(&d);
    let _ = flatten(&d);
    let (orig, _) = fx_entries(id)[0].clone();
    let arrays = orig.bytes();
    let i = d.layers.len() - 1;
    moved(&mut d.layers[i], 10, 7);
    let _ = flatten(&d);
    let both = fx_layer_bytes(id);
    assert!(both >= arrays && both < arrays + arrays / 4, "both entries count the arrays once ({both} for {arrays})");
    // The budget evicts oldest first: the entry built from scratch goes before the moved one.
    fx_evict(id, |m| m.rect() == orig.rect());
    let left = fx_entries(id);
    assert_eq!(left.len(), 1, "the moved entry stays");
    assert!(left[0].0.shares_arrays(&orig));
    let moved_only = fx_layer_bytes(id);
    assert!(moved_only >= arrays, "the arrays the moved entry keeps alive are still counted ({moved_only} for {arrays})");
    drop(left);
    fx_evict(id, |_| true);
    assert_eq!(fx_layer_bytes(id), 0, "nothing is counted once the last entry goes");
}
