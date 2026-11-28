//! Tests that run paint commands, moved from the engine-core `layer_place` tests: 4 test(s).

use photocraft_doc::{Document, LayerId};

use serde_json::{Value, json};

use photocraft_engine::Session;

/// Background, then `A` and `B` (`B` active).
fn session() -> (Session, LayerId, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    let a = id(s.execute("layer.new.layer", json!({"name": "A"})).unwrap());
    let b = id(s.execute("layer.new.layer", json!({"name": "B"})).unwrap());
    (s, a, b)
}

fn id(v: Value) -> LayerId {
    LayerId(v["layer"].as_u64().unwrap())
}

fn doc(s: &Session) -> &Document {
    &s.active().unwrap().doc
}

fn top(s: &Session) -> Vec<LayerId> {
    doc(s).layers.iter().map(|l| l.id).collect()
}

fn children(s: &Session, group: LayerId) -> Vec<LayerId> {
    doc(s).layer(group).unwrap().children().unwrap().iter().map(|l| l.id).collect()
}

fn set_expanded(s: &mut Session, group: LayerId, expanded: bool) {
    s.execute("layer.setExpanded", json!({"layer": group.0, "expanded": expanded})).unwrap();
}

fn select(s: &mut Session, layer: LayerId) {
    s.execute("layer.select", json!({"layer": layer.0})).unwrap();
}

/// The clipping rule Photoshop follows on every insert (#2203): a new layer is clipped exactly
/// when the layer just above its slot is clipped.
#[test]
fn a_new_layer_joins_the_clipping_mask_it_lands_in() {
    let (mut s, base, c1) = session();
    let c2 = id(s.execute("layer.new.layer", json!({"name": "C2"})).unwrap());
    for c in [c1, c2] {
        s.execute("layer.setProps", json!({"layer": c.0, "clipped": true})).unwrap();
    }
    let clipped = |s: &Session, l: LayerId| doc(s).layer(l).unwrap().clipped;
    // On the base: between the base and its clipped layers.
    select(&mut s, base);
    let on_base = id(s.execute("layer.new.layer", json!({})).unwrap());
    assert!(clipped(&s, on_base));
    // In the middle of the chain.
    select(&mut s, c1);
    let mid = id(s.execute("layer.new.layer", json!({})).unwrap());
    assert!(clipped(&s, mid));
    // Above the top clipped layer it is outside the mask.
    select(&mut s, c2);
    let above = id(s.execute("layer.new.layer", json!({})).unwrap());
    assert!(!clipped(&s, above));
    // ⌘-click New Layer under a clipped layer joins it too; under the base it does not.
    select(&mut s, c2);
    let l = id(s.execute("layer.new.layer", json!({"position": "below"})).unwrap());
    assert!(clipped(&s, l));
    select(&mut s, base);
    let l = id(s.execute("layer.new.layer", json!({"position": "below"})).unwrap());
    assert!(!clipped(&s, l));
    // A group joins as well (Photoshop clips groups too), and so do the other new layers.
    select(&mut s, base);
    let l = id(s.execute("layer.new.group", json!({})).unwrap());
    assert!(clipped(&s, l));
    select(&mut s, base);
    let l = id(s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap());
    assert!(clipped(&s, l));
    // A layer made from a clipped layer (Layer via Copy) stays in its mask.
    select(&mut s, c1);
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 4, "height": 4})).unwrap();
    s.execute("edit.fill", json!({"color": "#00ff00"})).unwrap();
    let l = id(s.execute("layer.new.layerViaCopy", json!({})).unwrap());
    assert!(clipped(&s, l));
}

#[test]
fn a_pasted_layer_goes_into_an_open_group() {
    let (mut s, a, _) = session();
    // A whole pixel layer on the clipboard (no selection).
    select(&mut s, a);
    s.execute("edit.fill", json!({"color": "#00ff00"})).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    let into = id(s.execute("layer.new.group", json!({"name": "Into"})).unwrap());
    let pasted = id(s.execute("edit.paste", json!({})).unwrap());
    assert_eq!(children(&s, into), vec![pasted]);
}

/// Any lock on an open group still lets New Layer, New Group and new adjustment and fill layers in,
/// as in Photoshop (checked with Lock All). A layer that is edited right after it is made (Type,
/// Shape, Paste, Render…) goes above a group whose lock would refuse that edit,
/// so it never comes out locked.
#[test]
fn locked_groups_take_new_layers_but_not_the_ones_edited_at_once() {
    for locks in [json!({"all": true}), json!({"pixels": true}), json!({"transparency": true}), json!({"position": true})] {
        let (mut s, _, b) = session();
        select(&mut s, b);
        s.execute("edit.fill", json!({"color": "#00ff00"})).unwrap();
        s.execute("select.all", json!({})).unwrap();
        s.execute("edit.copy", json!({})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        let g = id(s.execute("layer.new.group", json!({})).unwrap());
        s.execute("layer.setProps", json!({"layer": g.0, "locks": locks})).unwrap();
        for (cmd, p) in [
            ("layer.new.layer", json!({})),
            ("layer.new.group", json!({})),
            ("layer.newAdjustmentLayer.invert", json!({})),
            ("layer.newFillLayer.solidColor", json!({"color": "#ff0000"})),
        ] {
            select(&mut s, g);
            let l = id(s.execute(cmd, p).unwrap());
            assert_eq!(children(&s, g).last(), Some(&l), "{cmd} goes inside the group locked with {locks}");
        }
        for (cmd, p) in [
            ("type.create", json!({"x": 2, "y": 10, "text": "Hi"})),
            ("shape.create", json!({"kind": "rect", "rect": [1, 1, 5, 5]})),
            ("edit.paste", json!({})),
            ("filter.render.tree", json!({})),
            ("gradient.fill.create", json!({"from": [0, 0], "to": [16, 16]})),
        ] {
            select(&mut s, g);
            let l = id(s.execute(cmd, p).unwrap_or_else(|e| panic!("{cmd} with {locks}: {e}")));
            assert!(!children(&s, g).contains(&l), "{cmd} stays out of the group locked with {locks}");
            assert_eq!(top(&s).iter().position(|x| *x == l), top(&s).iter().position(|x| *x == g).map(|i| i + 1), "{cmd} directly above the group");
            assert!(!doc(&s).effective_locks(l).any(), "{cmd}: the new layer is not locked");
            if cmd == "filter.render.tree" {
                assert!(doc(&s).layer(l).unwrap().surface().unwrap().tile_count() > 0, "Render drew the tree with {locks}");
            }
        }
    }
    // An open group inside a locked one still takes New Layer (the lock reaches it through the outer group).
    let (mut s, _, b) = session();
    select(&mut s, b);
    let outer = id(s.execute("layer.new.group", json!({})).unwrap());
    let inner = id(s.execute("layer.new.group", json!({})).unwrap());
    assert_eq!(children(&s, outer), vec![inner]);
    s.execute("layer.setProps", json!({"layer": outer.0, "locks": {"pixels": true}})).unwrap();
    select(&mut s, inner);
    let plain = id(s.execute("layer.new.layer", json!({})).unwrap());
    assert_eq!(children(&s, inner), vec![plain]);
}

/// Stamp Visible goes directly above an open active group, never inside it: the stamp is the
/// composite already, and inside it the group's opacity, mode or mask would apply to it again.
/// With a layer inside a group active, it still lands above that layer, inside the group.
#[test]
fn stamp_visible_goes_above_an_open_group() {
    let (mut s, a, b) = session();
    select(&mut s, a);
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let g = id(s.execute("layer.groupLayers", json!({})).unwrap());
    set_expanded(&mut s, g, true);
    s.execute("layer.setProps", json!({"layer": g.0, "opacity": 0.5})).unwrap();
    select(&mut s, g);
    let stamp = id(s.execute("layer.stampVisible", json!({})).unwrap());
    assert_eq!(children(&s, g), vec![a], "the stamp stays out of the group");
    assert_eq!(top(&s)[1..], [g, stamp, b]);
    // A layer inside the group active: the stamp goes above it, inside the group.
    select(&mut s, a);
    let inside = id(s.execute("layer.stampVisible", json!({})).unwrap());
    assert_eq!(children(&s, g), vec![a, inside]);
}
