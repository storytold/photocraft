//! Explicit subscription smoke test. Ordinary CI and broad --ignored runs do not send requests.
use photocraft_engine::{Session, jobs::Started};
use photocraft_geom::Rect;
use serde_json::json;

#[test]
#[ignore = "live installed Codex runtime and ChatGPT plan usage; requires PHOTOCRAFT_RUN_LIVE_CODEX=1"]
fn live_codex_canvas_returns_an_undoable_layer() {
    if std::env::var("PHOTOCRAFT_RUN_LIVE_CODEX").ok().as_deref() != Some("1") {
        eprintln!("Set PHOTOCRAFT_RUN_LIVE_CODEX=1 to explicitly opt in to this subscription smoke test.");
        return;
    }
    let mut s = Session::new();
    s.execute("file.new", json!({"width":1024,"height":1024,"background":"white","name":"AI live test"})).unwrap();
    for (x, color) in [(100, "#ff0000"), (620, "#0000ff")] {
        s.execute("select.rect", json!({"x":x,"y":340,"width":300,"height":300})).unwrap();
        s.execute("edit.fill", json!({"contents":"color","color":color})).unwrap();
    }
    s.execute("select.deselect", json!({})).unwrap();
    let original = s.active().unwrap().doc.clone();
    let started = s.start("edit.ai.codex", json!({
        "prompt":"Change only the red square on the left to yellow. Preserve the blue square on the right, the white background and the composition. Keep the original square canvas aspect ratio."
    })).expect("Codex edit must start a background job");
    let result = match started {
        Started::Job(job) => s.wait_job(job).expect("Codex edit must return a real generated image"),
        Started::Done(_) => panic!("AI editing should use a background job"),
    };
    let doc = &s.active().unwrap().doc;
    assert_eq!(doc.size, original.size);
    assert_eq!(doc.layers.len(), original.layers.len() + 1);
    assert_eq!(doc.layers[0].id, original.layers[0].id);
    assert_eq!(doc.layers[0].surface().unwrap().read_region(doc.bounds()), original.layers[0].surface().unwrap().read_region(original.bounds()));
    let generated = doc.layers.last().unwrap().surface().unwrap();
    let yellow = generated.read_region(Rect::new(250, 490, 251, 491));
    let blue = generated.read_region(Rect::new(770, 490, 771, 491));
    assert!(yellow[0] > 0.6 && yellow[1] > 0.6 && yellow[2] < 0.4, "left square should be yellow: {yellow:?}");
    assert!(blue[0] < 0.4 && blue[1] < 0.4 && blue[2] > 0.6, "right square should remain blue: {blue:?}");
    let output = photocraft_io::export(doc, "png", &Default::default()).unwrap();
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/ai-codex-live-result.png");
    write_evidence(&path, &output.bytes);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.layers.len(), original.layers.len());
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.layers.len(), original.layers.len() + 1);
    println!("Live canvas -> Codex -> layer -> undo/redo passed: {result}; image: {}", path.display());
}

fn write_evidence(path: &std::path::Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn evidence_creates_missing_output_directory() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("missing/target/ai-codex-live-result.png");
    write_evidence(&path, b"synthetic evidence");
    assert_eq!(std::fs::read(path).unwrap(), b"synthetic evidence");
}
