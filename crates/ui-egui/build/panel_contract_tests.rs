#[path = "panel_contract.rs"]
mod panel_contract;
fn manifests() -> (String, String) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    (std::fs::read_to_string(root.join("Cargo.toml")).unwrap(), std::fs::read_to_string(root.join("vendor/craft-ui/Cargo.toml")).unwrap())
}
#[test]
fn local_adapter_matches_the_workspace_pins() {
    let (root, local) = manifests();
    assert!(panel_contract::compatible(&root, &local).is_ok());
    assert!(panel_contract::check(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")).is_ok());
}
#[test]
fn updated_framework_pins_cannot_silently_use_the_old_adapter() {
    let (root, local) = manifests();
    for revision in ["c5a1cbfb5f9049f9270955caffb5bee5997aebf7", "6d518fc799681571963842f95b55a6b68431b15a"] {
        let changed = root.replace(revision, "0000000000000000000000000000000000000001");
        assert!(panel_contract::compatible(&changed, &local).unwrap_err().contains("Reconcile"));
    }
}
#[test]
fn removing_local_override_allows_a_reviewed_upstream_library() {
    assert!(panel_contract::compatible("[workspace]", "").is_ok());
}

#[test]
fn standalone_consumers_without_a_local_override_are_unaffected() {
    let dir = tempfile::tempdir().unwrap();
    assert!(panel_contract::check(dir.path()).is_ok());
}
