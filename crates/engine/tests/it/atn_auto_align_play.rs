//! Playing a recorded Auto-Align Layers step (#14201) in the full app's session, where
//! `edit.autoAlignLayers` is registered; the step's import and export tests sit beside the
//! importer in engine-core.

use craft_psd::atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem};
use craft_psd::descriptor::{Class, Descriptor, Id, ReferenceItem, UnicodeString, Value as D};
use photocraft_doc::LayerId;
use photocraft_engine::Session;
use photocraft_geom::Rect;
use serde_json::json;

fn enm(type_id: &str, value: &str) -> D {
    D::Enumerated { type_id: Id::new(type_id), value: Id::new(value) }
}

/// One action whose single step is a recorded Auto-Align with Reposition, aimed at the selected
/// layers.
fn reposition_action() -> Vec<u8> {
    let target = ReferenceItem::Enumerated {
        class: Class { name: UnicodeString::new_nul(""), class_id: Id::new("Lyr ") },
        type_id: Id::new("Ordn"),
        value: Id::new("Trgt"),
    };
    let step = Descriptor::new("null")
        .with("null", D::Reference(vec![target]))
        .with("Usng", enm("ADSt", "ADSContent"))
        .with("apply", enm("projection", "translation"))
        .with("vignette", D::Boolean(false))
        .with("radialDistort", D::Boolean(false));
    atn::write(&AtnFile {
        name: UnicodeString::new_nul("Align"),
        actions: vec![AtnAction {
            name: UnicodeString::new_nul("Step"),
            items: vec![AtnItem { event: AtnEvent::Code(*b"Algn"), dictionary: b"Align".to_vec(), descriptor: Some(step), ..Default::default() }],
            ..Default::default()
        }],
        ..Default::default()
    })
    .unwrap()
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
fn a_played_auto_align_step_aligns_the_layers_in_one_history_state() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 300, "height": 240, "background": "transparent"})).unwrap();
    let a = texture_layer(&mut s, "A", 20, 20);
    let b = texture_layer(&mut s, "B", 31, 13);
    s.execute("layer.select", json!({"layer": a.0})).unwrap();
    s.execute("layer.select", json!({"layer": b.0, "mode": "add"})).unwrap();
    let bounds = |s: &Session| s.active().unwrap().doc.layer(b).unwrap().surface().unwrap().content_bounds();
    let before = bounds(&s);
    let steps = s.active().unwrap().history.past_len();

    let loaded = s.execute("actions.load", json!({"data": photocraft_paint::tile::b64_encode(&reposition_action())})).unwrap();
    assert_eq!(loaded["unsupported"], json!([]));
    let result = s.execute("actions.play", json!({"action": loaded["actions"][0]})).unwrap();
    assert!(result.get("failed").is_none(), "{result}");
    let after = bounds(&s);
    assert!((after.x0 - 20).abs() <= 1 && (after.y0 - 20).abs() <= 1, "B moved onto A: {after:?}");
    assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "one history state");

    s.undo();
    assert_eq!(bounds(&s), before, "one undo restores the layers");
}
