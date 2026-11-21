//! Group from Layers is refused while the Background is among the selected layers (#14709), as
//! the menu rows already grey it. Layer > Group Layers keeps grouping it (not measured).

use super::*;

/// A 32 x 32 document with a white Background and the pixel layers Red, Green and Blue above it.
fn session() -> (Session, LayerId, [LayerId; 3]) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 32, "height": 32, "background": "white"})).unwrap();
    let bg = s.active().unwrap().doc.layers[0].id;
    assert!(s.active().unwrap().doc.is_background(bg));
    let ids = ["Red", "Green", "Blue"].map(|name| LayerId(s.execute("layer.new.layer", json!({"name": name})).unwrap()["layer"].as_u64().unwrap()));
    (s, bg, ids)
}

fn select(s: &mut Session, ids: &[LayerId], active: LayerId) {
    set_selection(s, ids.to_vec(), Some(active), Some(active)).unwrap();
}

/// Everything a refused command must leave alone: the layer tree, the selection and the history.
fn snapshot(s: &Session) -> (Vec<Layer>, Vec<LayerId>, Option<LayerId>, Vec<String>) {
    let d = s.active().unwrap();
    (d.doc.layers.clone(), d.selected_layers(), d.active_layer, d.history.entries())
}

fn enabled(s: &Session) -> std::result::Result<(), String> {
    (crate::commands::find("layer.new.groupFromLayers").unwrap().enabled)(s)
}

fn refused(s: &mut Session, params: Value) {
    let before = snapshot(s);
    assert!(s.execute("layer.new.groupFromLayers", params.clone()).is_err(), "{params} must be refused");
    assert!(snapshot(s) == before, "{params} changed the document");
    assert!(s.active().unwrap().doc.background().is_some(), "the Background is still there");
}

#[test]
fn the_background_alone_is_refused() {
    let (mut s, bg, _) = session();
    select(&mut s, &[bg], bg);
    assert!(enabled(&s).is_err(), "disabled with the Background selected");
    refused(&mut s, json!({}));
    refused(&mut s, json!({"name": "Set", "color": "red", "mode": "normal", "opacity": 50}));
}

#[test]
fn the_background_with_other_layers_is_refused() {
    let (mut s, bg, [red, green, _]) = session();
    for active in [red, bg] {
        select(&mut s, &[bg, red], active);
        assert!(enabled(&s).is_err(), "disabled with the Background and Red selected");
        refused(&mut s, json!({}));
    }
    select(&mut s, &[bg, red, green], green);
    refused(&mut s, json!({}));
}

#[test]
fn an_explicit_background_layer_is_refused() {
    let (mut s, bg, [red, _, blue]) = session();
    select(&mut s, &[blue], blue);
    refused(&mut s, json!({"layer": bg.0}));
    select(&mut s, &[red, blue], red);
    refused(&mut s, json!({"layer": bg.0}));
    select(&mut s, &[bg], bg);
    refused(&mut s, json!({"layer": bg.0}));
}

#[test]
fn a_background_only_document_is_refused() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 32, "height": 32, "background": "white"})).unwrap();
    assert_eq!(s.active().unwrap().doc.layers.len(), 1);
    assert!(enabled(&s).is_err());
    refused(&mut s, json!({}));
}

fn children(s: &Session, group: LayerId) -> Vec<LayerId> {
    s.active().unwrap().doc.layer(group).unwrap().children().unwrap().iter().map(|l| l.id).collect()
}

#[test]
fn layers_without_the_background_still_group() {
    let (mut s, bg, [red, green, blue]) = session();
    select(&mut s, &[blue], blue);
    assert!(enabled(&s).is_ok());
    let g = LayerId(s.execute("layer.new.groupFromLayers", json!({})).unwrap()["layer"].as_u64().unwrap());
    assert_eq!(children(&s, g), vec![blue]);

    select(&mut s, &[red, green], green);
    let g = LayerId(s.execute("layer.new.groupFromLayers", json!({})).unwrap()["layer"].as_u64().unwrap());
    assert_eq!(children(&s, g), vec![red, green]);
    assert!(s.active().unwrap().doc.is_background(bg), "the Background stays");
}

#[test]
fn an_explicit_layer_other_than_the_background_still_groups() {
    let (mut s, bg, [red, _, blue]) = session();
    select(&mut s, &[blue], blue);
    let g = LayerId(s.execute("layer.new.groupFromLayers", json!({"layer": red.0})).unwrap()["layer"].as_u64().unwrap());
    assert_eq!(children(&s, g), vec![red]);
    assert!(s.active().unwrap().doc.is_background(bg));
}

/// Layer > Group Layers with the Background selected is not measured: it keeps today's result.
#[test]
fn group_layers_keeps_grouping_the_background() {
    let (mut s, bg, [red, ..]) = session();
    select(&mut s, &[bg, red], red);
    let g = LayerId(s.execute("layer.groupLayers", json!({})).unwrap()["layer"].as_u64().unwrap());
    assert_eq!(children(&s, g), vec![bg, red]);
}
