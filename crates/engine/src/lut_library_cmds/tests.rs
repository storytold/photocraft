use super::*;

fn temp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("photocraft-lutcmds-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn session(lib: Option<&std::path::Path>) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    s.lut_library = lib.map(LutLibrary::new);
    s
}

fn write_pack(dir: &std::path::Path) {
    let cube = photocraft_cms::lutfile::write_cube(&photocraft_cms::lutfile::LutFile::identity(5));
    std::fs::write(dir.join("Identity.cube"), cube).unwrap();
    std::fs::write(dir.join("Junk.cube"), "not a LUT").unwrap();
}

#[test]
fn installed_luts_are_listed_and_usable_by_color_lookup() {
    let src = temp("use-src");
    write_pack(&src);
    let mut s = session(Some(&temp("use-lib")));
    let r = s.execute("lut.installPack", json!({"path": src.to_string_lossy(), "name": "Test Pack"})).unwrap();
    assert_eq!(r["pack"], "Test Pack");
    assert_eq!(r["count"], 1);
    assert_eq!(r["skipped"][0]["file"], "Junk.cube");
    assert_eq!(r["replaced"], false);

    let lib = s.execute("lut.library", json!({})).unwrap();
    assert_eq!(lib["available"], true);
    assert_eq!(lib["rev"], 1);
    assert_eq!(lib["packs"][0]["name"], "Test Pack");
    assert_eq!(lib["packs"][0]["luts"][0]["name"], "Identity");
    let path = lib["packs"][0]["luts"][0]["location"].as_str().unwrap().to_string();
    // The listed location is what Color Lookup loads.
    s.execute("layer.newAdjustmentLayer.colorLookup", json!({"file": path})).unwrap();
    assert_eq!(s.execute("lut.library", json!({"pack": "Other"})).unwrap()["packs"].as_array().unwrap().len(), 0);

    s.execute("lut.removePack", json!({"name": "Test Pack"})).unwrap();
    let lib = s.execute("lut.library", json!({})).unwrap();
    assert_eq!(lib["rev"], 2);
    assert!(lib["packs"].as_array().unwrap().is_empty());
}

#[test]
fn without_a_library_listing_is_empty_and_changes_fail() {
    let mut s = session(None);
    let lib = s.execute("lut.library", json!({})).unwrap();
    assert_eq!(lib, json!({"available": false, "packs": [], "favorites": [], "recent": []}));
    assert!(s.execute("lut.installPack", json!({"path": "."})).is_err());
    assert!(s.execute("lut.removePack", json!({"name": "x"})).is_err());
    assert!(!s.is_enabled("lut.installPack"));
}

#[test]
fn bad_params_are_errors_not_panics() {
    let mut s = session(Some(&temp("bad-lib")));
    for (id, p) in [
        ("lut.installPack", json!({})),
        ("lut.installPack", json!({"path": 5})),
        ("lut.installPack", json!({"path": ""})),
        ("lut.installPack", json!({"path": "/no/such/folder/anywhere"})),
        ("lut.installPack", json!({"path": ".", "name": [1]})),
        ("lut.installPack", json!({"path": ".", "replace": "yes"})),
        ("lut.removePack", json!({})),
        ("lut.removePack", json!({"name": 3})),
        ("lut.removePack", json!({"name": "../.."})),
        ("lut.library", json!({"pack": 7})),
    ] {
        assert!(s.execute(id, p.clone()).is_err(), "{id} {p}");
    }
}
