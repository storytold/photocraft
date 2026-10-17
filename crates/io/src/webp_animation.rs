//! Editable WebP frame stacks. Visibility chooses the editing frame; it never drops frames
//! from an animated save. Names/order follow WebPShop's documented layer convention.

use photocraft_codecs::{self as codecs, AnimationFrame, Format};
use photocraft_color::BlendMode;
use photocraft_doc::{Document, LayerContent};
use photocraft_raster::Interrupt;

use crate::{ExportOptions, ExportResult, ImportResult, IoError};

/// Each composited WebP frame as a document, in chronological order with its exact delay.
/// Unlike the flat decoder, this refuses incomplete animations. Still WebP yields one frame.
pub fn import_webp_frames(name: &str, bytes: &[u8], each: &mut dyn FnMut(Document, u32) -> Result<(), IoError>) -> Result<usize, IoError> {
    let animation = codecs::decode_webp_frames(bytes, &codecs::DecodeOptions::default())?;
    let n = animation.frames.len();
    for frame in animation.frames {
        each(crate::flat::image_to_document(name, &frame.image)?.document, frame.delay_ms)?;
    }
    Ok(n)
}

pub(crate) fn import_document(name: &str, bytes: &[u8], ctl: &Interrupt) -> Result<ImportResult, IoError> {
    let animation = codecs::decode_webp_frames(bytes, &codecs::DecodeOptions::default())?;
    let loops = animation.loop_count;
    let mut frames = animation.frames.into_iter();
    let first = frames.next().ok_or_else(|| IoError::Unsupported("WebP animation has no frames".into()))?;
    let mut result = crate::flat::image_to_document(name, &first.image)?;
    result.document.file_encoding = crate::flat::file_encoding(bytes);
    result.document.file_encoding.webp_animation_loops = loops;
    for (i, frame) in std::iter::once(first).chain(frames).enumerate() {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let mut layer = photocraft_doc::Layer::new(
            format!("Frame{} ({} ms)", i + 1, frame.delay_ms),
            LayerContent::Raster(crate::flat::image_surface(&frame.image, codecs::ChannelLayout::Rgba, codecs::SampleType::U8)?),
        );
        layer.visible = i == 0;
        if i == 0 {
            result.document.layers.clear();
        }
        result.document.layers.push(layer);
    }
    Ok(result)
}

/// Exact WebPShop-style duration suffix. Layer prefixes may be freely renamed.
pub(crate) fn delay_ms(name: &str) -> Result<u32, IoError> {
    let bad =
        || IoError::Unsupported(format!("WebP frame layer `{name}` needs a duration suffix such as (100 ms), between 0 and {} ms", codecs::MAX_WEBP_DELAY_MS));
    let suffix = name.trim_end().rsplit_once('(').map(|(_, s)| s).and_then(|s| s.strip_suffix(" ms)")).ok_or_else(bad)?;
    if suffix.is_empty() || !suffix.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad());
    }
    suffix.parse::<u32>().ok().filter(|n| *n <= codecs::MAX_WEBP_DELAY_MS).ok_or_else(bad)
}

pub(crate) fn export_document(doc: &Document, opts: &ExportOptions, ctl: &Interrupt) -> Result<ExportResult, IoError> {
    let loops = doc.file_encoding.webp_animation_loops.ok_or_else(|| IoError::Unsupported("not a WebP animation".into()))?;
    let limits = codecs::Limits::default();
    limits.check(doc.size.width, doc.size.height, codecs::ChannelLayout::Rgba, codecs::SampleType::U8)?;
    if doc.size.width > 16383 || doc.size.height > 16383 {
        return Err(IoError::Unsupported("WebP frames exceed 16383 pixels per side".into()));
    }
    if doc.layers.is_empty() || doc.layers.len() > codecs::MAX_WEBP_FRAMES {
        return Err(IoError::Unsupported("WebP animation has no frames or too many frames".into()));
    }
    let bytes = u64::from(doc.size.width).saturating_mul(u64::from(doc.size.height)).saturating_mul(4).saturating_mul(doc.layers.len() as u64 + 3);
    if bytes > limits.max_alloc {
        return Err(IoError::Unsupported("WebP animation frames exceed the decoded memory budget".into()));
    }
    // Validate the whole stack before rendering anything. A failed save never drops a layer.
    let mut delays = Vec::new();
    for layer in &doc.layers {
        if !matches!(layer.content, LayerContent::Raster(_))
            || layer.clipped
            || layer.blend != BlendMode::Normal
            || layer.video.is_some()
            || layer.excluded_channels != 0
            || !layer.blend_if.is_default()
            || layer.advanced != Default::default()
        {
            return Err(IoError::Unsupported(format!(
                "WebP frame `{}` must be a raster layer with Normal blending, without clipping, video or advanced blending; rasterize it before saving",
                layer.name
            )));
        }
        delays.push(delay_ms(&layer.name)?);
    }
    let mut frames = Vec::new();
    let mut warnings = Vec::new();
    let mut one = doc.clone();
    one.file_encoding.webp_animation_loops = None;
    one.timeline = None;
    for (layer, delay_ms) in doc.layers.iter().zip(delays) {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let mut layer = layer.clone();
        layer.visible = true;
        one.layers = vec![layer];
        let (image, notes) = crate::flat::prepare_flat_image(&one, Format::WebP, opts, &Interrupt::NONE)?;
        let fidelity = codecs::fidelity_warnings_with(&image, Format::WebP, &opts.encode);
        if let Some(fatal) = fidelity.iter().find(|w| w.is_fatal()) {
            return Err(IoError::Unsupported(fatal.to_string()));
        }
        for note in notes.into_iter().chain(fidelity.into_iter().map(|w| w.to_string())) {
            if !warnings.contains(&note) {
                warnings.push(note);
            }
        }
        frames.push(AnimationFrame { image: crate::flat::webp_frame_pixels(image)?, delay_ms });
        ctl.progress(0.5 * frames.len() as f32 / doc.layers.len() as f32);
    }
    let bytes = codecs::encode_webp_animation(&frames, loops, &opts.encode)?;
    ctl.check().map_err(|_| IoError::Cancelled)?;
    warnings.push("Animation frames, durations and loop count saved; editable layer names, masks, effects and other document structure are not kept in WebP. Save as .pcraft to retain them".into());
    Ok(ExportResult { bytes, warnings })
}
