//! Duplicates, archives, favourites and the parsed-table cache.

use std::io::Write;

use super::*;
use photocraft_cms::lutfile::{LutFile, write_cube};

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("photocraft-lutx-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn put(dir: &Path, rel: &str, text: &str) {
    let path = dir.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn go(lib: &LutLibrary, src: &Path, pack: Option<&str>) -> Result<InstallReport, String> {
    lib.install(src, pack, true, &mut |_, _| true)
}

/// A zip archive (stored or deflated entries), written by hand.
fn make_zip(entries: &[(&str, &[u8], bool)]) -> Vec<u8> {
    let (mut out, mut central) = (Vec::new(), Vec::new());
    for (name, data, deflate) in entries {
        let packed = if *deflate {
            let mut enc = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
            enc.write_all(data).unwrap();
            enc.finish().unwrap()
        } else {
            data.to_vec()
        };
        let method: u16 = if *deflate { 8 } else { 0 };
        let offset = out.len() as u32;
        let mut local = Vec::new();
        local.extend(0x0403_4b50u32.to_le_bytes());
        local.extend([20, 0, 0, 0]);
        local.extend(method.to_le_bytes());
        local.extend([0u8; 4]);
        local.extend(0u32.to_le_bytes());
        local.extend((packed.len() as u32).to_le_bytes());
        local.extend((data.len() as u32).to_le_bytes());
        local.extend((name.len() as u16).to_le_bytes());
        local.extend([0, 0]);
        local.extend(name.as_bytes());
        out.extend(&local);
        out.extend(&packed);
        central.extend(0x0201_4b50u32.to_le_bytes());
        central.extend([20, 0, 20, 0, 0, 0]);
        central.extend(method.to_le_bytes());
        central.extend([0u8; 4]);
        central.extend(0u32.to_le_bytes());
        central.extend((packed.len() as u32).to_le_bytes());
        central.extend((data.len() as u32).to_le_bytes());
        central.extend((name.len() as u16).to_le_bytes());
        central.extend([0u8; 12]);
        central.extend(offset.to_le_bytes());
        central.extend(name.as_bytes());
    }
    let cd_at = out.len() as u32;
    out.extend(&central);
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend([0u8; 4]);
    out.extend((entries.len() as u16).to_le_bytes());
    out.extend((entries.len() as u16).to_le_bytes());
    out.extend((central.len() as u32).to_le_bytes());
    out.extend(cd_at.to_le_bytes());
    out.extend([0, 0]);
    out
}

#[test]
fn installing_the_same_luts_twice_is_noticed() {
    let base = temp("dup");
    let (a, b) = (base.join("Photoshop"), base.join("All"));
    put(&a, "one.cube", &write_cube(&LutFile::identity(3)));
    put(&a, "two.cube", &write_cube(&LutFile::identity(5)));
    // The parent folder holds the same files one level down, plus one new LUT.
    put(&b, "Photoshop/one.cube", &write_cube(&LutFile::identity(3)));
    put(&b, "Photoshop/two.cube", &write_cube(&LutFile::identity(5)));
    let lib = LutLibrary::new(base.join("lib"));
    go(&lib, &a, Some("Photoshop")).unwrap();

    let e = go(&lib, &b.join("Photoshop"), Some("Again")).unwrap_err();
    assert!(e.contains("already installed") && e.contains("Photoshop/"), "{e}");
    assert_eq!(lib.list().unwrap().len(), 1, "nothing was installed");

    put(&b, "new.cube", &write_cube(&LutFile::identity(7)));
    let r = go(&lib, &b, Some("All")).unwrap();
    assert_eq!(r.installed, vec!["new.cube"], "only the new LUT is installed");
    assert_eq!(r.duplicates.len(), 2);
    assert!(r.duplicates[0].1.starts_with("Photoshop/"));

    let r = lib
        .install_with(&b.join("Photoshop"), &InstallOptions { pack: Some("Copy".into()), allow_duplicates: true, ..InstallOptions::default() }, &mut |_, _| {
            true
        })
        .unwrap();
    assert_eq!(r.installed.len(), 2, "duplicates can be installed on request");
}

#[test]
fn a_pack_contained_in_another_is_flagged_but_never_removed() {
    let base = temp("redundant");
    let (a, b, c) = (base.join("a"), base.join("b"), base.join("c"));
    put(&a, "one.cube", &write_cube(&LutFile::identity(3)));
    put(&a, "two.cube", &write_cube(&LutFile::identity(5)));
    // The same files one folder down: the parent folder is the duplicate.
    put(&b, "Photoshop/one.cube", &write_cube(&LutFile::identity(3)));
    put(&b, "Photoshop/two.cube", &write_cube(&LutFile::identity(5)));
    put(&c, "three.cube", &write_cube(&LutFile::identity(7)));
    let lib = LutLibrary::new(base.join("lib"));
    for (src, name) in [(&a, "Photoshop"), (&c, "Other")] {
        go(&lib, src, Some(name)).unwrap();
    }
    lib.install_with(&b, &InstallOptions { pack: Some("LUTs".into()), allow_duplicates: true, ..InstallOptions::default() }, &mut |_, _| true).unwrap();
    let r = lib.redundant_packs();
    assert_eq!(r.len(), 1, "{r:?}");
    assert_eq!(r.get("LUTs").map(String::as_str), Some("Photoshop"));
    assert_eq!(lib.list().unwrap().len(), 3, "flagging deletes nothing");
}

#[test]
fn a_pack_installed_without_hashes_is_hashed_on_demand_and_reinstalling_a_pack_is_not_a_duplicate() {
    let src = temp("nohash-src");
    put(&src, "a.cube", &write_cube(&LutFile::identity(3)));
    let lib = LutLibrary::new(temp("nohash-lib"));
    go(&lib, &src, Some("p")).unwrap();
    fs::remove_file(lib.root().join("p").join(".hashes")).unwrap();
    assert!(go(&lib, &src, Some("q")).unwrap_err().contains("already installed"));
    assert!(lib.root().join("p").join(".hashes").is_file(), "the missing sidecar was rebuilt");
    // Replacing a pack with itself compares against the other packs only.
    assert!(go(&lib, &src, Some("p")).unwrap().replaced);
}

#[test]
fn a_zip_installs_like_a_folder_and_hostile_entries_stay_out() {
    let cube = write_cube(&LutFile::identity(3));
    let zip = make_zip(&[
        ("Looks/Warm.cube", cube.as_bytes(), true),
        ("Looks/Sub/Cool.cube", write_cube(&LutFile::identity(5)).as_bytes(), false),
        ("../../escape.cube", cube.as_bytes(), false),
        ("/abs/evil.cube", cube.as_bytes(), false),
        ("C:/evil.cube", cube.as_bytes(), false),
        ("__MACOSX/Looks/._Warm.cube", cube.as_bytes(), false),
        ("readme.txt", b"hello", false),
    ]);
    let base = temp("zip");
    fs::write(base.join("My Pack.zip"), zip).unwrap();
    let lib = LutLibrary::new(base.join("lib"));
    let r = go(&lib, &base.join("My Pack.zip"), None).unwrap();
    assert_eq!(r.pack, "My Pack");
    assert_eq!(r.installed, vec!["Looks/Sub/Cool.cube", "Looks/Warm.cube"]);
    assert!(!base.join("escape.cube").exists() && !lib.root().parent().unwrap().join("escape.cube").exists());
}

#[test]
fn bad_archives_are_errors() {
    let base = temp("badzip");
    fs::write(base.join("junk.zip"), b"this is not a zip file at all, just text").unwrap();
    fs::write(base.join("empty.zip"), make_zip(&[("readme.txt", b"x", false)])).unwrap();
    let lib = LutLibrary::new(base.join("lib"));
    assert!(go(&lib, &base.join("junk.zip"), None).unwrap_err().contains("not a zip"));
    assert!(go(&lib, &base.join("empty.zip"), None).unwrap_err().contains("no .cube"));
}

#[test]
fn a_zip_larger_than_the_remaining_budget_is_refused_before_it_is_unpacked() {
    let (a, b) = (write_cube(&LutFile::identity(5)), write_cube(&LutFile::identity(6)));
    let each = a.len().max(b.len()) as u64;
    let base = temp("zipbudget");
    fs::write(base.join("Big.zip"), make_zip(&[("a.cube", a.as_bytes(), false), ("b.cube", b.as_bytes(), true)])).unwrap();

    // The extractor stops at the first file that does not fit and writes nothing for it.
    let out = base.join("out");
    let err = zip::extract(&base.join("Big.zip"), &out, each / 2, 1 << 30, &mut |_, _| true).unwrap_err();
    assert!(err.contains("the LUT library would grow past 1 GiB; remove a pack first"), "{err}");
    assert_eq!(collect(&out).map(|f| f.len()).unwrap_or(0), 0);

    // Through install: refused with the library untouched and nothing left in the library root.
    let lib = LutLibrary::new(base.join("lib"));
    let opts = InstallOptions::default();
    let err = lib.install_capped(&base.join("Big.zip"), &opts, each, &mut |_, _| true).unwrap_err();
    assert!(err.contains("would grow past"), "{err}");
    assert!(lib.list().unwrap().is_empty());

    // Room for both: installs.
    let ok = lib.install_capped(&base.join("Big.zip"), &opts, a.len() as u64 + b.len() as u64, &mut |_, _| true).unwrap();
    assert_eq!(ok.installed.len(), 2);
    // Replacing the pack gets its own size back, so the same archive still fits at the same cap.
    let again = lib.install_capped(&base.join("Big.zip"), &opts, a.len() as u64 + b.len() as u64, &mut |_, _| true).unwrap();
    assert!(again.replaced);
    assert_eq!(again.installed.len(), 2);
}

#[test]
fn favourites_and_recent_survive_a_restart_and_a_removed_pack() {
    let src = temp("fav-src");
    put(&src, "a.cube", &write_cube(&LutFile::identity(3)));
    put(&src, "Sub/b.cube", &write_cube(&LutFile::identity(5)));
    let root = temp("fav-lib");
    let mut lib = LutLibrary::new(&root);
    go(&lib, &src, Some("p")).unwrap();
    lib.set_favorite("p/a.cube", true).unwrap();
    lib.set_favorite("p/Sub/b.cube", true).unwrap();
    lib.note_used("p/a.cube").unwrap();
    lib.note_used("p/Sub/b.cube").unwrap();
    lib.note_used("p/a.cube").unwrap();
    assert_eq!(lib.recent(), ["p/a.cube", "p/Sub/b.cube"]);
    assert!(lib.is_favorite("p/a.cube"));
    for bad in ["", "a.cube", "p/", "p/missing.cube", "../x/a.cube", "p/../a.cube", "p//a.cube", ".state/x.cube", "p/a.txt", "p\\a.cube"] {
        assert!(lib.set_favorite(bad, true).is_err(), "{bad:?}");
        assert!(lib.note_used(bad).is_err(), "{bad:?}");
    }
    let again = LutLibrary::new(&root);
    assert_eq!(again.favorites(), ["p/a.cube", "p/Sub/b.cube"]);
    assert_eq!(again.recent(), ["p/a.cube", "p/Sub/b.cube"]);
    lib.set_favorite("p/a.cube", false).unwrap();
    assert!(!lib.is_favorite("p/a.cube"));
    lib.remove_pack("p").unwrap();
    assert!(lib.favorites().is_empty() && lib.recent().is_empty(), "a removed pack leaves no stars behind");
    // A corrupt state file is an empty state.
    fs::write(root.join(".state.json"), "{ nope").unwrap();
    assert!(LutLibrary::new(&root).favorites().is_empty());
}

#[test]
fn recent_is_capped_and_hand_deleted_files_are_pruned() {
    let src = temp("cap-src");
    for i in 0..(state::MAX_RECENT + 4) {
        put(&src, &format!("l{i:02}.cube"), &write_cube(&LutFile::identity(2 + (i % 3))));
    }
    let mut lib = LutLibrary::new(temp("cap-lib"));
    // Identical content installs once, so give every file its own title.
    for i in 0..(state::MAX_RECENT + 4) {
        put(&src, &format!("l{i:02}.cube"), &format!("TITLE \"t{i}\"\n{}", write_cube(&LutFile::identity(3))));
    }
    go(&lib, &src, Some("p")).unwrap();
    for i in 0..(state::MAX_RECENT + 4) {
        lib.note_used(&format!("p/l{i:02}.cube")).unwrap();
    }
    assert_eq!(lib.recent().len(), state::MAX_RECENT);
    fs::remove_file(lib.path_of("p", "l15.cube")).unwrap();
    lib.prune_state();
    assert!(!lib.recent().iter().any(|r| r == "p/l15.cube"));
}
