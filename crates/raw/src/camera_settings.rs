//! Public Nikon MakerNote / PictureControl layouts documented by ExifTool tag tables.
//! Numeric controls are retained as raw codes: Auto and n/a are not invented slider values.
use crate::{
    tiff::{Tiff, tag},
    tiffep,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PictureControl {
    pub version: String,
    pub name: String,
    pub base: String,
    pub adjustment: u8,
    pub contrast_code: u8,
    pub brightness_code: u8,
    pub saturation_code: u8,
    pub hue_code: u8,
    pub filter_effect_code: u8,
    pub toning_effect_code: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraSettings {
    pub make: String,
    pub model: String,
    pub picture_control: Option<PictureControl>,
    /// Nikon MakerNote ColorSpace: 1=sRGB, 2=Adobe RGB. Unknown remains None.
    pub color_space: Option<u32>,
}

fn text(bytes: &[u8]) -> Option<String> {
    let end = bytes.iter().position(|v| *v == 0).unwrap_or(bytes.len());
    let text = std::str::from_utf8(bytes.get(..end)?).ok()?.trim();
    (!text.is_empty() && text.chars().all(|c| !c.is_control())).then(|| text.to_string())
}

pub fn picture_control(bytes: &[u8]) -> Option<PictureControl> {
    let version = text(bytes.get(..4)?)?;
    let (name, base, adjustment, contrast, brightness, saturation, hue) = match version.as_str() {
        "0100" => (4, 24, 48, 51, 52, 53, 54),
        "0200" | "0201" => (4, 24, 48, 55, 57, 59, 61),
        "0300" | "0301" | "0310" => (8, 28, 54, 63, 65, 67, 69),
        _ => return None,
    };
    let filter = if version == "0100" { hue + 1 } else { hue + 2 };
    Some(PictureControl {
        version,
        name: text(bytes.get(name..name + 20)?)?,
        base: text(bytes.get(base..base + 20)?)?,
        adjustment: *bytes.get(adjustment)?,
        contrast_code: *bytes.get(contrast)?,
        brightness_code: *bytes.get(brightness)?,
        saturation_code: *bytes.get(saturation)?,
        hue_code: *bytes.get(hue)?,
        filter_effect_code: *bytes.get(filter)?,
        toning_effect_code: *bytes.get(filter + 1)?,
    })
}

pub fn camera_settings(bytes: &[u8]) -> Option<CameraSettings> {
    let t = Tiff::new(bytes)?;
    let ifds = t.all_ifds();
    let make = ifds.iter().find_map(|i| t.tag_ascii(i, tag::MAKE))?;
    let model = ifds.iter().find_map(|i| t.tag_ascii(i, tag::MODEL)).unwrap_or_default();
    let (picture_control, color_space) = if let Some((note, ifd)) = tiffep::nikon_maker_note(&t, &ifds) {
        let control = ifd.get(0x0023).and_then(|e| note.raw(e)).and_then(picture_control);
        (control, note.tag_uint(&ifd, 0x001e).filter(|v| matches!(v, 1 | 2)))
    } else {
        (None, None)
    };
    Some(CameraSettings { make, model, picture_control, color_space })
}
