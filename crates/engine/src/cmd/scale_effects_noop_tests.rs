//! Scale Effects at 100 %, measured against the reference application (#14991).
//!
//! OK with Scale 100 leaves the stored effect values unchanged and adds no history state; any
//! other scale adds one "Scale Effects" state.

use std::sync::Arc;

use photocraft_doc::{Effect, LayerId};

use super::*;

const CMD: &str = "layer.layerStyle.scaleEffects";

/// A document whose active layer has a Drop Shadow (distance 10, size 4) and a preserved PSD
/// effects block, saved (clean) as a file just opened would be.
fn styled() -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Shape"})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.layerStyle.dropShadow", json!({"distance": 10, "size": 4})).unwrap();
    s.edit("psd block", |doc, _| {
        doc.layer_mut(id).unwrap().effects.psd_raw = Some(Arc::new(vec![1, 2, 3]));
        Ok(())
    })
    .unwrap();
    let d = s.active_mut().unwrap();
    d.saved_revision = d.revision;
    (s, id)
}

fn shadow(s: &Session, id: LayerId) -> (f32, f32) {
    match s.active().unwrap().doc.layer(id).unwrap().effects.items.first() {
        Some(Effect::DropShadow(sh)) => (sh.distance, sh.size),
        e => panic!("no drop shadow: {e:?}"),
    }
}

fn psd_raw(s: &Session, id: LayerId) -> Option<Vec<u8>> {
    s.active().unwrap().doc.layer(id).unwrap().effects.psd_raw.as_deref().cloned()
}

#[test]
fn scale_100_changes_nothing_and_adds_no_history_state() {
    for params in [json!({"scale": 100}), json!({})] {
        let (mut s, id) = styled();
        let history = s.active().unwrap().history.entries();
        s.execute(CMD, params.clone()).unwrap();
        let d = s.active().unwrap();
        assert_eq!(d.history.entries(), history, "{params}: no history state");
        assert!(!d.is_dirty(), "{params}: the document stays unmodified");
        assert_eq!(psd_raw(&s, id), Some(vec![1, 2, 3]), "{params}: the preserved effects block is kept");
        assert_eq!(shadow(&s, id), (10.0, 4.0));
    }
}

#[test]
fn other_scales_add_one_scale_effects_state() {
    for (scale, want) in [(50, (5.0, 2.0)), (200, (20.0, 8.0))] {
        let (mut s, id) = styled();
        let history = s.active().unwrap().history.entries();
        s.execute(CMD, json!({"scale": scale})).unwrap();
        let d = s.active().unwrap();
        assert_eq!(d.history.entries().len(), history.len() + 1, "scale {scale}: one history state");
        assert_eq!(d.history.entries().last().map(String::as_str), Some("Scale Effects"));
        assert!(d.is_dirty());
        assert_eq!(shadow(&s, id), want);
    }
}

#[test]
fn scale_100_still_errors_without_a_document_or_layer() {
    let mut s = Session::new();
    assert!(s.execute(CMD, json!({"scale": 100})).is_err(), "no document");
    let (mut s, _) = styled();
    assert!(s.execute(CMD, json!({"scale": 100, "layer": 9999})).is_err(), "missing layer");
}
