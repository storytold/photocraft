use crate::{EngineError, Session};
use serde_json::{Value, json};
use std::sync::Arc;

const EFFECTS: [&str; 10] =
    ["bevelEmboss", "stroke", "innerShadow", "innerGlow", "satin", "colorOverlay", "gradientOverlay", "patternOverlay", "outerGlow", "dropShadow"];

fn style_commands() -> impl Iterator<Item = (String, Value)> {
    std::iter::once(("layer.layerStyle.blendingOptions".into(), json!({"opacity": 50, "fillOpacity": 40, "blend": "Multiply"})))
        .chain(EFFECTS.map(|kind| (format!("layer.layerStyle.{kind}"), json!({}))))
}

fn white_document() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16, "background": "white"})).unwrap();
    s
}

#[test]
fn background_commands_do_not_change_the_document_or_history() {
    let mut s = white_document();
    let st = s.active().unwrap();
    let before = Arc::clone(&st.doc);
    let revision = st.revision;
    let history = st.history.entries();
    for (id, params) in style_commands() {
        let enabled = s.is_enabled(&id);
        let result = s.execute(&id, params);
        assert!(!enabled && matches!(&result, Err(EngineError::Disabled(_, why)) if why.contains("Background")), "{id}: enabled={enabled}, {result:?}");
        let st = s.active().unwrap();
        assert!(Arc::ptr_eq(&st.doc, &before), "{id} changed the document");
        assert_eq!(st.revision, revision, "{id} changed the revision");
        assert_eq!(st.history.entries(), history, "{id} changed history");
    }
    assert!(s.is_enabled("layer.layerStyle.globalLight"));
    assert!(s.is_enabled("layer.layerStyle.hideAllEffects"));
}

#[test]
fn explicit_targets_use_the_target_layer_instead_of_the_active_layer() {
    let mut s = white_document();
    let bg = s.active().unwrap().doc.background().unwrap().id.0;
    s.execute("layer.new.layer", json!({"name": "Paint"})).unwrap();
    let paint = s.active().unwrap().active_layer.unwrap().0;
    let before = Arc::clone(&s.active().unwrap().doc);
    for (id, mut params) in style_commands() {
        for target in [json!(bg), json!("Background"), json!({"name": "Background"}), json!({"background": true})] {
            params["layer"] = target;
            assert!(!s.is_enabled_with(&id, &params), "{id}: {params}");
            assert!(s.disabled_reason_with(&id, &params).is_some_and(|why| why.contains("Background")), "{id}: {params}");
            assert!(matches!(s.execute(&id, params.clone()), Err(EngineError::Disabled(_, why)) if why.contains("Background")), "{id}: {params}");
            let st = s.active().unwrap();
            assert_eq!(st.active_layer.unwrap().0, paint);
            assert!(Arc::ptr_eq(&st.doc, &before), "{id} changed the document");
        }
    }

    s.execute("layer.setProps", json!({"layer": paint, "name": "Background"})).unwrap();
    s.execute("layer.select", json!({"layer": bg})).unwrap();
    for (id, mut params) in style_commands() {
        params["layer"] = json!({"name": "Background"});
        assert!(s.is_enabled_with(&id, &params), "{id}: {params}");
        assert_eq!(s.disabled_reason_with(&id, &params), None, "{id}: {params}");
        s.execute(&id, params).unwrap_or_else(|err| panic!("{id}: {err}"));
        assert_eq!(s.active().unwrap().active_layer.unwrap().0, bg);
    }
    let st = s.active().unwrap();
    assert_eq!(st.doc.background().unwrap().id.0, bg);
    assert_eq!(st.doc.layer(photocraft_doc::LayerId(paint)).unwrap().effects.items.len(), EFFECTS.len());
}

#[test]
fn converting_the_background_enables_every_style_command() {
    let mut s = white_document();
    s.execute("layer.new.layerFromBackground", json!({})).unwrap();
    assert!(s.active().unwrap().doc.background().is_none());
    for (id, params) in style_commands() {
        assert!(s.is_enabled(&id), "{id} is disabled on Layer 0");
        assert!(s.is_enabled_with(&id, &params), "{id}: {params}");
        assert_eq!(s.disabled_reason_with(&id, &params), None, "{id}: {params}");
        s.execute(&id, params).unwrap_or_else(|err| panic!("{id}: {err}"));
    }
    let st = s.active().unwrap();
    let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
    assert_eq!(layer.opacity, 0.5);
    assert_eq!(layer.effects.items.len(), EFFECTS.len());
}

#[test]
fn style_queries_validate_explicit_targets_without_a_selection() {
    let mut s = white_document();
    s.execute("layer.new.layer", json!({"name": "Paint"})).unwrap();
    let target = s.active().unwrap().active_layer.unwrap().0;
    s.active_mut().unwrap().set_active_layer(None);
    let before = Arc::clone(&s.active().unwrap().doc);
    for (id, mut params) in style_commands() {
        assert!(!s.is_enabled(&id), "{id}");
        params["layer"] = json!(target);
        assert!(s.is_enabled_with(&id, &params), "{id}");
        assert_eq!(s.disabled_reason_with(&id, &params), None);
        assert_eq!(s.active().unwrap().active_layer, None);
        assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
        for layer in [json!(u64::MAX), json!("missing"), json!(-1), json!({"invalid": true})] {
            params["layer"] = layer;
            assert!(!s.is_enabled_with(&id, &params), "{id}: {params}");
            assert!(s.execute(&id, params.clone()).is_err(), "{id}: {params}");
            assert_eq!(s.active().unwrap().active_layer, None);
            assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
        }
    }
}

#[test]
fn background_refusal_preserves_a_pending_floating_selection() {
    for background in [false, true] {
        for (id, mut params) in style_commands() {
            let mut s = white_document();
            let bg = s.active().unwrap().doc.background().unwrap().id.0;
            s.execute("layer.new.layer", json!({"name": "Paint"})).unwrap();
            let paint = s.active().unwrap().active_layer.unwrap().0;
            s.execute("select.rect", json!({"x": 0, "y": 0, "width": 2, "height": 2})).unwrap();
            s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
            s.execute("select.float", json!({"dx": 3, "dy": 0})).unwrap();
            params["layer"] = json!(bg);
            let st = s.active().unwrap();
            let before = Arc::clone(&st.doc);
            let revision = st.revision;
            let history = st.history.entries();
            let parts = Arc::clone(&crate::float_cmds::floating(st).unwrap().parts);
            let result = if background { s.start(&id, params.clone()).map(|_| ()) } else { s.execute(&id, params.clone()).map(|_| ()) };
            assert!(matches!(result, Err(EngineError::Disabled(_, why)) if why.contains("Background")), "{id}");
            let st = s.active().unwrap();
            assert!(Arc::ptr_eq(&before, &st.doc), "{id}: background={background}");
            assert_eq!(st.revision, revision);
            assert_eq!(st.history.entries(), history);
            let floating = crate::float_cmds::floating(st).unwrap();
            assert_eq!(floating.offset, (3, 0));
            assert!(Arc::ptr_eq(&parts, &floating.parts));
            params["layer"] = json!(paint);
            s.execute(&id, params).unwrap();
            let st = s.active().unwrap();
            assert!(crate::float_cmds::floating(st).is_none(), "{id} on Paint still commits the floating piece");
            let pixels = st.doc.layer(photocraft_doc::LayerId(paint)).unwrap().surface().unwrap();
            assert_eq!(pixels.pixel(3, 0), vec![1.0, 0.0, 0.0, 1.0], "{id}: the piece reaches its destination");
            assert_eq!(pixels.pixel(0, 0), vec![0.0; 4], "{id}: the piece leaves its original position");
        }
    }
}

#[test]
fn missing_style_targets_report_available_layer_ids() {
    for background in [false, true] {
        let mut s = white_document();
        let bg = s.active().unwrap().doc.background().unwrap().id;
        for selected in [true, false] {
            s.active_mut().unwrap().set_active_layer(selected.then_some(bg));
            let before = Arc::clone(&s.active().unwrap().doc);
            for (id, mut params) in style_commands() {
                params["layer"] = json!(u64::MAX);
                assert!(!s.is_enabled_with(&id, &params));
                let result = if background { s.start(&id, params).map(|_| ()) } else { s.execute(&id, params).map(|_| ()) };
                assert!(
                    matches!(result, Err(EngineError::UnknownLayer { id: missing, known })
                    if missing.0 == u64::MAX && known.contains(&format!("{} \"Background\"", bg.0))),
                    "{id}"
                );
                assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
                assert_eq!(s.active().unwrap().active_layer, selected.then_some(bg));
            }
        }
    }
}

#[test]
fn background_refusal_defers_pending_history_budget_changes() {
    for background in [false, true] {
        let mut s = white_document();
        let bg = s.active().unwrap().doc.background().unwrap().id;
        // Model the pending scratch epoch without changing the process-wide scratch policy.
        // Direct preference storage leaves the old, unlimited History policy in place.
        s.prefs.edit(|p| p.performance.memory_usage_mb = 1);
        s.active_mut().unwrap().history.max_bytes = 0;
        for i in 0..8 {
            s.edit("Paint", |doc, _| {
                doc.layer_mut(bg).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::from_xywh(0, 0, 1, 1), &[i as f32 / 8.0, 0.0, 0.0, 1.0]);
                Ok(())
            })
            .unwrap();
        }
        s.scratch_epoch = photocraft_raster::spill::epoch().wrapping_sub(1);
        let pending_epoch = s.scratch_epoch;
        let st = s.active().unwrap();
        let history = st.history.entries();
        let before = Arc::clone(&st.doc);
        assert!(st.memory_bytes() > s.prefs().performance.history_budget_bytes());
        for (id, params) in style_commands() {
            let result = if background { s.start(&id, params).map(|_| ()) } else { s.execute(&id, params).map(|_| ()) };
            assert!(matches!(result, Err(EngineError::Disabled(_, why)) if why.contains("Background")), "{id}");
            assert_eq!(s.scratch_epoch, pending_epoch);
            let st = s.active().unwrap();
            assert_eq!(st.history.max_bytes, 0, "{id}: a refusal must not apply a pending budget");
            assert_eq!(st.history.entries(), history, "{id}: a refusal must not remove undo states");
            assert!(Arc::ptr_eq(&st.doc, &before));
        }
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        assert_ne!(s.scratch_epoch, pending_epoch);
        let cap = s.active().unwrap().history.max_bytes;
        assert!(cap == 0 || cap == s.prefs().performance.history_budget_bytes());
        if cap > 0 {
            assert!(s.active().unwrap().history.entries().len() < history.len());
        }
    }
}
