//! Click-local red-eye correction: replace a red pupil around a click with a dark neutral grey.
//!
//! The kernel never scans a full document. Callers pass the search window ([`SEARCH_RADIUS`]
//! around the click) plus a [`halo_radius`] margin. Pupil Size sets how wide the dark core is and
//! Darken Amount how dark it gets; neither changes how far the click searches. The pupil edge is
//! soft because the darkening fades out towards it, not because the mask is blurred: a blurred
//! mask would tint the pixels around the pupil (sclera, iris) instead of leaving them alone.

#![allow(clippy::needless_range_loop)]

/// Largest window (search + halo) the engine may allocate, in pixels on a side.
pub const MAX_WINDOW: u32 = 512;
/// Clicks one command may apply in a single history step.
pub const MAX_POINTS: usize = 64;
/// How far from the click the pupil is searched, in document pixels (any Pupil Size).
pub const SEARCH_RADIUS: f32 = 64.0;

/// How far outside the search window the engine reads: a 1 px margin. Nothing in the kernel
/// filters the mask, so no pixel outside the search circle affects the result.
pub fn halo_radius() -> i32 {
    1
}

/// Range check for a raw Pupil Size: the read rect (`4 + 1.2 × size` + halo) the value would have
/// needed when Pupil Size set the search radius, before clamping it to 1..=100. The engine keeps
/// rejecting values whose side would exceed [`MAX_WINDOW`].
///
/// `None` when `pupil_size` is non-finite or the side would overflow [`MAX_WINDOW`].
pub fn unclamped_window_side(pupil_size: f32) -> Option<u32> {
    if !pupil_size.is_finite() {
        return None;
    }
    let r = 4.0 + 1.20 * pupil_size;
    if !r.is_finite() {
        return None;
    }
    let half = r.abs() + halo_radius() as f32;
    if !half.is_finite() || half > (MAX_WINDOW as f32) {
        return None;
    }
    let side = (2.0 * half + 1.0).ceil();
    if !side.is_finite() || side > MAX_WINDOW as f32 {
        return None;
    }
    Some(side as u32)
}

fn luma(p: [f32; 4]) -> f32 {
    0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]
}

fn is_red_eye(p: [f32; 4]) -> bool {
    let (r, g, b) = (p[0], p[1], p[2]);
    if !(r.is_finite() && g.is_finite() && b.is_finite()) {
        return false;
    }
    if r <= g + 0.15 || r <= b + 0.15 {
        return false;
    }
    if luma(p) >= 0.92 {
        return false;
    }
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    max - min >= 0.08
}

/// A dark, neutral pixel: what a corrected pupil looks like.
fn is_dark_neutral(p: [f32; 4]) -> bool {
    let max = p[0].max(p[1]).max(p[2]);
    let min = p[0].min(p[1]).min(p[2]);
    max.is_finite() && min.is_finite() && luma(p) < 0.35 && max - min < 0.08
}

fn idx(x: usize, y: usize, w: usize) -> Option<usize> {
    y.checked_mul(w).and_then(|row| row.checked_add(x))
}

fn within_search(dx: i32, dy: i32, radius: f32) -> bool {
    let d = (dx as f32).hypot(dy as f32);
    d.is_finite() && d <= radius
}

fn nearest_seed(mask: &[bool], w: usize, h: usize, origin: (i32, i32), click: (i32, i32), radius: f32) -> Option<usize> {
    let mut best: Option<(usize, i64)> = None;
    for y in 0..h {
        for x in 0..w {
            let Some(i) = idx(x, y, w) else { continue };
            if !mask.get(i).copied().unwrap_or(false) {
                continue;
            }
            let dx = origin.0.saturating_add(x as i32).saturating_sub(click.0);
            let dy = origin.1.saturating_add(y as i32).saturating_sub(click.1);
            if !within_search(dx, dy, radius) {
                continue;
            }
            let d2 = (dx as i64).saturating_mul(dx as i64).saturating_add((dy as i64).saturating_mul(dy as i64));
            match best {
                Some((_, bd)) if d2 >= bd => {}
                _ => best = Some((i, d2)),
            }
        }
    }
    best.map(|(i, _)| i)
}

fn flood_keep(mask: &mut [bool], w: usize, h: usize, seed: usize) {
    let n = mask.len();
    if seed >= n || w == 0 {
        return;
    }
    let mut keep = vec![false; n];
    let mut stack = vec![seed];
    while let Some(i) = stack.pop() {
        let Some(slot) = keep.get_mut(i) else { continue };
        if *slot || !mask.get(i).copied().unwrap_or(false) {
            continue;
        }
        *slot = true;
        let x = i % w;
        let y = i / w;
        if x > 0 {
            stack.push(i - 1);
        }
        if x + 1 < w {
            stack.push(i + 1);
        }
        if y > 0
            && let Some(up) = i.checked_sub(w)
        {
            stack.push(up);
        }
        if y + 1 < h
            && let Some(down) = i.checked_add(w)
        {
            stack.push(down);
        }
        if stack.len() > n.saturating_mul(4) {
            break;
        }
    }
    for (m, k) in mask.iter_mut().zip(keep.iter()) {
        *m = *k;
    }
}

/// Darkening at the core for a Darken Amount fraction `t` in 0..=1 (before the redness weight).
fn depth(t: f32) -> f32 {
    let u = 1.0 - t.clamp(0.0, 1.0);
    0.13 + 0.81 * (1.0 - u * u)
}

/// What the click found: the flood-filled pupil and its shape.
struct Pupil {
    /// Centroid in window pixel coordinates (pixel centres at `+0.5`).
    centre: (f32, f32),
    /// Radius of a disc with the pupil's area.
    radius: f32,
    /// Mean redness `(R − max(G, B)) / R` over the pupil, 0..=1.
    redness: f32,
}

fn measure(px: &[[f32; 4]], mask: &[bool], w: usize) -> Option<Pupil> {
    let (mut n, mut sx, mut sy, mut red) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for (i, on) in mask.iter().enumerate() {
        if !*on || w == 0 {
            continue;
        }
        let Some(p) = px.get(i) else { continue };
        n += 1.0;
        sx += (i % w) as f64 + 0.5;
        sy += (i / w) as f64 + 0.5;
        let (r, g, b) = (p[0], p[1], p[2]);
        if r > 1.0e-6 {
            red += f64::from(((r - g.max(b)) / r).clamp(0.0, 1.0));
        }
    }
    if n < 1.0 {
        return None;
    }
    let centre = ((sx / n) as f32, (sy / n) as f32);
    let radius = ((n / std::f64::consts::PI).sqrt() as f32).max(0.5);
    let redness = (red / n) as f32;
    [centre.0, centre.1, radius, redness].iter().all(|v| v.is_finite()).then_some(Pupil { centre, radius, redness })
}

/// Correct red-eye in `px` (row-major RGBA, including the [`halo_radius`] border).
///
/// `origin` is the document coordinate of `px[0]`. Returns how many pixels changed. The pupil is
/// the red region connected to the red pixel nearest the click within [`SEARCH_RADIUS`]; pixels
/// farther than that from `click` are left untouched. Each pupil pixel becomes a neutral grey at
/// its own luminance (`0.2126 R + 0.7874 (G+B)/2`), darkened by a radial profile around the pupil's
/// centroid whose core width follows `pupil_size` (1..=100) and depth `darken` (0..=100). When a
/// dark neutral pixel (a corrected pupil) is nearer the click than any red one, nothing changes,
/// so a second click is a no-op wherever in the pupil it lands.
pub fn apply(px: &mut [[f32; 4]], w: usize, h: usize, click: (i32, i32), origin: (i32, i32), pupil_size: f32, darken: f32) -> u32 {
    let n = w.saturating_mul(h);
    if w == 0 || h == 0 || px.len() < n {
        return 0;
    }
    let radius = SEARCH_RADIUS;
    let t_amt = if darken.is_finite() { (darken.clamp(0.0, 100.0) / 100.0).clamp(0.0, 1.0) } else { 0.5 };
    // Darken Amount 0 changes no pixel, as in the reference (#12573).
    if t_amt <= 0.0 {
        return 0;
    }
    let s_pupil = if pupil_size.is_finite() { pupil_size.clamp(1.0, 100.0) / 100.0 } else { 0.5 };
    let mut hard = vec![false; n];
    // Squared distance from the click to the nearest red and the nearest dark neutral pixel.
    let (mut red_d2, mut dark_d2) = (i64::MAX, i64::MAX);
    for y in 0..h {
        for x in 0..w {
            let Some(i) = idx(x, y, w) else { continue };
            let Some(p) = px.get(i).copied() else { continue };
            let dx = origin.0.saturating_add(x as i32).saturating_sub(click.0);
            let dy = origin.1.saturating_add(y as i32).saturating_sub(click.1);
            if !within_search(dx, dy, radius) {
                continue;
            }
            let d2 = i64::from(dx).saturating_mul(i64::from(dx)).saturating_add(i64::from(dy).saturating_mul(i64::from(dy)));
            if is_red_eye(p) {
                if let Some(slot) = hard.get_mut(i) {
                    *slot = true;
                }
                red_d2 = red_d2.min(d2);
            } else if is_dark_neutral(p) {
                dark_d2 = dark_d2.min(d2);
            }
        }
    }
    // A click on or in a pupil that is already dark and neutral (a corrected one, also when the
    // press lands on its highlight) has nothing to fix: the corrected pupil is nearer than any red,
    // which would otherwise be the skin around the eye.
    if dark_d2 < red_d2 {
        return 0;
    }
    let Some(seed) = nearest_seed(&hard, w, h, origin, click, radius) else {
        return 0;
    };
    flood_keep(&mut hard, w, h, seed);
    let Some(pupil) = measure(px, &hard, w) else {
        return 0;
    };
    // Pupil Size widens the dark core and deepens it a little; Darken Amount sets the depth.
    let core = (pupil.radius * (0.3 + 0.95 * s_pupil)).max(0.5);
    let amp = depth(t_amt) * (0.75 + 0.5 * s_pupil) * (pupil.redness / 0.8).clamp(0.0, 1.0);
    let mut changed = 0u32;
    for y in 0..h {
        for x in 0..w {
            let Some(i) = idx(x, y, w) else { continue };
            // The mask holds only red pixels inside the search circle, so the highlight (luma ≥ 0.92),
            // sclera and skin around the pupil are never touched.
            if !hard.get(i).copied().unwrap_or(false) {
                continue;
            }
            let Some(p) = px.get_mut(i) else { continue };
            let (r0, g0, b0, a0) = (p[0], p[1], p[2], p[3]);
            let d = (x as f32 + 0.5 - pupil.centre.0).hypot(y as f32 + 0.5 - pupil.centre.1) / core;
            let profile = (-(d * d * d)).exp();
            let dark = (amp * profile).clamp(0.0, 0.97);
            let level = (0.2126 * r0 + 0.7874 * 0.5 * (g0 + b0)) * (1.0 - dark);
            if !level.is_finite() {
                continue;
            }
            let (r2, g2, b2) = (level, level, level);
            if (r2 - r0).abs() < 1.0e-6 && (g2 - g0).abs() < 1.0e-6 && (b2 - b0).abs() < 1.0e-6 {
                continue;
            }
            *p = [r2, g2, b2, a0];
            changed = changed.saturating_add(1);
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(w: usize, h: usize, f: impl Fn(usize, usize) -> [f32; 4]) -> Vec<[f32; 4]> {
        let mut px = vec![[0.0; 4]; w * h];
        for y in 0..h {
            for x in 0..w {
                if let Some(slot) = px.get_mut(y * w + x) {
                    *slot = f(x, y);
                }
            }
        }
        px
    }

    fn disc(x: usize, y: usize, cx: f32, cy: f32, r: f32) -> bool {
        (x as f32 - cx).hypot(y as f32 - cy) <= r
    }

    fn portrait_pixel(x: usize, y: usize) -> [f32; 4] {
        // Brown that fails R > G+0.15 so the iris is never in the red-eye mask.
        let iris = [0.38, 0.28, 0.16, 1.0];
        let red = [0.95, 0.10, 0.10, 1.0];
        let white = [1.0, 1.0, 1.0, 1.0];
        if disc(x, y, 16.0, 16.0, 1.2) {
            white
        } else if disc(x, y, 16.0, 16.0, 10.0) {
            red
        } else {
            iris
        }
    }

    #[test]
    fn halo_and_search_radius() {
        assert_eq!(halo_radius(), 1);
        assert_eq!(SEARCH_RADIUS, 64.0);
        assert!(unclamped_window_side(50.0).is_some_and(|s| s < MAX_WINDOW));
        assert!(unclamped_window_side(10_000.0).is_none());
        assert!(unclamped_window_side(f32::NAN).is_none());
    }

    #[test]
    fn red_disc_is_neutralized_and_iris_is_left_alone() {
        let w = 32usize;
        let mut px = sample(w, w, portrait_pixel);
        let iris_before = px[2 * w + 2];
        let n = apply(&mut px, w, w, (16, 16), (0, 0), 50.0, 100.0);
        assert!(n > 0, "corrected {n} pixels");
        // 5 px from the centre: in the red ring, outside the catchlight.
        let after = px[11 * w + 16];
        assert!((after[0] - after[1]).abs() < 1e-5 && (after[0] - after[2]).abs() < 1e-5, "neutral grey: {after:?}");
        assert!(after[0] < 0.2126 * 0.95 + 0.7874 * 0.10, "darker than the pupil's own grey: {after:?}");
        assert!(luma(after) < luma([0.95, 0.10, 0.10, 1.0]) - 0.05, "pupil darkened");
        let iris_after = px[2 * w + 2];
        for c in 0..4 {
            assert!((iris_after[c] - iris_before[c]).abs() < 1e-5, "iris {iris_after:?} vs {iris_before:?}");
        }
        let catch = px[16 * w + 16];
        assert!(catch[0] >= 0.9 && catch[1] >= 0.9 && catch[2] >= 0.9, "catchlight {catch:?}");
    }

    #[test]
    fn gray_pixels_are_not_treated_as_red_eye() {
        let w = 16usize;
        let mut px = sample(w, w, |_, _| [0.4, 0.4, 0.4, 1.0]);
        let n = apply(&mut px, w, w, (8, 8), (0, 0), 50.0, 50.0);
        assert_eq!(n, 0);
        assert_eq!(px[8 * w + 8], [0.4, 0.4, 0.4, 1.0]);
    }

    /// Apply on the engine's read rect (search + `halo`) cut out of `src`, pasted back.
    fn apply_windowed(src: &[[f32; 4]], w: usize, h: usize, click: (i32, i32), halo: i32) -> Vec<[f32; 4]> {
        let ext = SEARCH_RADIUS.ceil() as i32 + halo;
        let x0 = (click.0 - ext).max(0) as usize;
        let y0 = (click.1 - ext).max(0) as usize;
        let x1 = ((click.0 + ext + 1) as usize).min(w);
        let y1 = ((click.1 + ext + 1) as usize).min(h);
        let (bw, bh) = (x1 - x0, y1 - y0);
        let mut buf: Vec<[f32; 4]> = (y0..y1).flat_map(|y| (x0..x1).map(move |x| src[y * w + x])).collect();
        apply(&mut buf, bw, bh, click, (x0 as i32, y0 as i32), 50.0, 50.0);
        let mut out = src.to_vec();
        for y in 0..bh {
            for x in 0..bw {
                out[(y0 + y) * w + x0 + x] = buf[y * bw + x];
            }
        }
        out
    }

    fn max_err(a: &[[f32; 4]], b: &[[f32; 4]]) -> f32 {
        a.iter().zip(b).flat_map(|(p, q)| (0..3).map(move |c| (p[c] - q[c]).abs())).fold(0.0, f32::max)
    }

    #[test]
    fn the_read_rect_with_its_halo_matches_the_whole_image() {
        let w = 256usize;
        // A red pupil wider than the search circle, on a brown iris: the mask reaches the window edge.
        let src = sample(w, w, |x, y| if disc(x, y, 128.0, 128.0, 90.0) { [0.95, 0.10, 0.10, 1.0] } else { [0.38, 0.28, 0.16, 1.0] });
        let mut reference = src.clone();
        assert!(apply(&mut reference, w, w, (128, 128), (0, 0), 50.0, 50.0) > 0);
        let windowed = apply_windowed(&src, w, w, (128, 128), halo_radius());
        let err = max_err(&reference, &windowed);
        assert!(err < 1e-4, "windowed vs whole image max err {err}");
    }

    #[test]
    fn empty_or_mismatched_buffer_changes_nothing() {
        let mut px: Vec<[f32; 4]> = Vec::new();
        assert_eq!(apply(&mut px, 0, 0, (0, 0), (0, 0), 50.0, 50.0), 0);
        let mut tiny = vec![[0.95, 0.1, 0.1, 1.0]; 4];
        assert_eq!(apply(&mut tiny, 8, 8, (1, 1), (0, 0), 50.0, 50.0), 0);
    }
}

#[cfg(test)]
#[path = "redeye_neutral_result_tests.rs"]
mod neutral_result_tests;
