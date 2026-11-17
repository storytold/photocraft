//! Translate between OpenEXR's associated colour and PhotoCraft's straight-alpha layers.

use craft_exr::decode::{Limits, PartInfo, SampleType};
use photocraft_codecs::{Image, SampleType as CodecSample};

// Give emission a little coverage so straight-alpha layers can carry it, then restore
// zero coverage when writing associated colour on export.
const EMISSION_ALPHA: f32 = 1.0 / 16_384.0;

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

pub(crate) fn unpremultiply_image(image: &mut Image) {
    let layout = image.layout();
    if image.sample_type() == CodecSample::F32 && layout.has_alpha() {
        unpremultiply(image.data_mut(), layout.channels());
    }
}

pub(crate) fn unpremultiply(data: &mut [u8], channels: usize) {
    for pixel in data.chunks_exact_mut(channels * 4) {
        let Some((alpha, colours)) = pixel.as_chunks_mut::<4>().0.split_last_mut() else { continue };
        let a = f32::from_ne_bytes(*alpha);
        if !(0.0..1.0).contains(&a) {
            continue;
        }
        let a = if a == 0.0 {
            if colours.iter().all(|colour| f32::from_ne_bytes(*colour) == 0.0) {
                continue;
            }
            *alpha = EMISSION_ALPHA.to_ne_bytes();
            EMISSION_ALPHA
        } else if a == EMISSION_ALPHA {
            // Reserve the exact marker for zero-alpha emission. Move a real sample at the
            // marker up by one float32 step, preserving its coverage at export.
            let a = a.next_up();
            *alpha = a.to_ne_bytes();
            a
        } else {
            a
        };
        for colour in colours {
            *colour = (f32::from_ne_bytes(*colour) / a).clamp(f32::MIN, f32::MAX).to_ne_bytes();
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
        if !(0.0..1.0).contains(&a) {
            continue;
        }
        for colour in colours {
            *colour = (f32::from_ne_bytes(*colour) * a).to_ne_bytes();
        }
        if a == EMISSION_ALPHA {
            *alpha = 0.0f32.to_ne_bytes();
        }
    }
}
