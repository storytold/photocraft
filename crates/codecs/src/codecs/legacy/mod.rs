//! Clean-room baseline codecs from the public specifications linked in docs/export-formats.md.
//! Unsupported compression/colour variants return errors; all dimensions precede allocation.
mod binary;
mod text;
use crate::fidelity::Plan;
use crate::{ChannelLayout as L, CodecError as E, EncodeOptions, Format as F, Image, Limits, SampleType as S};

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
    format: F,
}
impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8], format: F) -> Self {
        Self { bytes, pos: 0, format }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], E> {
        let end = self.pos.checked_add(n).ok_or_else(|| self.bad("offset overflow"))?;
        let v = self.bytes.get(self.pos..end).ok_or_else(|| self.bad("truncated file"))?;
        self.pos = end;
        Ok(v)
    }
    fn u8(&mut self) -> Result<u8, E> {
        self.take(1)?.first().copied().ok_or_else(|| self.bad("missing byte"))
    }
    fn be16(&mut self) -> Result<u16, E> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().map_err(|_| self.bad("short integer"))?))
    }
    fn be32(&mut self) -> Result<u32, E> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().map_err(|_| self.bad("short integer"))?))
    }
    fn le16(&mut self) -> Result<u16, E> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().map_err(|_| self.bad("short integer"))?))
    }
    fn le32(&mut self) -> Result<u32, E> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().map_err(|_| self.bad("short integer"))?))
    }
    fn bad(&self, msg: &str) -> E {
        E::malformed(self.format, msg)
    }
}
fn buffer(n: usize, limits: &Limits) -> Result<Vec<u8>, E> {
    if n as u64 > limits.max_alloc {
        return Err(E::LimitExceeded("raster buffer exceeds allocation budget".into()));
    }
    let mut b = Vec::new();
    b.try_reserve_exact(n).map_err(|e| E::LimitExceeded(e.to_string()))?;
    b.resize(n, 0);
    Ok(b)
}
fn layout(ch: u32, f: F) -> Result<L, E> {
    match ch {
        1 => Ok(L::Gray),
        2 => Ok(L::GrayA),
        3 => Ok(L::Rgb),
        4 => Ok(L::Rgba),
        _ => Err(E::unsupported(f, "unsupported channel count")),
    }
}
fn byte(b: &[u8], i: usize, f: F) -> Result<u8, E> {
    b.get(i).copied().ok_or_else(|| E::malformed(f, "truncated pixel data"))
}
fn set(b: &mut [u8], i: usize, v: u8, f: F) -> Result<(), E> {
    *b.get_mut(i).ok_or_else(|| E::malformed(f, "pixel offset out of bounds"))? = v;
    Ok(())
}

pub(crate) fn encode(f: F, src: &Image, p: Plan, opts: &EncodeOptions) -> Result<Vec<u8>, E> {
    let img = src.converted(p.layout, p.sample);
    match f {
        F::Xbm | F::Xpm => text::encode(f, &img),
        _ => binary::encode(f, &img, opts),
    }
}
pub(crate) fn decode(f: F, b: &[u8], l: &Limits) -> Result<Image, E> {
    match f {
        F::Xbm | F::Xpm => text::decode(f, b, l),
        _ => binary::decode(f, b, l),
    }
}
pub(crate) fn detect(b: &[u8]) -> Option<F> {
    if b.starts_with(b"farbfeld") {
        return Some(F::Farbfeld);
    }
    if b.starts_with(&[1, 218]) {
        return Some(F::Sgi);
    }
    if b.starts_with(&[0x59, 0xa6, 0x6a, 0x95]) {
        return Some(F::SunRaster);
    }
    if b.starts_with(b"icns") {
        return Some(F::Icns);
    }
    if b.get(20..24) == Some(b"GIMP") {
        return Some(F::Gbr);
    }
    if b.get(20..24) == Some(b"GPAT") {
        return Some(F::GimpPat);
    }
    if b.starts_with(&[0, 0, 2, 0]) && b.get(4..6) != Some(&[0, 0]) {
        return Some(F::Cur);
    }
    if b.starts_with(&[10]) && b.get(2) == Some(&1) && b.get(3) == Some(&8) {
        return Some(F::Pcx);
    }
    if b.starts_with(b"/* XPM */") {
        return Some(F::Xpm);
    }
    if let Ok(s) = std::str::from_utf8(b.get(..b.len().min(4096)).unwrap_or_default())
        && s.contains("#define")
        && s.contains("_width")
        && s.contains("_bits")
    {
        return Some(F::Xbm);
    }
    // WBMP has no unique signature. Require a complete, exactly-sized Type-0 payload, after
    // the unique signatures above; never guess from just its two zero header bytes.
    if b.starts_with(&[0, 0])
        && binary::wbmp_header(b)
            .is_ok_and(|(w, h, off)| w > 0 && h > 0 && (u64::from(w).div_ceil(8) * u64::from(h)).checked_add(off as u64) == Some(b.len() as u64))
    {
        return Some(F::Wbmp);
    }
    None
}
