//! Tests that run paint commands, moved from the engine-core `vector_cmds` tests: 2 test(s).

use photocraft_doc::Document;
use photocraft_engine::Session;
use serde_json::json;

fn session(w: u32, h: u32, depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "background": "white", "depth": depth})).unwrap();
    s
}

fn doc(s: &Session) -> &Document {
    &s.active().unwrap().doc
}

/// #2978: Simulate Pressure tapers the stroke at both ends (the brush's size follows pen
/// pressure); without it the stroke is as wide near its ends as in the middle.
#[test]
fn stroke_path_simulate_pressure_tapers_both_ends() {
    let coverage = |simulate: bool, x: i32| -> usize {
        let mut s = session(200, 60, 8);
        s.execute("layer.new.layer", json!({})).unwrap();
        // Simulate Pressure tapers a brush that follows pen pressure; the default brush stopped
        // doing that on its own (#11809).
        s.execute("tools.setBrush", json!({"brush": {"pressureSize": true}})).unwrap();
        s.execute("path.set", json!({"path": {"subpaths": [{"closed": false, "knots": [[20, 30], [180, 30]]}]}})).unwrap();
        let p = json!({"tool": "brush", "size": 20, "hardness": 1.0, "color": "#000000", "opacity": 100, "simulatePressure": simulate});
        s.execute("path.stroke", p).unwrap();
        let d = doc(&s);
        let surf = d.layer(s.active().unwrap().active_layer.unwrap()).unwrap().surface().unwrap();
        (0..60).filter(|&y| surf.rgba(x, y)[3] > 0.5).count()
    };
    let (end, middle) = (coverage(true, 26), coverage(true, 100));
    assert!(middle >= 18 && end * 3 < middle, "simulated: {end} px near the start, {middle} px in the middle");
    let (end, middle) = (coverage(false, 26), coverage(false, 100));
    assert!(middle >= 18 && end + 2 >= middle, "plain: {end} px near the start, {middle} px in the middle");
}

/// #2978: Stroke Path runs the other brush-based tools along the path (here the Blur tool) as one
/// history step, and only the path.stroke call is journalled, so an action replays it once.
#[test]
fn stroke_path_with_the_blur_tool_is_one_step() {
    let mut s = session(100, 60, 8);
    s.execute("layer.new.layer", json!({})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    s.execute("path.set", json!({"path": {"subpaths": [{"knots": [[0, 0], [100, 0], [100, 60], [0, 60]]}]}})).unwrap();
    s.execute("path.fill", json!({"color": "#ffffff"})).unwrap();
    s.execute("path.set", json!({"path": {"subpaths": [{"knots": [[0, 0], [50, 0], [50, 60], [0, 60]]}]}})).unwrap();
    s.execute("path.fill", json!({"color": "#000000"})).unwrap();
    let px = |s: &Session, x: i32| doc(s).layer(id).unwrap().surface().unwrap().rgba(x, 30)[0];
    assert!(px(&s, 49) < 0.01 && px(&s, 50) > 0.99);
    // Two subpaths along the edge: still one step.
    let edge = json!({"subpaths": [{"closed": false, "knots": [[50, 5], [50, 28]]}, {"closed": false, "knots": [[50, 28], [50, 55]]}]});
    s.execute("path.set", json!({"path": edge})).unwrap();
    let steps = s.active().unwrap().history.past_len();
    let journal = s.journal.len();
    s.execute("path.stroke", json!({"tool": "blur", "size": 10, "strength": 100})).unwrap();
    assert!(px(&s, 49) > 0.02 && px(&s, 50) < 0.98, "the edge is blurred: {} {}", px(&s, 49), px(&s, 50));
    assert_eq!(s.active().unwrap().history.past_len(), steps + 1);
    let ids: Vec<&str> = s.journal[journal..].iter().map(|(c, _)| c.as_str()).collect();
    assert_eq!(ids, ["path.stroke"]);
    s.execute("edit.undo", json!({})).unwrap();
    assert!(px(&s, 49) < 0.01 && px(&s, 50) > 0.99);
    assert!(s.execute("path.stroke", json!({"tool": "blur", "simulatePressure": "yes"})).is_err());
    assert!(s.execute("path.stroke", json!({"tool": "airbrush"})).is_err());
}
