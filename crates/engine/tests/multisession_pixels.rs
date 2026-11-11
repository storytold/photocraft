//! F4: the pixel selection (`Document.selection`) belongs to each named client. Everything goes
//! through the public commands and their `client` parameter: `select.rect`, `edit.fill`,
//! `filter.blur.gaussianBlur` (a job, via `Session::start`), `document.inspect` (its
//! `selectionBounds` is the selection the calling client sees) and `document.pixel`.
use photocraft_engine::Session;
use photocraft_engine::jobs::Started;
use serde_json::{Value, json};

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

/// A white 64x32 document.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 32, "background": "white"})).unwrap();
    s
}

/// Params with an optional `client`.
fn with(client: Option<&str>, mut p: Value) -> Value {
    if let Some(c) = client {
        p["client"] = json!(c);
    }
    p
}

fn select(s: &mut Session, client: Option<&str>, x: i32, y: i32, w: u32, h: u32) {
    s.execute("select.rect", with(client, json!({"x": x, "y": y, "width": w, "height": h}))).unwrap();
}

/// A named client may only change the layers it created: give it one, which becomes its active
/// layer.
fn own_layer(s: &mut Session, client: &str) {
    s.execute("layer.new.layer", with(Some(client), json!({"name": client}))).unwrap();
}

fn fill(s: &mut Session, client: Option<&str>, color: &str) {
    s.execute("edit.fill", with(client, json!({"contents": "color", "color": color}))).unwrap();
}

/// `[x, y, width, height]` of the selection the client sees, or null (document.inspect).
fn bounds(s: &mut Session, client: Option<&str>) -> Value {
    s.execute("document.inspect", with(client, json!({}))).unwrap()["selectionBounds"].clone()
}

fn pixel(s: &mut Session, x: i32, y: i32) -> [f32; 4] {
    let v = s.execute("document.pixel", json!({"x": x, "y": y})).unwrap();
    let a: Vec<f32> = v.as_array().unwrap().iter().map(|c| c.as_f64().unwrap() as f32).collect();
    [a[0], a[1], a[2], a[3]]
}

fn is(p: [f32; 4], want: [f32; 4]) -> bool {
    p.iter().zip(&want).all(|(a, b)| (a - b).abs() <= 0.02)
}

#[test]
/// F4 (a). Fails if the pixel selection is shared by the clients: s1-b's fill would also hit
/// the rectangle of s1-a (or the second select.rect would replace the first one's area).
fn each_client_fills_only_its_own_rectangle() {
    let mut s = session();
    own_layer(&mut s, "s1-a");
    own_layer(&mut s, "s1-b");
    select(&mut s, Some("s1-a"), 0, 0, 16, 32);
    select(&mut s, Some("s1-b"), 32, 0, 16, 32);
    fill(&mut s, Some("s1-a"), "#ff0000");
    fill(&mut s, Some("s1-b"), "#0000ff");
    for (x, y) in [(0, 0), (8, 16), (15, 31)] {
        assert!(is(pixel(&mut s, x, y), RED), "s1-a rectangle at {x},{y}");
    }
    for (x, y) in [(32, 0), (40, 16), (47, 31)] {
        assert!(is(pixel(&mut s, x, y), BLUE), "s1-b rectangle at {x},{y}");
    }
    for (x, y) in [(16, 0), (24, 16), (31, 31), (48, 0), (63, 31)] {
        assert!(is(pixel(&mut s, x, y), WHITE), "outside both rectangles at {x},{y}");
    }
}

#[test]
/// F4 (b). Fails if the pixel selection is shared: s1-b's select.rect would replace the bounds
/// that s1-a reads afterwards.
fn a_selection_by_one_client_leaves_the_other_clients_selection_intact() {
    let mut s = session();
    select(&mut s, Some("s1-a"), 2, 3, 10, 5);
    assert_eq!(bounds(&mut s, Some("s1-a")), json!([2, 3, 10, 5]));
    select(&mut s, Some("s1-b"), 30, 10, 20, 8);
    assert_eq!(bounds(&mut s, Some("s1-a")), json!([2, 3, 10, 5]), "s1-a after s1-b selected");
    assert_eq!(bounds(&mut s, Some("s1-b")), json!([30, 10, 20, 8]));
    // Deselecting for s1-b does not deselect s1-a either.
    s.execute("select.deselect", with(Some("s1-b"), json!({}))).unwrap();
    assert_eq!(bounds(&mut s, Some("s1-b")), Value::Null);
    assert_eq!(bounds(&mut s, Some("s1-a")), json!([2, 3, 10, 5]), "s1-a after s1-b deselected");
}

#[test]
/// F4 (c). Fails if a new client inherits the document's current pixel selection (the one s1-a
/// just made): it would read those bounds and its fill would stay inside them.
fn a_new_client_has_no_pixel_selection() {
    let mut s = session();
    select(&mut s, Some("s1-a"), 0, 0, 16, 32);
    assert_eq!(bounds(&mut s, Some("never-seen")), Value::Null);
    own_layer(&mut s, "never-seen");
    fill(&mut s, Some("never-seen"), "#ff0000");
    assert!(is(pixel(&mut s, 8, 16), RED), "the whole document is open to a client without selection");
    assert!(is(pixel(&mut s, 40, 16), RED), "outside s1-a's rectangle too");
    assert_eq!(bounds(&mut s, Some("s1-a")), json!([0, 0, 16, 32]));
}

#[test]
/// F4 (d). Fails if the agents' selections go through the interface's slot: the interface (no
/// `client`) would read or fill their rectangles instead of its own.
fn the_interface_selection_survives_the_agents() {
    let mut s = session();
    select(&mut s, None, 56, 0, 8, 8);
    own_layer(&mut s, "s1-a");
    select(&mut s, Some("s1-a"), 0, 0, 16, 32);
    fill(&mut s, Some("s1-a"), "#ff0000");
    select(&mut s, Some("s1-b"), 32, 0, 16, 32);
    s.execute("select.deselect", with(Some("s1-b"), json!({}))).unwrap();
    assert_eq!(bounds(&mut s, None), json!([56, 0, 8, 8]), "the interface's selection");
    fill(&mut s, None, "#0000ff");
    assert!(is(pixel(&mut s, 60, 4), BLUE), "inside the interface's rectangle");
    assert!(is(pixel(&mut s, 40, 16), WHITE), "outside it: s1-b's old rectangle untouched");
    assert!(is(pixel(&mut s, 8, 16), RED), "s1-a's pixels are kept");
    assert_eq!(bounds(&mut s, Some("s1-a")), json!([0, 0, 16, 32]));
}

#[test]
/// F4 (e). Fails if a job started with `client` ignores that client's selection or touches the
/// other slots: the blur would also soften the white next to the red edge (x=16, outside
/// s1-a's rectangle) or the blue edge of s1-b (x=32), or change the stored selections.
fn a_job_uses_the_selection_of_its_client_and_leaves_the_others() {
    let mut s = session();
    select(&mut s, None, 56, 0, 8, 8);
    own_layer(&mut s, "s1-b");
    own_layer(&mut s, "s1-a");
    select(&mut s, Some("s1-b"), 32, 0, 16, 32);
    fill(&mut s, Some("s1-b"), "#0000ff");
    select(&mut s, Some("s1-a"), 0, 0, 16, 32);
    fill(&mut s, Some("s1-a"), "#ff0000");
    let Started::Job(id) = s.start("filter.blur.gaussianBlur", json!({"radius": 4, "client": "s1-a"})).unwrap() else {
        panic!("Gaussian Blur runs as a background job");
    };
    s.wait_job(id).unwrap();
    // Inside s1-a's rectangle the red edge is softened; everything outside stays exact.
    assert!(!is(pixel(&mut s, 15, 16), RED), "the edge inside s1-a's selection is blurred");
    assert!(is(pixel(&mut s, 16, 16), WHITE), "just outside s1-a's selection");
    assert!(is(pixel(&mut s, 32, 16), BLUE), "s1-b's edge is not blurred");
    assert!(is(pixel(&mut s, 31, 16), WHITE), "next to s1-b's rectangle");
    assert_eq!(bounds(&mut s, Some("s1-a")), json!([0, 0, 16, 32]));
    assert_eq!(bounds(&mut s, Some("s1-b")), json!([32, 0, 16, 32]));
    assert_eq!(bounds(&mut s, None), json!([56, 0, 8, 8]), "the interface's selection");
    assert_eq!(bounds(&mut s, Some("never-seen")), Value::Null);
}
