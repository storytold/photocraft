//! Decoding a channel a band of rows at a time: [`ChannelData::row_decoder`].
//!
//! [`ChannelData::decode`] returns the whole plane, so turning a layer into tiles holds every
//! channel's plane plus an interleaved copy at once: for a 20000×15000 RGBA layer that is 2.4 GB
//! on top of the tiles. A [`RowDecoder`] hands out any band of rows instead. RLE and raw channels
//! are read band by band (a channel still in its [`Stored`](crate::Stored) source reads only the
//! band's bytes: RLE's per-row byte counts give each row's offset); ZIP streams cannot be entered
//! part way, so a ZIP channel is inflated once, when the decoder is made.
//!
//! The merged image has the same decoders, one per plane, while it is raw or RLE:
//! [`ImageData::row_decoders`].

use std::borrow::Cow;

use crate::compression::{Compression, MAX_DECODED_BYTES, PlaneLayout, decode_planes, packbits};
use crate::error::{PsdError, Result};
use crate::header::{Header, Version};
use crate::image_data::ImageData;
use crate::layer::ChannelData;
use crate::source::Stored;

/// Where a channel's encoded bytes are.
#[derive(Clone, Copy)]
enum Bytes<'a> {
    Memory(&'a [u8]),
    Source(&'a Stored),
}

impl<'a> Bytes<'a> {
    fn of(data: &'a [u8], stored: Option<&'a Stored>) -> Self {
        stored.map_or(Bytes::Memory(data), Bytes::Source)
    }

    fn len(&self) -> u64 {
        match self {
            Bytes::Memory(b) => b.len() as u64,
            Bytes::Source(s) => s.len,
        }
    }

    fn get(&self, at: u64, len: u64) -> Result<Cow<'_, [u8]>> {
        match self {
            Bytes::Memory(b) => {
                let range = usize::try_from(at).ok().zip(usize::try_from(len).ok()).and_then(|(a, n)| b.get(a..a.checked_add(n)?));
                range
                    .map(Cow::Borrowed)
                    .ok_or(PsdError::UnexpectedEof { offset: usize::try_from(at).unwrap_or(usize::MAX), needed: usize::try_from(len).unwrap_or(usize::MAX) })
            }
            Bytes::Source(s) => {
                if at.checked_add(len).is_none_or(|end| end > s.len) {
                    return Err(PsdError::UnexpectedEof {
                        offset: usize::try_from(at).unwrap_or(usize::MAX),
                        needed: usize::try_from(len).unwrap_or(usize::MAX),
                    });
                }
                s.read_part(at, len).map(Cow::Owned)
            }
        }
    }
}

enum Kind<'a> {
    /// Rows at `at + row * row_bytes`.
    Raw { bytes: Bytes<'a>, at: u64 },
    /// `offsets[r]..offsets[r + 1]` are row `r`'s PackBits bytes (offsets from the start of the
    /// data, after the count table).
    Rle { bytes: Bytes<'a>, offsets: Vec<u64> },
    /// The whole decoded plane.
    Decoded(Vec<u8>),
    /// No data and no rows.
    Empty,
}

/// Decodes bands of rows of one channel: see the [module docs](self).
pub struct RowDecoder<'a> {
    kind: Kind<'a>,
    row_bytes: usize,
    height: usize,
}

impl ChannelData {
    /// A decoder of this channel's rows, for a `width × height` plane. Fails where
    /// [`Self::decode`] would on everything it can check without decoding the rows (sizes, the
    /// RLE count table, the whole of a ZIP stream); a damaged row fails when its band is decoded.
    pub fn row_decoder(&self, width: usize, height: usize, depth: u16, version: Version) -> Result<RowDecoder<'_>> {
        let layout = PlaneLayout { planes: 1, width, height, depth, version };
        let total = layout.decoded_len()?;
        let row_bytes = layout.row_bytes();
        let bytes = Bytes::of(&self.data, self.stored.as_ref());
        let kind = match self.compression {
            None if total == 0 => Kind::Empty,
            None => return Err(PsdError::invalid("channel has no data")),
            Some(Compression::Raw) => {
                check_raw_len(&bytes, total as u64)?;
                Kind::Raw { bytes, at: 0 }
            }
            Some(Compression::Rle) => Kind::Rle { bytes, offsets: rle_offsets(&bytes, height, total as u64, version)? },
            Some(c) => Kind::Decoded(decode_planes(c, &self.encoded()?, &layout)?),
        };
        Ok(RowDecoder { kind, row_bytes, height })
    }
}

impl ImageData {
    /// A decoder of the rows of each plane of the merged image `header` describes, while it is
    /// raw or RLE: the planes are then read a band at a time (straight from the file when the
    /// bytes were left in the source) and never held whole, so the image is not bound by
    /// [`MAX_DECODED_BYTES`] as a whole; each band is. A ZIP image has no such decoders (it is an
    /// error to ask): [`Self::decode`] inflates it whole.
    ///
    /// Fails on what can be checked without decoding the rows (sizes, the RLE count table); a
    /// damaged row fails when its band is decoded.
    pub fn row_decoders(&self, header: &Header) -> Result<Vec<RowDecoder<'_>>> {
        let layout = Self::layout(header);
        let (row_bytes, height) = (layout.row_bytes(), layout.height);
        let total = layout.total_bytes()?;
        let bytes = Bytes::of(&self.data, self.stored.as_ref());
        let kinds: Vec<Kind<'_>> = match self.compression {
            Compression::Raw => {
                check_raw_len(&bytes, total)?;
                let plane = (row_bytes as u64).saturating_mul(height as u64);
                (0..layout.planes as u64).map(|p| Kind::Raw { bytes, at: p.saturating_mul(plane) }).collect()
            }
            Compression::Rle => {
                // One count table for the rows of every plane: each plane takes its part.
                let offsets = rle_offsets(&bytes, layout.rows()?, total, header.version)?;
                let plane = |p: usize| offsets.get(p.checked_mul(height)?..=p.checked_add(1)?.checked_mul(height)?);
                (0..layout.planes)
                    .map(|p| {
                        let rows = plane(p).ok_or_else(|| PsdError::invalid("RLE row table too short"))?;
                        let mut part = Vec::new();
                        part.try_reserve_exact(rows.len()).map_err(|_| PsdError::OutOfMemory("not enough memory for the RLE row table"))?;
                        part.extend_from_slice(rows);
                        Ok(Kind::Rle { bytes, offsets: part })
                    })
                    .collect::<Result<_>>()?
            }
            _ => return Err(PsdError::invalid("only a raw or RLE merged image is decoded by rows")),
        };
        Ok(kinds.into_iter().map(|kind| RowDecoder { kind, row_bytes, height }).collect())
    }
}

/// Raw data of `total` decoded bytes must all be there.
fn check_raw_len(bytes: &Bytes<'_>, total: u64) -> Result<()> {
    if bytes.len() < total {
        return Err(PsdError::UnexpectedEof {
            offset: usize::try_from(bytes.len()).unwrap_or(usize::MAX),
            needed: usize::try_from(total - bytes.len()).unwrap_or(usize::MAX),
        });
    }
    Ok(())
}

/// Reads the RLE count table of `rows` rows at the start of `bytes`: the offset of each row's
/// PackBits bytes from the start of the data, then that of the end of the last row. `total` is
/// the decoded size of the rows together.
fn rle_offsets(bytes: &Bytes<'_>, rows: usize, total: u64, version: Version) -> Result<Vec<u64>> {
    let size: u64 = if version.is_psb() { 4 } else { 2 };
    let table_len = (rows as u64).saturating_mul(size);
    if table_len > bytes.len() {
        return Err(PsdError::UnexpectedEof { offset: 0, needed: usize::try_from(table_len - bytes.len()).unwrap_or(usize::MAX) });
    }
    let table = bytes.get(0, table_len)?;
    let mut offsets = Vec::new();
    offsets.try_reserve_exact(rows.saturating_add(1)).map_err(|_| PsdError::OutOfMemory("not enough memory for the RLE row table"))?;
    let mut at = table_len;
    offsets.push(at);
    for c in table.chunks_exact(size as usize) {
        let n = match *c {
            [a, b] => u64::from(u16::from_be_bytes([a, b])),
            [a, b, c, d] => u64::from(u32::from_be_bytes([a, b, c, d])),
            _ => 0,
        };
        at = at.saturating_add(n);
        offsets.push(at);
    }
    let data_len = bytes.len() - table_len;
    // As `decode_planes` checks: PackBits expands at most 64x.
    if total > data_len.saturating_mul(64) {
        return Err(PsdError::Decompress("RLE data too short for declared size".into()));
    }
    if at > bytes.len() {
        return Err(PsdError::UnexpectedEof {
            offset: usize::try_from(bytes.len()).unwrap_or(usize::MAX),
            needed: usize::try_from(at - bytes.len()).unwrap_or(usize::MAX),
        });
    }
    Ok(offsets)
}

impl RowDecoder<'_> {
    /// Bytes per decoded row.
    pub fn row_bytes(&self) -> usize {
        self.row_bytes
    }

    /// Number of rows.
    pub fn height(&self) -> usize {
        self.height
    }

    /// Rows `r0..r1` as planar big-endian samples (`(r1 - r0) × row_bytes` bytes).
    pub fn rows(&self, r0: usize, r1: usize) -> Result<Vec<u8>> {
        if r0 > r1 || r1 > self.height {
            return Err(PsdError::invalid(format!("rows {r0}..{r1} outside 0..{}", self.height)));
        }
        let n = (r1 - r0).checked_mul(self.row_bytes).ok_or(PsdError::LimitExceeded("band size overflow"))?;
        if n as u64 > MAX_DECODED_BYTES {
            return Err(PsdError::LimitExceeded("decoded channel data exceeds MAX_DECODED_BYTES"));
        }
        let mut out = Vec::new();
        out.try_reserve_exact(n).map_err(|_| PsdError::OutOfMemory("not enough memory for the decoded channel data"))?;
        match &self.kind {
            Kind::Empty => {}
            Kind::Raw { bytes, at } => out.extend_from_slice(&bytes.get(at.saturating_add((r0 as u64).saturating_mul(self.row_bytes as u64)), n as u64)?),
            Kind::Decoded(plane) => {
                out.extend_from_slice(plane.get(r0 * self.row_bytes..r1 * self.row_bytes).ok_or_else(|| PsdError::invalid("decoded plane too short"))?)
            }
            Kind::Rle { bytes, offsets } => {
                let (Some(&start), Some(&end)) = (offsets.get(r0), offsets.get(r1)) else {
                    return Err(PsdError::invalid("RLE row table too short"));
                };
                let band = bytes.get(start, end - start)?;
                for r in r0..r1 {
                    let (Some(&a), Some(&b)) = (offsets.get(r), offsets.get(r + 1)) else {
                        return Err(PsdError::invalid("RLE row table too short"));
                    };
                    let row = band.get((a - start) as usize..(b - start) as usize).ok_or_else(|| PsdError::invalid("RLE row outside its band"))?;
                    packbits::decode_into(row, self.row_bytes, &mut out)?;
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use crate::source::Source;
    use crate::{PsdFile, testgen};

    /// Every channel of every generated file, decoded in uneven bands from memory and from a
    /// source, is its whole-plane decode.
    #[test]
    fn bands_join_up_to_the_whole_plane() {
        let mut checked = 0;
        for case in testgen::all_cases() {
            let bytes = case.file.to_bytes().unwrap();
            let eager = PsdFile::from_bytes(&bytes).unwrap();
            let lazy = PsdFile::from_source(Source::new(bytes)).unwrap();
            let (d, v) = (eager.header.depth, eager.header.version);
            for file in [&eager, &lazy] {
                for rec in file.layers() {
                    for ch in &rec.channels {
                        let (w, h) = rec.channel_rect(ch.id).size().unwrap();
                        let whole = ch.decode(w, h, d, v);
                        let dec = ch.row_decoder(w, h, d, v);
                        let (Ok(whole), Ok(dec)) = (whole, dec) else { continue };
                        let mut joined = Vec::new();
                        let mut r = 0;
                        for step in [1, 3, 2, 7].iter().cycle() {
                            if r >= h {
                                break;
                            }
                            let r1 = (r + step).min(h);
                            joined.extend(dec.rows(r, r1).unwrap());
                            r = r1;
                        }
                        assert_eq!(joined, whole, "{}", case.name);
                        assert!(dec.rows(0, h + 1).is_err());
                        checked += 1;
                    }
                }
            }
        }
        assert!(checked > 100, "{checked}");
    }

    /// The planes of every generated raw or RLE merged image, decoded in uneven bands from
    /// memory and from a source, are its whole decode. A ZIP image has no row decoders.
    #[test]
    fn merged_bands_join_up_to_the_whole_image() {
        use crate::Compression;
        let mut checked = 0;
        for case in testgen::all_cases() {
            let bytes = case.file.to_bytes().unwrap();
            let eager = PsdFile::from_bytes(&bytes).unwrap();
            let lazy = PsdFile::from_source(Source::new(bytes)).unwrap();
            for file in [&eager, &lazy] {
                let planes = file.image_data.row_decoders(&file.header);
                if !matches!(file.image_data.compression, Compression::Raw | Compression::Rle) {
                    assert!(planes.is_err(), "{}", case.name);
                    continue;
                }
                let (Ok(whole), Ok(planes)) = (file.decode_merged(), planes) else { continue };
                let h = file.header.height as usize;
                let mut joined = Vec::new();
                for plane in &planes {
                    let mut r = 0;
                    for step in [1, 3, 2, 7].iter().cycle() {
                        if r >= h {
                            break;
                        }
                        let r1 = (r + step).min(h);
                        joined.extend(plane.rows(r, r1).unwrap());
                        r = r1;
                    }
                }
                assert_eq!(joined, whole, "{}", case.name);
                checked += 1;
            }
        }
        assert!(checked > 4, "{checked}");
    }

    #[test]
    fn a_damaged_rle_row_fails_its_band() {
        use crate::Compression;
        use crate::layer::ChannelData;
        // Two rows of 4 bytes: a valid row, then one that ends early.
        let mut data = vec![0, 2, 0, 2];
        data.extend([0xFD, 7]); // header -3: 1 - (-3) = 4 copies of 7
        data.extend([0x01, 9]); // a literal of 2 bytes, only 1 present
        let ch = ChannelData::new(0, Some(Compression::Rle), data);
        let dec = ch.row_decoder(4, 2, 8, crate::Version::Psd).unwrap();
        assert_eq!(dec.rows(0, 1).unwrap(), vec![7; 4]);
        assert!(dec.rows(1, 2).is_err());
        assert!(ch.decode(4, 2, 8, crate::Version::Psd).is_err());
    }
}
