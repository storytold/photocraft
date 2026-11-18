//! Public Nikon MakerNote / PictureControl layouts documented by ExifTool tag tables.
//! Numeric controls are retained as raw codes: Auto and n/a are not invented slider values.
use craft_tiff::{ParseOptions, Tiff, tags};

/// Recognized Nikon Picture Control metadata with adjustment bytes preserved verbatim.
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

/// Camera identity and optional rendering metadata recovered from TIFF-based RAW input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraSettings {
    pub make: String,
    pub model: String,
    pub picture_control: Option<PictureControl>,
    /// Nikon MakerNote ColorSpace: 1=sRGB, 2=Adobe RGB. Unknown remains None.
    pub color_space: Option<u32>,
}

/// Decodes trimmed, NUL-terminated UTF-8 metadata while rejecting control characters.
fn text(bytes: &[u8]) -> Option<String> {
    let end = bytes.iter().position(|v| *v == 0).unwrap_or(bytes.len());
    let text = std::str::from_utf8(bytes.get(..end)?).ok()?.trim();
    (!text.is_empty() && text.chars().all(|c| !c.is_control())).then(|| text.to_string())
}

/// Reads a supported Nikon Picture Control layout without interpreting Auto or n/a codes.
/// Unknown versions, malformed text and truncated fields return `None`.
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

/// Extracts camera identity and recognized Nikon settings from TIFF-based RAW bytes.
/// Invalid TIFF or a missing make returns `None`; absent optional metadata stays unset.
pub fn camera_settings(bytes: &[u8]) -> Option<CameraSettings> {
    let opts = ParseOptions { max_ifds: 64, max_depth: 8, value_budget: Some(1 << 20), ..Default::default() };
    let t = Tiff::parse_with(bytes, &opts).ok()?;
    let ifds = t.all_ifds();
    let make = ifds.iter().find_map(|i| i.string(tags::MAKE))?;
    let model = ifds.iter().find_map(|i| i.string(tags::MODEL)).unwrap_or_default();
    let note = ifds
        .iter()
        .find_map(|i| i.get(tags::MAKER_NOTE))
        .and_then(|e| craft_tiff::makernote::parse_makernote(bytes, e.offset, e.count() as u64, t.order, &make));
    let (picture_control, color_space) =
        match note.filter(|n| matches!(n.kind, craft_tiff::makernote::MakerNoteKind::NikonV1 | craft_tiff::makernote::MakerNoteKind::NikonV3)) {
            Some(note) => (note.ifd.bytes(0x0023).and_then(picture_control), note.ifd.u32(0x001e).filter(|v| matches!(v, 1 | 2))),
            None => (None, None),
        };
    Some(CameraSettings { make, model, picture_control, color_space })
}
