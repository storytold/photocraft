use photocraft_engine::Session;
use serde_json::json;

#[test]
/// Verify an unmatched query is read-only and malformed query parameters are rejected.
fn camera_profile_inventory_is_a_read_only_query_for_any_model() {
    let mut s = Session::new();
    let r = s.execute("raw.profiles", json!({"model":"nonexistent-synthetic-test-body-7744"})).unwrap();
    assert_eq!(r["count"], 0);
    assert_eq!(r["rawDecoderSupportIsSeparate"], true);
    assert!(s.active().is_none());
    assert!(s.journal.is_empty());
    for bad in [json!({"model":1}), json!({"refresh":"yes"}), json!({"extra":true}), json!([])] {
        assert!(s.execute("raw.profiles", bad).is_err());
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
/// Keep native inventory at nine distinct CC0 models, excluding the unapproved Nikon Z f asset.
fn native_inventory_exposes_exact_bundled_coverage_and_license() {
    let mut session = Session::new();
    let result = session.execute("raw.profiles", json!({})).unwrap();
    assert_eq!(result["bundledCount"], 9);
    let bundled: Vec<_> = result["profiles"].as_array().unwrap().iter().filter(|p| p["source"] == "bundled-dcp").collect();
    assert_eq!(bundled.len(), 9);
    assert_eq!(bundled.iter().map(|p| p["cameraModel"].as_str().unwrap()).collect::<std::collections::BTreeSet<_>>().len(), 9);
    assert!(bundled.iter().all(|p| p["embedPolicy"] == 3 && matches!(p["license"].as_str(), Some("CC0-1.0"))));
    assert!(!bundled.iter().any(|p| p["cameraModel"].as_str().unwrap().eq_ignore_ascii_case("Nikon Z f")));
    assert!(session.active().is_none());
    assert!(session.journal.is_empty());
}
