use super::*;

fn session() -> photocraft_engine::Session {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s
}

fn shadow_fields(kind: &str, global: bool) -> Map<String, Value> {
    let mut f = Map::new();
    f.insert("selected".into(), json!("fx1"));
    f.insert("globalLight".into(), json!(120.0));
    f.insert("effects".into(), json!([{"id": "fx1", "kind": kind, "on": true, "params": {"angle": 120.0, "distance": 3.0, "useGlobalLight": global}}]));
    f
}

fn angle_distance(f: &Map<String, Value>) -> (f64, f64) {
    let p = &entry(f, "fx1").unwrap()["params"];
    (p["angle"].as_f64().unwrap(), p["distance"].as_f64().unwrap())
}

/// The measured sequence from 120° / 3 px: (+20, +20) then (-20, 0), each its own press.
fn drag_measured_sequence(f: &mut Map<String, Value>) -> [(f64, f64); 2] {
    drag_shadow(f, true, [0.0, 0.0]);
    drag_shadow(f, false, [20.0, 20.0]);
    let first = angle_distance(f);
    drag_shadow(f, true, [0.0, 0.0]);
    drag_shadow(f, false, [-20.0, 0.0]);
    [first, angle_distance(f)]
}

/// #14894: a drag sets the Angle by cutting off the fraction (133.6° → 133°), as measured.
#[test]
fn drop_shadow_drag_truncates_the_angle() {
    let mut f = shadow_fields("dropShadow", false);
    assert_eq!(drag_measured_sequence(&mut f), [(133.0, 31.0), (92.0, 23.0)]);
}

/// #14894: Inner Shadow drags use the same rule.
#[test]
fn inner_shadow_drag_truncates_the_angle() {
    let mut f = shadow_fields("innerShadow", false);
    assert_eq!(drag_measured_sequence(&mut f), [(133.0, 31.0), (92.0, 23.0)]);
}

/// #14894: with Use Global Light on, the drag changes the Global Light; Cancel (dropping the
/// fields) leaves the document's at 120, and OK commits the dragged angle.
#[test]
fn drag_with_global_light_moves_it_and_cancel_restores_it() {
    let mut s = session();
    s.execute("layer.layerStyle.dropShadow", json!({})).unwrap();
    let st = s.active().unwrap();
    let id = st.active_layer.unwrap();
    assert_eq!(st.doc.global_light.angle, 120.0);
    let mut f = initial_fields(st.doc.layer(id).unwrap(), Some("dropShadow"), st.doc.global_light.angle);
    set_param(&mut f, "fx1", "useGlobalLight", json!(true));
    set_param(&mut f, "fx1", "angle", json!(120.0));
    set_param(&mut f, "fx1", "distance", json!(3.0));
    assert_eq!(drag_measured_sequence(&mut f), [(133.0, 31.0), (92.0, 23.0)]);
    assert_eq!(f["pendingGlobalLight"], json!(92.0), "the drag moves the Global Light");
    // Cancel: the fields are dropped and the document was never touched.
    assert_eq!(s.active().unwrap().doc.global_light.angle, 120.0);
    // OK: the dragged angle becomes the document's Global Light.
    apply(&f, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
    assert_eq!(s.active().unwrap().doc.global_light.angle, 92.0);
}
