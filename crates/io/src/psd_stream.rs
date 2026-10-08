//! Native import without a whole PSB byte buffer or whole decoded channel buffers.
use crate::{
    ImportResult, IoError,
    pixels::{interleave, max_sample, zero_sample},
};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_psd::{
    LayerRecord,
    stream::{Index, Rows},
};
use photocraft_raster::{Interrupt, Surface};
use std::{
    collections::HashMap,
    io::{BufReader, Read, Seek, SeekFrom, Write},
};

pub(crate) fn eligible(index: &Index) -> bool {
    let file = &index.metadata;
    matches!(
        file.header.color_mode,
        photocraft_psd::ColorMode::Rgb | photocraft_psd::ColorMode::Grayscale | photocraft_psd::ColorMode::Cmyk | photocraft_psd::ColorMode::Lab
    ) && matches!(file.header.depth, 8 | 16 | 32)
        && !file.layers().is_empty()
        && usize::from(file.header.channels) <= usize::from(file.header.color_mode.color_channels().unwrap_or(0)) + usize::from(file.merged_has_alpha())
}

struct Decode<'a, R> {
    open: &'a dyn Fn() -> std::io::Result<R>,
    index: &'a Index,
    ctl: Interrupt<'a>,
    done: u64,
    total: u64,
}
impl<R: Read + Seek> Decode<'_, R> {
    fn surface(&mut self, layer: usize, rec: &LayerRecord, ids: &[i16], fmt: PixelFormat, default: &[f32]) -> Result<Surface, IoError> {
        let r = if ids.len() == 1 { rec.channel_rect(*ids.first().ok_or_else(|| IoError::Unsupported("empty channel set".into()))?) } else { rec.rect };
        if r.is_empty() {
            return Ok(Surface::with_default(fmt, default));
        }
        let (width, height) = r.size()?;
        let working =
            (width as u64).saturating_mul(64).saturating_mul(fmt.bytes_per_pixel() as u64).saturating_mul(3).saturating_add(ids.len() as u64 * (3 << 20));
        let _working = photocraft_raster::memory::try_reserve(working).map_err(IoError::Unsupported)?;
        let ranges = self.index.channels.get(layer).ok_or_else(|| IoError::Unsupported("missing indexed layer".into()))?;
        let mut readers = Vec::new();
        for id in ids {
            readers.push(if let Some(range) = ranges.iter().find(|c| c.id == *id) {
                Some(Rows::new(
                    BufReader::with_capacity(1 << 20, (self.open)().map_err(|e| IoError::Unsupported(e.to_string()))?),
                    *range,
                    width,
                    height,
                    self.index.metadata.header.depth,
                    self.index.metadata.header.version,
                )?)
            } else {
                None
            });
        }
        let mut out = Surface::with_default(fmt, default);
        let fill: Vec<Vec<u8>> = ids.iter().map(|id| if *id == -1 { max_sample(fmt.sample) } else { zero_sample(fmt.sample) }).collect();
        let invert: Vec<bool> = ids.iter().map(|id| fmt.mode == ColorMode::Cmyk && *id >= 0).collect();
        let mut planes: Vec<Vec<u8>> = ids.iter().map(|_| Vec::new()).collect();
        let mut row = Vec::new();
        let mut y = 0usize;
        while y < height {
            self.ctl.check().map_err(|_| IoError::Cancelled)?;
            let rows = (height - y).min(64);
            for (reader, plane) in readers.iter_mut().zip(&mut planes) {
                plane.clear();
                if let Some(reader) = reader {
                    for _ in 0..rows {
                        self.ctl.check().map_err(|_| IoError::Cancelled)?;
                        reader.read_row(&mut row)?;
                        plane.extend_from_slice(&row);
                    }
                }
            }
            let refs: Vec<Option<&[u8]>> = readers.iter().zip(&planes).map(|(r, p)| r.as_ref().map(|_| p.as_slice())).collect();
            let mut bytes = interleave(&refs, &fill, width * rows, fmt.sample, &invert);
            if fmt.mode == ColorMode::Lab && fmt.sample == SampleType::U16 {
                crate::pixels::lab16_chroma(&mut bytes, ids.len(), true);
            }
            let top = i32::try_from(i64::from(r.top) + y as i64).map_err(|_| IoError::Unsupported("layer coordinates overflow".into()))?;
            let bottom = top.checked_add(rows as i32).ok_or_else(|| IoError::Unsupported("layer coordinates overflow".into()))?;
            out.write_interleaved(Rect::new(r.left, top, r.right, bottom), &bytes);
            y += rows;
            self.done = self.done.saturating_add(rows as u64 * ids.len() as u64);
            self.ctl.progress(0.05 + 0.85 * self.done as f32 / self.total.max(1) as f32);
        }
        for reader in readers.into_iter().flatten() {
            reader.finish()?;
        }
        out.prune();
        Ok(out)
    }
}

pub(crate) fn import<R: Read + Seek>(open: &dyn Fn() -> std::io::Result<R>, index: &Index, ctl: Interrupt<'_>) -> Result<ImportResult, IoError> {
    let file = &index.metadata;
    let mode = match file.header.color_mode {
        photocraft_psd::ColorMode::Rgb => ColorMode::Rgb,
        photocraft_psd::ColorMode::Grayscale => ColorMode::Grayscale,
        photocraft_psd::ColorMode::Cmyk => ColorMode::Cmyk,
        photocraft_psd::ColorMode::Lab => ColorMode::Lab,
        _ => return Err(IoError::Unsupported("stream color mode".into())),
    };
    let sample = crate::pixels::sample_for_depth(file.header.depth);
    let fmt = PixelFormat::new(mode, sample, true);
    let total = file.layers().iter().map(|r| r.channel_rect(0).size().map_or(0, |(_, h)| h as u64 * (mode.color_channels() as u64 + 1))).sum();
    let mut decoder = Decode { open, index, ctl, done: 0, total };
    let mut surfaces = HashMap::new();
    for (i, rec) in file.layers().iter().enumerate() {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let key = rec as *const LayerRecord as usize;
        let ids: Vec<i16> = (0..mode.color_channels() as i16).chain([-1]).collect();
        surfaces.insert((key, i16::MIN), decoder.surface(i, rec, &ids, fmt, &vec![0.0; fmt.channels()])?);
        for id in [-2, -3] {
            if rec.channel(id).is_some() {
                let default = rec.layer_mask().map_or(0, |m| if id == -3 { m.real.map_or(m.default_color, |m| m.background) } else { m.default_color });
                let fmt = PixelFormat::new(ColorMode::Grayscale, sample, false);
                surfaces.insert((key, id), decoder.surface(i, rec, &[id], fmt, &[f32::from(default) / 255.0])?);
            }
        }
    }
    let (document, warnings) = crate::psd_import::psd_to_document_decoded(file, &ctl, Some(&surfaces)).ok_or(IoError::Cancelled)?;
    Ok(ImportResult { document, warnings })
}

fn sink_error(e: std::io::Error) -> photocraft_psd::PsdError {
    photocraft_psd::PsdError::Invalid(format!("stream output: {e}"))
}

pub(crate) fn export_into<W: Write + Seek>(doc: &photocraft_doc::Document, out: &mut W, ctl: Interrupt<'_>) -> Result<Vec<String>, IoError> {
    if !matches!(doc.mode, ColorMode::Rgb | ColorMode::Grayscale | ColorMode::Cmyk | ColorMode::Lab) {
        return Err(IoError::Unsupported("seekable PSB export requires RGB, Gray, CMYK or Lab".into()));
    }
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let before = photocraft_raster::spill::read_error_generation();
    let fmt = doc.pixel_format();
    let metadata: u64 = doc
        .metadata
        .psd_resources
        .iter()
        .map(|(_, _, bytes)| bytes.len() as u64)
        .chain(doc.metadata.psd_global_blocks.iter().map(|(_, _, bytes)| bytes.len() as u64))
        .chain(doc.walk().iter().flat_map(|(_, _, l)| l.psd_blocks.iter().map(|(_, b)| b.len() as u64)))
        .fold(0_u64, u64::saturating_add);
    let embedded = doc
        .walk()
        .iter()
        .filter_map(|(_, _, l)| match &l.content {
            photocraft_doc::LayerContent::Smart(sm) => match &sm.source {
                photocraft_doc::SmartSource::Embedded { bytes, .. } => Some(bytes.len() as u64),
                _ => None,
            },
            _ => None,
        })
        .fold(0_u64, u64::saturating_add);
    let working = u64::from(doc.size.width)
        .saturating_mul(64)
        .saturating_mul(128)
        .saturating_add(64 << 20)
        .saturating_add(metadata.saturating_mul(4))
        .saturating_add(embedded.saturating_mul(4));
    let _working = photocraft_raster::memory::try_reserve(working).map_err(IoError::Unsupported)?;
    let opts = crate::psd_export::PsdExportOptions { force_psb: true, ..Default::default() };
    let (file, warnings, sources) = crate::psd_export::stream_metadata(doc, &opts);
    let check = || -> photocraft_psd::Result<()> {
        if ctl.cancelled() {
            return Err(photocraft_psd::PsdError::Invalid("stream export cancelled".into()));
        }
        photocraft_raster::spill::check_since(before).map_err(photocraft_psd::PsdError::Invalid)
    };
    let result = photocraft_psd::stream::write_layered(
        &file,
        out,
        |i, id, out| {
            check()?;
            let rec = file.layers().get(i).ok_or_else(|| photocraft_psd::PsdError::Invalid("missing output record".into()))?;
            let source = rec.layer_id().and_then(|id| sources.get(&id)).and_then(|(pixels, mask)| if id == -2 { mask.as_ref() } else { pixels.as_ref() });
            let r = rec.channel_rect(id);
            if r.is_empty() {
                return Ok(());
            }
            let s = source.ok_or_else(|| photocraft_psd::PsdError::Invalid("missing output pixel source".into()))?;
            let r = Rect::new(r.left, r.top, r.right, r.bottom);
            let channels = s.format().channels();
            let channel = if id == -1 { channels.saturating_sub(1) } else { id.max(0) as usize };
            let mut invert = vec![s.format().mode == ColorMode::Cmyk; channels];
            if (id < 0 || s.format().alpha)
                && let Some(a) = invert.last_mut()
            {
                *a = false;
            }
            let mut y = r.y0;
            while y < r.y1 {
                check()?;
                let band = Rect::new(r.x0, y, r.x1, y.saturating_add(64).min(r.y1));
                let mut bytes = s.to_interleaved(band);
                if s.format().mode == ColorMode::Lab && s.format().sample == SampleType::U16 {
                    crate::pixels::lab16_chroma(&mut bytes, channels, false);
                }
                let planes = crate::pixels::deinterleave(&bytes, channels, s.format().sample, &invert);
                out.write_all(planes.get(channel).ok_or_else(|| photocraft_psd::PsdError::Invalid("invalid output channel".into()))?).map_err(sink_error)?;
                y = band.y1;
            }
            Ok(())
        },
        |out| {
            check()?;
            let start = out.stream_position().map_err(sink_error)?;
            let row_bytes = u64::from(doc.size.width) * fmt.sample.bytes() as u64;
            let plane_bytes =
                row_bytes.checked_mul(u64::from(doc.size.height)).ok_or_else(|| photocraft_psd::PsdError::Invalid("output size overflow".into()))?;
            let cc = fmt.mode.color_channels();
            let white = photocraft_raster::from_rgba(&fmt, [1.0; 4]);
            let space = photocraft_compose::cmyk_space(doc);
            photocraft_compose::render_bands(doc, doc.bounds(), 64, |band| -> photocraft_psd::Result<()> {
                check()?;
                let mut planes: Vec<Vec<u8>> = (0..=cc).map(|_| Vec::with_capacity(band.px.len() * fmt.sample.bytes())).collect();
                photocraft_color::convert::with_cmyk_space(space.as_ref(), || {
                    let mut v = [0.0; 5];
                    for p in &band.px {
                        photocraft_raster::from_rgba_into(&fmt, *p, &mut v);
                        for (c, plane) in planes.iter_mut().enumerate() {
                            let value = v.get(c).copied().unwrap_or(0.0);
                            let value = if c < cc {
                                crate::pixels::matte(value, v.get(cc).copied().unwrap_or(0.0), white.get(c).copied().unwrap_or(1.0))
                            } else {
                                value
                            };
                            let value = if fmt.mode == ColorMode::Cmyk && c < cc { 1.0 - value } else { value };
                            let value = if fmt.mode == ColorMode::Lab && fmt.sample == SampleType::U16 && matches!(c, 1 | 2) {
                                value * crate::pixels::LAB16_CHROMA_MAX / 65535.0
                            } else {
                                value
                            };
                            crate::pixels::encode_be(value, fmt.sample, plane);
                        }
                    }
                });
                for (c, bytes) in planes.iter().enumerate() {
                    let offset = start
                        .checked_add(c as u64 * plane_bytes)
                        .and_then(|v| v.checked_add(band.rect.y0 as u64 * row_bytes))
                        .ok_or_else(|| photocraft_psd::PsdError::Invalid("output offset overflow".into()))?;
                    out.seek(SeekFrom::Start(offset)).map_err(sink_error)?;
                    out.write_all(bytes).map_err(sink_error)?;
                }
                ctl.progress(0.5 + 0.5 * band.rect.y1 as f32 / doc.size.height.max(1) as f32);
                Ok(())
            })?;
            for (i, channel) in doc.channels.iter().chain(doc.quick_mask.iter()).take(usize::from(file.header.channels).saturating_sub(cc + 1)).enumerate() {
                let index = cc + 1 + i;
                let mask_fmt = photocraft_color::PixelFormat::new(ColorMode::Grayscale, fmt.sample, false);
                let converted = (channel.surface.format() != mask_fmt).then(|| channel.surface.convert(mask_fmt));
                let surface = converted.as_ref().unwrap_or(&channel.surface);
                let mut y = 0;
                while y < doc.size.height {
                    check()?;
                    let end = y.saturating_add(64).min(doc.size.height);
                    let r = Rect::from_xywh(0, y as i32, doc.size.width, end - y);
                    let bytes = surface.to_interleaved(r);
                    let plane = crate::pixels::deinterleave(&bytes, 1, surface.format().sample, &[false]);
                    out.seek(SeekFrom::Start(start + index as u64 * plane_bytes + u64::from(y) * row_bytes)).map_err(sink_error)?;
                    out.write_all(plane.first().ok_or_else(|| photocraft_psd::PsdError::Invalid("missing extra output plane".into()))?).map_err(sink_error)?;
                    y = end;
                }
            }
            let end = start
                .checked_add(u64::from(file.header.channels).saturating_mul(plane_bytes))
                .ok_or_else(|| photocraft_psd::PsdError::Invalid("output size overflow".into()))?;
            out.seek(SeekFrom::Start(end)).map_err(sink_error)?;
            check()
        },
    );
    if ctl.cancelled() {
        return Err(IoError::Cancelled);
    }
    result?;
    photocraft_raster::spill::check_since(before).map_err(IoError::Unsupported)?;
    Ok(warnings)
}
