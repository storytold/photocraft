//! Rebuild an editable document from the optional OpenEXR layer-stack attribute.

use std::collections::HashMap;
use std::io::{Read, Seek};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use craft_cms::{Builtin, ColorSpace, Intent, Profile, Transform, curve::Curve, math};
use craft_exr::decode::{Limits, PartInfo, SampleType as ExrSample, Window};
use craft_exr::layer_stack::{ATTRIBUTE_NAME, LayerContent as StackContent, LayerStack, PixelRef, PixelSampling, SampleDepth};
use craft_exr::seek::{self, Index, Request, Stats, Stop};
use photocraft_color::{BlendMode, ColorMode, PixelFormat, SampleType, convert::linear_to_srgb};
use photocraft_doc::{Document, Group, Layer, LayerContent};
use photocraft_geom::{Rect, Size, TILE_SIZE};
use photocraft_raster::{Interrupt, Surface, memory, spill};

use crate::{ImportOptions, ImportResult, IoError};

const REC709: [[f32; 2]; 4] = [[0.64, 0.33], [0.30, 0.60], [0.15, 0.06], [0.3127, 0.3290]];

fn invalid(error: impl std::fmt::Display) -> IoError {
    IoError::Unsupported(format!("editable OpenEXR: {error}"))
}

fn read_error(error: seek::Error) -> IoError {
    match error {
        seek::Error::Cancelled => IoError::Cancelled,
        seek::Error::OutOfMemory(reason) => IoError::OutOfMemory(reason),
        error => invalid(error),
    }
}

/// Returns `None` for ordinary EXRs and unknown stack versions so the caller can keep its
/// regular flat-image path. Only the listed parts and channels are read into raster layers.
pub(crate) fn import<R: Read + Seek>(name: &str, source: &mut R, opts: &ImportOptions, ctl: &Interrupt<'_>) -> Result<Option<ImportResult>, IoError> {
    let mut limits = Limits::default();
    if opts.flat_max_pixel_bytes > 0 {
        limits.max_bytes = limits.max_bytes.min(opts.flat_max_pixel_bytes);
    }
    let index = match Index::build(source, &limits, &|| ctl.cancelled()) {
        Ok(index) => index,
        Err(seek::Error::Cancelled) => return Err(IoError::Cancelled),
        // Preserve ordinary EXR support when the selective reader cannot index a file.
        Err(_) => return Ok(None),
    };
    let mut attributes = index.parts().iter().filter_map(|part| part.text(ATTRIBUTE_NAME));
    let Some(text) = attributes.next() else { return Ok(None) };
    if attributes.any(|other| other != text) {
        return Err(invalid("conflicting layer-stack attributes"));
    }
    let Some(stack) = LayerStack::parse(text).map_err(invalid)? else { return Ok(None) };
    let depth = match stack.original_sample_depth {
        Some(SampleDepth::U8) => SampleType::U8,
        Some(SampleDepth::U16) => SampleType::U16,
        Some(SampleDepth::F32) | None => SampleType::F32,
    };
    let format = Document::pixel_format_for(ColorMode::Rgb, depth);
    let mut warnings = Vec::new();
    let source_profile = linear_profile(index.chromaticities())?;
    let profile = match stack.original_blend_profile.as_deref() {
        Some(id) => blend_profile(id)?,
        None => source_profile.clone(),
    };
    let encoding = if source_profile.same_colors(&profile) {
        Encoding::Linear
    } else if source_profile.same_colors(Builtin::LinearSrgb.profile()) && profile.same_colors(Builtin::Srgb.profile()) {
        Encoding::Srgb
    } else {
        Encoding::Cms(Box::new(Transform::new(&source_profile, &profile, Intent::RelativeColorimetric, false).map_err(invalid)?))
    };
    if stack.layers.iter().any(|layer| matches!(layer.content, StackContent::Raster { sampling: PixelSampling::Baked, .. })) {
        warnings.push("Rendered EXR layers contain baked source pixels; their original effects, transforms and masks are no longer editable.".into());
    }

    let display = index.display();
    let plans = stack
        .layers
        .iter()
        .map(|layer| match &layer.content {
            StackContent::Group => Ok(None),
            StackContent::Raster { pixels, .. } => raster_plan(index.parts(), pixels, display, format).map(Some),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let scratch_expected = spill::enabled();
    let tile_bytes = plans
        .iter()
        .flatten()
        .map(|p| p.rect.tile_count())
        .fold(0u64, u64::saturating_add)
        .saturating_mul((TILE_SIZE as u64).pow(2) * format.bytes_per_pixel() as u64);
    if tile_bytes > limits.max_bytes {
        return Err(IoError::OutOfMemory(format!("editable EXR tile storage exceeds the {}-byte import limit", limits.max_bytes)));
    }
    if !scratch_expected && (spill::stats().resident_bytes as u64).saturating_add(tile_bytes) > memory::refresh().tile_bytes {
        return Err(IoError::OutOfMemory("decoded EXR layer tiles exceed the available RAM budget; enable a scratch disk or close other documents".into()));
    }

    let max_bytes = usize::try_from(limits.max_bytes).unwrap_or(usize::MAX);
    let mut children: HashMap<Option<u64>, Vec<(u64, Layer)>> = HashMap::new();
    let layer_count = stack.layers.len().max(1);
    for (i, (spec, plan)) in stack.layers.into_iter().zip(plans).enumerate() {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let content = match plan {
            Some(plan) => LayerContent::Raster(plan.decode(source, &index, max_bytes, &encoding, scratch_expected, ctl)?),
            None => LayerContent::Group(Group { children: Vec::new(), expanded: true, artboard: None, frame_stroke: None }),
        };
        let mut layer = Layer::new(spec.name, content);
        layer.blend = blend_mode(&spec.blend).unwrap_or_else(|| {
            warnings.push(format!("{}: OpenEXR blend mode {} is unsupported; using Normal.", layer.name, spec.blend));
            BlendMode::Normal
        });
        layer.visible = spec.visible;
        layer.opacity = spec.opacity;
        layer.fill_opacity = spec.fill_opacity;
        layer.clipped = spec.clipped;
        children.entry(spec.parent).or_default().push((spec.id, layer));
        ctl.progress(0.05 + 0.9 * (i + 1) as f32 / layer_count as f32);
    }
    let mut document = Document::new(name, Size::new(display.width, display.height), ColorMode::Rgb, depth);
    document.icc_profile = Some(profile.to_bytes());
    document.layers = assemble(None, &mut children).into();
    Ok(Some(ImportResult { document, warnings, source_read_only: false, preview_only: false }))
}

fn assemble(parent: Option<u64>, children: &mut HashMap<Option<u64>, Vec<(u64, Layer)>>) -> Vec<Layer> {
    // The manifest validator bounds depth and guarantees that only groups have children.
    children
        .remove(&parent)
        .unwrap_or_default()
        .into_iter()
        .map(|(id, mut layer)| {
            if let LayerContent::Group(ref mut group) = layer.content {
                group.children = assemble(Some(id), children);
            }
            layer
        })
        .collect()
}

fn blend_mode(name: &str) -> Option<BlendMode> {
    let normalized: String = name.chars().filter(|c| !matches!(c, ' ' | '-' | '_')).map(|c| c.to_ascii_lowercase()).collect();
    BlendMode::from_key(match normalized.as_str() {
        "add" | "lineardodge" | "svg:plus" => "lineardodgeadd",
        "srcover" | "svg:srcover" => "normal",
        key => key.strip_prefix("svg:").unwrap_or(key),
    })
}

fn linear_profile(chromaticities: Option<[[f32; 2]; 4]>) -> Result<Profile, IoError> {
    let values = chromaticities.unwrap_or(REC709);
    if values.into_iter().flatten().zip(REC709.into_iter().flatten()).all(|(v, r)| (v - r).abs() < 1e-5) {
        return Ok(Builtin::LinearSrgb.profile().clone());
    }
    let [r, g, b, white] = values.map(|v| v.map(f64::from));
    let white = math::xy_to_xyz(white);
    let matrix = math::rgb_to_xyz_matrix(r, g, b, white).ok_or_else(|| invalid("invalid stored RGB chromaticities"))?;
    Profile::from_source_matrix(white, matrix, [Curve::Identity, Curve::Identity, Curve::Identity]).map_err(invalid)
}

fn blend_profile(id: &str) -> Result<Profile, IoError> {
    let profile = if let Some(encoded) = id.strip_prefix("icc:") {
        let bytes = STANDARD.decode(encoded).map_err(invalid)?;
        Profile::parse(&bytes).map_err(invalid)?
    } else {
        match id {
            "lin_rec709" => Builtin::LinearSrgb,
            name => Builtin::from_id(name).ok_or_else(|| invalid(format!("unknown original RGB blend profile {name}")))?,
        }
        .profile()
        .clone()
    };
    if profile.color_space != ColorSpace::Rgb {
        return Err(invalid("the original blend profile must be RGB"));
    }
    Ok(profile)
}

enum Encoding {
    Linear,
    Srgb,
    Cms(Box<Transform>),
}

impl Encoding {
    fn straight(&self, [r, g, b, alpha]: [f32; 4]) -> Result<[f32; 4], IoError> {
        if !alpha.is_finite() || alpha <= 0.0 {
            return Ok([0.0; 4]);
        }
        let rgb = [r, g, b].map(|v| v / alpha).map(|v| if v.is_finite() { v } else { 0.0 });
        let [r, g, b] = match self {
            Self::Linear => rgb,
            Self::Srgb => rgb.map(|v| linear_to_srgb(v.abs()).copysign(v)),
            Self::Cms(transform) => {
                let mut out = [0.0; 3];
                transform.eval(&rgb, &mut out).map_err(invalid)?;
                out
            }
        };
        Ok([r, g, b, alpha.min(1.0)])
    }
}

struct RasterPlan {
    part: usize,
    channels: Vec<(usize, usize)>,
    rect: Rect,
    format: PixelFormat,
}

fn raster_plan(parts: &[PartInfo], pixels: &PixelRef, display: Window, format: PixelFormat) -> Result<RasterPlan, IoError> {
    let (part, prefix) = match pixels {
        PixelRef::Part { name } => {
            let mut found = parts.iter().filter(|p| p.name.as_ref() == Some(name));
            let part = found.next().ok_or_else(|| invalid(format!("part {name} is missing")))?;
            if found.next().is_some() {
                return Err(invalid(format!("part name {name} is ambiguous")));
            }
            (part, None)
        }
        PixelRef::ChannelGroup { part, prefix } => {
            let part = parts.get(*part as usize).ok_or_else(|| invalid(format!("part {part} for channel group {prefix} is missing")))?;
            (part, (!prefix.is_empty()).then_some(prefix.as_str()))
        }
    };
    if part.deep {
        return Err(invalid("a layer references a deep part"));
    }
    let mut channels = Vec::with_capacity(4);
    for component in ["R", "G", "B", "A"] {
        let name = prefix.map_or_else(|| component.to_string(), |prefix| format!("{prefix}.{component}"));
        match part.channels.iter().position(|c| c.name == name) {
            Some(i) => {
                let channel = part.channels.get(i).ok_or_else(|| invalid("channel index outside the header"))?;
                if channel.sampling != (1, 1) || channel.sample == ExrSample::U32 {
                    return Err(invalid(format!("layer channel {name} needs full-resolution FLOAT or HALF samples")));
                }
                channels.push((part.index, i));
            }
            None if component == "A" => {}
            None => return Err(invalid(format!("layer channel {name} is missing"))),
        }
    }
    let window = part.data_window;
    let x = window.x.checked_sub(display.x).ok_or_else(|| invalid("layer X offset is out of range"))?;
    let y = window.y.checked_sub(display.y).ok_or_else(|| invalid("layer Y offset is out of range"))?;
    let rect = Rect::from_xywh(x, y, window.width, window.height);
    if rect.width() != window.width || rect.height() != window.height {
        return Err(invalid("layer bounds are out of range"));
    }
    Ok(RasterPlan { part: part.index, channels, rect, format })
}

impl RasterPlan {
    fn decode<R: Read + Seek>(
        &self,
        source: &mut R,
        index: &Index,
        max_bytes: usize,
        encoding: &Encoding,
        scratch_expected: bool,
        ctl: &Interrupt<'_>,
    ) -> Result<Surface, IoError> {
        let count = self.channels.len();
        let group = 0..count;
        let req = Request {
            data_window: Some(self.part),
            groups: std::slice::from_ref(&group),
            max_block_bytes: Some(64 << 20),
            ..Request::new(&self.channels, max_bytes)
        };
        let estimate = index.estimate(&req).map_err(read_error)?;
        let _working = memory::try_reserve(estimate.working_bytes).map_err(IoError::OutOfMemory)?;
        let expected = |value: usize| invalid(format!("unexpected EXR layer band size {value}"));
        let mut surface = Surface::new(self.format);
        seek::read_bands_try(
            source,
            index,
            &req,
            &|| ctl.cancelled(),
            &mut |mut band| {
                if scratch_expected && !spill::enabled() {
                    return Err(IoError::Read(spill::last_error().unwrap_or_else(|| "scratch storage was disabled while loading; retry the import".into())));
                }
                let band_rect = |x, y, w, h| {
                    let x = i32::try_from(x).ok().and_then(|x| self.rect.x0.checked_add(x))?;
                    let y = i32::try_from(y).ok().and_then(|y| self.rect.y0.checked_add(y))?;
                    Some(Rect::from_xywh(x, y, u32::try_from(w).ok()?, u32::try_from(h).ok()?))
                };
                let rect = band_rect(band.x, band.y, band.width, band.height)
                    .filter(|rect| self.rect.contains_rect(rect))
                    .ok_or_else(|| invalid("layer band lies outside its data window"))?;
                let pixel_count = band.width.checked_mul(band.height).ok_or_else(|| expected(band.samples.len()))?;
                if band.values_per_pixel != count || pixel_count.checked_mul(count) != Some(band.samples.len()) {
                    return Err(expected(band.samples.len()));
                }
                if count == 4 {
                    for px in band.samples.as_chunks_mut::<4>().0 {
                        *px = encoding.straight(*px)?;
                    }
                    surface.try_write_region(rect, &band.samples).map_err(|e| IoError::OutOfMemory(e.to_string()))
                } else {
                    let mut rgba = Vec::new();
                    rgba.try_reserve_exact(pixel_count.checked_mul(4).ok_or_else(|| expected(band.samples.len()))?)
                        .map_err(|e| IoError::OutOfMemory(e.to_string()))?;
                    for &[r, g, b] in band.samples.as_chunks::<3>().0 {
                        rgba.extend(encoding.straight([r, g, b, 1.0])?);
                    }
                    surface.try_write_region(rect, &rgba).map_err(|e| IoError::OutOfMemory(e.to_string()))
                }
            },
            &mut Stats::default(),
        )
        .map_err(|stop| match stop {
            Stop::Read(e) => read_error(e),
            Stop::Sink(e) => e,
        })?;
        Ok(surface)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use craft_exr::decode::Compression;
    use craft_exr::layer_stack::Layer as StackLayer;
    use craft_exr::write::{Channel, Metadata, Part, RowSamples, write_scanlines_with_metadata};

    use super::*;

    #[test]
    fn import_reads_an_empty_channel_group_as_bare_rgb() {
        let window = Window { x: 0, y: 0, width: 1, height: 1 };
        let channels = ["R", "G", "B"].map(|name| Channel { name, sample: ExrSample::F32 });
        let parts = [Part { name: Some("source"), data_window: window, channels: &channels, compression: Compression::Zip1 }];
        let layer = StackLayer {
            id: 1,
            name: "Unprefixed".into(),
            parent: None,
            visible: true,
            blend: "normal".into(),
            opacity: 1.0,
            fill_opacity: 1.0,
            clipped: false,
            content: StackContent::Raster { pixels: PixelRef::ChannelGroup { part: 0, prefix: String::new() }, sampling: PixelSampling::Raw },
        };
        let attr = LayerStack { layers: vec![layer], original_blend_profile: None, original_sample_depth: None }.to_attribute().unwrap();
        let attributes = [(ATTRIBUTE_NAME, attr.as_str())];
        let metadata = Metadata { chromaticities: Some(REC709), text_attributes: &[&attributes] };
        let mut encoded = Cursor::new(Vec::new());
        let expected = [0.25, 0.5, 0.75, 1.0];
        write_scanlines_with_metadata(&mut encoded, window, &parts, &metadata, |_, _, channel, mut samples| {
            if let RowSamples::F32(values) = &mut samples {
                values.fill(expected[channel]);
            }
            Ok(samples.len())
        })
        .unwrap();

        let doc = crate::import("unprefixed.exr", encoded.get_ref()).unwrap().document;
        assert_eq!(doc.layers[0].name, "Unprefixed");
        assert_eq!(doc.layers[0].surface().unwrap().rgba(0, 0), expected);
    }

    #[test]
    fn import_rebuilds_group_and_preserves_off_canvas_hdr_rgba() {
        let display = Window { x: 8, y: 4, width: 4, height: 3 };
        let crop = Window { x: 7, y: 5, width: 2, height: 1 };
        let beauty = ["R", "G", "B", "A"].map(|name| Channel { name, sample: ExrSample::F32 });
        let pixels = ["Paint 2.R", "Paint 2.G", "Paint 2.B", "Paint 2.A"].map(|name| Channel { name, sample: ExrSample::F32 });
        let parts = [
            Part { name: Some("Beauty"), data_window: display, channels: &beauty, compression: Compression::Zip1 },
            Part { name: Some("layers"), data_window: crop, channels: &pixels, compression: Compression::Zip1 },
        ];
        let group = StackLayer {
            id: 2,
            name: "Group".into(),
            parent: None,
            visible: true,
            blend: "passthrough".into(),
            opacity: 0.75,
            fill_opacity: 1.0,
            clipped: false,
            content: StackContent::Group,
        };
        let layer = StackLayer {
            id: 1,
            name: "Paint 2".into(),
            parent: Some(2),
            visible: false,
            blend: "multiply".into(),
            opacity: 0.5,
            fill_opacity: 0.25,
            clipped: true,
            content: StackContent::Raster { pixels: PixelRef::ChannelGroup { part: 1, prefix: "Paint 2".into() }, sampling: PixelSampling::Baked },
        };
        let attr = LayerStack { layers: vec![layer, group], original_blend_profile: Some("srgb".into()), original_sample_depth: None }.to_attribute().unwrap();
        let attributes = [(ATTRIBUTE_NAME, attr.as_str())];
        let metadata = Metadata { chromaticities: Some(REC709), text_attributes: &[&attributes, &[]] };
        let mut encoded = Cursor::new(Vec::new());
        write_scanlines_with_metadata(&mut encoded, display, &parts, &metadata, |part, _, channel, mut samples| {
            if let RowSamples::F32(values) = &mut samples {
                values.fill(if part == 0 { 1.0 } else { [0.75, 0.125, 0.0, 0.5][channel] });
            }
            Ok(samples.len())
        })
        .unwrap();

        let result = crate::import("layers.exr", encoded.get_ref()).unwrap();
        let doc = result.document;
        assert_eq!(doc.size, Size::new(4, 3));
        assert_eq!(doc.depth, SampleType::F32);
        assert_eq!(doc.icc_profile, Some(Builtin::Srgb.profile().to_bytes()));
        assert_eq!(doc.layers.len(), 1);
        let group = &doc.layers[0];
        assert_eq!(group.blend, BlendMode::PassThrough);
        assert_eq!(group.opacity, 0.75);
        let LayerContent::Group(group) = &group.content else { panic!("expected a group") };
        let paint = &group.children[0];
        assert_eq!(paint.name, "Paint 2");
        assert!(!paint.visible && paint.clipped);
        assert_eq!((paint.blend, paint.opacity, paint.fill_opacity), (BlendMode::Multiply, 0.5, 0.25));
        let LayerContent::Raster(surface) = &paint.content else { panic!("expected pixels") };
        assert_eq!(surface.format(), PixelFormat::RGBA32F);
        let p = surface.rgba(-1, 1);
        assert!((p[0] - linear_to_srgb(1.5)).abs() < 1e-5);
        assert!((p[1] - linear_to_srgb(0.25)).abs() < 1e-5);
        assert_eq!(p[3], 0.5);
        assert_eq!(surface.rgba(1, 1), [0.0; 4]);
        assert!(result.warnings.iter().any(|s| s.contains("baked source pixels")));
    }
}
