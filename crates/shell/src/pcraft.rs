//! Bounded reader for the STORE ZIP previews written by PhotoCraft.
//! Only the manifest and thumbnail prefix is read; layer/tile payloads are never decoded.
use crate::psd::Extent;

const MAX_PREFIX: usize = 16 * 1024 * 1024;
const MAX_PNG: usize = 4 * 1024 * 1024;

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at.checked_add(2)?)?.try_into().ok()?))
}
fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

/// Locate the thumbnail in the native writer's manifest-first ZIP layout. Repacked/deflated
/// archives and ZIP64 deliberately fall back to an icon rather than decompressing arbitrary data.
struct Thumbnail {
    start: usize,
    end: usize,
    crc: u32,
}

fn locate(head: &[u8]) -> Option<(Extent, Option<Thumbnail>)> {
    let mut at = 0usize;
    for entry in 0..4 {
        let fixed_end = at.checked_add(30)?;
        if fixed_end > MAX_PREFIX {
            return None;
        }
        if head.len() < fixed_end {
            return Some((Extent::NeedAtLeast(fixed_end), None));
        }
        let h = head.get(at..fixed_end)?;
        if h.get(..4)? != b"PK\x03\x04" || u16_at(h, 6)? & !0x800 != 0 || u16_at(h, 8)? != 0 {
            return None;
        }
        let size = usize::try_from(u32_at(h, 18)?).ok()?;
        if size != usize::try_from(u32_at(h, 22)?).ok()? {
            return None;
        }
        let name_end = fixed_end.checked_add(usize::from(u16_at(h, 26)?))?;
        let data_at = name_end.checked_add(usize::from(u16_at(h, 28)?))?;
        let end = data_at.checked_add(size)?;
        if end > MAX_PREFIX {
            return None;
        }
        let Some(name) = head.get(fixed_end..name_end) else {
            return Some((Extent::NeedAtLeast(data_at), None));
        };
        if entry == 0 && name != b"manifest.json" {
            return None;
        }
        if name == b"thumb.png" {
            if size > MAX_PNG {
                return None;
            }
            return Some((Extent::Known(end), Some(Thumbnail { start: data_at, end, crc: u32_at(h, 14)? })));
        }
        if entry > 0 {
            return None;
        }
        if head.len() < end {
            return Some((Extent::NeedAtLeast(end.checked_add(30)?), None));
        }
        at = end;
    }
    None
}

pub fn thumbnail_extent(head: &[u8]) -> Option<Extent> {
    locate(head).map(|(extent, _)| extent)
}

pub fn find_thumbnail_png(head: &[u8]) -> Option<&[u8]> {
    let (_, Some(Thumbnail { start, end, crc })) = locate(head)? else {
        return None;
    };
    let png = head.get(start..end)?;
    if png.get(..8)? != b"\x89PNG\r\n\x1a\n" || png.get(8..16)? != b"\0\0\0\rIHDR" || crc32(png) != crc {
        return None;
    }
    let width = u32::from_be_bytes(png.get(16..20)?.try_into().ok()?);
    let height = u32::from_be_bytes(png.get(20..24)?.try_into().ok()?);
    if !(1..=2048).contains(&width) || !(1..=2048).contains(&height) {
        return None;
    }
    // The IHDR, including its CRC, must be complete before invoking the native decoder.
    png.get(..33)?;
    Some(png)
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & (0u32.wrapping_sub(crc & 1)));
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(out: &mut Vec<u8>, name: &[u8], data: &[u8]) {
        let mut h = [0u8; 30];
        h[..4].copy_from_slice(b"PK\x03\x04");
        h[14..18].copy_from_slice(&crc32(data).to_le_bytes());
        h[18..22].copy_from_slice(&(data.len() as u32).to_le_bytes());
        h[22..26].copy_from_slice(&(data.len() as u32).to_le_bytes());
        h[26..28].copy_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend(h);
        out.extend(name);
        out.extend(data);
    }
    fn sample(w: u32, h: u32) -> (Vec<u8>, Vec<u8>) {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend(w.to_be_bytes());
        png.extend(h.to_be_bytes());
        png.extend([0u8; 9]);
        let mut zip = Vec::new();
        entry(&mut zip, b"manifest.json", b"{}");
        entry(&mut zip, b"thumb.png", &png);
        (zip, png)
    }
    #[test]
    fn native_thumbnail_does_not_require_tile_payloads_or_directory() {
        let (mut zip, png) = sample(256, 128);
        let end = zip.len();
        zip.extend([42; 1000]);
        assert_eq!(thumbnail_extent(&zip), Some(Extent::Known(end)));
        assert_eq!(find_thumbnail_png(&zip), Some(png.as_slice()));
    }
    #[test]
    fn partial_reads_request_progress_and_truncations_are_rejected() {
        let (zip, _) = sample(256, 128);
        for n in 0..zip.len() {
            assert!(find_thumbnail_png(&zip[..n]).is_none());
            let e = thumbnail_extent(&zip[..n]).unwrap();
            let (Extent::Known(end) | Extent::NeedAtLeast(end)) = e;
            assert!(end > n && end <= MAX_PREFIX);
        }
    }
    #[test]
    fn invalid_dimensions_sizes_flags_crc_and_formats_are_rejected() {
        for (w, h) in [(0, 1), (1, 0), (2049, 1), (1, u32::MAX)] {
            assert!(find_thumbnail_png(&sample(w, h).0).is_none());
        }
        let (zip, _) = sample(1, 1);
        for (at, b) in [(6, 1), (6, 8), (8, 8), (18, 255), (22, 255)] {
            let mut bad = zip.clone();
            bad[at] = b;
            assert!(find_thumbnail_png(&bad).is_none());
        }
        let mut bad = zip.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert!(find_thumbnail_png(&bad).is_none());
        assert!(thumbnail_extent(&[0; 64]).is_none());
        assert_eq!(crc32(b"123456789"), 0xcbf43926);
    }
    #[test]
    fn corrupted_prefixes_never_panic() {
        let (zip, _) = sample(256, 128);
        let mut seed = 27u64;
        for _ in 0..20_000 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let mut bad = zip.clone();
            let at = seed as usize % bad.len();
            bad[at] = (seed >> 8) as u8;
            let _ = thumbnail_extent(&bad);
            let _ = find_thumbnail_png(&bad);
        }
    }
}
