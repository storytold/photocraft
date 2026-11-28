use super::*;

/// A surface of `fmt` whose every sample over `r` differs from its neighbours' (deterministic).
fn noise(fmt: photocraft_color::PixelFormat, r: Rect, seed: u32) -> Surface {
    let n = fmt.channels();
    let mut px = Vec::with_capacity(r.width() as usize * r.height() as usize * n);
    for y in r.y0..r.y1 {
        for x in r.x0..r.x1 {
            for c in 0..n as u32 {
                let h = (x as u32).wrapping_mul(73_856_093) ^ (y as u32).wrapping_mul(19_349_663) ^ (c + seed).wrapping_mul(83_492_791);
                px.push((h % 1001) as f32 / 1000.0);
            }
        }
    }
    let mut s = Surface::new(fmt);
    s.write_region(r, &px);
    s
}

fn format(mode: photocraft_color::ColorMode, depth: u32, alpha: bool) -> photocraft_color::PixelFormat {
    let sample = match depth {
        8 => photocraft_color::SampleType::U8,
        16 => photocraft_color::SampleType::U16,
        _ => photocraft_color::SampleType::F32,
    };
    photocraft_color::PixelFormat::new(mode, sample, alpha)
}

/// #9332: Paste into a mask or channel reads the pasted pixels a tile at a time, with the same
/// result as reading the whole area into one buffer first: a source and target at each depth, a
/// soft limit, over an area that starts and ends inside tiles.
#[test]
fn paste_into_mask_per_tile_matches_the_area_buffer_at_every_depth() {
    use photocraft_color::ColorMode;
    let bounds = Rect::new(0, 0, 700, 520);
    let area = Rect::new(37, 21, 637, 497);
    for depth in [8, 16, 32] {
        let src = noise(format(ColorMode::Rgb, depth, true), bounds, 1);
        let limit = noise(photocraft_color::PixelFormat::GRAY8, bounds, 2);
        // A layer mask is 8-bit gray; an alpha channel or spot channel takes the document's depth.
        for target in [photocraft_color::PixelFormat::GRAY8, format(ColorMode::Grayscale, depth, false)] {
            let base = noise(target, bounds, 3);
            for lim in [None, Some(&limit)] {
                let (mut tiled, mut whole) = (base.clone(), base.clone());
                composite_over(&mut tiled, &src, area, lim).unwrap();
                composite_over_area_buffer(&mut whole, &src, area, lim).unwrap();
                assert_eq!(tiled.read_region(bounds), whole.read_region(bounds), "{depth}-bit source into {target:?}, limit {}", lim.is_some());
            }
        }
    }
}

/// #9332: Paste into a mask holds no buffer the size of the pasted area. The test failpoint lets
/// one fallible request through and fails the next: the result-tile list `blend_into` reserves
/// must be the only one (before, a 16 B/px read of the whole area came first, so this failed).
/// It counts requests through `crate::allocation` on this thread, not bytes: a plain `vec!` of
/// the area would not trip it.
#[test]
fn paste_into_mask_needs_no_area_sized_buffer() {
    let area = Rect::new(0, 0, 1024, 768);
    let src = noise(photocraft_color::PixelFormat::RGBA8, area, 5);
    let mut mask = noise(photocraft_color::PixelFormat::GRAY8, area, 6);
    crate::allocation::fail_after_for_test(1);
    let result = composite_over(&mut mask, &src, area, None);
    assert!(result.is_ok(), "a second fallible request (an area-sized buffer): {result:?}");
    // The failpoint is still armed: the paste made exactly one request.
    assert!(crate::allocation::checkpoint("after the paste").is_err(), "the paste made no fallible request at all");
}
