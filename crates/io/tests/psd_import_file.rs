//! `import_file` parses a PSD/PSB from the open file (channel data read as it is decoded) and must
//! open the same document, with the same warnings, as importing the file's bytes.

use photocraft_doc::MAX_GROUP_DEPTH;
use photocraft_io::{ImportResult, import, import_file};
use photocraft_psd::testgen;
use photocraft_raster::Interrupt;

struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("photocraft-import-file-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }

    fn write(&self, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The document's debug form with every `…Id(n)` number blanked.
fn without_ids(doc: &photocraft_doc::Document) -> String {
    let s = format!("{doc:?}");
    let mut out = String::with_capacity(s.len());
    let mut rest = s.as_str();
    while let Some(at) = rest.find("Id(") {
        out.push_str(&rest[..at + 3]);
        rest = rest[at + 3..].trim_start_matches(|c: char| c.is_ascii_digit());
    }
    out.push_str(rest);
    out
}

fn open(name: &str, path: &std::path::Path) -> Result<ImportResult, photocraft_io::IoError> {
    import_file(name, path, MAX_GROUP_DEPTH, &Interrupt::NONE)
}

#[test]
fn import_file_opens_what_importing_the_bytes_opens() {
    let dir = TempDir::new("same");
    let mut checked = 0;
    for case in testgen::all_cases() {
        let ext = if case.file.header.version.is_psb() { "psb" } else { "psd" };
        let name = format!("case.{ext}");
        let bytes = case.file.to_bytes().unwrap();
        let path = dir.write(&name, &bytes);
        let from_bytes = import(&name, &bytes);
        let from_file = open(&name, &path);
        match (from_bytes, from_file) {
            (Ok(a), Ok(b)) => {
                // Hidden layers opened from the file decode when first used.
                b.document.decode_lazy_layers().unwrap();
                // Ids come from a process-wide counter, so the two opens differ only there.
                assert!(without_ids(&b.document) == without_ids(&a.document), "{}", case.name);
                assert_eq!(b.warnings, a.warnings, "{}", case.name);
                checked += 1;
            }
            (Err(_), Err(_)) => {}
            (a, b) => panic!("{}: bytes {:?}, file {:?}", case.name, a.map(|r| r.warnings), b.map(|r| r.warnings)),
        }
    }
    assert!(checked > 50, "only {checked} cases opened");
}

#[test]
fn import_file_reads_other_formats_whole() {
    let dir = TempDir::new("other");
    let doc = photocraft_doc::Document::new("x", photocraft_geom::Size::new(3, 2), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    let png = photocraft_io::export(&doc, "x.png", &Default::default()).unwrap().bytes;
    let path = dir.write("x.png", &png);
    let r = open("x.png", &path).unwrap();
    assert_eq!(r.document.size, doc.size);
}

#[test]
fn import_file_fails_cleanly() {
    let dir = TempDir::new("fail");
    assert!(open("missing.psd", &dir.0.join("missing.psd")).is_err());
    let bytes = testgen::small(photocraft_psd::Version::Psd, photocraft_psd::Compression::Rle).to_bytes().unwrap();
    // Cut inside the layer records, inside the channel data, and inside the merged image.
    for cut in [10, 60, bytes.len() / 2, bytes.len() - 5] {
        let name = format!("cut{cut}.psd");
        let path = dir.write(&name, &bytes[..cut]);
        let from_bytes = import(&name, &bytes[..cut]);
        let from_file = open(&name, &path);
        // The merged image is not read when the layers are what opens, so a file cut inside it
        // can open from the file where the strict parse of its bytes refused it.
        if from_bytes.is_ok() {
            assert!(from_file.is_ok(), "cut at {cut}");
        }
    }
}

#[test]
fn a_cancelled_import_file_reports_cancelled() {
    let dir = TempDir::new("cancel");
    let bytes = testgen::small(photocraft_psd::Version::Psd, photocraft_psd::Compression::Zip).to_bytes().unwrap();
    let path = dir.write("c.psd", &bytes);
    let cancel = || true;
    let ctl = Interrupt::cancel_only(&cancel);
    assert!(matches!(import_file("c.psd", &path, MAX_GROUP_DEPTH, &ctl), Err(photocraft_io::IoError::Cancelled)));
}

/// A hidden layer opened from a file stays undecoded (compositing skips it), shows a preview
/// for its thumbnail, decodes to the pixels a full import has, and a save fails rather than
/// writing it empty when the file can no longer be read.
#[cfg(unix)]
#[test]
fn hidden_layers_decode_when_first_used() {
    let dir = TempDir::new("lazy");
    for c in [photocraft_psd::Compression::Rle, photocraft_psd::Compression::Raw, photocraft_psd::Compression::Zip] {
        let mut file = testgen::layered(photocraft_psd::Version::Psd, photocraft_psd::ColorMode::Rgb, 8, c);
        let hidden = file.layers().iter().position(|l| l.channels.iter().any(|ch| ch.id == 0) && !l.rect.is_empty()).unwrap();
        file.layers_mut()[hidden].flags.0 |= 2;
        let bytes = file.to_bytes().unwrap();
        let path = dir.write("lazy.psd", &bytes);
        let eager = import("lazy.psd", &bytes).unwrap().document;
        let lazy = open("lazy.psd", &path).unwrap().document;
        let pending: Vec<_> = lazy.walk().into_iter().filter(|(_, _, l)| l.surface().is_some_and(|s| s.is_pending())).map(|(_, _, l)| l.name.clone()).collect();
        let name = file.layers()[hidden].name();
        assert!(pending.contains(&name), "{c:?}: {name} not among {pending:?}");
        let hidden_count = file.layers().iter().filter(|l| !l.is_visible() && !l.rect.is_empty() && l.channels.iter().any(|ch| ch.id == 0)).count();
        assert_eq!(pending.len(), hidden_count, "{c:?}: {pending:?}");
        let layer = lazy.walk().into_iter().map(|(_, _, l)| l).find(|l| l.surface().is_some_and(|s| s.is_pending())).unwrap();
        let s = layer.surface().unwrap();
        let mut px = vec![0.0; s.channels()];
        let b = s.pending_bounds().unwrap();
        assert!(s.read_preview_pixel(b.x0, b.y0, &mut px));
        // The composite of a hidden layer's document doesn't need it.
        assert_eq!(photocraft_compose::flatten(&lazy).px, photocraft_compose::flatten(&eager).px, "{c:?}");
        assert!(s.is_pending(), "{c:?}: compositing decoded the hidden layer");
        lazy.decode_lazy_layers().unwrap();
        assert!(!s.is_pending());
        assert!(without_ids(&lazy) == without_ids(&eager), "{c:?}");
        // The file changed after opening: the hidden layer can't be read, the save says so.
        let again = open("lazy.psd", &path).unwrap().document;
        std::fs::write(&path, b"replaced in place").unwrap();
        assert!(again.decode_lazy_layers().is_err(), "{c:?}");
        assert!(photocraft_io::export(&again, "x.psd", &Default::default()).is_err(), "{c:?}");
    }
}
