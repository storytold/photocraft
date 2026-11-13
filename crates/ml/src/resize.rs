//! Bilinear resampling of interleaved `f32` buffers (half-pixel centres, edges clamped).

/// `src` (`sw × sh × ch`) resampled to `dw × dh`. Empty or mismatched input gives zeros.
pub(crate) fn bilinear(src: &[f32], sw: usize, sh: usize, ch: usize, dw: usize, dh: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; dw.saturating_mul(dh).saturating_mul(ch)];
    if sw == 0 || sh == 0 || dw == 0 || dh == 0 || ch == 0 || src.len() != sw * sh * ch {
        return out;
    }
    let (fx, fy) = (sw as f32 / dw as f32, sh as f32 / dh as f32);
    // Shrinking by more than 2x: average the source footprint first (box prefilter) so thin lines
    // do not alias away; upscaling and mild shrinking go straight to bilinear.
    let pre;
    let (src, sw, sh, fx, fy) = if fx > 2.0 || fy > 2.0 {
        let (bx, by) = ((fx.floor() as usize).max(1), (fy.floor() as usize).max(1));
        let (nw, nh) = (sw / bx, sh / by);
        if nw == 0 || nh == 0 {
            (src, sw, sh, fx, fy)
        } else {
            pre = box_down(src, sw, sh, ch, bx, by, nw, nh);
            let nfx = nw as f32 / dw as f32;
            let nfy = nh as f32 / dh as f32;
            (pre.as_slice(), nw, nh, nfx, nfy)
        }
    } else {
        (src, sw, sh, fx, fy)
    };
    for y in 0..dh {
        let sy = ((y as f32 + 0.5) * fy - 0.5).clamp(0.0, (sh - 1) as f32);
        let (y0, ty) = (sy.floor() as usize, sy - sy.floor());
        let y1 = (y0 + 1).min(sh - 1);
        for x in 0..dw {
            let sx = ((x as f32 + 0.5) * fx - 0.5).clamp(0.0, (sw - 1) as f32);
            let (x0, tx) = (sx.floor() as usize, sx - sx.floor());
            let x1 = (x0 + 1).min(sw - 1);
            for c in 0..ch {
                let at = |xx: usize, yy: usize| src.get((yy * sw + xx) * ch + c).copied().unwrap_or(0.0);
                let top = at(x0, y0) * (1.0 - tx) + at(x1, y0) * tx;
                let bot = at(x0, y1) * (1.0 - tx) + at(x1, y1) * tx;
                if let Some(o) = out.get_mut((y * dw + x) * ch + c) {
                    *o = top * (1.0 - ty) + bot * ty;
                }
            }
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn box_down(src: &[f32], sw: usize, _sh: usize, ch: usize, bx: usize, by: usize, nw: usize, nh: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; nw * nh * ch];
    let inv = 1.0 / (bx * by) as f32;
    for y in 0..nh {
        for x in 0..nw {
            for c in 0..ch {
                let mut sum = 0.0;
                for yy in 0..by {
                    for xx in 0..bx {
                        sum += src.get(((y * by + yy) * sw + x * bx + xx) * ch + c).copied().unwrap_or(0.0);
                    }
                }
                if let Some(o) = out.get_mut((y * nw + x) * ch + c) {
                    *o = sum * inv;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_stays_constant() {
        let src = vec![0.25f32; 8 * 6 * 3];
        for (dw, dh) in [(16, 12), (3, 2), (8, 6), (1, 1)] {
            let out = bilinear(&src, 8, 6, 3, dw, dh);
            assert_eq!(out.len(), dw * dh * 3);
            assert!(out.iter().all(|v| (v - 0.25).abs() < 1e-6), "{dw}x{dh}");
        }
    }

    #[test]
    fn bad_input_is_zeros_not_a_panic() {
        assert_eq!(bilinear(&[1.0], 2, 2, 1, 2, 2), vec![0.0; 4]);
        assert!(bilinear(&[], 0, 0, 1, 2, 2).iter().all(|v| *v == 0.0));
        assert!(bilinear(&[1.0], 1, 1, 1, 0, 3).is_empty());
    }

    #[test]
    fn big_shrink_averages() {
        let mut src = vec![0.0f32; 16 * 16];
        for (i, v) in src.iter_mut().enumerate() {
            *v = if (i % 16 + i / 16) % 2 == 0 { 1.0 } else { 0.0 };
        }
        let out = bilinear(&src, 16, 16, 1, 2, 2);
        assert!(out.iter().all(|v| (v - 0.5).abs() < 0.05), "{out:?}");
    }
}
