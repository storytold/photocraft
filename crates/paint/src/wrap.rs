//! View › Pattern Preview geometry: the document repeats its canvas (the *tile*) in every
//! direction, so a stroke past one edge carries on at the opposite one. Coordinates outside the
//! tile name the pixel a whole number of tile sizes away that lies on it.

use photocraft_geom::Rect;

/// `v` moved a whole number of periods into `lo..hi` (`v` itself when that is empty).
#[inline]
pub fn wrap_coord(v: i32, lo: i32, hi: i32) -> i32 {
    let period = i64::from(hi) - i64::from(lo);
    if period <= 0 {
        return v;
    }
    (i64::from(lo) + (i64::from(v) - i64::from(lo)).rem_euclid(period)) as i32
}

/// Whether `rect` fits in `tile`, so it covers no pixel of the tile twice.
pub fn fits(rect: Rect, tile: Rect) -> bool {
    rect.width() <= tile.width() && rect.height() <= tile.height()
}

/// The part of `tile` that `rect` covers once moved onto it: per axis, the rectangle's span moved
/// a whole number of periods, or the whole tile where the span is longer or crosses an edge.
pub fn fold_area(rect: Rect, tile: Rect) -> Rect {
    if rect.is_empty() || tile.is_empty() {
        return Rect::EMPTY;
    }
    let span = |a0: i32, a1: i32, lo: i32, hi: i32| {
        let start = wrap_coord(a0, lo, hi);
        let len = i64::from(a1) - i64::from(a0);
        if i64::from(start) + len > i64::from(hi) { (lo, hi) } else { (start, start + len as i32) }
    };
    let (x0, x1) = span(rect.x0, rect.x1, tile.x0, tile.x1);
    let (y0, y1) = span(rect.y0, rect.y1, tile.y0, tile.y1);
    Rect::new(x0, y0, x1, y1)
}

/// One axis of `a0..a1` cut where it crosses the edges of `lo..hi`: each run's start on the tile,
/// its start in `a0..a1`, and its length.
fn runs(a0: i32, a1: i32, lo: i32, hi: i32) -> Vec<(i32, i32, i32)> {
    let period = i64::from(hi) - i64::from(lo);
    let mut out = Vec::new();
    let mut a = i64::from(a0);
    while a < i64::from(a1) {
        let k = (a - i64::from(lo)).div_euclid(period) * period;
        let end = (i64::from(a1) - k).min(i64::from(hi));
        let start = a - k;
        out.push((start as i32, a as i32, (end - start) as i32));
        a = end + k;
    }
    out
}

/// `r` cut where it crosses the edges of `tile`: each part moved onto the tile, with the offset
/// that moves it back (`part.translate(dx, dy)` is that part of `r`). A rectangle bigger than the
/// tile gives parts that overlap on it.
pub fn pieces(r: Rect, tile: Rect) -> Vec<(Rect, (i32, i32))> {
    if r.is_empty() || tile.is_empty() {
        return Vec::new();
    }
    let xs = runs(r.x0, r.x1, tile.x0, tile.x1);
    runs(r.y0, r.y1, tile.y0, tile.y1)
        .into_iter()
        .flat_map(|(ty, uy, h)| xs.iter().map(move |&(tx, ux, w)| (Rect::new(tx, ty, tx + w, ty + h), (ux - tx, uy - ty))))
        .collect()
}

/// The values over `r` of an image repeating `tile` apart, given `read`, which returns a
/// rectangle of the tile (`n` values per pixel, row-major). A rectangle crossing at most one edge
/// per axis is read in its parts; a bigger one reads the tile area it covers once.
pub fn read<T: Copy + Default>(r: Rect, tile: Rect, n: usize, mut read: impl FnMut(Rect) -> Vec<T>) -> Vec<T> {
    let w = r.width() as usize;
    let mut out = vec![T::default(); w * r.height() as usize * n];
    if r.is_empty() || tile.is_empty() {
        return out;
    }
    let (xs, ys) = (runs(r.x0, r.x1, tile.x0, tile.x1), runs(r.y0, r.y1, tile.y0, tile.y1));
    let mut copy = |src: &[T], at: usize, x: i32, y: i32, len: i32| {
        let o = ((y - r.y0) as usize * w + (x - r.x0) as usize) * n;
        let len = len as usize * n;
        if let (Some(d), Some(s)) = (out.get_mut(o..o + len), src.get(at..at + len)) {
            d.copy_from_slice(s);
        }
    };
    if xs.len() * ys.len() <= 4 {
        for &(ty, uy, h) in &ys {
            for &(tx, ux, len) in &xs {
                let part = read(Rect::new(tx, ty, tx + len, ty + h));
                for row in 0..h {
                    copy(&part, row as usize * len as usize * n, ux, uy + row, len);
                }
            }
        }
        return out;
    }
    let area = fold_area(r, tile);
    let src = read(area);
    let aw = area.width() as usize;
    for &(ty, uy, h) in &ys {
        for row in 0..h {
            for &(tx, ux, len) in &xs {
                copy(&src, ((ty + row - area.y0) as usize * aw + (tx - area.x0) as usize) * n, ux, uy + row, len);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinates_wrap_onto_the_tile_from_any_distance() {
        assert_eq!([-17, -1, 0, 15, 16, 33, 1_000_000].map(|v| wrap_coord(v, 0, 16)), [15, 15, 0, 15, 0, 1, 0]);
        assert_eq!(wrap_coord(-3, 10, 20), 17);
    }

    #[test]
    fn pieces_cover_the_rectangle_once_and_land_on_the_tile() {
        let tile = Rect::new(0, 0, 10, 8);
        for r in [Rect::new(7, 6, 13, 11), Rect::new(-25, 3, 4, 5), Rect::new(2, 2, 5, 5), Rect::new(-3, -3, 30, 20)] {
            let mut seen = std::collections::HashMap::new();
            for (part, (dx, dy)) in pieces(r, tile) {
                assert!(tile.contains(part.x0, part.y0) && part.x1 <= tile.x1 && part.y1 <= tile.y1, "{part:?} on the tile");
                for y in part.y0..part.y1 {
                    for x in part.x0..part.x1 {
                        *seen.entry((x + dx, y + dy)).or_insert(0) += 1;
                        assert_eq!((wrap_coord(x + dx, 0, 10), wrap_coord(y + dy, 0, 8)), (x, y));
                    }
                }
            }
            assert_eq!(seen.len(), (r.width() * r.height()) as usize, "{r:?}");
            assert!(seen.values().all(|&c| c == 1) && seen.keys().all(|&(x, y)| r.contains(x, y)), "{r:?}");
        }
    }

    #[test]
    fn a_read_repeats_the_tile_whatever_the_rectangle() {
        let tile = Rect::new(4, 2, 14, 10);
        let value = |x: i32, y: i32| (wrap_coord(x, 4, 14) * 100 + wrap_coord(y, 2, 10)) as f32;
        let mut reads = 0;
        // Two values per pixel; `read` only ever asks for rectangles of the tile.
        let mut from_tile = |q: Rect| {
            reads += 1;
            assert!(q.x0 >= tile.x0 && q.y0 >= tile.y0 && q.x1 <= tile.x1 && q.y1 <= tile.y1, "{q:?}");
            (q.y0..q.y1).flat_map(|y| (q.x0..q.x1).flat_map(move |x| [value(x, y), -value(x, y)])).collect::<Vec<f32>>()
        };
        for r in [Rect::new(10, 7, 17, 12), Rect::new(5, 3, 9, 6), Rect::new(-31, -5, 40, 30), Rect::new(0, 0, 0, 5)] {
            let got = read(r, tile, 2, &mut from_tile);
            let want: Vec<f32> = (r.y0..r.y1).flat_map(|y| (r.x0..r.x1).flat_map(move |x| [value(x, y), -value(x, y)])).collect();
            assert_eq!(got, want, "{r:?}");
        }
        assert!(reads <= 4 + 1 + 1, "small rectangles read in parts, big ones once ({reads})");
    }

    #[test]
    fn the_fold_area_is_where_a_rectangle_lands() {
        let tile = Rect::new(0, 0, 10, 8);
        assert_eq!(fold_area(Rect::new(12, 9, 15, 12), tile), Rect::new(2, 1, 5, 4));
        assert_eq!(fold_area(Rect::new(8, 2, 12, 4), tile), Rect::new(0, 2, 10, 4));
        assert_eq!(fold_area(Rect::new(-50, -50, 50, 50), tile), tile);
        assert_eq!(fold_area(Rect::EMPTY, tile), Rect::EMPTY);
        assert!(fits(Rect::new(3, 3, 13, 11), tile) && !fits(Rect::new(0, 0, 11, 2), tile));
    }
}
