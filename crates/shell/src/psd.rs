//! Allocation-free PSD parser that locates the embedded thumbnail.
//!
//! Layout (all integers big-endian): header (26 bytes) | color mode data (u32 length) | image
//! resources (u32 length) | ... Each image resource block is `8BIM`, a u16 id, a Pascal name
//! padded to an even size, a u32 size and the data padded to an even size.

/// Thumbnail resource written by Photoshop 5 and later (JFIF, RGB).
const RESOURCE_THUMBNAIL: u16 = 0x040C;
/// `format` value of a JPEG RGB thumbnail (0 would be raw RGB).
const FORMAT_JPEG_RGB: u32 = 1;
/// The fixed header (26 bytes) plus the length of the color mode data (4 bytes).
const FIXED_LEN: usize = 30;
/// Longest side, in pixels, of a thumbnail that is decoded. Photoshop writes 160 at most; the cap
/// keeps a hostile file from declaring a huge image that the decoder would allocate for.
const MAX_THUMBNAIL_SIDE: u16 = 2048;

/// Bounds-checked big-endian cursor over a byte slice. Never panics.
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    /// A reader over `data`, positioned at the start.
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    /// The next `len` bytes, or `None` (without moving) when fewer remain.
    fn take(&mut self, len: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(len)?;
        let slice = self.data.get(self.pos..end)?;
        self.pos = end;
        Some(slice)
    }

    /// Reads one byte.
    fn u8(&mut self) -> Option<u8> {
        self.take(1)?.first().copied()
    }

    /// Reads a big-endian `u16`.
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_be_bytes(self.take(2)?.try_into().ok()?))
    }

    /// Reads a big-endian `u32`.
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_be_bytes(self.take(4)?.try_into().ok()?))
    }

    /// The bytes not read yet.
    fn rest(&self) -> &'a [u8] {
        self.data.get(self.pos..).unwrap_or(&[])
    }
}

/// The JFIF bytes of the embedded thumbnail, borrowed from `psd`.
///
/// `None` for anything that is not a well-formed PSD/PSB with a JPEG thumbnail. A truncated file
/// is fine as long as the thumbnail block itself is complete.
pub fn find_thumbnail_jpeg(psd: &[u8]) -> Option<&[u8]> {
    let mut r = Reader::new(psd);
    if r.take(4)? != b"8BPS" {
        return None;
    }
    let version = r.u16()?;
    if version != 1 && version != 2 {
        return None;
    }
    // Reserved (6), channels (2), height (4), width (4), depth (2), mode (2).
    r.take(20)?;

    let color_mode_len = usize::try_from(r.u32()?).ok()?;
    r.take(color_mode_len)?;

    let resources_len = usize::try_from(r.u32()?).ok()?;
    let rest = r.rest();
    let mut blocks = Reader::new(rest.get(..resources_len.min(rest.len()))?);

    loop {
        if blocks.take(4)? != b"8BIM" {
            return None;
        }
        let id = blocks.u16()?;
        let name_len = usize::from(blocks.u8()?);
        // The Pascal string (length byte + text) is padded to an even size.
        blocks.take(name_len + (1 + name_len) % 2)?;
        let size = usize::try_from(blocks.u32()?).ok()?;
        let data = blocks.take(size)?;
        if size % 2 == 1 {
            let _ = blocks.take(1); // The padding may be missing at the very end.
        }
        if id == RESOURCE_THUMBNAIL {
            return parse_thumbnail(data);
        }
    }
}

/// The JPEG inside a 1036 thumbnail resource block (after its 28-byte header), if it is a JFIF.
fn parse_thumbnail(block: &[u8]) -> Option<&[u8]> {
    let mut r = Reader::new(block);
    if r.u32()? != FORMAT_JPEG_RGB {
        return None;
    }
    // Width, height, width bytes, total size.
    r.take(16)?;
    let compressed = usize::try_from(r.u32()?).ok()?;
    // Bits per pixel and planes.
    r.take(4)?;

    let data = r.rest();
    let jpeg = if compressed > 0 { data.get(..compressed.min(data.len()))? } else { data };
    plausible_jpeg(jpeg).then_some(jpeg)
}

/// A JPEG whose own header declares a size worth decoding.
fn plausible_jpeg(jpeg: &[u8]) -> bool {
    let sides = 1..=MAX_THUMBNAIL_SIDE;
    matches!(jpeg_size(jpeg), Some((width, height)) if sides.contains(&width) && sides.contains(&height))
}

/// Width and height from the first start-of-frame marker of a JPEG, read without decoding it.
fn jpeg_size(jpeg: &[u8]) -> Option<(u16, u16)> {
    let mut r = Reader::new(jpeg);
    if r.u16()? != 0xFFD8 {
        return None;
    }
    loop {
        // A marker is 0xFF, any number of 0xFF fill bytes, then its code.
        if r.u8()? != 0xFF {
            return None;
        }
        let mut code = 0xFF;
        while code == 0xFF {
            code = r.u8()?;
        }
        match code {
            // Markers without a length: TEM and RST0-7.
            0x01 | 0xD0..=0xD7 => continue,
            // The end of the image or its data before any frame header.
            0xD9 | 0xDA => return None,
            _ => {}
        }
        let len = usize::from(r.u16()?).checked_sub(2)?;
        let body = r.take(len)?;
        // SOF0-15, except DHT (C4), JPG (C8) and DAC (CC).
        if matches!(code, 0xC0..=0xCF) && !matches!(code, 0xC4 | 0xC8 | 0xCC) {
            let mut frame = Reader::new(body);
            frame.u8()?; // Sample precision.
            let height = frame.u16()?;
            let width = frame.u16()?;
            return Some((width, height));
        }
    }
}

/// How much of the start of a file the image resources occupy.
#[derive(Debug, PartialEq, Eq)]
pub enum Extent {
    /// The image resources end at this offset.
    Known(usize),
    /// Too little of the file is known to tell; at least this many bytes are needed.
    NeedAtLeast(usize),
}

/// Where the image resources of the PSD/PSB starting with `head` end, so a caller reads no more of
/// the (possibly gigabytes large) file than that. `None` when it is not a PSD/PSB.
pub fn resources_extent(head: &[u8]) -> Option<Extent> {
    if head.len() < FIXED_LEN {
        let magic_ok = head.get(..4).is_none_or(|m| m == b"8BPS");
        return magic_ok.then_some(Extent::NeedAtLeast(FIXED_LEN));
    }
    let mut r = Reader::new(head);
    if r.take(4)? != b"8BPS" || !matches!(r.u16()?, 1 | 2) {
        return None;
    }
    // Reserved (6), channels (2), height (4), width (4), depth (2), mode (2).
    r.take(20)?;
    let color_mode_len = usize::try_from(r.u32()?).ok()?;
    let length_at = r.pos.checked_add(color_mode_len)?;
    let resources_at = length_at.checked_add(4)?;
    let Some(length) = head.get(length_at..resources_at) else {
        return Some(Extent::NeedAtLeast(resources_at));
    };
    let resources_len = usize::try_from(u32::from_be_bytes(length.try_into().ok()?)).ok()?;
    Some(Extent::Known(resources_at.checked_add(resources_len)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal PSD whose only image resource is a 1036 thumbnail around `jpeg`.
    fn sample(jpeg: &[u8]) -> Vec<u8> {
        let mut thumb = Vec::new();
        thumb.extend_from_slice(&1u32.to_be_bytes());
        thumb.extend_from_slice(&[0; 16]);
        thumb.extend_from_slice(&(jpeg.len() as u32).to_be_bytes());
        thumb.extend_from_slice(&[0, 24, 0, 1]);
        thumb.extend_from_slice(jpeg);

        let mut res = Vec::new();
        res.extend_from_slice(b"8BIM");
        res.extend_from_slice(&RESOURCE_THUMBNAIL.to_be_bytes());
        res.extend_from_slice(&[0, 0]); // empty, padded name
        res.extend_from_slice(&(thumb.len() as u32).to_be_bytes());
        res.extend_from_slice(&thumb);
        if thumb.len() % 2 == 1 {
            res.push(0);
        }

        let mut psd = b"8BPS\0\x01".to_vec();
        psd.extend_from_slice(&[0; 20]);
        psd.extend_from_slice(&0u32.to_be_bytes());
        psd.extend_from_slice(&(res.len() as u32).to_be_bytes());
        psd.extend_from_slice(&res);
        psd
    }

    /// The start of a JPEG: SOI, a SOF0 frame header of `width` x `height`, then EOI.
    fn jpeg(width: u16, height: u16) -> Vec<u8> {
        let mut j = vec![0xFF, 0xD8, 0xFF, 0xC0, 0, 17, 8];
        j.extend_from_slice(&height.to_be_bytes());
        j.extend_from_slice(&width.to_be_bytes());
        j.extend_from_slice(&[3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
        j.extend_from_slice(&[0xFF, 0xD9]);
        j
    }

    #[test]
    fn finds_thumbnail() {
        let jpeg = jpeg(160, 90);
        assert_eq!(find_thumbnail_jpeg(&sample(&jpeg)), Some(&jpeg[..]));
    }

    #[test]
    fn psb_uses_the_same_resource_layout_as_psd() {
        let jpeg = jpeg(160, 90);
        let mut psb = sample(&jpeg);
        psb[4..6].copy_from_slice(&2u16.to_be_bytes());
        assert_eq!(find_thumbnail_jpeg(&psb), Some(jpeg.as_slice()));
        assert_eq!(resources_extent(&psb), Some(Extent::Known(psb.len())));
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(find_thumbnail_jpeg(b""), None);
        assert_eq!(find_thumbnail_jpeg(b"8BPS"), None);
        assert_eq!(find_thumbnail_jpeg(b"not a psd at all, just text"), None);
        assert_eq!(find_thumbnail_jpeg(&sample(b"not a jpeg")), None);
    }

    #[test]
    fn rejects_jpegs_that_declare_an_unreasonable_size() {
        for (w, h) in [(0, 90), (160, 0), (2049, 90), (160, 2049), (65535, 65535)] {
            assert_eq!(find_thumbnail_jpeg(&sample(&jpeg(w, h))), None, "{w}x{h}");
        }
        assert!(find_thumbnail_jpeg(&sample(&jpeg(2048, 2048))).is_some());
        // Headers that never reach a frame, or whose lengths lie.
        assert_eq!(find_thumbnail_jpeg(&sample(&[0xFF, 0xD8, 0xFF, 0xD9])), None);
        assert_eq!(find_thumbnail_jpeg(&sample(&[0xFF, 0xD8, 0xFF, 0xDA, 0, 2])), None);
        assert_eq!(find_thumbnail_jpeg(&sample(&[0xFF, 0xD8, 0xFF, 0xE0, 0xFF, 0xFF, 1, 2])), None);
        assert_eq!(find_thumbnail_jpeg(&sample(&[0xFF, 0xD8, 0xFF, 0xC0, 0, 1])), None);
    }

    #[test]
    fn truncation_never_panics_and_never_succeeds_early() {
        let jpeg = jpeg(160, 90);
        let psd = sample(&jpeg);
        // The last byte is padding, so the thumbnail is complete only up to len - 1.
        for n in 0..psd.len() - 1 {
            assert_eq!(find_thumbnail_jpeg(&psd[..n]), None, "prefix of {n} bytes");
        }
    }

    #[test]
    fn corrupted_files_never_panic() {
        let psd = sample(&jpeg(160, 90));
        // A small xorshift generator keeps the test deterministic without a dependency.
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..20_000 {
            let mut copy = psd.clone();
            for _ in 0..=(next() % 4) {
                let at = (next() as usize) % copy.len();
                copy[at] = next() as u8;
            }
            let _ = find_thumbnail_jpeg(&copy);
            let _ = resources_extent(&copy);
        }
    }

    #[test]
    fn extent_of_the_image_resources() {
        let psd = sample(&jpeg(160, 90));
        assert_eq!(resources_extent(&psd), Some(Extent::Known(psd.len())));
        // Enough of the start to read the lengths, then not enough, then nothing.
        assert_eq!(resources_extent(&psd[..FIXED_LEN + 4]), Some(Extent::Known(psd.len())));
        assert_eq!(resources_extent(&psd[..FIXED_LEN]), Some(Extent::NeedAtLeast(FIXED_LEN + 4)));
        assert_eq!(resources_extent(&psd[..10]), Some(Extent::NeedAtLeast(FIXED_LEN)));
        assert_eq!(resources_extent(b""), Some(Extent::NeedAtLeast(FIXED_LEN)));
        assert_eq!(resources_extent(b"nope"), None);
        assert_eq!(resources_extent(&[b'x'; 64]), None);
        // Color mode data in front of the resources moves where their length is.
        let mut with_color_mode = psd[..26].to_vec();
        with_color_mode.extend_from_slice(&768u32.to_be_bytes());
        assert_eq!(resources_extent(&with_color_mode), Some(Extent::NeedAtLeast(26 + 4 + 768 + 4)));
        // Lengths this large are only asked for, never allocated here (the reader caps them).
        let mut huge = psd[..FIXED_LEN].to_vec();
        huge[26..30].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(matches!(resources_extent(&huge), None | Some(Extent::NeedAtLeast(_))));
    }
}
