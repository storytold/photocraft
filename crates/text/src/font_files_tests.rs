//! Registering font files and folders from disk (#3393). The fonts are renamed copies of the
//! bundled Inter, each test with its own family names.
use super::*;
use std::path::{Path, PathBuf};

fn font(family: &str) -> Vec<u8> {
    photocraft_testkit::fonts::renamed_font(family).unwrap()
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("photocraft-font-files-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

const DEFAULT: FontFileLimits = FontFileLimits::DEFAULT;

#[test]
fn registers_a_file_a_nested_folder_and_a_collection() {
    let dir = temp_dir("folder");
    write(&dir.join("Alpha.ttf"), &font("PC3393 Folder Alpha"));
    write(&dir.join("sub/deeper/BETA.TTF"), &font("PC3393 Folder Beta"));
    let (gamma, delta) = (font("PC3393 Folder Gamma"), font("PC3393 Folder Delta"));
    write(&dir.join("sub/pair.otc"), &crate::tests::make_ttc(&[gamma.as_slice(), delta.as_slice()]));
    // Not a font extension: a folder scan doesn't pick it up.
    write(&dir.join("readme.txt"), b"brand fonts");

    let mut db = FontDb::new();
    let r = db.register_font_path(&dir, DEFAULT).unwrap();
    assert_eq!(r.families, ["PC3393 Folder Alpha", "PC3393 Folder Beta", "PC3393 Folder Delta", "PC3393 Folder Gamma"]);
    assert_eq!((r.files, r.skipped.len()), (3, 0), "{r:?}");
    let listed = db.families();
    assert!(r.families.iter().all(|f| listed.contains(f)), "{listed:?}");
    assert_eq!(db.faces("PC3393 Folder Beta").len(), 1);

    // A single file is judged by its data, whatever its name.
    let single = dir.join("other/epsilon.font");
    write(&single, &font("PC3393 Folder Epsilon"));
    let r = db.register_font_path(&single, DEFAULT).unwrap();
    assert_eq!((r.families, r.files), (vec!["PC3393 Folder Epsilon".to_string()], 1));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn missing_paths_non_fonts_and_empty_folders_are_errors() {
    let dir = temp_dir("errors");
    let mut db = FontDb::new();
    let before = db.families();

    let missing = dir.join("missing");
    assert_eq!(db.register_font_path(&missing, DEFAULT), Err(FontFileError::NotFound(missing.display().to_string())));

    let reason = |db: &mut FontDb, path: &Path| match db.register_font_path(path, DEFAULT) {
        Err(FontFileError::NoFonts { failures, .. }) if failures.len() == 1 => failures[0].1.clone(),
        other => panic!("{}: {other:?}", path.display()),
    };
    write(&dir.join("notes.txt"), b"not a font");
    assert!(reason(&mut db, &dir.join("notes.txt")).starts_with("not a font file"));
    // A font header with nothing usable behind it.
    write(&dir.join("broken.ttf"), b"\x00\x01\x00\x00 truncated");
    assert_eq!(reason(&mut db, &dir.join("broken.ttf")), NO_FACES);
    write(&dir.join("web.woff2"), b"wOF2\x00\x01\x00\x00 rest");
    assert!(reason(&mut db, &dir.join("web.woff2")).contains("WOFF"));
    write(&dir.join("empty.ttf"), b"");
    assert!(reason(&mut db, &dir.join("empty.ttf")).starts_with("not a font file"));
    // A folder of only broken font files: each one named, relative to the folder.
    write(&dir.join("bad/sub/broken.otf"), b"OTTO broken");
    assert_eq!(
        db.register_font_path(&dir.join("bad"), DEFAULT),
        Err(FontFileError::NoFonts { path: dir.join("bad").display().to_string(), failures: vec![("sub/broken.otf".into(), NOT_A_FONT.into())] })
    );

    std::fs::create_dir_all(dir.join("empty")).unwrap();
    assert!(matches!(db.register_font_path(&dir.join("empty"), DEFAULT), Err(FontFileError::NoFontFiles(_))));
    write(&dir.join("docs/readme.txt"), b"no fonts here");
    assert!(matches!(db.register_font_path(&dir.join("docs"), DEFAULT), Err(FontFileError::NoFontFiles(_))));

    assert_eq!(db.families(), before, "a failed registration adds nothing");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_folder_with_bad_files_registers_the_good_ones() {
    let dir = temp_dir("mixed");
    write(&dir.join("Good.ttf"), &font("PC3393 Mixed Good"));
    write(&dir.join("sub/bad.ttf"), b"garbage");
    write(&dir.join("sub/web.otf"), b"wOFF in disguise");
    let mut db = FontDb::new();
    let r = db.register_font_path(&dir, DEFAULT).unwrap();
    assert_eq!((r.families, r.files), (vec!["PC3393 Mixed Good".to_string()], 1));
    assert_eq!(r.skipped, [("sub/bad.ttf".to_string(), NOT_A_FONT.to_string()), ("sub/web.otf".to_string(), WOFF.to_string())]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn registering_the_same_file_again_adds_no_faces() {
    let dir = temp_dir("again");
    let bytes = font("PC3393 Again");
    write(&dir.join("a.ttf"), &bytes);
    write(&dir.join("copy/a.ttf"), &bytes);
    let mut db = FontDb::new();
    let r = db.register_font_path(&dir, DEFAULT).unwrap();
    assert_eq!((r.families, r.files), (vec!["PC3393 Again".to_string()], 2));
    assert_eq!(db.faces("PC3393 Again").len(), 1, "two copies of one file are one face");
    let r = db.register_font_path(&dir.join("a.ttf"), DEFAULT).unwrap();
    assert_eq!(r.families, ["PC3393 Again"], "a file registered before still reports its families");
    assert_eq!(db.faces("PC3393 Again").len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn limits_on_file_size_file_count_and_total_size() {
    let dir = temp_dir("limits");
    let (one, two) = (font("PC3393 Limits One"), font("PC3393 Limits Two"));
    write(&dir.join("one.ttf"), &one);
    write(&dir.join("two.ttf"), &two);
    let size = one.len().max(two.len()) as u64;
    let shown = dir.display().to_string();
    let mut db = FontDb::new();
    let before = db.families();

    let few = FontFileLimits { max_files: 1, ..DEFAULT };
    assert_eq!(db.register_font_path(&dir, few), Err(FontFileError::TooManyFiles { path: shown.clone(), max: 1 }));
    let small_total = FontFileLimits { max_total_bytes: size + 1, ..DEFAULT };
    assert_eq!(db.register_font_path(&dir, small_total), Err(FontFileError::TooLarge { path: shown.clone(), max_bytes: size + 1 }));
    // An oversized file is skipped unread; alone, it is the error.
    let small_file = FontFileLimits { max_file_bytes: 1000, ..DEFAULT };
    let Err(FontFileError::NoFonts { failures, .. }) = db.register_font_path(&dir, small_file) else { panic!("oversized files registered") };
    assert_eq!(failures, [("one.ttf".to_string(), "larger than 1000 bytes".to_string()), ("two.ttf".to_string(), "larger than 1000 bytes".to_string())]);
    assert_eq!(db.families(), before, "nothing registered past a limit");

    // The same limits hold for bytes read elsewhere (the automation workspace).
    let files = || vec![("one.ttf".to_string(), Ok(one.clone())), ("two.ttf".to_string(), Ok(two.clone()))];
    assert!(matches!(db.register_font_files("mem", files(), few), Err(FontFileError::TooManyFiles { max: 1, .. })));
    assert!(matches!(db.register_font_files("mem", files(), small_total), Err(FontFileError::TooLarge { .. })));
    assert!(matches!(db.register_font_files("mem", Vec::new(), DEFAULT), Err(FontFileError::NoFontFiles(_))));
    let r = db.register_font_files("mem", vec![("one.ttf".into(), Ok(one.clone())), ("gone.ttf".into(), Err("access denied".into()))], DEFAULT).unwrap();
    assert_eq!((r.families, r.skipped), (vec!["PC3393 Limits One".to_string()], vec![("gone.ttf".to_string(), "access denied".to_string())]));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn errors_read_as_sentences() {
    let failures = (0..5).map(|i| (format!("f{i}.ttf"), "bad".to_string())).collect();
    let e = FontFileError::NoFonts { path: "fonts".into(), failures };
    assert_eq!(e.to_string(), "no fonts could be read from `fonts`: f0.ttf: bad; f1.ttf: bad; f2.ttf: bad; and 2 more");
    assert_eq!(
        FontFileError::TooLarge { path: "fonts".into(), max_bytes: 1 << 30 }.to_string(),
        "the font files in `fonts` add up to more than 1 GiB; register a smaller folder"
    );
    assert_eq!(too_large(256 << 20), "larger than 256 MiB");
}
