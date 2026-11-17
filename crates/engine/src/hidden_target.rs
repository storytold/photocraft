//! Painting and interactive Move refuse hidden targets (#571), including layers inside hidden
//! groups. Recorded Move steps can move them without changing visibility. Alpha channel and
//! Quick Mask targets have no layer, so the hidden-layer guard does not apply to them.

use photocraft_doc::{Document, LayerId};
use serde_json::Value;

use crate::Session;

/// Photoshop's alert text.
pub const HIDDEN: &str = "Could not complete your request because the target layer is hidden.";

/// Commands that paint on, or move, the target layer.
const GATED: &[&str] = &[
    "paint.stroke",
    "paint.pencil",
    "paint.mixerBrush",
    "paint.colorReplacement",
    "paint.bucket",
    "paint.gradient",
    "paint.magicEraser",
    "paint.backgroundEraser",
    "paint.cloneStamp",
    "paint.healingBrush",
    "paint.spotHealing",
    "paint.patch",
    "paint.dodge",
    "paint.burn",
    "paint.sponge",
    "paint.blur",
    "paint.sharpen",
    "paint.smudge",
    "paint.historyBrush",
    "paint.artHistoryBrush",
    "layer.translate",
];

/// Is the layer shown: it and every group around it visible.
fn shown(doc: &Document, id: LayerId) -> bool {
    doc.path_of(id).is_some_and(|path| (1..=path.len()).all(|n| path.get(..n).and_then(|p| doc.layer_at(p)).is_some_and(|l| l.visible)))
}

/// Why command `id` with `params` must not run: its target layer is hidden. `None` for other
/// commands, channel targets, and visible layers.
pub fn refusal(s: &Session, id: &str, params: &Value) -> Option<&'static str> {
    if !GATED.contains(&id) {
        return None;
    }
    // Photoshop's recorded Move runs on hidden layers too. Keep the interactive tool and
    // painting guards; playback must neither reveal its target nor skip the recorded move.
    if id == "layer.translate" && s.actions.playing > 0 {
        return None;
    }
    let params = crate::channel_cmds::inject_target(s, id, params.clone());
    if crate::channel_cmds::is_channel_target(&params) {
        return None;
    }
    let st = s.active()?;
    let layer = crate::commands::layer_param(s, &params).ok()?;
    (st.doc.layer(layer).is_some() && !shown(&st.doc, layer)).then_some(HIDDEN)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::HIDDEN;
    use crate::Session;

    fn session() -> (Session, u64) {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 64, "height": 64, "background": "white"})).unwrap();
        let id = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
        (s, id)
    }

    fn pixel(s: &mut Session) -> serde_json::Value {
        s.execute("document.pixel", json!({"x": 10, "y": 10})).unwrap()
    }

    #[test]
    fn painting_or_moving_a_hidden_layer_is_refused() {
        let (mut s, id) = session();
        s.execute("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
        let steps = s.active().unwrap().history.entries().len();
        for (cmd, p) in [
            ("paint.stroke", json!({"points": [[10, 10], [20, 20]], "color": "#ff0000"})),
            ("paint.bucket", json!({"x": 10, "y": 10})),
            ("paint.gradient", json!({"from": [0, 0], "to": [60, 60]})),
            ("layer.translate", json!({"dx": 5, "dy": 0})),
        ] {
            let e = s.execute(cmd, p).unwrap_err().to_string();
            assert!(e.contains(HIDDEN), "{cmd}: {e}");
        }
        assert_eq!(s.active().unwrap().history.entries().len(), steps, "nothing changed");
        // Shown again, it paints.
        s.execute("layer.setProps", json!({"layer": id, "visible": true})).unwrap();
        s.execute("paint.stroke", json!({"points": [[10, 10], [12, 12]], "color": "#ff0000", "size": 9})).unwrap();
        assert_ne!(pixel(&mut s), json!([1.0, 1.0, 1.0, 1.0]));
    }

    #[test]
    fn a_layer_in_a_hidden_group_is_hidden() {
        let (mut s, id) = session();
        s.execute("layer.groupLayers", json!({})).unwrap();
        let g = s.active().unwrap().active_layer.unwrap().0;
        assert_ne!(g, id, "the new group is active");
        s.execute("layer.setProps", json!({"layer": g, "visible": false})).unwrap();
        s.execute("layer.select", json!({"layer": id})).unwrap();
        assert!(s.execute("paint.stroke", json!({"points": [[10, 10]]})).unwrap_err().to_string().contains(HIDDEN));
    }

    #[test]
    fn alpha_channels_of_a_hidden_layer_still_paint() {
        let (mut s, id) = session();
        s.execute("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
        s.execute("channel.new", json!({})).unwrap();
        s.execute("paint.stroke", json!({"points": [[10, 10], [20, 20]], "target": {"channel": 0}})).unwrap();
    }

    #[test]
    fn recorded_move_keeps_hidden_layers_and_groups_hidden() {
        use crate::actions_cmds::Action;
        use photocraft_doc::LayerId;
        use photocraft_geom::Rect;

        let action = Action::new("Move", vec![("layer.translate".into(), json!({"dx":-19,"dy":0})), ("layer.setProps".into(), json!({"name":"After"}))]);
        let (bytes, skipped) = crate::atn_cmds::export("Moves", &[&action], &[], false).unwrap();
        assert!(skipped.is_empty());
        for shape in [false, true] {
            for (visible, parent_visible) in [(true, true), (true, false), (false, true), (false, false)] {
                let (mut s, raster) = session();
                let id = if shape {
                    LayerId(s.execute("shape.create", json!({"kind":"rect","rect":[4,4,8,8],"fill":"#ff0000"})).unwrap()["layer"].as_u64().unwrap())
                } else {
                    s.edit("Pixels", |doc, _| {
                        doc.layer_mut(LayerId(raster)).unwrap().surface_mut().unwrap().fill_rect(Rect::new(4, 4, 12, 12), &[1.0, 0.0, 0.0, 1.0]);
                        Ok(())
                    })
                    .unwrap();
                    LayerId(raster)
                };
                s.execute("layer.groupLayers", json!({})).unwrap();
                let group = s.active().unwrap().active_layer.unwrap();
                s.execute("layer.setProps", json!({"layer":group.0,"visible":parent_visible})).unwrap();
                s.execute("layer.setProps", json!({"layer":id.0,"visible":visible})).unwrap();
                s.execute("layer.select", json!({"layer":id.0})).unwrap();
                let before = s.active().unwrap().doc.clone();
                let imported = crate::atn_cmds::import(&bytes).unwrap();
                assert!(imported.unsupported.is_empty());
                s.actions.list = imported.actions;
                let played = s.execute("actions.play", json!({"action":"Move"})).unwrap();
                assert!(played.get("failed").is_none(), "{played}");
                let doc = &s.active().unwrap().doc;
                let layer = doc.layer(id).unwrap();
                if let photocraft_doc::LayerContent::Shape(shape) = &layer.content {
                    let photocraft_doc::LiveShape::Rect { rect, .. } = shape.live.as_ref().unwrap() else { panic!("expected a live rectangle") };
                    assert_eq!(*rect, [-15.0, 4.0, 8.0, 8.0]);
                } else {
                    assert_eq!(crate::layer_multi_cmds::layer_bounds(layer), Some(Rect::new(-15, 4, -7, 12)), "visible={visible}, parent={parent_visible}");
                }
                assert_eq!(layer.name, "After", "the action continues after Move");
                assert_eq!(layer.visible, visible);
                assert_eq!(doc.layer(group).unwrap().visible, parent_visible);
                assert_eq!(s.actions.playing, 0);
                if !visible || !parent_visible {
                    assert!(s.execute("layer.translate", json!({"dx":1})).unwrap_err().to_string().contains(HIDDEN), "interactive Move still refuses");
                }
                assert!(s.undo());
                assert_eq!(s.active().unwrap().doc.layers, before.layers, "Undo restores the move and the following step together");
            }
        }
    }

    #[test]
    fn recorded_move_does_not_bypass_painting_or_position_locks() {
        use crate::actions_cmds::Action;

        let (mut s, id) = session();
        s.execute("layer.setProps", json!({"layer":id,"visible":false})).unwrap();
        s.actions.list.push(Action::new("Paint", vec![("paint.bucket".into(), json!({"x":10,"y":10}))]));
        let painted = s.execute("actions.play", json!({"action":"Paint"})).unwrap();
        assert!(painted.get("failed").is_some() && painted.to_string().contains(HIDDEN));
        s.execute("layer.lockLayers", json!({"position":true})).unwrap();
        let before = s.active().unwrap().doc.clone();
        s.actions.list.push(Action::new("Locked move", vec![("layer.translate".into(), json!({"dx":-19,"dy":0}))]));
        let moved = s.execute("actions.play", json!({"action":"Locked move"})).unwrap();
        assert!(moved.get("failed").is_some() && moved.to_string().contains("position-locked"));
        assert_eq!(s.active().unwrap().doc.layers, before.layers);
        assert_eq!(s.actions.playing, 0);
    }

    #[test]
    fn named_destinations_cannot_bypass_hidden_layer_protection() {
        let (mut s, hidden) = session();
        s.execute("layer.setProps", json!({"layer":hidden,"name":"hidden","visible":false})).unwrap();
        s.execute("layer.new.layer", json!({"name":"visible"})).unwrap();
        for command in ["paint.stroke", "paint.pencil", "paint.bucket", "paint.gradient"] {
            let params = json!({"layer":"hidden","target":{"colors":[0,2]},"points":[[8,8]],"from":[0,0],"to":[16,16],"x":8,"y":8});
            let before = s.active().unwrap().doc.clone();
            let steps = s.active().unwrap().history.past_len();
            assert!(s.execute(command, params).unwrap_err().to_string().contains(HIDDEN));
            assert_eq!(s.active().unwrap().doc.layer(photocraft_doc::LayerId(hidden)), before.layer(photocraft_doc::LayerId(hidden)));
            assert_eq!(s.active().unwrap().history.past_len(), steps);
        }
    }
}
