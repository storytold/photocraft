//! Background full CPU redraws (#4024): block order and the kept composites. The redraws on a device are in `gpu_tests`.

use super::*;

#[test]
fn blocks_cover_the_document_once() {
    let bounds = Rect::new(0, 0, 2500, 1100);
    let b = blocks(bounds);
    assert_eq!(b.len(), 3 * 2);
    assert_eq!(b.iter().map(|r| r.width() as u64 * r.height() as u64).sum::<u64>(), 2500 * 1100);
    for (i, r) in b.iter().enumerate() {
        assert!(r.x0 % BLOCK == 0 && r.y0 % BLOCK == 0, "block {i} starts on the grid: {r:?}");
        assert!(b[i + 1..].iter().all(|o| o.intersect(r).is_empty()), "block {i} overlaps another");
    }
}

#[test]
fn visible_blocks_come_first_nearest_the_view_centre() {
    let b = blocks(Rect::new(0, 0, 4096, 4096));
    // The view shows the right half of the second block row.
    let visible = Rect::new(2100, 1100, 4000, 1900);
    let o = order(&b, 0..b.len(), visible);
    assert_eq!(o.len(), 16);
    let shown: Vec<usize> = (0..b.len()).filter(|&i| !b[i].intersect(&visible).is_empty()).collect();
    assert_eq!(shown.len(), 2);
    assert!(o[..2].iter().all(|i| shown.contains(i)), "{o:?}");
    // Then the block row's neighbours, the far corners last.
    assert!(o[14..].contains(&12), "the bottom-left corner is among the last: {o:?}");
    // Only the blocks asked for.
    assert_eq!(order(&b, [3, 9].into_iter(), visible).len(), 2);
}

/// A kept composite of `doc`: `bytes` texels per block.
fn kept(doc: &Arc<Document>, bytes: usize) -> Kept {
    let all = blocks(doc.bounds());
    let texels: Vec<Vec<u8>> = all.iter().map(|_| vec![1u8; bytes]).collect();
    Kept {
        doc: Arc::downgrade(doc),
        id: doc.id,
        display: 7,
        text_gamma: 1,
        format: eframe::wgpu::TextureFormat::Rgba8Unorm,
        bytes: (bytes * all.len()) as u64,
        blocks: all,
        texels,
    }
}

#[test]
fn kept_composites_come_back_only_for_the_exact_state_and_stay_bounded() {
    use eframe::wgpu::TextureFormat::{Rgba8Unorm, Rgba16Float};
    let mut a = Document::new("kept", photocraft_geom::Size::new(2000, 1500), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
    let mut layer = photocraft_doc::Layer::raster("one", a.pixel_format());
    if let photocraft_doc::LayerContent::Raster(s) = &mut layer.content {
        s.fill_rect(Rect::new(0, 0, 300, 300), &[0.2, 0.4, 0.6, 1.0]);
    }
    a.layers.push(layer);
    let a = Arc::new(a);
    let all = blocks(a.bounds());
    let changed = |f: &dyn Fn(&mut Document)| {
        let mut d = (*a).clone();
        f(&mut d);
        Arc::new(d)
    };
    let mut cache = KeptComposites::default();
    cache.store(kept(&a, 4), &a, u64::MAX);
    // The same state hits, also as an equal document that isn't the same Arc (a restore after a solo).
    assert!(cache.find(&a, 7, 1, Rgba8Unorm, &all).is_some());
    assert!(cache.find(&Arc::new((*a).clone()), 7, 1, Rgba8Unorm, &all).is_some());
    // Any change to what the composite depends on misses.
    let hidden = changed(&|d| d.layers[0].visible = false);
    let faded = changed(&|d| d.layers[0].opacity = 0.5);
    let multiplied = changed(&|d| d.layers[0].blend = photocraft_color::BlendMode::Multiply);
    let painted = changed(&|d| {
        if let photocraft_doc::LayerContent::Raster(s) = &mut d.layers[0].content {
            s.fill_rect(Rect::new(10, 10, 11, 11), &[1.0, 0.0, 0.0, 1.0]);
        }
    });
    for (what, d) in [("visibility", &hidden), ("opacity", &faded), ("blend mode", &multiplied), ("pixels", &painted)] {
        assert!(cache.find(d, 7, 1, Rgba8Unorm, &all).is_none(), "{what} changed");
    }
    assert!(cache.find(&a, 8, 1, Rgba8Unorm, &all).is_none(), "another display transform");
    assert!(cache.find(&a, 7, 2, Rgba8Unorm, &all).is_none(), "another text gamma");
    assert!(cache.find(&a, 7, 1, Rgba16Float, &all).is_none(), "another texture format");
    assert!(cache.find(&a, 7, 1, Rgba8Unorm, &all[1..]).is_none(), "other blocks");
    // At most KEEP entries, the least recently used going first.
    cache.store(kept(&hidden, 4), &hidden, u64::MAX);
    assert!(cache.find(&a, 7, 1, Rgba8Unorm, &all).is_some());
    cache.store(kept(&faded, 4), &faded, u64::MAX);
    assert_eq!(cache.entries.len(), KEEP);
    assert!(cache.find(&hidden, 7, 1, Rgba8Unorm, &all).is_none(), "the least recently used went");
    assert!(cache.find(&a, 7, 1, Rgba8Unorm, &all).is_some() && cache.find(&faded, 7, 1, Rgba8Unorm, &all).is_some());
    // A stored entry makes the next frame take the cap again, over every kept document.
    cache.capped = (vec![(a.id, 1)], 1);
    cache.store(kept(&faded, 4), &faded, u64::MAX);
    assert_eq!(cache.capped, Default::default());
    // Storing a state again replaces its entry.
    cache.store(kept(&faded, 4), &faded, u64::MAX);
    assert_eq!(cache.entries.len(), 2);
    // The byte cap: a smaller one drops the oldest, and a composite over it isn't kept.
    let one = kept(&a, 4).bytes;
    cache.trim(one);
    assert_eq!(cache.entries.len(), 1);
    assert!(cache.bytes() <= one);
    cache.trim(0);
    cache.store(kept(&a, 4), &a, one - 1);
    assert!(cache.entries.is_empty(), "over the cap");
    // A closed document's composites go.
    cache.store(kept(&a, 4), &a, u64::MAX);
    cache.retain(|id| id != a.id);
    assert!(cache.entries.is_empty());
    // A state History no longer holds isn't kept alive: its entry is dead and goes.
    let gone = changed(&|d| d.layers[0].opacity = 0.25);
    cache.store(kept(&gone, 4), &gone, u64::MAX);
    let equal = Arc::new((*gone).clone());
    drop(gone);
    assert!(cache.find(&equal, 7, 1, Rgba8Unorm, &all).is_none(), "no strong reference to the drawn state");
    cache.trim(u64::MAX);
    assert!(cache.entries.is_empty());
}
