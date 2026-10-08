//! Seekable PSD/PSB metadata and row decoding. Pixel ranges stay in the source; no whole-file
//! or whole-channel buffer is created. `metadata` deliberately contains no encoded pixels.
use std::io::{Read, Seek, SeekFrom, Take, Write};

use crate::image_data::ImageData;
use crate::io::Reader;
use crate::{Compression, GlobalLayerMask, Header, LayerInfo, LayerInfoPlacement, LayerRecord, PlaneLayout, PsdError, PsdFile, Result, TaggedBlock, Version};

fn io(e: std::io::Error) -> PsdError {
    PsdError::invalid(format!("stream I/O: {e}"))
}
fn invalid() -> PsdError {
    PsdError::invalid("invalid stream range")
}
fn array<const N: usize>(r: &mut impl Read) -> Result<[u8; N]> {
    let mut b = [0; N];
    r.read_exact(&mut b).map_err(io)?;
    Ok(b)
}
fn length(r: &mut impl Read, long: bool) -> Result<u64> {
    Ok(if long { u64::from_be_bytes(array(r)?) } else { u64::from(u32::from_be_bytes(array(r)?)) })
}

#[derive(Clone, Copy, Debug)]
/// One checked encoded layer-channel range, excluding its compression marker.
pub struct ChannelRange {
    /// Photoshop channel id (negative for transparency and masks).
    pub id: i16,
    /// Absolute byte offset in the source.
    pub offset: u64,
    /// Encoded byte count.
    pub length: u64,
    /// Channel codec.
    pub compression: Compression,
}

/// Metadata and independent source ranges; encoded pixels are never retained in this model.
pub struct Index {
    /// Parsed metadata only. Do not serialize this model: pixel channels are in `channels`.
    pub metadata: PsdFile,
    /// Ranges per layer record, in stored order.
    pub channels: Vec<Vec<ChannelRange>>,
}

struct Scan<'a, R> {
    reader: &'a mut R,
    allowance: u64,
    cancelled: &'a dyn Fn() -> bool,
}
impl<R: Read + Seek> Scan<'_, R> {
    fn position(&mut self) -> Result<u64> {
        self.reader.stream_position().map_err(io)
    }
    fn end(&mut self, length: u64, boundary: u64) -> Result<u64> {
        self.position()?.checked_add(length).filter(|end| *end <= boundary).ok_or_else(invalid)
    }
    fn bytes(&mut self, n: u64, boundary: u64) -> Result<Vec<u8>> {
        self.end(n, boundary)?;
        self.allowance = self.allowance.checked_sub(n).ok_or(PsdError::LimitExceeded("stream metadata exceeds working-memory budget"))?;
        let n = usize::try_from(n).map_err(|_| invalid())?;
        let mut out = Vec::new();
        out.try_reserve_exact(n).map_err(|_| PsdError::LimitExceeded("stream metadata allocation"))?;
        out.resize(n, 0);
        for chunk in out.chunks_mut(64 << 10) {
            if (self.cancelled)() {
                return Err(PsdError::invalid("stream cancelled"));
            }
            self.reader.read_exact(chunk).map_err(io)?;
        }
        Ok(out)
    }
    fn layers(&mut self, end: u64, version: Version) -> Result<(LayerInfo, Vec<Vec<ChannelRange>>)> {
        let start = self.position()?;
        let count = i16::from_be_bytes(array(self.reader)?);
        let n = usize::from(count.unsigned_abs());
        if (n as u64).saturating_mul(34) > end.saturating_sub(self.position()?) {
            return Err(invalid());
        }
        let mut layers = Vec::with_capacity(n);
        let mut lengths = Vec::with_capacity(n);
        for _ in 0..n {
            let mut record = self.bytes(18, end)?;
            let nch = u16::from_be_bytes(record.get(16..18).ok_or_else(invalid)?.try_into().map_err(|_| invalid())?);
            if nch > crate::layer::MAX_LAYER_CHANNELS {
                return Err(PsdError::LimitExceeded("too many layer channels"));
            }
            let fixed = u64::from(nch) * if version.is_psb() { 10 } else { 6 } + 16;
            record.extend(self.bytes(fixed, end)?);
            let extra = record.get(record.len().saturating_sub(4)..).ok_or_else(invalid)?;
            let extra = u32::from_be_bytes(extra.try_into().map_err(|_| invalid())?);
            record.extend(self.bytes(u64::from(extra), end)?);
            let (rec, lens) = LayerRecord::read(&mut Reader::new(&record), version)?;
            layers.push(rec);
            lengths.push(lens);
        }
        let mut ranges = Vec::with_capacity(n);
        for (rec, lens) in layers.iter_mut().zip(lengths) {
            let mut channels = Vec::new();
            for (ch, len) in rec.channels.iter_mut().zip(lens) {
                if len == 0 {
                    continue;
                }
                if len < 2 {
                    return Err(invalid());
                }
                let channel_end = self.end(len, end)?;
                let compression = Compression::from_u16(u16::from_be_bytes(array(self.reader)?));
                ch.compression = Some(compression);
                channels.push(ChannelRange { id: ch.id, offset: self.position()?, length: len - 2, compression });
                self.reader.seek(SeekFrom::Start(channel_end)).map_err(io)?;
            }
            ranges.push(channels);
        }
        let body = self.position()?.saturating_sub(start);
        let rest = end.saturating_sub(self.position()?);
        let pad = self.bytes(rest, end)?;
        let padding = if pad.len() as u64 == body % 2 && pad.iter().all(|b| *b == 0) { None } else { Some(pad) };
        Ok((LayerInfo { merged_alpha: count < 0, layers, padding }, ranges))
    }
}

impl Index {
    /// Index a seekable file with an aggregate metadata cap and cooperative cancellation.
    pub fn read<R: Read + Seek>(reader: &mut R, metadata_budget: u64, cancelled: &dyn Fn() -> bool) -> Result<Self> {
        let end = reader.seek(SeekFrom::End(0)).map_err(io)?;
        reader.seek(SeekFrom::Start(0)).map_err(io)?;
        let header = Header::read(&mut Reader::new(&array::<26>(reader)?))?;
        let mut s = Scan { reader, allowance: metadata_budget, cancelled };
        let n = length(s.reader, false)?;
        let color_mode_data = s.bytes(n, end)?;
        let n = length(s.reader, false)?;
        let resources = s.bytes(n, end)?;
        let mut resource_bytes = (n as u32).to_be_bytes().to_vec();
        resource_bytes.extend(resources);
        let resources = crate::resources::read_section(&mut Reader::new(&resource_bytes))?;
        drop(resource_bytes);
        let lm_len = length(s.reader, header.version.is_psb())?;
        let lm_end = s.end(lm_len, end)?;
        let mut layer_info = None;
        let mut channels = Vec::new();
        let mut global_layer_mask = None;
        let mut global_blocks = Vec::new();
        let mut layer_mask_trailing = Vec::new();
        let mut layer_info_placement = LayerInfoPlacement::Section;
        if lm_len > 0 {
            let n = length(s.reader, header.version.is_psb())?;
            let li_end = s.end(n, lm_end)?;
            if n > 0 {
                let (info, c) = s.layers(li_end, header.version)?;
                layer_info = Some(info);
                channels = c;
            }
            if lm_end.saturating_sub(s.position()?) >= 4 {
                let n = length(s.reader, false)?;
                global_layer_mask = Some(GlobalLayerMask { data: s.bytes(n, lm_end)? });
                while lm_end.saturating_sub(s.position()?) >= 12 {
                    let start = s.position()?;
                    let signature = array::<4>(s.reader)?;
                    if signature != *b"8BIM" && signature != *b"8B64" {
                        s.reader.seek(SeekFrom::Start(start)).map_err(io)?;
                        break;
                    }
                    let key = array::<4>(s.reader)?;
                    let n = length(s.reader, crate::tagged::uses_long_length(header.version, &key))?;
                    let block_end = s.end(n, lm_end)?;
                    let is_layers = layer_info.is_none() && matches!(&key, b"Lr16" | b"Lr32" | b"Layr");
                    let data = if is_layers {
                        let (info, c) = s.layers(block_end, header.version)?;
                        layer_info = Some(info);
                        channels = c;
                        Vec::new()
                    } else {
                        s.bytes(n, lm_end)?
                    };
                    // Same padding detector as the byte parser, without reading the next block.
                    let at = s.position()?;
                    let rest = lm_end.saturating_sub(at);
                    let peek = s.bytes(rest.min(7), lm_end)?;
                    s.reader.seek(SeekFrom::Start(at)).map_err(io)?;
                    let pad = (0..=3usize)
                        .find(|k| *k as u64 == rest || peek.get(*k..).is_some_and(|b| b.starts_with(b"8BIM") || b.starts_with(b"8B64")))
                        .unwrap_or(0);
                    let bytes = s.bytes(pad as u64, lm_end)?;
                    let padding = if bytes.len() as u64 == n % 2 && bytes.iter().all(|b| *b == 0) { None } else { Some(bytes) };
                    if is_layers {
                        layer_info_placement = LayerInfoPlacement::GlobalBlock { index: global_blocks.len(), signature, key, padding };
                    } else {
                        global_blocks.push(TaggedBlock { signature, key, data, padding });
                    }
                }
            }
            let left = lm_end.saturating_sub(s.position()?);
            layer_mask_trailing = s.bytes(left, lm_end)?;
        }
        // The merged image is intentionally not read: layered imports below consume only layers.
        let compression = Compression::from_u16(u16::from_be_bytes(array(s.reader)?));
        Ok(Self {
            metadata: PsdFile {
                header,
                color_mode_data,
                resources,
                layer_info,
                layer_info_placement,
                global_layer_mask,
                global_blocks,
                layer_mask_trailing,
                image_data: ImageData { compression, data: Vec::new() },
            },
            channels,
        })
    }
}

enum Input<R: Read> {
    Raw(Take<R>),
    Rle(Take<R>, Vec<usize>),
    Zip(flate2::read::ZlibDecoder<Take<R>>),
}
/// A sequential channel decoder with bounded row buffers, including ZIP prediction.
pub struct Rows<R: Read> {
    input: Input<R>,
    layout: PlaneLayout,
    prediction: bool,
    row: usize,
    encoded: Vec<u8>,
}
impl<R: Read + Seek> Rows<R> {
    /// Open a checked range on an independent reader. Clone file handles with shared seek
    /// positions must not be used for concurrent channels.
    pub fn new(mut reader: R, range: ChannelRange, width: usize, height: usize, depth: u16, version: Version) -> Result<Self> {
        if width > 300_000 || height > 300_000 || !matches!(depth, 1 | 8 | 16 | 32) {
            return Err(PsdError::LimitExceeded("stream channel dimensions"));
        }
        reader.seek(SeekFrom::Start(range.offset)).map_err(io)?;
        let mut source = reader.take(range.length);
        let layout = PlaneLayout { planes: 1, width, height, depth, version };
        let input = match range.compression {
            Compression::Raw => Input::Raw(source),
            Compression::Rle => {
                let mut counts = Vec::with_capacity(height);
                let mut total = (height as u64).checked_mul(if version.is_psb() { 4 } else { 2 }).ok_or_else(invalid)?;
                for _ in 0..height {
                    let n =
                        if version.is_psb() { u32::from_be_bytes(array(&mut source)?) as usize } else { usize::from(u16::from_be_bytes(array(&mut source)?)) };
                    if n > layout.row_bytes().saturating_mul(2).saturating_add(1024) {
                        return Err(PsdError::LimitExceeded("encoded RLE row"));
                    }
                    total = total.checked_add(n as u64).ok_or_else(invalid)?;
                    counts.push(n);
                }
                if total > range.length {
                    return Err(invalid());
                }
                Input::Rle(source, counts)
            }
            Compression::Zip | Compression::ZipPrediction => Input::Zip(flate2::read::ZlibDecoder::new(source)),
            Compression::Unknown(n) => return Err(PsdError::Unsupported(format!("stream compression {n}"))),
        };
        Ok(Self { input, layout, prediction: range.compression == Compression::ZipPrediction, row: 0, encoded: Vec::new() })
    }
    /// Decode the next native big-endian sample row, reusing `out`.
    pub fn read_row(&mut self, out: &mut Vec<u8>) -> Result<()> {
        if self.row >= self.layout.height {
            return Err(invalid());
        }
        let bytes = self.layout.row_bytes();
        out.resize(bytes, 0);
        match &mut self.input {
            Input::Raw(r) => r.read_exact(out).map_err(io)?,
            Input::Zip(r) => r.read_exact(out).map_err(io)?,
            Input::Rle(r, counts) => {
                out.clear();
                self.encoded.resize(*counts.get(self.row).ok_or_else(invalid)?, 0);
                r.read_exact(&mut self.encoded).map_err(io)?;
                crate::compression::packbits::decode_into(&self.encoded, bytes, out)?;
            }
        }
        if self.prediction {
            let row = PlaneLayout { height: 1, ..self.layout };
            crate::compression::unpredict(out, &row)?;
        }
        self.row += 1;
        Ok(())
    }
    /// Checks the zlib checksum and rejects extra decoded samples after the expected last row.
    pub fn finish(mut self) -> Result<()> {
        if self.row != self.layout.height {
            return Err(invalid());
        }
        let mut extra = [0; 1];
        let n = match &mut self.input {
            Input::Raw(r) | Input::Rle(r, _) => r.read(&mut extra),
            Input::Zip(r) => r.read(&mut extra),
        }
        .map_err(io)?;
        if n != 0 {
            return Err(PsdError::invalid("extra stream channel samples"));
        }
        Ok(())
    }
}

fn patch<W: Write + Seek>(out: &mut W, at: u64, value: u64) -> Result<()> {
    let end = out.stream_position().map_err(io)?;
    out.seek(SeekFrom::Start(at)).map_err(io)?;
    out.write_all(&value.to_be_bytes()).map_err(io)?;
    out.seek(SeekFrom::Start(end)).map_err(io)?;
    Ok(())
}

fn write_layers<W: Write + Seek>(out: &mut W, info: &LayerInfo, pixels: &mut impl FnMut(usize, i16, &mut W) -> Result<()>) -> Result<()> {
    let start = out.stream_position().map_err(io)?;
    let count = i16::try_from(info.layers.len()).map_err(|_| PsdError::LimitExceeded("too many streamed layers"))?;
    out.write_all(&(if info.merged_alpha { -count } else { count }).to_be_bytes()).map_err(io)?;
    let mut records = Vec::with_capacity(info.layers.len());
    for rec in &info.layers {
        records.push(out.stream_position().map_err(io)?);
        let mut encoded = Vec::new();
        rec.write(&mut encoded, Version::Psb)?;
        out.write_all(&encoded).map_err(io)?;
    }
    for (i, rec) in info.layers.iter().enumerate() {
        for (c, channel) in rec.channels.iter().enumerate() {
            let begin = out.stream_position().map_err(io)?;
            if let Some(compression) = channel.compression {
                out.write_all(&compression.as_u16().to_be_bytes()).map_err(io)?;
                pixels(i, channel.id, out)?;
            }
            let len = out.stream_position().map_err(io)?.saturating_sub(begin);
            let at = records.get(i).copied().ok_or_else(invalid)?.checked_add(20 + c as u64 * 10).ok_or_else(invalid)?;
            patch(out, at, len)?;
        }
    }
    let length = out.stream_position().map_err(io)?.saturating_sub(start);
    let pad = (4 - length % 4) % 4;
    for _ in 0..pad {
        out.write_all(&[0]).map_err(io)?;
    }
    Ok(())
}

/// Write a PSB directly to a seekable sink, patching channel and section lengths after each
/// bounded encoded stream. Callbacks supply encoded layer channels and the merged image; this
/// function never buffers their data. Publication/atomic replacement belongs to the caller.
pub fn write_layered<W: Write + Seek>(
    file: &PsdFile,
    out: &mut W,
    mut pixels: impl FnMut(usize, i16, &mut W) -> Result<()>,
    merged: impl FnOnce(&mut W) -> Result<()>,
) -> Result<()> {
    if file.header.version != Version::Psb {
        return Err(PsdError::Unsupported("seekable writer requires PSB".into()));
    }
    file.header.validate()?;
    let mut prefix = Vec::new();
    file.header.write(&mut prefix);
    prefix.extend(u32::try_from(file.color_mode_data.len()).map_err(|_| invalid())?.to_be_bytes());
    prefix.extend(&file.color_mode_data);
    crate::resources::write_section(&mut prefix, &file.resources)?;
    out.write_all(&prefix).map_err(io)?;
    let lm_at = out.stream_position().map_err(io)?;
    out.write_all(&0u64.to_be_bytes()).map_err(io)?;
    let start = out.stream_position().map_err(io)?;
    let li_at = start;
    out.write_all(&0u64.to_be_bytes()).map_err(io)?;
    if let (Some(info), LayerInfoPlacement::Section) = (&file.layer_info, &file.layer_info_placement) {
        let begin = out.stream_position().map_err(io)?;
        write_layers(out, info, &mut pixels)?;
        let n = out.stream_position().map_err(io)?.saturating_sub(begin);
        patch(out, li_at, n)?;
    }
    let mask = file.global_layer_mask.as_ref().map_or(&[][..], |m| m.data.as_slice());
    out.write_all(&u32::try_from(mask.len()).map_err(|_| invalid())?.to_be_bytes()).map_err(io)?;
    out.write_all(mask).map_err(io)?;
    for i in 0..=file.global_blocks.len() {
        if let (Some(info), LayerInfoPlacement::GlobalBlock { index, signature, key, .. }) = (&file.layer_info, &file.layer_info_placement)
            && i == (*index).min(file.global_blocks.len())
        {
            out.write_all(signature).map_err(io)?;
            out.write_all(key).map_err(io)?;
            let at = out.stream_position().map_err(io)?;
            out.write_all(&0u64.to_be_bytes()).map_err(io)?;
            let begin = out.stream_position().map_err(io)?;
            write_layers(out, info, &mut pixels)?;
            let n = out.stream_position().map_err(io)?.saturating_sub(begin);
            patch(out, at, n)?;
        }
        if let Some(block) = file.global_blocks.get(i) {
            let mut bytes = Vec::new();
            crate::tagged::write_blocks(&mut bytes, std::slice::from_ref(block), Version::Psb)?;
            out.write_all(&bytes).map_err(io)?;
        }
    }
    out.write_all(&file.layer_mask_trailing).map_err(io)?;
    let n = out.stream_position().map_err(io)?.saturating_sub(start);
    patch(out, lm_at, n)?;
    out.write_all(&file.image_data.compression.as_u16().to_be_bytes()).map_err(io)?;
    merged(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn indexed_channels_match_byte_decoder_at_every_depth_and_compression() {
        for version in [Version::Psd, Version::Psb] {
            for depth in [8, 16, 32] {
                for compression in Compression::ALL {
                    let file = crate::testgen::layered(version, crate::ColorMode::Rgb, depth, compression);
                    let bytes = file.to_bytes().unwrap();
                    let index = Index::read(&mut Cursor::new(&bytes), 32 << 20, &|| false).unwrap();
                    assert_eq!(index.metadata.resources, file.resources);
                    assert_eq!(index.metadata.global_blocks, file.global_blocks);
                    for (rec, ranges) in index.metadata.layers().iter().zip(&index.channels) {
                        for range in ranges {
                            let (w, h) = rec.channel_rect(range.id).size().unwrap();
                            let mut rows = Rows::new(Cursor::new(&bytes), *range, w, h, depth, version).unwrap();
                            let mut decoded = Vec::new();
                            let mut row = Vec::new();
                            for _ in 0..h {
                                rows.read_row(&mut row).unwrap();
                                decoded.extend_from_slice(&row);
                            }
                            rows.finish().unwrap();
                            let original = file.layers().iter().find(|l| l.name == rec.name).unwrap();
                            assert_eq!(decoded, original.decode_channel(range.id, depth, version).unwrap());
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn bounded_index_and_cancel_do_not_allocate_declared_pixel_bytes() {
        let file = crate::testgen::layered(Version::Psb, crate::ColorMode::Rgb, 8, Compression::Rle);
        let bytes = file.to_bytes().unwrap();
        assert!(Index::read(&mut Cursor::new(&bytes), 2, &|| false).is_err());
        assert!(Index::read(&mut Cursor::new(&bytes), 32 << 20, &|| true).is_err());
        for n in [0, 25, 30, bytes.len() / 2] {
            assert!(Index::read(&mut Cursor::new(&bytes[..n]), 32 << 20, &|| false).is_err());
        }
    }

    #[test]
    fn rows_reject_corrupt_checksum_and_truncated_data() {
        let layout = PlaneLayout { planes: 1, width: 10, height: 4, depth: 8, version: Version::Psb };
        let encoded = crate::compression::encode_planes(Compression::Zip, &[1; 40], &layout).unwrap();
        for corrupt in [false, true] {
            let mut bytes = encoded.clone();
            if corrupt {
                let n = bytes.len();
                bytes[n - 1] ^= 0xff;
            } else {
                bytes.truncate(bytes.len() - 3);
            }
            let range = ChannelRange { id: 0, offset: 0, length: bytes.len() as u64, compression: Compression::Zip };
            let mut rows = Rows::new(Cursor::new(&bytes), range, 10, 4, 8, Version::Psb).unwrap();
            let mut row = Vec::new();
            let mut error = false;
            for _ in 0..4 {
                if rows.read_row(&mut row).is_err() {
                    error = true;
                    break;
                }
            }
            assert!(error || rows.finish().is_err(), "truncated or corrupt zlib must fail");
        }
    }

    #[test]
    fn seekable_writer_patches_layer_channel_and_global_block_lengths() {
        for depth in [8, 16, 32] {
            for compression in Compression::ALL {
                let file = crate::testgen::layered(Version::Psb, crate::ColorMode::Rgb, depth, compression);
                let mut out = Cursor::new(Vec::new());
                write_layered(
                    &file,
                    &mut out,
                    |layer, id, out| out.write_all(&file.layers()[layer].channel(id).unwrap().data).map_err(io),
                    |out| out.write_all(&file.image_data.data).map_err(io),
                )
                .unwrap();
                let read = PsdFile::from_bytes(&out.into_inner()).unwrap();
                assert_eq!(read.header, file.header);
                assert_eq!(read.global_blocks, file.global_blocks);
                assert_eq!(read.resources, file.resources);
                assert_eq!(read.image_data, file.image_data);
                assert_eq!(read.layers(), file.layers());
            }
        }
    }
}
