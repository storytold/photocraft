//! Package into a folder that already holds the package (#14075): an error naming the folder
//! without `overwrite`, writing nothing; with it the old package folder is replaced as a whole,
//! and nothing next to it is touched.

use super::*;

fn tmp(tag: &str) -> String {
    let d = std::env::temp_dir().join(format!("photocraft-pkg-overwrite-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.to_string_lossy().replace('\\', "/")
}

/// A saved `pkg-test.pcraft` in `<dir>/src` with two linked PNGs, `link-red.png` and
/// `link-blue.png`, placed in that order.
fn document(dir: &str) -> Session {
    let src = format!("{dir}/src");
    std::fs::create_dir_all(&src).unwrap();
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 200, "height": 150, "name": "pkg-test.pcraft", "background": "#ffffff"})).unwrap();
    for (name, colour) in [("link-red.png", "#ff0000"), ("link-blue.png", "#0000ff")] {
        let link = format!("{src}/{name}");
        let mut l = crate::session();
        l.execute("file.new", json!({"width": 40, "height": 30, "background": colour})).unwrap();
        crate::file_cmds::save_doc(&l.active().unwrap().doc, &link, None).unwrap();
        s.execute("file.placeLinked", json!({"path": link})).unwrap();
    }
    let path = format!("{src}/pkg-test.pcraft");
    crate::file_cmds::save_doc(&s.active().unwrap().doc, &path, None).unwrap();
    s.active_mut().unwrap().path = Some(path);
    s
}

/// Every file and folder under `root`, relative, sorted; files with their bytes.
fn listing(root: &str) -> Vec<String> {
    fn walk(base: &std::path::Path, at: &std::path::Path, out: &mut Vec<String>) {
        for e in std::fs::read_dir(at).unwrap().flatten() {
            let path = e.path();
            let rel = path.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/");
            if path.is_dir() {
                out.push(format!("{rel}/"));
                walk(base, &path, out);
            } else {
                out.push(format!("{rel} = {:?}", std::fs::read(&path).unwrap().len()));
            }
        }
    }
    let mut v = Vec::new();
    walk(std::path::Path::new(root), std::path::Path::new(root), &mut v);
    v.sort();
    v
}

fn names(root: &str) -> Vec<String> {
    listing(root).into_iter().map(|l| l.split(" = ").next().unwrap().to_string()).collect()
}

/// `dest` with `sibling.txt`, packaged once, then `marker.txt` and `Links/stale.png` added to
/// the package as leftovers.
fn packaged_once(tag: &str) -> (String, String, Session) {
    let dir = tmp(tag);
    let mut s = document(&dir);
    let dest = format!("{dir}/dest");
    std::fs::create_dir_all(&dest).unwrap();
    std::fs::write(format!("{dest}/sibling.txt"), b"keep me").unwrap();
    let r = s.execute("file.package", json!({"dir": dest})).unwrap();
    let folder = r["folder"].as_str().unwrap().to_string();
    std::fs::write(format!("{folder}/marker.txt"), b"old").unwrap();
    std::fs::write(format!("{folder}/Links/stale.png"), b"old").unwrap();
    (dir, dest, s)
}

#[test]
fn an_existing_package_folder_without_overwrite_is_an_error_naming_it_and_writes_nothing() {
    let (_dir, dest, mut s) = packaged_once("refuse");
    let before = listing(&dest);
    let rev = s.active().unwrap().revision;
    for params in [json!({"dir": dest}), json!({"dir": dest, "overwrite": false}), json!({"dir": dest, "overwrite": null})] {
        let err = s.execute("file.package", params.clone()).unwrap_err().to_string();
        assert!(err.contains(&format!("{dest}/pkg-test")) && err.contains("already exists") && err.contains("overwrite"), "{params}: {err}");
        assert_eq!(listing(&dest), before, "{params}: nothing is written");
    }
    assert_eq!(s.active().unwrap().revision, rev, "the document is unchanged");
}

#[test]
fn overwrite_leaves_exactly_the_new_package_with_plain_link_names() {
    let (_dir, dest, mut s) = packaged_once("replace");
    let r = s.execute("file.package", json!({"dir": dest, "overwrite": true})).unwrap();
    assert_eq!(
        names(&dest),
        ["pkg-test/", "pkg-test/Links/", "pkg-test/Links/link-blue.png", "pkg-test/Links/link-red.png", "pkg-test/pkg-test.pcraft", "sibling.txt"],
        "no leftovers, no numbered copies, the sibling kept"
    );
    let tails: Vec<&str> = r["links"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().rsplit('/').next().unwrap()).collect();
    assert_eq!(tails, ["link-red.png", "link-blue.png"]);
    assert_eq!(std::fs::read(format!("{dest}/sibling.txt")).unwrap(), b"keep me");
    // The packaged document points at the plain-named copies.
    let doc = photocraft_format::load_path(std::path::Path::new(r["document"].as_str().unwrap())).unwrap();
    let mut linked: Vec<String> = doc
        .walk()
        .iter()
        .filter_map(|(_, _, l)| match &l.content {
            LayerContent::Smart(so) => match &so.source {
                SmartSource::Linked { path } => Some(path.rsplit('/').next().unwrap().to_string()),
                _ => None,
            },
            _ => None,
        })
        .collect();
    linked.sort();
    assert_eq!(linked, ["link-blue.png", "link-red.png"]);
}

#[test]
fn overwrite_without_an_existing_folder_packages_as_usual() {
    let dir = tmp("fresh");
    let mut s = document(&dir);
    let dest = format!("{dir}/dest");
    s.execute("file.package", json!({"dir": dest, "overwrite": true})).unwrap();
    assert_eq!(names(&dest), ["pkg-test/", "pkg-test/Links/", "pkg-test/Links/link-blue.png", "pkg-test/Links/link-red.png", "pkg-test/pkg-test.pcraft"]);
}

#[test]
fn a_file_in_place_of_the_package_folder_is_refused_and_kept() {
    let dir = tmp("file");
    let mut s = document(&dir);
    let dest = format!("{dir}/dest");
    std::fs::create_dir_all(&dest).unwrap();
    std::fs::write(format!("{dest}/pkg-test"), b"not a folder").unwrap();
    let rev = s.active().unwrap().revision;
    for overwrite in [false, true] {
        let err = s.execute("file.package", json!({"dir": dest, "overwrite": overwrite})).unwrap_err().to_string();
        assert!(err.contains(&format!("{dest}/pkg-test")) && err.contains("a file"), "{err}");
        assert_eq!(std::fs::read(format!("{dest}/pkg-test")).unwrap(), b"not a folder");
        assert_eq!(names(&dest), ["pkg-test"]);
    }
    assert_eq!(s.active().unwrap().revision, rev);
    assert_eq!(s.documents().len(), 1, "the document stays open");
}

#[cfg(unix)]
#[test]
fn a_symbolic_link_in_place_of_the_package_folder_is_refused_and_its_target_kept() {
    let dir = tmp("symlink");
    let mut s = document(&dir);
    let dest = format!("{dir}/dest");
    let elsewhere = format!("{dir}/elsewhere");
    std::fs::create_dir_all(&dest).unwrap();
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::write(format!("{elsewhere}/precious.txt"), b"keep").unwrap();
    std::os::unix::fs::symlink(&elsewhere, format!("{dest}/pkg-test")).unwrap();
    let err = s.execute("file.package", json!({"dir": dest, "overwrite": true})).unwrap_err().to_string();
    assert!(err.contains("symbolic link"), "{err}");
    assert_eq!(std::fs::read(format!("{elsewhere}/precious.txt")).unwrap(), b"keep");
    assert!(std::fs::symlink_metadata(format!("{dest}/pkg-test")).unwrap().file_type().is_symlink());
}

#[test]
fn a_wrong_type_overwrite_is_an_error_and_writes_nothing() {
    let (_dir, dest, mut s) = packaged_once("badparam");
    let before = listing(&dest);
    for v in [json!("yes"), json!(1), json!([true]), json!({"x": 1})] {
        let err = s.execute("file.package", json!({"dir": dest, "overwrite": v})).unwrap_err().to_string();
        assert!(err.contains("overwrite"), "{v}: {err}");
        assert_eq!(listing(&dest), before, "{v}");
    }
}
