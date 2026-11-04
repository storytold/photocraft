//! #7670: integer ATN Ripple amounts must never silently replace native fractions.
use photocraft_engine::actions_cmds::Action;
use photocraft_engine::{Session, atn_cmds};
use photocraft_psd::{atn, descriptor::Value as D};
use serde_json::{Value, json};

const RIPPLE: &str = "filter.distort.ripple";

/// Build a deterministic RGB image at the requested depth that exposes Ripple rounding.
fn fixture(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 24, "height": 18, "depth": depth})).unwrap();
    s.edit("procedural fixture", |doc, active| {
        let surface = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let format = surface.format();
        let bytes: Vec<_> =
            (0..432).flat_map(|i| photocraft_raster::from_rgba(&format, [(i % 11) as f32 / 10.0, (i % 7) as f32 / 6.0, (i % 5) as f32 / 4.0, 1.0])).collect();
        surface.write_region(photocraft_doc::Rect::from_xywh(0, 0, 24, 18), &bytes);
        Ok(())
    })
    .unwrap();
    s
}

/// Flatten the active document to compare direct, native and ATN playback pixels.
fn pixels(s: &Session) -> Vec<[f32; 4]> {
    photocraft_compose::flatten(&s.active().unwrap().doc).into_px()
}

/// Fractions retain native settings and pixels without approximating the integer Photoshop event.
#[test]
fn fractional_amounts_use_native_events_without_pixel_loss() {
    for depth in [8, 16, 32] {
        for size in ["small", "medium", "large"] {
            for amount in [81.4, -81.4, 0.5, -0.5, 998.9, -998.9] {
                let params = json!({"amount": amount, "size": size});
                let action = Action::new("Fractional Ripple", vec![(RIPPLE.into(), params.clone())]);
                let mut direct = fixture(depth);
                direct.execute(RIPPLE, params).unwrap();
                let mut native = fixture(depth);
                native.actions.list.push(action.clone());
                native.execute("actions.play", json!({"action": "Fractional Ripple"})).unwrap();
                assert_eq!(pixels(&direct), pixels(&native));

                if amount == 81.4 {
                    let mut rounded = fixture(depth);
                    rounded.execute(RIPPLE, json!({"amount": 81, "size": size})).unwrap();
                    assert_ne!(pixels(&direct), pixels(&rounded), "fixture must expose rounding: {depth}, {size}");
                }
                let (bytes, skipped) = atn_cmds::export("Ripple", &[&action], &[], false).unwrap();
                assert!(skipped.is_empty());
                let item = &atn::parse(&bytes).unwrap().actions[0].items[0];
                assert_eq!(item.event, atn::AtnEvent::name("photocraftCommand"));
                let imported = atn_cmds::import(&bytes).unwrap();
                assert!(imported.unsupported.is_empty() && imported.warnings.is_empty());
                assert_eq!(imported.actions[0].steps, action.steps, "native fallback retains exact fractions");
                let mut replay = fixture(depth);
                replay.actions.list = imported.actions;
                replay.execute("actions.play", json!({"action":action.name})).unwrap();
                assert_eq!(pixels(&replay), pixels(&direct), "native fallback must not round pixels");
                crate::atn_export::fallback("Ripple", &action);
            }
        }
    }
}

/// Whole amounts and omitted defaults still round-trip through binary ATN and actual pixels.
#[test]
fn integer_amounts_sizes_and_defaults_preserve_playback() {
    for depth in [8, 16, 32] {
        let mut cases = vec![Value::Null, json!({}), json!({"target": "pixels"})];
        for size in ["small", "medium", "large"] {
            for amount in [-999, -81, 0, 81, 999] {
                cases.push(json!({"amount": amount, "size": size}));
            }
        }
        for params in cases {
            let action = Action::new("Integer Ripple", vec![(RIPPLE.into(), params.clone())]);
            let mut direct = fixture(depth);
            direct.execute(RIPPLE, params.clone()).unwrap();
            let mut native = fixture(depth);
            native.actions.list.push(action.clone());
            native.execute("actions.play", json!({"action": "Integer Ripple"})).unwrap();
            assert_eq!(pixels(&native), pixels(&direct));

            let (bytes, skipped) = atn_cmds::export("Ripple", &[&action], &[], false).unwrap();
            assert!(skipped.is_empty());
            let file = atn::parse(&bytes).unwrap();
            let descriptor = file.actions[0].items[0].descriptor.as_ref().unwrap();
            if let Some(amount) = params.get("amount").and_then(Value::as_i64) {
                assert!(matches!(descriptor.get("Amnt"), Some(D::Integer(n)) if i64::from(*n) == amount));
            }
            let imported = atn_cmds::import(&bytes).unwrap();
            assert!(imported.unsupported.is_empty() && imported.warnings.is_empty());
            assert_eq!(imported.actions.len(), 1);
            assert_eq!(imported.actions[0].steps.len(), 1);
            assert_eq!(imported.actions[0].steps[0].0, RIPPLE);
            let mut replay = fixture(depth);
            replay.actions.list = imported.actions;
            replay.execute("actions.play", json!({"action": "Integer Ripple"})).unwrap();
            assert_eq!(pixels(&replay), pixels(&direct), "depth {depth}: {params}");
        }
    }
}

/// Opt-in skipping reports precisely the lost edits, keeping the supported steps in order.
#[test]
fn skipping_fractional_amounts_reports_action_and_step() {
    let action = Action::new(
        "Mixed Ripple",
        [json!(10), json!(81.4), json!(-20), json!(-0.5)].into_iter().map(|amount| (RIPPLE.into(), json!({"amount": amount, "size": "small"}))).collect(),
    );
    let items = crate::atn_export::preserved("Ripple", &action);
    assert_eq!(items.len(), 4);
    for index in [1, 3] {
        assert_eq!(items[index].event, atn::AtnEvent::name("photocraftCommand"));
    }
    let (bytes, skipped) = atn_cmds::export("Ripple", &[&action], &[], true).unwrap();
    assert_eq!(
        skipped,
        vec![
            atn_cmds::Skipped { action: "Mixed Ripple".into(), step: 1, id: RIPPLE.into() },
            atn_cmds::Skipped { action: "Mixed Ripple".into(), step: 3, id: RIPPLE.into() },
        ]
    );
    let imported = atn_cmds::import(&bytes).unwrap();
    assert!(imported.unsupported.is_empty() && imported.warnings.is_empty());
    assert_eq!(imported.actions.len(), 1);
    let steps = &imported.actions[0].steps;
    assert_eq!(steps.len(), 2);
    for (step, amount) in steps.iter().zip([10.0, -20.0]) {
        assert_eq!(step.0, RIPPLE);
        assert_eq!(step.1["amount"].as_f64(), Some(amount));
        assert_eq!(step.1["size"], "small");
    }
}

/// A dedicated Ripple mapping must retain the plain-event target restriction.
#[test]
fn non_pixel_targets_and_malformed_fields_use_native_events_or_are_skipped() {
    for params in [
        json!({"amount": 81, "target": "mask"}),
        json!({"amount": 81, "target": "quickMask"}),
        json!({"amount": 81, "target": "channel"}),
        json!({"amount": 81, "target": null}),
        json!({"amount": null}),
        json!({"amount": "81.4"}),
        json!({"amount": 2147483648_i64}),
        json!({"amount": -2147483649_i64}),
        json!({"amount": 1e100}),
        json!({"amount": 81, "size": "unknown"}),
    ] {
        let action = Action::new("Unsupported Ripple", vec![(RIPPLE.into(), params.clone())]);
        crate::atn_export::fallback("Ripple", &action);
        let (bytes, skipped) = atn_cmds::export("Ripple", &[&action], &[], true).unwrap();
        assert_eq!(skipped, vec![atn_cmds::Skipped { action: "Unsupported Ripple".into(), step: 0, id: RIPPLE.into() }]);
        assert!(atn_cmds::import(&bytes).unwrap().actions[0].steps.is_empty());
    }
}
