use super::*;
use photocraft_cms::lutfile::{LutFile, write_cube};

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("photocraft-lutlib-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn cube(size: usize) -> String {
    write_cube(&LutFile::identity(size))
}

fn put(dir: &Path, rel: &str, text: &str) {
    let path = dir.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn go(lib: &LutLibrary, src: &Path, pack: Option<&str>, replace: bool) -> Result<InstallReport, String> {
    lib.install(src, pack, replace, &mut |_, _| true)
}

#[test]
fn clean_component_keeps_names_portable() {
    assert_eq!(clean_component("Kodak 2383 (D55)"), "Kodak 2383 (D55)");
    assert_eq!(clean_component("a/b\\c:d*e?"), "a_b_c_d_e_");
    assert_eq!(clean_component("..\\..\\evil"), "_.._evil");
    assert_eq!(clean_component(".hidden"), "hidden");
    assert_eq!(clean_component("  trailing. "), "trailing");
    assert_eq!(clean_component(""), "_");
    assert_eq!(clean_component("..."), "_");
    assert_eq!(clean_component("con"), "_con");
    assert_eq!(clean_component("LPT1.cube"), "_LPT1.cube");
    assert_eq!(clean_component(&"x".repeat(500)).chars().count(), MAX_NAME_CHARS);
    assert_eq!(clean_component("Café Noir"), "Café Noir");
}

#[test]
fn installs_valid_luts_and_reports_the_rest() {
    let src = temp("install-src");
    put(&src, "Warm 1.cube", &cube(5));
    put(&src, "COOL.CUBE", &cube(3));
    put(&src, "broken.cube", "this is not a LUT");
    put(&src, "notes.txt", "ignored");
    let lib = LutLibrary::new(temp("install-lib"));
    let r = go(&lib, &src, Some("My Pack"), true).unwrap();
    assert_eq!(r.pack, "My Pack");
    assert!(!r.replaced);
    assert_eq!(r.installed, vec!["COOL.cube", "Warm 1.cube"]);
    assert_eq!(r.skipped.len(), 1);
    assert!(r.skipped[0].0.contains("broken.cube"));
    let packs = lib.list().unwrap();
    assert_eq!(packs.len(), 1);
    assert_eq!(packs[0].name, "My Pack");
    let names: Vec<_> = packs[0].luts.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["COOL", "Warm 1"]);
    assert!(packs[0].bytes() > 0);
    // The installed copy is a LUT Color Lookup can read.
    let path = lib.path_of("My Pack", "Warm 1.cube");
    assert!(photocraft_cms::lutfile::parse("Warm 1.cube", &fs::read(path).unwrap()).is_ok());
    // No staging directory is left behind.
    assert!(fs::read_dir(lib.root()).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().starts_with('.')));
}

#[test]
fn default_pack_name_is_the_folder_name_and_subfolders_are_kept() {
    let base = temp("subdirs");
    let src = base.join("Film Stocks");
    put(&src, "Kodak/Portra 400.cube", &cube(4));
    put(&src, "Fuji/Velvia.3dl", "");
    put(&src, "Fuji/Provia.cube", &cube(4));
    let lib = LutLibrary::new(base.join("lib"));
    let r = go(&lib, &src, None, true).unwrap();
    assert_eq!(r.pack, "Film Stocks");
    assert_eq!(r.installed, vec!["Fuji/Provia.cube", "Kodak/Portra 400.cube"]);
    assert_eq!(r.skipped.len(), 1, "the empty .3dl is not a LUT");
    let files: Vec<_> = lib.list().unwrap()[0].luts.iter().map(|l| l.file.clone()).collect();
    assert_eq!(files, r.installed);
}

#[test]
fn colliding_names_are_kept_apart() {
    let src = temp("collide-src");
    put(&src, "a#b.cube", &cube(3));
    put(&src, "a&b.cube", &cube(3));
    let lib = LutLibrary::new(temp("collide-lib"));
    let r = go(&lib, &src, Some("p"), true).unwrap();
    assert_eq!(r.installed.len(), 2);
    assert_ne!(r.installed[0].to_lowercase(), r.installed[1].to_lowercase());
}

#[test]
fn reinstalling_replaces_and_a_failed_install_keeps_the_old_pack() {
    let src = temp("replace-src");
    put(&src, "one.cube", &cube(3));
    let lib = LutLibrary::new(temp("replace-lib"));
    go(&lib, &src, Some("p"), true).unwrap();
    assert!(go(&lib, &src, Some("p"), false).unwrap_err().contains("already installed"));

    put(&src, "two.cube", &cube(3));
    let r = go(&lib, &src, Some("p"), true).unwrap();
    assert!(r.replaced);
    assert_eq!(lib.list().unwrap()[0].luts.len(), 2);

    let bad = temp("replace-bad");
    put(&bad, "x.cube", "nope");
    let e = go(&lib, &bad, Some("p"), true).unwrap_err();
    assert!(e.contains("none of the 1 files"), "{e}");
    assert_eq!(lib.list().unwrap()[0].luts.len(), 2, "the installed pack survives");
    assert!(fs::read_dir(lib.root()).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().starts_with('.')));
}

#[test]
fn cancelling_leaves_the_library_untouched() {
    let src = temp("cancel-src");
    put(&src, "a.cube", &cube(3));
    put(&src, "b.cube", &cube(3));
    let lib = LutLibrary::new(temp("cancel-lib"));
    let mut calls = 0;
    let e = lib
        .install(&src, Some("p"), true, &mut |_, _| {
            calls += 1;
            calls < 2
        })
        .unwrap_err();
    assert_eq!(e, "cancelled");
    assert!(lib.list().unwrap().is_empty());
    assert!(fs::read_dir(lib.root()).unwrap().flatten().next().is_none(), "staging is cleaned up");
}

#[test]
fn bad_sources_are_errors() {
    let lib = LutLibrary::new(temp("bad-lib"));
    let empty = temp("bad-empty");
    assert!(go(&lib, &empty, None, true).unwrap_err().contains("no .cube"));
    assert!(go(&lib, &empty.join("missing"), None, true).is_err());
    let file = empty.join("f.cube");
    fs::write(&file, cube(3)).unwrap();
    assert!(go(&lib, &file, None, true).unwrap_err().contains("not a folder"));
    // The library cannot be installed into itself.
    put(lib.root(), "p/a.cube", &cube(3));
    assert!(go(&lib, &lib.root().join("p"), None, true).unwrap_err().contains("inside the LUT library"));
}

#[test]
fn depth_and_oversize_are_bounded() {
    let src = temp("bounds-src");
    let deep = (0..=MAX_DEPTH + 1).map(|i| format!("d{i}")).collect::<Vec<_>>().join("/");
    put(&src, &format!("{deep}/deep.cube"), &cube(3));
    put(&src, "ok.cube", &cube(3));
    let lib = LutLibrary::new(temp("bounds-lib"));
    let r = go(&lib, &src, Some("p"), true).unwrap();
    assert_eq!(r.installed, vec!["ok.cube"], "files below the depth limit are not searched");
}

#[test]
fn remove_pack_is_exact_and_stays_inside_the_library() {
    let src = temp("remove-src");
    put(&src, "a.cube", &cube(3));
    let mut lib = LutLibrary::new(temp("remove-lib"));
    go(&lib, &src, Some("p"), true).unwrap();
    let other = temp("remove-src-2");
    put(&other, "b.cube", &cube(5));
    go(&lib, &other, Some("q"), true).unwrap();
    for bad in ["", "..", "../p", "p/..", "p/a.cube", ".hidden", "missing", "P*"] {
        assert!(lib.remove_pack(bad).is_err(), "{bad:?}");
    }
    assert_eq!(lib.list().unwrap().len(), 2);
    let rev = lib.rev;
    lib.remove_pack("p").unwrap();
    assert_eq!(lib.rev, rev + 1);
    assert_eq!(lib.list().unwrap().iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["q"]);
}

#[test]
fn a_missing_library_is_empty() {
    let lib = LutLibrary::new(std::env::temp_dir().join(format!("photocraft-lutlib-{}-nothing-here", std::process::id())));
    assert!(lib.list().unwrap().is_empty());
}

#[cfg(unix)]
#[test]
fn symbolic_links_are_not_followed() {
    let base = temp("symlink");
    let src = base.join("src");
    put(&src, "real.cube", &cube(3));
    put(&base, "outside/secret.cube", &cube(3));
    std::os::unix::fs::symlink(base.join("outside"), src.join("link")).unwrap();
    std::os::unix::fs::symlink(base.join("outside/secret.cube"), src.join("alias.cube")).unwrap();
    let lib = LutLibrary::new(base.join("lib"));
    let r = go(&lib, &src, Some("p"), true).unwrap();
    assert_eq!(r.installed, vec!["real.cube"]);
}
