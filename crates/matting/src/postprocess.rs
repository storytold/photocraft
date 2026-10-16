//! Model output → alpha matte: sigmoid the logits and resample the padded
//! square back onto the source rectangle.
//!
//! BiRefNet returns one `1 × 1 × S × S` map of **logits** (unbounded). The
//! reference pipeline squashes them with a plain sigmoid and resamples with
//! bilinear interpolation, which is what we do — including the trick of
//! sampling only the padded source rectangle, so the matte returns to the
//! exact source grid without an intermediate square raster.

use crate::preprocess::Padding;

/// `logits` is the `S * S` plane of the model output; the result is `w * h`
/// 8-bit alpha values.
pub fn build_alpha(logits: &[f32], padding: &Padding, width: u32, height: u32) -> Vec<u8> {
    let side = padding.side as usize;
    debug_assert_eq!(logits.len(), side * side, "postprocess wants the full square plane");
    let (sw, sh) = padding.scaled;
    let (ox, oy) = padding.offset;
    let mut out = vec![0u8; (width as usize) * (height as usize)];
    let sx = sw as f32 / width as f32;
    let sy = sh as f32 / height as f32;
    for y in 0..height as usize {
        // Source pixel centre inside the scaled image, then into the square.
        let fy = (y as f32 + 0.5) * sy - 0.5;
        let gy = (fy.clamp(0.0, sh as f32 - 1.0) + oy).min(side as f32 - 1.0);
        let y0 = gy.floor() as usize;
        let y1 = (y0 + 1).min(side - 1);
        let wy = gy - y0 as f32;
        for x in 0..width as usize {
            let fx = (x as f32 + 0.5) * sx - 0.5;
            let gx = (fx.clamp(0.0, sw as f32 - 1.0) + ox).min(side as f32 - 1.0);
            let x0 = gx.floor() as usize;
            let x1 = (x0 + 1).min(side - 1);
            let wx = gx - x0 as f32;
            let s = |yy: usize, xx: usize| 1.0 / (1.0 + (-logits[yy * side + xx]).exp());
            let v = (s(y0, x0) * (1.0 - wx) + s(y0, x1) * wx) * (1.0 - wy) + (s(y1, x0) * (1.0 - wx) + s(y1, x1) * wx) * wy;
            out[y * width as usize + x] = (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logits_are_squashed_and_unpadded() {
        // Uniform logit 0 → alpha 128 everywhere (sigmoid 0.5), whatever the padding.
        let pad = Padding { side: 4, offset: (1.0, 1.0), scaled: (2, 2) };
        let a = build_alpha(&vec![0.0; 16], &pad, 3, 2);
        assert!(a.iter().all(|&v| v == 128), "{a:?}");
        // Strongly positive logits inside the content rows → opaque.
        let mut hi = vec![-20.0f32; 16];
        for row in 1..=2 {
            for col in 1..=2 {
                hi[row * 4 + col] = 20.0;
            }
        }
        let a = build_alpha(&hi, &pad, 2, 2);
        assert!(a.iter().all(|&v| v > 250), "{a:?}");
    }
}
