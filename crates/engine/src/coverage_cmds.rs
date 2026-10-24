//! Ordinary native coverage filters; no file-format-specific runtime behavior.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use photocraft_algo::{FilterParams, coverage};
use serde_json::Value;
pub const SET_ALPHA: &str = "filter.other.setAlpha";
pub const CLAMP_ALPHA: &str = "filter.other.clampAlpha";
pub const THRESHOLD_ALPHA: &str = "filter.other.thresholdAlpha";
pub const ALPHA_TO_COLOR: &str = "filter.other.alphaToColor";
pub const ALPHA_CURVE: &str = "filter.other.alphaCurve";
pub const FILL_HOLES: &str = "filter.other.fillHoles";
pub const PERCENTILE: &str = "filter.other.percentile";
pub const CHANNEL_NOISE: &str = "filter.noise.channelNoise";
pub const CONVOLUTION: &str = "filter.other.convolution";
pub(crate) fn owns(id: &str) -> bool {
    matches!(id, SET_ALPHA | CLAMP_ALPHA | THRESHOLD_ALPHA | ALPHA_CURVE | ALPHA_TO_COLOR | FILL_HOLES | PERCENTILE | CHANNEL_NOISE | CONVOLUTION)
}
pub(crate) fn params(id: &str, p: &Value) -> Result<FilterParams> {
    let bad = |msg: &str| EngineError::BadParams { cmd: id.into(), msg: msg.into() };
    if !p.is_object() {
        return Err(bad("params must be an object"));
    }
    for (key, value) in p.as_object().into_iter().flatten() {
        let specific = match id {
            SET_ALPHA | THRESHOLD_ALPHA => key == "value",
            CLAMP_ALPHA => key == "low" || key == "high",
            ALPHA_CURVE => key == "values",
            ALPHA_TO_COLOR => false,
            PERCENTILE => matches!(key.as_str(), "radius" | "colorPercentile" | "alphaPercentile" | "shape" | "channels"),
            CHANNEL_NOISE => matches!(key.as_str(), "amounts" | "distribution" | "monochromatic" | "relative" | "seed"),
            CONVOLUTION => matches!(key.as_str(), "kernel" | "scale" | "offset" | "channels" | "alphaWeighted" | "edge"),
            _ => key == "connectivity" || key == "channel",
        };
        if specific {
            continue;
        }
        match key.as_str() {
            "coalesce" if value.is_string() => {}
            "layer" if value.as_u64().is_some() => {}
            "target" if matches!(value.as_str(), Some("pixels" | "mask" | "quickMask")) => {}
            "target"
                if value
                    .as_object()
                    .is_some_and(|m| m.len() == 1 && m.get("channel").is_some_and(|v| v.as_u64().is_some_and(|i| usize::try_from(i).is_ok()))) => {}
            _ => return Err(bad("unknown or invalid coverage parameter")),
        }
    }
    match id {
        CONVOLUTION => {
            let input =
                p.get("kernel").and_then(Value::as_array).filter(|v| v.len() == 25).ok_or_else(|| bad("kernel must contain exactly 25 finite coefficients"))?;
            let mut kernel = Vec::new();
            kernel.try_reserve_exact(25).map_err(|_| bad("convolution parameters unavailable"))?;
            for v in input {
                kernel.push(v.as_f64().filter(|v| v.is_finite()).ok_or_else(|| bad("kernel coefficients must be finite"))?);
            }
            let scale = p.get("scale").and_then(Value::as_f64).filter(|v| v.is_finite() && *v != 0.).ok_or_else(|| bad("scale must be finite and nonzero"))?;
            let offset =
                p.get("offset").and_then(Value::as_f64).filter(|v| v.is_finite()).ok_or_else(|| bad("offset must be finite normalized native units"))?;
            let input = p
                .get("channels")
                .and_then(Value::as_array)
                .filter(|v| !v.is_empty() && v.len() <= 5)
                .ok_or_else(|| bad("channels must contain 1..5 native channel booleans"))?;
            let mut channels = Vec::new();
            channels.try_reserve_exact(input.len()).map_err(|_| bad("convolution parameters unavailable"))?;
            for v in input {
                channels.push(v.as_bool().ok_or_else(|| bad("channels must contain booleans"))?);
            }
            let alpha_weighted = p.get("alphaWeighted").and_then(Value::as_bool).ok_or_else(|| bad("alphaWeighted must be boolean"))?;
            let edge = match p.get("edge").and_then(Value::as_str) {
                Some("transparent") => photocraft_algo::convolution::Edge::Transparent,
                Some("repeat") => photocraft_algo::convolution::Edge::Repeat,
                Some("wrap") => photocraft_algo::convolution::Edge::Wrap,
                Some("black") => photocraft_algo::convolution::Edge::Black,
                Some("white") => photocraft_algo::convolution::Edge::White,
                _ => return Err(bad("edge must be transparent, repeat, wrap, black or white")),
            };
            let options = photocraft_algo::convolution::Params { kernel, scale, offset, channels, alpha_weighted, edge };
            photocraft_algo::convolution::validate(&options, options.channels.len()).map_err(error)?;
            Ok(FilterParams::Convolution(options))
        }
        CHANNEL_NOISE => {
            let input = p
                .get("amounts")
                .and_then(Value::as_array)
                .filter(|v| !v.is_empty() && v.len() <= 5)
                .ok_or_else(|| bad("amounts must contain 1..5 native channel amounts"))?;
            let mut amounts = Vec::new();
            amounts.try_reserve_exact(input.len()).map_err(|_| bad("noise parameters unavailable"))?;
            for item in input {
                amounts
                    .push(item.as_f64().filter(|v| v.is_finite() && (0.0..=400.).contains(v)).ok_or_else(|| bad("amounts must be finite numbers in 0..400"))?
                        as f32);
            }
            let boolean = |name| p.get(name).and_then(Value::as_bool).ok_or_else(|| bad("monochromatic and relative must be booleans"));
            let distribution = match p.get("distribution").and_then(Value::as_str) {
                Some("uniform") => photocraft_algo::Distribution::Uniform,
                Some("gaussian") => photocraft_algo::Distribution::Gaussian,
                _ => return Err(bad("distribution must be uniform or gaussian")),
            };
            let seed = p
                .get("seed")
                .and_then(Value::as_f64)
                .filter(|n| n.is_finite() && n.fract() == 0. && (0.0..=f64::from(u32::MAX)).contains(n))
                .ok_or_else(|| bad("seed must be an unsigned 32-bit integer"))? as u32;
            Ok(FilterParams::ChannelNoise(photocraft_algo::channel_noise::Params {
                amounts,
                distribution,
                monochromatic: boolean("monochromatic")?,
                relative: boolean("relative")?,
                seed,
            }))
        }
        PERCENTILE => {
            let radius = p
                .get("radius")
                .and_then(Value::as_f64)
                .filter(|n| n.is_finite() && n.fract() == 0. && (1.0..=500.0).contains(n))
                .ok_or_else(|| bad("radius must be an integer in 1..500"))? as u16;
            let number = |name| {
                p.get(name)
                    .and_then(Value::as_f64)
                    .filter(|n| n.is_finite() && (0.0..=100.0).contains(n))
                    .map(|n| n as f32)
                    .ok_or_else(|| bad("percentiles must be finite numbers in 0..100"))
            };
            let shape = match p.get("shape").and_then(Value::as_str) {
                Some("disc") => photocraft_algo::rank::Neighborhood::Disc,
                Some("square") => photocraft_algo::rank::Neighborhood::Square,
                Some("diamond") => photocraft_algo::rank::Neighborhood::Diamond,
                _ => return Err(bad("shape must be disc, square or diamond")),
            };
            let channels = match p.get("channels") {
                None => None,
                Some(v) => {
                    let input = v.as_array().filter(|v| v.len() <= 5).ok_or_else(|| bad("channels must be 0..5 native booleans"))?;
                    let mut mask = Vec::new();
                    mask.try_reserve_exact(input.len()).map_err(|_| bad("native rank channel mask unavailable"))?;
                    for v in input {
                        mask.push(v.as_bool().ok_or_else(|| bad("channels must contain booleans"))?);
                    }
                    Some(mask)
                }
            };
            Ok(FilterParams::Percentile(photocraft_algo::rank::Params {
                radius,
                color_percentile: number("colorPercentile")?,
                alpha_percentile: number("alphaPercentile")?,
                shape,
                channels,
            }))
        }
        SET_ALPHA | THRESHOLD_ALPHA => {
            let value = p
                .get("value")
                .and_then(Value::as_f64)
                .filter(|n| n.is_finite() && (0.0..=1.0).contains(n))
                .ok_or_else(|| bad("value must be a finite number in 0..1"))?;
            if p.get("target").is_some_and(|v| v.as_str() != Some("pixels")) {
                return Err(bad(if id == SET_ALPHA { "Set Alpha requires layer pixels" } else { "Threshold Alpha requires layer pixels" }));
            }
            Ok(if id == SET_ALPHA { FilterParams::SetAlpha { value: value as f32 } } else { FilterParams::ThresholdAlpha { value: value as f32 } })
        }
        CLAMP_ALPHA => {
            let read = |key| {
                p.get(key)
                    .and_then(Value::as_f64)
                    .filter(|n| n.is_finite() && (0.0..=1.0).contains(n))
                    .ok_or_else(|| bad("low and high must be finite numbers in 0..1"))
            };
            let low = read("low")?;
            let high = read("high")?;
            if low > high {
                return Err(bad("low must not exceed high"));
            }
            if p.get("target").is_some_and(|v| v.as_str() != Some("pixels")) {
                return Err(bad("Clamp Alpha requires layer pixels"));
            }
            Ok(FilterParams::ClampAlpha { low: low as f32, high: high as f32 })
        }
        ALPHA_TO_COLOR => {
            if p.get("target").is_some_and(|v| v.as_str() != Some("pixels")) {
                return Err(bad("Alpha to Color requires layer pixels"));
            }
            Ok(FilterParams::AlphaToColor)
        }
        ALPHA_CURVE => {
            if p.get("target").is_some_and(|v| v.as_str() != Some("pixels")) {
                return Err(bad("Alpha Curve requires layer pixels"));
            }
            let input = p
                .get("values")
                .and_then(Value::as_array)
                .filter(|v| v.len() == 256)
                .ok_or_else(|| bad("values must contain exactly 256 finite numbers in 0..1"))?;
            let mut values = Vec::new();
            values.try_reserve_exact(256).map_err(|_| bad("alpha curve allocation unavailable"))?;
            for item in input {
                let value = item
                    .as_f64()
                    .filter(|v| v.is_finite() && (0.0..=1.0).contains(v))
                    .ok_or_else(|| bad("values must contain exactly 256 finite numbers in 0..1"))?;
                values.push(value as f32);
            }
            Ok(FilterParams::AlphaCurve { values })
        }
        FILL_HOLES => {
            let connectivity = match p.get("connectivity") {
                None => 4,
                Some(v) => v.as_f64().filter(|v| *v == 4.0 || *v == 8.0).ok_or_else(|| bad("connectivity must be 4 or 8"))? as u8,
            };
            let alpha_only = match p.get("channel").and_then(Value::as_str) {
                None if p.get("channel").is_none() => false,
                Some("each") => false,
                Some("alpha") => true,
                _ => return Err(bad("channel must be each or alpha")),
            };
            Ok(FilterParams::FillHoles { connectivity, alpha_only })
        }
        _ => Err(bad("unknown coverage filter")),
    }
}
pub(crate) fn operation(fp: &FilterParams) -> Option<coverage::Operation<'_>> {
    match fp {
        FilterParams::AlphaToColor => Some(coverage::Operation::AlphaToColor),
        FilterParams::AlphaCurve { values } => Some(coverage::Operation::AlphaCurve(values)),
        FilterParams::SetAlpha { value } => Some(coverage::Operation::SetAlpha(*value)),
        FilterParams::ClampAlpha { low, high } => Some(coverage::Operation::ClampAlpha { low: *low, high: *high }),
        FilterParams::ThresholdAlpha { value } => Some(coverage::Operation::ThresholdAlpha(*value)),
        FilterParams::FillHoles { connectivity, alpha_only } => Some(coverage::Operation::FillHoles { connectivity: *connectivity, alpha_only: *alpha_only }),
        _ => None,
    }
}
pub(crate) fn error(error: coverage::Error) -> EngineError {
    if error == coverage::Error::Cancelled { EngineError::Cancelled } else { EngineError::Other(error.to_string()) }
}
fn alpha_enabled(s: &Session) -> std::result::Result<(), String> {
    if crate::channel_cmds::edits_channel(s) {
        return Err("Alpha coverage filters work on layer pixels".into());
    }
    crate::filters::has_filterable_layer(s)?;
    let d = s.active().ok_or("no document open")?;
    if d.channel_view.target == crate::channel_cmds::ChannelTarget::LayerMask {
        return Err("Alpha coverage filters work on layer pixels, not masks".into());
    }
    let layer = crate::active_layer_of(s)?;
    let locks = d.doc.effective_locks(layer.id);
    if locks.pixels || locks.all || (locks.transparency && !d.doc.is_background(layer.id)) {
        return Err("the layer pixels or transparency are locked".into());
    }
    Ok(())
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    params(SET_ALPHA, p)?;
    crate::filters::run_filter(s, SET_ALPHA, p)
}
fn fill(s: &mut Session, p: &Value) -> Result<Value> {
    params(FILL_HOLES, p)?;
    crate::filters::run_filter(s, FILL_HOLES, p)
}
fn clamp(s: &mut Session, p: &Value) -> Result<Value> {
    params(CLAMP_ALPHA, p)?;
    crate::filters::run_filter(s, CLAMP_ALPHA, p)
}
fn threshold(s: &mut Session, p: &Value) -> Result<Value> {
    params(THRESHOLD_ALPHA, p)?;
    crate::filters::run_filter(s, THRESHOLD_ALPHA, p)
}
fn curve(s: &mut Session, p: &Value) -> Result<Value> {
    params(ALPHA_CURVE, p)?;
    crate::filters::run_filter(s, ALPHA_CURVE, p)
}
fn to_color(s: &mut Session, p: &Value) -> Result<Value> {
    params(ALPHA_TO_COLOR, p)?;
    crate::filters::run_filter(s, ALPHA_TO_COLOR, p)
}
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: CONVOLUTION,
            label: "Convolution…",
            menu: &[],
            shortcut: None,
            params: r#"{"kernel":[25 finite coefficients],"scale":nonzero,"offset":normalized,"channels":[native channel booleans,alpha last],"alphaWeighted":bool,"edge":"transparent|repeat|wrap|black|white"}"#,
            enabled: crate::filters::has_filterable_layer,
            run: |s, p| {
                params(CONVOLUTION, p)?;
                crate::filters::run_filter(s, CONVOLUTION, p)
            },
            journal: true,
        },
        CommandSpec {
            id: CHANNEL_NOISE,
            label: "Channel Noise…",
            menu: &[],
            shortcut: None,
            params: r#"{"amounts":[native channels 0..400,alpha last],"distribution":"uniform|gaussian","monochromatic":bool,"relative":bool,"seed":u32}"#,
            enabled: crate::filters::has_filterable_layer,
            run: |s, p| {
                params(CHANNEL_NOISE, p)?;
                crate::filters::run_filter(s, CHANNEL_NOISE, p)
            },
            journal: true,
        },
        CommandSpec {
            id: PERCENTILE,
            label: "Percentile…",
            menu: &[],
            shortcut: None,
            params: r#"{"radius":1..500,"colorPercentile":0..100,"alphaPercentile":0..100,"shape":"disc|square|diamond","channels":[bool]?=all}"#,
            enabled: crate::filters::has_filterable_layer,
            run: |s, p| {
                params(PERCENTILE, p)?;
                crate::filters::run_filter(s, PERCENTILE, p)
            },
            journal: true,
        },
        CommandSpec {
            id: ALPHA_TO_COLOR,
            label: "Alpha to Color",
            menu: &[],
            shortcut: None,
            params: r#"{} (native neutral stored channels; opaque output)"#,
            enabled: alpha_enabled,
            run: to_color,
            journal: true,
        },
        CommandSpec {
            id: ALPHA_CURVE,
            label: "Alpha Curve",
            menu: &[],
            shortcut: None,
            params: r#"{"values":[256 finite samples in 0..1]}"#,
            enabled: alpha_enabled,
            run: curve,
            journal: true,
        },
        CommandSpec {
            id: CLAMP_ALPHA,
            label: "Clamp Alpha…",
            menu: &[],
            shortcut: None,
            params: r#"{"low":0..1,"high":low..1}"#,
            enabled: alpha_enabled,
            run: clamp,
            journal: true,
        },
        CommandSpec {
            id: THRESHOLD_ALPHA,
            label: "Threshold Alpha…",
            menu: &[],
            shortcut: None,
            params: r#"{"value":0..1} (native alpha > value becomes 1; equality becomes 0)"#,
            enabled: alpha_enabled,
            run: threshold,
            journal: true,
        },
        CommandSpec {
            id: SET_ALPHA,
            label: "Set Alpha…",
            menu: &[],
            shortcut: None,
            params: r#"{"value":0..1}"#,
            enabled: alpha_enabled,
            run: set,
            journal: true,
        },
        CommandSpec {
            id: FILL_HOLES,
            label: "Fill Holes…",
            menu: &[],
            shortcut: None,
            params: r#"{"connectivity":4|8=4,"channel":"each|alpha"="each"}"#,
            enabled: crate::filters::has_filterable_layer,
            run: fill,
            journal: true,
        },
    ]
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn native_enum_controls_require_explicit_valid_values() {
        let mut kernel = vec![0.; 25];
        kernel[12] = 1.;
        for (id, key, good) in [
            (CONVOLUTION, "edge", json!({"kernel":kernel,"scale":1,"offset":0,"channels":[true,true,true,false],"alphaWeighted":false,"edge":"repeat"})),
            (CHANNEL_NOISE, "distribution", json!({"amounts":[10,10,10,0],"distribution":"uniform","monochromatic":false,"relative":false,"seed":1})),
            (PERCENTILE, "shape", json!({"radius":1,"colorPercentile":50,"alphaPercentile":50,"shape":"disc"})),
        ] {
            assert!(params(id, &good).is_ok());
            let mut missing = good.clone();
            missing.as_object_mut().unwrap().remove(key);
            assert!(params(id, &missing).is_err(), "{id} requires {key}");
            for invalid in [json!(null), json!(false), json!(7), json!("unknown")] {
                let mut value = good.clone();
                value[key] = invalid;
                assert!(params(id, &value).is_err(), "{id}: {value}");
            }
        }
    }

    #[test]
    fn coverage_filter_drag_coalesces_and_undo_restores_original_document() {
        let mut session = Session::new();
        session.execute("file.new", json!({"width":2,"height":2,"depth":32})).unwrap();
        session.execute("layer.new.layer", json!({})).unwrap();
        let original = session.active().unwrap().doc.clone();
        let history = session.active().unwrap().history.past_len();
        session.execute(SET_ALPHA, json!({"value":0.25,"coalesce":"coverage-drag"})).unwrap();
        session.execute(SET_ALPHA, json!({"value":0.75,"coalesce":"coverage-drag"})).unwrap();
        assert_eq!(session.active().unwrap().history.past_len(), history + 1);
        let id = session.active().unwrap().active_layer.unwrap();
        assert_eq!(session.active().unwrap().doc.layer(id).unwrap().surface().unwrap().pixel(0, 0)[3], 0.75);
        session.undo();
        assert_eq!(*session.active().unwrap().doc, *original);
        assert!(params(SET_ALPHA, &json!({"value":0.5,"coalesce":7})).is_err());
        let error = params(THRESHOLD_ALPHA, &json!({"value":0.5,"target":"mask"})).unwrap_err().to_string();
        assert!(error.contains("Threshold Alpha requires layer pixels"), "{error}");
    }

    #[test]
    fn color_only_noise_and_convolution_preserve_background_and_transparency_locks() {
        let mut kernel = vec![0.; 25];
        kernel[12] = 0.5;
        for depth in [8, 16, 32] {
            for (id, good, alpha_key, alpha_values) in [
                (
                    CONVOLUTION,
                    json!({"kernel":kernel,"scale":1,"offset":0,"channels":[true,true,true,false],"alphaWeighted":false,"edge":"repeat"}),
                    "channels",
                    json!([true, true, true, true]),
                ),
                (
                    CHANNEL_NOISE,
                    json!({"amounts":[30,30,30,0],"distribution":"uniform","monochromatic":false,"relative":false,"seed":71}),
                    "amounts",
                    json!([30, 30, 30, 100]),
                ),
            ] {
                let mut session = Session::new();
                session.execute("file.new", json!({"width":2,"height":2,"depth":depth,"background":"white"})).unwrap();
                let background = session.active().unwrap().active_layer.unwrap();
                assert!(session.active().unwrap().doc.is_background(background));
                session.execute(id, good.clone()).unwrap();
                assert!(session.active().unwrap().doc.is_background(background), "{id} {depth}");

                session.execute("layer.new.layer", json!({})).unwrap();
                let layer = session.active().unwrap().active_layer.unwrap();
                session
                    .edit("source", |doc, _| {
                        doc.layer_mut(layer).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(0, 0, 2, 2), &[0.4, 0.3, 0.2, 0.6]);
                        Ok(())
                    })
                    .unwrap();
                session.execute("layer.lockLayers", json!({"transparency":true})).unwrap();
                let alpha = session.active().unwrap().doc.layer(layer).unwrap().surface().unwrap().pixel(0, 0)[3];
                session.execute(id, good.clone()).unwrap();
                assert_eq!(session.active().unwrap().doc.layer(layer).unwrap().surface().unwrap().pixel(0, 0)[3], alpha);
                let before = session.active().unwrap().doc.clone();
                let history = session.active().unwrap().history.past_len();
                let mut changes_alpha = good.clone();
                changes_alpha[alpha_key] = alpha_values;
                assert!(session.execute(id, changes_alpha).is_err());
                assert_eq!(*session.active().unwrap().doc, *before);
                assert_eq!(session.active().unwrap().history.past_len(), history);
            }
        }
    }
    #[test]
    fn strict_recorded_parameters_refuse_hostile_values() {
        for (id, values) in [
            (
                SET_ALPHA,
                vec![
                    json!(null),
                    json!({}),
                    json!({"value":-1}),
                    json!({"value":2}),
                    json!({"value":"0"}),
                    json!({"value":1e300}),
                    json!({"value":1,"target":"mask"}),
                ],
            ),
            (FILL_HOLES, vec![json!(null), json!({"connectivity":0}), json!({"connectivity":4.5}), json!({"channel":null}), json!({"channel":"blue"})]),
        ] {
            for p in values {
                assert!(params(id, &p).is_err(), "{id} {p}");
            }
        }
    }
    #[test]
    fn native_commands_are_atomic_and_undoable() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":3,"height":3,"depth":32})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.edit("source", |d, id| {
            d.layer_mut(id.unwrap()).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(0, 0, 3, 3), &[-2., 3., 0.4, 0.]);
            Ok(())
        })
        .unwrap();
        let before = s.active().unwrap().history.past_len();
        s.execute(SET_ALPHA, json!({"value":0.4})).unwrap();
        let pixel = |s: &Session| {
            let d = s.active().unwrap();
            d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().pixel(1, 1)
        };
        assert_eq!(pixel(&s), vec![-2., 3., 0.4, 0.4]);
        assert_eq!(s.active().unwrap().history.past_len(), before + 1);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(pixel(&s), vec![-2., 3., 0.4, 0.]);
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(pixel(&s)[3], 0.4);
    }
    #[test]
    fn empty_document_and_adversarial_dispatch_never_panic() {
        for id in [SET_ALPHA, CLAMP_ALPHA, THRESHOLD_ALPHA, ALPHA_CURVE, ALPHA_TO_COLOR, FILL_HOLES] {
            for p in [json!({}), json!(null), json!({"value":-1,"connectivity":999})] {
                let result = std::panic::catch_unwind(|| {
                    let mut s = Session::new();
                    s.execute(id, p)
                });
                assert!(result.is_ok());
                assert!(result.unwrap().is_err());
            }
        }
    }
    #[test]
    fn checked_smart_replay_preserves_errors_cancellation_and_native_filters() {
        use photocraft_color::{ColorMode, PixelFormat, SampleType};
        use photocraft_geom::Rect;
        use photocraft_raster::{Interrupt, Surface};
        let r = Rect::new(0, 0, 3, 3);
        let mut surf = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::F32, true));
        surf.fill_rect(r, &[0.2, 0.4, 2., 1.]);
        surf.write_pixel(1, 1, &[0.2, 0.4, 2., 0.]);
        let before = surf.read_region(r);
        for params in [json!({"channel":"alpha","extra":1}), json!({"connectivity":7}), json!({"channel":null})] {
            assert!(crate::filters::apply_filter_to_surface(FILL_HOLES, &params, &surf, r, None, r).is_err());
        }
        let cancel = || true;
        assert!(matches!(
            crate::filters::apply_filter_to_surface_in(FILL_HOLES, &json!({"channel":"alpha"}), &surf, r, None, r, &Interrupt::cancel_only(&cancel)),
            Err(EngineError::Cancelled)
        ));
        assert!(crate::filters::apply_filter_to_surface(FILL_HOLES, &json!({}), &surf, r, None, Rect::new(i32::MIN, 0, i32::MAX, 3)).is_err());
        assert_eq!(surf.read_region(r), before);
        let got = crate::filters::apply_filter_to_surface(FILL_HOLES, &json!({"channel":"alpha"}), &surf, r, None, r).unwrap().unwrap();
        assert_eq!(got.pixel(1, 1), vec![0.2, 0.4, 2., 1.]);
    }
    #[test]
    fn ordinary_blur_smart_replay_honors_mid_work_cancellation() {
        use photocraft_color::{ColorMode, PixelFormat, SampleType};
        use photocraft_geom::Rect;
        use photocraft_raster::{Interrupt, Surface};
        let r = Rect::new(0, 0, 320, 96);
        let mut source = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::F32, true));
        source.fill_rect(r, &[-2., 3., 0.4, 0.75]);
        let before = source.clone();
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let cancel = || calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) > 3;
        assert!(matches!(
            crate::filters::apply_filter_to_surface_in("filter.blur.boxBlur", &json!({"radius":8}), &source, r, None, r, &Interrupt::cancel_only(&cancel)),
            Err(EngineError::Cancelled)
        ));
        assert!(calls.load(std::sync::atomic::Ordering::Relaxed) > 3);
        assert_eq!(source, before);
    }
    #[test]
    fn ordinary_smart_object_keeps_editable_native_coverage_filters() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":3,"height":3,"depth":32})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.edit("nonempty native source", |doc, id| {
            doc.layer_mut(id.unwrap()).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(0, 0, 3, 3), &[-2., 3., 0.4, 0.5]);
            Ok(())
        })
        .unwrap();
        s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        s.execute(SET_ALPHA, json!({"value":0.5})).unwrap();
        s.execute(CLAMP_ALPHA, json!({"low":0.25,"high":0.75})).unwrap();
        s.execute(THRESHOLD_ALPHA, json!({"value":0.25})).unwrap();
        s.execute(FILL_HOLES, json!({"channel":"alpha"})).unwrap();
        s.execute(ALPHA_CURVE, json!({"values":(0..256).map(|i|i as f32/255.).collect::<Vec<_>>()})).unwrap();
        s.execute(ALPHA_TO_COLOR, json!({})).unwrap();
        let d = s.active().unwrap();
        let photocraft_doc::LayerContent::Smart(sm) = &d.doc.layer(d.active_layer.unwrap()).unwrap().content else { panic!("ordinary smart object") };
        assert_eq!(sm.smart_filters.len(), 6);
        assert_eq!(sm.smart_filters[0].command, SET_ALPHA);
        assert_eq!(sm.smart_filters[1].command, CLAMP_ALPHA);
        assert_eq!(sm.smart_filters[2].command, THRESHOLD_ALPHA);
        assert_eq!(sm.smart_filters[3].command, FILL_HOLES);
        assert_eq!(sm.smart_filters[4].command, ALPHA_CURVE);
        assert_eq!(sm.smart_filters[5].command, ALPHA_TO_COLOR);
        let saved = photocraft_format::save_to_bytes(&d.doc, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
        let photocraft_doc::LayerContent::Smart(reopened_sm) = &reopened.layer(d.active_layer.unwrap()).unwrap().content else { panic!("native smart reopen") };
        assert_eq!(reopened_sm.smart_filters, sm.smart_filters);

        assert!((sm.cache.as_ref().unwrap().pixel(1, 1).last().unwrap() - 1.0).abs() < 0.003);
    }
    #[test]
    fn last_filter_repeats_strict_coverage_params() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":3,"height":3})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        for (id, p) in [
            (SET_ALPHA, json!({"value":0.25})),
            (CLAMP_ALPHA, json!({"low":0.2,"high":0.8})),
            (THRESHOLD_ALPHA, json!({"value":0.5})),
            (ALPHA_CURVE, json!({"values":(0..256).map(|i|i as f32/255.).collect::<Vec<_>>()})),
            (ALPHA_TO_COLOR, json!({})),
            (FILL_HOLES, json!({"channel":"alpha"})),
            (PERCENTILE, json!({"radius":1,"colorPercentile":37,"alphaPercentile":83,"shape":"diamond"})),
            (CHANNEL_NOISE, json!({"amounts":[20,40,60,80],"distribution":"uniform","monochromatic":false,"relative":true,"seed":71})),
            (
                CONVOLUTION,
                json!({"kernel":(0..25).map(|i|if i==12{1.}else{0.}).collect::<Vec<_>>(),"scale":1,"offset":0,"channels":[true,true,true,true],"alphaWeighted":false,"edge":"repeat"}),
            ),
        ] {
            s.execute(id, p).unwrap();
            let before = s.active().unwrap().history.past_len();
            assert!(s.is_enabled("filter.lastFilter"));
            s.execute("filter.lastFilter", json!({})).unwrap();
            assert_eq!(s.active().unwrap().history.past_len(), before + 1);
        }
    }
    #[test]
    fn convolution_strict_params_native_channel_history_and_smart_reopen() {
        let good = json!({"kernel":(0..25).map(|i|if i==12{1.}else{0.}).collect::<Vec<_>>(),"scale":1,"offset":0.25,"channels":[true,false,true,true],"alphaWeighted":true,"edge":"repeat"});
        for (key, bad) in [
            ("kernel", json!([])),
            ("kernel", json!([1, 2, 3])),
            ("scale", json!(0)),
            ("offset", json!("0")),
            ("channels", json!([1, 0, 1, 0])),
            ("alphaWeighted", json!(1)),
            ("edge", json!("unknown")),
            ("extra", json!(0)),
        ] {
            let mut p = good.clone();
            p[key] = bad;
            assert!(params(CONVOLUTION, &p).is_err());
            let result = std::panic::catch_unwind(|| Session::new().execute(CONVOLUTION, p));
            assert!(result.is_ok());
            assert!(result.unwrap().is_err());
        }
        let mut s = Session::new();
        s.execute("file.new", json!({"width":3,"height":3,"depth":32})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.edit("convolution source", |doc, id| {
            doc.layer_mut(id.unwrap()).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(0, 0, 3, 3), &[-2., 3., 0.4, 0.5]);
            Ok(())
        })
        .unwrap();
        let before = s.active().unwrap().history.past_len();
        let mut wrong = good.clone();
        wrong["channels"] = json!([true, true, true]);
        assert!(s.execute(CONVOLUTION, wrong).is_err());
        assert_eq!(s.active().unwrap().history.past_len(), before);
        s.execute(CONVOLUTION, good.clone()).unwrap();
        let pixel = |s: &Session| {
            let d = s.active().unwrap();
            d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().pixel(1, 1)
        };
        let got = pixel(&s);
        assert_eq!(&got[..2], &[-1.75, 3.]);
        assert!((got[2] - 0.65).abs() < 1e-6);
        assert_eq!(got[3], 0.75);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(pixel(&s), vec![-2., 3., 0.4, 0.5]);
        s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        s.execute(CONVOLUTION, good.clone()).unwrap();
        let d = s.active().unwrap();
        let photocraft_doc::LayerContent::Smart(sm) = &d.doc.layer(d.active_layer.unwrap()).unwrap().content else { panic!("native smart") };
        assert_eq!(sm.smart_filters[0].command, CONVOLUTION);
        assert_eq!(sm.smart_filters[0].params, good);
        let bytes = photocraft_format::save_to_bytes(&d.doc, &Default::default()).unwrap();
        let restored = photocraft_format::load_from_bytes(&bytes).unwrap();
        let photocraft_doc::LayerContent::Smart(restored) = &restored.layer(d.active_layer.unwrap()).unwrap().content else { panic!("native reopen") };
        assert_eq!(restored.smart_filters, sm.smart_filters);
    }
    #[test]
    fn generated_integer_parameter_representations_are_equivalent_and_strict() {
        for value in [json!(4), json!(4.0), json!(8), json!(8.0)] {
            assert!(matches!(params(FILL_HOLES, &json!({"connectivity":value})).unwrap(), FilterParams::FillHoles { connectivity: 4 | 8, .. }));
        }
        for value in [json!(4.5), json!(7.0), json!(-4), json!("4"), json!(true), json!(null)] {
            assert!(params(FILL_HOLES, &json!({"connectivity":value})).is_err());
        }
        for value in [json!(0), json!(0.0), json!(71.0), json!(u32::MAX), json!(f64::from(u32::MAX))] {
            let p = json!({"amounts":[0,0,0,1],"distribution":"uniform","monochromatic":false,"relative":false,"seed":value});
            let FilterParams::ChannelNoise(parsed) = params(CHANNEL_NOISE, &p).unwrap() else { panic!("native channel noise") };
            assert_eq!(f64::from(parsed.seed), value.as_f64().unwrap());
        }
        for value in [json!(-1), json!(0.5), json!(4294967296u64), json!("71"), json!(true), json!(null)] {
            assert!(params(CHANNEL_NOISE, &json!({"amounts":[0,0,0,1],"distribution":"uniform","monochromatic":false,"relative":false,"seed":value})).is_err());
        }
    }
    #[test]
    fn channel_noise_strict_model_count_history_smart_reopen() {
        let good = json!({"amounts":[0,0,0,200],"distribution":"uniform","monochromatic":false,"relative":false,"seed":71});
        for bad in [
            json!(null),
            json!({}),
            json!({"amounts":[0,0,0,401],"distribution":"uniform","monochromatic":false,"relative":false,"seed":71}),
            json!({"amounts":[0,0,0,1],"distribution":"unknown","monochromatic":false,"relative":false,"seed":71}),
            json!({"amounts":[0,0,0,1],"distribution":"uniform","monochromatic":0,"relative":false,"seed":71}),
            json!({"amounts":[0,0,0,1],"distribution":"uniform","monochromatic":false,"relative":false,"seed":-1}),
            json!({"amounts":[0,0,0,1],"distribution":"uniform","monochromatic":false,"relative":false,"seed":71,"extra":true}),
        ] {
            assert!(params(CHANNEL_NOISE, &bad).is_err());
            let result = std::panic::catch_unwind(|| Session::new().execute(CHANNEL_NOISE, bad));
            assert!(result.is_ok());
            assert!(result.unwrap().is_err());
        }
        let mut s = Session::new();
        s.execute("file.new", json!({"width":3,"height":3,"depth":32})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.edit("noise source", |doc, id| {
            doc.layer_mut(id.unwrap()).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(0, 0, 3, 3), &[-2., 3., 0.4, 0.5]);
            Ok(())
        })
        .unwrap();
        let before = s.active().unwrap().history.past_len();
        let mut wrong = good.clone();
        wrong["amounts"] = json!([0, 0, 0]);
        assert!(s.execute(CHANNEL_NOISE, wrong).is_err());
        assert_eq!(s.active().unwrap().history.past_len(), before);
        s.execute(CHANNEL_NOISE, good.clone()).unwrap();
        let pixel = |s: &Session| {
            let d = s.active().unwrap();
            d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().pixel(1, 1)
        };
        let value = pixel(&s);
        assert_eq!(&value[..3], &[-2., 3., 0.4]);
        assert!((0.0..=1.0).contains(&value[3]));
        assert_ne!(value[3], 0.5);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(pixel(&s), vec![-2., 3., 0.4, 0.5]);
        s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        s.execute(CHANNEL_NOISE, good.clone()).unwrap();
        let d = s.active().unwrap();
        let photocraft_doc::LayerContent::Smart(sm) = &d.doc.layer(d.active_layer.unwrap()).unwrap().content else { panic!("native smart") };
        assert_eq!(sm.smart_filters[0].command, CHANNEL_NOISE);
        assert_eq!(sm.smart_filters[0].params, good);
        let saved = photocraft_format::save_to_bytes(&d.doc, &Default::default()).unwrap();
        let restored = photocraft_format::load_from_bytes(&saved).unwrap();
        let photocraft_doc::LayerContent::Smart(restored) = &restored.layer(d.active_layer.unwrap()).unwrap().content else { panic!("native reopen") };
        assert_eq!(restored.smart_filters, sm.smart_filters);
    }
    #[test]
    fn percentile_channel_selection_respects_transparency_locks_and_native_smart_params() {
        let good = json!({"radius":1,"colorPercentile":0,"alphaPercentile":100,"shape":"square","channels":[true,false,false,false]});
        for mask in [json!(null), json!(true), json!([1, 0, 1, 0]), json!([true, true, true, true, true, true])] {
            let mut p = good.clone();
            p["channels"] = mask;
            assert!(params(PERCENTILE, &p).is_err());
        }
        let mut s = Session::new();
        s.execute("file.new", json!({"width":3,"height":3,"depth":32})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.edit("rank masked source", |d, id| {
            let source = d.layer_mut(id.unwrap()).unwrap().surface_mut().unwrap();
            source.fill_rect(photocraft_geom::Rect::new(0, 0, 3, 3), &[-2., 3., 0.4, 0.25]);
            source.write_pixel(1, 1, &[4., -0., -3.5, 0.75]);
            Ok(())
        })
        .unwrap();
        s.execute("layer.lockLayers", json!({"transparency":true})).unwrap();
        let pixel = |s: &Session| {
            let d = s.active().unwrap();
            d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().pixel(1, 1)
        };
        let original = pixel(&s);
        s.execute(PERCENTILE, good.clone()).unwrap();
        let got = pixel(&s);
        assert_eq!(got[0], -2.);
        for c in 1..4 {
            assert_eq!(got[c].to_bits(), original[c].to_bits());
        }
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(pixel(&s), original);
        let mut alpha = good.clone();
        alpha["channels"] = json!([false, false, false, true]);
        let before = s.active().unwrap().history.past_len();
        assert!(s.execute(PERCENTILE, alpha).is_err());
        assert_eq!(s.active().unwrap().history.past_len(), before);
        assert_eq!(pixel(&s), original);
        s.execute("layer.lockLayers", json!({"transparency":false})).unwrap();
        s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        s.execute(PERCENTILE, good.clone()).unwrap();
        let d = s.active().unwrap();
        let photocraft_doc::LayerContent::Smart(sm) = &d.doc.layer(d.active_layer.unwrap()).unwrap().content else { panic!("native masked rank") };
        assert_eq!(sm.smart_filters[0].params, good);
        let bytes = photocraft_format::save_to_bytes(&d.doc, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
        let photocraft_doc::LayerContent::Smart(restored) = &reopened.layer(d.active_layer.unwrap()).unwrap().content else { panic!("native reopen") };
        assert_eq!(restored.smart_filters, sm.smart_filters);
    }
    #[test]
    fn percentile_strict_dispatch_native_history_and_editable_reopen() {
        let good = json!({"radius":1,"colorPercentile":0,"alphaPercentile":100,"shape":"square"});
        let mut floating_radius = good.clone();
        floating_radius["radius"] = json!(1.0);
        assert_eq!(params(PERCENTILE, &floating_radius).unwrap(), params(PERCENTILE, &good).unwrap());
        for bad in [
            json!(null),
            json!({}),
            json!({"radius":1.5,"colorPercentile":50,"alphaPercentile":50,"shape":"disc"}),
            json!({"radius":501,"colorPercentile":50,"alphaPercentile":50,"shape":"disc"}),
            json!({"radius":1,"colorPercentile":101,"alphaPercentile":50,"shape":"disc"}),
            json!({"radius":1,"colorPercentile":50,"alphaPercentile":50,"shape":"unknown"}),
            json!({"radius":1,"colorPercentile":50,"alphaPercentile":50,"shape":"disc","extra":0}),
        ] {
            assert!(params(PERCENTILE, &bad).is_err());
            let result = std::panic::catch_unwind(|| Session::new().execute(PERCENTILE, bad));
            assert!(result.is_ok());
            assert!(result.unwrap().is_err());
        }
        let mut s = Session::new();
        s.execute("file.new", json!({"width":3,"height":3,"depth":32})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.edit("rank source", |d, id| {
            let surf = d.layer_mut(id.unwrap()).unwrap().surface_mut().unwrap();
            surf.fill_rect(photocraft_geom::Rect::new(0, 0, 3, 3), &[-2., 3., 0.4, 0.25]);
            surf.write_pixel(1, 1, &[4., -3., 0.9, 0.75]);
            Ok(())
        })
        .unwrap();
        let pixel = |s: &Session| {
            let d = s.active().unwrap();
            d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().pixel(1, 1)
        };
        let original = pixel(&s);
        let before = s.active().unwrap().history.past_len();
        s.execute(PERCENTILE, good.clone()).unwrap();
        assert_eq!(pixel(&s), vec![-2., -3., 0.4, 0.75]);
        assert_eq!(s.active().unwrap().history.past_len(), before + 1);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(pixel(&s), original);
        s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        s.execute(PERCENTILE, good.clone()).unwrap();
        let d = s.active().unwrap();
        let photocraft_doc::LayerContent::Smart(sm) = &d.doc.layer(d.active_layer.unwrap()).unwrap().content else { panic!("native smart") };
        assert_eq!(sm.smart_filters.len(), 1);
        assert_eq!(sm.smart_filters[0].command, PERCENTILE);
        assert_eq!(sm.smart_filters[0].params, good);
        let saved = photocraft_format::save_to_bytes(&d.doc, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
        let photocraft_doc::LayerContent::Smart(restored) = &reopened.layer(d.active_layer.unwrap()).unwrap().content else { panic!("native reopen") };
        assert_eq!(restored.smart_filters, sm.smart_filters);
    }
    #[test]
    fn clamp_threshold_strict_params_and_history() {
        for (id, bad) in [
            (CLAMP_ALPHA, json!({"low":0.8,"high":0.2})),
            (CLAMP_ALPHA, json!({"low":null,"high":1})),
            (THRESHOLD_ALPHA, json!({"value":true})),
            (THRESHOLD_ALPHA, json!({"value":0.5,"extra":1})),
        ] {
            assert!(params(id, &bad).is_err());
        }
        let mut s = Session::new();
        s.execute("file.new", json!({"width":3,"height":3,"depth":32})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute(SET_ALPHA, json!({"value":0.5})).unwrap();
        for (id, p, expected) in [(CLAMP_ALPHA, json!({"low":0.7,"high":0.8}), 0.7), (THRESHOLD_ALPHA, json!({"value":0.5}), 0.0)] {
            s.execute(id, p).unwrap();
            let alpha = |s: &Session| {
                let d = s.active().unwrap();
                d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().pixel(1, 1)[3]
            };
            assert_eq!(alpha(&s), expected);
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(alpha(&s), 0.5);
        }
    }
    #[test]
    fn alpha_transfer_strict_bounds_and_native_levels_authored_curve_are_undoable() {
        for p in [
            json!({}),
            json!({"values":[]}),
            json!({"values":vec![0.;255]}),
            json!({"values":vec![0.;257]}),
            json!({"values":vec![1.1;256]}),
            json!({"values":vec![0.5;256],"extra":1}),
        ] {
            assert!(params(ALPHA_CURVE, &p).is_err());
        }
        for p in [json!({"value":1}), json!({"target":"mask"}), json!(null)] {
            assert!(params(ALPHA_TO_COLOR, &p).is_err());
        }
        let ramp_rect = photocraft_geom::Rect::new(0, 0, 256, 1);
        let mut ramp = photocraft_raster::Surface::new(photocraft_color::PixelFormat::new(
            photocraft_color::ColorMode::Grayscale,
            photocraft_color::SampleType::F32,
            false,
        ));
        ramp.write_region(ramp_rect, &(0..256).map(|i| i as f32 / 255.).collect::<Vec<_>>());
        let native = crate::adjust_cmds::adjust_as_filter("levels", &json!({"gamma":2.0}), &ramp).unwrap();
        let values = native.read_region(ramp_rect);
        assert_eq!(values.len(), 256);
        let mut s = Session::new();
        s.execute("file.new", json!({"width":3,"height":3,"depth":32})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.edit("source", |d, id| {
            d.layer_mut(id.unwrap()).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(0, 0, 3, 3), &[-2., 3., 0.4, 64. / 255.]);
            Ok(())
        })
        .unwrap();
        let pixel = |s: &Session| {
            let d = s.active().unwrap();
            d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().pixel(1, 1)
        };
        s.execute(ALPHA_CURVE, json!({"values":values})).unwrap();
        assert_eq!(&pixel(&s)[..3], &[-2., 3., 0.4]);
        assert!((pixel(&s)[3] - native.pixel(64, 0)[0]).abs() < 1e-6);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(pixel(&s)[3], 64. / 255.);
        s.execute(ALPHA_TO_COLOR, json!({})).unwrap();
        assert_eq!(pixel(&s), vec![64. / 255., 64. / 255., 64. / 255., 1.]);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(pixel(&s), vec![-2., 3., 0.4, 64. / 255.]);
    }
}
