//! Colour weights shared by the Skin Smooth WASM filter and native mask preview.
//! RGB is normalized in the filter input's colour space, before any display transform.

pub const MAX_SAMPLES: usize = 16;

pub fn valid_samples(samples: &[[f32; 3]]) -> bool {
    samples.len() <= MAX_SAMPLES && samples.iter().flatten().all(|v| v.is_finite() && (0.0..=1.0).contains(v))
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub fn chroma([r, g, b]: [f32; 3]) -> [f32; 2] {
    [128.0 - 0.168736 * r - 0.331264 * g + 0.5 * b, 128.0 + 0.5 * r - 0.418688 * g - 0.081312 * b]
}

/// Repeated samples use max, never an additive accumulation. Empty positives use the original
/// automatic skin model; negatives also work against that model. Preserve its luma protection.
pub fn weight(rgb: [f32; 3], tolerance: f32, positive: &[[f32; 3]], negative: &[[f32; 3]]) -> f32 {
    let [r, g, b] = rgb.map(|v| v * 255.0);
    let y = 0.299 * r + 0.587 * g + 0.114 * b;
    let [cb, cr] = chroma([r, g, b]);
    let spread = 0.6 + tolerance / 100.0;
    let range = |[a, b]: [f32; 2]| {
        let d = ((cb - a) / (19.0 * spread)).powi(2) + ((cr - b) / (24.0 * spread)).powi(2);
        1.0 - smoothstep(0.35, 1.6, d)
    };
    let maximum = |samples: &[[f32; 3]]| samples.iter().map(|rgb| range(chroma(rgb.map(|v| v * 255.0)))).fold(0.0, f32::max);
    let selected = if positive.is_empty() { range([108.0, 151.0]) } else { maximum(positive) };
    selected * (1.0 - maximum(negative)) * smoothstep(12.0, 38.0, y) * (1.0 - smoothstep(237.0, 254.0, y))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sampled_ranges_union_subtract_and_keep_luma_protection() {
        let skin = [0.7, 0.48, 0.36];
        let blue = [0.2, 0.4, 0.8];
        for tolerance in [0.0, 55.0, 100.0] {
            assert_eq!(weight(skin, tolerance, &[skin], &[]), 1.0);
            assert_eq!(weight(skin, tolerance, &[skin, skin], &[]), 1.0);
            assert_eq!(weight(skin, tolerance, &[blue, skin], &[skin]), 0.0);
            assert_eq!(weight(skin, tolerance, &[], &[skin]), 0.0);
            assert_eq!(weight(blue, tolerance, &[blue], &[]), 1.0);
            assert_eq!(weight([0.0; 3], tolerance, &[[0.0; 3]], &[]), 0.0);
            assert_eq!(weight([1.0; 3], tolerance, &[[1.0; 3]], &[]), 0.0);
        }
        assert!(weight(blue, 55.0, &[], &[]) < weight(blue, 55.0, &[blue], &[]));
        assert!(!valid_samples(&[[f32::NAN; 3]]));
        assert!(!valid_samples(&[[1.1; 3]]));
        assert!(!valid_samples(&[[0.5; 3]; MAX_SAMPLES + 1]));
    }
}
