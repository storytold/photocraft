//! Guided filter (K. He, J. Sun, X. Tang, "Guided Image Filtering", TPAMI 2013): snaps a soft mask
//! to the edges of the image it belongs to, in O(n) whatever the radius. Used to put hair-level
//! detail back into a matte that a network produced at a lower resolution.

/// Mean over a `(2r+1)²` window (clamped at the borders), separable, `f64` running sums.
fn box_mean(src: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let mut tmp = vec![0.0f32; src.len()];
    let mut out = vec![0.0f32; src.len()];
    if w == 0 || h == 0 || src.len() != w * h {
        return out;
    }
    let line = |get: &dyn Fn(usize) -> f32, n: usize, put: &mut dyn FnMut(usize, f32)| {
        let mut sum = 0.0f64;
        let (mut lo, mut hi) = (0usize, 0usize); // window is [lo, hi)
        for i in 0..n {
            let (want_lo, want_hi) = (i.saturating_sub(r), (i + r + 1).min(n));
            while hi < want_hi {
                sum += f64::from(get(hi));
                hi += 1;
            }
            while lo < want_lo {
                sum -= f64::from(get(lo));
                lo += 1;
            }
            put(i, (sum / (hi - lo) as f64) as f32);
        }
    };
    for y in 0..h {
        let row = &src[y * w..(y + 1) * w];
        let trow = &mut tmp[y * w..(y + 1) * w];
        line(&|i| row[i], w, &mut |i, v| trow[i] = v);
    }
    for x in 0..w {
        line(&|i| tmp[i * w + x], h, &mut |i, v| out[i * w + x] = v);
    }
    out
}

/// Filters `p` (a `w × h` mask) guided by `guide` (the image's luminance, `0..=1`) with window
/// radius `r` and regularisation `eps`. The result is clamped to `0..=1`; a size mismatch returns
/// `p` unchanged.
pub fn guided_filter(guide: &[f32], p: &[f32], w: usize, h: usize, r: usize, eps: f32) -> Vec<f32> {
    let n = w.saturating_mul(h);
    if n == 0 || guide.len() != n || p.len() != n {
        return p.to_vec();
    }
    let r = r.min(w.max(h));
    let mean_i = box_mean(guide, w, h, r);
    let mean_p = box_mean(p, w, h, r);
    let ii: Vec<f32> = guide.iter().map(|v| v * v).collect();
    let ip: Vec<f32> = guide.iter().zip(p).map(|(i, p)| i * p).collect();
    let corr_i = box_mean(&ii, w, h, r);
    let corr_ip = box_mean(&ip, w, h, r);
    let mut a = vec![0.0f32; n];
    let mut b = vec![0.0f32; n];
    for k in 0..n {
        let var = corr_i[k] - mean_i[k] * mean_i[k];
        let cov = corr_ip[k] - mean_i[k] * mean_p[k];
        a[k] = cov / (var + eps);
        b[k] = mean_p[k] - a[k] * mean_i[k];
    }
    let mean_a = box_mean(&a, w, h, r);
    let mean_b = box_mean(&b, w, h, r);
    (0..n).map(|k| (mean_a[k] * guide[k] + mean_b[k]).clamp(0.0, 1.0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_mean_of_constant() {
        let m = box_mean(&[0.5; 7 * 5], 7, 5, 2);
        assert!(m.iter().all(|v| (v - 0.5).abs() < 1e-6));
    }

    #[test]
    fn snaps_a_blurry_mask_to_the_image_edge() {
        // Image: bright left half, dark right half, edge at x = 20. Mask: the same step, blurred and
        // shifted by 3 px. The filtered mask's edge must land closer to the image's.
        let (w, h) = (40usize, 8usize);
        let guide: Vec<f32> = (0..w * h).map(|i| if i % w < 20 { 0.9 } else { 0.1 }).collect();
        let p: Vec<f32> = (0..w * h).map(|i| ((23.0 - (i % w) as f32) / 6.0 + 0.5).clamp(0.0, 1.0)).collect();
        let q = guided_filter(&guide, &p, w, h, 6, 1e-3);
        let edge = |m: &[f32]| (0..w).find(|x| m[3 * w + x] < 0.5).unwrap_or(w);
        assert!(edge(&q).abs_diff(20) < edge(&p).abs_diff(20), "{} vs {}", edge(&q), edge(&p));
        assert!(q.iter().all(|v| (0.0..=1.0).contains(v)));
    }

    #[test]
    fn mismatched_sizes_return_the_mask() {
        assert_eq!(guided_filter(&[0.0; 3], &[0.5; 4], 2, 2, 1, 1e-3), vec![0.5; 4]);
        assert!(guided_filter(&[], &[], 0, 0, 1, 1e-3).is_empty());
    }
}
