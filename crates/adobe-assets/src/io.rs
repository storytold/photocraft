//! A bounds-checked big-endian cursor and the matching writers.

use crate::error::{AssetError, Result};

/// A cursor over a byte slice. Every read is bounds checked.
#[derive(Debug, Clone)]
pub(crate) struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Reader { data, pos: 0 }
    }

    pub(crate) fn pos(&self) -> usize {
        self.pos
    }

    pub(crate) fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    /// The bytes between `start` and the current position.
    pub(crate) fn since(&self, start: usize) -> &'a [u8] {
        self.data.get(start..self.pos).unwrap_or_default()
    }

    pub(crate) fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        if n > self.remaining() {
            return Err(AssetError::UnexpectedEof { offset: self.pos, needed: n - self.remaining() });
        }
        let b = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(b)
    }

    pub(crate) fn skip(&mut self, n: usize) -> Result<()> {
        self.bytes(n).map(|_| ())
    }

    pub(crate) fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut a = [0u8; N];
        a.copy_from_slice(self.bytes(N)?);
        Ok(a)
    }

    pub(crate) fn u8(&mut self) -> Result<u8> {
        Ok(self.array::<1>()?[0])
    }

    pub(crate) fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.array()?))
    }

    pub(crate) fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.array()?))
    }

    /// Returns `Err` unless `count * min_item_size` bytes remain. Used to validate element
    /// counts before allocating or looping.
    pub(crate) fn check_count(&self, count: u64, min_item_size: u64) -> Result<()> {
        let need = count.saturating_mul(min_item_size);
        let have = self.remaining() as u64;
        if need > have {
            return Err(AssetError::UnexpectedEof { offset: self.pos, needed: usize::try_from(need - have).unwrap_or(usize::MAX) });
        }
        Ok(())
    }

    /// A u32 count of UTF-16 code units, then the units.
    pub(crate) fn unicode(&mut self) -> Result<Vec<u16>> {
        let n = self.u32()?;
        self.check_count(u64::from(n), 2)?;
        (0..n).map(|_| self.u16()).collect()
    }

    /// A u32 byte length, then the bytes.
    pub(crate) fn byte_string(&mut self) -> Result<&'a [u8]> {
        let n = self.u32()? as usize;
        self.bytes(n)
    }
}

/// Big-endian writing helpers on `Vec<u8>`.
pub(crate) trait WriteExt {
    fn put_u8(&mut self, v: u8);
    fn put_u16(&mut self, v: u16);
    fn put_u32(&mut self, v: u32);
    /// A u32 count of UTF-16 code units, then the units.
    fn put_unicode(&mut self, units: &[u16]);
    /// A u32 byte length, then the bytes.
    fn put_byte_string(&mut self, b: &[u8]);
}

impl WriteExt for Vec<u8> {
    fn put_u8(&mut self, v: u8) {
        self.push(v);
    }
    fn put_u16(&mut self, v: u16) {
        self.extend_from_slice(&v.to_be_bytes());
    }
    fn put_u32(&mut self, v: u32) {
        self.extend_from_slice(&v.to_be_bytes());
    }
    fn put_unicode(&mut self, units: &[u16]) {
        self.put_u32(units.len() as u32);
        for u in units {
            self.put_u16(*u);
        }
    }
    fn put_byte_string(&mut self, b: &[u8]) {
        self.put_u32(b.len() as u32);
        self.extend_from_slice(b);
    }
}
