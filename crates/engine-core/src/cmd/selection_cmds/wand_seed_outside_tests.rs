//! #11779: with Contiguous on, the Magic Wand's Subtract and Intersect grow the region from the
//! seed through the current selection only. A seed outside the selection subtracts nothing and
//! intersects to nothing.

#[test]
fn the_fill_passes_only_pixels_at_least_half_selected() {
    use photocraft_algo::selection::wand_region_within;
    use photocraft_geom::Rect;
    // One flat grey row of four pixels; only the coverage limit decides the region.
    let img = [[128u8, 128, 128, 255]; 4];
    let area = Rect::new(0, 0, 4, 1);
    let fill = |within: [f32; 4], seed: i32| wand_region_within(&img, area, (seed, 0), 0.0, true, false, Some(&within));
    assert!(fill([0.49, 1.0, 1.0, 1.0], 0).is_none(), "a seed just under half selected finds nothing");
    let r = fill([0.5, 1.0, 1.0, 1.0], 0).unwrap();
    assert_eq!((0..4).map(|x| r.at(x, 0) >= 0.5).collect::<Vec<_>>(), [true; 4], "half selected counts");
    let r = fill([1.0, 1.0, 0.49, 1.0], 0).unwrap();
    assert_eq!((0..4).map(|x| r.at(x, 0) >= 0.5).collect::<Vec<_>>(), [true, true, false, false], "the fill stops at 0.49");
}
