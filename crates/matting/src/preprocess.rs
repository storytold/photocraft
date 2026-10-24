//! Image → model input: the BiRefNet preprocessing contract.
//!
//! BiRefNet-family models are trained on square 1024×1024 (or 2048×2048)
//! frames normalised to [-1, 1] *after* scaling to [0, 1]. PhotoCraft
//! hands us straight (non-premultiplied) RGBA8; the alpha channel is not a
//! model input — RGB is resampled bilinearly onto the square canvas, padded
//! with the canonical BiRefNet grey (128) so the padding reads as "unknown
//! mid-grey" rather than black or white matte, then normalised:
//!
//! ```text
//! x = (c / 255 - mean) / std      mean = (0.5, 0.5, 0.5)
//!                                std  = (0.5, 0.5, 0.5)
//! ```
//!
//! The padding rectangle is returned so the postprocessor can map the
//! network's square mask back onto the original aspect ratio without a
//! second resample blur.

/// What the caller must know to undo the square padding later.
#[derive(Debug, Clone, Copy)]
pub struct Padding {
    /// Model input side (`resolution`).
    pub side: u32,
    /// Top-left of the scaled image inside the square frame.
    pub offset: (f32, f32),
    /// Size the image was scaled to before padding.
    pub scaled: (u32, u32),
}

/// NCHW f32 tensor of `resolution` × `resolution` × 3 plus the padding rect.
///
/// `rgba` is `width * height * 4` straight RGBA8 samples.
pub fn build_input(rgba: &[u8], width: u32, height: u32, resolution: u32) -> (Vec<f32>, Padding) {
    // Fit the longest side, keep the aspect ratio (the model sees the whole
    // image; padding is the cheapest honest way to do that on a square input).
    let scale = (resolution as f32 / width as f32).min(resolution as f32 / height as f32);
    let sw = ((width as f32 * scale) as u32).max(1).min(resolution);
    let sh = ((height as f32 * scale) as u32).max(1).min(resolution);
    let ox = ((resolution - sw) as f32 / 2.0).floor();
    let oy = ((resolution - sh) as f32 / 2.0).floor();
    let padding = Padding { side: resolution, offset: (ox, oy), scaled: (sw, sh) };

    const MEAN: [f32; 3] = [0.5, 0.5, 0.5];
    const STD: [f32; 3] = [0.5, 0.5, 0.5];
    // 0.50196 ≈ (128/255 - mean) / std differs per channel; fill with the
    // padded grey *after* normalisation by precomputing the per-channel value.
    let grey = |c: usize| (128.0 / 255.0 - MEAN[c]) / STD[c];

    let plane: usize = (sw as usize) * (sh as usize);
    // Bilinear scale into a compact RGB plane first (cheaper than sampling
    // the source per output pixel with clamping logic inline).
    let mut rgb = vec![0u8; plane * 3];
    resample_bilinear(rgba, width, height, &mut rgb, sw, sh);

    let side = resolution as usize;
    let mut out = vec![0f32; 3 * side * side];
    for c in 0..3 {
        let base = c * side * side;
        for i in 0..side * side {
            out[base + i] = grey(c);
        }
    }
    for y in 0..sh as usize {
        let dy = (y as f32 + oy) as usize;
        for x in 0..sw as usize {
            let dx = (x as f32 + ox) as usize;
            for c in 0..3 {
                let v = rgb[(y * sw as usize + x) * 3 + c] as f32 / 255.0;
                out[c * side * side + dy * side + dx] = (v - MEAN[c]) / STD[c];
            }
        }
    }
    (out, padding)
}

/// Bilinear RGBA → RGB scale (alpha ignored; the matte is what we compute).
fn resample_bilinear(rgba: &[u8], w: u32, h: u32, dst: &mut [u8], dw: u32, dh: u32) {
    let sx = w as f32 / dw as f32;
    let sy = h as f32 / dh as f32;
    for y in 0..dh as usize {
        let fy = (y as f32 + 0.5) * sy - 0.5;
        let y0 = fy.floor().max(0.0) as usize;
        let y1 = (y0 + 1).min(h as usize - 1);
        let wy = (fy - y0 as f32).clamp(0.0, 1.0);
        for x in 0..dw as usize {
            let fx = (x as f32 + 0.5) * sx - 0.5;
            let x0 = fx.floor().max(0.0) as usize;
            let x1 = (x0 + 1).min(w as usize - 1);
            let wx = (fx - x0 as f32).clamp(0.0, 1.0);
            for c in 0..3 {
                let a = |yy: usize, xx: usize| rgba[(yy * w as usize + xx) * 4 + c] as f32;
                let top = a(y0, x0) * (1.0 - wx) + a(y0, x1) * wx;
                let bot = a(y1, x0) * (1.0 - wx) + a(y1, x1) * wx;
                dst[(y * dw as usize + x) * 3 + c] = (top * (1.0 - wy) + bot * wy + 0.5) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_is_normalised_nchw_with_grey_padding() {
        // 2×1 black/white halves → 4×4 square: the padded rows are the grey constant.
        let rgba = vec![0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 255, 255, 255, 255, 255];
        let (t, pad) = build_input(&rgba, 2, 1, 4);
        assert_eq!(t.len(), 3 * 16);
        assert_eq!(pad.scaled, (4, 2));
        // Row 0 is padding grey on every channel.
        let grey_r = (128.0 / 255.0 - 0.5) / 0.5;
        assert!((t[0] - grey_r).abs() < 1e-6);
        // Black lands on the R-plane mean-subtracted value, -mean/std.
        let dy = (pad.offset.1 as usize) * 4 + pad.offset.0 as usize;
        let black_r = (0.0 - 0.5) / 0.5;
        assert!((t[dy] - black_r).abs() < 1e-6, "black R: {} vs {black_r}", t[dy]);
        // White = (1 - 0.5) / 0.5.
        let white_r = (1.0 - 0.5) / 0.5;
        assert!((t[dy + 3] - white_r).abs() < 0.02, "{:?}", &t[dy..dy + 4]);
    }
}
