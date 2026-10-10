//! Olympus ORF with uncompressed sensor data (older Four Thirds bodies such as
//! the E-1 and E-400, and padded 12-bit storage on the E-300 / E-500 / E-330).
//!
//! The container is TIFF with an `IIRO` / `IIRS` / `MMOR` header; the sensor
//! data is IFD0's strips, either 16-bit samples whose top ValidBits bits are
//! significant or ten little-endian 12-bit samples followed by one zero byte
//! per 16-byte block. The latter layout is described in LightCraft PR #651
//! and independently checked here against CC0 camera files. The CFA
//! comes from the EXIF CFAPattern. Levels come from the publicly documented
//! Olympus maker-note ImageProcessing tags (ExifTool's Olympus table):
//! WB_RBLevels (0x0100), WB_GLevel (0x011F), BlackLevel2 (0x0600), ValidBits
//! (0x0611) and CropLeft / CropTop / CropWidth / CropHeight (0x0612–0x0615).
//!
//! Olympus' own compressed sensor data is not decoded.

use crate::error::{RawError, Result};
use crate::sensor::{BlackLevels, JpegLayout, Plane, Rect, Sensor, read_plane};
use crate::tiff::{Ifd, Tiff, tag};
use crate::tiffep::{cfa, rggb_by_position};
use crate::{Limits, RawFormat};

const CAMERA_SETTINGS: u16 = 0x2020;
const PREVIEW_IMAGE_START: u16 = 0x0101;
const PREVIEW_IMAGE_LENGTH: u16 = 0x0102;
const IMAGE_PROCESSING: u16 = 0x2040;
const WB_RB_LEVELS: u16 = 0x0100;
const WB_G_LEVEL: u16 = 0x011F;
const BLACK_LEVEL_2: u16 = 0x0600;
const VALID_BITS: u16 = 0x0611;
const CROP: [u16; 4] = [0x0612, 0x0613, 0x0614, 0x0615];

/// A sub-IFD of the Olympus maker note (`sub`: 0x2020 CameraSettings, 0x2040
/// ImageProcessing…), with the TIFF view its offsets are relative to and that
/// view's absolute position in the file. New-style notes (`OLYMPUS\0` + byte
/// order) are relative to the note; old-style ones (`OLYMP\0`) to the file.
fn maker_subifd<'a>(t: &Tiff<'a>, ifds: &[Ifd], sub: u16) -> Option<(Tiff<'a>, Ifd, usize)> {
    let e = ifds.iter().find_map(|i| i.get(tag::MAKER_NOTE).copied())?;
    let head = t.bytes(e.at, 12)?;
    let (view, main, base) = if head.starts_with(b"OLYMPUS\0") {
        let le = match head.get(8..10)? {
            b"II" => true,
            b"MM" => false,
            _ => return None,
        };
        let inner = Tiff { data: t.data.get(e.at..)?, le, first_ifd: 12 };
        let ifd = inner.ifd_at(12, 0)?;
        (inner, ifd, e.at)
    } else if head.starts_with(b"OLYMP\0") {
        (*t, t.ifd_at(e.at.checked_add(8)?, 0)?, 0)
    } else {
        return None;
    };
    let ip = main.get(sub)?;
    let at = match ip.typ {
        4 | 13 => view.uint(ip, 0)? as usize,
        _ => ip.at,
    };
    let ifd = view.ifd_at(at, 0)?;
    Some((view, ifd, base))
}

/// Absolute offset and length of the camera's preview JPEG (maker note
/// CameraSettings PreviewImageStart / PreviewImageLength, 0x0101 / 0x0102).
pub(crate) fn preview_range(t: &Tiff) -> Option<(usize, usize)> {
    let (view, cs, base) = maker_subifd(t, &t.all_ifds(), CAMERA_SETTINGS)?;
    let start = view.tag_uint(&cs, PREVIEW_IMAGE_START)? as usize;
    let len = view.tag_uint(&cs, PREVIEW_IMAGE_LENGTH)? as usize;
    Some((base.checked_add(start)?, len))
}

pub(crate) fn decode(t: &Tiff, limits: &Limits) -> Result<Sensor> {
    let ifds = t.all_ifds();
    let raw = t.ifd_at(t.first_ifd, 0).ok_or_else(|| RawError::malformed("ORF has no IFD0"))?;
    let width = u64::from(t.tag_uint(&raw, tag::IMAGE_WIDTH).unwrap_or(0));
    let height = u64::from(t.tag_uint(&raw, tag::IMAGE_LENGTH).unwrap_or(0));
    // Every ORF, compressed or not, declares its strip byte counts. Missing or unreadable counts
    // mean IFD0 is cut short (a partial download or a damaged file), not the compressed variant:
    // an empty sum must not be mistaken for "fewer than 16 bits per sample".
    let counts = t.tag_uints(&raw, tag::STRIP_BYTE_COUNTS);
    if counts.is_empty() {
        return Err(RawError::malformed("ORF IFD0 has no readable strip byte counts"));
    }
    let stored: u64 = counts.iter().map(|&c| u64::from(c)).sum();
    if t.tag_uint(&raw, tag::COMPRESSION).unwrap_or(1) != 1 {
        return Err(RawError::unsupported("Olympus compressed (or packed) ORF is not decoded"));
    }
    let padded_len = (width / 10).checked_mul(height).and_then(|blocks| blocks.checked_mul(16));
    let plane = if t.le && width > 0 && width.is_multiple_of(10) && counts.len() == 1 && padded_len == Some(stored) {
        read_padded12(t, &raw, limits, width, height, stored)?
    } else {
        if stored < width.saturating_mul(height).saturating_mul(2) {
            return Err(RawError::unsupported("Olympus compressed (or packed) ORF is not decoded"));
        }
        read_plane(t, &raw, limits, JpegLayout::Flat)?
    };
    if plane.samples != 1 {
        return Err(RawError::unsupported(format!("ORF with {} samples per pixel", plane.samples)));
    }
    let cfa = cfa(t, &raw, &ifds).ok_or_else(|| RawError::unsupported("ORF: CFA pattern not recorded"))?;
    if !cfa.is_bayer() {
        return Err(RawError::unsupported("ORF: non-Bayer CFA"));
    }
    let full = Rect::new(0, 0, plane.width, plane.height);
    let mut warnings = Vec::new();
    let ip = maker_subifd(t, &ifds, IMAGE_PROCESSING);
    let floats = |tg: u16| ip.as_ref().map(|(v, i, _)| v.tag_floats(i, tg)).unwrap_or_default();

    let black = match rggb_by_position(&cfa, &floats(BLACK_LEVEL_2)) {
        Some(v) => BlackLevels { rows: 2, cols: 2, values: v.to_vec(), delta_h: Vec::new(), delta_v: Vec::new() },
        None => {
            warnings.push("ORF: black level not recorded; assumed 0".to_string());
            BlackLevels::uniform(0.0)
        }
    };
    let mut data = plane.data;
    let white = match floats(VALID_BITS).first() {
        Some(&b) if (8.0..=16.0).contains(&b) => {
            let bits = b as u32;
            let top = (1u32 << bits) - 1;
            // Observed: the E-1 / E-400 store the valid bits left-justified in
            // the 16-bit container (samples are multiples of 16 up to 65520).
            if data.iter().any(|&v| u32::from(v) > top) {
                let shift = 16 - bits;
                data.iter_mut().for_each(|v| *v >>= shift);
            }
            top as f32
        }
        _ => crate::cr2::clip_level(&data, plane.bits),
    };
    let g = floats(WB_G_LEVEL).first().copied().filter(|g| *g > 0.0).unwrap_or(256.0);
    let camera_wb = match floats(WB_RB_LEVELS).as_slice() {
        [r, b, ..] => Some([r / g, 1.0, b / g]).filter(|m| m.iter().all(|v| (0.25..8.0).contains(v))),
        _ => None,
    };
    let crop = match CROP.map(|tg| floats(tg).first().copied()) {
        [Some(x), Some(y), Some(w), Some(h)] if [x, y, w, h].iter().all(|v| (0.0..1e6).contains(v)) => {
            let r = Rect::new(x as usize, y as usize, w as usize, h as usize).intersect(&full);
            if r.is_empty() { full } else { r }
        }
        _ => full,
    };
    Ok(Sensor {
        format: RawFormat::Orf,
        make: ifds.iter().find_map(|i| t.tag_ascii(i, tag::MAKE)),
        model: ifds.iter().find_map(|i| t.tag_ascii(i, tag::MODEL)),
        width: plane.width,
        height: plane.height,
        samples: 1,
        data,
        cfa: Some(cfa),
        linearization: None,
        black,
        white: [white; 3],
        active: full,
        crop,
        color: Default::default(),
        camera_wb,
        orientation: crate::tiff::orientation(t.tag_uint(&raw, tag::ORIENTATION)),
        baseline_exposure: 0.0,
        gain_maps: Vec::new(),
        tone_curve: Vec::new(),
        warnings,
    })
}

/// A single strip of ten 12-bit samples per 16-byte block. This is a storage
/// signature, not a camera-model guess; other packed layouts stay unsupported.
fn read_padded12(t: &Tiff, raw: &Ifd, limits: &Limits, width: u64, height: u64, stored: u64) -> Result<Plane> {
    let offsets = t.tag_uints(raw, tag::STRIP_OFFSETS);
    let [offset] = offsets.as_slice() else {
        return Err(RawError::unsupported("ORF padded 12-bit storage requires one strip"));
    };
    if !matches!(t.tag_uint(raw, tag::BITS_PER_SAMPLE), Some(12 | 16))
        || t.tag_uint(raw, tag::SAMPLES_PER_PIXEL).unwrap_or(1) != 1
        || t.tag_uint(raw, tag::SAMPLE_FORMAT).unwrap_or(1) != 1
    {
        return Err(RawError::unsupported("ORF padded storage requires unsigned 12-bit samples"));
    }
    limits.check(width, height, 2)?;
    let len = usize::try_from(stored).map_err(|_| RawError::malformed("ORF strip size overflow"))?;
    let bytes = t.bytes(*offset as usize, len).ok_or_else(|| RawError::malformed("ORF padded strip truncated"))?;
    let mut data = Vec::with_capacity(width as usize * height as usize);
    for block in bytes.as_chunks::<16>().0 {
        let [samples @ .., pad] = block;
        if *pad != 0 {
            return Err(RawError::malformed("ORF padded strip has nonzero padding"));
        }
        for &[a, b, c] in samples.as_chunks::<3>().0 {
            data.push(u16::from(a) | (u16::from(b & 0x0f) << 8));
            data.push(u16::from(b >> 4) | (u16::from(c) << 4));
        }
    }
    Ok(Plane { width: width as usize, height: height as usize, samples: 1, bits: 12, data })
}
