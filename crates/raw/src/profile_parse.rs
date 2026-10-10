//! Public DNG 1.7.1 camera-profile tags; parsing and usage-policy validation.
use super::{CameraProfile, HueSatMap};
use crate::{
    Calibration, RawError, color,
    tiff::{Ifd, Tiff},
};

const IDENTITY: [[f64; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
pub const MAX_BYTES: usize = 32 * 1024 * 1024;

fn floats(t: &Tiff<'_>, ifd: &Ifd, tag: u16, max: usize) -> Result<Vec<f64>, RawError> {
    let e = ifd.get(tag).ok_or_else(|| RawError::malformed(format!("DCP missing tag {tag}")))?;
    if e.count as usize > max || t.raw(e).is_none() {
        return Err(RawError::malformed(format!("invalid DCP tag {tag} size")));
    }
    (0..e.count as usize).map(|i| t.float(e, i).ok_or_else(|| RawError::malformed(format!("invalid DCP tag {tag} number")))).collect()
}
fn matrix(t: &Tiff<'_>, ifd: &Ifd, tag: u16) -> Result<[[f64; 3]; 3], RawError> {
    if ifd.get(tag).is_some_and(|e| e.count == 12) {
        return Err(RawError::unsupported("four-plane camera profiles require a four-channel RAW decoder"));
    }
    let values = floats(t, ifd, tag, 9)?;
    color::from_slice(&values).ok_or_else(|| RawError::malformed("DCP matrix must have nine values"))
}
fn string(t: &Tiff<'_>, ifd: &Ifd, tag: u16) -> Result<String, RawError> {
    let e = ifd.get(tag).filter(|e| e.typ == 2 && e.count <= 129).ok_or_else(|| RawError::malformed(format!("DCP missing/oversized string {tag}")))?;
    t.ascii(e).filter(|s| s.chars().all(|c| !c.is_control())).ok_or_else(|| RawError::malformed("invalid DCP string"))
}
fn scalar(t: &Tiff<'_>, ifd: &Ifd, tag: u16, default: f64) -> Result<f64, RawError> {
    if !ifd.has(tag) {
        return Ok(default);
    }
    let v = floats(t, ifd, tag, 1)?;
    v.first().copied().ok_or_else(|| RawError::malformed("empty DCP scalar"))
}
fn table(t: &Tiff<'_>, ifd: &Ifd, dims_tag: u16, data_tag: u16, encoding_tag: u16) -> Result<Option<HueSatMap>, RawError> {
    if !ifd.has(data_tag) {
        return Ok(None);
    }
    let dims = t.tag_uints(ifd, dims_tag);
    let [h, s, v] = dims.as_slice() else {
        return Err(RawError::malformed("DCP table needs three dimensions"));
    };
    let dims = [*h as usize, *s as usize, *v as usize];
    let count = dims
        .iter()
        .try_fold(1usize, |a, b| a.checked_mul(*b))
        .filter(|n| *n > 0 && *n <= 1_048_576)
        .and_then(|n| n.checked_mul(3))
        .ok_or_else(|| RawError::malformed("DCP table dimensions exceed limits"))?;
    let e = ifd.get(data_tag).ok_or_else(|| RawError::malformed("missing DCP table"))?;
    if e.typ != 11 || e.count as usize != count {
        return Err(RawError::malformed("DCP table dimensions and data do not match"));
    }
    let values = floats(t, ifd, data_tag, count)?;
    let encoding = scalar(t, ifd, encoding_tag, 0.0)?;
    if !matches!(encoding, 0.0 | 1.0) {
        return Err(RawError::unsupported("DCP table encoding"));
    }
    let map = HueSatMap { dims, srgb_value: encoding == 1.0, values: values.as_chunks::<3>().0.iter().map(|p| p.map(|v| v as f32)).collect() };
    map.validate()?;
    Ok(Some(map))
}

pub(super) fn parse(bytes: &[u8]) -> Result<CameraProfile, RawError> {
    if bytes.len() > MAX_BYTES {
        return Err(RawError::malformed("DCP file exceeds 32 MiB"));
    }
    let t = Tiff::camera_profile(bytes).ok_or_else(|| RawError::malformed("not a DCP camera profile"))?;
    let ifd = t.ifd_at(t.first_ifd, 0).ok_or_else(|| RawError::malformed("DCP has no profile IFD"))?;
    if t.u16_at(t.first_ifd).map(usize::from) != Some(ifd.entries.len()) {
        return Err(RawError::malformed("truncated DCP directory"));
    }
    let mut seen = std::collections::HashSet::new();
    for e in &ifd.entries {
        if !seen.insert(e.tag) || t.raw(e).is_none() {
            return Err(RawError::malformed("duplicate or truncated DCP tag"));
        }
        // Reject unsupported processing, rather than silently ignoring HDR/third illuminants/RGB tables.
        if !matches!(
            e.tag,
            50706
                | 50707
                | 50708
                | 50721
                | 50722
                | 50778
                | 50779
                | 50932
                | 50936
                | 50937
                | 50938
                | 50939
                | 50940
                | 50941
                | 50942
                | 50964
                | 50965
                | 50981
                | 50982
                | 51107
                | 51108
                | 51109
                | 51110
                | 52552
        ) {
            return Err(RawError::unsupported(format!("DCP tag {} is not supported", e.tag)));
        }
    }
    let mut calibrations = Vec::new();
    for (cm, fm, ill) in [(50721, 50964, 50778), (50722, 50965, 50779)] {
        if !ifd.has(cm) {
            if ifd.has(fm) {
                return Err(RawError::malformed("DCP ForwardMatrix has no ColorMatrix"));
            }
            continue;
        }
        let temperature = t.tag_uint(&ifd, ill).and_then(color::illuminant_temperature).ok_or_else(|| RawError::unsupported("DCP calibration illuminant"))?;
        calibrations.push(Calibration {
            temperature,
            color_matrix: matrix(&t, &ifd, cm)?,
            forward_matrix: if ifd.has(fm) { Some(matrix(&t, &ifd, fm)?) } else { None },
            camera_calibration: IDENTITY,
        });
    }
    let reverse = matches!(calibrations.as_slice(),[a,b] if a.temperature>b.temperature);
    if reverse {
        calibrations.reverse();
    }
    let mut hue_sat_maps = Vec::new();
    if let Some(map) = table(&t, &ifd, 50937, 50938, 51107)? {
        hue_sat_maps.push(map);
    }
    if let Some(map) = table(&t, &ifd, 50937, 50939, 51107)? {
        if hue_sat_maps.is_empty() {
            return Err(RawError::malformed("DCP second HSV map has no first map"));
        }
        hue_sat_maps.push(map);
    }
    if reverse && hue_sat_maps.len() == 2 {
        hue_sat_maps.reverse();
    }
    let points = if ifd.has(50940) { floats(&t, &ifd, 50940, 8192)? } else { Vec::new() };
    if points.len() % 2 != 0 {
        return Err(RawError::malformed("DCP tone curve has an odd coordinate count"));
    }
    let black = scalar(&t, &ifd, 51110, 0.0)?;
    if !matches!(black, 0.0 | 1.0) {
        return Err(RawError::unsupported("DCP black-render mode"));
    }
    let policy = scalar(&t, &ifd, 50941, 0.0)?;
    if !matches!(policy, 0.0 | 1.0 | 2.0 | 3.0) {
        return Err(RawError::unsupported("unknown DCP usage policy"));
    }
    let copyright = if let Some(e) = ifd.get(50942) {
        if !matches!(e.typ, 1 | 2) || e.count > 4096 {
            return Err(RawError::malformed("invalid DCP copyright notice"));
        }
        t.ascii(e).unwrap_or_default()
    } else {
        String::new()
    };
    let p = CameraProfile {
        name: string(&t, &ifd, 50936)?,
        camera_model: string(&t, &ifd, 50708)?,
        calibrations,
        exposure_offset: scalar(&t, &ifd, 51109, 0.0)?,
        tone_curve: points.as_chunks::<2>().0.to_vec(),
        hue_sat_maps,
        look_table: table(&t, &ifd, 50981, 50982, 51108)?,
        default_black_render: black as u32,
        embed_policy: policy as u32,
        copyright,
    };
    p.validate_data()?;
    Ok(p)
}
