//! Translate between OpenEXR's associated colour and PhotoCraft's straight-alpha layers.

use craft_exr::decode::{Limits, PartInfo, SampleType};
use photocraft_codecs::{Image, SampleType as CodecSample};

// A power of two smaller than half the least positive HALF value, as recommended by
// OpenEXR. It lets a straight-alpha layer carry light from a zero-coverage pixel.
const MIN_ALPHA: f32 = 1.0 / 67_108_864.0;

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
        if a == 0.0 && colours.iter().all(|colour| f32::from_ne_bytes(*colour) == 0.0) {
            continue;
        }
        let a = a.max(MIN_ALPHA);
        *alpha = a.to_ne_bytes();
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
        if a == MIN_ALPHA {
            *alpha = 0.0f32.to_ne_bytes();
        }
    }
}
