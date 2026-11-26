//! Translate between OpenEXR's associated colour and PhotoCraft's straight-alpha layers.

use craft_exr::decode::{Limits, PartInfo, SampleType};
use photocraft_codecs::{Image, SampleType as CodecSample};
use photocraft_color::{EMISSIVE_ALPHA, is_emissive_alpha};

pub(crate) fn parts(bytes: &[u8]) -> Vec<PartInfo> {
    // This is a header-only query. The actual decoder enforces its own pixel limits.
    std::panic::catch_unwind(|| craft_exr::decode::parts(bytes, &Limits::none())).ok().and_then(Result::ok).unwrap_or_default()
}

pub(crate) fn ordinary_import(parts: &[PartInfo]) -> bool {
    if parts.iter().any(|part| part.deep) {
        return false;
    }
    craft_exr::decode::pick_part(parts).and_then(|index| parts.get(index)).is_some_and(|part| selected_part(part, None))
}

pub(crate) fn selected_part(part: &PartInfo, names: Option<&[&str]>) -> bool {
    if part.deep || part.tiled {
        return false;
    }
    if let Some(names) = names {
        return names.iter().all(|name| part.channels.iter().any(|channel| channel.name == *name && channel.sample == SampleType::F32));
    }
    let Some((_, channels)) = craft_exr::channels::color_channels(part.channels.iter().map(|channel| channel.name.as_str())) else { return false };
    channels.iter().all(|&index| part.channels.get(index).is_some_and(|channel| channel.sample == SampleType::F32))
}

pub(crate) fn prepare_import(image: &mut Image, associated: bool) {
    let layout = image.layout();
    if !layout.has_alpha() {
        return;
    }
    match image.sample_type() {
        CodecSample::F32 => prepare_import_f32(image.data_mut(), layout.channels(), associated),
        CodecSample::F16 => {
            for pixel in image.data_mut().chunks_exact_mut(layout.channels() * 2) {
                let Some(alpha) = pixel.as_chunks_mut::<2>().0.last_mut() else { continue };
                if u16::from_ne_bytes(*alpha) == 0xbc00 {
                    // HALF -1.0 would become the internal emission marker after expansion.
                    *alpha = 0x8000u16.to_ne_bytes();
                }
            }
        }
        _ => {}
    }
}

pub(crate) fn prepare_import_f32(data: &mut [u8], channels: usize, associated: bool) {
    for pixel in data.chunks_exact_mut(channels * 4) {
        let Some((alpha, colours)) = pixel.as_chunks_mut::<4>().0.split_last_mut() else { continue };
        let a = f32::from_ne_bytes(*alpha);
        if is_emissive_alpha(a) {
            // A negative alpha in the file denotes no coverage. Only our conversion of an
            // associated zero-coverage pixel may create an emitter.
            *alpha = (-0.0f32).to_ne_bytes();
            continue;
        }
        if !associated || !(0.0..1.0).contains(&a) {
            continue;
        }
        let peak = colours.iter().map(|colour| f32::from_ne_bytes(*colour).abs()).filter(|value| value.is_finite()).fold(0.0, f32::max);
        if a == 0.0 {
            // Keep emission associated at zero coverage. A positive alpha cannot be a
            // marker: ordinary pixels may use every representable coverage value.
            if colours.iter().any(|colour| f32::from_ne_bytes(*colour) != 0.0) {
                *alpha = EMISSIVE_ALPHA.to_ne_bytes();
            }
            continue;
        }
        let mut coverage = a;
        // No straight float32 sample can retain both the original coverage and this colour
        // when the quotient is too large. Raise coverage only as far as needed to keep the
        // associated colour. Round the bound upward so division cannot overflow again.
        let peak = f64::from(peak);
        let max = f64::from(f32::MAX);
        if peak > max * f64::from(coverage) {
            let needed = peak / max;
            coverage = needed as f32;
            if f64::from(coverage) * max < peak {
                coverage = coverage.next_up();
            }
        }
        *alpha = coverage.to_ne_bytes();
        for colour in colours {
            *colour = (f32::from_ne_bytes(*colour) / coverage).clamp(f32::MIN, f32::MAX).to_ne_bytes();
        }
    }
}

pub(crate) fn premultiply_image(image: &mut Image) {
    let layout = image.layout();
    if !layout.has_alpha() {
        return;
    }
    if image.sample_type() != CodecSample::F32 {
        *image = image.convert(layout, CodecSample::F32);
    }
    for pixel in image.data_mut().chunks_exact_mut(layout.channels() * 4) {
        let Some((alpha, colours)) = pixel.as_chunks_mut::<4>().0.split_last_mut() else { continue };
        let a = f32::from_ne_bytes(*alpha);
        if is_emissive_alpha(a) {
            *alpha = 0.0f32.to_ne_bytes();
            continue;
        }
        if !(0.0..1.0).contains(&a) {
            continue;
        }
        for colour in colours {
            *colour = (f32::from_ne_bytes(*colour) * a).to_ne_bytes();
        }
    }
}
