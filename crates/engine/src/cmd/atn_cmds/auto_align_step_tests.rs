//! The Auto-Align Layers step of `.atn` actions (#14201): the `Algn` event with `using` =
//! `ADSContent` plays as `edit.autoAlignLayers`, and an Auto-Align run records back to it.

use photocraft_doc::LayerId;
use photocraft_geom::Rect;

use super::super::*;
use crate::actions_cmds::ActionStep;

/// A recorded Auto-Align step with these keys after `null` and `Usng`.
fn auto_align(keys: Vec<(&str, D)>) -> Descriptor {
    let mut d = desc("null").with("null", target_ref("Lyr ")).with("Usng", enm("ADSt", "ADSContent"));
    for (key, value) in keys {
        d = d.with(key, value);
    }
    d
}

/// One action with this single recorded `Algn` step.
fn align_file(descriptor: Descriptor) -> Vec<u8> {
    atn::write(&AtnFile {
        name: UnicodeString::new_nul("Align"),
        actions: vec![AtnAction {
            name: UnicodeString::new_nul("Step"),
            items: vec![AtnItem { event: AtnEvent::Code(*b"Algn"), dictionary: b"Align".to_vec(), descriptor: Some(descriptor), ..Default::default() }],
            ..Default::default()
        }],
        ..Default::default()
    })
    .unwrap()
}

/// The command and params the step imports as, or `None` when it is reported unsupported.
fn imported(descriptor: Descriptor) -> Option<(String, Value)> {
    let loaded = import(&align_file(descriptor)).unwrap();
    if !loaded.unsupported.is_empty() {
        return None;
    }
    let step = &loaded.actions[0].steps[0];
    Some((step.command.clone(), step.params.clone()))
}

fn apply(value: &str) -> (&'static str, D) {
    ("apply", enm("projection", value))
}

#[test]
fn every_recorded_projection_plays_as_auto_align() {
    for (recorded, projection) in [
        ("auto", "auto"),
        ("perspective", "perspective"),
        ("cylindrical", "cylindrical"),
        ("spherical", "spherical"),
        ("sceneCollage", "collage"),
        ("translation", "reposition"),
    ] {
        for distort in [false, true] {
            let step = auto_align(vec![apply(recorded), ("vignette", D::Boolean(false)), ("radialDistort", D::Boolean(distort))]);
            assert_eq!(
                imported(step),
                Some(("edit.autoAlignLayers".into(), json!({"projection": projection, "geometricCorrection": distort}))),
                "{recorded} / radialDistort {distort}"
            );
        }
    }
}

/// Missing keys take the dialog's defaults: Auto, both Lens Correction boxes off.
#[test]
fn a_step_without_its_options_plays_with_the_defaults() {
    assert_eq!(imported(auto_align(vec![])), Some(("edit.autoAlignLayers".into(), json!({"projection": "auto", "geometricCorrection": false}))));
}

/// PhotoCraft has no Vignette Removal yet (#14197): the step is unsupported, not played without it.
#[test]
fn a_step_with_vignette_removal_is_unsupported() {
    assert_eq!(imported(auto_align(vec![apply("auto"), ("vignette", D::Boolean(true)), ("radialDistort", D::Boolean(false))])), None);
}

#[test]
fn an_unknown_projection_or_a_wrong_typed_key_is_unsupported() {
    let cases = [
        vec![apply("fisheye")],
        vec![("apply", D::Boolean(true))],
        vec![("apply", text("auto"))],
        vec![apply("auto"), ("radialDistort", D::Integer(1))],
        vec![apply("auto"), ("vignette", enm("Ordn", "Trgt"))],
    ];
    for keys in cases {
        let step = auto_align(keys);
        assert_eq!(imported(step.clone()), None, "{step:?}");
    }
}

/// Every projection and Geometric Distortion setting records as the `Algn` / `ADSContent` step
/// and loads back to the same command and params.
#[test]
fn an_auto_align_run_records_and_reads_back() {
    for projection in ["auto", "perspective", "collage", "cylindrical", "spherical", "reposition"] {
        for geometric in [false, true] {
            let params = json!({"projection": projection, "geometricCorrection": geometric});
            let action = Action::new("Align", vec![ActionStep::new("edit.autoAlignLayers", params.clone())]);
            let (bytes, skipped) = export("S", &[&action], &[], false).unwrap();
            assert!(skipped.is_empty(), "{params}");
            let file = atn::parse(&bytes).unwrap();
            let item = &file.actions[0].items[0];
            assert_eq!(item.event, AtnEvent::Code(*b"Algn"));
            let d = item.descriptor.as_ref().unwrap();
            assert_eq!(get_enum(d, "Usng").as_deref(), Some("ADSContent"));
            assert!(get_enum(d, "apply").is_some(), "{params}: no apply");
            assert_eq!(get_bool(d, "vignette"), Some(false));
            assert_eq!(get_bool(d, "radialDistort"), Some(geometric));
            let loaded = import(&bytes).unwrap();
            assert_eq!((loaded.actions[0].steps[0].command.as_str(), &loaded.actions[0].steps[0].params), ("edit.autoAlignLayers", &params));
        }
    }
}

/// The dialog's run with its default interpolation and registration records; a chosen reference
/// layer or another interpolation or registration has no place in the step, so it is not exported.
#[test]
fn a_run_the_step_cannot_hold_is_not_exported() {
    let recorded = json!({"projection": "auto", "geometricCorrection": false, "interpolation": "bicubic", "registration": "auto"});
    assert!(export_item("edit.autoAlignLayers", &recorded, "S").is_some());
    for extra in [json!({"reference": 3}), json!({"interpolation": "nearest"}), json!({"registration": "intensity"}), json!({"projection": "fisheye"})] {
        let mut params = recorded.clone();
        params.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        assert!(export_item("edit.autoAlignLayers", &params, "S").is_none(), "{params}");
    }
}

/// A deterministic texture in a new layer at (dx, dy), as the Auto-Align command tests use.
fn texture_layer(s: &mut Session, name: &str, dx: i32, dy: i32) -> LayerId {
    s.execute("layer.new.layer", json!({"name": name})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    s.edit("tex", |doc, _| {
        let surf = doc.layer_mut(id).unwrap().surface_mut().unwrap();
        let mut seed = 99u64;
        let mut rnd = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) as f64 / (1u64 << 31) as f64
        };
        surf.fill_rect(Rect::new(dx, dy, dx + 240, dy + 180), &[0.4, 0.4, 0.4, 1.0]);
        for _ in 0..120 {
            let (x, y) = ((rnd() * 220.0) as i32, (rnd() * 160.0) as i32);
            let (sw, sh) = (4 + (rnd() * 14.0) as i32, 4 + (rnd() * 10.0) as i32);
            let v = rnd() as f32;
            surf.fill_rect(Rect::new(dx + x, dy + y, dx + x + sw, dy + y + sh), &[v, 1.0 - v, v * 0.5, 1.0]);
        }
        Ok(())
    })
    .unwrap();
    id
}

/// Playing a recorded Reposition step aligns the selected layers in one undoable step.
#[test]
fn a_played_step_aligns_the_layers_in_one_history_state() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 300, "height": 240, "background": "transparent"})).unwrap();
    let a = texture_layer(&mut s, "A", 20, 20);
    let b = texture_layer(&mut s, "B", 31, 13);
    s.execute("layer.select", json!({"layer": a.0})).unwrap();
    s.execute("layer.select", json!({"layer": b.0, "mode": "add"})).unwrap();
    let bounds = |s: &Session| s.active().unwrap().doc.layer(b).unwrap().surface().unwrap().content_bounds();
    let before = bounds(&s);

    let step = auto_align(vec![apply("translation"), ("vignette", D::Boolean(false)), ("radialDistort", D::Boolean(false))]);
    let loaded = s.execute("actions.load", json!({"data": photocraft_paint::tile::b64_encode(&align_file(step))})).unwrap();
    assert_eq!(loaded["unsupported"], json!([]));
    let result = s.execute("actions.play", json!({"action": loaded["actions"][0]})).unwrap();
    assert!(result.get("failed").is_none(), "{result}");
    let after = bounds(&s);
    assert!((after.x0 - 20).abs() <= 1 && (after.y0 - 20).abs() <= 1, "B moved onto A: {after:?}");

    s.undo();
    assert_eq!(bounds(&s), before, "one undo restores the layers");
}
