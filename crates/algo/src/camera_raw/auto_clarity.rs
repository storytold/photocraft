use super::luma;

/// Suggest a small midtone Clarity boost when the image has weak local contrast. Compare
/// opposing neighbors at two distances to avoid treating flat areas or fine noise as structure.
pub fn auto_clarity(px: &[[f32; 4]], w: usize, h: usize) -> Option<f32> {
    if w.min(h) < 8 || w.checked_mul(h) != Some(px.len()) {
        return None;
    }
    let radius = (w.min(h) / 40).max(2);
    let luminance = |x: usize, y: usize| {
        let [r, g, b, a] = *px.get(y * w + x)?;
        let rgb = [r, g, b];
        (a.is_finite() && a >= 0.5 && rgb.iter().all(|v| v.is_finite())).then(|| luma(rgb.map(|v| v.clamp(0.0, 1.0))))
    };
    let mut count = 0;
    let mut coarse = 0.0;
    let mut fine = 0.0;
    let mut strong = 0;
    for y in (radius..h - radius).step_by(h.div_ceil(64)) {
        for x in (radius..w - radius).step_by(w.div_ceil(64)) {
            if luminance(x, y).is_none_or(|v| !(0.15..=0.85).contains(&v)) {
                continue;
            }
            let contrast = |r: usize| {
                let dx = luminance(x + r, y)? - luminance(x - r, y)?;
                let dy = luminance(x, y + r)? - luminance(x, y - r)?;
                Some(dx.abs().max(dy.abs()))
            };
            if let (Some(c), Some(f)) = (contrast(radius), contrast(1)) {
                count += 1;
                coarse += c;
                fine += f;
                strong += usize::from(c > 0.15);
            }
        }
    }
    if count < 16 || strong * 10 > count || fine > coarse * 0.8 {
        return None;
    }
    let average = coarse / count as f32;
    (0.008..0.07).contains(&average).then(|| (18.0 * (1.0 - average / 0.08)).round())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clarity_prefers_weak_structure_over_flat_or_strong_patterns() {
        let pattern = |amount: f32| {
            (0..64 * 64)
                .map(|i| {
                    let v = 0.5 + amount * ((i % 64) as f32 * std::f32::consts::TAU / 32.0).sin();
                    [v, v, v, 1.0]
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(auto_clarity(&pattern(0.0), 64, 64), None);
        assert!(auto_clarity(&pattern(0.05), 64, 64).is_some_and(|v| (1.0..=18.0).contains(&v)));
        assert_eq!(auto_clarity(&pattern(0.35), 64, 64), None);
        assert_eq!(auto_clarity(&pattern(0.05), usize::MAX, 64), None);
        assert_eq!(auto_clarity(&[[0.5, 0.5, 0.5, 0.0]; 64], 8, 8), None);
        assert_eq!(auto_clarity(&[[f32::NAN, 0.5, 0.5, 1.0]; 64], 8, 8), None);
    }
}
