//! `photocraft --version --json` lets a launcher feature-detect without opening a window: it
//! prints the `app.capabilities` object and exits 0. No display variables, so a regression that
//! falls through to the window fails fast instead of opening one.

fn run(args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_photocraft"))
        .args(args)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .output()
        .expect("failed to run photocraft binary")
}

#[test]
fn version_json_prints_capabilities() {
    let out = run(&["--version", "--json"]);
    assert!(out.status.success(), "{out:?}");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("JSON on stdout");
    assert_eq!(v["app"], "photocraft");
    assert_eq!(v["protocol"], 1);
    let features = v["features"].as_array().expect("features array");
    assert!(features.iter().any(|f| f == "events"), "{v}");
}

#[test]
fn plain_version_is_unchanged() {
    let out = run(&["--version"]);
    assert!(out.status.success(), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("photocraft "));
}
