//! Tests that run paint commands, moved from the engine-core `param_check` tests: 1 test(s).

use serde_json::json;

use photocraft_engine::Session;

#[test]
fn a_misspelled_key_fails_through_dispatch_instead_of_running_on_defaults() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    let err = s.execute("paint.gradient", json!({"from": [0, 0], "to": [7, 7], "colour": "red"})).unwrap_err().to_string();
    assert!(err.contains("unknown parameter `colour`") && err.contains("paint.gradient"), "{err}");
    let err = s.precheck("filter.blur.gaussianBlur", &json!({"radus": 3})).unwrap_err().to_string();
    assert!(err.contains("unknown parameter `radus`") && err.contains("expected: radius"), "{err}");
}
