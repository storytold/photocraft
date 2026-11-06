//! Replace Contents hashes and decodes the new file once, whatever the number of instances.

use super::*;

/// A solid PNG of `w × h` written to `dir/name`.
fn png_file(dir: &std::path::Path, name: &str, w: u32, h: u32, rgb: [f32; 3]) -> String {
    let doc = Document::with_background(
        "p",
        Size::new(w, h),
        photocraft_color::ColorMode::Rgb,
        photocraft_color::SampleType::U8,
        photocraft_color::Color::rgba(rgb[0], rgb[1], rgb[2], 1.0),
    );
    let path = dir.join(name);
    std::fs::write(&path, photocraft_io::export(&doc, "png", &Default::default()).unwrap().bytes).unwrap();
    path.to_str().unwrap().to_string()
}

/// A scratch directory for test `tag`, created if missing.
fn tmp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("pcraft-replace-once-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A converted smart object (painted block) and its layer id.
fn smart_session() -> (Session, LayerId) {
    let mut s = session(8);
    paint(&mut s);
    let id = LayerId(convert(&mut s));
    (s, id)
}

/// Runs `layer.smartObjects.replaceContents` on `id` from `path`.
fn replace(s: &mut Session, id: LayerId, path: &str) {
    s.execute("layer.smartObjects.replaceContents", json!({"layer": id.0, "path": path})).unwrap();
}

/// The digest is the buffer's blake3, remembered for the buffer, never served to a new one.
#[test]
fn contents_digest_is_blake3_and_never_stale_for_a_new_buffer() {
    let digest = super::super::rendering::contents_digest;
    let a = Arc::new(vec![1u8; 4096]);
    assert_eq!(digest(&a), *blake3::hash(&a).as_bytes());
    assert_eq!(digest(&a), *blake3::hash(&a).as_bytes(), "remembered");
    drop(a);
    // A new buffer (possibly at the freed address) must be hashed, not served the old digest.
    let b = Arc::new(vec![2u8; 4096]);
    assert_eq!(digest(&b), *blake3::hash(&b).as_bytes());
}

/// Three instances of one smart object share the new file: it is hashed once for all of them.
#[test]
fn replacing_shared_contents_hashes_the_new_file_once_for_all_instances() {
    let hashes = || super::super::rendering::CONTENTS_HASHES.with(std::cell::Cell::get);
    let dir = tmp_dir("hash-once");
    let (mut s, id) = smart_session();
    for _ in 0..2 {
        s.execute("layer.duplicate", json!({"layer": id.0})).unwrap();
    }
    let before = hashes();
    replace(&mut s, id, &png_file(&dir, "big.png", 300, 200, [0.5, 0.2, 0.9]));
    let hashed = hashes() - before;
    assert_eq!(hashed, 1, "hashed the new file {hashed} times");
    std::fs::remove_dir_all(&dir).ok();
}

/// The file is decoded once to check it; the re-render reuses that image.
#[test]
fn replacing_contents_decodes_the_new_file_once() {
    let dir = tmp_dir("decode-once");
    let (mut s, id) = smart_session();
    s.execute("layer.duplicate", json!({"layer": id.0})).unwrap();
    let path = png_file(&dir, "d.png", 121, 79, [0.3, 0.6, 0.9]);
    let decodes = || DECODES.with(std::cell::Cell::get);
    let before = decodes();
    replace(&mut s, id, &path);
    assert_eq!(decodes() - before, 1, "decoded the new file {} times", decodes() - before);
    std::fs::remove_dir_all(&dir).ok();
}
