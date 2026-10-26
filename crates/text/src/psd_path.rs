//! Type on a path in a PSD (#7962): the path lives only in the type layer's frame in the
//! document's `Txt2` block (EngineData marks such a layer as box text, `ShapeType 1`, with the
//! path's bounds as its `BoxBounds`).
//!
//! Observed layout (Photoshop 2026, `text/path-circle.psd` and `text/path-wave-open.psd` in
//! photocraft-corpus): a frame `<< /0 << /1 << /0 [ … ] >> /2 << /0 2 /2 [ matrix ] … >> >> >>`
//! has the kind `/2 /0` (absent for point text, 1 for a text box, 2 for type on a path) and its
//! outline at `/1 /0` as a flat list of cubic Bézier segments, eight numbers each
//! (`p0 c1 c2 p1`, the next segment starting where the previous one ends, a closed path ending
//! where it started), in the frame's space; `/2 /2` maps the frame's space to the `TySh` text
//! space. On a closed path Photoshop sets the type from the path's last anchor (the circle's
//! glyph 0 sits on its fourth anchor and runs on through the first), on an open path from its
//! first. Neither oracle has a moved start or flipped type, so those are written into the path
//! itself (see [`frame_path`]).

use photocraft_doc::text::TextShape;
use photocraft_doc::vector::{Knot, PathOp, Subpath};
use photocraft_geom::{Affine, Point};

use crate::engine_data::Value as E;

/// The `Txt2` frame kind (`/2 /0`) of type on a path.
pub(crate) const PATH_FRAME: i64 = 2;
/// The `Txt2` frame kind (`/2 /0`) of a text box.
const BOX_FRAME: i64 = 1;
/// Most cubic segments read from a frame (a hostile file cannot make this large).
const MAX_SEGMENTS: usize = 4096;
/// How close (px) two points must be to count as the same: Photoshop writes a segment's start as
/// an exact copy of the previous segment's end.
const SAME: f64 = 1e-3;

/// The kind of the frame dictionary `frame` (the entry's `/0`): 0 point text, 1 box, 2 path.
pub(crate) fn frame_kind(frame: &E) -> i64 {
    frame.path(&["2", "0"]).and_then(E::as_i64).unwrap_or(0)
}

/// Every entry of `v` as a finite number, or `None`.
fn numbers(v: &E) -> Option<Vec<f64>> {
    v.as_array()?.iter().map(E::as_f64).collect()
}

fn same(a: Point, b: Point) -> bool {
    (a.x - b.x).hypot(a.y - b.y) <= SAME
}

/// Whether the handles `a` (in) and `b` (out) of `anchor` are collinear on opposite sides.
fn smooth(anchor: Point, a: Point, b: Point) -> bool {
    let (ax, ay, bx, by) = (a.x - anchor.x, a.y - anchor.y, b.x - anchor.x, b.y - anchor.y);
    let (la, lb) = (ax.hypot(ay), bx.hypot(by));
    la > SAME && lb > SAME && (ax * by - ay * bx).abs() <= 1e-3 * la * lb && ax * bx + ay * by < 0.0
}

fn knot(in_ctrl: Point, anchor: Point, out_ctrl: Point) -> Knot {
    Knot { anchor, in_ctrl, out_ctrl, smooth: smooth(anchor, in_ctrl, out_ctrl) }
}

/// The type path of a path frame `frame` (the entry's `/0`), in the `TySh` text space, with the
/// type starting at its first knot. `Err` says why it can't be read (the caller keeps the box or
/// point text EngineData describes).
pub(crate) fn read(frame: &E) -> Result<Subpath, String> {
    let raw = frame.path(&["1", "0"]).ok_or("the frame has no path")?;
    let v = numbers(raw).ok_or("a path coordinate is not a number")?;
    if v.is_empty() || v.len() % 8 != 0 {
        return Err(format!("its path has {} coordinates, not whole curve segments", v.len()));
    }
    if v.len() / 8 > MAX_SEGMENTS {
        return Err(format!("its path has more than {MAX_SEGMENTS} segments"));
    }
    let matrix = match frame.path(&["2", "2"]) {
        None => Affine::IDENTITY,
        Some(m) => {
            let Some([a, b, c, d, e, f]) = numbers(m).and_then(|m| <[f64; 6]>::try_from(m).ok()) else {
                return Err("its frame matrix is not six numbers".into());
            };
            let m = Affine { m: [a, b, c, d, e, f] };
            let det = m.determinant();
            if !det.is_finite() || det.abs() <= 1e-12 {
                return Err("its frame matrix is degenerate".into());
            }
            m
        }
    };
    let pts: Vec<Point> = v.as_chunks::<2>().0.iter().map(|&[x, y]| matrix.apply(Point::new(x, y))).collect();
    if pts.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
        return Err("its path is out of range".into());
    }
    let segs: Vec<[Point; 4]> = pts.as_chunks::<4>().0.to_vec();
    if segs.windows(2).any(|w| matches!(w, [a, b] if !same(a[3], b[0]))) {
        return Err("its path has more than one piece".into());
    }
    let (Some(first), Some(last)) = (segs.first(), segs.last()) else { return Err("the frame has no path".into()) };
    let (x0, x1) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| (lo.min(p.x), hi.max(p.x)));
    let (y0, y1) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| (lo.min(p.y), hi.max(p.y)));
    if x1 - x0 <= SAME && y1 - y0 <= SAME {
        return Err("its path has no length".into());
    }
    let closed = same(last[3], first[0]);
    let mut knots: Vec<Knot> = Vec::with_capacity(segs.len() + 1);
    for (i, s) in segs.iter().enumerate() {
        let prev = match i.checked_sub(1).and_then(|j| segs.get(j)) {
            Some(p) => p[2],
            None if closed => last[2],
            None => s[0],
        };
        knots.push(knot(prev, s[0], s[1]));
    }
    if closed {
        // The type starts on the closing segment: from the last anchor.
        knots.rotate_right(1);
    } else {
        knots.push(knot(last[2], last[3], last[3]));
    }
    Ok(Subpath { closed, knots, op: PathOp::Combine })
}

/// `path` with its knots in the opposite order (the segments run backwards).
fn reversed(path: &Subpath) -> Subpath {
    let mut knots: Vec<Knot> = path.knots.iter().rev().map(|k| Knot { anchor: k.anchor, in_ctrl: k.out_ctrl, out_ctrl: k.in_ctrl, smooth: k.smooth }).collect();
    if path.closed {
        // Keep the first knot first: k0, k(n-1), …, k1.
        knots.rotate_right(1);
    }
    Subpath { closed: path.closed, knots, op: path.op }
}

fn lerp(a: Point, b: Point, t: f64) -> Point {
    Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
}

fn cubic(s: &[Point; 4], t: f64) -> Point {
    let (a, b, c) = (lerp(s[0], s[1], t), lerp(s[1], s[2], t), lerp(s[2], s[3], t));
    lerp(lerp(a, b, t), lerp(b, c, t), t)
}

/// Pieces each segment is measured in for [`starting_at`].
const STEPS: usize = 64;

/// `path` starting `s` px along it (the type origin moved there): an open path loses the part
/// before `s`, a closed one starts at a new smooth knot there. Unchanged for `s` at or before the
/// start, or (open) at or past the end.
fn starting_at(path: &Subpath, s: f64) -> Subpath {
    let segs = path.segments();
    let lengths: Vec<f64> = segs
        .iter()
        .map(|seg| {
            (1..=STEPS)
                .map(|i| (cubic(seg, (i - 1) as f64 / STEPS as f64), cubic(seg, i as f64 / STEPS as f64)))
                .map(|(a, b)| (b.x - a.x).hypot(b.y - a.y))
                .sum()
        })
        .collect();
    let total: f64 = lengths.iter().sum();
    if !s.is_finite() || !total.is_finite() || total <= SAME {
        return path.clone();
    }
    let s = if path.closed { s.rem_euclid(total) } else { s };
    if s <= SAME || (!path.closed && s >= total - SAME) || (path.closed && s >= total - SAME) {
        return path.clone();
    }
    // The segment and parameter at arc length `s` (linear within a measured piece).
    let mut left = s;
    let mut at = None;
    for (i, (seg, len)) in segs.iter().zip(&lengths).enumerate() {
        if left > *len {
            left -= len;
            continue;
        }
        let mut walked = 0.0;
        for k in 1..=STEPS {
            let (t0, t1) = ((k - 1) as f64 / STEPS as f64, k as f64 / STEPS as f64);
            let (a, b) = (cubic(seg, t0), cubic(seg, t1));
            let piece = (b.x - a.x).hypot(b.y - a.y);
            if walked + piece >= left || k == STEPS {
                let f = if piece > 0.0 { ((left - walked) / piece).clamp(0.0, 1.0) } else { 0.0 };
                at = Some((i, t0 + (t1 - t0) * f, *seg));
                break;
            }
            walked += piece;
        }
        break;
    }
    let Some((i, t, seg)) = at else { return path.clone() };
    // de Casteljau: left half p0 a ab m, right half m bc c p3.
    let (a, b, c) = (lerp(seg[0], seg[1], t), lerp(seg[1], seg[2], t), lerp(seg[2], seg[3], t));
    let (ab, bc) = (lerp(a, b, t), lerp(b, c, t));
    let m = lerp(ab, bc, t);
    let n = path.knots.len();
    if n == 0 {
        return path.clone();
    }
    let next = (i + 1) % n;
    let mut knots = Vec::with_capacity(n + 1);
    knots.push(Knot { anchor: m, in_ctrl: if path.closed { ab } else { m }, out_ctrl: bc, smooth: path.closed });
    let count = if path.closed { n } else { n - next };
    for r in 0..count {
        let j = (next + r) % n;
        let Some(mut k) = path.knots.get(j).copied() else { continue };
        if j == next {
            k.in_ctrl = c;
        }
        if path.closed && j == i {
            k.out_ctrl = a;
        }
        knots.push(k);
    }
    Subpath { closed: path.closed, knots, op: path.op }
}

/// The path a frame is written with for `TextShape::Path { path, start, flip }`: flipped type is
/// the path run backwards and a moved start the path from there on (exactly how PhotoCraft lays
/// type on a path out), so Photoshop, which starts at the path's beginning, sets the same type.
pub(crate) fn frame_path(path: &Subpath, start: f32, flip: bool) -> Subpath {
    let path = if flip { reversed(path) } else { path.clone() };
    starting_at(&path, f64::from(start))
}

fn real(v: f64) -> E {
    E::Real(v)
}

/// The segment list (`/1 /0`) Photoshop reads `path` from: on a closed path the type starts at
/// the last anchor, so the first knot goes last.
fn segment_list(path: &Subpath) -> Option<Vec<E>> {
    let mut knots = path.knots.clone();
    if path.closed && !knots.is_empty() {
        knots.rotate_left(1);
    }
    let segs = Subpath { closed: path.closed, knots, op: path.op }.segments();
    let out: Vec<E> = segs.iter().flatten().flat_map(|p| [real(p.x), real(p.y)]).collect();
    (!out.is_empty() && segs.iter().flatten().all(|p| p.x.is_finite() && p.y.is_finite())).then_some(out)
}

/// Whether two type paths are the same within [`SAME`] (the smooth flags aside).
fn same_path(a: &Subpath, b: &Subpath) -> bool {
    a.closed == b.closed
        && a.knots.len() == b.knots.len()
        && a.knots.iter().zip(&b.knots).all(|(p, q)| same(p.anchor, q.anchor) && same(p.in_ctrl, q.in_ctrl) && same(p.out_ctrl, q.out_ctrl))
}

/// The point-text frame Photoshop writes (`<< /0 << /2 << /6 [ -1.0 -1.0 ] /11 << /4 -1 /7 false
/// >> >> >> >>`; also what [`crate::psd`] pads the frame list with).
pub(crate) fn point_frame() -> E {
    let kind = E::Dict(vec![("4".into(), E::Int(-1)), ("7".into(), E::Bool(false))]);
    let data = E::Dict(vec![("6".into(), E::Array(vec![real(-1.0), real(-1.0)])), ("11".into(), kind)]);
    E::Dict(vec![("0".into(), E::Dict(vec![("2".into(), data)]))])
}

/// A text box's frame as Photoshop writes it (`text/paragraph-box-left.psd`): the rectangle as
/// four straight segments from its top left, clockwise.
fn box_frame(x: f64, y: f64, w: f64, h: f64) -> E {
    let c = [(x, y), (x + w, y), (x + w, y + h), (x, y + h), (x, y)];
    let pts: Vec<E> =
        c.windows(2).flat_map(|p| p.iter().flat_map(|&(a, b)| [(a, b), (a, b)]).collect::<Vec<_>>()).flat_map(|(a, b)| [real(a), real(b)]).collect();
    let kind = E::Dict(vec![("4".into(), E::Int(-2)), ("18".into(), real(-2.0)), ("22".into(), real(0.025))]);
    let data = E::Dict(vec![("0".into(), E::Int(BOX_FRAME)), ("6".into(), E::Array(vec![real(-2.0), real(-2.0)])), ("11".into(), kind)]);
    E::Dict(vec![("0".into(), E::Dict(vec![("1".into(), E::Dict(vec![("0".into(), E::Array(pts))])), ("2".into(), data)]))])
}

/// Brings the frame list entry `entry` (`<< /0 frame >>`) of a type layer up to date with its
/// `shape`: type on a path gets a path frame for its path (left byte for byte when it already
/// holds that path, e.g. Photoshop's own after an unedited save), and a layer no longer on a path
/// loses its path frame (else it would reopen on the path).
pub(crate) fn update_frame(entry: &mut E, shape: &TextShape) {
    let frame = entry.get("0");
    let kind = frame.map_or(0, frame_kind);
    match shape {
        TextShape::Path { path, start, flip } => {
            let path = frame_path(path, *start, *flip);
            if kind == PATH_FRAME && frame.and_then(|f| read(f).ok()).is_some_and(|p| same_path(&p, &path)) {
                return;
            }
            let Some(list) = segment_list(&path) else { return };
            // Keep the other keys of a path frame; anything else becomes one.
            let mut data = match frame.and_then(|f| f.get("2")) {
                Some(d @ E::Dict(_)) if kind == PATH_FRAME => d.clone(),
                _ => E::Dict(vec![("6".into(), E::Array(vec![real(-1.0), real(-1.0)]))]),
            };
            data.set("0", E::Int(PATH_FRAME));
            // The path is written in text space.
            data.set("2", E::Array([1.0, 0.0, 0.0, 1.0, 0.0, 0.0].map(real).to_vec()));
            let mut frame = match frame {
                Some(f @ E::Dict(_)) if kind == PATH_FRAME => f.clone(),
                _ => E::dict(),
            };
            frame.set("1", E::Dict(vec![("0".into(), E::Array(list))]));
            frame.set("2", data);
            *entry = E::Dict(vec![("0".into(), frame)]);
        }
        // Leave unreadable path frames unchanged. They were not imported as detached paths.
        _ if kind != PATH_FRAME || frame.is_none_or(|f| read(f).is_err()) => {}
        TextShape::Box { x, y, width, height } => *entry = box_frame(f64::from(*x), f64::from(*y), f64::from(*width), f64::from(*height)),
        TextShape::Point => *entry = point_frame(),
        // Area type saves as a text box over its path's bounds, as its EngineData does.
        TextShape::Area { path } => {
            *entry = match crate::area::path_bounds(path) {
                Some((x0, y0, x1, y1)) => box_frame(x0, y0, x1 - x0, y1 - y0),
                None => point_frame(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine_data::parse;

    fn frame(src: &str) -> E {
        parse(src.as_bytes()).unwrap()
    }

    /// The frame of `text/path-circle.psd` (Photoshop 2026).
    const CIRCLE: &str = "<< /1 << /0 [ 32.0 128.0 32.0 74.98067 74.98067 32.0 128.0 32.0 128.0 32.0 181.01933 32.0 224.0 74.98067 224.0 128.0 224.0 128.0 224.0 181.01933 181.01933 224.0 128.0 224.0 128.0 224.0 74.98067 224.0 32.0 181.01933 32.0 128.0 ] >> /2 << /0 2 /2 [ 1.0 0.0 0.0 1.0 -32.0 -32.0 ] /6 [ -1.0 -1.0 ] >> >>";
    /// The frame of `text/path-wave-open.psd` (Photoshop 2026).
    const WAVE: &str = "<< /1 << /0 [ 16.0 100.0 46.0 40.0 120.0 40.0 160.0 80.0 160.0 80.0 200.0 120.0 274.0 100.0 304.0 60.0 ] >> /2 << /0 2 /2 [ 1.0 0.0 0.0 1.0 -16.0 -52.12246 ] /6 [ -1.0 -1.0 ] >> >>";

    #[test]
    fn photoshop_path_frames_read_into_text_space() {
        let wave = read(&frame(WAVE)).unwrap();
        assert!(!wave.closed);
        assert_eq!(wave.knots.len(), 3);
        let p = |x: f64, y: f64| Point::new(x - 16.0, y - 52.12246);
        let k = &wave.knots;
        assert!(same(k[0].anchor, p(16.0, 100.0)) && same(k[0].in_ctrl, p(16.0, 100.0)) && same(k[0].out_ctrl, p(46.0, 40.0)));
        assert!(same(k[1].in_ctrl, p(120.0, 40.0)) && same(k[1].anchor, p(160.0, 80.0)) && same(k[1].out_ctrl, p(200.0, 120.0)) && k[1].smooth);
        assert!(same(k[2].in_ctrl, p(274.0, 100.0)) && same(k[2].anchor, p(304.0, 60.0)));

        let circle = read(&frame(CIRCLE)).unwrap();
        assert!(circle.closed);
        assert_eq!(circle.knots.len(), 4);
        // The type starts at the bottom anchor (Photoshop's glyph 0) and runs to the left one.
        assert!(same(circle.knots[0].anchor, Point::new(96.0, 192.0)));
        assert!(same(circle.knots[1].anchor, Point::new(0.0, 96.0)));
        assert!(circle.knots.iter().all(|k| k.smooth));
    }

    #[test]
    fn written_frames_read_back() {
        for src in [CIRCLE, WAVE] {
            let path = read(&frame(src)).unwrap();
            let mut entry = E::Dict(vec![("0".into(), frame(src))]);
            let before = entry.clone();
            update_frame(&mut entry, &TextShape::Path { path: path.clone(), start: 0.0, flip: false });
            assert_eq!(entry, before, "an unchanged path keeps Photoshop's frame");
            let mut moved = path.clone();
            for k in &mut moved.knots {
                *k = k.transform(&Affine::translate(5.0, -3.0));
            }
            update_frame(&mut entry, &TextShape::Path { path: moved.clone(), start: 0.0, flip: false });
            let back = read(entry.get("0").unwrap()).unwrap();
            assert!(same_path(&back, &moved), "{back:?}");
            assert_eq!(entry.path(&["0", "2", "6"]), before.path(&["0", "2", "6"]), "the frame's other keys stay");
        }
    }

    /// Flipped type and a moved start are written into the path, which lays the type out the same.
    #[test]
    fn flip_and_start_are_baked_into_the_frame() {
        use crate::warp::Warp;
        for src in [CIRCLE, WAVE] {
            let path = read(&frame(src)).unwrap();
            for (start, flip) in [(40.0, false), (0.0, true), (25.0, true)] {
                let shape = TextShape::Path { path: path.clone(), start, flip };
                let baked = TextShape::Path { path: frame_path(&path, start, flip), start: 0.0, flip: false };
                let (a, b) = (Warp::on_path(&shape).unwrap(), Warp::on_path(&baked).unwrap());
                for x in [0.0, 10.0, 50.0, 120.0] {
                    for y in [-12.0, 0.0, 4.0] {
                        let (p, q) = (a.apply(x, y), b.apply(x, y));
                        assert!((p.0 - q.0).hypot(p.1 - q.1) < 0.5, "{src:.10} start {start} flip {flip} at ({x}, {y}): {p:?} vs {q:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn hostile_frames_are_errors() {
        for src in [
            "<< /2 << /0 2 >> >>",
            "<< /1 << /0 [ ] >> /2 << /0 2 >> >>",
            "<< /1 << /0 [ 1.0 2.0 3.0 ] >> /2 << /0 2 >> >>",
            "<< /1 << /0 [ 0 0 1 1 2 2 (x) 3 ] >> /2 << /0 2 >> >>",
            "<< /1 << /0 [ 0 0 1 1 2 2 1e999 3 ] >> /2 << /0 2 >> >>",
            "<< /1 << /0 [ 0 0 1 1 2 2 3 3 ] >> /2 << /0 2 /2 [ 0 0 0 0 0 0 ] >> >>",
            "<< /1 << /0 [ 0 0 1 1 2 2 3 3 ] >> /2 << /0 2 /2 [ 1 0 0 ] >> >>",
            "<< /1 << /0 [ 0 0 1 1 2 2 3 3 ] >> /2 << /0 2 /2 [ 1e300 0 0 1e300 0 0 ] >> >>",
            "<< /1 << /0 [ 0 0 1 1 2 2 3 3 50 50 60 60 70 70 80 80 ] >> /2 << /0 2 >> >>",
            "<< /1 << /0 [ 5 5 5 5 5 5 5 5 ] >> /2 << /0 2 >> >>",
            "<< /1 << /0 (path) >> /2 << /0 2 >> >>",
        ] {
            assert!(read(&frame(src)).is_err(), "{src}");
        }
        let many = format!("<< /1 << /0 [ {} ] >> /2 << /0 2 >> >>", "0 0 1 1 2 2 0 0 ".repeat(MAX_SEGMENTS + 1));
        assert!(read(&frame(&many)).is_err());
    }

    fn d(items: Vec<(&str, E)>) -> E {
        E::Dict(items.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    /// A Photoshop-shaped `Txt2` block with one point-text object for `text` (`\r` breaks, trailing
    /// `\r`) drawn in frame 0.
    fn photoshop_txt2(text: &str) -> Vec<u8> {
        photoshop_txt2_in(text, point_frame())
    }

    /// [`photoshop_txt2`] with the frame list entry `frame`.
    fn photoshop_txt2_in(text: &str, frame: E) -> Vec<u8> {
        photoshop_txt2_of(vec![photoshop_object(text, 0)], vec![frame])
    }

    /// A Photoshop-shaped `Txt2` block holding `objects` and the frame list `frames`.
    fn photoshop_txt2_of(objects: Vec<E>, frames: Vec<E>) -> Vec<u8> {
        crate::engine_data::write_bare(&[
            ("98".into(), d(vec![("0", E::Int(14))])),
            ("0".into(), d(vec![("8", d(vec![("0", E::Array(frames))]))])),
            ("1".into(), d(vec![("1", E::Array(objects))])),
        ])
    }

    /// A Photoshop-shaped text object for `text` drawn in frame `at`.
    fn photoshop_object(text: &str, at: i64) -> E {
        let len = text.encode_utf16().count() as i64;
        let run = || {
            d(vec![("0", d(vec![("0", d(vec![("0", E::String(String::new())), ("6", d(vec![("0", E::Int(0)), ("1", E::Real(12.0))]))]))])), ("1", E::Int(len))])
        };
        let model = d(vec![("0", E::String(text.into())), ("5", d(vec![("0", E::Array(vec![run()]))])), ("6", d(vec![("0", E::Array(vec![run()]))]))]);
        let strike = d(vec![("99", E::Name("PC".into())), ("6", E::Array(vec![d(vec![("99", E::Name("F".into())), ("10", E::Int(at))])]))]);
        let view = d(vec![
            ("0", E::Array(vec![d(vec![("0", E::Int(at))])])),
            ("1", d(vec![("0", E::Array(vec![d(vec![("0", d(vec![("1", E::Int(1))])), ("1", E::Int(len))])]))])),
            ("2", E::Array(vec![strike])),
        ]);
        d(vec![("0", model), ("1", view)])
    }

    fn layer(text: &str, shape: TextShape) -> photocraft_doc::TextLayer {
        use photocraft_doc::text::{CharStyle, TextRun};
        let mut t = photocraft_doc::TextLayer {
            text: text.into(),
            runs: vec![TextRun { len: text.len(), style: CharStyle { size_pt: 12.0, ..Default::default() } }],
            shape,
            transform: Affine::translate(32.0, 20.0),
            ..Default::default()
        };
        t.sync_summary();
        t
    }

    /// A layer read back, with what [`crate::psd::apply_txt2_path`] said.
    type Opened = (photocraft_doc::TextLayer, Result<bool, String>);

    /// Saves `layers` (object `i` each) into a file whose `Txt2` is `prev`, then reads each back:
    /// its `TySh` and what [`crate::psd::apply_txt2_path`] makes of the saved `Txt2`.
    fn save_and_open(layers: &[&photocraft_doc::TextLayer], prev: &[u8]) -> (Vec<u8>, Vec<Opened>) {
        let objects: Vec<(i32, &photocraft_doc::TextLayer)> = layers.iter().enumerate().map(|(i, l)| (i as i32, *l)).collect();
        let txt2 = crate::psd::build_txt2(&objects, Some(prev));
        let parsed = crate::psd::parse_txt2(&txt2).unwrap();
        let back = objects
            .iter()
            .map(|(i, l)| {
                let tysh = crate::psd::set_text_index(&crate::psd::build_tysh(l, 72.0, None), *i).unwrap();
                let mut back = crate::psd::text_layer_from_tysh(&tysh, 72.0).unwrap();
                crate::psd::apply_txt2(&mut back, &tysh, &parsed);
                let applied = crate::psd::apply_txt2_path(&mut back, &tysh, &parsed);
                crate::psd::apply_txt2_frame(&mut back, &tysh, &parsed);
                (back, applied)
            })
            .collect();
        (txt2, back)
    }

    /// #7962: type on a path survives a PSD save and open, for the file's own text object and for
    /// a layer the file had no object for; a moved start and flipped type come back laid out the
    /// same, in the path; a second save changes nothing.
    #[test]
    fn path_text_round_trips_through_tysh_and_txt2() {
        let circle = read(&frame(CIRCLE)).unwrap();
        let wave = read(&frame(WAVE)).unwrap();
        let a = layer("On a circle", TextShape::Path { path: circle.clone(), start: 0.0, flip: false });
        let b = layer("Along a wave", TextShape::Path { path: wave.clone(), start: 30.0, flip: true });
        let prev = photoshop_txt2("On a circle\r");
        let (txt2, back) = save_and_open(&[&a, &b], &prev);
        for ((l, applied), (want, start, flip)) in back.iter().zip([(&circle, 0.0, false), (&wave, 30.0, true)]) {
            assert_eq!(applied, &Ok(true), "{}", l.text);
            let TextShape::Path { path, start: s, flip: f } = &l.shape else { panic!("{}: {:?}", l.text, l.shape) };
            assert_eq!((*s, *f), (0.0, false));
            assert!(same_path(path, &frame_path(want, start, flip)), "{}: {path:?}", l.text);
            assert_eq!(l.transform, Affine::translate(32.0, 20.0), "the frame's matrix is not applied twice");
        }
        let (again, _) = save_and_open(&[&back[0].0, &back[1].0], &txt2);
        assert_eq!(again, txt2, "a re-save of the opened file changes nothing");
        // Off its path, the layer opens as point text again.
        let detached = layer("On a circle", TextShape::Point);
        let (_, back) = save_and_open(&[&detached], &txt2);
        assert_eq!(back[0].1, Ok(false));
        assert_eq!(back[0].0.shape, TextShape::Point);
    }

    /// A malformed path frame leaves the layer as EngineData describes it, with the reason.
    #[test]
    fn a_malformed_path_frame_keeps_the_engine_data_shape() {
        let a = layer("Hostile", TextShape::Box { x: 0.0, y: 0.0, width: 50.0, height: 20.0 });
        let tysh = crate::psd::set_text_index(&crate::psd::build_tysh(&a, 72.0, None), 0).unwrap();
        for bad in ["<< /1 << /0 [ 1 2 3 ] >> /2 << /0 2 >> >>", "<< /1 << /0 [ 0 0 1 1 2 2 3 3 ] >> /2 << /0 2 /2 [ 0 0 0 0 0 0 ] >> >>"] {
            let entry = E::Dict(vec![("0".into(), frame(bad))]);
            let txt2 = crate::psd::parse_txt2(&photoshop_txt2_in("Hostile\r", entry)).unwrap();
            let mut l = crate::psd::text_layer_from_tysh(&tysh, 72.0).unwrap();
            let before = l.shape.clone();
            assert!(crate::psd::apply_txt2_path(&mut l, &tysh, &txt2).is_err(), "{bad}");
            assert_eq!(l.shape, before);
        }
    }

    /// A layer taken off its path doesn't keep its path frame (it would reopen on the path).
    #[test]
    fn a_layer_off_its_path_loses_the_path_frame() {
        let mut entry = E::Dict(vec![("0".into(), frame(CIRCLE))]);
        update_frame(&mut entry, &TextShape::Point);
        assert_eq!(entry, point_frame());
        let mut entry = E::Dict(vec![("0".into(), frame(WAVE))]);
        update_frame(&mut entry, &TextShape::Box { x: 1.0, y: 2.0, width: 30.0, height: 40.0 });
        assert_eq!(entry.get("0").map(frame_kind), Some(BOX_FRAME));
        assert_eq!(
            entry.path(&["0", "1", "0"]).and_then(numbers).unwrap()[..8],
            [1.0, 2.0, 1.0, 2.0, 31.0, 2.0, 31.0, 2.0],
            "Photoshop's box outline: corners as straight segments from the top left"
        );
        // A point or box frame is left alone.
        let mut point = point_frame();
        update_frame(&mut point, &TextShape::Point);
        assert_eq!(point, point_frame());
    }

    /// A path frame the import could not read (the layer opened as EngineData's box or point
    /// text) is saved back as it was, not replaced by a box or point frame.
    #[test]
    fn an_unreadable_path_frame_is_kept() {
        let two_pieces = "<< /1 << /0 [ 0 0 1 1 2 2 3 3 50 50 60 60 70 70 80 80 ] >> /2 << /0 2 >> >>";
        for shape in [TextShape::Point, TextShape::Box { x: 0.0, y: 0.0, width: 80.0, height: 80.0 }] {
            let mut entry = E::Dict(vec![("0".into(), frame(two_pieces))]);
            let before = entry.clone();
            update_frame(&mut entry, &shape);
            assert_eq!(entry, before, "{shape:?}");
        }
    }

    /// The frame number the object at `index` of the saved `txt2` names, and that frame.
    fn saved_frame(txt2: &[u8], index: usize) -> (i64, E) {
        let t = crate::psd::parse_txt2(txt2).unwrap();
        let object = &t.path(&["1", "1"]).and_then(E::as_array).unwrap()[index];
        let at = object.path(&["1", "0"]).and_then(E::as_array).unwrap()[0].get("0").and_then(E::as_i64).unwrap();
        (at, t.path(&["0", "8", "0"]).and_then(E::as_array).unwrap()[at as usize].clone())
    }

    /// An edited layer whose object the save can't refit keeps drawing in that object's own frame,
    /// not in the frame its slot number names, which is the path layer's here.
    #[test]
    fn an_object_rebuilt_from_a_template_keeps_its_own_frame() {
        let circle = read(&frame(CIRCLE)).unwrap();
        let on_path = layer("On a circle", TextShape::Path { path: circle, start: 0.0, flip: false });
        // Object 1's style run has a length that is no count, so it can't be refitted.
        let mut broken = photoshop_object("Old\r", 0);
        let run = d(vec![("0", d(vec![("0", d(vec![("0", E::String(String::new()))]))])), ("1", E::String("x".into()))]);
        broken.get_mut("0").unwrap().set("6", d(vec![("0", E::Array(vec![run]))]));
        let path_frame = E::Dict(vec![("0".into(), frame(CIRCLE))]);
        let prev = photoshop_txt2_of(vec![photoshop_object("On a circle\r", 1), broken], vec![point_frame(), path_frame.clone()]);
        let (txt2, back) = save_and_open(&[&on_path, &layer("New", TextShape::Point)], &prev);
        assert_eq!(saved_frame(&txt2, 1).0, 0, "the rebuilt object draws in its own frame");
        assert_eq!(saved_frame(&txt2, 0), (1, path_frame));
        assert_eq!(back[0].1, Ok(true));
        assert!(matches!(back[0].0.shape, TextShape::Path { .. }), "{:?}", back[0].0.shape);
    }

    /// A new layer whose slot number is the frame another object is drawn in gets a frame of its
    /// own: neither layer takes the other's shape, on this save or the next.
    #[test]
    fn a_new_object_gets_a_frame_no_other_object_is_drawn_in() {
        let circle = read(&frame(CIRCLE)).unwrap();
        let on_path = layer("On a circle", TextShape::Path { path: circle, start: 0.0, flip: false });
        let path_frame = E::Dict(vec![("0".into(), frame(CIRCLE))]);
        let prev = photoshop_txt2_of(vec![photoshop_object("On a circle\r", 1)], vec![point_frame(), path_frame.clone()]);
        let new = layer("New", TextShape::Point);
        let (txt2, back) = save_and_open(&[&on_path, &new], &prev);
        assert_eq!(saved_frame(&txt2, 1), (2, point_frame()), "a frame past the file's own");
        assert_eq!(saved_frame(&txt2, 0), (1, path_frame));
        assert_eq!(back[0].1, Ok(true));
        assert_eq!((&back[1].1, &back[1].0.shape), (&Ok(false), &TextShape::Point));
        // The path layer taken off its path on the next save loses its path frame.
        let (again, _) = save_and_open(&[&layer("On a circle", TextShape::Point), &new], &txt2);
        assert_eq!(saved_frame(&again, 0), (1, point_frame()));
    }

    /// A rebuilt object whose old frame number is past the frame list doesn't name a frame the
    /// block lacks (Photoshop refuses such files, #7698).
    #[test]
    fn a_rebuilt_object_never_names_a_missing_frame() {
        let mut broken = photoshop_object("Old\r", 70_000);
        let run = d(vec![("0", d(vec![("0", d(vec![("0", E::String(String::new()))]))])), ("1", E::String("x".into()))]);
        broken.get_mut("0").unwrap().set("6", d(vec![("0", E::Array(vec![run]))]));
        let path_frame = E::Dict(vec![("0".into(), frame(CIRCLE))]);
        let prev = photoshop_txt2_of(vec![photoshop_object("On a circle\r", 1), broken], vec![point_frame(), path_frame.clone()]);
        let on_path = layer("On a circle", TextShape::Path { path: read(&frame(CIRCLE)).unwrap(), start: 0.0, flip: false });
        let (txt2, _) = save_and_open(&[&on_path, &layer("New", TextShape::Point)], &prev);
        assert_eq!(saved_frame(&txt2, 1), (2, point_frame()));
        assert_eq!(saved_frame(&txt2, 0), (1, path_frame));
    }

    /// An object the file keeps that names a frame past the frame list doesn't grow the list (to
    /// 65536 point frames here).
    #[test]
    fn a_kept_object_naming_a_far_frame_adds_no_frames() {
        let prev = photoshop_txt2_of(vec![photoshop_object("Far\r", crate::psd::MAX_TEXT_INDEX.into())], vec![point_frame()]);
        let out = crate::psd::parse_txt2(&crate::psd::build_txt2(&[(0, &layer("Far", TextShape::Point))], Some(&prev))).unwrap();
        assert_eq!(out.path(&["0", "8", "0"]).and_then(E::as_array).map(<[E]>::len), Some(1));
    }

    /// A frame the file already has two objects drawn in is left as the file has it, not given
    /// the shape of whichever layer comes last.
    #[test]
    fn a_frame_two_objects_name_is_kept() {
        let circle = read(&frame(CIRCLE)).unwrap();
        let on_path = layer("On a circle", TextShape::Path { path: circle, start: 0.0, flip: false });
        let path_frame = E::Dict(vec![("0".into(), frame(CIRCLE))]);
        let objects = vec![photoshop_object("On a circle\r", 1), photoshop_object("Point\r", 1)];
        let prev = photoshop_txt2_of(objects, vec![point_frame(), path_frame.clone()]);
        let (txt2, _) = save_and_open(&[&on_path, &layer("Point", TextShape::Point)], &prev);
        assert_eq!(saved_frame(&txt2, 0), (1, path_frame));
        assert_eq!(saved_frame(&txt2, 1).0, 1);
    }

    /// Area type's frame becomes a text box over its path's bounds, as its EngineData describes it.
    #[test]
    fn an_area_layer_gets_a_box_frame() {
        let square =
            photocraft_doc::vector::Path::new(vec![photocraft_doc::vector::Subpath::polygon(&[(10.0, 20.0), (50.0, 20.0), (50.0, 80.0), (10.0, 80.0)])]);
        let mut entry = E::Dict(vec![("0".into(), frame(CIRCLE))]);
        update_frame(&mut entry, &TextShape::Area { path: square });
        assert_eq!(entry, box_frame(10.0, 20.0, 40.0, 60.0));
    }
}
