//! Independently reconstructed binary interface offsets. See ABI-PROVENANCE.md.
//! This is an aligned byte record, not a translated vendor SDK declaration.
use crate::pixels::Rect;

#[derive(Clone, Copy)]
pub struct Offsets {
    pub parameters: usize,
    pub size: usize,
    pub planes: usize,
    pub filter: usize,
    pub input: usize,
    pub input_low: usize,
    pub output: usize,
    pub output_low: usize,
    pub input_data: usize,
    pub input_stride: usize,
    pub output_data: usize,
    pub output_stride: usize,
    pub mask: usize,
    pub signature: usize,
    pub mode: usize,
    pub platform: usize,
    pub properties: usize,
}
#[cfg(target_pointer_width = "32")]
pub const O: Offsets = Offsets {
    parameters: 0xc,
    size: 0x10,
    planes: 0x14,
    filter: 0x16,
    input: 0x34,
    input_low: 0x3c,
    output: 0x40,
    output_low: 0x48,
    input_data: 0x4c,
    input_stride: 0x50,
    output_data: 0x54,
    output_stride: 0x58,
    mask: 0x60,
    signature: 0x78,
    mode: 0x80,
    platform: 0xbc,
    properties: 0x140,
};
#[cfg(target_pointer_width = "64")]
pub const O: Offsets = Offsets {
    parameters: 0x14,
    size: 0x1c,
    planes: 0x20,
    filter: 0x22,
    input: 0x40,
    input_low: 0x48,
    output: 0x4c,
    output_low: 0x54,
    input_data: 0x58,
    input_stride: 0x60,
    output_data: 0x64,
    output_stride: 0x6c,
    mask: 0x74,
    signature: 0x90,
    mode: 0x9c,
    platform: 0xd8,
    properties: 0x188,
};

/// 4096 bytes, aligned for either architecture. Unused extension slots are zero.
pub struct Record([u64; 512]);
impl Record {
    pub fn new() -> Self {
        Self([0; 512])
    }
    pub fn ptr(&mut self) -> *mut std::ffi::c_void {
        self.0.as_mut_ptr().cast()
    }
    pub fn get<const N: usize>(&self, offset: usize) -> [u8; N] {
        let mut out = [0; N];
        for (i, b) in out.iter_mut().enumerate() {
            let at = offset + i;
            if let Some(word) = self.0.get(at / 8) {
                *b = word.to_le_bytes()[at % 8];
            }
        }
        out
    }
    pub fn put(&mut self, offset: usize, value: &[u8]) {
        for (i, b) in value.iter().enumerate() {
            let at = offset + i;
            if let Some(word) = self.0.get_mut(at / 8) {
                let mut bytes = word.to_le_bytes();
                bytes[at % 8] = *b;
                *word = u64::from_le_bytes(bytes);
            }
        }
    }
    pub fn i16(&self, at: usize) -> i16 {
        i16::from_le_bytes(self.get(at))
    }
    pub fn set16(&mut self, at: usize, n: i16) {
        self.put(at, &n.to_le_bytes())
    }
    pub fn set32(&mut self, at: usize, n: i32) {
        self.put(at, &n.to_le_bytes())
    }
    pub fn set_ptr(&mut self, at: usize, n: usize) {
        self.put(at, &n.to_le_bytes())
    }
    pub fn rect(&self, at: usize) -> Rect {
        Rect { top: i32::from(self.i16(at)), left: i32::from(self.i16(at + 2)), bottom: i32::from(self.i16(at + 4)), right: i32::from(self.i16(at + 6)) }
    }
    pub fn set_rect(&mut self, at: usize, r: Rect) {
        for (i, n) in [r.top, r.left, r.bottom, r.right].iter().enumerate() {
            self.set16(at + i * 2, *n as i16);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unaligned_pointers_do_not_overwrite_neighboring_fields() {
        let mut r = Record::new();
        r.set32(0, 0x12345678);
        r.set_ptr(4, usize::MAX);
        assert_eq!(r.get::<4>(0), 0x12345678i32.to_le_bytes());
        assert_eq!(r.get::<{ size_of::<usize>() }>(4), usize::MAX.to_le_bytes());
        assert_eq!(r.get::<4>(4 + size_of::<usize>()), [0; 4]);
    }
    #[test]
    fn signed_rect_roundtrip() {
        let mut r = Record::new();
        let rect = Rect { top: -1, left: 3, bottom: 50, right: 70 };
        r.set_rect(O.input, rect);
        assert_eq!(r.rect(O.input), rect);
    }
}
