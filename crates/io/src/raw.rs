//! Camera raw files via `photocraft-raw`: developed to a 16-bit RGB document
//! in ProPhoto RGB (the embedded profile is the built-in ProPhoto-compatible
//! profile), or, for raw variants not decoded yet, the camera's embedded
//! JPEG preview with a warning.

use std::sync::Arc;

use photocraft_codecs::{self as codecs, ChannelLayout, Format, Image};
use photocraft_raw::{DevelopOptions, Limits, RawError, WhiteBalance};

use crate::flat::image_to_document;
use crate::{ImportResult, IoError};

/// `true` if `bytes` are a camera raw file `photocraft-raw` recognises.
pub fn is_raw(bytes: &[u8]) -> bool {
    photocraft_raw::is_raw(bytes)
}

/// The decode limits shared with the flat codecs.
fn limits() -> Limits {
    let l = codecs::Limits::default();
    Limits { max_width: l.max_width, max_height: l.max_height, max_pixels: l.max_pixels, max_alloc: l.max_alloc }
}

/// A raw file's embedded JPEG preview, turned upright. Its own EXIF orientation wins when it
/// has one; otherwise the raw's IFD0 orientation applies (TIFF-based raws record it there, and
/// their previews are usually stored as the sensor reads out). Developed raws are oriented by
/// `photocraft-raw` and carry no EXIF, so nothing is turned twice.
fn upright_preview(raw: &[u8], jpeg: &[u8]) -> Result<Image, IoError> {
    let img = codecs::decode_as_with(Format::Jpeg, jpeg, &codecs::DecodeOptions { keep_orientation: true, ..Default::default() })?;
    let own = img.meta.exif.as_deref().map_or(1, codecs::exif_orientation);
    let o = if own != 1 { own } else { codecs::exif_orientation(raw) };
    Ok(img.oriented(o)?)
}

/// Camera Raw's Temperature, Tint and Exposure, applied while developing the sensor data
/// (the open-time Camera Raw dialog) instead of to the developed RGB pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RawTuning {
    /// −100…100, as `photocraft_algo::camera_raw::CameraRaw::temperature`.
    pub temperature: f32,
    /// −100…100, as `CameraRaw::tint`.
    pub tint: f32,
    /// EV, as `CameraRaw::exposure`.
    pub exposure: f32,
}

/// Develops a raw file with `tuning` applied at the raw stage: Exposure as develop exposure,
/// Temperature / Tint as gains on the camera's as-shot white-balance multipliers (the same
/// gains the Camera Raw filter applies to RGB). The second value is `false` when the file has
/// no as-shot white balance: Temperature / Tint were then not applied, and the caller keeps
/// them as RGB adjustments.
pub fn import_raw_tuned(name: &str, bytes: &[u8], tuning: &RawTuning) -> Result<(ImportResult, bool), IoError> {
    let format = photocraft_raw::identify(bytes).map(|f| f.name()).unwrap_or("camera raw");
    let exposure = f64::from(tuning.exposure);
    if !exposure.is_finite() || !(-10.0..=10.0).contains(&exposure) {
        return Err(IoError::Raw(RawError::Malformed(format!("exposure {} EV is out of range", tuning.exposure))));
    }
    let sensor = photocraft_raw::decode(bytes, &limits()).map_err(IoError::Raw)?;
    let mut opts = DevelopOptions { limits: limits(), exposure, ..Default::default() };
    let wb_applied = match sensor.camera_wb {
        Some(m) => {
            let g = wb_gains(tuning.temperature, tuning.tint);
            opts.white_balance = WhiteBalance::Multipliers([m[0] * g[0], m[1] * g[1], m[2] * g[2]]);
            true
        }
        None => false,
    };
    let dev = photocraft_raw::develop_sensor(&sensor, &opts).map_err(IoError::Raw)?;
    let wb = if wb_applied {
        format!("as-shot white balance, temperature {:+.0}, tint {:+.0}", tuning.temperature, tuning.tint)
    } else {
        "estimated white balance".into()
    };
    let summary = format!("developed in Camera Raw (exposure {:+.2} EV, {wb})", tuning.exposure);
    Ok((developed_document(name, format, dev, &summary)?, wb_applied))
}

/// "Canon" + "Canon EOS 80D" and "NIKON CORPORATION" + "NIKON D3200" read as the model alone:
/// the model already starts with the make's first word.
fn camera_name(make: Option<&str>, model: Option<&str>) -> String {
    match (make, model) {
        (Some(make), Some(model))
            if make.split_whitespace().next().is_some_and(|brand| model.to_ascii_lowercase().starts_with(&brand.to_ascii_lowercase())) =>
        {
            model.to_string()
        }
        (make, model) => [make, model].into_iter().flatten().collect::<Vec<_>>().join(" "),
    }
}

/// Camera Raw's white-balance gains for Temperature / Tint (−100…100), the same the Camera
/// Raw filter applies ([`photocraft_algo::camera_raw::white_balance_gains`]).
fn wb_gains(temperature: f32, tint: f32) -> [f64; 3] {
    photocraft_algo::camera_raw::white_balance_gains(temperature.clamp(-100.0, 100.0), tint.clamp(-100.0, 100.0)).map(f64::from)
}

/// A developed raw as a 16-bit ProPhoto document, with a note saying how it was developed.
fn developed_document(name: &str, format: &str, dev: photocraft_raw::Developed, summary: &str) -> Result<ImportResult, IoError> {
    let img = Image::from_u16(dev.width, dev.height, ChannelLayout::Rgb, &dev.rgb)?;
    let mut r = image_to_document(name, &img)?;
    r.document.icc_profile = Some(Arc::new(photocraft_cms::Builtin::ProPhotoCompat.profile().to_bytes().to_vec()));
    let camera = camera_name(dev.info.make.as_deref(), dev.info.model.as_deref());
    r.warnings.push(format!(
        "{format}{} {summary} into 16-bit {}",
        if camera.is_empty() { String::new() } else { format!(" from {camera}") },
        photocraft_raw::OUTPUT_SPACE
    ));
    r.warnings.extend(dev.warnings);
    Ok(r)
}

/// Develops a raw file with the default settings.
pub fn import_raw(name: &str, bytes: &[u8]) -> Result<ImportResult, IoError> {
    import_raw_with(name, bytes, &DevelopOptions { limits: limits(), ..Default::default() })
}

/// In the import note of a raw developed from its sensor data with the default settings (not of
/// a preview fallback): what the open-time Camera Raw dialog looks for.
pub const DEVELOPED_NOTE: &str = "developed with default settings";

/// Develops a raw file with explicit settings.
pub fn import_raw_with(name: &str, bytes: &[u8], opts: &DevelopOptions) -> Result<ImportResult, IoError> {
    let format = photocraft_raw::identify(bytes).map(|f| f.name()).unwrap_or("camera raw");
    match photocraft_raw::develop(bytes, opts) {
        Ok(dev) => {
            let summary = format!("{DEVELOPED_NOTE} ({} demosaic, as-shot white balance)", opts.demosaic.id());
            developed_document(name, format, dev, &summary)
        }
        Err(RawError::Unsupported(reason)) => match photocraft_raw::embedded_preview(bytes) {
            Some(p) => {
                let img = upright_preview(bytes, p.jpeg)?;
                let mut r = image_to_document(name, &img)?;
                r.warnings.insert(
                    0,
                    format!(
                        "{format}: {reason} is not supported yet; opened the camera's embedded {}x{} JPEG preview instead (8-bit, not the raw sensor data)",
                        p.width, p.height
                    ),
                );
                Ok(r)
            }
            None => Err(IoError::Raw(RawError::Unsupported(reason))),
        },
        Err(e) => Err(IoError::Raw(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_names_drop_a_repeated_make() {
        assert_eq!(camera_name(Some("Canon"), Some("Canon EOS 80D")), "Canon EOS 80D");
        assert_eq!(camera_name(Some("NIKON CORPORATION"), Some("NIKON D3200")), "NIKON D3200");
        assert_eq!(camera_name(Some("SONY"), Some("ILCE-7M3")), "SONY ILCE-7M3");
        assert_eq!(camera_name(None, Some("X")), "X");
        assert_eq!(camera_name(Some("Panasonic"), Some("DC-G9")), "Panasonic DC-G9");
    }
}
