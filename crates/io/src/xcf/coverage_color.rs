//! Ordinary native color-adjustment parameter decoding for import-time coverage.
//! This converts command data to existing document types; native compose owns all math.
use photocraft_doc::{Adjustment, adjust::HueRange};
use serde_json::Value;

type Result<T> = std::result::Result<T, String>;
const RANGES: [&str; 6] = ["reds", "yellows", "greens", "cyans", "blues", "magentas"];
const BW_DEFAULTS: [f32; 6] = [40., 60., 40., 60., 20., 80.];

fn field<'a>(p: &'a Value, key: &str) -> Option<&'a Value> {
    p.get(key).filter(|v| !v.is_null())
}
fn number(p: &Value, key: &str, default: f32, lo: f32, hi: f32) -> Result<f32> {
    match field(p, key) {
        None => Ok(default.clamp(lo, hi)),
        Some(v) => v.as_f64().filter(|v| v.is_finite()).map(|v| (v as f32).clamp(lo, hi)).ok_or_else(|| format!("Native {key} must be a finite number")),
    }
}
fn boolean(p: &Value, key: &str, default: bool) -> Result<bool> {
    match field(p, key) {
        None => Ok(default),
        Some(v) => v.as_bool().ok_or_else(|| format!("Native {key} must be boolean")),
    }
}
fn array<const N: usize>(v: &Value, key: &str) -> Result<[f32; N]> {
    let input = v.as_array().filter(|a| a.len() == N).ok_or_else(|| format!("Native {key} needs exactly {N} numbers"))?;
    let mut output = [0.; N];
    for (target, value) in output.iter_mut().zip(input) {
        *target = value.as_f64().filter(|n| n.is_finite()).map(|v| (v as f32).clamp(-1e6, 1e6)).ok_or_else(|| format!("Native {key} needs finite numbers"))?;
    }
    Ok(output)
}
fn color(p: &Value, key: &str, default: [f32; 3]) -> Result<[f32; 3]> {
    let Some(v) = field(p, key) else { return Ok(default) };
    if let Some(text) = v.as_str() {
        let h = text.trim_start_matches('#');
        if !matches!(h.len(), 6 | 8) {
            return Err(format!("Native {key} needs hex RGB or RGB(A)"));
        }
        let mut rgb = [0.; 3];
        for (index, target) in rgb.iter_mut().enumerate() {
            *target = h
                .get(index * 2..index * 2 + 2)
                .and_then(|s| u8::from_str_radix(s, 16).ok())
                .map(|v| f32::from(v) / 255.)
                .ok_or_else(|| format!("Native {key} has invalid hex RGB"))?;
        }
        return Ok(rgb);
    }
    let a = v.as_array().filter(|a| a.len() >= 3).ok_or_else(|| format!("Native {key} needs RGB components"))?;
    let mut rgb = [0.; 3];
    for (target, value) in rgb.iter_mut().zip(a) {
        *target =
            value.as_f64().filter(|v| v.is_finite()).map(|v| (v as f32).clamp(0., 1.)).ok_or_else(|| format!("Native {key} needs finite RGB components"))?;
    }
    Ok(rgb)
}

/// None means another adapter owns this ordinary native command.
pub(super) fn adjustment(command: &str, p: &Value) -> Option<Result<Adjustment>> {
    if !matches!(
        command,
        "image.adjustments.blackWhite"
            | "image.adjustments.colorBalance"
            | "image.adjustments.hueSaturation"
            | "image.adjustments.photoFilter"
            | "image.adjustments.vibrance"
    ) {
        return None;
    }
    Some((|| {
        if !p.is_object() {
            return Err("Native color adjustment parameters must be an object".into());
        }
        Ok(match command {
            "image.adjustments.vibrance" => {
                Adjustment::Vibrance { vibrance: number(p, "vibrance", 0., -100., 100.)?, saturation: number(p, "saturation", 0., -100., 100.)? }
            }
            "image.adjustments.blackWhite" => {
                let mut weights = BW_DEFAULTS;
                for (target, key) in weights.iter_mut().zip(RANGES) {
                    *target = number(p, key, *target, -200., 300.)?;
                }
                let tint_color = color(p, "tintColor", [0.882, 0.827, 0.702])?;
                Adjustment::BlackWhite { weights, tint: boolean(p, "tint", false)?.then_some(tint_color) }
            }
            "image.adjustments.colorBalance" => {
                let mut tones = [[0.; 3]; 3];
                for (target, key) in tones.iter_mut().zip(["shadows", "midtones", "highlights"]) {
                    if let Some(v) = field(p, key) {
                        *target = array(v, key)?.map(|v: f32| v.clamp(-100., 100.));
                    }
                }
                Adjustment::ColorBalance {
                    shadows: tones[0],
                    midtones: tones[1],
                    highlights: tones[2],
                    preserve_luminosity: boolean(p, "preserveLuminosity", true)?,
                }
            }
            "image.adjustments.photoFilter" => {
                // XCF native mappings embed explicit RGB; do not silently ignore a preset
                // or look it up using an external file/resource policy.
                if field(p, "filter").is_some() {
                    return Err("Native coverage Photo Filter requires its embedded explicit color rather than a preset id".into());
                }
                Adjustment::PhotoFilter {
                    color: color(p, "color", [236. / 255., 138. / 255., 0.])?,
                    density: number(p, "density", 25., 0., 100.)? / 100.,
                    preserve_luminosity: boolean(p, "preserveLuminosity", true)?,
                }
            }
            _ => {
                let colorize = boolean(p, "colorize", false)?;
                let mut ranges = HueRange::defaults();
                for (range, key) in ranges.iter_mut().zip(RANGES) {
                    let Some(v) = field(p, key) else { continue };
                    if !v.is_object() {
                        return Err(format!("Native {key} hue range must be an object"));
                    }
                    range.hue = number(v, "hue", range.hue, -180., 180.)?;
                    range.saturation = number(v, "saturation", range.saturation, -100., 100.)?;
                    range.lightness = number(v, "lightness", range.lightness, -100., 100.)?;
                    if let Some(bounds) = field(v, "range") {
                        range.bounds = HueRange::canonical_bounds(array(bounds, "range")?);
                    }
                }
                Adjustment::HueSaturation {
                    hue: number(p, "hue", 0., if colorize { 0. } else { -180. }, if colorize { 360. } else { 180. })?,
                    saturation: number(p, "saturation", if colorize { 25. } else { 0. }, if colorize { 0. } else { -100. }, 100.)?,
                    lightness: number(p, "lightness", 0., -100., 100.)?,
                    colorize,
                    ranges,
                }
            }
        })
    })())
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{ColorMode, SampleType};
    use photocraft_geom::Rect;
    use serde_json::json;

    fn decode(id: &str, params: Value) -> Adjustment {
        adjustment(id, &params).unwrap().unwrap()
    }
    #[test]
    fn five_native_color_adapters_match_independently_authored_native_adjustments_at_all_depths() {
        let mut ranges = HueRange::defaults();
        ranges[0].hue = 17.;
        ranges[0].saturation = -23.;
        ranges[0].lightness = 9.;
        ranges[0].bounds = [-60., -20., 20., 60.];
        let cases = [
            ("image.adjustments.vibrance", json!({"vibrance":37,"saturation":-12}), Adjustment::Vibrance { vibrance: 37., saturation: -12. }),
            (
                "image.adjustments.blackWhite",
                json!({"reds":13,"blues":91,"tint":true,"tintColor":"#336699"}),
                Adjustment::BlackWhite { weights: [13., 60., 40., 60., 91., 80.], tint: Some([0.2, 0.4, 0.6]) },
            ),
            (
                "image.adjustments.colorBalance",
                json!({"shadows":[10,-20,30],"highlights":[-15,25,-35],"preserveLuminosity":false}),
                Adjustment::ColorBalance { shadows: [10., -20., 30.], midtones: [0.; 3], highlights: [-15., 25., -35.], preserve_luminosity: false },
            ),
            (
                "image.adjustments.photoFilter",
                json!({"color":[0.25,0.5,0.75],"density":40,"preserveLuminosity":false}),
                Adjustment::PhotoFilter { color: [0.25, 0.5, 0.75], density: 0.4, preserve_luminosity: false },
            ),
            (
                "image.adjustments.hueSaturation",
                json!({"hue":-25,"saturation":38,"lightness":-12,"reds":{"hue":17,"saturation":-23,"lightness":9,"range":[-60,-20,20,60]}}),
                Adjustment::HueSaturation { hue: -25., saturation: 38., lightness: -12., colorize: false, ranges },
            ),
        ];
        for (id, params, expected) in cases {
            let actual = decode(id, params);
            assert_eq!(actual, expected, "{id}");
            for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
                for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
                    let input = vec![[0.13, 0.42, 0.87, 0.25], [0.9, 0.2, 0.1, 1.], [-0.5, 1.5, 0.25, 0.]];
                    let mut a = photocraft_compose::Buffer { rect: Rect::new(0, 0, 3, 1), px: input.clone() };
                    let mut b = photocraft_compose::Buffer { rect: a.rect, px: input };
                    let transfer = photocraft_compose::adjust::Transfer::for_document(mode, depth);
                    let sample = (depth == SampleType::F32).then_some(depth);
                    photocraft_compose::adjust::apply_in(&actual, &mut a, transfer, sample, mode);
                    photocraft_compose::adjust::apply_in(&expected, &mut b, transfer, sample, mode);
                    assert_eq!(a.px, b.px, "{id}/{mode:?}/{depth:?}");
                    assert!(a.px.iter().flatten().all(|v| v.is_finite()));
                }
            }
        }
    }
    #[test]
    fn ordinary_defaults_nulls_colorize_and_native_field_clamping_are_preserved() {
        assert_eq!(decode("image.adjustments.blackWhite", json!({})), Adjustment::BlackWhite { weights: BW_DEFAULTS, tint: None });
        assert_eq!(
            decode("image.adjustments.photoFilter", json!({"color":null,"density":null})),
            Adjustment::PhotoFilter { color: [236. / 255., 138. / 255., 0.], density: 0.25, preserve_luminosity: true }
        );
        let Adjustment::HueSaturation { hue, saturation, lightness, colorize, .. } =
            decode("image.adjustments.hueSaturation", json!({"colorize":true,"hue":400,"lightness":-200}))
        else {
            panic!("native hue")
        };
        assert_eq!((hue, saturation, lightness, colorize), (360., 25., -100., true));
        let Adjustment::ColorBalance { shadows, .. } = decode("image.adjustments.colorBalance", json!({"shadows":[-300,20,400]})) else {
            panic!("native balance")
        };
        assert_eq!(shadows, [-100., 20., 100.]);
    }
    #[test]
    fn malformed_native_parameters_fail_and_unowned_commands_are_not_claimed() {
        for id in ["blackWhite", "colorBalance", "hueSaturation", "photoFilter", "vibrance"] {
            assert!(adjustment(&format!("image.adjustments.{id}"), &json!([])).unwrap().is_err());
        }
        for (id, p) in [
            ("blackWhite", json!({"tint":1})),
            ("blackWhite", json!({"reds":"50"})),
            ("colorBalance", json!({"shadows":[1,2]})),
            ("colorBalance", json!({"preserveLuminosity":1})),
            ("hueSaturation", json!({"reds":true})),
            ("hueSaturation", json!({"blues":{"range":[1,2,3]}})),
            ("photoFilter", json!({"color":"éabcde"})),
            ("photoFilter", json!({"filter":"sepia"})),
            ("photoFilter", json!({"color":[1,null,0]})),
            ("vibrance", json!({"vibrance":false})),
        ] {
            assert!(adjustment(&format!("image.adjustments.{id}"), &p).unwrap().is_err(), "{id}");
        }
        assert!(adjustment("source:operation", &json!({})).is_none());
        assert!(adjustment("image.adjustments.shadowsHighlights", &json!({})).is_none());
    }
}
