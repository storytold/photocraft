//! Bounds-checked little-endian records and row-wise, exact-size zlib inflation.

use std::io::Read;

use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};

use super::{IoError, Result, invalid};

pub(super) struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    pub(super) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }
    pub(super) fn remaining(&self) -> usize {
        self.bytes.len()
    }
    pub(super) fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let data = self.bytes.get(..n).ok_or_else(|| invalid("truncated record"))?;
        self.bytes = self.bytes.get(n..).ok_or_else(|| invalid("truncated record"))?;
        Ok(data)
    }
    pub(super) fn u8(&mut self) -> Result<u8> {
        self.take(1)?.first().copied().ok_or_else(|| invalid("truncated byte"))
    }
    pub(super) fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().map_err(|_| invalid("truncated word"))?))
    }
    pub(super) fn i16(&mut self) -> Result<i16> {
        Ok(self.u16()? as i16)
    }
    pub(super) fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().map_err(|_| invalid("truncated dword"))?))
    }
    pub(super) fn string(&mut self) -> Result<String> {
        let len = usize::from(self.u16()?);
        std::str::from_utf8(self.take(len)?).map(str::to_string).map_err(|_| invalid("layer name is not UTF-8"))
    }
    pub(super) fn end(&self) -> Result<()> {
        if self.bytes.is_empty() { Ok(()) } else { Err(invalid("unexpected trailing record data")) }
    }
}

pub(super) fn pixels(r: &mut Reader<'_>, compressed: bool, rect: Rect, surface: &mut Surface, ctl: &Interrupt<'_>) -> Result<()> {
    let data = r.take(r.remaining())?;
    if compressed {
        let mut z = flate2::bufread::ZlibDecoder::new(data);
        rows(&mut z, rect, surface, ctl)?;
        if z.total_in() != data.len() as u64 {
            return Err(invalid("trailing compressed cel data"));
        }
    } else {
        let expected = (rect.width() as usize)
            .checked_mul(rect.height() as usize)
            .and_then(|n| n.checked_mul(surface.format().bytes_per_pixel()))
            .ok_or_else(|| invalid("cel byte count overflow"))?;
        if data.len() != expected {
            return Err(invalid("raw cel byte count does not match its dimensions"));
        }
        rows(&mut &data[..], rect, surface, ctl)?;
    }
    surface.prune();
    Ok(())
}

fn rows(input: &mut impl Read, rect: Rect, surface: &mut Surface, ctl: &Interrupt<'_>) -> Result<()> {
    let mut row = vec![0; rect.width() as usize * surface.format().bytes_per_pixel()];
    for y in rect.y0..if rect.is_empty() { rect.y0 } else { rect.y1 } {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        input.read_exact(&mut row).map_err(|e| invalid(format!("invalid cel pixels: {e}")))?;
        surface.try_write_interleaved(Rect::from_xywh(rect.x0, y, rect.width(), 1), &row).map_err(|e| invalid(e.to_string()))?;
    }
    let mut extra = [0];
    if input.read(&mut extra).map_err(|e| invalid(format!("invalid cel stream: {e}")))? != 0 {
        return Err(invalid("cel expands beyond its declared dimensions"));
    }
    Ok(())
}
