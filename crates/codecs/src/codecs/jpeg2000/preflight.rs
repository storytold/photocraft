//! Check allocation-driving JPEG 2000 headers before entering the decoder.
//!
//! This is a bounded, allocation-free walk of the Annex A marker structure. It
//! does not validate entropy data; the decoder remains responsible for that.
//! The first implementation accepts full-size, unsigned, equally precise
//! components at a zero origin. Tile coding overrides, packed packet headers,
//! progression changes, ROI shifts and Part 2/HT coding are explicitly refused
//! until their separate resource accounting is implemented.

use crate::{CodecError, Format, Limits};

const SOC: u16 = 0xff4f;
const SIZ: u16 = 0xff51;
const COD: u16 = 0xff52;
const COC: u16 = 0xff53;
const SOT: u16 = 0xff90;
const SOD: u16 = 0xff93;
const EOC: u16 = 0xffd9;

// Tier-1 and midpoint reconstruction bound band coefficients by 2^Mb - 1.
// For B = 2^22 - 1, one 2D 5/3 level increases the carried LL bound by
// B + ceil(B/2) + H + ceil(H/2), where H = 2B + ceil(B/2). At 32 levels
// this bounds samples by 708,837,247; RCT and a 16-bit DC shift stay below
// 1,772,125,886. The backend's unchecked i32 lifting sums fit as well.
const MAX_REVERSIBLE_MAGNITUDE_BITS: u8 = 22;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Info {
    pub width: u32,
    pub height: u32,
    pub components: usize,
    pub precision: u8,
}

#[derive(Clone, Copy)]
struct Geometry {
    info: Info,
    tile_width: u32,
    tile_height: u32,
    tiles: u64,
}

#[derive(Clone, Copy)]
struct Coding {
    layers: u16,
    levels: u8,
    block_x: u8,
    block_y: u8,
    split_segments: bool,
    precincts: [u8; 33],
}

struct Reader<'a> {
    data: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    fn peek(&self) -> Result<u16, CodecError> {
        be16(self.data, self.position)
    }

    fn marker(&mut self) -> Result<(u16, usize), CodecError> {
        let start = self.position;
        let marker = self.peek()?;
        if marker & 0xff00 != 0xff00 || marker == 0xffff || marker == 0xff00 {
            return Err(malformed("expected a JPEG 2000 marker"));
        }
        self.position = start.checked_add(2).ok_or_else(overflow)?;
        Ok((marker, start))
    }

    fn payload(&mut self) -> Result<&'a [u8], CodecError> {
        let length = usize::from(be16(self.data, self.position)?);
        if length < 2 {
            return Err(malformed("marker length is smaller than its length field"));
        }
        let start = self.position.checked_add(2).ok_or_else(overflow)?;
        let end = self.position.checked_add(length).ok_or_else(overflow)?;
        let payload = self.data.get(start..end).ok_or_else(|| malformed("truncated marker segment"))?;
        self.position = end;
        Ok(payload)
    }
}

/// The estimate includes pixel planes, conversion/wavelet scratch, packet and
/// code-block bookkeeping, and copied compressed/header data. It is deliberately
/// conservative, rather than a promise about the allocator's exact RSS. Count
/// all tiles' packet work: small tiles and many layers can dominate even when
/// the final image has very few pixels.
struct Budget<'a> {
    limits: &'a Limits,
    bytes: u64,
}

impl<'a> Budget<'a> {
    fn new(limits: &'a Limits) -> Self {
        Self { limits, bytes: 0 }
    }

    fn add(&mut self, count: u64, bytes_each: u64) -> Result<(), CodecError> {
        let bytes = count.checked_mul(bytes_each).ok_or_else(overflow)?;
        self.bytes = self.bytes.checked_add(bytes).ok_or_else(overflow)?;
        // Vec allocations must fit isize too, including with Limits::none().
        if self.bytes > self.limits.max_alloc || self.bytes > isize::MAX as u64 {
            return Err(CodecError::LimitExceeded(format!(
                "JPEG 2000 estimated decode allocation {} bytes exceeds the {} byte budget",
                self.bytes,
                self.limits.max_alloc.min(isize::MAX as u64)
            )));
        }
        Ok(())
    }
}

pub(super) fn check(codestream: &[u8], limits: &Limits) -> Result<Info, CodecError> {
    let mut reader = Reader { data: codestream, position: 0 };
    if reader.marker()?.0 != SOC || reader.marker()?.0 != SIZ {
        return Err(malformed("codestream must begin with SOC and SIZ"));
    }
    let geometry = geometry(reader.payload()?, limits)?;
    let mut budget = Budget::new(limits);
    budget.add(u64::try_from(codestream.len()).map_err(|_| overflow())?, 8)?;
    let samples = u64::from(geometry.info.width)
        .checked_mul(u64::from(geometry.info.height))
        .and_then(|n| n.checked_mul(geometry.info.components as u64))
        .ok_or_else(overflow)?;
    // oxideav 0.0.17 retains 14 bytes/coefficient in CodeBlock arrays while the
    // global i32 image (4) and f64 sub-bands + interleaved tile (16) coexist.
    // Its f64 -> f32 + i32 conversion also reaches 34 bytes/sample. Round up
    // to 48 for sample copies/interleaving; packet/block records are separate.
    budget.add(samples, 48)?;
    // 9/7 filtering also holds two f64 line buffers plus an extended line:
    // 24 * axis + at most 88 bytes. This can dominate a 1xN/Nx1 tile, so it
    // needs a separate allowance. Charge 32 * (largest tile axis + 16) per
    // component, conservatively covering either kernel and component overrides.
    let tile_axis = geometry.tile_width.min(geometry.info.width).max(geometry.tile_height.min(geometry.info.height));
    let line_samples = u64::from(tile_axis).checked_add(16).and_then(|n| n.checked_mul(geometry.info.components as u64)).ok_or_else(overflow)?;
    budget.add(line_samples, 32)?;
    budget.add(geometry.tiles, 1024)?;
    budget.add(1, 256)?;

    let mut default = None;
    let mut overrides = [None; 4];
    while reader.peek()? != SOT {
        let (marker, _) = reader.marker()?;
        if marker == EOC || marker == SOC || marker == SIZ || marker == SOD {
            return Err(malformed("unexpected delimiter in the main header"));
        }
        budget.add(1, 256)?;
        let payload = reader.payload()?;
        match marker {
            COD => {
                if default.is_some() {
                    return Err(malformed("duplicate main-header COD"));
                }
                default = Some(cod(payload, geometry.info.components)?);
            }
            COC => {
                let component = usize::from(byte(payload, 0)?);
                if component >= geometry.info.components {
                    return Err(malformed("COC component is outside SIZ"));
                }
                let slot = overrides.get_mut(component).ok_or_else(|| malformed("invalid COC component"))?;
                if slot.is_some() {
                    return Err(malformed("duplicate main-header COC"));
                }
                *slot = Some(coding(payload, 1, 2, 1, true)?);
            }
            _ => other_marker(marker, payload, false, &mut budget)?,
        }
    }
    let default = default.ok_or_else(|| malformed("missing main-header COD"))?;
    for component in 0..geometry.info.components {
        let mut style = overrides.get(component).copied().flatten().unwrap_or(default);
        // COC changes component coding, but inherits the quality-layer count.
        style.layers = default.layers;
        account_packets(geometry, style, &mut budget)?;
    }

    let mut tile_parts = 0_u64;
    loop {
        let (marker, start) = reader.marker()?;
        if marker == EOC {
            if tile_parts == 0 || reader.position != codestream.len() {
                return Err(malformed("missing tile data or bytes after EOC"));
            }
            return Ok(geometry.info);
        }
        if marker != SOT {
            return Err(malformed("expected SOT or EOC after a tile-part"));
        }
        tile_parts = tile_parts.checked_add(1).ok_or_else(overflow)?;
        budget.add(1, 1024)?;
        let payload = reader.payload()?;
        if payload.len() != 8 {
            return Err(malformed("SOT must have an eight-byte payload"));
        }
        if u64::from(be16(payload, 0)?) >= geometry.tiles {
            return Err(malformed("SOT tile index is outside SIZ"));
        }
        let part = byte(payload, 6)?;
        let total_parts = byte(payload, 7)?;
        if part == 255 {
            return Err(malformed("SOT tile-part index exceeds the Part 1 bounds"));
        }
        if total_parts != 0 && part >= total_parts {
            return Err(malformed("SOT tile-part index is outside its declared count"));
        }
        let psot = be32(payload, 2)?;
        let end = if psot == 0 {
            None
        } else {
            let end = start.checked_add(usize::try_from(psot).map_err(|_| overflow())?).ok_or_else(overflow)?;
            if end > codestream.len() || end < reader.position {
                return Err(malformed("SOT length is outside the codestream"));
            }
            Some(end)
        };
        loop {
            let (marker, _) = reader.marker()?;
            if end.is_some_and(|end| reader.position > end) {
                return Err(malformed("tile header exceeds SOT length"));
            }
            if marker == SOD {
                break;
            }
            if matches!(marker, SOC | SIZ | SOT | EOC) {
                return Err(malformed("unexpected delimiter in a tile header"));
            }
            budget.add(1, 256)?;
            let payload = reader.payload()?;
            if end.is_some_and(|end| reader.position > end) {
                return Err(malformed("tile header exceeds SOT length"));
            }
            other_marker(marker, payload, true, &mut budget)?;
        }
        reader.position = match end {
            Some(end) => end,
            None => entropy_end(codestream, reader.position)?,
        };
    }
}

fn geometry(payload: &[u8], limits: &Limits) -> Result<Geometry, CodecError> {
    let capabilities = be16(payload, 0)?;
    let x_size = be32(payload, 2)?;
    let y_size = be32(payload, 6)?;
    let x_origin = be32(payload, 10)?;
    let y_origin = be32(payload, 14)?;
    let tile_width = be32(payload, 18)?;
    let tile_height = be32(payload, 22)?;
    let tile_x = be32(payload, 26)?;
    let tile_y = be32(payload, 30)?;
    let components = usize::from(be16(payload, 34)?);
    if !(1..=4).contains(&components) {
        return Err(unsupported("only one to four image components are supported"));
    }
    if payload.len() != 36 + components * 3 {
        return Err(malformed("SIZ component table has the wrong length"));
    }
    let width = x_size.checked_sub(x_origin).filter(|n| *n != 0).ok_or_else(|| malformed("invalid SIZ width"))?;
    let height = y_size.checked_sub(y_origin).filter(|n| *n != 0).ok_or_else(|| malformed("invalid SIZ height"))?;
    let precision = (byte(payload, 36)? & 0x7f) + 1;
    limits.check_bytes(width, height, components as u64 * if precision <= 8 { 1 } else { 2 })?;
    if capabilities & 0xc000 != 0 {
        return Err(unsupported("Part 2 and HTJ2K codestreams are not supported"));
    }
    if x_origin != 0 || y_origin != 0 || tile_x != 0 || tile_y != 0 {
        return Err(unsupported("nonzero image or tile origins are not supported"));
    }
    if tile_width == 0 || tile_height == 0 {
        return Err(malformed("zero-sized SIZ tile"));
    }
    for component in 0..components {
        let position = 36 + component * 3;
        let descriptor = byte(payload, position)?;
        let bits = (descriptor & 0x7f) + 1;
        if descriptor & 0x80 != 0 || bits > 16 {
            return Err(unsupported("only unsigned one to sixteen-bit samples are supported"));
        }
        if bits != precision {
            return Err(unsupported("mixed component precisions are not supported"));
        }
        if byte(payload, position + 1)? != 1 || byte(payload, position + 2)? != 1 {
            return Err(unsupported("subsampled components are not supported"));
        }
    }
    let tiles = u64::from(width).div_ceil(u64::from(tile_width)).checked_mul(u64::from(height).div_ceil(u64::from(tile_height))).ok_or_else(overflow)?;
    // Annex A's Isot field cannot address a larger tile grid. Bound this before
    // the backend builds per-tile structures, even with unlimited user limits.
    if tiles > 65_535 {
        return Err(CodecError::LimitExceeded("JPEG 2000 tile grid exceeds 65,535 tiles".into()));
    }
    Ok(Geometry { info: Info { width, height, components, precision }, tile_width, tile_height, tiles })
}

fn cod(payload: &[u8], components: usize) -> Result<Coding, CodecError> {
    let progression = byte(payload, 1)?;
    if progression > 4 {
        return Err(unsupported("unknown packet progression order"));
    }
    let layers = be16(payload, 2)?;
    if layers == 0 {
        return Err(malformed("COD has zero quality layers"));
    }
    match byte(payload, 4)? {
        0 => {}
        1 if components >= 3 => {}
        _ => return Err(unsupported("unsupported multi-component transform")),
    }
    coding(payload, 0, 5, layers, false)
}

fn coding(payload: &[u8], flag_position: usize, position: usize, layers: u16, component: bool) -> Result<Coding, CodecError> {
    let flags = byte(payload, flag_position)?;
    let allowed = if component { 1 } else { 7 };
    if flags & !allowed != 0 {
        return Err(unsupported("unsupported coding-style flags"));
    }
    let levels = byte(payload, position)?;
    if levels > 32 {
        return Err(malformed("more than 32 wavelet decompositions"));
    }
    let block_x = byte(payload, position + 1)?.checked_add(2).ok_or_else(|| malformed("invalid code-block width"))?;
    let block_y = byte(payload, position + 2)?.checked_add(2).ok_or_else(|| malformed("invalid code-block height"))?;
    if block_x > 10 || block_y > 10 || u16::from(block_x) + u16::from(block_y) > 12 {
        return Err(malformed("code-block dimensions exceed the Part 1 bounds"));
    }
    let block_style = byte(payload, position + 3)?;
    if block_style & 0xc0 != 0 {
        return Err(unsupported("HTJ2K and reserved code-block styles are not supported"));
    }
    if byte(payload, position + 4)? > 1 {
        return Err(unsupported("only the 5/3 and 9/7 wavelets are supported"));
    }
    let has_precincts = flags & 1 != 0;
    let precinct_count = if has_precincts { usize::from(levels) + 1 } else { 0 };
    let start = position.checked_add(5).ok_or_else(overflow)?;
    if payload.len() != start.checked_add(precinct_count).ok_or_else(overflow)? {
        return Err(malformed("COD/COC precinct table has the wrong length"));
    }
    let mut precincts = [0xff; 33];
    if has_precincts {
        for (resolution, (slot, value)) in precincts.iter_mut().zip(payload.get(start..).ok_or_else(|| malformed("missing precinct table"))?).enumerate() {
            if resolution > 0 && (value & 15 == 0 || value >> 4 == 0) {
                return Err(malformed("zero precinct exponent above resolution zero"));
            }
            *slot = *value;
        }
    }
    Ok(Coding { layers, levels, block_x, block_y, split_segments: block_style & 0x05 != 0, precincts })
}

fn account_packets(geometry: Geometry, coding: Coding, budget: &mut Budget<'_>) -> Result<(), CodecError> {
    let width = u64::from(geometry.tile_width.min(geometry.info.width));
    let height = u64::from(geometry.tile_height.min(geometry.info.height));
    // A later tile's reduced-resolution corners may straddle one extra grid
    // cell. The single-tile zero-origin case needs no such padding.
    let padding = u64::from(geometry.tiles > 1);
    for resolution in 0..=coding.levels {
        let shift = u32::from(coding.levels - resolution);
        let divisor = 1_u64.checked_shl(shift).ok_or_else(overflow)?;
        let level_width = width.div_ceil(divisor).checked_add(padding).ok_or_else(overflow)?;
        let level_height = height.div_ceil(divisor).checked_add(padding).ok_or_else(overflow)?;
        let precinct = *coding.precincts.get(usize::from(resolution)).ok_or_else(|| malformed("missing precinct exponent"))?;
        let ppx = precinct & 15;
        let ppy = precinct >> 4;
        let nx = level_width.div_ceil(1_u64 << ppx).checked_add(padding).ok_or_else(overflow)?;
        let ny = level_height.div_ceil(1_u64 << ppy).checked_add(padding).ok_or_else(overflow)?;
        let precincts = nx.checked_mul(ny).and_then(|n| n.checked_mul(geometry.tiles)).ok_or_else(overflow)?;
        let packets = precincts.checked_mul(u64::from(coding.layers)).ok_or_else(overflow)?;
        // Precinct geometry/state maps exist even for empty sub-bands. Packet
        // plans also own their small sub-band vectors and retained headers.
        budget.add(precincts, 512)?;
        budget.add(packets, 256)?;

        let (band_width, band_height, bands, max_x, max_y) = if resolution == 0 {
            (level_width, level_height, 1_u64, ppx, ppy)
        } else {
            (
                level_width.div_ceil(2).checked_add(padding).ok_or_else(overflow)?,
                level_height.div_ceil(2).checked_add(padding).ok_or_else(overflow)?,
                3,
                ppx.checked_sub(1).ok_or_else(|| malformed("invalid precinct width"))?,
                ppy.checked_sub(1).ok_or_else(|| malformed("invalid precinct height"))?,
            )
        };
        // Each precinct boundary may split a code-block grid. Counting these
        // splits avoids underestimating work for precincts smaller than blocks.
        let bx = band_width
            .div_ceil(1_u64 << coding.block_x.min(max_x))
            .checked_add(nx.saturating_sub(1))
            .and_then(|n| n.checked_add(padding))
            .ok_or_else(overflow)?;
        let by = band_height
            .div_ceil(1_u64 << coding.block_y.min(max_y))
            .checked_add(ny.saturating_sub(1))
            .and_then(|n| n.checked_add(padding))
            .ok_or_else(overflow)?;
        let blocks = bx.checked_mul(by).and_then(|n| n.checked_mul(bands)).and_then(|n| n.checked_mul(geometry.tiles)).ok_or_else(overflow)?;
        budget.add(blocks, 512)?;
        let contributions = blocks.checked_mul(u64::from(coding.layers)).ok_or_else(overflow)?;
        // Contribution vectors grow geometrically and may reserve at least
        // four entries even when a packet refers to only one code-block.
        budget.add(contributions, 512)?;
        if coding.split_segments {
            // Annex B permits up to 164 coding passes in each contribution.
            // BYPASS / TERMALL can retain that many separate segment records,
            // including zero-byte segments: compressed byte length alone does
            // not bound these Vecs. Include growth and temporary span vectors.
            budget.add(contributions.checked_mul(164).ok_or_else(overflow)?, 128)?;
        }
    }
    Ok(())
}

fn other_marker(marker: u16, payload: &[u8], tile: bool, budget: &mut Budget<'_>) -> Result<(), CodecError> {
    match marker {
        COD | COC if tile => Err(unsupported("tile coding overrides are not supported")),
        0xff50 | 0xff5f | 0xff60 | 0xff61 => Err(unsupported("capability extensions, progression changes and packed packet headers are not supported")),
        0xff5e => {
            if payload.len() != 3 || byte(payload, 1)? != 0 {
                return Err(malformed("invalid RGN marker"));
            }
            if byte(payload, 2)? != 0 {
                return Err(unsupported("ROI shifts are not supported"));
            }
            Ok(())
        }
        0xff5c => quantization(payload, false), // QCD.
        0xff5d => quantization(payload, true),  // QCC.
        0xff64 => Ok(()),                       // COM.
        // One-byte packet lengths can expand to retained u64 entries and
        // temporary concatenation tables. TLM has similar copied bookkeeping.
        0xff58 if tile => budget.add(u64::try_from(payload.len()).map_err(|_| overflow())?, 32),
        0xff55 | 0xff57 if !tile => budget.add(u64::try_from(payload.len()).map_err(|_| overflow())?, 32),
        0xff63 if !tile => Ok(()), // CRG.
        _ => Err(unsupported(format!("header marker {marker:#06x} is not supported"))),
    }
}

fn quantization(payload: &[u8], component: bool) -> Result<(), CodecError> {
    // SIZ is limited to four components, so QCC has a one-byte component index.
    let position = usize::from(component);
    let sq = byte(payload, position)?;
    if sq & 0x1f != 0 {
        // Scalar-derived/expounded quantization uses the floating-point 9/7
        // path. The backend validates the style and its pairing with COD/COC.
        return Ok(());
    }
    let entries = payload.get(position + 1..).filter(|entries| !entries.is_empty()).ok_or_else(|| malformed("missing reversible quantization entries"))?;
    let guard = sq >> 5;
    for &entry in entries {
        let bits = guard.checked_add(entry >> 3).and_then(|sum| sum.checked_sub(1)).ok_or_else(|| malformed("invalid reversible magnitude bit count"))?;
        if bits > MAX_REVERSIBLE_MAGNITUDE_BITS {
            return Err(unsupported("reversible quantization exceeds 22 magnitude bits"));
        }
    }
    // Check every main/tile table, including overridden ones. This bounds every
    // effective band without duplicating QCD/QCC precedence in the marker walk.
    Ok(())
}

fn entropy_end(data: &[u8], mut position: usize) -> Result<usize, CodecError> {
    while let Some(pair) = data.get(position..).and_then(|rest| rest.get(..2)) {
        if pair == [0xff, 0x90] || pair == [0xff, 0xd9] {
            return Ok(position);
        }
        position = position.checked_add(1).ok_or_else(overflow)?;
    }
    Err(malformed("tile-part with zero Psot has no closing marker"))
}

fn byte(data: &[u8], position: usize) -> Result<u8, CodecError> {
    data.get(position).copied().ok_or_else(|| malformed("truncated marker fields"))
}

fn be16(data: &[u8], position: usize) -> Result<u16, CodecError> {
    let end = position.checked_add(2).ok_or_else(overflow)?;
    let bytes: [u8; 2] =
        data.get(position..end).ok_or_else(|| malformed("truncated 16-bit field"))?.try_into().map_err(|_| malformed("invalid 16-bit field"))?;
    Ok(u16::from_be_bytes(bytes))
}

fn be32(data: &[u8], position: usize) -> Result<u32, CodecError> {
    let end = position.checked_add(4).ok_or_else(overflow)?;
    let bytes: [u8; 4] =
        data.get(position..end).ok_or_else(|| malformed("truncated 32-bit field"))?.try_into().map_err(|_| malformed("invalid 32-bit field"))?;
    Ok(u32::from_be_bytes(bytes))
}

fn malformed(message: impl std::fmt::Display) -> CodecError {
    CodecError::malformed(Format::Jpeg2000, message)
}

fn unsupported(reason: impl Into<String>) -> CodecError {
    CodecError::unsupported(Format::Jpeg2000, reason)
}

fn overflow() -> CodecError {
    CodecError::LimitExceeded("JPEG 2000 decode allocation does not fit in memory".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(out: &mut Vec<u8>, marker: u16, body: &[u8]) {
        out.extend_from_slice(&marker.to_be_bytes());
        out.extend_from_slice(&u16::try_from(body.len() + 2).unwrap().to_be_bytes());
        out.extend_from_slice(body);
    }

    #[allow(clippy::too_many_arguments)]
    fn header(width: u32, height: u32, components: usize, precision: u8, tile: (u32, u32), layers: u16, levels: u8, precincts: &[u8]) -> Vec<u8> {
        let mut out = SOC.to_be_bytes().to_vec();
        let mut siz = 0_u16.to_be_bytes().to_vec();
        for value in [width, height, 0, 0, tile.0, tile.1, 0, 0] {
            siz.extend_from_slice(&value.to_be_bytes());
        }
        siz.extend_from_slice(&(components as u16).to_be_bytes());
        for _ in 0..components {
            siz.extend_from_slice(&[precision - 1, 1, 1]);
        }
        segment(&mut out, SIZ, &siz);
        let mut cod = vec![u8::from(!precincts.is_empty()), 0];
        cod.extend_from_slice(&layers.to_be_bytes());
        cod.extend_from_slice(&[u8::from(components >= 3), levels, 4, 4, 0, 1]);
        cod.extend_from_slice(precincts);
        segment(&mut out, COD, &cod);
        segment(&mut out, 0xff5c, &[0x40, 0x40]);
        out
    }

    fn tile(out: &mut Vec<u8>, index: u16, psot_zero: bool, extra: &[u8]) {
        let mut sot = index.to_be_bytes().to_vec();
        let psot = if psot_zero { 0 } else { 15 + extra.len() as u32 };
        sot.extend_from_slice(&psot.to_be_bytes());
        sot.extend_from_slice(&[0, 1]);
        segment(out, SOT, &sot);
        out.extend_from_slice(extra);
        out.extend_from_slice(&SOD.to_be_bytes());
        out.push(0);
    }

    fn finish(mut out: Vec<u8>) -> Vec<u8> {
        tile(&mut out, 0, false, &[]);
        out.extend_from_slice(&EOC.to_be_bytes());
        out
    }

    fn ordinary() -> Vec<u8> {
        finish(header(32, 24, 3, 8, (32, 24), 1, 2, &[]))
    }

    #[test]
    fn accepts_baseline_precisions_wavelets_and_progressions() {
        for precision in [1, 8, 12, 16] {
            for components in 1..=4 {
                let image = finish(header(32, 24, components, precision, (32, 24), 2, 2, &[0x44; 3]));
                assert_eq!(check(&image, &Limits::default()).unwrap(), Info { width: 32, height: 24, components, precision });
            }
        }
        for progression in 0..=4 {
            for kernel in 0..=1 {
                let mut image = ordinary();
                let cod = 2 + 2 + 2 + 36 + 3 * 3;
                image[cod + 4 + 1] = progression;
                image[cod + 4 + 9] = kernel;
                assert!(check(&image, &Limits::default()).is_ok());
            }
        }
    }

    #[test]
    fn accepts_multiple_tiles_and_zero_psot() {
        let mut image = header(17, 13, 1, 16, (9, 7), 1, 1, &[]);
        for index in 0..4 {
            tile(&mut image, index, index == 3, &[]);
        }
        image.extend_from_slice(&EOC.to_be_bytes());
        assert_eq!(check(&image, &Limits::default()).unwrap().width, 17);
    }

    #[test]
    fn bounds_every_reversible_main_and_tile_quantization_table() {
        for marker in [0xff5c, 0xff5d] {
            for in_tile in [false, true] {
                for (guard, epsilon, supported) in [(2, 21, true), (2, 22, false), (7, 16, true), (7, 17, false)] {
                    let mut image = header(8, 8, 3, 16, (8, 8), 1, 1, &[]);
                    image.truncate(image.len() - 6); // Replace the helper's QCD.
                    let mut table = vec![guard << 5, 8 << 3, 8 << 3, 8 << 3, epsilon << 3];
                    if marker == 0xff5d {
                        table.insert(0, 0); // QCC component zero.
                    }
                    if marker == 0xff5c && !in_tile {
                        segment(&mut image, marker, &table);
                    } else {
                        segment(&mut image, 0xff5c, &[0x40, 16 << 3, 17 << 3, 17 << 3, 18 << 3]);
                        if !in_tile {
                            segment(&mut image, marker, &table);
                        }
                    }
                    let image = if in_tile {
                        let mut extra = Vec::new();
                        segment(&mut extra, marker, &table);
                        tile(&mut image, 0, false, &extra);
                        image.extend_from_slice(&EOC.to_be_bytes());
                        image
                    } else {
                        finish(image)
                    };
                    let result = check(&image, &Limits::default());
                    if supported {
                        assert!(result.is_ok(), "marker={marker:#06x}, tile={in_tile}, G={guard}, epsilon={epsilon}: {result:?}");
                    } else {
                        assert!(
                            matches!(result, Err(CodecError::Unsupported { .. })),
                            "marker={marker:#06x}, tile={in_tile}, G={guard}, epsilon={epsilon}: {result:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn accepts_native_sixteen_bit_rct_tables_at_the_wavelet_level_limit() {
        let mut image = header(1, 1, 3, 16, (1, 1), 1, 32, &[]);
        image.truncate(image.len() - 6);
        let mut luma = vec![0x40, 16 << 3];
        let mut chroma = vec![0x40, 17 << 3];
        for _ in 0..32 {
            luma.extend_from_slice(&[17 << 3, 17 << 3, 18 << 3]);
            chroma.extend_from_slice(&[18 << 3, 18 << 3, 19 << 3]);
        }
        segment(&mut image, 0xff5c, &luma);
        for component in [1, 2] {
            let mut table = vec![component];
            table.extend_from_slice(&chroma);
            segment(&mut image, 0xff5d, &table);
        }
        assert!(check(&finish(image), &Limits::default()).is_ok());
    }

    #[test]
    fn leaves_irreversible_quantization_headroom_unchanged() {
        let mut image = header(1, 1, 1, 16, (1, 1), 1, 0, &[]);
        image.truncate(image.len() - 6);
        let cod = image.windows(2).position(|bytes| bytes == COD.to_be_bytes()).unwrap();
        image[cod + 13] = 0; // Irreversible 9/7 kernel.
        // The native 16-bit lossy encoder's additional fine bits can exceed
        // the reversible cap; scalar quantization must retain that headroom.
        for style in [1, 2] {
            let mut candidate = image.clone();
            let step = (25_u16 << 11).to_be_bytes();
            segment(&mut candidate, 0xff5c, &[0x40 | style, step[0], step[1]]);
            assert!(check(&finish(candidate), &Limits::default()).is_ok());
        }
    }

    #[test]
    fn rejects_missing_and_underflowing_reversible_quantization_entries() {
        for payload in [&[][..], &[0][..], &[0, 0][..]] {
            assert!(matches!(quantization(payload, false), Err(CodecError::Malformed { .. })));
        }
        for payload in [&[0][..], &[0, 0][..], &[0, 0, 0][..]] {
            assert!(matches!(quantization(payload, true), Err(CodecError::Malformed { .. })));
        }
    }

    #[test]
    fn rejects_sample_models_explicitly() {
        for position in [42, 43, 45] {
            let mut image = ordinary();
            image[position] = match position {
                42 => 0x87, // Signed eight-bit component.
                43 => 2,    // Subsampled component.
                _ => 15,    // A sixteen-bit component beside eight-bit ones.
            };
            assert!(matches!(check(&image, &Limits::default()), Err(CodecError::Unsupported { .. })));
        }
    }

    #[test]
    fn catches_geometry_and_packet_bombs_before_decoding() {
        let geometry_bomb = finish(header(u32::MAX, u32::MAX, 4, 16, (u32::MAX, u32::MAX), 1, 0, &[]));
        assert!(matches!(check(&geometry_bomb, &Limits::none()), Err(CodecError::LimitExceeded(_))));
        // Dimensions are checked immediately after SIZ, before any requirement
        // for a complete main header can disguise a declared allocation bomb.
        assert!(matches!(check(&geometry_bomb[..54], &Limits::none()), Err(CodecError::LimitExceeded(_))));
        let layer_bomb = finish(header(1, 1, 1, 8, (1, 1), u16::MAX, 0, &[]));
        assert!(matches!(check(&layer_bomb, &Limits { max_alloc: 16_384, ..Limits::none() }), Err(CodecError::LimitExceeded(_))));
        let precinct_bomb = finish(header(4096, 4096, 1, 8, (4096, 4096), 1, 0, &[0]));
        assert!(matches!(check(&precinct_bomb, &Limits::default()), Err(CodecError::LimitExceeded(_))));
        let tiles_bomb = finish(header(256, 256, 1, 8, (1, 1), 1, 0, &[]));
        assert!(matches!(check(&tiles_bomb, &Limits::none()), Err(CodecError::LimitExceeded(_))));
    }

    #[test]
    fn accounts_main_component_overrides() {
        let mut image = header(16, 16, 1, 8, (16, 16), 1, 0, &[]);
        let limits = Limits { max_alloc: 100_000, ..Limits::none() };
        assert!(check(&finish(image.clone()), &limits).is_ok());
        segment(&mut image, COC, &[0, 1, 0, 4, 4, 0, 1, 0]);
        assert!(matches!(check(&finish(image), &limits), Err(CodecError::LimitExceeded(_))));
    }

    #[test]
    fn tiny_budgets_report_limits() {
        assert!(matches!(check(&ordinary(), &Limits { max_alloc: 1, ..Limits::none() }), Err(CodecError::LimitExceeded(_))));
    }

    #[test]
    fn sample_budget_covers_retained_coefficients_and_float_conversion() {
        for kernel in [0, 1] {
            let mut image = finish(header(256, 256, 1, 16, (256, 256), 1, 1, &[]));
            let cod = image.windows(2).position(|v| v == COD.to_be_bytes()).unwrap();
            image[cod + 13] = kernel;
            // The former 32-byte charge accepted a cap below the 9/7 lane's
            // 34-byte live arrays. Keep one conservative guard for both kernels.
            let tight = Limits { max_alloc: 33 * 256 * 256, ..Limits::none() };
            assert!(matches!(check(&image, &tight), Err(CodecError::LimitExceeded(_))));
            let roomy = Limits { max_alloc: 64 * 256 * 256, ..Limits::none() };
            assert!(check(&image, &roomy).is_ok());
        }
    }

    #[test]
    fn skinny_wavelet_tiles_reserve_line_scratch_in_addition_to_samples() {
        for (width, height, block) in [(1, 4096, (0, 8)), (4096, 1, (8, 0))] {
            let mut image = finish(header(width, height, 1, 16, (width, height), 1, 1, &[]));
            let cod = image.windows(2).position(|v| v == COD.to_be_bytes()).unwrap();
            // Legal 4x1024 / 1024x4 blocks keep record overhead small enough
            // that it cannot accidentally stand in for the missing line buffers.
            image[cod + 10] = block.0;
            image[cod + 11] = block.1;
            image[cod + 13] = 0;
            let tight = Limits { max_alloc: 72 * 4096, ..Limits::none() };
            assert!(matches!(check(&image, &tight), Err(CodecError::LimitExceeded(_))));
            let roomy = Limits { max_alloc: 128 * 4096, ..Limits::none() };
            assert!(check(&image, &roomy).is_ok());
        }
    }

    #[test]
    fn accounts_split_codeword_segments_even_when_their_bytes_are_zero() {
        let limits = Limits { max_alloc: 8_192, ..Limits::none() };
        let base = header(1, 1, 1, 8, (1, 1), 1, 0, &[]);
        assert!(check(&finish(base.clone()), &limits).is_ok());
        let cod = 2 + 4 + 36 + 3;
        for style in [0x01, 0x04, 0x05] {
            let mut image = base.clone();
            image[cod + 4 + 8] = style;
            assert!(matches!(check(&finish(image), &limits), Err(CodecError::LimitExceeded(_))));
            let mut image = base.clone();
            segment(&mut image, COC, &[0, 0, 0, 4, 4, style, 1]);
            assert!(matches!(check(&finish(image), &limits), Err(CodecError::LimitExceeded(_))));
        }
        // Context reset, vertical causality, predictable termination and
        // segmentation symbols do not introduce per-pass segment vectors.
        let mut image = base;
        image[cod + 4 + 8] = 0x3a;
        assert!(check(&finish(image), &limits).is_ok());
    }

    #[test]
    fn accounts_expanded_packet_length_tables_before_the_backend_parses_them() {
        let limits = Limits { max_alloc: 60_000, ..Limits::none() };
        let header = header(1, 1, 1, 8, (1, 1), 1, 0, &[]);
        // A similarly sized opaque comment only needs the compressed-data
        // allowance; thousands of one-byte PLT entries expand to u64 tables.
        for (marker, exceeds) in [(0xff64, false), (0xff58, true)] {
            let mut extra = Vec::new();
            segment(&mut extra, marker, &vec![0; 4_097]);
            let mut image = header.clone();
            tile(&mut image, 0, false, &extra);
            image.extend_from_slice(&EOC.to_be_bytes());
            assert_eq!(matches!(check(&image, &limits), Err(CodecError::LimitExceeded(_))), exceeds);
        }
    }

    #[test]
    fn rejects_duplicate_coding_headers_and_reserved_tile_part_indices() {
        let base = header(1, 1, 1, 8, (1, 1), 1, 0, &[]);
        let mut image = base.clone();
        segment(&mut image, COD, &[0, 0, 0, 1, 0, 0, 4, 4, 0, 1]);
        assert!(matches!(check(&finish(image), &Limits::default()), Err(CodecError::Malformed { .. })));
        let mut image = base.clone();
        for _ in 0..2 {
            segment(&mut image, COC, &[0, 0, 0, 4, 4, 0, 1]);
        }
        assert!(matches!(check(&finish(image), &Limits::default()), Err(CodecError::Malformed { .. })));
        let sot = base.len();
        let mut image = finish(base);
        image[sot + 10] = 255;
        image[sot + 11] = 0;
        assert!(matches!(check(&image, &Limits::default()), Err(CodecError::Malformed { .. })));
    }

    #[test]
    fn refuses_features_without_resource_accounting() {
        for marker in [COD, COC, 0xff50, 0xff5f, 0xff61] {
            let mut image = header(8, 8, 1, 8, (8, 8), 1, 0, &[]);
            let mut extra = Vec::new();
            segment(&mut extra, marker, &[0; 10]);
            tile(&mut image, 0, false, &extra);
            image.extend_from_slice(&EOC.to_be_bytes());
            assert!(matches!(check(&image, &Limits::default()), Err(CodecError::Unsupported { .. })));
        }
    }

    #[test]
    fn malformed_marker_walk_never_panics() {
        let image = ordinary();
        for length in 0..image.len() {
            assert!(check(&image[..length], &Limits::default()).is_err());
        }
        for index in 0..image.len() {
            let mut changed = image.clone();
            changed[index] ^= 0xff;
            let _ = check(&changed, &Limits::default());
        }
    }
}
