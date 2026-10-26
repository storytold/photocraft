//! Gamut checks against a proof (output) profile: Photoshop's View › Gamut Warning.

use crate::clut::Clut;
use crate::pipeline::{Pipeline, Stage};
use crate::profile::Profile;
use crate::transform::link_stages;
use crate::{CmsError, Intent};

/// Default ΔE76 threshold above which a colour counts as out of gamut.
pub const DEFAULT_THRESHOLD: f32 = 4.0;

#[cfg(not(target_arch = "wasm32"))]
const PAR_MIN_PIXELS: usize = 16 * 1024;
#[cfg(not(target_arch = "wasm32"))]
const PAR_CHUNK_PIXELS: usize = 16 * 1024;

/// Precomputed out-of-gamut test for colours of `src` against `proof`.
///
/// A colour is out of gamut when sending it to the proof device (relative colorimetric,
/// clipping) and back to Lab moves it by more than the threshold (ΔE76). The ΔE is tabulated
/// over the source device space (33³ grid for 3 channels, 17⁴ for 4, exact otherwise).
#[derive(Debug)]
pub struct GamutCheck {
    inputs: usize,
    grid: Option<Clut>,
    exact: (Pipeline, Pipeline),
    pub threshold: f32,
}

impl GamutCheck {
    pub fn new(src: &Profile, proof: &Profile, threshold: f32) -> Result<GamutCheck, CmsError> {
        let lab = crate::Builtin::LabD50.profile();
        let direct = Pipeline::new(src.channels(), link_stages(src, lab, Intent::RelativeColorimetric, false)?);
        let mut via = link_stages(src, proof, Intent::RelativeColorimetric, false)?;
        via.push(Stage::Clamp01);
        via.extend(link_stages(proof, lab, Intent::RelativeColorimetric, false)?);
        let via = Pipeline::new(src.channels(), via);
        let de = |p: &[f32], o: &mut [f32]| {
            let (mut a, mut b) = ([0.0f32; 16], [0.0f32; 16]);
            direct.eval(p, &mut a);
            via.eval(p, &mut b);
            let d = |i: usize, s: f32| (a[i] - b[i]) * s;
            o[0] = (d(0, 100.0).powi(2) + d(1, 255.0).powi(2) + d(2, 255.0).powi(2)).sqrt();
        };
        let n = src.channels();
        let grid = match n {
            1 => Some(Clut::sample(vec![256], 1, de)),
            3 => Some(Clut::sample(vec![33; 3], 1, de)),
            4 => Some(Clut::sample(vec![17; 4], 1, de)),
            _ => None,
        };
        Ok(GamutCheck { inputs: n, grid, exact: (direct, via), threshold })
    }

    pub fn inputs(&self) -> usize {
        self.inputs
    }

    /// ΔE76 between the colour and its proof round trip.
    pub fn delta_e(&self, px: &[f32]) -> f32 {
        let mut o = [0.0f32; 1];
        match &self.grid {
            Some(g) => {
                let mut c = [0.0f32; 16];
                for (d, s) in c.iter_mut().zip(&px[..self.inputs]) {
                    *d = s.clamp(0.0, 1.0);
                }
                g.eval(&c[..self.inputs], &mut o);
            }
            None => {
                let (mut a, mut b) = ([0.0f32; 16], [0.0f32; 16]);
                self.exact.0.eval(px, &mut a);
                self.exact.1.eval(px, &mut b);
                o[0] = ((a[0] - b[0]) * 100.0).hypot((a[1] - b[1]) * 255.0).hypot((a[2] - b[2]) * 255.0);
            }
        }
        o[0]
    }

    pub fn out_of_gamut(&self, px: &[f32]) -> bool {
        self.delta_e(px) > self.threshold
    }

    /// Out-of-gamut mask for RGBA pixels: `255` where the pixel has alpha > 0 and its first
    /// `inputs()` channels are out of gamut, `0` elsewhere. Returns the number of flagged
    /// pixels. Only `min(px.len(), mask.len())` pixels are processed. Runs on the rayon pool
    /// (not on wasm); the result is identical to calling [`Self::out_of_gamut`] per pixel.
    pub fn mask_rgba(&self, px: &[[f32; 4]], mask: &mut [u8]) -> usize {
        let n = px.len().min(mask.len());
        let (px, mask) = (&px[..n], &mut mask[..n]);
        let run = |p: &[[f32; 4]], m: &mut [u8]| {
            let mut count = 0;
            for (p, m) in p.iter().zip(m.iter_mut()) {
                let hit = p[3] > 0.0 && self.out_of_gamut(&p[..3]);
                *m = if hit { 255 } else { 0 };
                count += usize::from(hit);
            }
            count
        };
        #[cfg(not(target_arch = "wasm32"))]
        if n >= PAR_MIN_PIXELS {
            use rayon::prelude::*;
            return px.par_chunks(PAR_CHUNK_PIXELS).zip(mask.par_chunks_mut(PAR_CHUNK_PIXELS)).map(|(p, m)| run(p, m)).sum();
        }
        run(px, mask)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Builtin;

    fn image(n: usize) -> Vec<[f32; 4]> {
        let mut s = 0x9E37_79B9u32;
        let mut rnd = move || {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s >> 8) as f32 / (1u32 << 24) as f32
        };
        // Includes out-of-range values and zero alpha.
        (0..n).map(|i| [rnd() * 1.2 - 0.1, rnd(), rnd(), if i % 7 == 0 { 0.0 } else { rnd() }]).collect()
    }

    #[test]
    fn mask_rgba_matches_per_pixel_check() {
        let src = Builtin::Srgb.profile();
        // Sizes straddle the parallel threshold and the chunk size, odd ones included.
        for (proof, threshold) in [(Builtin::CoatedCmyk, 4.0), (Builtin::CoatedCmyk, 1.5), (Builtin::GrayGamma22, 4.0), (Builtin::DisplayP3, 0.5)] {
            let check = GamutCheck::new(src, proof.profile(), threshold).unwrap();
            for n in [0, 1, 1023, 16 * 1024 - 1, 16 * 1024, 16 * 1024 + 1, 50_001] {
                let px = image(n);
                let want: Vec<u8> = px.iter().map(|p| if p[3] > 0.0 && check.out_of_gamut(&p[..3]) { 255 } else { 0 }).collect();
                let mut got = vec![7u8; n];
                let count = check.mask_rgba(&px, &mut got);
                assert_eq!(got, want, "{proof:?} threshold {threshold} n {n}");
                assert_eq!(count, want.iter().filter(|&&m| m == 255).count());
            }
        }
    }
}
