//! Adobe Technical Note 5902 names an arbitrary instance by its face prefix and user-space
//! coordinates. A PSD can therefore identify the actual face and position between named styles.
use photocraft_doc::text::FontVariation;

use super::{FaceInfo, NamedInstance, ResolvedFont, fallback_style};

const MAX_NAME: usize = 127;

/// Name ID 25 distinguishes separate variable faces such as a family's italic file.
pub(super) fn prefix(font: &skrifa::FontRef<'_>) -> Option<String> {
    use skrifa::{MetadataProvider, string::StringId};
    let name = |id| font.localized_strings(id).english_or_first().map(|s| s.to_string());
    if let Some(explicit) = name(StringId::VARIATIONS_POSTSCRIPT_NAME_PREFIX) {
        let prefix: String = explicit.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
        if !prefix.is_empty() {
            return Some(prefix);
        }
    }
    let family = name(StringId::TYPOGRAPHIC_FAMILY_NAME).or_else(|| name(StringId::FAMILY_NAME))?;
    let prefix: String = family.chars().filter(char::is_ascii_alphanumeric).collect();
    (!prefix.is_empty()).then_some(prefix)
}

pub(crate) struct VariationName<'a> {
    pub(crate) prefix: &'a str,
    pub(crate) axes: Vec<FontVariation>,
}

fn parse_axis_field(field: &str) -> Option<FontVariation> {
    if !field.as_bytes().first().is_some_and(|b| b.is_ascii_digit() || matches!(b, b'-' | b'.')) {
        return None;
    }
    let i = field.find(|c: char| c.is_ascii_alphabetic())?;
    let (number, tag) = field.split_at(i);
    if tag.len() > 4 || !tag.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return None;
    }
    let fixed = (number.parse::<f64>().ok()? * 65536.0).round();
    if !(f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&fixed) {
        return None;
    }
    let value = (fixed / 65536.0) as f32;
    Some(FontVariation { axis: format!("{tag:<4}"), value })
}

impl<'a> VariationName<'a> {
    pub(crate) fn parse(ps: &'a str) -> Option<Self> {
        if ps.len() > MAX_NAME {
            return None;
        }
        let (at, axes) = ps.match_indices('_').find_map(|(at, _)| {
            let axes: Option<Vec<FontVariation>> = ps.get(at + 1..)?.split('_').map(parse_axis_field).collect();
            axes.map(|axes| (at, axes))
        })?;
        let prefix = ps.get(..at)?;
        if prefix.is_empty() || !prefix.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            return None;
        }
        for (i, v) in axes.iter().enumerate() {
            if axes[..i].iter().any(|prev| prev.axis == v.axis) {
                return None;
            }
        }
        Some(Self { prefix, axes })
    }

    pub(crate) fn matches(&self, face: &FaceInfo) -> bool {
        face.variation_prefix.as_deref() == Some(self.prefix)
            && self.axes.iter().all(|v| face.axes.iter().any(|(tag, min, _, max)| *tag == v.axis && (*min..=*max).contains(&v.value)))
    }

    pub(crate) fn instance(&self, face: &FaceInfo, face_index: usize, ps: &str) -> NamedInstance {
        let f = ResolvedFont { family: face.family.clone(), weight: face.weight.round() as u16, italic: face.italic, exact: true }.with_variations(&self.axes);
        NamedInstance {
            face_index,
            style: fallback_style(f32::from(f.weight), f.italic),
            postscript_name: Some(ps.into()),
            explicit_postscript: false,
            weight: f32::from(f.weight),
            italic: f.italic,
            axes: self.axes.iter().map(|v| (v.axis.clone(), v.value)).collect(),
        }
    }
}

impl ResolvedFont {
    /// Explicit axis values take precedence over the CSS attributes of the named face.
    pub fn with_variations(mut self, axes: &[FontVariation]) -> Self {
        let value = |tag| axes.iter().find(|v| v.axis == tag && v.value.is_finite()).map(|v| v.value);
        if let Some(w) = value("wght") {
            self.weight = w.round().clamp(1.0, 1000.0) as u16;
        }
        if let Some(italic) = value("ital").map(|v| v >= 0.5).or_else(|| value("slnt").map(|v| v < -0.5)) {
            self.italic = italic;
        }
        self
    }
}

impl FaceInfo {
    /// A reversible PostScript name for a custom position. Supply the non-default user coordinates.
    pub fn variation_postscript<'a>(&self, coords: impl IntoIterator<Item = (&'a str, f32)>) -> Option<String> {
        use std::fmt::Write;
        let mut name = self.variation_prefix.clone()?;
        let prefix_len = name.len();
        for (tag, value) in coords {
            let tag = tag.trim_end_matches(' ');
            if tag.len() > 4
                || !value.is_finite()
                || !tag.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
                || !tag.bytes().all(|b| b.is_ascii_alphanumeric())
            {
                return None;
            }
            write!(name, "_{}{tag}", coordinate(value)).ok()?;
            if name.len() > MAX_NAME {
                return None;
            }
        }
        (name.len() > prefix_len).then_some(name)
    }
}

/// The shortest decimal precision that recovers the original signed 16.16 coordinate.
fn coordinate(value: f32) -> String {
    let fixed = (f64::from(value) * 65536.0).round();
    if fixed == 0.0 {
        return "0".into();
    }
    // Five fractional digits can distinguish every 16.16 value.
    for scale in [1.0, 10.0, 100.0, 1000.0, 10_000.0, 100_000.0] {
        let n = (fixed / 65536.0 * scale).round() / scale;
        if (n * 65536.0).round() == fixed {
            let text = n.to_string();
            if let Some(fraction) = text.strip_prefix("-0.") {
                return format!("-.{fraction}");
            }
            if let Some(fraction) = text.strip_prefix("0.") {
                return format!(".{fraction}");
            }
            return text;
        }
    }
    value.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arbitrary_names_keep_fixed_point_coordinates_and_padded_tags() {
        for n in [0, 1, -1, 32768, -32768, 119_372, 752_358_400] {
            let value = n as f32 / 65536.0;
            let name = format!("Var_Italic_{}GR_83.25wdth", coordinate(value));
            let parsed = VariationName::parse(&name).unwrap();
            assert_eq!(parsed.prefix, "Var_Italic");
            assert_eq!(parsed.axes[0].axis, "GR  ");
            assert_eq!(parsed.axes[0].value, value);
            assert_eq!(parsed.axes[1].value, 83.25);
        }
        for bad in ["Font_Name", "Font_500wght_600wght", "Font_500wght_", "Font_5abcde", "Font_Infwght"] {
            assert!(VariationName::parse(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn numeric_underscore_variation_prefix_round_trips_back_to_face() {
        let face = FaceInfo {
            family: "Var 2".into(),
            style: "Regular".into(),
            postscript_name: Some("Var_2-Regular".into()),
            weight: 400.0,
            italic: false,
            axes: vec![("wght".into(), 100.0, 400.0, 900.0), ("wdth".into(), 75.0, 100.0, 100.0)],
            variation_prefix: Some("Var_2".into()),
            font_axes: Vec::new(),
            instances: Vec::new(),
        };
        let ps = face.variation_postscript([("wght", 650.0), ("wdth", 82.5)]).unwrap();
        assert_eq!(ps, "Var_2_650wght_82.5wdth");
        let parsed = VariationName::parse(&ps).unwrap();
        assert_eq!(parsed.prefix, "Var_2");
        assert!(parsed.matches(&face));
        let inst = parsed.instance(&face, 0, &ps);
        assert_eq!(inst.weight, 650.0);
        assert_eq!(inst.axes, vec![("wght".into(), 650.0), ("wdth".into(), 82.5)]);
    }
}
