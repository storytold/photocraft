//! Image resources section (`8BIM` resource blocks).

use crate::descriptor::UnicodeString;
use crate::error::{PsdError, Result};
use crate::io::{Reader, WriteExt, read_pascal, write_pascal};

/// Well-known resource ids.
pub mod ids {
    /// Resolution info (`ResolutionInfo` structure).
    pub const RESOLUTION_INFO: u16 = 1005;
    /// Target layer index.
    pub const LAYER_STATE: u16 = 1024;
    /// Layer group ids (one u16 per layer).
    pub const LAYER_GROUP_INFO: u16 = 1026;
    /// Legacy (Photoshop 4) thumbnail, BGR.
    pub const THUMBNAIL_PS4: u16 = 1033;
    /// Thumbnail (JFIF).
    pub const THUMBNAIL: u16 = 1036;
    /// Global lighting angle.
    pub const GLOBAL_ANGLE: u16 = 1037;
    /// ICC profile.
    pub const ICC_PROFILE: u16 = 1039;
    /// Global altitude.
    pub const GLOBAL_ALTITUDE: u16 = 1049;
    /// Version info (has real merged data flag).
    pub const VERSION_INFO: u16 = 1057;
    /// Pixel aspect ratio (`PixelAspectRatio` structure).
    pub const PIXEL_ASPECT_RATIO: u16 = 1064;
    /// Color Sampler points (`ColorSamplers`), the legacy version 1 record.
    pub const COLOR_SAMPLERS_OLD: u16 = 1038;
    /// Color Sampler points (`ColorSamplers`).
    pub const COLOR_SAMPLERS: u16 = 1073;
    /// EXIF data 1.
    pub const EXIF: u16 = 1058;
    /// XMP metadata.
    pub const XMP: u16 = 1060;
}

/// One image resource block.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ImageResource {
    /// Signature, normally `8BIM` (also `MeSa`, `AgHg`, `PHUT`, `DCSR` seen in the wild).
    pub signature: [u8; 4],
    /// Resource id.
    pub id: u16,
    /// Pascal name bytes (usually empty).
    pub name: Vec<u8>,
    /// Resource data (without the even padding).
    pub data: Vec<u8>,
}

/// Fixed-point resolution info (resource 1005).
///
/// `h_res_fixed` and `v_res_fixed` are always pixels per inch (16.16 fixed).
/// `h_res_unit` and `v_res_unit` only say how to display that number
/// (1 = pixels/inch, 2 = pixels/cm) and do not rescale the stored value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ResolutionInfo {
    /// Horizontal resolution, 16.16 fixed point, always pixels per inch.
    pub h_res_fixed: u32,
    /// Display unit for the horizontal resolution (1 = px/in, 2 = px/cm).
    pub h_res_unit: u16,
    /// Ruler unit for width (1 in, 2 cm, 3 pt, 4 pica, 5 column).
    pub width_unit: u16,
    /// Vertical resolution, 16.16 fixed point, always pixels per inch.
    pub v_res_fixed: u32,
    /// Display unit for the vertical resolution (1 = px/in, 2 = px/cm).
    pub v_res_unit: u16,
    /// Ruler unit for height (1 in, 2 cm, 3 pt, 4 pica, 5 column).
    pub height_unit: u16,
}

impl ResolutionInfo {
    /// Resolution info for `dpi` pixels per inch in both directions.
    pub fn from_dpi(dpi: f64) -> Self {
        let f = (dpi * 65536.0).round().clamp(0.0, u32::MAX as f64) as u32;
        ResolutionInfo { h_res_fixed: f, h_res_unit: 1, width_unit: 1, v_res_fixed: f, v_res_unit: 1, height_unit: 1 }
    }
    /// Horizontal resolution in pixels per inch.
    pub fn h_res(&self) -> f64 {
        f64::from(self.h_res_fixed) / 65536.0
    }
    /// Vertical resolution in pixels per inch.
    pub fn v_res(&self) -> f64 {
        f64::from(self.v_res_fixed) / 65536.0
    }
    /// Parses 16 bytes.
    pub fn from_bytes(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b);
        Ok(ResolutionInfo {
            h_res_fixed: r.u32()?,
            h_res_unit: r.u16()?,
            width_unit: r.u16()?,
            v_res_fixed: r.u32()?,
            v_res_unit: r.u16()?,
            height_unit: r.u16()?,
        })
    }
    /// Serializes to 16 bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(16);
        v.put_u32(self.h_res_fixed);
        v.put_u16(self.h_res_unit);
        v.put_u16(self.width_unit);
        v.put_u32(self.v_res_fixed);
        v.put_u16(self.v_res_unit);
        v.put_u16(self.height_unit);
        v
    }
}

/// Pixel aspect ratio (resource 1064): the width of a pixel over its height.
///
/// Version 2 is current; version 1 files stored NTSC and PAL ratios about 5 % off, but the
/// number is still the ratio the file asked for.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PixelAspectRatio {
    /// Structure version (1 or 2).
    pub version: u32,
    /// Pixel width / height (1 = square).
    pub ratio: f64,
}

impl PixelAspectRatio {
    /// A version 2 record of `ratio`.
    pub fn new(ratio: f64) -> Self {
        PixelAspectRatio { version: 2, ratio }
    }
    /// Parses 12 bytes.
    pub fn from_bytes(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b);
        Ok(PixelAspectRatio { version: r.u32()?, ratio: r.f64()? })
    }
    /// Serializes to 12 bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(12);
        v.put_u32(self.version);
        v.put_f64(self.ratio);
        v
    }
}

/// Color Sampler points (resources 1073 and, legacy, 1038): positions in document pixels, `[x, y]`.
///
/// Photoshop writes both. 1073 is version 3: per point a 2-byte record length (14), the vertical
/// then horizontal position as 32-bit floats, a colour space (−1) and a 2-byte field (8).
/// 1038 is version 1: the vertical then horizontal position in 1/32-pixel fixed point and the
/// colour space. Photoshop stores the centre of the sampled pixel (`x + 0.5`); version 2 (floats
/// and a depth, no record length) is read too.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ColorSamplers {
    /// Positions `[x, y]` in document pixels, in sampler order (#1, #2, …).
    pub points: Vec<[f64; 2]>,
}

impl ColorSamplers {
    /// Parses resource 1073 (versions 1–3) or 1038 (version 1).
    pub fn from_bytes(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b);
        let version = r.u32()?;
        let count = r.u32()?;
        let min_record = match version {
            1 => 10,
            2 => 12,
            3 => 14,
            v => return Err(PsdError::Unsupported(format!("color samplers version {v}"))),
        };
        r.check_count(u64::from(count), min_record)?;
        let mut points = Vec::with_capacity(usize::try_from(count).unwrap_or(0));
        for _ in 0..count {
            let float = |r: &mut Reader<'_>| r.u32().map(|v| f64::from(f32::from_bits(v)));
            let (y, x) = match version {
                1 => {
                    let (y, x) = (r.i32()?, r.i32()?);
                    r.skip(2)?;
                    (f64::from(y) / 32.0, f64::from(x) / 32.0)
                }
                2 => {
                    let (y, x) = (float(&mut r)?, float(&mut r)?);
                    r.skip(4)?;
                    (y, x)
                }
                _ => {
                    let size = usize::from(r.u16()?);
                    if size < 14 {
                        return Err(PsdError::Invalid(format!("color sampler record of {size} bytes")));
                    }
                    let (y, x) = (float(&mut r)?, float(&mut r)?);
                    r.skip(size - 10)?;
                    (y, x)
                }
            };
            points.push([x, y]);
        }
        Ok(ColorSamplers { points })
    }

    /// Resource 1073 as Photoshop writes it (version 3).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(8 + 14 * self.points.len());
        v.put_u32(3);
        v.put_u32(u32::try_from(self.points.len()).unwrap_or(u32::MAX));
        for &[x, y] in &self.points {
            v.put_u16(14);
            v.put_u32((y as f32).to_bits());
            v.put_u32((x as f32).to_bits());
            v.put_i16(-1);
            v.put_u16(8);
        }
        v
    }

    /// The legacy resource 1038 (version 1), which Photoshop writes alongside 1073.
    pub fn to_legacy_bytes(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(8 + 10 * self.points.len());
        v.put_u32(1);
        v.put_u32(u32::try_from(self.points.len()).unwrap_or(u32::MAX));
        for &[x, y] in &self.points {
            // `as` saturates (and NaN is 0): an out-of-range point can't wrap.
            v.put_i32((y * 32.0).round() as i32);
            v.put_i32((x * 32.0).round() as i32);
            v.put_i16(-1);
        }
        v
    }
}

/// Version info (resource 1057).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct VersionInfo {
    /// Version (1).
    pub version: u32,
    /// `false` when the merged image is a placeholder.
    pub has_real_merged_data: bool,
    /// Writer application name.
    pub writer: UnicodeString,
    /// Reader application name.
    pub reader: UnicodeString,
    /// File version.
    pub file_version: u32,
}

impl VersionInfo {
    /// Parses resource data.
    pub fn from_bytes(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b);
        Ok(VersionInfo {
            version: r.u32()?,
            has_real_merged_data: r.u8()? != 0,
            writer: UnicodeString::read(&mut r)?,
            reader: UnicodeString::read(&mut r)?,
            file_version: r.u32()?,
        })
    }
    /// Serializes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = Vec::new();
        v.put_u32(self.version);
        v.put_u8(u8::from(self.has_real_merged_data));
        self.writer.write(&mut v);
        self.reader.write(&mut v);
        v.put_u32(self.file_version);
        v
    }
}

/// Typed view of a known resource.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ResourceData<'a> {
    /// 1005
    ResolutionInfo(ResolutionInfo),
    /// 1024: index of the target layer.
    LayerState(u16),
    /// 1026: group id per layer.
    LayerGroupInfo(Vec<u16>),
    /// 1036 / 1033: raw thumbnail resource (28-byte header + JFIF/raw data).
    Thumbnail(&'a [u8]),
    /// 1037: global lighting angle in degrees.
    GlobalAngle(i32),
    /// 1039: ICC profile bytes.
    IccProfile(&'a [u8]),
    /// 1049: global altitude.
    GlobalAltitude(i32),
    /// 1057
    VersionInfo(VersionInfo),
    /// 1058: EXIF bytes.
    Exif(&'a [u8]),
    /// 1060: XMP packet (UTF-8).
    Xmp(&'a [u8]),
}

impl ImageResource {
    /// A new `8BIM` resource with an empty name.
    pub fn new(id: u16, data: Vec<u8>) -> Self {
        ImageResource { signature: *b"8BIM", id, name: Vec::new(), data }
    }

    /// Typed parse of well-known resources. Returns `None` for unknown ids,
    /// `Some(Err)` if the data is malformed.
    pub fn parsed(&self) -> Option<Result<ResourceData<'_>>> {
        let d = &self.data[..];
        let be_i32 = |d: &[u8]| -> Result<i32> { Reader::new(d).i32() };
        Some(match self.id {
            ids::RESOLUTION_INFO => ResolutionInfo::from_bytes(d).map(ResourceData::ResolutionInfo),
            ids::LAYER_STATE => Reader::new(d).u16().map(ResourceData::LayerState),
            ids::LAYER_GROUP_INFO => {
                if !d.len().is_multiple_of(2) {
                    Err(PsdError::invalid("layer group info has odd length"))
                } else {
                    Ok(ResourceData::LayerGroupInfo(d.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes([c[0], c[1]])).collect()))
                }
            }
            ids::THUMBNAIL | ids::THUMBNAIL_PS4 => Ok(ResourceData::Thumbnail(d)),
            ids::GLOBAL_ANGLE => be_i32(d).map(ResourceData::GlobalAngle),
            ids::ICC_PROFILE => Ok(ResourceData::IccProfile(d)),
            ids::GLOBAL_ALTITUDE => be_i32(d).map(ResourceData::GlobalAltitude),
            ids::VERSION_INFO => VersionInfo::from_bytes(d).map(ResourceData::VersionInfo),
            ids::EXIF => Ok(ResourceData::Exif(d)),
            ids::XMP => Ok(ResourceData::Xmp(d)),
            _ => return None,
        })
    }

    /// XMP packet as a string (resource 1060).
    pub fn xmp_str(&self) -> Option<&str> {
        (self.id == ids::XMP).then(|| std::str::from_utf8(&self.data).ok()).flatten()
    }

    pub(crate) fn read(r: &mut Reader<'_>) -> Result<Self> {
        let signature = r.array::<4>()?;
        if !matches!(&signature, b"8BIM" | b"MeSa" | b"AgHg" | b"PHUT" | b"DCSR") {
            return Err(PsdError::InvalidSignature { expected: "8BIM resource", found: signature });
        }
        let id = r.u16()?;
        let name = read_pascal(r, 2)?;
        let len = r.u32()?;
        let data = r.bytes_u64(u64::from(len))?.to_vec();
        if len % 2 == 1 {
            r.skip(1)?;
        }
        Ok(ImageResource { signature, id, name, data })
    }

    pub(crate) fn write(&self, out: &mut Vec<u8>) -> Result<()> {
        out.put(&self.signature);
        out.put_u16(self.id);
        write_pascal(out, &self.name, 2);
        out.put_len(self.data.len() as u64, false)?;
        out.put(&self.data);
        if self.data.len() % 2 == 1 {
            out.put_u8(0);
        }
        Ok(())
    }
}

pub(crate) fn read_section(r: &mut Reader<'_>) -> Result<Vec<ImageResource>> {
    let len = r.u32()?;
    let mut s = r.sub(u64::from(len))?;
    let mut v = Vec::new();
    while !s.is_empty() {
        v.push(ImageResource::read(&mut s)?);
    }
    Ok(v)
}

/// Serialized size of the resource section [`write_section`] writes, its length field included.
pub(crate) fn section_len(res: &[ImageResource]) -> u64 {
    res.iter().fold(4u64, |n, r| {
        let name = 1 + r.name.len().min(255);
        let name = name + name % 2;
        let data = r.data.len().saturating_add(r.data.len() % 2);
        n.saturating_add((4 + 2 + name + 4) as u64).saturating_add(data as u64)
    })
}

pub(crate) fn write_section(out: &mut Vec<u8>, res: &[ImageResource]) -> Result<()> {
    let at = out.begin_len(false);
    for r in res {
        r.write(out)?;
    }
    out.end_len(at, false)
}

/// Builds the data for a 1057 version info resource, with the writer and reader names Photoshop
/// 2026 writes ("Adobe Photoshop", "Adobe Photoshop 2026"), without a NUL terminator.
pub fn version_info_resource(has_real_merged_data: bool) -> ImageResource {
    ImageResource::new(
        ids::VERSION_INFO,
        VersionInfo {
            version: 1,
            has_real_merged_data,
            writer: UnicodeString::new("Adobe Photoshop"),
            reader: UnicodeString::new("Adobe Photoshop 2026"),
            file_version: 1,
        }
        .to_bytes(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(res: &[ImageResource]) -> Vec<u8> {
        let mut out = Vec::new();
        write_section(&mut out, res).unwrap();
        let back = read_section(&mut Reader::new(&out)).unwrap();
        assert_eq!(back, res);
        out
    }

    /// Resources 1073 and 1038 as Photoshop 2026 wrote them for samplers clicked on pixels
    /// (10, 20), (150, 100) and (299, 199) of a 300 × 200 document.
    const PS_1073: &str = "00000003 00000003 000e 41a40000 41280000 ffff 0008 000e 42c90000 43168000 ffff 0008 000e 43478000 4395c000 ffff 0008";
    const PS_1038: &str = "00000001 00000003 00000290 00000150 ffff 00000c90 000012d0 ffff 000018f0 00002570 ffff";

    fn hex(s: &str) -> Vec<u8> {
        let s: String = s.split_whitespace().collect();
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn color_samplers_read_and_write_photoshops_bytes() {
        let want = vec![[10.5, 20.5], [150.5, 100.5], [299.5, 199.5]];
        let new = ColorSamplers::from_bytes(&hex(PS_1073)).unwrap();
        assert_eq!(new.points, want);
        assert_eq!(ColorSamplers::from_bytes(&hex(PS_1038)).unwrap().points, want);
        assert_eq!(new.to_bytes(), hex(PS_1073));
        assert_eq!(new.to_legacy_bytes(), hex(PS_1038));
        // Version 2: floats and a depth, no record length.
        let v2 = hex("00000002 00000001 41a40000 41280000 ffff 0008");
        assert_eq!(ColorSamplers::from_bytes(&v2).unwrap().points, [[10.5, 20.5]]);
    }

    #[test]
    fn damaged_color_samplers_are_errors_not_panics() {
        for bad in [
            "",
            "00000003",
            "00000003 ffffffff",
            "00000009 00000000",
            "00000003 00000001 0004 41a40000 41280000 ffff 0008",
            "00000001 00000002 00000290 00000150 ffff",
        ] {
            assert!(ColorSamplers::from_bytes(&hex(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn pixel_aspect_ratio_round_trips_and_short_data_is_an_error() {
        let par = PixelAspectRatio::new(0.9);
        let bytes = par.to_bytes();
        assert_eq!(bytes.len(), 12);
        assert_eq!(&bytes[..4], &[0, 0, 0, 2]);
        assert_eq!(PixelAspectRatio::from_bytes(&bytes).unwrap(), par);
        assert!(PixelAspectRatio::from_bytes(&bytes[..11]).is_err());
        assert!(PixelAspectRatio::from_bytes(&[]).is_err());
    }

    #[test]
    fn empty_section() {
        assert_eq!(roundtrip(&[]), vec![0, 0, 0, 0]);
    }

    #[test]
    fn odd_data_is_padded() {
        let out = roundtrip(&[ImageResource::new(1000, vec![1, 2, 3])]);
        // len(4) + sig(4) + id(2) + name(2) + size(4) + data(3) + pad(1)
        assert_eq!(out.len(), 4 + 4 + 2 + 2 + 4 + 4);
        assert_eq!(*out.last().unwrap(), 0);
    }

    #[test]
    fn named_resources() {
        let mut r = ImageResource::new(2000, vec![9; 4]);
        r.name = b"path".to_vec();
        let mut r2 = ImageResource::new(2001, vec![]);
        r2.name = b"abc".to_vec();
        r2.signature = *b"MeSa";
        roundtrip(&[r, r2]);
    }

    #[test]
    fn bad_signature() {
        let mut out = Vec::new();
        write_section(&mut out, &[ImageResource::new(1, vec![])]).unwrap();
        out[4] = b'X';
        assert!(read_section(&mut Reader::new(&out)).is_err());
    }

    #[test]
    fn truncation_errors() {
        let mut out = Vec::new();
        write_section(&mut out, &[ImageResource::new(1005, ResolutionInfo::from_dpi(72.0).to_bytes())]).unwrap();
        for cut in 0..out.len() {
            assert!(read_section(&mut Reader::new(&out[..cut])).is_err());
        }
    }

    #[test]
    fn typed_resolution() {
        let ri = ResolutionInfo::from_dpi(300.0);
        let r = ImageResource::new(ids::RESOLUTION_INFO, ri.to_bytes());
        match r.parsed() {
            Some(Ok(ResourceData::ResolutionInfo(p))) => {
                assert_eq!(p, ri);
                assert!((p.h_res() - 300.0).abs() < 1e-9);
                assert!((p.v_res() - 300.0).abs() < 1e-9);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn typed_misc() {
        let r = ImageResource::new(ids::GLOBAL_ANGLE, 120i32.to_be_bytes().to_vec());
        assert_eq!(r.parsed(), Some(Ok(ResourceData::GlobalAngle(120))));
        let r = ImageResource::new(ids::GLOBAL_ALTITUDE, 30i32.to_be_bytes().to_vec());
        assert_eq!(r.parsed(), Some(Ok(ResourceData::GlobalAltitude(30))));
        let r = ImageResource::new(ids::LAYER_STATE, vec![0, 2]);
        assert_eq!(r.parsed(), Some(Ok(ResourceData::LayerState(2))));
        let r = ImageResource::new(ids::LAYER_GROUP_INFO, vec![0, 1, 0, 2]);
        assert_eq!(r.parsed(), Some(Ok(ResourceData::LayerGroupInfo(vec![1, 2]))));
        let r = ImageResource::new(ids::LAYER_GROUP_INFO, vec![0]);
        assert!(matches!(r.parsed(), Some(Err(_))));
        let r = ImageResource::new(ids::ICC_PROFILE, vec![1]);
        assert_eq!(r.parsed(), Some(Ok(ResourceData::IccProfile(&[1]))));
        let r = ImageResource::new(ids::EXIF, vec![2]);
        assert_eq!(r.parsed(), Some(Ok(ResourceData::Exif(&[2]))));
        let r = ImageResource::new(ids::THUMBNAIL, vec![3]);
        assert_eq!(r.parsed(), Some(Ok(ResourceData::Thumbnail(&[3]))));
        let r = ImageResource::new(ids::XMP, b"<x/>".to_vec());
        assert_eq!(r.xmp_str(), Some("<x/>"));
        assert_eq!(ImageResource::new(4242, vec![]).parsed(), None);
    }

    #[test]
    fn version_info_roundtrip() {
        let r = version_info_resource(false);
        match r.parsed() {
            Some(Ok(ResourceData::VersionInfo(v))) => {
                assert!(!v.has_real_merged_data);
                assert_eq!(v.to_bytes(), r.data);
            }
            other => panic!("{other:?}"),
        }
    }

    /// Byte for byte what Photoshop 2026 writes (every Photoshop-authored oracle in the corpus):
    /// u32 count + UTF-16BE, no NUL terminator.
    #[test]
    fn version_info_matches_photoshop_bytes() {
        let mut want = vec![0, 0, 0, 1, 1];
        for s in ["Adobe Photoshop", "Adobe Photoshop 2026"] {
            want.extend_from_slice(&(s.len() as u32).to_be_bytes());
            want.extend(s.encode_utf16().flat_map(u16::to_be_bytes));
        }
        want.extend_from_slice(&[0, 0, 0, 1]);
        assert_eq!(version_info_resource(true).data, want);
    }
}
