//! #14713: Group from Layers (and Group Layers, which runs the same code) keeps clip groups whole
//! as the reference application does: a base takes the layers clipped to it into the group, still
//! clipped, whether they were selected or not, and a clipped layer that goes in without its base is
//! released. The new group is never clipped. One history step; one undo restores the stack.

use serde_json::{Value, json};

use crate::Session;

/// The issue's stack, bottom to top: Background, Red, Green (clipped to Red), Blue.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16, "background": "white"})).unwrap();
    for name in ["Red", "Green", "Blue"] {
        let id = s.execute("layer.new.layer", json!({"name": name})).unwrap()["layer"].clone();
        if name == "Green" {
            s.execute("layer.setProps", json!({"layer": id, "clipped": true})).unwrap();
        }
    }
    assert_eq!(stack(&s), "Blue, Green*, Red, Background");
    s
}

fn id(s: &Session, name: &str) -> Value {
    let doc = &s.active().unwrap().doc;
    json!(doc.walk().into_iter().find(|(_, _, l)| l.name == name).unwrap().2.id.0)
}

fn select(s: &mut Session, names: &[&str]) {
    for (i, name) in names.iter().enumerate() {
        let layer = id(s, name);
        s.execute("layer.select", json!({"layer": layer, "mode": if i == 0 { "replace" } else { "add" }})).unwrap();
    }
}

/// The stack top to bottom, `*` marking clipped layers and braces a group's children.
fn stack(s: &Session) -> String {
    fn list(layers: &[photocraft_doc::Layer]) -> String {
        let row = |l: &photocraft_doc::Layer| {
            let name = format!("{}{}", l.name, if l.clipped { "*" } else { "" });
            match l.children() {
                Some(kids) => format!("{name}{{{}}}", list(kids)),
                None => name,
            }
        };
        layers.iter().rev().map(row).collect::<Vec<_>>().join(", ")
    }
    list(&s.active().unwrap().doc.layers)
}

fn group(s: &mut Session, names: &[&str], command: &str) {
    select(s, names);
    s.execute(command, json!({})).unwrap_or_else(|e| panic!("{command} on {names:?}: {e}"));
}

#[test]
fn the_four_measured_selections_give_the_reference_trees() {
    for command in ["layer.new.groupFromLayers", "layer.groupLayers"] {
        let cases: [(&[&str], &str); 4] = [
            (&["Green"], "Blue, Group 1{Green}, Red, Background"),
            (&["Red"], "Blue, Group 1{Green*, Red}, Background"),
            (&["Red", "Green"], "Blue, Group 1{Green*, Red}, Background"),
            (&["Green", "Blue"], "Group 1{Blue, Green}, Red, Background"),
        ];
        for (names, want) in cases {
            let mut s = session();
            group(&mut s, names, command);
            assert_eq!(stack(&s), want, "{command} on {names:?}");
        }
    }
}

#[test]
fn layers_outside_a_clip_group_keep_todays_result() {
    let mut s = session();
    group(&mut s, &["Blue"], "layer.new.groupFromLayers");
    assert_eq!(stack(&s), "Group 1{Blue}, Green*, Red, Background");
}

#[test]
fn one_history_step_and_one_undo_restores_the_clipping() {
    let mut s = session();
    select(&mut s, &["Red"]);
    let before = s.active().unwrap().history.past_len();
    s.execute("layer.new.groupFromLayers", json!({})).unwrap();
    assert_eq!(s.active().unwrap().history.past_len(), before + 1, "one history step");
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(stack(&s), "Blue, Green*, Red, Background");
}

#[test]
fn one_undo_restores_the_clipping_a_group_released() {
    // Green goes in without its base, so grouping clears its clipping; the one undo must set it
    // again, not only put Green back in place.
    let mut s = session();
    select(&mut s, &["Green"]);
    let before = s.active().unwrap().history.past_len();
    s.execute("layer.new.groupFromLayers", json!({})).unwrap();
    assert_eq!(stack(&s), "Blue, Group 1{Green}, Red, Background", "Green is released");
    assert_eq!(s.active().unwrap().history.past_len(), before + 1, "one history step");
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(stack(&s), "Blue, Green*, Red, Background");
}

#[test]
fn a_clipped_layer_grouped_with_an_unrelated_layer_below_it_is_released() {
    // Background, Low, Red, Green* (clipped to Red), Blue: Low + Green into a group must not leave
    // Green clipped to Low.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16, "background": "white"})).unwrap();
    for name in ["Low", "Red", "Green", "Blue"] {
        let layer = s.execute("layer.new.layer", json!({"name": name})).unwrap()["layer"].clone();
        if name == "Green" {
            s.execute("layer.setProps", json!({"layer": layer, "clipped": true})).unwrap();
        }
    }
    group(&mut s, &["Low", "Green"], "layer.new.groupFromLayers");
    assert_eq!(stack(&s), "Blue, Group 1{Green, Low}, Red, Background");
}
