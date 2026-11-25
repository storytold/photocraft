//! Layer Comps to Files names its files as the reference application does (#13940): the comment after the comp name,
//! `-` for spaces in PNG names only, and `.tif` / `.jpg` for TIFF and JPEG.

use super::*;

/// A one-layer document with the comps `(name, comment)` in panel order.
fn session(comps: &[(&str, &str)]) -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 16, "height": 12})).unwrap();
    for (name, comment) in comps {
        s.execute("layerComp.new", json!({"name": name, "comment": comment})).unwrap();
    }
    s
}

/// Runs Layer Comps to Files with `format` into a fresh folder of its own; the folder and the file names.
fn export(s: &mut Session, format: &str) -> (std::path::PathBuf, Vec<String>) {
    static RUN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let run = RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("photocraft-comp-names-{format}-{}-{run}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let params = json!({"dir": dir.to_string_lossy(), "prefix": "lcomps-test", "format": format});
    let r = s.execute("file.export.layerCompsToFiles", params).unwrap();
    let names =
        r["files"].as_array().unwrap().iter().map(|f| std::path::Path::new(f.as_str().unwrap()).file_name().unwrap().to_string_lossy().into_owned()).collect();
    (dir, names)
}

#[test]
fn png_names_carry_the_comment_and_use_dashes_for_spaces() {
    let mut s = session(&[("Comp A", ""), ("Comp B", "second comp"), ("Comp C", "a/b: c"), ("Comp D", "  ")]);
    let (dir, names) = export(&mut s, "png");
    assert_eq!(
        names,
        [
            "lcomps-test_0000_Comp-A.png",
            "lcomps-test_0001_Comp-B_second-comp.png",
            // The comment goes through the same replacement as the name.
            "lcomps-test_0002_Comp-C_a_b_-c.png",
            // A blank comment adds nothing.
            "lcomps-test_0003_Comp-D.png",
        ]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn other_formats_keep_spaces_and_write_tif_and_jpg() {
    for (format, ext) in [("jpg", "jpg"), ("jpeg", "jpg"), ("psd", "psd"), ("tiff", "tif"), ("bmp", "bmp")] {
        let mut s = session(&[("Comp A", ""), ("Comp B", "second comp")]);
        let (dir, names) = export(&mut s, format);
        assert_eq!(names, [format!("lcomps-test_0000_Comp A.{ext}"), format!("lcomps-test_0001_Comp B_second comp.{ext}")], "{format}");
        // The file is what its extension says.
        let path = dir.join(&names[1]);
        assert!(photocraft_io::import(&names[1], &std::fs::read(&path).unwrap()).is_ok(), "{format}: {}", path.display());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// A comment of several sentences no longer stops the export at its comp (#13940 review): the
/// comment is cut to fit, between characters, and that comp and the next keep their indices and
/// extension.
#[test]
fn a_long_comment_is_shortened_and_the_export_goes_on() {
    let long = "Hintergrund für die Nachtszene, mit gedämpftem Licht über dem Fluss. ".repeat(6);
    for (format, ext) in [("png", "png"), ("tiff", "tif")] {
        let mut s = session(&[("Comp A", &long), ("Comp B", "")]);
        let (dir, names) = export(&mut s, format);
        assert_eq!(names.len(), 2, "{format}: {names:?}");
        let full = if ext == "png" { sanitize(&long).replace(' ', "-") } else { sanitize(&long) };
        let first = names[0].strip_prefix("lcomps-test_0000_Comp").and_then(|n| n.strip_suffix(&format!(".{ext}"))).unwrap_or_default();
        let kept = first.get(3..).unwrap_or_default();
        assert!(!kept.is_empty() && full.starts_with(kept) && kept.len() < full.len(), "{format}: the comment is shortened, not dropped: {}", names[0]);
        assert!(names[0].len() <= 255, "{format}: {} bytes", names[0].len());
        assert_eq!(names[1], format!("lcomps-test_0001_Comp{}B.{ext}", if ext == "png" { "-" } else { " " }), "{format}");
        assert!(names.iter().all(|n| dir.join(n).is_file()), "{format}: both files are written");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// When the comp name has to be cut, a short comment doesn't take the bytes the cut leaves free:
/// the comment goes before the name does.
#[test]
fn a_cut_comp_name_leaves_out_the_comment() {
    let prefix = "p".repeat(243);
    let name = comp_file_name(&prefix, 0, "界", "x", "png");
    assert!(name.len() <= MAX_FILE_NAME_BYTES, "{} bytes", name.len());
    assert!(name.ends_with("_0000_.png"), "{name}");
}

/// A part that fits stays exactly as sanitized: only a cut trims the spaces it leaves.
#[test]
fn a_part_that_fits_keeps_its_trailing_space() {
    let prefix = sanitize("A .");
    assert!(comp_file_name(&prefix, 0, "Name", "x", "png").starts_with(&format!("{prefix}_0000_Name")), "{prefix:?}");
}
