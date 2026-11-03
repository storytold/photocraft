//! `import_file` parses a PSD/PSB from the open file (channel data read as it is decoded) and must
//! open the same document, with the same warnings, as importing the file's bytes.

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
    import_file(name, path, &photocraft_io::ImportOptions::default(), &Interrupt::NONE)
}

// Every generated case still runs once, with the same document and warning comparisons.
// Separate nextest processes let the runner schedule the matrix alongside other tests rather
// than leaving one multi-minute test on the critical path.
fn import_file_opens_what_importing_the_bytes_opens_part(part: usize) {
    let dir = TempDir::new(&format!("same-{part}"));
    let mut checked = 0;
    for case in testgen::all_cases().into_iter().skip(part).step_by(4) {
        let ext = if case.file.header.version.is_psb() { "psb" } else { "psd" };
        let name = format!("case.{ext}");
        let bytes = case.file.to_bytes().unwrap();
        let path = dir.write(&name, &bytes);
        let from_bytes = import(&name, &bytes);
        let from_file = open(&name, &path);
        match (from_bytes, from_file) {
            (Ok(a), Ok(b)) => {
                // Ids come from a process-wide counter, so the two opens differ only there.
                assert!(without_ids(&b.document) == without_ids(&a.document), "{}", case.name);
                assert_eq!(b.warnings, a.warnings, "{}", case.name);
                checked += 1;
            }
            (Err(_), Err(_)) => {}
            (a, b) => panic!("{}: bytes {:?}, file {:?}", case.name, a.map(|r| r.warnings), b.map(|r| r.warnings)),
        }
    }
    // Four parts each opening at least 13 cases retain (and slightly strengthen) the former
    // aggregate floor of more than 50 successful opens.
    assert!(checked >= 13, "part {part}: only {checked} cases opened");
}

#[test]
fn import_file_opens_what_importing_the_bytes_opens_part_0() {
    import_file_opens_what_importing_the_bytes_opens_part(0);
}

#[test]
fn import_file_opens_what_importing_the_bytes_opens_part_1() {
    import_file_opens_what_importing_the_bytes_opens_part(1);
}

#[test]
fn import_file_opens_what_importing_the_bytes_opens_part_2() {
    import_file_opens_what_importing_the_bytes_opens_part(2);
}

#[test]
fn import_file_opens_what_importing_the_bytes_opens_part_3() {
    import_file_opens_what_importing_the_bytes_opens_part(3);
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
    assert!(matches!(import_file("c.psd", &path, &photocraft_io::ImportOptions::default(), &ctl), Err(photocraft_io::IoError::Cancelled)));
}

/// A read that fails while the layers are decoded from the file (it is truncated or rewritten,
/// its drive goes away) fails the open: no layer is left empty for a save to lose.
#[test]
fn a_read_failing_while_layers_decode_fails_the_open() {
    use photocraft_psd::ReadAt;
    /// The file's bytes, failing every read of channel data (as a dropped network share or a
    /// truncated file would) except the 2-byte compression fields the parse reads.
    struct Failing {
        bytes: Vec<u8>,
        data: Vec<(u64, u64)>,
    }
    impl ReadAt for Failing {
        fn len(&self) -> u64 {
            self.bytes.len() as u64
        }
        fn read_at(&self, offset: u64, buf: &mut [u8]) -> photocraft_psd::Result<()> {
            let end = offset + buf.len() as u64;
            if self.data.iter().any(|&(a, b)| offset < b && end > a) {
                return Err(photocraft_psd::PsdError::Read("the drive went away".into()));
            }
            self.bytes.as_slice().read_at(offset, buf)
        }
    }
    for c in [photocraft_psd::Compression::Rle, photocraft_psd::Compression::Raw, photocraft_psd::Compression::Zip] {
        let file = testgen::layered(photocraft_psd::Version::Psd, photocraft_psd::ColorMode::Rgb, 8, c);
        let bytes = file.to_bytes().unwrap();
        let lazy = photocraft_psd::PsdFile::from_source(photocraft_psd::Source::new(bytes.clone())).unwrap();
        let data: Vec<(u64, u64)> = lazy
            .layers()
            .iter()
            .flat_map(|l| &l.channels)
            .filter_map(|ch| ch.stored.as_ref())
            .filter(|s| s.len > 0)
            .map(|s| (s.offset, s.offset + s.len))
            .collect();
        assert!(!data.is_empty());
        // Opening the same file whole succeeds: only the failed reads fail the open.
        assert!(import("x.psd", &bytes).is_ok());
        let r = photocraft_io::import_source_for_open_with("x.psd", photocraft_psd::Source::new(Failing { bytes, data }), &Interrupt::NONE);
        match r {
            Err(photocraft_io::IoError::Read(msg)) => assert!(msg.contains("the drive went away"), "{c:?}: {msg}"),
            r => panic!("{c:?}: the open didn't fail: {:?}", r.map(|(r, damaged)| (r.warnings, damaged))),
        }
    }
}
// A held public mutex proves a dependency, not system-font initialization latency.
#[derive(Clone, Copy)]
enum NativeTextFixture {
    FlatRaster,
    GroupedRaster,
    GroupedCachedType,
}

fn native_text_fixture(dir: &TempDir, kind: NativeTextFixture) -> std::path::PathBuf {
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_doc::{
        Document, Layer, LayerContent, TextLayer,
        text::{CharStyle, TextRun},
    };
    use photocraft_geom::{Affine, Rect, Size};

    let mut doc = Document::new("native text dependency", Size::new(96, 40), ColorMode::Rgb, SampleType::U8);
    let leaf = match kind {
        NativeTextFixture::GroupedCachedType => {
            let mut text = TextLayer {
                text: "Font control".into(),
                runs: vec![TextRun {
                    len: 12,
                    style: CharStyle { font_family: "Inter".into(), postscript_name: Some("Inter-Regular".into()), size_pt: 12.0, ..Default::default() },
                }],
                transform: Affine { m: [1.0, 0.0, 0.0, 1.0, 4.0, 20.0] },
                ..Default::default()
            };
            text.sync_summary();
            // Prepare the cached appearance unlocked, before the observed import.
            photocraft_text::TextEngine::new().render_layer(&mut text, doc.resolution_dpi, doc.pixel_format());
            assert!(!text.cache.as_ref().unwrap().content_bounds().is_empty());
            Layer::new("type", LayerContent::Text(text))
        }
        _ => {
            let mut leaf = Layer::raster("raster", PixelFormat::RGBA8);
            leaf.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 3, 2), &[0.25, 0.5, 0.75, 1.0]);
            leaf
        }
    };
    doc.layers.push(match kind {
        NativeTextFixture::FlatRaster => leaf,
        _ => Layer::group("outer", vec![Layer::group("inner", vec![leaf])]),
    });
    let bytes = photocraft_io::export(&doc, "native.psd", &Default::default()).unwrap().bytes;
    assert!(bytes.starts_with(b"8BPS"));
    dir.write("native.psd", &bytes)
}

fn check_native_text_fixture(doc: &photocraft_doc::Document, kind: NativeTextFixture) {
    use photocraft_doc::LayerContent;
    assert_eq!((doc.size.width, doc.size.height), (96, 40));
    let layers = doc.walk();
    let types: Vec<_> = layers
        .iter()
        .filter_map(|(_, depth, layer)| match &layer.content {
            LayerContent::Text(t) => Some((*depth, t)),
            _ => None,
        })
        .collect();
    match kind {
        NativeTextFixture::GroupedCachedType => {
            assert_eq!(types.len(), 1);
            let (depth, t) = types[0];
            assert!(depth >= 2, "editable type is genuinely nested");
            assert_eq!(t.text, "Font control");
            assert_eq!(t.font_family, "Inter");
            assert_eq!(t.char_runs()[0].style.font_family, "Inter");
            assert_eq!(t.char_runs()[0].style.postscript_name.as_deref(), Some("Inter-Regular"));
            assert!(!t.cache.as_ref().expect("cached type keeps its appearance").content_bounds().is_empty());
        }
        _ => {
            assert!(types.is_empty(), "no editable type at any depth");
            let (depth, pixels) = layers
                .iter()
                .find_map(|(_, depth, layer)| match &layer.content {
                    LayerContent::Raster(s) if layer.name == "raster" => Some((*depth, s)),
                    _ => None,
                })
                .expect("ordinary raster layer survives import");
            assert_eq!(pixels.pixel(0, 0), vec![64.0 / 255.0, 128.0 / 255.0, 191.0 / 255.0, 1.0]);
            if matches!(kind, NativeTextFixture::GroupedRaster) {
                assert!(depth >= 2, "raster is genuinely nested");
            } else {
                assert_eq!(depth, 0);
            }
        }
    }
}

fn native_text_dependency_case(tag: &str, kind: NativeTextFixture) {
    use std::sync::mpsc;
    use std::time::Duration;
    let dir = TempDir::new(tag);
    let path = native_text_fixture(&dir, kind);
    // Normal unlocked import establishes the same file's successful model and appearance.
    let expected = photocraft_io::import_file_for_open_with("native.psd", &path, &Default::default(), &Interrupt::NONE).unwrap();
    check_native_text_fixture(&expected.0.document, kind);
    let held = photocraft_text::shared().lock().unwrap_or_else(|e| e.into_inner());
    let (started_tx, started_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let _ = started_tx.send(());
        let imported = photocraft_io::import_file_for_open_with("native.psd", &path, &Default::default(), &Interrupt::NONE);
        let _ = done_tx.send(());
        imported
    });
    let started = started_rx.recv_timeout(Duration::from_secs(10));
    let observed = started.as_ref().ok().map(|_| done_rx.recv_timeout(Duration::from_secs(10)));
    // Always release before joining/asserting, including the expected baseline timeout.
    // The external exact-test process bound also covers a worker that fails to drain.
    drop(held);
    let actual = worker.join().expect("import worker panicked").expect("ordinary file import failed");
    assert!(started.is_ok(), "worker never acknowledged the public import call");
    check_native_text_fixture(&actual.0.document, kind);
    assert_eq!(actual.1, expected.1);
    assert_eq!(actual.0.warnings, expected.0.warnings);
    assert_eq!(without_ids(&actual.0.document), without_ids(&expected.0.document));
    // Compare actual raster/cache samples as well as the editable model/debug representation.
    let a = actual.0.document.walk();
    let b = expected.0.document.walk();
    for ((_, _, l), (_, _, expected)) in a.iter().zip(&b) {
        if let (Some(pixels), Some(want)) = (l.surface(), expected.surface()) {
            let rect = want.content_bounds();
            assert_eq!(pixels.content_bounds(), rect);
            assert_eq!(pixels.read_region(rect), want.read_region(rect));
        }
    }
    match kind {
        NativeTextFixture::GroupedCachedType => assert!(
            matches!(observed, Some(Err(mpsc::RecvTimeoutError::Timeout))),
            "real type retains text preparation; timeout is not a font-speed measurement"
        ),
        _ => assert!(matches!(observed, Some(Ok(()))), "no-type native PSD waited for the held text engine"),
    }
}

#[test]
fn native_psd_text_dependency_flat_raster_returns_with_engine_held() {
    native_text_dependency_case("font-wait-flat", NativeTextFixture::FlatRaster);
}

#[test]
fn native_psd_text_dependency_grouped_raster_returns_with_engine_held() {
    native_text_dependency_case("font-wait-grouped", NativeTextFixture::GroupedRaster);
}

#[test]
fn native_psd_text_dependency_grouped_type_retains_preparation() {
    native_text_dependency_case("font-wait-type", NativeTextFixture::GroupedCachedType);
}
