//! Agents apply an installed LUT by its library id: `file` (a path) is refused for them, `lut` is not,
//! and the id only ever names a LUT the library lists.

use photocraft_automation::Headless;
use photocraft_engine::lut_library::LutLibrary;
use serde_json::json;

/// A 2-point identity table, written out so the test needs no colour crate.
const IDENTITY_CUBE: &str = "TITLE \"id\"
LUT_3D_SIZE 2
0 0 0
1 0 0
0 1 0
1 1 0
0 0 1
1 0 1
0 1 1
1 1 1
";

fn headless_with_library(name: &str) -> (Headless, std::path::PathBuf) {
    let base = std::env::temp_dir().join(format!("pc-lut-ids-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("lib").join("Pack")).unwrap();
    std::fs::write(base.join("lib").join("Pack").join("Identity.cube"), IDENTITY_CUBE).unwrap();
    std::fs::write(base.join("outside.cube"), IDENTITY_CUBE).unwrap();
    let mut headless = Headless::new();
    headless.session.lut_library = Some(LutLibrary::new(base.join("lib")));
    headless.command_run("file.new", json!({"width": 8, "height": 8})).unwrap();
    (headless, base)
}

#[test]
fn file_outside_the_workspace_is_refused_but_a_library_id_is_applied() {
    let (mut headless, base) = headless_with_library("authorize");
    let location = base.join("lib").join("Pack").join("Identity.cube").to_string_lossy().into_owned();
    let outside = base.join("outside.cube").to_string_lossy().into_owned();

    // On the Background pixel layer, before any adjustment layer is active.
    headless.command_run("image.adjustments.colorLookup", json!({"lut": "Pack/Identity.cube"})).unwrap();

    for path in [&location, &outside] {
        assert!(headless.command_run("layer.newAdjustmentLayer.colorLookup", json!({"file": path})).is_err(), "file {path}");
    }
    headless.command_run("layer.newAdjustmentLayer.colorLookup", json!({"lut": "none"})).unwrap();
    assert!(headless.command_run("layer.setAdjustment", json!({"file": location})).is_err());
    assert!(headless.command_run("image.adjustments.colorLookup", json!({"file": outside})).is_err());

    headless.command_run("layer.newAdjustmentLayer.colorLookup", json!({"lut": "Pack/Identity.cube"})).unwrap();
    headless.command_run("layer.setAdjustment", json!({"lut": "none"})).unwrap();
    headless.command_run("layer.setAdjustment", json!({"lut": "Pack/Identity.cube"})).unwrap();
}

#[test]
fn ids_that_try_to_leave_the_library_are_refused_for_agents_too() {
    let (mut headless, _) = headless_with_library("hostile");
    for id in ["../outside.cube", "Pack/../../outside.cube", "/etc/x.cube", "C:\\x.cube", "Pack\\Identity.cube"] {
        let e = headless.command_run("layer.newAdjustmentLayer.colorLookup", json!({"lut": id})).unwrap_err().to_string();
        assert!(e.contains("unknown look"), "{id}: {e}");
    }
}

#[test]
fn the_library_listing_gives_ids_that_apply() {
    let (mut headless, _) = headless_with_library("listing");
    let listing = headless.command_run("lut.library", json!({})).unwrap();
    let id = listing["packs"][0]["luts"][0]["id"].as_str().unwrap().to_string();
    headless.command_run("layer.newAdjustmentLayer.colorLookup", json!({"lut": id})).unwrap();
}
