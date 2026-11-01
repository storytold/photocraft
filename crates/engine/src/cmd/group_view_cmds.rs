//! Layers panel disclosure state: expanding and collapsing layer groups (#126).
//!
//! A group's open/closed state is document data ([`photocraft_doc::Group::expanded`], the PSD
//! section divider's open/closed folder type, saved in `.pcraft` too) but, as in Photoshop,
//! toggling it is a view change: no history step, and a clean document stays clean.
//!
//! Photoshop's disclosure-triangle modifiers map to the params: ⌥-click is `nested` (the group
//! and every group inside it), ⌘-click is `siblings` (the group and the groups beside it in the
//! same parent), ⌥⌘-click is both. The Layers panel menu's Collapse All Groups is `all`.

use std::sync::Arc;

use photocraft_doc::{Layer, LayerContent, LayerId};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

const ID: &str = "layer.setExpanded";

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: ID.into(), msg: msg.into() }
}

fn has_group(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    if d.doc.walk().iter().any(|(_, _, l)| l.is_group()) { Ok(()) } else { Err("the document has no layer groups".into()) }
}

fn opt_bool(p: &Value, key: &str) -> Result<Option<bool>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_bool().map(Some).ok_or_else(|| bad(format!("`{key}` must be true or false"))),
    }
}

fn set_all(layers: &mut [Layer], expanded: bool) -> usize {
    let mut n = 0;
    for l in layers {
        if let LayerContent::Group(g) = &mut l.content {
            g.expanded = expanded;
            n += 1 + set_all(&mut g.children, expanded);
        }
    }
    n
}

/// Open or close `l` if it is a group, and with `nested` every group inside it too. Returns how
/// many groups it set.
fn set_group(l: &mut Layer, expanded: bool, nested: bool) -> usize {
    let LayerContent::Group(g) = &mut l.content else { return 0 };
    g.expanded = expanded;
    1 + if nested { set_all(&mut g.children, expanded) } else { 0 }
}

/// `{"layer":id?,"expanded":bool?,"all":bool?,"nested":bool?,"siblings":bool?}`: open or close a
/// group (no `expanded`: toggle from that group's state). `nested` also sets every group inside
/// it, `siblings` the groups beside it in the same parent (with `nested`, theirs too). With
/// `all`, every group in the document takes the new state; without a layer, `all` toggles from
/// "any group open".
fn set_expanded(s: &mut Session, p: &Value) -> Result<Value> {
    let want = opt_bool(p, "expanded")?;
    let all = opt_bool(p, "all")?.unwrap_or(false);
    let nested = opt_bool(p, "nested")?.unwrap_or(false);
    let siblings = opt_bool(p, "siblings")?.unwrap_or(false);
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let layer = match p.get("layer") {
        None | Some(Value::Null) if all => None,
        None | Some(Value::Null) => Some(st.active_layer.ok_or_else(|| bad("no layer given and no active layer"))?),
        Some(v) => Some(LayerId(v.as_u64().ok_or_else(|| bad("`layer` must be a layer id"))?)),
    };
    let current = match layer {
        Some(id) => match &st.doc.layer(id).ok_or(EngineError::NoLayer(id))?.content {
            LayerContent::Group(g) => g.expanded,
            _ => return Err(bad(format!("layer {} is not a group", id.0))),
        },
        None => st.doc.walk().iter().any(|(_, _, l)| matches!(&l.content, LayerContent::Group(g) if g.expanded)),
    };
    let expanded = want.unwrap_or(!current);
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    // Copy-on-write: history snapshots keep their own state (layers share tiles, so this is cheap).
    let doc = Arc::make_mut(&mut st.doc);
    let changed = if all {
        set_all(&mut doc.layers, expanded)
    } else {
        let id = layer.ok_or_else(|| bad("no layer"))?;
        let path = doc.path_of(id).ok_or(EngineError::NoLayer(id))?;
        let (&index, parent) = path.split_last().ok_or(EngineError::NoLayer(id))?;
        let level = if parent.is_empty() { &mut doc.layers } else { doc.layer_at_mut(parent).and_then(Layer::children_mut).ok_or(EngineError::NoLayer(id))? };
        if siblings {
            level.iter_mut().map(|l| set_group(l, expanded, nested)).sum()
        } else {
            level.get_mut(index).map_or(0, |l| set_group(l, expanded, nested))
        }
    };
    Ok(json!({"expanded": expanded, "groups": changed}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: ID,
        label: "Expand/Collapse Group",
        menu: &[],
        shortcut: None,
        params: r##"{"layer":id?,"expanded":bool?,"all":bool?,"nested":bool?,"siblings":bool?} (no expanded: toggle; nested: also the groups inside it; siblings: also the groups beside it; all: every group; not an undo step)"##,
        enabled: has_group,
        run: set_expanded,
        journal: false,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> (Session, u64, u64) {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
        s.execute("layer.new.layer", json!({"name": "deep"})).unwrap();
        let inner = s.execute("layer.groupLayers", json!({"name": "inner"})).unwrap()["layer"].as_u64().unwrap();
        let outer = s.execute("layer.groupLayers", json!({"layer": inner, "name": "outer"})).unwrap()["layer"].as_u64().unwrap();
        // Group Layers creates collapsed groups; the tests start from both open.
        assert!(!expanded(&s, inner) && !expanded(&s, outer));
        s.execute(ID, json!({"all": true, "expanded": true})).unwrap();
        (s, inner, outer)
    }

    fn expanded(s: &Session, id: u64) -> bool {
        matches!(&s.active().unwrap().doc.layer(LayerId(id)).unwrap().content, LayerContent::Group(g) if g.expanded)
    }

    #[test]
    fn toggles_one_group_without_history_or_dirtying() {
        let (mut s, inner, outer) = session();
        let st = s.active_mut().unwrap();
        st.saved_revision = st.revision;
        let undo_steps = s.active().unwrap().history.entries().len();
        assert!(expanded(&s, inner) && expanded(&s, outer));
        assert_eq!(s.execute(ID, json!({"layer": inner})).unwrap()["expanded"], false);
        assert!(!expanded(&s, inner) && expanded(&s, outer));
        assert_eq!(s.active().unwrap().history.entries().len(), undo_steps, "no history step");
        assert_eq!(s.active().unwrap().saved_revision, s.active().unwrap().revision, "still clean");
        s.execute(ID, json!({"layer": inner, "expanded": true})).unwrap();
        assert!(expanded(&s, inner));
        // The inspect JSON reports it.
        s.execute(ID, json!({"layer": outer, "expanded": false})).unwrap();
        assert_eq!(s.execute("document.inspect", json!({})).unwrap()["layers"][0]["expanded"], false);
    }

    #[test]
    fn all_sets_every_group() {
        let (mut s, inner, outer) = session();
        s.execute(ID, json!({"layer": outer, "all": true})).unwrap();
        assert!(!expanded(&s, inner) && !expanded(&s, outer));
        assert_eq!(s.execute(ID, json!({"all": true})).unwrap()["groups"], 2);
        assert!(expanded(&s, inner) && expanded(&s, outer));
    }

    fn new_layer(s: &mut Session, cmd: &str, p: Value) -> u64 {
        s.execute(cmd, p).unwrap()["layer"].as_u64().unwrap()
    }

    /// Background, `a { a1 { x }, a2 { y } }`, `b { b1 { z } }` and `top`, every group open.
    /// Returns `[a, a1, a2, b, b1]`.
    fn siblings_session() -> (Session, [u64; 5]) {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
        let x = new_layer(&mut s, "layer.new.layer", json!({"name": "x"}));
        let a1 = new_layer(&mut s, "layer.groupLayers", json!({"layer": x, "name": "a1"}));
        let a = new_layer(&mut s, "layer.groupLayers", json!({"layer": a1, "name": "a"}));
        // `layer.new.layer` lands above the active layer, in its parent.
        s.execute("layer.select", json!({"layer": a1})).unwrap();
        let y = new_layer(&mut s, "layer.new.layer", json!({"name": "y"}));
        let a2 = new_layer(&mut s, "layer.groupLayers", json!({"layer": y, "name": "a2"}));
        s.execute("layer.select", json!({"layer": a})).unwrap();
        let z = new_layer(&mut s, "layer.new.layer", json!({"name": "z"}));
        let b1 = new_layer(&mut s, "layer.groupLayers", json!({"layer": z, "name": "b1"}));
        let b = new_layer(&mut s, "layer.groupLayers", json!({"layer": b1, "name": "b"}));
        s.execute("layer.select", json!({"layer": b})).unwrap();
        s.execute("layer.new.layer", json!({"name": "top"})).unwrap();
        s.execute(ID, json!({"all": true, "expanded": true})).unwrap();
        let tree: Vec<String> = s.active().unwrap().doc.walk().iter().map(|(_, d, l)| format!("{d}:{}", l.name)).collect();
        assert_eq!(tree, ["0:Background", "0:a", "1:a1", "2:x", "1:a2", "2:y", "0:b", "1:b1", "2:z", "0:top"]);
        (s, [a, a1, a2, b, b1])
    }

    fn open_groups(s: &Session, ids: [u64; 5]) -> [bool; 5] {
        ids.map(|id| expanded(s, id))
    }

    #[test]
    fn nested_sets_the_group_and_every_group_inside_it() {
        // Photoshop's ⌥-click: the clicked group's subtree, nothing beside it.
        let (mut s, ids @ [a, a1, ..]) = siblings_session();
        assert_eq!(s.execute(ID, json!({"layer": a, "nested": true})).unwrap()["groups"], 3);
        assert_eq!(open_groups(&s, ids), [false, false, false, true, true]);
        s.execute(ID, json!({"layer": a, "nested": true})).unwrap();
        assert_eq!(open_groups(&s, ids), [true; 5], "toggles back open, nested groups too");
        // A group with no groups inside: only itself.
        assert_eq!(s.execute(ID, json!({"layer": a1, "nested": true})).unwrap()["groups"], 1);
        assert_eq!(open_groups(&s, ids), [true, false, true, true, true]);
    }

    #[test]
    fn siblings_set_the_groups_beside_it_and_leave_nested_ones() {
        // Photoshop's ⌘-click: the clicked level only; groups inside keep their state.
        let (mut s, ids @ [a, a1, ..]) = siblings_session();
        assert_eq!(s.execute(ID, json!({"layer": a, "siblings": true})).unwrap()["groups"], 2, "pixel layers aren't counted");
        assert_eq!(open_groups(&s, ids), [false, true, true, false, true]);
        s.execute(ID, json!({"layer": a, "siblings": true})).unwrap();
        assert_eq!(open_groups(&s, ids), [true; 5]);
        // Inside a group, the siblings are that group's other children, not the top level.
        s.execute(ID, json!({"layer": a1, "siblings": true})).unwrap();
        assert_eq!(open_groups(&s, ids), [true, false, false, true, true]);
    }

    #[test]
    fn siblings_and_nested_together_set_the_level_and_everything_below_it() {
        // Photoshop's ⌥⌘-click; on a top-level group that is every group in the document.
        let (mut s, ids @ [_, a1, _, b, _]) = siblings_session();
        assert_eq!(s.execute(ID, json!({"layer": b, "siblings": true, "nested": true})).unwrap()["groups"], 5);
        assert_eq!(open_groups(&s, ids), [false; 5]);
        s.execute(ID, json!({"layer": a1, "siblings": true, "nested": true, "expanded": true})).unwrap();
        assert_eq!(open_groups(&s, ids), [false, true, true, false, false], "a nested level leaves the levels above alone");
    }

    #[test]
    fn a_toggle_follows_the_clicked_group() {
        let (mut s, ids @ [a, _, _, b, _]) = siblings_session();
        s.execute(ID, json!({"layer": a, "expanded": false})).unwrap();
        assert_eq!(s.execute(ID, json!({"layer": a, "siblings": true})).unwrap()["expanded"], true);
        assert!(expanded(&s, a) && expanded(&s, b), "the closed group opens, and the open sibling stays open");
        s.execute(ID, json!({"layer": b, "siblings": true})).unwrap();
        assert_eq!(open_groups(&s, ids), [false, true, true, false, true]);
    }

    #[test]
    fn an_empty_group_is_set_like_any_other() {
        let (mut s, ids @ [a, ..]) = siblings_session();
        // A new group lands above the active layer (`top`), at the top level, with nothing inside.
        let empty = new_layer(&mut s, "layer.new.group", json!({"name": "empty"}));
        let last = s.active().unwrap().doc.walk().last().map(|(_, d, l)| format!("{d}:{}", l.name));
        assert_eq!(last.as_deref(), Some("0:empty"));
        assert_eq!(s.execute(ID, json!({"layer": empty, "nested": true})).unwrap()["groups"], 1);
        assert!(!expanded(&s, empty));
        assert_eq!(open_groups(&s, ids), [true; 5], "nothing beside it changes");
        assert_eq!(s.execute(ID, json!({"layer": a, "siblings": true, "expanded": true})).unwrap()["groups"], 3, "a, b and the empty group");
        assert!(expanded(&s, empty));
        assert_eq!(s.execute(ID, json!({"layer": empty, "siblings": true, "nested": true})).unwrap()["groups"], 6);
        assert!(!expanded(&s, empty) && open_groups(&s, ids) == [false; 5]);
    }

    #[test]
    fn a_locked_group_still_opens_and_closes() {
        // Locks guard what is in a layer, not whether the panel shows it. The groups inside a
        // locked group are locked through it.
        let (mut s, ids @ [a, a1, ..]) = siblings_session();
        s.execute("layer.select", json!({"layer": a})).unwrap();
        s.execute("layer.lockLayers", json!({"all": true})).unwrap();
        assert!(s.active().unwrap().doc.layer(LayerId(a)).unwrap().locks.all);
        assert_eq!(s.execute(ID, json!({"layer": a})).unwrap()["expanded"], false);
        assert_eq!(s.execute(ID, json!({"layer": a, "nested": true})).unwrap()["groups"], 3);
        assert_eq!(open_groups(&s, ids), [true; 5]);
        assert_eq!(s.execute(ID, json!({"layer": a1, "siblings": true})).unwrap()["groups"], 2);
        assert_eq!(open_groups(&s, ids), [true, false, false, true, true]);
    }

    #[test]
    fn modifier_clicks_add_no_history_step_and_keep_a_clean_document_clean() {
        let (mut s, [a, a1, ..]) = siblings_session();
        let st = s.active_mut().unwrap();
        st.saved_revision = st.revision;
        let undo_steps = s.active().unwrap().history.entries().len();
        for p in [json!({"layer": a, "nested": true}), json!({"layer": a1, "siblings": true}), json!({"layer": a, "siblings": true, "nested": true})] {
            s.execute(ID, p.clone()).unwrap();
            let st = s.active().unwrap();
            assert_eq!(st.history.entries().len(), undo_steps, "{p}: no history step");
            assert_eq!(st.saved_revision, st.revision, "{p}: still clean");
        }
    }

    #[test]
    fn bad_params_fail_gracefully() {
        let (mut s, _, _) = session();
        let bg = s.active().unwrap().doc.layers[0].id.0;
        for p in [
            json!({"layer": bg}),
            json!({"layer": bg, "siblings": true}),
            json!({"layer": u64::MAX}),
            json!({"layer": "x"}),
            json!({"layer": -1}),
            json!({"expanded": "yes", "all": true}),
            json!({"all": 3}),
            json!({"nested": "yes"}),
            json!({"siblings": 1}),
        ] {
            assert!(s.execute(ID, p.clone()).is_err(), "{p}");
        }
        let mut empty = Session::new();
        assert!(empty.execute(ID, json!({})).is_err());
        empty.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        assert!(empty.execute(ID, json!({"all": true})).is_err(), "no groups: disabled");
    }
}
