//! Layer › New › Frame from Layers. A frame clips its contents to a rectangle, so a template can
//! hold an image (or placeholder) that is cropped to the frame. We build it from existing
//! primitives: group the selected layers, then clip the group with a rectangular layer mask the
//! size of their bounds — the same visual result Photoshop's Frame tool produces for this command.
//! Headless and scriptable like every other command.

use photocraft_doc::{LayerMask, PixelFormat};
use photocraft_geom::Rect;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

fn has_layer(s: &Session) -> std::result::Result<(), String> {
    match s.active() {
        Some(d) if !d.selected_layers.is_empty() || d.active_layer.is_some() => Ok(()),
        Some(_) => Err("select a layer".into()),
        None => Err("no document".into()),
    }
}

/// A reveal mask that is on inside `rect` and off (hidden) everywhere else.
fn clip_mask(rect: Rect) -> LayerMask {
    let mut mask = LayerMask::hide_all();
    let (w, h) = (rect.width() as usize, rect.height() as usize);
    if w > 0 && h > 0 {
        mask.surface.write_region(rect, &vec![1.0f32; w * h]);
        mask.surface.prune();
    }
    // Default to GRAY8 like the other mask constructors.
    debug_assert_eq!(mask.surface.format(), PixelFormat::GRAY8);
    mask
}

/// Groups the selected layers, then turns the group into a frame, as one history step: one undo
/// takes the whole command back (#496).
fn frame_from_layers(s: &mut Session, p: &Value) -> Result<Value> {
    let mut gp = serde_json::Map::new();
    if let Some(name) = p.get("name") {
        gp.insert("name".into(), name.clone());
    }
    // Keep the nested grouping authorization even though it now shares the frame's transaction.
    if let Some(authorize) = s.authorize {
        authorize("layer.new.groupFromLayers", &Value::Object(gp))?;
    }
    let ids = crate::layer_multi_cmds::selected(s);
    let name = p.get("name").and_then(Value::as_str).map(str::to_string);
    // Two edits followed by purge_last lose the original snapshot when the second edit trims
    // history to one state. Build the group and its mask before recording a single edit instead.
    let (gid, rect) = s.edit("Frame from Layers", |doc, active| {
        let gid = crate::layer_multi_cmds::group_selection(doc, &ids, name)?;
        let rect =
            crate::layer_multi_cmds::layer_bounds(doc.layer(gid).ok_or(EngineError::NoLayer(gid))?).filter(|r| !r.is_empty()).unwrap_or_else(|| doc.bounds());
        let g = doc.layer_mut(gid).ok_or(EngineError::NoLayer(gid))?;
        g.mask = Some(clip_mask(rect));
        *active = Some(gid);
        Ok((gid, rect))
    })?;
    crate::layer_multi_cmds::reselect(s, vec![gid], Some(gid));
    Ok(json!({"layer": gid.0, "frame": [rect.x0, rect.y0, rect.width(), rect.height()]}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "layer.new.frameFromLayers",
        label: "Frame from Layers",
        menu: &["Layer", "New"],
        shortcut: None,
        params: r#"{"name":str?} → {layer, frame:[x,y,w,h]}: groups the selected layers and clips them to a frame rect"#,
        enabled: has_layer,
        journal: true,
        run: |s, p| frame_from_layers(s, p),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{Layer, LayerContent, LayerId};
    use photocraft_geom::Rect;

    fn session() -> (Session, LayerId, LayerId) {
        session_at_depth(8)
    }

    fn session_at_depth(depth: u32) -> (Session, LayerId, LayerId) {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 80, "height": 60, "depth": depth})).unwrap();
        let (a, b) = s
            .edit("setup", |doc, _| {
                let fmt = doc.pixel_format();
                let mut a = Layer::raster("A", fmt);
                a.surface_mut().unwrap().fill_rect(Rect::new(4, 4, 24, 24), &photocraft_raster::from_rgba(&fmt, [1.0, 0.0, 0.0, 1.0]));
                let mut b = Layer::raster("B", fmt);
                b.surface_mut().unwrap().fill_rect(Rect::new(30, 20, 50, 40), &photocraft_raster::from_rgba(&fmt, [0.0, 0.0, 1.0, 1.0]));
                let (ia, ib) = (a.id, b.id);
                doc.layers.push(a);
                doc.layers.push(b);
                Ok((ia, ib))
            })
            .unwrap();
        (s, a, b)
    }

    #[test]
    fn frames_selected_layers_into_a_masked_group() {
        let (mut s, a, b) = session();
        s.execute("layer.select", json!({"layer": a.0, "mode": "replace"})).unwrap();
        s.execute("layer.select", json!({"layer": b.0, "mode": "add"})).unwrap();
        let r = s.execute("layer.new.frameFromLayers", json!({"name": "Frame 1"})).unwrap();
        let gid = LayerId(r["layer"].as_u64().unwrap());
        // frame rect = union of the two layers' bounds (4,4)..(50,40).
        let frame = r["frame"].as_array().unwrap();
        assert_eq!(frame[2].as_i64().unwrap(), 50 - 4); // width
        assert_eq!(frame[3].as_i64().unwrap(), 40 - 4); // height
        let st = s.active().unwrap();
        let g = st.doc.layer(gid).unwrap();
        assert!(matches!(g.content, LayerContent::Group(_)), "it's a group");
        assert!(g.mask.is_some(), "the group has a clip mask");
        // Undoable (last step is the frame).
        assert_eq!(st.history.entries().last().map(|e| e.as_str()), Some("Frame from Layers"));
    }

    #[test]
    fn frame_from_layers_is_one_history_step() {
        // #496: grouping and framing undo together.
        let (mut s, a, b) = session();
        let entries = s.active().unwrap().history.entries();
        let layers = |s: &Session| s.active().unwrap().doc.layers.iter().map(|l| l.id).collect::<Vec<_>>();
        let before = layers(&s);
        s.execute("layer.select", json!({"layer": a.0, "mode": "replace"})).unwrap();
        s.execute("layer.select", json!({"layer": b.0, "mode": "add"})).unwrap();
        let gid = LayerId(s.execute("layer.new.frameFromLayers", json!({})).unwrap()["layer"].as_u64().unwrap());
        let mut framed = entries.clone();
        framed.push("Frame from Layers".into());
        assert_eq!(s.active().unwrap().history.entries(), framed);
        assert!(s.undo());
        assert_eq!(s.active().unwrap().history.entries(), entries);
        assert_eq!(layers(&s), before, "one undo leaves no group behind");
        assert!(s.redo());
        assert!(s.active().unwrap().doc.layer(gid).is_some_and(|g| g.mask.is_some()), "redo brings back the framed group");
    }

    #[test]
    fn needs_a_selected_layer() {
        let mut s = Session::new();
        assert!(s.execute("layer.new.frameFromLayers", json!({})).is_err());
    }

    #[test]
    fn frame_from_layers_keeps_its_undo_at_the_history_limit() {
        for depth in [8, 16, 32] {
            let (mut s, a, b) = session_at_depth(depth);
            s.execute("prefs.set", json!({"values": {"performance.historyStates": 1}})).unwrap();
            s.execute("layer.select", json!({"layer": a.0, "mode": "replace"})).unwrap();
            s.execute("layer.select", json!({"layer": b.0, "mode": "add"})).unwrap();
            let before = s.active().unwrap().doc.clone();
            let selected = s.active().unwrap().selected_layers();
            let gid = LayerId(s.execute("layer.new.frameFromLayers", json!({})).unwrap()["layer"].as_u64().unwrap());

            assert_eq!(s.active().unwrap().history.past_len(), 1, "{depth}-bit: the whole command retains one undo");
            assert!(s.undo());
            assert!(std::sync::Arc::ptr_eq(&s.active().unwrap().doc, &before));
            assert_eq!(s.active().unwrap().selected_layers(), selected);
            assert!(s.redo());
            assert!(s.active().unwrap().doc.layer(gid).is_some_and(|g| g.mask.is_some()));
            assert_eq!(s.active().unwrap().selected_layers(), vec![gid]);
        }
    }

    #[test]
    fn frame_from_layers_keeps_its_undo_over_the_memory_budget() {
        let (mut s, a, b) = session();
        s.execute("layer.select", json!({"layer": a.0, "mode": "replace"})).unwrap();
        s.execute("layer.select", json!({"layer": b.0, "mode": "add"})).unwrap();
        // Even when the current document exceeds its budget, history promises one undo.
        s.active_mut().unwrap().history.max_bytes = 1;
        let before = s.active().unwrap().doc.clone();
        s.execute("layer.new.frameFromLayers", json!({})).unwrap();
        assert_eq!(s.active().unwrap().history.past_len(), 1);
        assert!(s.undo());
        assert!(std::sync::Arc::ptr_eq(&s.active().unwrap().doc, &before));
    }

    #[test]
    fn frame_from_layers_does_not_evict_an_extra_history_state() {
        let (mut s, _, _) = session();
        s.execute("prefs.set", json!({"values": {"performance.historyStates": 2}})).unwrap();
        let before = s.active().unwrap().doc.clone();
        let oldest = s.active().unwrap().history.state(0).unwrap();
        s.execute("layer.new.frameFromLayers", json!({})).unwrap();
        assert_eq!(s.active().unwrap().history.past_len(), 2);
        assert!(s.undo());
        assert!(std::sync::Arc::ptr_eq(&s.active().unwrap().doc, &before));
        assert!(s.undo());
        assert!(std::sync::Arc::ptr_eq(&s.active().unwrap().doc, &oldest));
    }

    #[test]
    fn frame_from_layers_respects_grouping_authorization() {
        let (mut s, _, _) = session();
        let before = s.active().unwrap().doc.clone();
        let history = s.active().unwrap().history.entries();
        let journal = s.journal.clone();
        s.authorize = Some(|id, params| {
            if id == "layer.new.groupFromLayers" {
                assert_eq!(params, &json!({"name": "Frame 1"}));
                return Err(EngineError::Other("grouping denied".into()));
            }
            Ok(())
        });
        let error = s.execute("layer.new.frameFromLayers", json!({"name": "Frame 1"})).unwrap_err();
        assert_eq!(error.to_string(), "grouping denied");
        assert!(std::sync::Arc::ptr_eq(&s.active().unwrap().doc, &before));
        assert_eq!(s.active().unwrap().history.entries(), history);
        assert_eq!(s.journal, journal);
    }

    #[test]
    fn frame_from_layers_journals_only_the_complete_command() {
        let (mut s, _, _) = session();
        let start = s.journal.len();
        let params = json!({"name": "Frame 1"});
        s.execute("layer.new.frameFromLayers", params.clone()).unwrap();
        assert_eq!(&s.journal[start..], &[("layer.new.frameFromLayers".to_string(), params)]);
    }

    #[test]
    fn frame_from_layers_honours_the_coalescing_key() {
        let (mut s, _, _) = session();
        let before = s.active().unwrap().doc.clone();
        let steps = s.active().unwrap().history.past_len();
        for _ in 0..2 {
            s.execute("layer.new.frameFromLayers", json!({"coalesce": "frames"})).unwrap();
        }
        assert_eq!(s.active().unwrap().history.past_len(), steps + 1);
        assert!(s.undo());
        assert!(std::sync::Arc::ptr_eq(&s.active().unwrap().doc, &before));
        assert!(s.redo());
        let doc = &s.active().unwrap().doc;
        let frame = doc.layer(s.active().unwrap().active_layer.unwrap()).unwrap();
        assert!(frame.mask.is_some());
        assert!(frame.children().unwrap()[0].mask.is_some(), "redo restores both coalesced frames");
    }

    #[test]
    fn frame_from_layers_preserves_state_when_grouping_is_too_deep() {
        let (mut s, _, _) = session();
        for _ in 0..photocraft_doc::MAX_GROUP_DEPTH {
            s.execute("layer.groupLayers", json!({})).unwrap();
        }
        let before = s.active().unwrap().doc.clone();
        let history = s.active().unwrap().history.entries();
        let journal = s.journal.clone();
        assert!(s.execute("layer.new.frameFromLayers", json!({})).unwrap_err().to_string().contains("deeper than"));
        assert!(std::sync::Arc::ptr_eq(&s.active().unwrap().doc, &before));
        assert_eq!(s.active().unwrap().history.entries(), history);
        assert_eq!(s.journal, journal);
    }
}
