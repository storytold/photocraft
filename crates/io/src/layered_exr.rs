//! Stream an RGB document's layer pixels to a tiled multipart OpenEXR.

use std::collections::HashSet;
use std::io::{Seek, Write};

use base64::{Engine, engine::general_purpose::STANDARD};
use craft_cms::{Builtin, ColorSpace, Intent, Profile, Transform};
use craft_exr::decode::{Compression, Error, SampleType, Window};
use craft_exr::layer_stack::{ATTRIBUTE_NAME, Layer as StackLayer, LayerContent as StackContent, LayerStack, PixelRef, PixelSampling, SampleDepth};
use craft_exr::write::{Channel, Metadata, Part, RowSamples, TilePart, TileWriter, write_tiles};
use photocraft_color::{ColorMode, SampleType as DocSample, convert::srgb_to_linear};
use photocraft_compose::{Buffer, RenderBandsError};
use photocraft_doc::{Document, Layer, LayerContent, LayerId};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{Interrupt, Surface, spill};

use crate::IoError;

/// Lossless compression choices for flat EXR tiles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LayeredExrCompression {
    None,
    Rle,
    Zips,
    #[default]
    Zip,
    Piz,
}

impl From<LayeredExrCompression> for Compression {
    fn from(value: LayeredExrCompression) -> Self {
        match value {
            LayeredExrCompression::None => Self::None,
            LayeredExrCompression::Rle => Self::Rle,
            LayeredExrCompression::Zips => Self::Zip1,
            LayeredExrCompression::Zip => Self::Zip16,
            LayeredExrCompression::Piz => Self::Piz,
        }
    }
}

#[derive(Clone, Debug)]
pub struct LayeredExrOptions {
    pub composite: bool,
    /// Raster layer IDs. Omit to export all raster layers, including hidden layers.
    pub layers: Option<Vec<LayerId>>,
    pub compression: LayeredExrCompression,
}

impl Default for LayeredExrOptions {
    fn default() -> Self {
        Self { composite: true, layers: None, compression: LayeredExrCompression::default() }
    }
}

const RGBA: [Channel<'static>; 4] = [
    Channel { name: "R", sample: SampleType::F32 },
    Channel { name: "G", sample: SampleType::F32 },
    Channel { name: "B", sample: SampleType::F32 },
    Channel { name: "A", sample: SampleType::F32 },
];
const RGB_CHROMATICITIES: [[f32; 2]; 4] = [[0.64, 0.33], [0.30, 0.60], [0.15, 0.06], [0.3127, 0.3290]];
const EDGE: u32 = TILE_SIZE as u32;

enum Pixels<'a> {
    Composite,
    Raster(&'a Surface),
}

struct Source<'a> {
    name: String,
    bounds: Rect,
    pixels: Pixels<'a>,
}

/// Write to a fresh seekable destination. Native callers stage this stream atomically.
/// Returned warnings describe document features represented only in the composite.
pub fn write_layered_exr(
    doc: &Document,
    out: &mut (impl Write + Seek + ?Sized),
    opts: &LayeredExrOptions,
    ctl: &Interrupt<'_>,
) -> Result<Vec<String>, IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    if doc.mode != ColorMode::Rgb || doc.bounds().is_empty() {
        return Err(IoError::Unsupported("Layered OpenEXR requires a nonempty RGB document".into()));
    }
    photocraft_compose::discard_unreadable_pixel_caches();
    doc.scratch_health().map_err(IoError::Unsupported)?;
    let scratch_generation = spill::read_error_generation();
    let (profile, profile_id) = working_profile(doc)?;
    let (sources, layers, warnings) = sources(doc, opts)?;
    let manifest = if layers.is_empty() {
        None
    } else {
        let depth = match doc.depth {
            DocSample::U8 => SampleDepth::U8,
            DocSample::U16 => SampleDepth::U16,
            DocSample::F32 => SampleDepth::F32,
        };
        let stack = LayerStack { layers, original_blend_profile: Some(profile_id), original_sample_depth: Some(depth) };
        Some(stack.to_attribute().map_err(|e| IoError::Unsupported(e.to_string()))?)
    };
    let first = manifest.as_deref().map(|manifest| [(ATTRIBUTE_NAME, manifest)]);
    let mut attributes: Vec<&[(&str, &str)]> = vec![&[]; sources.len()];
    if let Some(slot) = attributes.first_mut()
        && let Some(first) = &first
    {
        *slot = first;
    }
    let metadata = Metadata { chromaticities: Some(RGB_CHROMATICITIES), text_attributes: &attributes };
    let parts: Vec<_> = sources
        .iter()
        .map(|s| TilePart {
            part: Part { name: Some(&s.name), data_window: window(s.bounds), channels: &RGBA, compression: opts.compression.into() },
            tile_width: EDGE,
            tile_height: EDGE,
        })
        .collect();
    let linear = Builtin::LinearSrgb.profile();
    let srgb = profile.same_colors(Builtin::Srgb.profile());
    let transform = (!srgb && !profile.same_colors(linear))
        .then(|| Transform::new(&profile, linear, Intent::RelativeColorimetric, false))
        .transpose()
        .map_err(|e| IoError::Unsupported(e.to_string()))?;
    let total: u64 = sources.iter().map(|s| u64::from(s.bounds.width().div_ceil(EDGE)) * u64::from(s.bounds.height().div_ceil(EDGE))).sum();
    let mut done = 0u64;
    let mut source_error = None;
    let result = write_tiles(out, window(doc.bounds()), &parts, &metadata, |index, rect, tile| {
        let mut fill = || -> Result<(), IoError> {
            ctl.check().map_err(|_| IoError::Cancelled)?;
            let source = sources.get(index).ok_or_else(|| IoError::Unsupported("Unknown OpenEXR part".into()))?;
            let rect = Rect::from_xywh(rect.x, rect.y, rect.width, rect.height);
            let mut pixels = match source.pixels {
                Pixels::Composite => render_tile(doc, rect)?,
                Pixels::Raster(surface) => raster_tile(surface, rect)?,
            };
            if let Some(transform) = &transform {
                transform.apply(pixels.px.as_flattened_mut(), 4).map_err(|e| IoError::Unsupported(e.to_string()))?;
            }
            for [r, g, b, a] in &mut pixels.px {
                if srgb {
                    // Match import's signed extension, including out-of-gamut negative HDR values.
                    for v in [&mut *r, &mut *g, &mut *b] {
                        *v = srgb_to_linear(v.abs()).copysign(*v);
                    }
                }
                *r *= *a;
                *g *= *a;
                *b *= *a;
            }
            fill_tile(tile, &pixels.px).map_err(|e| IoError::Unsupported(e.to_string()))?;
            Ok(())
        };
        fill().map_err(|error| {
            source_error = Some(error);
            Error::Encode("PhotoCraft could not produce an EXR tile".into())
        })?;
        done += 1;
        // The last callback precedes writing the final chunk and offset tables.
        ctl.progress(0.99 * (done as f32 / total.max(1) as f32));
        Ok(())
    });
    if let Some(error) = source_error {
        return Err(error);
    }
    result.map_err(|e| IoError::Unsupported(e.to_string()))?;
    spill::check_since(scratch_generation).map_err(IoError::Unsupported)?;
    doc.scratch_health().map_err(IoError::Unsupported)?;
    ctl.check().map_err(|_| IoError::Cancelled)?;
    ctl.progress(1.0);
    Ok(warnings)
}

fn window(rect: Rect) -> Window {
    Window { x: rect.x0, y: rect.y0, width: rect.width(), height: rect.height() }
}

fn working_profile(doc: &Document) -> Result<(Profile, String), IoError> {
    let Some(bytes) = &doc.icc_profile else {
        return Ok((Builtin::Srgb.profile().clone(), "srgb".into()));
    };
    let profile = Profile::parse(bytes).map_err(|e| IoError::Unsupported(format!("Invalid RGB document profile: {e}")))?;
    if profile.color_space != ColorSpace::Rgb {
        return Err(IoError::Unsupported("Layered OpenEXR requires an RGB colour profile".into()));
    }
    let profile_id = if profile.same_colors(Builtin::Srgb.profile()) {
        "srgb".into()
    } else if profile.same_colors(Builtin::LinearSrgb.profile()) {
        "lin_rec709".into()
    } else {
        format!("icc:{}", STANDARD.encode(bytes.as_slice()))
    };
    Ok((profile, profile_id))
}

fn sources<'a>(doc: &'a Document, opts: &LayeredExrOptions) -> Result<(Vec<Source<'a>>, Vec<StackLayer>, Vec<String>), IoError> {
    let mut sources = Vec::new();
    if opts.composite {
        sources.push(Source { name: "Beauty".into(), bounds: doc.bounds(), pixels: Pixels::Composite });
    }
    let walk = doc.walk();
    let selected = opts.layers.as_ref().map(|ids| ids.iter().copied().collect::<HashSet<_>>());
    let mut included = HashSet::new();
    let mut unmatched = selected.clone().unwrap_or_default();
    let mut unsupported = 0;
    let mut unrepresented = 0;
    for (path, _, layer) in &walk {
        if selected.as_ref().is_some_and(|ids| !ids.contains(&layer.id)) {
            continue;
        }
        if is_layered_exr_raster(layer) {
            unmatched.remove(&layer.id);
            for depth in 1..=path.len() {
                if let Some(ancestor) = doc.layer_at(&path[..depth]) {
                    included.insert(ancestor.id);
                }
            }
        } else if !matches!(layer.content, LayerContent::Group(_)) {
            unsupported += 1;
        }
    }
    if !unmatched.is_empty() {
        return Err(IoError::Unsupported("Select existing RGB raster layer IDs for layered OpenEXR".into()));
    }
    let mut layers = Vec::new();
    let mut missing_clip_bases = 0;
    for (path, _, layer) in walk {
        if !included.contains(&layer.id) {
            continue;
        }
        unrepresented += usize::from(has_unsupported_blending(layer));
        let content = match &layer.content {
            LayerContent::Group(_) => StackContent::Group,
            LayerContent::Raster(surface) => {
                let name = part_name(layer);
                let mut bounds = surface.tile_bounds();
                let format = surface.format();
                if !format.alpha || surface.default_pixel().get(3).is_some_and(|a| *a != 0.0) {
                    bounds = bounds.union(&doc.bounds());
                }
                if bounds.is_empty() {
                    bounds = Rect::from_xywh(0, 0, 1, 1);
                }
                sources.push(Source { name: name.clone(), bounds, pixels: Pixels::Raster(surface) });
                StackContent::Raster { pixels: PixelRef::Part { name }, sampling: PixelSampling::Raw }
            }
            _ => continue,
        };
        let parent = path.split_last().and_then(|(_, path)| doc.layer_at(path)).map(|layer| layer.id.0);
        let clipped = layer.clipped && has_exported_clip_base(doc, &path, &included);
        missing_clip_bases += usize::from(layer.clipped && !clipped);
        layers.push(StackLayer {
            id: layer.id.0,
            name: layer.name.clone(),
            parent,
            visible: layer.visible,
            blend: layer.blend.key().into(),
            opacity: layer.opacity,
            fill_opacity: layer.fill_opacity,
            clipped,
            content,
        });
    }
    if sources.is_empty() {
        return Err(IoError::Unsupported("This document has no RGB raster layers to export".into()));
    }
    let mut warnings = Vec::new();
    if unsupported > 0 {
        warnings.push(format!("{unsupported} non-raster layer(s) are available only in Beauty when included"));
    }
    if unrepresented > 0 {
        warnings.push(format!("{unrepresented} layer(s) have masks, effects or advanced blending represented only in Beauty when included"));
    }
    if missing_clip_bases > 0 {
        warnings.push(format!("{missing_clip_bases} layer(s) exported without clipping because their base layers are absent"));
    }
    Ok((sources, layers, warnings))
}

/// Whether a layer can supply an editable raster part in a layered OpenEXR export.
pub fn is_layered_exr_raster(layer: &Layer) -> bool {
    matches!(&layer.content, LayerContent::Raster(surface) if surface.format().mode == ColorMode::Rgb)
}

fn has_exported_clip_base(doc: &Document, path: &[usize], included: &HashSet<LayerId>) -> bool {
    let Some((&index, parent)) = path.split_last() else { return false };
    let siblings = if parent.is_empty() {
        &*doc.layers
    } else {
        let Some(siblings) = doc.layer_at(parent).and_then(Layer::children) else { return false };
        siblings
    };
    siblings.get(..index).and_then(|below| below.iter().rev().find(|layer| !layer.clipped)).is_some_and(|base| included.contains(&base.id))
}

fn has_unsupported_blending(layer: &Layer) -> bool {
    layer.mask.is_some()
        || layer.vector_mask.is_some()
        || layer.frame.is_some()
        || !layer.effects.items.is_empty()
        || layer.excluded_channels != 0
        || layer.blend_if != Default::default()
        || layer.advanced != Default::default()
        || matches!(&layer.content, LayerContent::Group(g) if g.artboard.is_some() || g.frame_stroke.is_some())
}

fn part_name(layer: &Layer) -> String {
    let label: String = layer.name.chars().take(200).map(|ch| if (ch.is_ascii_graphic() && ch != '.') || ch == ' ' { ch } else { '_' }).collect();
    let label = label.trim();
    let base = if label.is_empty() { "Layer" } else { label };
    format!("{base}_{}", layer.id.0)
}

fn render_tile(doc: &Document, rect: Rect) -> Result<Buffer, IoError> {
    let mut result = None;
    photocraft_compose::try_render_bands(doc, rect, TILE_SIZE, |band| -> Result<(), std::convert::Infallible> {
        result = Some(band);
        Ok(())
    })
    .map_err(|e| match e {
        RenderBandsError::Allocation(e) => IoError::OutOfMemory(e.to_string()),
        RenderBandsError::Sink(never) => match never {},
    })?;
    result.ok_or_else(|| IoError::Unsupported("The compositor returned an empty EXR tile".into()))
}

fn raster_tile(surface: &Surface, rect: Rect) -> Result<Buffer, IoError> {
    let mut result = Buffer::try_transparent(rect).map_err(|e| IoError::OutOfMemory(e.to_string()))?;
    surface.read_rgba_into(rect, &mut result.px);
    surface.region_health(rect).map_err(IoError::Unsupported)?;
    Ok(result)
}

fn fill_tile(tile: &mut TileWriter<'_>, pixels: &[[f32; 4]]) -> craft_exr::decode::Result<()> {
    for channel in 0..4 {
        tile.fill_channel(channel, |samples| {
            let RowSamples::F32(samples) = samples else {
                return Err(Error::Encode("Expected FLOAT OpenEXR colour channel".into()));
            };
            for (dst, src) in samples.iter_mut().zip(pixels) {
                *dst = src[channel];
            }
            Ok(pixels.len())
        })?;
    }
    Ok(())
}
