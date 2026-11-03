//! A bounded, screened colour correction near edit boundaries; never changes protected samples.
#[cfg(test)]
use super::{PixelFormat, SampleType};
use super::{Rect, Result, Surface, bad};
use std::collections::VecDeque;

pub(super) fn match_edges(generated: &mut Surface, reference: &Surface, mask: &Surface, region: Rect, ctx: &crate::jobs::JobCtx) -> Result<()> {
    let (w, h) = (region.width() as usize, region.height() as usize);
    if w == 0 || h == 0 || w > 1024 || h > 1024 || generated.format() != reference.format() {
        return Err(bad("invalid harmonization region or pixel format"));
    }
    let channels = generated.format().channels();
    let colors = channels - usize::from(generated.format().alpha);
    let local = Rect::from_xywh(0, 0, w as u32, h as u32);
    let mut pixels = generated.read_region(local);
    let original = reference.read_region(local);
    let weights = mask.read_region(region);
    let mut distance = vec![u8::MAX; w * h];
    let mut correction = vec![0.0f32; w * h * colors];
    let mut queue = VecDeque::new();
    let neighbors = |i: usize| [i.checked_sub(w), (i + w < w * h).then_some(i + w), (!i.is_multiple_of(w)).then(|| i - 1), (i % w + 1 < w).then_some(i + 1)];
    // Transparent reference pixels cannot supply a meaningful colour match.
    let opaque = |i: usize| !reference.format().alpha || original.get(i * channels + colors).copied().unwrap_or(0.0) > 0.99;
    for i in 0..w * h {
        if weights.get(i).copied().unwrap_or(0.0) <= 0.0 {
            continue;
        }
        let adjacent: Vec<usize> = neighbors(i).into_iter().flatten().filter(|&j| weights.get(j).copied().unwrap_or(1.0) == 0.0 && opaque(j)).collect();
        if adjacent.is_empty() {
            continue;
        }
        if let Some(d) = distance.get_mut(i) {
            *d = 0;
        }
        for c in 0..colors {
            let delta = adjacent.iter().map(|&j| original.get(j * channels + c).copied().unwrap_or(0.0)).sum::<f32>() / adjacent.len() as f32
                - pixels.get(i * channels + c).copied().unwrap_or(0.0);
            if let Some(v) = correction.get_mut(i * colors + c) {
                *v = if delta.is_finite() { delta.clamp(-0.25, 0.25) } else { 0.0 };
            }
        }
        queue.push_back(i);
    }
    // Extend only a narrow band: distant generated content keeps its own appearance.
    while let Some(i) = queue.pop_front() {
        if distance.get(i).copied().unwrap_or(u8::MAX) >= 31 {
            continue;
        }
        for j in neighbors(i).into_iter().flatten() {
            if weights.get(j).copied().unwrap_or(0.0) <= 0.0 || distance.get(j).copied().unwrap_or(u8::MAX) != u8::MAX {
                continue;
            }
            let d = distance.get(i).copied().unwrap_or(31).saturating_add(1);
            if let Some(out) = distance.get_mut(j) {
                *out = d;
            }
            for c in 0..colors {
                let v = correction.get(i * colors + c).copied().unwrap_or(0.0);
                if let Some(out) = correction.get_mut(j * colors + c) {
                    *out = v;
                }
            }
            queue.push_back(j);
        }
    }
    let mut next = correction.clone();
    for _ in 0..32 {
        ctx.check()?;
        for i in 0..w * h {
            if distance.get(i).copied().unwrap_or(u8::MAX) == 0 || distance.get(i).copied().unwrap_or(u8::MAX) == u8::MAX {
                continue;
            }
            for c in 0..colors {
                let mut sum = 0.0;
                let mut count = 0;
                for j in neighbors(i).into_iter().flatten().filter(|&j| distance.get(j).copied().unwrap_or(u8::MAX) != u8::MAX) {
                    sum += correction.get(j * colors + c).copied().unwrap_or(0.0);
                    count += 1;
                }
                if count > 0
                    && let Some(v) = next.get_mut(i * colors + c)
                {
                    *v = sum / count as f32;
                }
            }
        }
        std::mem::swap(&mut next, &mut correction);
    }
    for (i, d) in distance.into_iter().enumerate() {
        if d == u8::MAX {
            continue;
        }
        let fade = (1.0 - d as f32 / 32.0).powi(2);
        for c in 0..colors {
            if let Some(v) = pixels.get_mut(i * channels + c) {
                *v += correction.get(i * colors + c).copied().unwrap_or(0.0) * fade;
            }
        }
    }
    generated.write_region(local, &pixels);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{PixelFormat, Rect, SampleType, Surface, match_edges};
    #[test]
    fn corrects_a_color_jump_without_touching_protection_alpha_or_distant_content() {
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for mode in
                [photocraft_color::ColorMode::Rgb, photocraft_color::ColorMode::Grayscale, photocraft_color::ColorMode::Cmyk, photocraft_color::ColorMode::Lab]
            {
                let fmt = PixelFormat::new(mode, sample, true);
                let r = Rect::from_xywh(0, 0, 80, 64);
                let mut original = Surface::new(fmt);
                let mut generated = Surface::new(fmt);
                let mut a = vec![0.4; fmt.channels()];
                *a.last_mut().unwrap() = 1.0;
                let mut b = vec![0.6; fmt.channels()];
                *b.last_mut().unwrap() = 1.0;
                original.fill_rect(r, &a);
                generated.fill_rect(r, &b);
                let mut mask = Surface::new(PixelFormat::GRAY8);
                mask.fill_rect(Rect::from_xywh(20, 0, 60, 64), &[1.0]);
                let before = generated.clone();
                match_edges(&mut generated, &original, &mask, r, &crate::jobs::JobCtx::new()).unwrap();
                assert!((generated.sample_channel(20, 32, 0) - 0.4).abs() < 0.01);
                assert_eq!(generated.sample_channel(19, 32, 0), before.sample_channel(19, 32, 0));
                assert_eq!(generated.sample_channel(79, 32, 0), before.sample_channel(79, 32, 0));
                assert_eq!(generated.sample_channel(20, 32, fmt.channels() - 1), 1.0);
            }
        }
    }
}
