//! `layer.new.artboardFromGroup` names its history step "Artboard From Layer Group" (#14550); the
//! recorded action event keeps its own label.

use super::*;
use crate::actions_cmds::Action;
use photocraft_geom::Rect;

fn session_with_group() -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 100, "height": 80})).unwrap();
    s.edit("sq", |doc, active| {
        let fmt = doc.pixel_format();
        let mut l = Layer::raster("Square", fmt);
        l.surface_mut().unwrap().fill_rect(Rect::new(10, 20, 30, 40), &photocraft_raster::from_rgba(&fmt, [1.0, 0.0, 0.0, 1.0]));
        let id = doc.insert_above(None, l);
        *active = Some(id);
        Ok(id)
    })
    .unwrap();
    let g = s.execute("layer.groupLayers", json!({})).unwrap();
    let gid = LayerId(g["layer"].as_u64().unwrap());
    (s, gid)
}

fn doc(s: &Session) -> &Document {
    &s.active().unwrap().doc
}

fn steps(s: &Session) -> Vec<String> {
    s.active().unwrap().history.entries()
}

#[test]
fn the_history_step_is_named_artboard_from_layer_group() {
    let (mut s, gid) = session_with_group();
    let before = steps(&s);
    s.execute("layer.new.artboardFromGroup", json!({"layer": gid.0})).unwrap();
    let after = steps(&s);
    assert_eq!(after.len(), before.len() + 1, "exactly one history step: {after:?}");
    assert_eq!(after.last().map(String::as_str), Some("Artboard From Layer Group"));
}

#[test]
fn one_undo_restores_the_group() {
    let (mut s, gid) = session_with_group();
    let layers_before = doc(&s).layers.clone();
    let bounds_before = doc(&s).bounds();
    s.execute("layer.new.artboardFromGroup", json!({"layer": gid.0})).unwrap();
    assert!(doc(&s).layer(gid).unwrap().artboard().is_some());
    s.execute("history.undo", json!({})).unwrap();
    let d = doc(&s);
    assert!(d.layer(gid).unwrap().is_group());
    assert!(d.layer(gid).unwrap().artboard().is_none());
    assert_eq!(d.bounds(), bounds_before);
    assert_eq!(d.layers.len(), layers_before.len());
    let kids = |layers: &[Layer]| layers.iter().find(|l| l.id == gid).and_then(|l| l.children()).map(|c| c.iter().map(|k| k.name.clone()).collect::<Vec<_>>());
    assert_eq!(kids(&d.layers), kids(&layers_before));
}

#[test]
fn a_recorded_step_keeps_the_event_label() {
    let action = Action::new("Board", vec![("layer.new.artboardFromGroup".into(), json!({}))]);
    let (bytes, _) = crate::atn_cmds::export("Set", &[&action], &[], false).unwrap();
    let has = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
    assert!(has(b"Artboard from Group"), "the action event label is unchanged");
    assert!(!has(b"Artboard From Layer Group"));
}
