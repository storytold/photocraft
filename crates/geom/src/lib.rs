//! Geometry primitives shared by every PhotoCraft crate.
//!
//! Document space is integer pixels with the origin at the canvas top-left.
//! Layers may extend beyond the canvas (negative coordinates are valid).
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use serde::{Deserialize, Serialize};

pub mod warp;

/// Edge length of a raster tile in pixels.
pub const TILE_SIZE: i32 = 256;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

impl Size {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
    pub fn area(self) -> u64 {
        self.width as u64 * self.height as u64
    }
    pub fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// Integer rectangle, half-open: `[x0, x1) × [y0, y1)`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl Rect {
    pub const EMPTY: Rect = Rect { x0: 0, y0: 0, x1: 0, y1: 0 };

    pub const fn new(x0: i32, y0: i32, x1: i32, y1: i32) -> Self {
        Self { x0, y0, x1, y1 }
    }
    pub fn from_xywh(x: i32, y: i32, w: u32, h: u32) -> Self {
        // Widen before adding: `w as i32` wraps negative for widths above `i32::MAX`, which
        // would turn the far edge backwards (an empty, inverted rect) instead of clamping it
        // to `i32::MAX` the way `saturating_add` alone would suggest.
        let x1 = (i64::from(x) + i64::from(w)).min(i64::from(i32::MAX)) as i32;
        let y1 = (i64::from(y) + i64::from(h)).min(i64::from(i32::MAX)) as i32;
        Self::new(x, y, x1, y1)
    }
    pub fn from_size(size: Size) -> Self {
        Self::from_xywh(0, 0, size.width, size.height)
    }
    /// Horizontal extent; 0 when `x1 <= x0`. Exact even for i32-extreme edges.
    pub fn width(&self) -> u32 {
        if self.x1 > self.x0 { self.x1.abs_diff(self.x0) } else { 0 }
    }
    /// Vertical extent; 0 when `y1 <= y0`. Exact even for i32-extreme edges.
    pub fn height(&self) -> u32 {
        if self.y1 > self.y0 { self.y1.abs_diff(self.y0) } else { 0 }
    }
    pub fn size(&self) -> Size {
        Size::new(self.width(), self.height())
    }
    pub fn is_empty(&self) -> bool {
        self.x1 <= self.x0 || self.y1 <= self.y0
    }
    /// `other` lies entirely inside `self` (an empty `other` is contained in anything).
    pub fn contains_rect(&self, other: &Rect) -> bool {
        other.is_empty() || (other.x0 >= self.x0 && other.y0 >= self.y0 && other.x1 <= self.x1 && other.y1 <= self.y1)
    }
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x0 && x < self.x1 && y >= self.y0 && y < self.y1
    }
    /// The two rects share at least one pixel. Same answer as `!self.intersect(o).is_empty()`,
    /// without building the intersection. Empty or inverted rects overlap nothing.
    pub fn intersects(&self, o: &Rect) -> bool {
        !self.is_empty() && !o.is_empty() && self.x0 < o.x1 && o.x0 < self.x1 && self.y0 < o.y1 && o.y0 < self.y1
    }
    /// Midpoint of the rect in document space (pixel edges, so a 1×1 rect at the origin centres
    /// on `(0.5, 0.5)`), or `None` when the rect is empty. Exact even for i32-extreme edges,
    /// where `x0 + x1` would overflow.
    pub fn center(&self) -> Option<Point> {
        if self.is_empty() {
            return None;
        }
        let mid = |lo: i32, hi: i32| (f64::from(lo) + f64::from(hi)) / 2.0;
        Some(Point::new(mid(self.x0, self.x1), mid(self.y0, self.y1)))
    }
    pub fn intersect(&self, o: &Rect) -> Rect {
        let r = Rect::new(self.x0.max(o.x0), self.y0.max(o.y0), self.x1.min(o.x1), self.y1.min(o.y1));
        if r.is_empty() { Rect::EMPTY } else { r }
    }
    pub fn union(&self, o: &Rect) -> Rect {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        Rect::new(self.x0.min(o.x0), self.y0.min(o.y0), self.x1.max(o.x1), self.y1.max(o.y1))
    }
    pub fn translate(&self, dx: i32, dy: i32) -> Rect {
        Rect::new(self.x0.saturating_add(dx), self.y0.saturating_add(dy), self.x1.saturating_add(dx), self.y1.saturating_add(dy))
    }
    pub fn inflate(&self, d: i32) -> Rect {
        Rect::new(self.x0.saturating_sub(d), self.y0.saturating_sub(d), self.x1.saturating_add(d), self.y1.saturating_add(d))
    }
    /// How many tiles overlap this rect, without visiting them (a rect can span billions).
    pub fn tile_count(&self) -> u64 {
        if self.is_empty() {
            return 0;
        }
        let (a, b) = (TileCoord::containing(self.x0, self.y0), TileCoord::containing(self.x1 - 1, self.y1 - 1));
        ((i64::from(b.tx) - i64::from(a.tx) + 1) as u64) * ((i64::from(b.ty) - i64::from(a.ty) + 1) as u64)
    }
    /// All tiles overlapping this rect.
    pub fn tiles(&self) -> impl Iterator<Item = TileCoord> + use<> {
        let (a, b) = if self.is_empty() {
            (TileCoord::new(0, 0), TileCoord::new(-1, -1))
        } else {
            (TileCoord::containing(self.x0, self.y0), TileCoord::containing(self.x1 - 1, self.y1 - 1))
        };
        (a.ty..=b.ty).flat_map(move |ty| (a.tx..=b.tx).map(move |tx| TileCoord::new(tx, ty)))
    }
}

/// Index of a tile in the infinite tile grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TileCoord {
    pub tx: i32,
    pub ty: i32,
}

impl TileCoord {
    pub const fn new(tx: i32, ty: i32) -> Self {
        Self { tx, ty }
    }
    pub fn containing(x: i32, y: i32) -> Self {
        Self::new(x.div_euclid(TILE_SIZE), y.div_euclid(TILE_SIZE))
    }
    pub fn rect(&self) -> Rect {
        // `tx`/`ty` can come from a file (this type is deserialized), so saturate instead of
        // overflowing (a debug-build panic / release-build wrap) on extreme coordinates.
        Rect::from_xywh(self.tx.saturating_mul(TILE_SIZE), self.ty.saturating_mul(TILE_SIZE), TILE_SIZE as u32, TILE_SIZE as u32)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// 2D affine transform `[a c e; b d f; 0 0 1]` (column vectors).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Affine {
    pub m: [f64; 6],
}

impl Default for Affine {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Affine {
    pub const IDENTITY: Affine = Affine { m: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0] };

    pub fn translate(dx: f64, dy: f64) -> Self {
        Affine { m: [1.0, 0.0, 0.0, 1.0, dx, dy] }
    }
    pub fn scale(s: f64) -> Self {
        Affine { m: [s, 0.0, 0.0, s, 0.0, 0.0] }
    }
    /// Non-uniform scale about the origin (a negative factor mirrors that axis).
    pub fn scale_xy(sx: f64, sy: f64) -> Self {
        Affine { m: [sx, 0.0, 0.0, sy, 0.0, 0.0] }
    }
    pub fn rotate(radians: f64) -> Self {
        let (s, c) = radians.sin_cos();
        Affine { m: [c, s, -s, c, 0.0, 0.0] }
    }
    /// Rotation by `radians` about `center` (which stays fixed) instead of the origin.
    pub fn rotate_about(radians: f64, center: Point) -> Self {
        Affine::translate(-center.x, -center.y).then(&Affine::rotate(radians)).then(&Affine::translate(center.x, center.y))
    }
    /// `self`, then `other`: apply `self` first, then `other` (so `a.then(&b) == b.mul(&a)`).
    pub fn then(&self, other: &Affine) -> Affine {
        other.mul(self)
    }
    pub fn mul(&self, o: &Affine) -> Affine {
        let [a, b, c, d, e, f] = self.m;
        let [oa, ob, oc, od, oe, of] = o.m;
        Affine { m: [a * oa + c * ob, b * oa + d * ob, a * oc + c * od, b * oc + d * od, a * oe + c * of + e, b * oe + d * of + f] }
    }
    pub fn apply(&self, p: Point) -> Point {
        let [a, b, c, d, e, f] = self.m;
        Point::new(a * p.x + c * p.y + e, b * p.x + d * p.y + f)
    }
    pub fn determinant(&self) -> f64 {
        self.m[0] * self.m[3] - self.m[1] * self.m[2]
    }
    /// Returns `None` for a near-singular matrix or non-finite determinant/inverse coefficients.
    ///
    /// "Near-singular" is relative: the two columns of the linear part are (nearly) parallel,
    /// i.e. `|det|` is tiny next to the product of the column lengths (the sine of the angle
    /// between them). An absolute cut-off on `det` would wrongly reject a well-conditioned
    /// transform that is merely small, such as `scale(1e-7)`.
    pub fn inverse(&self) -> Option<Affine> {
        let det = self.determinant();
        if !det.is_finite() {
            return None;
        }
        let [a, b, c, d, e, f] = self.m;
        // Scale each column by its largest component first. The sine of the angle between the
        // columns is then `det' / (|col1'| * |col2'|)` with every term in a safe range, whereas
        // multiplying the raw column lengths overflows for finite, invertible inputs such as
        // `[1e154, 1e154, 1e154, 1.5e154]`. A zero column (largest component 0) is singular.
        let (s1, s2) = (a.abs().max(b.abs()), c.abs().max(d.abs()));
        if s1 == 0.0 || s2 == 0.0 {
            return None;
        }
        let (a1, b1, c1, d1) = (a / s1, b / s1, c / s2, d / s2);
        let sine = (a1 * d1 - b1 * c1) / (a1.hypot(b1) * c1.hypot(d1));
        if sine.abs() <= 1e-12 {
            return None;
        }
        let inv = 1.0 / det;
        let inverse = Affine { m: [d * inv, -b * inv, -c * inv, a * inv, (c * f - d * e) * inv, (b * e - a * f) * inv] };
        inverse.m.iter().all(|v| v.is_finite()).then_some(inverse)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_basics() {
        let r = Rect::from_xywh(10, 20, 30, 40);
        assert_eq!(r.width(), 30);
        assert_eq!(r.height(), 40);
        assert!(r.contains(10, 20));
        assert!(!r.contains(40, 20));
        assert!(Rect::new(5, 5, 5, 10).is_empty());
    }

    #[test]
    fn intersect_and_union() {
        let a = Rect::new(0, 0, 10, 10);
        let b = Rect::new(5, 5, 15, 15);
        assert_eq!(a.intersect(&b), Rect::new(5, 5, 10, 10));
        assert_eq!(a.union(&b), Rect::new(0, 0, 15, 15));
        assert_eq!(a.intersect(&Rect::new(20, 20, 30, 30)), Rect::EMPTY);
        assert_eq!(Rect::EMPTY.union(&a), a);
    }

    #[test]
    fn tile_coords_handle_negatives() {
        assert_eq!(TileCoord::containing(0, 0), TileCoord::new(0, 0));
        assert_eq!(TileCoord::containing(255, 255), TileCoord::new(0, 0));
        assert_eq!(TileCoord::containing(256, 0), TileCoord::new(1, 0));
        assert_eq!(TileCoord::containing(-1, -1), TileCoord::new(-1, -1));
        assert_eq!(TileCoord::containing(-256, -257), TileCoord::new(-1, -2));
    }

    #[test]
    fn rect_tiles_enumeration() {
        let r = Rect::new(-10, 0, 300, 10);
        let t: Vec<_> = r.tiles().collect();
        assert_eq!(t, vec![TileCoord::new(-1, 0), TileCoord::new(0, 0), TileCoord::new(1, 0)]);
        assert_eq!(Rect::EMPTY.tiles().count(), 0);
        assert_eq!(Rect::new(0, 0, 256, 256).tiles().count(), 1);
        assert_eq!(Rect::new(0, 0, 257, 257).tiles().count(), 4);
    }

    #[test]
    fn affine_inverse_roundtrip() {
        let t = Affine::translate(5.0, -3.0).then(&Affine::rotate(0.7)).then(&Affine::scale(2.5));
        let inv = t.inverse().unwrap();
        let p = Point::new(12.0, -7.5);
        let q = inv.apply(t.apply(p));
        assert!((q.x - p.x).abs() < 1e-9 && (q.y - p.y).abs() < 1e-9);
        assert!(Affine::scale(0.0).inverse().is_none());
    }

    #[test]
    fn affine_inverse_rejects_non_finite_coefficients() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            for index in 0..6 {
                let mut transform = Affine::IDENTITY;
                transform.m[index] = value;
                assert!(transform.inverse().is_none(), "coefficient {index}: {value}");
            }
        }
    }

    #[test]
    fn affine_inverse_rejects_determinant_overflow() {
        assert!(Affine::scale(f64::MAX).inverse().is_none());
    }

    #[test]
    fn affine_inverse_rejects_translation_overflow() {
        let transform = Affine { m: [1.0, 0.0, 1.0, 1.0, -f64::MAX, f64::MAX] };
        assert_eq!(transform.determinant(), 1.0);
        assert!(transform.inverse().is_none());
    }

    #[test]
    fn affine_inverse_keeps_large_finite_translations() {
        assert_eq!(Affine::translate(f64::MAX, -f64::MAX).inverse(), Some(Affine::translate(-f64::MAX, f64::MAX)));
    }

    #[test]
    fn affine_then_order() {
        let t = Affine::translate(10.0, 0.0).then(&Affine::scale(2.0));
        // translate first, then scale
        assert_eq!(t.apply(Point::new(1.0, 0.0)), Point::new(22.0, 0.0));
    }

    #[test]
    fn translate_and_inflate_saturate_on_overflow() {
        let r = Rect::new(i32::MAX - 10, i32::MIN + 10, i32::MAX, i32::MIN + 20);
        let t = r.translate(100, -100);
        assert_eq!(t.x0, i32::MAX);
        assert_eq!(t.x1, i32::MAX);
        assert_eq!(t.y0, i32::MIN);
        assert_eq!(t.y1, i32::MIN);
        let grown = Rect::new(i32::MAX - 5, i32::MIN + 5, i32::MAX - 1, i32::MIN + 10).inflate(10);
        assert_eq!(grown.x1, i32::MAX);
        assert_eq!(grown.y0, i32::MIN);
    }

    #[test]
    fn from_xywh_clamps_huge_sizes_instead_of_wrapping() {
        // Widths above `i32::MAX` used to wrap negative through `w as i32`, producing an
        // empty inverted rect instead of clamping the far edge to `i32::MAX`.
        assert_eq!(Rect::from_xywh(0, 0, u32::MAX, 10), Rect::new(0, 0, i32::MAX, 10));
        assert_eq!(Rect::from_xywh(-100, 5, u32::MAX, u32::MAX), Rect::new(-100, 5, i32::MAX, i32::MAX));
        assert_eq!(Rect::from_xywh(10, 20, i32::MAX as u32 + 1, 40).width(), i32::MAX as u32 - 10);
        // Ordinary sizes are unchanged.
        assert_eq!(Rect::from_xywh(10, 20, 30, 40), Rect::new(10, 20, 40, 60));
    }

    #[test]
    fn tile_rect_saturates_on_extreme_coords() {
        // Tile coords are deserialized from files; `tx * TILE_SIZE` used to overflow.
        let r = TileCoord::new(i32::MAX, i32::MIN).rect();
        assert_eq!((r.x0, r.y0), (i32::MAX, i32::MIN));
        assert_eq!(TileCoord::new(2, -3).rect(), Rect::new(512, -768, 768, -512));
    }

    #[test]
    fn width_and_height_do_not_overflow_on_extreme_edges() {
        // Inverted extreme rect: empty, so 0 (x1 - x0 used to overflow and panic).
        let r = Rect::new(i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        assert_eq!((r.width(), r.height()), (0, 0));
        assert!(r.is_empty());
        // Widest possible rect: the true extent fits in u32.
        let r = Rect::new(i32::MIN, i32::MIN + 20, i32::MAX, i32::MAX - 5);
        assert_eq!(r.width(), u32::MAX);
        assert_eq!(r.height(), u32::MAX - 25);
        assert_eq!(r.size(), Size::new(u32::MAX, u32::MAX - 25));
    }

    #[test]
    fn affine_inverse_accepts_uniformly_tiny_and_huge_scales() {
        // The singularity test used to be an absolute `|det| < 1e-12`, so a perfectly
        // well-conditioned `scale(1e-7)` (det 1e-14) was reported as singular.
        let p = Point::new(2.0, 5.0);
        // Pure scales across extreme magnitudes round-trip (relative error only).
        for s in [1e-7, 1e-9, 1e-30, 1e-100, 1e7, 1e100] {
            let t = Affine::scale(s);
            let inv = t.inverse().unwrap_or(Affine::IDENTITY);
            assert_ne!(inv, Affine::IDENTITY, "scale({s}) must be invertible");
            let q = inv.apply(t.apply(p));
            assert!((q.x - p.x).abs() < 1e-9 * p.x && (q.y - p.y).abs() < 1e-9 * p.y, "scale({s}): {q:?}");
        }
        // With a translation the same holds while the scale is not so small that the offset
        // swamps the scaled point in f64 (that loss is inherent, not the inverse's).
        for s in [1e-7, 1e-9, 1e7] {
            let t = Affine::scale(s).then(&Affine::translate(3.0, -4.0));
            let inv = t.inverse().unwrap_or(Affine::IDENTITY);
            assert_ne!(inv, Affine::IDENTITY, "scale({s}) + translate must be invertible");
            let q = inv.apply(t.apply(p));
            assert!((q.x - p.x).abs() < 1e-6 && (q.y - p.y).abs() < 1e-6, "scale({s}): {q:?}");
        }
    }

    #[test]
    fn affine_inverse_still_rejects_singular_matrices_at_any_magnitude() {
        // Parallel columns (rank 1) stay singular whatever the overall scale.
        for k in [1e-9, 1.0, 1e9] {
            let t = Affine { m: [k, 2.0 * k, 3.0 * k, 6.0 * k, 0.0, 0.0] };
            assert!(t.inverse().is_none(), "rank-1 at scale {k}");
        }
        // A zero column, or an all-zero linear part, is singular too.
        assert!(Affine { m: [0.0, 0.0, 1.0, 0.0, 5.0, 5.0] }.inverse().is_none());
        assert!(Affine { m: [0.0; 6] }.inverse().is_none());
        // Nearly parallel columns (angle ~1e-13 rad) are still rejected.
        assert!(Affine { m: [1.0, 0.0, 1.0, 1e-13, 0.0, 0.0] }.inverse().is_none());
        // ...while a modest shear stays invertible.
        assert!(Affine { m: [1.0, 0.0, 1.0, 1e-6, 0.0, 0.0] }.inverse().is_some());
    }

    #[test]
    fn affine_inverse_does_not_overflow_on_huge_finite_columns() {
        // The product of the raw column lengths is `inf` here, yet the matrix is invertible
        // (det ~ 5e307, finite) and the old absolute test accepted it.
        let t = Affine { m: [1e154, 1e154, 1e154, 1.5e154, 0.0, 0.0] };
        let inv = t.inverse().unwrap_or(Affine::IDENTITY);
        assert_ne!(inv, Affine::IDENTITY);
        assert!(inv.m.iter().all(|v| v.is_finite()));
        // Rank-1 matrices of the same magnitude are still singular.
        assert!(Affine { m: [1e154, 1e154, 2e154, 2e154, 0.0, 0.0] }.inverse().is_none());
        // And so is a zero column of any magnitude next to a huge one.
        assert!(Affine { m: [0.0, 0.0, 1e300, 1e300, 0.0, 0.0] }.inverse().is_none());
    }

    #[test]
    fn affine_then_applies_self_first() {
        // `a.then(b)` is "a, then b" (the doc comment used to claim the opposite).
        let a = Affine::translate(1.0, 0.0);
        let b = Affine::scale(10.0);
        assert_eq!(a.then(&b).apply(Point::new(0.0, 0.0)), Point::new(10.0, 0.0));
        assert_eq!(b.then(&a).apply(Point::new(0.0, 0.0)), Point::new(1.0, 0.0));
        assert_eq!(a.then(&b), b.mul(&a));
    }

    #[test]
    fn rect_intersects_matches_intersect() {
        let a = Rect::new(0, 0, 10, 10);
        assert!(a.intersects(&Rect::new(5, 5, 15, 15)));
        assert!(a.intersects(&a));
        // Half-open: touching edges do not overlap.
        assert!(!a.intersects(&Rect::new(10, 0, 20, 10)));
        assert!(!a.intersects(&Rect::new(0, 10, 10, 20)));
        // Empty and inverted rects overlap nothing, even when "inside" the other.
        assert!(!a.intersects(&Rect::new(5, 5, 5, 8)));
        assert!(!a.intersects(&Rect::new(8, 8, 2, 2)));
        assert!(!Rect::EMPTY.intersects(&a));
        // Extreme edges neither overflow nor panic.
        let wide = Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX);
        assert!(wide.intersects(&a) && a.intersects(&wide));
        // Agrees with `intersect` on a grid of cases.
        let cases = [Rect::new(-3, -3, 3, 3), Rect::new(9, 9, 12, 12), Rect::new(10, 10, 12, 12), Rect::new(-5, 2, 0, 4), Rect::new(1, 1, 2, 2)];
        for c in cases {
            assert_eq!(a.intersects(&c), !a.intersect(&c).is_empty(), "{c:?}");
        }
    }

    #[test]
    fn rect_center_is_exact_even_for_extreme_edges() {
        assert_eq!(Rect::new(0, 0, 10, 20).center(), Some(Point::new(5.0, 10.0)));
        assert_eq!(Rect::new(-5, -5, 0, 0).center(), Some(Point::new(-2.5, -2.5)));
        assert_eq!(Rect::new(1, 1, 2, 2).center(), Some(Point::new(1.5, 1.5)));
        // `x0 + x1` overflows i32; the midpoint must still be right.
        assert_eq!(Rect::new(i32::MAX - 2, 0, i32::MAX, 2).center(), Some(Point::new(f64::from(i32::MAX) - 1.0, 1.0)));
        assert_eq!(Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX).center(), Some(Point::new(-0.5, -0.5)));
        // An empty rect has no centre.
        assert_eq!(Rect::EMPTY.center(), None);
        assert_eq!(Rect::new(4, 4, 4, 9).center(), None);
    }

    #[test]
    fn affine_scale_xy_and_rotate_about() {
        let s = Affine::scale_xy(2.0, -3.0);
        assert_eq!(s.apply(Point::new(1.0, 1.0)), Point::new(2.0, -3.0));
        assert_eq!(Affine::scale_xy(4.0, 4.0), Affine::scale(4.0));
        // Rotating a quarter turn about (10, 10) sends (11, 10) to (10, 11) and fixes the centre.
        let c = Point::new(10.0, 10.0);
        let r = Affine::rotate_about(std::f64::consts::FRAC_PI_2, c);
        let near = |p: Point, x: f64, y: f64| (p.x - x).abs() < 1e-9 && (p.y - y).abs() < 1e-9;
        assert!(near(r.apply(c), 10.0, 10.0));
        assert!(near(r.apply(Point::new(11.0, 10.0)), 10.0, 11.0));
        // Zero rotation is the identity and the result stays invertible.
        assert!(near(Affine::rotate_about(0.0, c).apply(Point::new(3.0, 4.0)), 3.0, 4.0));
        let inv = r.inverse().unwrap_or(Affine::IDENTITY);
        assert!(near(inv.apply(r.apply(Point::new(7.0, -2.0))), 7.0, -2.0));
    }
}
