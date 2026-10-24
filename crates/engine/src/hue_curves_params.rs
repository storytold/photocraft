//! Parameters of the Hue Curves adjustment (`image.adjustments.hueCurves`,
//! `layer.newAdjustmentLayer.hueCurves`, `layer.setAdjustment`): PhotoCraft's own, so there is no
//! Photoshop dialog to mirror. Each curve is a list of `[hue degrees, value]` points:
//!
//! * `hue`: hue shift in degrees, -180..180 (0 leaves the hue),
//! * `saturation`: saturation factor, 0..2 (1 leaves it, 0 greys, 2 doubles),
//! * `luminance`: lightness change in percent, -100..100.
//!
//! Hues wrap (370 is 10) and the curves run periodically through red. An empty list is neutral.
//! A curve the params leave out keeps `base`'s.

use photocraft_doc::Adjustment;
use photocraft_doc::adjust::CurvePoint;
use serde_json::{Value, json};

use crate::{EngineError, Result};

/// Curve keys in storage order, with each value's range.
pub const CURVES: [(&str, f32, f32); 3] = [("hue", -180.0, 180.0), ("saturation", 0.0, 2.0), ("luminance", -100.0, 100.0)];

/// Most points one curve takes.
pub const MAX_POINTS: usize = 32;

const CMD: &str = "hueCurves adjustment";

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: CMD.into(), msg: msg.into() }
}

fn curve(key: &str, v: &Value, lo: f32, hi: f32) -> Result<Vec<CurvePoint>> {
    let shape = || bad(format!("`{key}` must be an array of [hue degrees, value] pairs (at most {MAX_POINTS})"));
    let a = v.as_array().filter(|a| a.len() <= MAX_POINTS).ok_or_else(shape)?;
    let mut pts = a
        .iter()
        .map(|q| {
            let q = q.as_array().filter(|q| q.len() == 2).ok_or_else(shape)?;
            let n = |i: usize| q.get(i).and_then(Value::as_f64).filter(|x| x.is_finite()).ok_or_else(|| bad(format!("`{key}` points must be finite numbers")));
            let hue = (n(0)? as f32).rem_euclid(360.0);
            // rem_euclid of a tiny negative rounds up to 360, which is hue 0.
            Ok(CurvePoint { input: if hue >= 360.0 { 0.0 } else { hue }, output: (n(1)? as f32).clamp(lo, hi) })
        })
        .collect::<Result<Vec<_>>>()?;
    pts.sort_by(|a, b| a.input.total_cmp(&b.input));
    pts.dedup_by(|b, a| (b.input - a.input).abs() < 1e-3);
    Ok(pts)
}

/// Hue Curves from `p` over `base` (neutral when None or another kind).
pub fn from_params(p: &Value, base: Option<&Adjustment>) -> Result<Adjustment> {
    let [mut hue, mut saturation, mut luminance] = match base {
        Some(Adjustment::HueCurves { hue, saturation, luminance }) => [hue.clone(), saturation.clone(), luminance.clone()],
        _ => Default::default(),
    };
    for ((key, lo, hi), slot) in CURVES.into_iter().zip([&mut hue, &mut saturation, &mut luminance]) {
        if let Some(v) = p.get(key).filter(|v| !v.is_null()) {
            *slot = curve(key, v, lo, hi)?;
        }
    }
    Ok(Adjustment::HueCurves { hue, saturation, luminance })
}

/// The complete parameter set (the inverse of [`from_params`]).
pub fn to_params(hue: &[CurvePoint], saturation: &[CurvePoint], luminance: &[CurvePoint]) -> Value {
    let arr = |c: &[CurvePoint]| json!(c.iter().map(|p| [p.input, p.output]).collect::<Vec<_>>());
    json!({"hue": arr(hue), "saturation": arr(saturation), "luminance": arr(luminance)})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_round_trip_and_merge() {
        let a = from_params(&json!({"hue": [[0, 30], [120, 0], [240, 0]], "saturation": [[120, 0.5]], "luminance": [[-30, 20]]}), None).unwrap();
        let Adjustment::HueCurves { hue, saturation, luminance } = &a else { panic!("{a:?}") };
        assert_eq!(hue.len(), 3);
        assert_eq!(saturation[0].output, 0.5);
        assert_eq!(luminance[0].input, 330.0, "hues wrap");
        assert_eq!(from_params(&to_params(hue, saturation, luminance), None).unwrap(), a);
        // A partial update keeps the other curves.
        let b = from_params(&json!({"saturation": []}), Some(&a)).unwrap();
        let Adjustment::HueCurves { hue: h2, saturation: s2, .. } = &b else { panic!() };
        assert_eq!(h2, hue);
        assert!(s2.is_empty());
        // Values are clamped to each curve's range; points come out sorted.
        let c = from_params(&json!({"hue": [[200, 999], [10, -999]], "saturation": [[0, 9]]}), None).unwrap();
        let Adjustment::HueCurves { hue, saturation, .. } = c else { panic!() };
        assert_eq!((hue[0].input, hue[0].output, hue[1].output), (10.0, -180.0, 180.0));
        assert_eq!(saturation[0].output, 2.0);
    }

    #[test]
    fn bad_params_are_errors() {
        for p in [
            json!({"hue": 5}),
            json!({"hue": [[1]]}),
            json!({"hue": [["red", 3]]}),
            json!({"saturation": [[1, "x"]]}),
            json!({"luminance": vec![[0, 0]; MAX_POINTS + 1]}),
        ] {
            assert!(from_params(&p, None).is_err(), "{p}");
        }
    }
}
