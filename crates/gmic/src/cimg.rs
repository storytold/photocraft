//! Uncompressed `.cimg` image lists: G'MIC's native file format, used to pass pixels to and from
//! the `gmic` program without losing precision.
//!
//! The layout, as G'MIC writes it (`output file.cimg,float32`):
//!
//! ```text
//! <count> <type> <endianness>\n          e.g. "1 float32 little_endian"
//! <width> <height> <depth> <spectrum>\n  per image, followed by its samples
//! ```
//!
//! Samples are planar: x varies fastest, then y, then z, then the channel (G'MIC calls the
//! channel count the spectrum). Values are in G'MIC's usual 0–255 range for 8-bit-like data;
//! scaling is the caller's business ([`crate::apply`]). Compressed images (`#<size>` after the
//! dimensions) and 3D volumes (depth > 1) are rejected.

use crate::{Error, Result};

/// One 2D image: `channels` planes of `width × height` samples.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub channels: u32,
    /// Planar samples, `channels × height × width` (x fastest).
    pub data: Vec<f32>,
}

impl Image {
    /// An image of zeros, or `None` when its sample count overflows.
    pub fn zeros(width: u32, height: u32, channels: u32) -> Option<Image> {
        let n = samples(width, height, 1, channels)?;
        Some(Image { width, height, channels, data: vec![0.0; n] })
    }

    /// Sample `(x, y)` of channel `c`, or `None` outside the image.
    pub fn get(&self, x: u32, y: u32, c: u32) -> Option<f32> {
        if x >= self.width || y >= self.height || c >= self.channels {
            return None;
        }
        let i = (c as usize * self.height as usize + y as usize) * self.width as usize + x as usize;
        self.data.get(i).copied()
    }
}

fn samples(w: u32, h: u32, d: u32, c: u32) -> Option<usize> {
    (w as usize).checked_mul(h as usize)?.checked_mul(d as usize)?.checked_mul(c as usize)
}

/// Writes `images` as an uncompressed little-endian float32 `.cimg` list.
pub fn encode(images: &[Image]) -> Vec<u8> {
    let total: usize = images.iter().map(|i| i.data.len() * 4 + 48).sum();
    let mut out = Vec::with_capacity(total + 32);
    out.extend_from_slice(format!("{} float32 little_endian\n", images.len()).as_bytes());
    for img in images {
        out.extend_from_slice(format!("{} {} 1 {}\n", img.width, img.height, img.channels).as_bytes());
        for v in &img.data {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    out
}

#[derive(Clone, Copy)]
enum Sample {
    U8,
    I8,
    U16,
    I16,
    U32,
    I32,
    U64,
    I64,
    F32,
    F64,
}

impl Sample {
    fn parse(s: &str) -> Option<Sample> {
        Some(match s {
            "bool" | "uint8" | "uchar" | "unsigned_char" => Sample::U8,
            "int8" | "char" => Sample::I8,
            "uint16" | "ushort" | "unsigned_short" => Sample::U16,
            "int16" | "short" => Sample::I16,
            "uint32" | "uint" | "unsigned_int" => Sample::U32,
            "int32" | "int" => Sample::I32,
            "uint64" | "unsigned_int64" => Sample::U64,
            "int64" => Sample::I64,
            "float32" | "float" => Sample::F32,
            "float64" | "double" => Sample::F64,
            _ => return None,
        })
    }

    fn size(self) -> usize {
        match self {
            Sample::U8 | Sample::I8 => 1,
            Sample::U16 | Sample::I16 => 2,
            Sample::U32 | Sample::I32 | Sample::F32 => 4,
            Sample::U64 | Sample::I64 | Sample::F64 => 8,
        }
    }

    fn read(self, b: &[u8], big: bool) -> f32 {
        macro_rules! num {
            ($t:ty, $n:literal) => {{
                let mut a = [0u8; $n];
                if let Some(src) = b.get(..$n) {
                    a.copy_from_slice(src);
                }
                if big { <$t>::from_be_bytes(a) } else { <$t>::from_le_bytes(a) }
            }};
        }
        match self {
            Sample::U8 => f32::from(b.first().copied().unwrap_or(0)),
            Sample::I8 => f32::from(b.first().copied().unwrap_or(0) as i8),
            Sample::U16 => f32::from(num!(u16, 2)),
            Sample::I16 => f32::from(num!(i16, 2)),
            Sample::U32 => num!(u32, 4) as f32,
            Sample::I32 => num!(i32, 4) as f32,
            Sample::U64 => num!(u64, 8) as f32,
            Sample::I64 => num!(i64, 8) as f32,
            Sample::F32 => num!(f32, 4),
            Sample::F64 => num!(f64, 8) as f32,
        }
    }
}

/// The text line starting at `*pos` (without its `\n`); advances past it.
fn line<'a>(bytes: &'a [u8], pos: &mut usize) -> Result<&'a str> {
    let rest = bytes.get(*pos..).ok_or_else(|| Error::Cimg("truncated header".into()))?;
    // Header lines are short; a missing newline in the first 256 bytes means this isn't .cimg.
    let end = rest.iter().take(256).position(|&b| b == b'\n').ok_or_else(|| Error::Cimg("missing header line".into()))?;
    *pos += end + 1;
    std::str::from_utf8(rest.get(..end).unwrap_or_default()).map_err(|_| Error::Cimg("header is not text".into()))
}

/// Reads a `.cimg` list. `max_samples` caps the total sample count (and so the allocation).
pub fn decode(bytes: &[u8], max_samples: usize) -> Result<Vec<Image>> {
    let mut pos = 0;
    let head = line(bytes, &mut pos)?;
    let mut words = head.split_whitespace();
    let count: usize = words.next().and_then(|w| w.parse().ok()).ok_or_else(|| Error::Cimg(format!("bad header {head:?}")))?;
    let sample = words.next().and_then(Sample::parse).ok_or_else(|| Error::Cimg(format!("unsupported sample type in {head:?}")))?;
    let big = match words.next() {
        None | Some("little_endian") => false,
        Some("big_endian") => true,
        Some(other) => return Err(Error::Cimg(format!("unknown byte order {other:?}"))),
    };
    if count > 4096 {
        return Err(Error::Cimg(format!("{count} images is more than the limit (4096)")));
    }
    let mut images = Vec::with_capacity(count);
    let mut total = 0usize;
    for _ in 0..count {
        let dims = line(bytes, &mut pos)?;
        if dims.contains('#') {
            return Err(Error::Cimg("compressed .cimg data is not supported".into()));
        }
        let nums: Vec<u32> = dims.split_whitespace().map(|w| w.parse::<u32>()).collect::<std::result::Result<_, _>>().map_err(|_| Error::Cimg(format!("bad image size {dims:?}")))?;
        let &[w, h, d, c] = nums.as_slice() else { return Err(Error::Cimg(format!("bad image size {dims:?}"))) };
        let n = samples(w, h, d, c).ok_or_else(|| Error::Cimg("image size overflows".into()))?;
        if n == 0 {
            images.push(Image { width: 0, height: 0, channels: 0, data: Vec::new() });
            continue;
        }
        if d != 1 {
            return Err(Error::Cimg(format!("3D images (depth {d}) are not supported")));
        }
        total = total.checked_add(n).filter(|t| *t <= max_samples).ok_or_else(|| Error::Cimg(format!("image data is larger than the limit ({max_samples} samples)")))?;
        let len = n.checked_mul(sample.size()).ok_or_else(|| Error::Cimg("image size overflows".into()))?;
        let raw = bytes.get(pos..).and_then(|r| r.get(..len)).ok_or_else(|| Error::Cimg("truncated image data".into()))?;
        pos += len;
        let data = raw.chunks_exact(sample.size()).map(|b| sample.read(b, big)).collect();
        images.push(Image { width: w, height: h, channels: c, data });
    }
    Ok(images)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(w: u32, h: u32, c: u32) -> Image {
        let n = (w * h * c) as usize;
        Image { width: w, height: h, channels: c, data: (0..n).map(|i| i as f32 * 0.5 - 3.0).collect() }
    }

    #[test]
    fn round_trips_one_to_four_channels() {
        for c in 1..=4 {
            let list = vec![img(5, 3, c), img(1, 1, c)];
            let back = decode(&encode(&list), 1 << 20).unwrap();
            assert_eq!(back, list);
        }
    }

    #[test]
    fn reads_what_gmic_writes() {
        // `gmic 2,3,1,2 fill 1,2,...,12 o a.cimg,float32`, byte for byte.
        let mut b = b"1 float32 little_endian\n2 3 1 2\n".to_vec();
        for v in 1..=12 {
            b.extend_from_slice(&(v as f32).to_le_bytes());
        }
        let i = decode(&b, 100).unwrap().remove(0);
        assert_eq!((i.width, i.height, i.channels), (2, 3, 2));
        assert_eq!(i.get(1, 0, 0), Some(2.0));
        assert_eq!(i.get(0, 1, 0), Some(3.0));
        assert_eq!(i.get(0, 0, 1), Some(7.0), "planar: channel 1 starts after w×h samples");
        // The default (smallest) type for that data: `o b.cimg`.
        let mut u = b"1 uint8 little_endian\n2 3 1 2\n".to_vec();
        u.extend(1..=12u8);
        assert_eq!(decode(&u, 100).unwrap()[0].data, i.data);
    }

    #[test]
    fn big_endian_and_other_types() {
        let mut b = b"1 int16 big_endian\n2 1 1 1\n".to_vec();
        b.extend_from_slice(&(-2i16).to_be_bytes());
        b.extend_from_slice(&300i16.to_be_bytes());
        assert_eq!(decode(&b, 10).unwrap()[0].data, vec![-2.0, 300.0]);
        let mut d = b"1 float64\n1 1 1 1\n".to_vec();
        d.extend_from_slice(&1.5f64.to_le_bytes());
        assert_eq!(decode(&d, 10).unwrap()[0].data, vec![1.5]);
    }

    #[test]
    fn empty_images_are_kept() {
        let b = b"2 float32 little_endian\n0 0 0 0\n1 1 1 1\n\0\0\x80\x3f".to_vec();
        let l = decode(&b, 10).unwrap();
        assert_eq!(l.len(), 2);
        assert!(l[0].data.is_empty());
        assert_eq!(l[1].data, vec![1.0]);
    }

    #[test]
    fn malformed_input_is_an_error_not_a_panic() {
        let cases: &[&[u8]] = &[
            b"",
            b"garbage",
            b"1 float32 little_endian",
            b"1 complex little_endian\n1 1 1 1\n",
            b"1 float32 middle_endian\n1 1 1 1\n",
            b"1 float32\n2 2 1 1\n\0\0\0\0",
            b"1 float32\n-1 2 1 1\n",
            b"1 float32\n4294967295 4294967295 4294967295 4294967295\n",
            b"1 float32\n2 2 2 1\n",
            b"1 float32\n2 2 1 1 #16\n",
            b"1 float32\n2 2 1\n",
            b"99999 float32\n",
            b"1 float32\n\xff\xfe 1 1 1\n",
        ];
        for c in cases {
            assert!(decode(c, 1 << 20).is_err(), "{:?}", String::from_utf8_lossy(c));
        }
    }

    #[test]
    fn caps_the_allocation() {
        let b = b"1 float32\n1000 1000 1 4\n".to_vec();
        let e = decode(&b, 1000).unwrap_err();
        assert!(e.to_string().contains("limit"), "{e}");
    }
}
