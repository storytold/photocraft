//! [`PrefixCache`]: renders through it are the plain render bit for bit, whatever changes between
//! them, and an edit at the top of the stack resumes from the cached composite below it (#6533).
use super::*;
use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::{Adjustment, Document, Effect, FxCommon, Layer, LayerContent, LayerMask, Shadow};
use photocraft_geom::Size;

/// Every sample's bits equal (NaN included).
fn same_bits(got: &Buffer, want: &Buffer, what: &str) {
    assert_eq!(got.rect, want.rect, "{what}: rect");
    let bits = |b: &Buffer| b.px.iter().flat_map(|p| p.map(f32::to_bits)).collect::<Vec<_>>();
    assert!(bits(got) == bits(want), "{what}: pixels differ");
}

fn levels(gamma: f32) -> Adjustment {
    let mut a = Adjustment::identity_levels();
    if let Adjustment::Levels { master, .. } = &mut a {
        master.gamma = gamma;
    }
    a
}

fn solid(d: &Document, name: &str, rect: Rect, rgba: [f32; 4]) -> Layer {
    let mut l = Layer::raster(name, d.pixel_format());
    if let Some(s) = l.surface_mut() {
        s.fill_rect(rect, &photocraft_raster::from_rgba(&d.pixel_format(), rgba));
    }
    l
}

/// 700 x 600 (a 3 x 3 grid of render tiles, the last ones partial): plain, masked, clipped,
/// grouped and shadowed layers, an adjustment in the middle and one on top.
fn stack(depth: SampleType) -> Document {
    let mut d = Document::with_background("prefix", Size::new(700, 600), ColorMode::Rgb, depth, Color::WHITE);
    let all = d.bounds();
    let a = solid(&d, "a", all, [0.8, 0.3, 0.1, 0.6]);
    let mut b = solid(&d, "b", Rect::new(40, 30, 650, 560), [0.1, 0.5, 0.9, 1.0]);
    b.opacity = 0.7;
    b.blend = BlendMode::Multiply;
    let mut m = photocraft_raster::Surface::new(photocraft_color::PixelFormat::new(ColorMode::Grayscale, depth, false));
    m.fill_rect(Rect::new(0, 0, 500, 600), &[1.0]);
    m.fill_rect(Rect::new(500, 0, 700, 600), &[0.4]);
    b.mask = Some(LayerMask { surface: m, enabled: true, linked: true, density: 1.0, feather: 0.0 });
    let mut c = solid(&d, "c clipped to b", Rect::new(0, 200, 700, 420), [0.9, 0.9, 0.2, 1.0]);
    c.clipped = true;
    let group = Layer::group(
        "g",
        vec![solid(&d, "g1", Rect::new(300, 0, 400, 600), [0.2, 0.8, 0.3, 0.5]), solid(&d, "g2", Rect::new(0, 250, 700, 350), [0.6, 0.1, 0.7, 0.5])],
    );
    let mut shadowed = solid(&d, "shadowed", Rect::new(260, 240, 460, 380), [0.3, 0.3, 0.8, 1.0]);
    let common = FxCommon::new(BlendMode::Multiply, 0.8);
    shadowed.effects.items = vec![Effect::DropShadow(Shadow {
        common,
        color: Color::BLACK,
        angle: 120.0,
        use_global_light: false,
        distance: 9.0,
        spread: 0.2,
        size: 7.0,
        contour: photocraft_doc::Contour::Linear,
        anti_alias: false,
        noise: 0.0,
        knocks_out: true,
    })];
    let mut mid = Layer::new("levels below", LayerContent::Adjustment(levels(0.8)));
    mid.opacity = 0.8;
    let top_pixels = solid(&d, "e", Rect::new(500, 400, 690, 590), [0.0, 0.0, 0.0, 0.4]);
    let top = Layer::new("levels on top", LayerContent::Adjustment(levels(1.3)));
    d.layers.extend([a, b, c, group, shadowed, mid, top_pixels, top]);
    d
}

/// An edit between two renders.
type Edit = Box<dyn Fn(&mut Document)>;

/// Renders `d` through `cache` as a full refresh, in bands and over a part, each checked against
/// the plain render.
fn check(d: &Document, cache: &PrefixCache, what: &str) {
    let bounds = d.bounds();
    let want = render(d, bounds);
    same_bits(&render_cached(d, bounds, cache), &want, &format!("{what}: full"));
    let mut y = 0;
    let _ = render_bands_cached(d, bounds, 256, cache, |band| -> Result<(), ()> {
        same_bits(&band, &render(d, band.rect), &format!("{what}: band at {y}"));
        y = band.rect.y1;
        Ok(())
    });
    assert_eq!(y, bounds.y1, "{what}: the bands cover the document");
    let part = Rect::new(100, 90, 420, 333);
    same_bits(&render_cached(d, part, cache), &render(d, part), &format!("{what}: part"));
}

#[test]
fn renders_through_the_cache_match_the_plain_render_across_edits() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut d = stack(depth);
        let cache = PrefixCache::default();
        check(&d, &cache, &format!("{depth:?} first"));
        let top = d.layers.len() - 1;
        let edits: Vec<(&str, Edit)> = vec![
            ("hide the top adjustment", Box::new(move |d| d.layers[top].visible = false)),
            ("show it again", Box::new(move |d| d.layers[top].visible = true)),
            ("its settings", Box::new(move |d| d.layers[top].content = LayerContent::Adjustment(levels(1.7)))),
            ("a low layer's mask turned off", Box::new(|d| d.layers[2].mask.iter_mut().for_each(|m| m.enabled = false))),
            ("and on again", Box::new(|d| d.layers[2].mask.iter_mut().for_each(|m| m.enabled = true))),
            (
                "a drop shadow following the global light",
                Box::new(|d| {
                    if let Some(Effect::DropShadow(s)) = d.layers[5].effects.items.first_mut() {
                        s.use_global_light = true;
                        s.distance = 20.0;
                    }
                }),
            ),
            // An edit above it, so the tiles hold the composite up to and including the shadow;
            // then a document setting, not a layer, that changes how the shadow draws.
            ("the adjustment above it", Box::new(|d| d.layers[6].opacity = 0.7)),
            ("the global light's angle", Box::new(|d| d.global_light.angle = 30.0)),
            ("a middle layer's opacity", Box::new(|d| d.layers[2].opacity = 0.35)),
            ("the bottom layer", Box::new(|d| d.layers[1].blend = BlendMode::Screen)),
            // Twice, so the tiles hold the stack up to layer 5; then it clips into the group
            // below, and splitting the stack there would draw it unclipped.
            ("layer 5's opacity", Box::new(|d| d.layers[5].opacity = 0.9)),
            ("layer 5's opacity again", Box::new(|d| d.layers[5].opacity = 0.6)),
            ("a layer starts clipping", Box::new(|d| d.layers[5].clipped = true)),
            ("a layer stops clipping", Box::new(|d| d.layers[3].clipped = false)),
            ("the middle adjustment hidden", Box::new(|d| d.layers[6].visible = false)),
            ("a layer added on top", Box::new(|d| d.layers.push(Layer::new("invert", LayerContent::Adjustment(Adjustment::Invert))))),
            ("the top layer removed", Box::new(|d| drop(d.layers.pop()))),
            ("a layer removed below", Box::new(|d| drop(d.layers.remove(2)))),
            ("nothing", Box::new(|_| {})),
        ];
        for (what, edit) in &edits {
            edit(&mut d);
            check(&d, &cache, &format!("{depth:?} {what}"));
        }
        // A document of the same layers but another size, then mode: the cache starts over.
        let mut other = stack(depth);
        other.size = Size::new(640, 600);
        check(&other, &cache, &format!("{depth:?} another size"));
        check(&stack(depth), &cache, &format!("{depth:?} back"));
        assert!(cache.bytes() > 0 && cache.bytes() <= PREFIX_CACHE_BYTES);
    }
}

#[test]
fn hiding_the_top_adjustment_resumes_every_tile_from_the_cache() {
    let mut d = stack(SampleType::U16);
    let bounds = d.bounds();
    let cache = PrefixCache::default();
    let _ = render_cached(&d, bounds, &cache);
    assert_eq!(cache.hits(), 0, "nothing cached yet");
    // The first render captured below the top layer: 9 tiles of 16 bytes a pixel.
    assert_eq!(cache.bytes(), u64::from(bounds.width()) * u64::from(bounds.height()) * 16);
    let top = d.layers.len() - 1;
    d.layers[top].visible = false;
    same_bits(&render_cached(&d, bounds, &cache), &render(&d, bounds), "hidden");
    assert_eq!(cache.hits(), 9, "every tile resumed from its composite");
    // Its settings changed: again from the cache.
    d.layers[top].visible = true;
    d.layers[top].content = LayerContent::Adjustment(levels(0.6));
    same_bits(&render_cached(&d, bounds, &cache), &render(&d, bounds), "new settings");
    assert_eq!(cache.hits(), 18);
    // An edit at the bottom invalidates every tile; the render after it captures below it and
    // the next edit there hits again.
    d.layers[1].opacity = 0.5;
    same_bits(&render_cached(&d, bounds, &cache), &render(&d, bounds), "bottom edit");
    assert_eq!(cache.hits(), 18, "the bottom edit missed");
    d.layers[1].opacity = 0.25;
    same_bits(&render_cached(&d, bounds, &cache), &render(&d, bounds), "bottom edit again");
    assert_eq!(cache.hits(), 27, "the cache followed the edit down");
    // And back up: the top edit resumes from the composite below layer 1 and captures below it.
    d.layers[top].visible = false;
    same_bits(&render_cached(&d, bounds, &cache), &render(&d, bounds), "top again");
    d.layers[top].visible = true;
    same_bits(&render_cached(&d, bounds, &cache), &render(&d, bounds), "top shown");
    assert_eq!(cache.hits(), 45);
}

#[test]
fn a_render_of_an_older_document_cannot_fill_the_cache_for_a_newer_one() {
    // A background redraw an edit restarted may still run on the document before the edit.
    let older = stack(SampleType::U8);
    let mut newer = older.clone();
    newer.layers[2].opacity = 0.1;
    let cache = PrefixCache::default();
    let t = Rect::new(0, 0, 256, 256);
    let Some(old) = cache.prepare(&older) else { panic!("a checkpoint") };
    let Some(new) = cache.prepare(&newer) else { panic!("a checkpoint") };
    // The older render stores its tile after the newer one prepared: nothing is kept.
    cache.store(t, old, &render(&older, t));
    assert!(cache.lookup(t, new).is_none() && cache.lookup(t, old).is_none());
    assert_eq!(cache.bytes(), 0);
    // Interleaved full renders of both stay exact.
    for d in [&older, &newer, &older, &newer] {
        same_bits(&render_cached(d, d.bounds(), &cache), &render(d, d.bounds()), "interleaved");
    }
}

#[test]
fn the_budget_bounds_the_cache() {
    let d = stack(SampleType::F32);
    let bounds = d.bounds();
    let off = PrefixCache::with_budget(0);
    same_bits(&render_cached(&d, bounds, &off), &render(&d, bounds), "off");
    assert_eq!(off.bytes(), 0);
    // Room for two whole tiles: two are kept, the others still render.
    let tile = 256 * 256 * 16;
    let small = PrefixCache::with_budget(2 * tile);
    for _ in 0..2 {
        same_bits(&render_cached(&d, bounds, &small), &render(&d, bounds), "small");
        assert!(small.bytes() <= 2 * tile);
    }
    assert!(small.hits() >= 1, "what fits is used");
    small.set_budget(tile);
    assert_eq!(small.bytes(), 0, "over the new budget: dropped");
    small.forget(d.id);
    let cache = PrefixCache::default();
    let _ = render_cached(&d, bounds, &cache);
    cache.forget(photocraft_doc::DocId(d.id.0.wrapping_add(1)));
    assert!(cache.bytes() > 0, "another document's id");
    cache.forget(d.id);
    assert_eq!(cache.bytes(), 0);
}

#[test]
fn renders_off_the_tile_grid_add_nothing() {
    // A brush stroke's damage, every frame somewhere else: entries that would never hit again.
    let mut d = stack(SampleType::U8);
    let bounds = d.bounds();
    let cache = PrefixCache::default();
    let _ = render_cached(&d, bounds, &cache);
    let full = cache.bytes();
    for (i, r) in [Rect::new(10, 10, 70, 70), Rect::new(256, 256, 300, 300), Rect::new(0, 0, 256, 100), Rect::new(250, 500, 700, 600)].into_iter().enumerate() {
        d.layers[7].opacity = 0.5 + 0.1 * i as f32;
        same_bits(&render_cached(&d, r, &cache), &render(&d, r), "damage");
        assert!(cache.bytes() <= full, "{r:?} added entries");
    }
    // The whole tiles of the document's last row and column, cut at its edge, are on the grid.
    let edge = Rect::new(512, 512, 700, 600);
    let before = cache.bytes();
    d.layers[7].opacity = 1.0;
    let _ = render_cached(&d, edge, &cache);
    assert!(cache.bytes() >= before, "the edge tile is kept");
}
