use super::*;

/// A 40×20 document of the given depth: x 0..10 dark gray (#404040), x 10..20 mid gray
/// (#808080), x 20..30 pure red, the rest the white Background.
fn session_depth(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 20, "depth": depth})).unwrap();
    for (x, color) in [(0, "#404040"), (10, "#808080"), (20, "#ff0000")] {
        s.execute("select.rect", json!({"x": x, "y": 0, "width": 10, "height": 20, "mode": "replace"})).unwrap();
        s.execute("edit.fill", json!({"color": color})).unwrap();
    }
    s.execute("select.deselect", json!({})).unwrap();
    s
}

fn doc(s: &Session) -> &Document {
    &s.active().unwrap().doc
}

fn channel_at(s: &Session, i: usize, x: i32) -> f32 {
    doc(s).channels[i].surface.sample_channel(x, 5, 0)
}

fn mask_at(s: &Session, x: i32) -> f32 {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().mask.as_ref().unwrap().surface.sample_channel(x, 5, 0)
}

fn near(a: f32, b: f32, tol: f32) -> bool {
    (a - b).abs() <= tol
}

#[test]
fn specs_are_documented_and_registered() {
    for sp in specs() {
        assert!(crate::commands::find(sp.id).is_some(), "{} not registered", sp.id);
        assert!(sp.params.starts_with('{'), "{}", sp.id);
    }
}

#[test]
fn lights_into_a_layer_mask_at_every_depth() {
    for depth in [8, 16, 32] {
        let mut s = session_depth(depth);
        let r = s.execute(CMD, json!({"kind": "lights", "degree": 1, "output": "layerMask"})).unwrap();
        assert_eq!(r["layers"], 1);
        // #404040 ≈ 0.251, #808080 ≈ 0.502, white = 1.
        assert!(near(mask_at(&s, 5), 0.251, 0.02), "depth {depth}: {}", mask_at(&s, 5));
        assert!(near(mask_at(&s, 15), 0.502, 0.02), "depth {depth}: {}", mask_at(&s, 15));
        assert!(near(mask_at(&s, 35), 1.0, 0.01), "depth {depth}: {}", mask_at(&s, 35));
    }
}

#[test]
fn a_higher_degree_is_tighter() {
    let mut s = session_depth(8);
    s.execute(CMD, json!({"kind": "lights", "degree": 1, "output": "channel"})).unwrap();
    s.execute(CMD, json!({"kind": "lights", "degree": 3, "output": "channel"})).unwrap();
    assert!(channel_at(&s, 1, 15) < channel_at(&s, 0, 15), "Lights 3 is below Lights 1 on a midtone");
    assert!(near(channel_at(&s, 1, 35), 1.0, 0.01), "white stays fully selected");
}

#[test]
fn darks_and_midtones_into_channels() {
    let mut s = session_depth(8);
    let r = s.execute(CMD, json!({"kind": "darks", "degree": 2, "output": "channel"})).unwrap();
    assert_eq!(r["name"], "Darks 2");
    assert!(near(channel_at(&s, 0, 5), 0.749f32.powi(2), 0.03));
    assert!(near(channel_at(&s, 0, 35), 0.0, 0.01));
    s.execute(CMD, json!({"kind": "midtones", "output": "channel"})).unwrap();
    assert!(near(channel_at(&s, 1, 15), 1.0, 0.03), "mid gray is the peak of Midtones");
    assert!(near(channel_at(&s, 1, 35), 0.0, 0.01));
}

#[test]
fn a_second_mask_of_the_same_name_gets_a_free_name() {
    let mut s = session_depth(8);
    s.execute(CMD, json!({"kind": "lights", "output": "channel"})).unwrap();
    let r = s.execute(CMD, json!({"kind": "lights", "output": "channel"})).unwrap();
    assert_eq!(r["name"], "Lights 1 (2)");
    let r = s.execute(CMD, json!({"kind": "lights", "output": "channel", "name": "Mine"})).unwrap();
    assert_eq!(r["name"], "Mine");
}

#[test]
fn colour_masks_with_and_without_a_tone() {
    let mut s = session_depth(8);
    s.execute(CMD, json!({"kind": "reds", "output": "channel"})).unwrap();
    assert!(channel_at(&s, 0, 25) > 0.5, "the red patch is selected");
    assert!(near(channel_at(&s, 0, 35), 0.0, 0.01), "white is no colour");
    assert!(near(channel_at(&s, 0, 5), 0.0, 0.01), "gray is no colour");
    s.execute(CMD, json!({"kind": "reds", "tone": "darks", "toneDegree": 2, "output": "channel"})).unwrap();
    assert!(channel_at(&s, 1, 25) < channel_at(&s, 0, 25), "toning only lowers the mask");
}

#[test]
fn selection_output_combines_by_operation() {
    let mut s = session_depth(8);
    let r = s.execute(CMD, json!({"kind": "lights", "output": "selection"})).unwrap();
    assert_eq!(r["selected"], true);
    let at = |s: &Session, x: i32| doc(s).selection.as_ref().map_or(0.0, |m| m.sample_channel(x, 5, 0));
    assert!(near(at(&s, 35), 1.0, 0.01));
    assert!(at(&s, 5) < 0.5);
    s.execute(CMD, json!({"kind": "darks", "degree": 3, "output": "selection", "operation": "subtract"})).unwrap();
    assert!(near(at(&s, 35), 1.0, 0.01), "white is not dark, so subtracting Darks keeps it");
}

#[test]
fn one_history_step_and_undo() {
    let mut s = session_depth(8);
    s.execute(CMD, json!({"kind": "lights", "output": "channel"})).unwrap();
    assert_eq!(doc(&s).channels.len(), 1);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(doc(&s).channels.len(), 0, "one undo removes the whole mask");
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(doc(&s).channels.len(), 1);

    s.execute(CMD, json!({"kind": "darks", "output": "layerMask"})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    let d = s.active().unwrap();
    assert!(d.doc.layer(d.active_layer.unwrap()).unwrap().mask.is_none());
}

#[test]
fn bad_params_are_errors_not_panics() {
    let mut s = session_depth(8);
    for p in [
        json!({"degree": 0}),
        json!({"degree": 7}),
        json!({"degree": 2.5}),
        json!({"degree": "2"}),
        json!({"degree": -1}),
        json!({"kind": "bogus"}),
        json!({"kind": 3}),
        json!({"output": "bogus"}),
        json!({"operation": "bogus"}),
        json!({"kind": "reds", "tone": "bogus"}),
        json!({"kind": "reds", "tone": 4}),
        json!({"kind": "reds", "tone": "lights", "toneDegree": 9}),
        json!({"layer": "no such layer"}),
    ] {
        assert!(s.execute(CMD, p.clone()).is_err(), "{p} should be refused");
    }
    assert_eq!(doc(&s).channels.len(), 0, "a refused command leaves the document alone");
}

#[test]
fn no_document_is_an_error() {
    let mut s = Session::new();
    assert!(s.execute(CMD, json!({})).is_err());
}
