//! Bound provider output before decoding; allow same-aspect output to fit the original canvas.
pub(crate) const MAX_PIXELS: u64 = 16_777_216;
#[cfg(any(test, not(target_arch = "wasm32")))]
pub(crate) const MAX_FILE_BYTES: usize = 64 * 1024 * 1024;

#[cfg(any(test, not(target_arch = "wasm32")))]
pub(crate) fn result_dimensions(bytes: &[u8], canvas_width: u32, canvas_height: u32) -> Result<(u32, u32), &'static str> {
    if bytes.starts_with(b"\xff\xd8\xff") {
        let error = "AI result must be a PNG or JPEG under 64 MiB and 16 MP with the canvas aspect ratio; no layer was added";
        if bytes.len() > MAX_FILE_BYTES {
            return Err(error);
        }
        let (width, height) = jpeg_dimensions(bytes).ok_or(error)?;
        check_size(width, height, canvas_width, canvas_height).ok_or(error)?;
        Ok((width, height))
    } else {
        dimensions(bytes, canvas_width, canvas_height)
    }
}

/// Read SOF before allocating pixels. Segment lengths, marker fills and truncation are hostile.
#[cfg(any(test, not(target_arch = "wasm32")))]
fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut offset = 2usize;
    while offset < bytes.len() {
        if *bytes.get(offset)? != 0xff {
            return None;
        }
        while bytes.get(offset) == Some(&0xff) {
            offset = offset.checked_add(1)?;
        }
        let marker = *bytes.get(offset)?;
        offset = offset.checked_add(1)?;
        if matches!(marker, 0x00 | 0xd8..=0xda) {
            return None;
        }
        if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
            continue;
        }
        let length = usize::from(u16::from_be_bytes(bytes.get(offset..offset.checked_add(2)?)?.try_into().ok()?));
        if length < 2 {
            return None;
        }
        let segment = bytes.get(offset..offset.checked_add(length)?)?;
        if matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) {
            let height = u32::from(u16::from_be_bytes(segment.get(3..5)?.try_into().ok()?));
            let width = u32::from(u16::from_be_bytes(segment.get(5..7)?.try_into().ok()?));
            return Some((width, height));
        }
        offset = offset.checked_add(length)?;
    }
    None
}

#[cfg(any(test, not(target_arch = "wasm32")))]
fn check_size(width: u32, height: u32, canvas_width: u32, canvas_height: u32) -> Option<()> {
    if width == 0 || height == 0 || canvas_width == 0 || canvas_height == 0 || u64::from(width) * u64::from(height) > MAX_PIXELS {
        return None;
    }
    let a = u64::from(width) * u64::from(canvas_height);
    let b = u64::from(height) * u64::from(canvas_width);
    (a.abs_diff(b).saturating_mul(100) <= a.max(b)).then_some(())
}

#[cfg(any(test, not(target_arch = "wasm32")))]
pub(crate) fn dimensions(bytes: &[u8], canvas_width: u32, canvas_height: u32) -> Result<(u32, u32), &'static str> {
    let error = "AI result must be a PNG under 64 MiB and 16 MP with the canvas aspect ratio; no layer was added";
    let dim = |offset: usize| bytes.get(offset..offset + 4).and_then(|s| <[u8; 4]>::try_from(s).ok()).map(u32::from_be_bytes);
    if bytes.len() > MAX_FILE_BYTES || bytes.get(..8) != Some(b"\x89PNG\r\n\x1a\n") || bytes.get(12..16) != Some(b"IHDR") {
        return Err(error);
    }
    let (width, height) = (dim(16).ok_or(error)?, dim(20).ok_or(error)?);
    // Model dimensions may round by a pixel, but a materially different crop must not stretch.
    check_size(width, height, canvas_width, canvas_height).ok_or(error)?;
    Ok((width, height))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn header(w: u32, h: u32) -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        bytes.extend_from_slice(&w.to_be_bytes());
        bytes.extend_from_slice(&h.to_be_bytes());
        bytes
    }
    #[test]
    fn accepts_original_dimensions() {
        assert_eq!(dimensions(&header(1024, 1024), 1024, 1024), Ok((1024, 1024)));
    }
    #[test]
    fn accepts_real_codex_size_and_same_aspect_resize() {
        assert_eq!(dimensions(&header(1254, 1254), 1024, 1024), Ok((1254, 1254)));
        assert_eq!(dimensions(&header(1536, 1024), 1200, 800), Ok((1536, 1024)));
        assert!(dimensions(&header(1536, 1023), 1200, 800).is_ok());
    }
    #[test]
    fn refuses_changed_aspect_ratio() {
        assert!(dimensions(&header(1024, 1536), 1024, 1024).is_err());
    }
    #[test]
    fn refuses_malformed_zero_and_oversized_results() {
        assert!(dimensions(b"bad", 8, 6).is_err());
        for (w, h) in [(0, 6), (8, 0), (u32::MAX, u32::MAX), (5000, 5000)] {
            assert!(dimensions(&header(w, h), 8, 6).is_err());
        }
        assert!(dimensions(&header(8, 6), 0, 0).is_err());
    }

    #[test]
    fn jpeg_headers_are_bounded_and_checked_before_decode() {
        let header = |w: u16, h: u16| {
            let mut bytes = b"\xff\xd8\xff\xe0\0\x04xx\xff\xc0\0\x08\x08".to_vec();
            bytes.extend_from_slice(&h.to_be_bytes());
            bytes.extend_from_slice(&w.to_be_bytes());
            bytes.push(3);
            bytes
        };
        assert_eq!(result_dimensions(&header(800, 600), 8, 6), Ok((800, 600)));
        assert!(result_dimensions(&header(800, 600), 8, 8).is_err());
        assert!(result_dimensions(&header(0, 600), 8, 6).is_err());
        assert!(result_dimensions(&header(u16::MAX, u16::MAX), 8, 6).is_err());
        for bytes in [b"\xff\xd8\xff\xe0\0\x01".as_slice(), b"\xff\xd8\xff\xff", b"\xff\xd8\xff\xda\0\x08", b"\xff\xd8\xff\xe0\xff\xff"] {
            assert!(result_dimensions(bytes, 8, 6).is_err());
        }
    }
}
