//! A named client may only change the layers it created: it makes a layer, or duplicates one,
//! and works on that. Any command of a named client that would change, remove or move another
//! layer is refused and leaves the document as it was. Calls without `client` (the person at the
//! window) are not limited.
use photocraft_engine::Session;
use photocraft_engine::doc::LayerId;
use photocraft_engine::jobs::Started;
use serde_json::{Value, json};

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const RULE: &str = "may only change the layers it created";

/// A white 64x32 document.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 32, "background": "white"})).unwrap();
    s
}

fn run(s: &mut Session, who: Option<&str>, id: &str, mut p: Value) -> Result<Value, String> {
    if let Some(w) = who {
        p["client"] = json!(w);
    }
    s.execute(id, p).map_err(|e| e.to_string())
}

/// A new layer made by `who`; its id.
fn new_layer(s: &mut Session, who: Option<&str>, name: &str) -> u64 {
    run(s, who, "layer.new.layer", json!({"name": name})).unwrap()["layer"].as_u64().unwrap()
}

fn fill(s: &mut Session, who: Option<&str>, color: &str) -> Result<Value, String> {
    run(s, who, "edit.fill", json!({"contents": "color", "color": color}))
}

fn pixel(s: &mut Session, x: i32, y: i32) -> [f32; 4] {
    let v = s.execute("document.pixel", json!({"x": x, "y": y})).unwrap();
    let a: Vec<f32> = v.as_array().unwrap().iter().map(|c| c.as_f64().unwrap() as f32).collect();
    [a[0], a[1], a[2], a[3]]
}

fn is(p: [f32; 4], want: [f32; 4]) -> bool {
    p.iter().zip(&want).all(|(a, b)| (a - b).abs() <= 0.02)
}

fn name(s: &Session, layer: u64) -> Option<String> {
    s.active().and_then(|st| st.doc.layer(LayerId(layer)).map(|l| l.name.clone()))
}

#[test]
/// Fails if a named client can paint on a layer it did not create (here the background).
fn a_client_cannot_change_a_layer_it_did_not_create() {
    let mut s = session();
    let background = s.active().unwrap().active_layer.unwrap().0;
    let refused = fill(&mut s, Some("s1-a"), "#ff0000").unwrap_err();
    assert!(refused.contains(RULE), "{refused}");
    assert!(refused.contains(&format!("would change layer {background} ")), "{refused}");
    assert!(refused.contains("not created by a client"), "{refused}");
    assert!(is(pixel(&mut s, 8, 16), WHITE), "the background is unchanged");
}

#[test]
/// Fails if a client cannot work on its own layer, or if another client can rename or delete it.
fn a_client_changes_its_own_layer_and_the_other_clients_cannot() {
    let mut s = session();
    let mine = new_layer(&mut s, Some("s1-a"), "mine");
    fill(&mut s, Some("s1-a"), "#ff0000").unwrap();
    assert!(is(pixel(&mut s, 8, 16), RED), "s1-a painted its own layer");

    let renamed = run(&mut s, Some("s1-b"), "layer.renameLayer", json!({"layer": mine, "name": "taken"})).unwrap_err();
    assert!(renamed.contains(RULE) && renamed.contains("created by `s1-a`"), "{renamed}");
    assert_eq!(name(&s, mine).as_deref(), Some("mine"));

    let deleted = run(&mut s, Some("s1-b"), "layer.delete", json!({"layer": mine})).unwrap_err();
    assert!(deleted.contains(RULE), "{deleted}");
    assert_eq!(name(&s, mine).as_deref(), Some("mine"), "the layer is still there");
    assert!(is(pixel(&mut s, 8, 16), RED), "and so are its pixels");
}

#[test]
/// Fails if the rule also stops a call without `client`.
fn a_call_without_client_can_change_any_layer() {
    let mut s = session();
    let theirs = new_layer(&mut s, Some("s1-a"), "theirs");
    run(&mut s, None, "layer.renameLayer", json!({"layer": theirs, "name": "by hand"})).unwrap();
    assert_eq!(name(&s, theirs).as_deref(), Some("by hand"));
}

#[test]
/// Fails if a client cannot duplicate a layer it did not create and work on the copy, or if the
/// original changes.
fn a_client_works_on_its_copy_of_a_layer() {
    let mut s = session();
    let background = s.active().unwrap().active_layer.unwrap();
    let original = s.active().unwrap().doc.layer(background).unwrap().clone();
    run(&mut s, Some("s1-a"), "layer.duplicate", json!({})).unwrap();
    fill(&mut s, Some("s1-a"), "#ff0000").unwrap();
    assert!(is(pixel(&mut s, 8, 16), RED), "the copy is painted");
    assert_eq!(s.active().unwrap().doc.layer(background), Some(&original), "the original is unchanged");
}

#[test]
/// Fails if a background job started by a named client lands on a layer that client did not
/// create.
fn a_job_of_a_client_does_not_land_on_another_layer() {
    let mut s = session();
    run(&mut s, None, "select.rect", json!({"x": 0, "y": 0, "width": 16, "height": 32})).unwrap();
    fill(&mut s, None, "#ff0000").unwrap();
    run(&mut s, None, "select.deselect", json!({})).unwrap();
    let Started::Job(id) = s.start("filter.blur.gaussianBlur", json!({"radius": 4, "client": "s1-a"})).unwrap() else {
        panic!("Gaussian Blur runs as a background job");
    };
    let refused = s.wait_job(id).unwrap_err().to_string();
    assert!(refused.contains(RULE), "{refused}");
    assert!(is(pixel(&mut s, 15, 16), RED), "the edge is not blurred");
    assert!(is(pixel(&mut s, 16, 16), WHITE), "nor the white next to it");
}
