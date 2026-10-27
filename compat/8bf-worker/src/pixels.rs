//! Checked rectangle/plane packing at the native filter boundary.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub top: i32,
    pub left: i32,
    pub bottom: i32,
    pub right: i32,
}
impl Rect {
    pub fn empty(self) -> bool {
        self.bottom <= self.top || self.right <= self.left
    }
}

#[derive(Clone, Copy)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub channels: u8,
    pub depth: u8,
}
impl Image {
    fn sample_bytes(self) -> usize {
        usize::from(self.depth / 8)
    }
    fn pixel_bytes(self) -> usize {
        usize::from(self.channels) * self.sample_bytes()
    }
}
fn to_native(value: u16) -> u16 {
    ((u32::from(value) * 32768 + 32767) / 65535) as u16
}
fn from_native(value: u16) -> u16 {
    ((u32::from(value) * 65535 + 16384) / 32768) as u16
}

fn layout(len: usize, image: Image, r: Rect, lo: i16, hi: i16) -> Result<(usize, usize, usize, usize), String> {
    if !matches!(image.channels, 1 | 3) || !matches!(image.depth, 8 | 16) || image.width == 0 || image.height == 0 {
        return Err("unsupported image format".into());
    }
    let full = u64::from(image.width).checked_mul(u64::from(image.height)).and_then(|n| n.checked_mul(image.pixel_bytes() as u64)).ok_or("image overflow")?;
    if full > 512 * 1024 * 1024 || full != len as u64 {
        return Err("invalid image length".into());
    }
    if lo < 0 || hi < lo || hi >= i16::from(image.channels) {
        return Err("unsupported channel range".into());
    }
    if r.top < 0 || r.left < 0 || r.bottom < r.top || r.right < r.left || r.bottom as u32 > image.height || r.right as u32 > image.width {
        return Err("filter requested pixels outside image bounds".into());
    }
    Ok((image.width as usize, (r.right - r.left) as usize, (r.bottom - r.top) as usize, (hi - lo + 1) as usize))
}

pub fn pack(source: &[u8], image: Image, r: Rect, lo: i16, hi: i16) -> Result<Vec<u8>, String> {
    let (stride, w, h, planes) = layout(source.len(), image, r, lo, hi)?;
    let full_pixel = image.pixel_bytes();
    let sample = image.sample_bytes();
    let packed_pixel = planes * sample;
    let mut out = Vec::new();
    let size = w * h * packed_pixel;
    out.try_reserve_exact(size).map_err(|e| format!("8BF tile allocation: {e}"))?;
    out.resize(size, 0);
    for row in 0..h {
        let start = ((r.top as usize + row) * stride + r.left as usize) * full_pixel;
        let input = source.get(start..start + w * full_pixel).ok_or("input row outside image")?;
        let output = out.get_mut(row * w * packed_pixel..(row + 1) * w * packed_pixel).ok_or("output row outside allocation")?;
        if packed_pixel == full_pixel && sample == 1 {
            output.copy_from_slice(input);
        } else {
            for (src, dst) in input.chunks_exact(full_pixel).zip(output.chunks_exact_mut(packed_pixel)) {
                let selected = src.get(lo as usize * sample..(hi as usize + 1) * sample).ok_or("channel out of range")?;
                if sample == 1 {
                    dst.copy_from_slice(selected);
                } else {
                    for (value, to) in selected.as_chunks::<2>().0.iter().zip(dst.as_chunks_mut::<2>().0.iter_mut()) {
                        *to = to_native(u16::from_le_bytes(*value)).to_le_bytes();
                    }
                }
            }
        }
    }
    Ok(out)
}

pub fn unpack(output: &mut [u8], image: Image, r: Rect, lo: i16, hi: i16, packed: &[u8]) -> Result<(), String> {
    let (stride, w, h, planes) = layout(output.len(), image, r, lo, hi)?;
    let full_pixel = image.pixel_bytes();
    let sample = image.sample_bytes();
    let packed_pixel = planes * sample;
    if packed.len() != w * h * packed_pixel {
        return Err("filter output length mismatch".into());
    }
    if sample == 2 && packed.as_chunks::<2>().0.iter().any(|v| u16::from_le_bytes(*v) > 32768) {
        return Err("8BF filter returned a 16-bit sample outside the native 0..32768 range".into());
    }
    for row in 0..h {
        let start = ((r.top as usize + row) * stride + r.left as usize) * full_pixel;
        let dst = output.get_mut(start..start + w * full_pixel).ok_or("output row outside image")?;
        let src = packed.get(row * w * packed_pixel..(row + 1) * w * packed_pixel).ok_or("input row outside allocation")?;
        if packed_pixel == full_pixel && sample == 1 {
            dst.copy_from_slice(src);
        } else {
            for (src, dst) in src.chunks_exact(packed_pixel).zip(dst.chunks_exact_mut(full_pixel)) {
                let selected = dst.get_mut(lo as usize * sample..(hi as usize + 1) * sample).ok_or("channel out of range")?;
                if sample == 1 {
                    selected.copy_from_slice(src);
                } else {
                    for (value, to) in src.as_chunks::<2>().0.iter().zip(selected.as_chunks_mut::<2>().0.iter_mut()) {
                        let native = u16::from_le_bytes(*value);
                        // Keep every original full-range code for samples left unchanged.
                        if native != to_native(u16::from_le_bytes(*to)) {
                            *to = from_native(native).to_le_bytes();
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn packs_subset_planes_and_commits_only_requested_rectangle() {
        let image = Image { width: 4, height: 3, channels: 3, depth: 8 };
        let source: Vec<u8> = (0..36).collect();
        let rect = Rect { top: 1, left: 1, bottom: 2, right: 3 };
        let packed = pack(&source, image, rect, 1, 2).unwrap();
        assert_eq!(packed, vec![16, 17, 19, 20]);
        let mut output = source.clone();
        unpack(&mut output, image, rect, 1, 2, &[100, 101, 102, 103]).unwrap();
        assert_eq!(&output[15..21], &[15, 100, 101, 18, 102, 103]);
        assert_eq!(&output[..15], &source[..15]);
        assert_eq!(&output[21..], &source[21..]);
    }
    #[test]
    fn rejects_invalid_rectangles_planes_and_buffers() {
        let image = Image { width: 2, height: 2, channels: 3, depth: 8 };
        for rect in [
            Rect { top: -1, left: 0, bottom: 2, right: 2 },
            Rect { top: 0, left: 0, bottom: 3, right: 2 },
            Rect { top: 0, left: 0, bottom: 2, right: i32::MAX },
        ] {
            assert!(pack(&[0; 12], image, rect, 0, 2).is_err());
        }
        let rect = Rect { top: 0, left: 0, bottom: 2, right: 2 };
        assert!(pack(&[0; 11], image, rect, 0, 2).is_err());
        assert!(pack(&[0; 12], image, rect, -1, 2).is_err());
        assert!(pack(&[0; 12], image, rect, 0, 3).is_err());
        assert!(unpack(&mut [0; 12], image, rect, 0, 2, &[0; 11]).is_err());
    }
    #[test]
    fn all_full_range_gray16_codes_survive_identity() {
        let image = Image { width: 256, height: 256, channels: 1, depth: 16 };
        let rect = Rect { top: 0, left: 0, bottom: 256, right: 256 };
        let source: Vec<u8> = (0..=u16::MAX).flat_map(u16::to_le_bytes).collect();
        let packed = pack(&source, image, rect, 0, 0).unwrap();
        assert_eq!(&packed[..6], &[0, 0, 1, 0, 1, 0]);
        assert_eq!(&packed[packed.len() - 2..], &[0, 128]);
        let mut output = source.clone();
        unpack(&mut output, image, rect, 0, 0, &packed).unwrap();
        assert_eq!(output, source);
    }
    #[test]
    fn sixteen_bit_subset_planes_and_invalid_values() {
        let image = Image { width: 2, height: 1, channels: 3, depth: 16 };
        let rect = Rect { top: 0, left: 1, bottom: 1, right: 2 };
        let source: Vec<u8> = [1u16, 3, 5, 7, 65535, 11].into_iter().flat_map(u16::to_le_bytes).collect();
        assert_eq!(pack(&source, image, rect, 1, 1).unwrap(), [0, 128]);
        let mut output = source.clone();
        assert!(unpack(&mut output, image, rect, 1, 1, &[255, 255]).is_err());
        assert_eq!(output, source);
        unpack(&mut output, image, rect, 1, 1, &[0, 0]).unwrap();
        let expected: Vec<u8> = [1u16, 3, 5, 7, 0, 11].into_iter().flat_map(u16::to_le_bytes).collect();
        assert_eq!(output, expected);
    }
}
