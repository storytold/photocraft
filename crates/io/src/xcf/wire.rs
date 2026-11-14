//! Bounds-checked XCF framing, properties and tile compression.

use super::{HashMap, MAX_PAYLOAD, MAX_STRING, Read, Result, invalid};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Compression {
    None,
    Rle,
    Zlib,
}

/// Big-endian reads with bounds checks; every failure is an error, never a panic.
pub(super) struct Cursor<'a> {
    pub(super) b: &'a [u8],
    pub(super) pos: usize,
    /// XCF 11 and later store pointers as 64-bit.
    pub(super) wide: bool,
}

impl<'a> Cursor<'a> {
    pub(super) fn at(&self, pos: usize) -> Cursor<'a> {
        Cursor { b: self.b, pos, wide: self.wide }
    }

    pub(super) fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).ok_or_else(|| invalid("offset overflow"))?;
        let s = self.b.get(self.pos..end).ok_or_else(|| invalid(format!("the file ends early (wanted {n} bytes at {})", self.pos)))?;
        self.pos = end;
        Ok(s)
    }

    pub(super) fn u32(&mut self) -> Result<u32> {
        let s = self.take(4)?;
        Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    }

    pub(super) fn ptr(&mut self) -> Result<usize> {
        if self.wide {
            let s = self.take(8)?;
            usize::try_from(u64::from_be_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]])).map_err(|_| invalid("pointer does not fit in memory"))
        } else {
            self.u32().map(|v| v as usize)
        }
    }

    /// A STRING: `n+1` then `n` bytes and a NUL (0 for the empty string).
    pub(super) fn string(&mut self) -> Result<String> {
        Ok(String::from_utf8_lossy(self.string_bytes()?).into_owned())
    }

    /// The same bounded STRING read without allocating or decoding an unused field.
    pub(super) fn string_bytes(&mut self) -> Result<&'a [u8]> {
        let n = self.u32()? as usize;
        if n > MAX_STRING {
            return Err(invalid(format!("a string of {n} bytes exceeds the limit")));
        }
        let s = self.take(n)?;
        Ok(s.strip_suffix(&[0]).unwrap_or(s))
    }

    pub(super) fn skip(&mut self, n: usize) -> Result<()> {
        self.take(n).map(|_| ())
    }
}

/// One property record: its type and payload.
pub(super) struct Prop<'a> {
    pub(super) offset: usize,
    pub(super) kind: u32,
    pub(super) data: &'a [u8],
}

/// Read a drawable/path property list, using the declared framing for image-only properties.
pub(super) fn props<'a>(c: &mut Cursor<'a>) -> Result<Vec<Prop<'a>>> {
    props_version(c, None)
}

/// Image properties can carry historical lengths that GIMP itself ignores. Resolve their
/// documented framing using the actual file epoch rather than probing payload bytes as headers.
pub(super) fn props_version<'a>(c: &mut Cursor<'a>, version: Option<u32>) -> Result<Vec<Prop<'a>>> {
    props_version_with_interrupt(c, version, &photocraft_raster::Interrupt::NONE)
}

/// The same bounded property framing, with cancellation between records for header-only callers.
pub(super) fn props_version_with_interrupt<'a>(c: &mut Cursor<'a>, version: Option<u32>, ctl: &photocraft_raster::Interrupt<'_>) -> Result<Vec<Prop<'a>>> {
    let mut out = Vec::new();
    loop {
        ctl.check().map_err(|_| super::IoError::Cancelled)?;
        let offset = c.pos;
        let kind = c.u32()?;
        let len = c.u32()? as usize;
        if kind == 0 {
            return Ok(out);
        }
        // PROP_COLORMAP's length word was wrong in some releases (n+4 for 3n+4 bytes); its real
        // size follows from the count. Unit strings changed at XCF21.
        // PROP_LINK_LAYER's length can be four bytes short (#8857); read all its fields.
        let len = match kind {
            1 => {
                let n = c.at(c.pos).u32()? as usize;
                4usize.checked_add(n.checked_mul(3).ok_or_else(|| invalid("colormap overflow"))?).ok_or_else(|| invalid("colormap overflow"))?
            }
            24 => {
                if let Some(version) = version {
                    user_unit_len(c, version)?
                } else {
                    len
                }
            }
            48 => link_layer_len(c)?,
            _ => len,
        };
        if len > MAX_PAYLOAD {
            return Err(invalid(format!("property {kind} declares {len} bytes")));
        }
        let data = c.take(len)?;
        out.try_reserve(1).map_err(|_| invalid("not enough memory for XCF properties"))?;
        out.push(Prop { offset, kind, data });
        if out.len() > 4096 {
            return Err(invalid("too many properties"));
        }
    }
}

/// A PROP_LINK_LAYER payload: flags, a path STRING, then the source width and height.
/// The dimensions are present for raster links too, where their values are unused.
pub(super) fn link_layer_len(c: &Cursor<'_>) -> Result<usize> {
    let mut p = c.at(c.pos);
    p.skip(4)?;
    p.string_bytes()?;
    p.skip(8)?;
    Ok(p.pos - c.pos)
}

/// The real size of a PROP_USER_UNIT payload: a float, a count and three (XCF 21+) or five
/// strings. Measured without consuming.
pub(super) fn user_unit_len(c: &Cursor<'_>, version: u32) -> Result<usize> {
    let mut p = c.at(c.pos);
    p.skip(8)?;
    for _ in 0..if version < 21 { 5 } else { 3 } {
        p.string()?;
    }
    Ok(p.pos - c.pos)
}

pub(super) fn be_u32(b: &[u8]) -> Option<u32> {
    b.get(..4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

pub(super) fn be_i32(b: &[u8]) -> Option<i32> {
    be_u32(b).map(|v| v as i32)
}

pub(super) fn be_f32(b: &[u8]) -> Option<f32> {
    be_u32(b).map(f32::from_bits)
}

/// The parasites of a PROP_PARASITES payload as (name, data); a later duplicate wins.
pub(super) fn parasites(data: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut c = Cursor { b: data, pos: 0, wide: false };
    let mut out: Vec<(String, Vec<u8>)> = Vec::new();
    // Map each name to its output slot so duplicate replacement is O(1). The parasite
    // records are file-controlled, so scanning `out` with iter_mut().find() per record is
    // O(n^2) and lets a crafted payload with many distinct names stall import. The map
    // preserves insertion order and last-value-wins behavior.
    let mut index: HashMap<String, usize> = HashMap::new();
    while c.pos < data.len() {
        let Ok(name) = c.string() else { break };
        let (Ok(_flags), Ok(len)) = (c.u32(), c.u32()) else { break };
        let Ok(payload) = c.take(len as usize) else { break };
        match index.get(&name) {
            Some(&i) => out[i].1 = payload.to_vec(),
            None => {
                index.insert(name.clone(), out.len());
                out.push((name, payload.to_vec()));
            }
        }
    }
    out
}

/// What one tile's bytes decode to, for a drawable of `bpp` bytes per pixel.
pub(super) fn decode_tile(c: &Cursor<'_>, at: usize, end: usize, compression: Compression, bpp: usize, pixels: usize) -> Result<Vec<u8>> {
    let expected = pixels.checked_mul(bpp).ok_or_else(|| invalid("tile size overflow"))?;
    let input = c.b.get(at..end.min(c.b.len())).ok_or_else(|| invalid("a tile pointer is outside the file"))?;
    match compression {
        Compression::None => input.get(..expected).map(<[u8]>::to_vec).ok_or_else(|| invalid("an uncompressed tile is cut short")),
        Compression::Zlib => {
            let mut out = Vec::with_capacity(expected);
            // Read one byte beyond the expected tile: stopping exactly at its pixels would
            // skip the decoder's trailer/checksum check and accept truncation or excess data.
            flate2::read::ZlibDecoder::new(input).take(expected as u64 + 1).read_to_end(&mut out).map_err(|e| invalid(format!("zlib tile: {e}")))?;
            if out.len() != expected {
                return Err(invalid("a zlib tile has an invalid decoded size"));
            }
            Ok(out)
        }
        Compression::Rle => rle_decode(input, bpp, pixels),
    }
}

/// RLE tile data: one run-length stream per byte position of the pixel, each of `pixels` bytes.
pub(super) fn rle_decode(input: &[u8], bpp: usize, pixels: usize) -> Result<Vec<u8>> {
    let mut out = vec![0u8; pixels * bpp];
    let mut pos = 0usize;
    let mut plane = Vec::with_capacity(pixels);
    for b in 0..bpp {
        plane.clear();
        while plane.len() < pixels {
            let op = *input.get(pos).ok_or_else(|| invalid("an RLE tile is cut short"))?;
            pos += 1;
            let (run, verbatim) = match op {
                0..=126 => (usize::from(op) + 1, false),
                127 => {
                    let (p, q) = (input.get(pos).copied(), input.get(pos + 1).copied());
                    pos += 2;
                    (
                        usize::from(p.ok_or_else(|| invalid("an RLE tile is cut short"))?) * 256
                            + usize::from(q.ok_or_else(|| invalid("an RLE tile is cut short"))?),
                        false,
                    )
                }
                128 => {
                    let (p, q) = (input.get(pos).copied(), input.get(pos + 1).copied());
                    pos += 2;
                    (
                        usize::from(p.ok_or_else(|| invalid("an RLE tile is cut short"))?) * 256
                            + usize::from(q.ok_or_else(|| invalid("an RLE tile is cut short"))?),
                        true,
                    )
                }
                _ => (256 - usize::from(op), true),
            };
            if run == 0 {
                return Err(invalid("an RLE run has zero length"));
            }
            if plane.len() + run > pixels {
                return Err(invalid("an RLE run crosses the end of its tile"));
            }
            if verbatim {
                let data = input.get(pos..pos + run).ok_or_else(|| invalid("an RLE tile is cut short"))?;
                pos += run;
                plane.extend_from_slice(data);
            } else {
                let v = *input.get(pos).ok_or_else(|| invalid("an RLE tile is cut short"))?;
                pos += 1;
                plane.resize(plane.len() + run, v);
            }
        }
        for (i, v) in plane.iter().enumerate() {
            out[i * bpp + b] = *v;
        }
    }
    Ok(out)
}
