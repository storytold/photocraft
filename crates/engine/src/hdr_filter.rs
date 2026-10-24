//! Ordinary editable native local HDR tone adaptation; independent of any source format.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use photocraft_algo::tone::{HdrToning, checked_hdr};
use serde_json::Value;
pub const ID: &str = "filter.other.hdrToning";
pub(crate) fn error(e: checked_hdr::Error) -> EngineError {
    if e == checked_hdr::Error::Cancelled { EngineError::Cancelled } else { EngineError::Other(e.to_string()) }
}
pub(crate) fn params(p: &Value) -> Result<HdrToning> {
    let bad = |msg: &str| EngineError::BadParams { cmd: ID.into(), msg: msg.into() };
    let object = p.as_object().ok_or_else(|| bad("parameters must be an object"))?;
    for (key, value) in object {
        match key.as_str() {
            "radius" | "strength" | "gamma" | "exposure" | "detail" | "shadow" | "highlight" | "vibrance" | "saturation" | "curve" => {}
            "layer" if value.as_u64().is_some() => {}
            "coalesce" if value.is_string() => {}
            "target" if matches!(value.as_str(), Some("pixels" | "mask" | "quickMask")) => {}
            "target"
                if value.as_object().is_some_and(|m| m.len() == 1 && m.get("channel").and_then(Value::as_u64).is_some_and(|v| usize::try_from(v).is_ok())) => {}
            _ => return Err(bad("unknown or invalid native HDR parameter")),
        }
    }
    let d = HdrToning::default();
    let read = |key: &str, default: f32| -> Result<f32> {
        match p.get(key) {
            None => Ok(default),
            Some(v) => v
                .as_f64()
                .filter(|v| v.is_finite() && v.abs() <= f64::from(f32::MAX))
                .map(|v| v as f32)
                .ok_or_else(|| bad("HDR controls require finite numbers")),
        }
    };
    let curve = match p.get("curve") {
        None => Vec::new(),
        Some(v) => {
            let a = v.as_array().filter(|a| a.len() <= 4096).ok_or_else(|| bad("curve must contain at most 4096 [input, output] pairs in 0..1"))?;
            let mut result = Vec::new();
            result.try_reserve_exact(a.len()).map_err(|_| bad("HDR curve allocation unavailable"))?;
            for point in a {
                let pair = point.as_array().filter(|a| a.len() == 2).ok_or_else(|| bad("curve requires [input, output] pairs"))?;
                let x =
                    pair.first().and_then(Value::as_f64).filter(|v| v.is_finite() && (0. ..=1.).contains(v)).ok_or_else(|| bad("curve input must be 0..1"))?;
                let y =
                    pair.get(1).and_then(Value::as_f64).filter(|v| v.is_finite() && (0. ..=1.).contains(v)).ok_or_else(|| bad("curve output must be 0..1"))?;
                result.push((x as f32, y as f32));
            }
            result
        }
    };
    let result = HdrToning {
        radius: read("radius", d.radius)?,
        strength: read("strength", d.strength)?,
        gamma: read("gamma", d.gamma)?,
        exposure: read("exposure", d.exposure)?,
        detail: read("detail", d.detail)?,
        shadow: read("shadow", d.shadow)?,
        highlight: read("highlight", d.highlight)?,
        vibrance: read("vibrance", d.vibrance)?,
        saturation: read("saturation", d.saturation)?,
        curve,
    };
    checked_hdr::validate(&result).map_err(|_| bad("native HDR control is outside its documented range"))?;
    Ok(result)
}
fn run(s: &mut Session, p: &Value) -> Result<Value> {
    params(p)?;
    crate::filters::run_filter(s, ID, p)
}
pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: ID,
        label: "HDR Toning…",
        menu: &[],
        shortcut: None,
        params: r#"{"radius":1..500=30,"strength":0.1..4=0.5,"gamma":0.1..2=1,"exposure":-5..5=0,"detail":-100..300=30,"shadow":-100..100=0,"highlight":-100..100=0,"vibrance":-100..100=0,"saturation":-100..100=20,"curve":[[input0..1,output0..1]],"coalesce":string?} (native local adaptation; preserves layers and alpha)"#,
        enabled: crate::filters::has_filterable_layer,
        run,
        journal: true,
    }]
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn strict_native_parameters() {
        assert_eq!(params(&json!({})).unwrap(), HdrToning::default());
        for value in [
            json!(null),
            json!({"radius":0}),
            json!({"exposure":100}),
            json!({"curve":[[0,2]]}),
            json!({"curve":[[0,1,2]]}),
            json!({"unknown":1}),
            json!({"target":"bad"}),
            json!({"gamma":"1"}),
            json!({"coalesce":true}),
            json!({"coalesce":7}),
        ] {
            assert!(params(&value).is_err(), "{value}");
        }
    }
    #[test]
    fn native_hdr_drag_coalesces_execute_and_undo_at_every_depth() {
        for depth in [8, 16, 32] {
            let mut session = Session::new();
            session.execute("file.new", json!({"width":2,"height":2,"depth":depth})).unwrap();
            session.execute("layer.new.layer", json!({})).unwrap();
            let id = session.active().unwrap().active_layer.unwrap();
            session
                .edit("fixture", |doc, _| {
                    doc.layer_mut(id).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(0, 0, 2, 2), &[0.4, 0.2, 0.1, 0.75]);
                    Ok(())
                })
                .unwrap();
            let before = session.active().unwrap().doc.clone();
            let history = session.active().unwrap().history.past_len();
            session.execute(ID, json!({"radius":1,"exposure":0.25,"coalesce":"hdr-drag"})).unwrap();
            session.execute(ID, json!({"radius":1,"exposure":0.75,"coalesce":"hdr-drag"})).unwrap();
            assert_eq!(session.active().unwrap().history.past_len(), history + 1);
            let changed = session.active().unwrap().doc.clone();
            assert_ne!(*changed, *before);
            session.undo();
            assert_eq!(*session.active().unwrap().doc, *before);
            session.redo();
            assert_eq!(*session.active().unwrap().doc, *changed);
            session.execute(ID, json!({"radius":1,"exposure":-0.5,"coalesce":"another-drag"})).unwrap();
            assert_eq!(session.active().unwrap().history.past_len(), history + 2);
        }
    }
    #[test]
    fn native_smart_replay_is_editable_persistent_and_undoable() {
        use photocraft_doc::LayerContent;
        use photocraft_geom::Rect;
        let mut s = Session::new();
        s.execute("file.new", json!({"width":3,"height":3,"depth":32})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        let id = s.active().unwrap().active_layer.unwrap();
        s.edit("fixture", |doc, _| {
            doc.layer_mut(id).unwrap().surface_mut().unwrap().fill_rect(Rect::new(0, 0, 3, 3), &[2., 0.3, 0.1, 0.6]);
            Ok(())
        })
        .unwrap();
        s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        let before = s.active().unwrap().doc.clone();
        s.execute(ID, json!({"radius":3,"exposure":0.5})).unwrap();
        let changed = s.active().unwrap().doc.clone();
        let layer = changed.layers.iter().find(|l| matches!(l.content, LayerContent::Smart(_))).unwrap();
        let LayerContent::Smart(sm) = &layer.content else { panic!() };
        assert_eq!(sm.smart_filters.len(), 1);
        assert_eq!(sm.smart_filters[0].command, ID);
        let pixels = crate::smart_cmds::render(&changed, sm).unwrap().unwrap();
        assert!((pixels.pixel(1, 1)[3] - 0.6).abs() < 1e-5);
        assert!(pixels.pixel(1, 1)[0] <= 1.);
        s.undo();
        assert_eq!(*s.active().unwrap().doc, *before);
        s.redo();
        assert_eq!(*s.active().unwrap().doc, *changed);
        let bytes = photocraft_format::save_to_bytes(&changed, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
        let LayerContent::Smart(saved) = &reopened.layer(layer.id).unwrap().content else { panic!() };
        assert_eq!(saved.smart_filters, sm.smart_filters);
        assert_eq!(crate::smart_cmds::render(&reopened, saved).unwrap().unwrap(), pixels);
        let mut edited = reopened.clone();
        let LayerContent::Smart(edited_sm) = &mut edited.layer_mut(layer.id).unwrap().content else { panic!() };
        edited_sm.smart_filters[0].params = json!({"radius":3,"exposure":-2});
        edited_sm.cache = None;
        let snapshot = edited_sm.clone();
        assert_ne!(crate::smart_cmds::render(&edited, &snapshot).unwrap().unwrap().pixel(1, 1), pixels.pixel(1, 1));
    }
    #[test]
    fn native_raster_last_filter_locks_and_existing_hdr_flatten_are_preserved() {
        use photocraft_doc::LayerContent;
        let mut s = Session::new();
        s.execute("file.new", json!({"width":3,"height":3,"depth":32})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        let source_id = s.active().unwrap().active_layer.unwrap();
        s.edit("nonempty HDR fixture", |doc, _| {
            doc.layer_mut(source_id).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(0, 0, 3, 3), &[1.5, 0.4, 0.2, 0.75]);
            Ok(())
        })
        .unwrap();
        s.execute(ID, json!({})).unwrap();
        let n = s.active().unwrap().history.past_len();
        assert!(s.is_enabled("filter.lastFilter"));
        s.execute("filter.lastFilter", json!({})).unwrap();
        assert_eq!(s.active().unwrap().history.past_len(), n + 1);
        let id = s.active().unwrap().active_layer.unwrap();
        s.edit("lock", |doc, _| {
            doc.layer_mut(id).unwrap().locks.pixels = true;
            Ok(())
        })
        .unwrap();
        assert!(s.execute(ID, json!({})).is_err());
        s.undo();
        s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        s.execute("image.adjustments.hdrToning", json!({})).unwrap();
        let doc = &s.active().unwrap().doc;
        assert_eq!(doc.layers.len(), 1);
        assert!(matches!(doc.layers[0].content, LayerContent::Raster(_)));
    }
    #[test]
    fn replay_refuses_bad_controls_and_cancellation() {
        use photocraft_color::{ColorMode, PixelFormat, SampleType};
        use photocraft_geom::Rect;
        use photocraft_raster::{Interrupt, Surface};
        let bounds = Rect::new(-2, 3, 2, 6);
        let mut source = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::F32, true));
        source.fill_rect(bounds, &[2., 0.4, 0.1, 0.7]);
        let before = source.clone();
        assert!(crate::filters::apply_filter_to_surface(ID, &json!({"radius":-1}), &source, bounds, None, bounds).is_err());
        let cancelled = || true;
        assert!(matches!(
            crate::filters::apply_filter_to_surface_in(ID, &json!({}), &source, bounds, None, bounds, &Interrupt::cancel_only(&cancelled)),
            Err(EngineError::Cancelled)
        ));
        assert_eq!(source, before);
    }
}
