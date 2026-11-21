//! The commands an Alt-click on the line between two layers runs (#967). Where the line is and the
//! click that reports it are craft-layers' (`LayersEvent::ClipLine`).

use photocraft_doc::LayerId;
use photocraft_engine::Session;
use serde_json::json;

use super::ClipLine;

/// "Layer 1", a Color Balance adjustment above it and "Top" above that.
fn chain() -> (Session, [LayerId; 3]) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    let l1 = s.execute("layer.new.layer", json!({"name": "Layer 1"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.newAdjustmentLayer.colorBalance", json!({})).unwrap();
    let cb = s.active().unwrap().active_layer.unwrap().0;
    let top = s.execute("layer.new.layer", json!({"name": "Top"})).unwrap()["layer"].as_u64().unwrap();
    (s, [LayerId(l1), LayerId(cb), LayerId(top)])
}

fn clipped(s: &Session, id: LayerId) -> bool {
    s.active().unwrap().doc.layer(id).unwrap().clipped
}

/// An Alt-click on the line below `upper`: it releases a clipped layer and clips any other.
fn click(s: &mut Session, upper: LayerId, key: &str) {
    let line = ClipLine { upper, release: clipped(s, upper) };
    for (command, params) in line.commands(&s.active().unwrap().doc.clone(), key) {
        s.execute(&command, params).unwrap();
    }
}

#[test]
fn releasing_frees_the_clipped_layers_above_too_and_selects_the_layer() {
    // Photoshop 25.4: a release in the middle of a clipping chain frees the layer above the line
    // and every clipped layer stacked above it, in one history step, and makes it active.
    let (mut s, [_, cb, top]) = chain();
    click(&mut s, cb, "clip-line:1");
    click(&mut s, top, "clip-line:2");
    assert!(clipped(&s, cb) && clipped(&s, top));
    assert_eq!(s.active().unwrap().active_layer, Some(top), "clipping keeps the active layer");
    let before = s.active().unwrap().history.entries().len();
    click(&mut s, cb, "clip-line:3");
    assert!(!clipped(&s, cb) && !clipped(&s, top), "the clipped layer above is released too");
    assert_eq!(s.active().unwrap().active_layer, Some(cb), "the released layer becomes active");
    assert_eq!(s.active().unwrap().history.entries().len(), before + 1, "one history step");
    assert!(s.undo());
    assert!(clipped(&s, cb) && clipped(&s, top), "one Undo brings the chain back");
    // Releasing the top of a chain leaves the layers below it clipped.
    click(&mut s, top, "clip-line:4");
    assert!(!clipped(&s, top) && clipped(&s, cb));
}
