//! Saving and loading through `BundleStore`: the built-in ZIP and directory stores match the
//! writer's own save paths byte for byte, and a store from outside the crate sees every step.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use crate::common::*;
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::Document;
use photocraft_format::*;
use photocraft_raster::Rgba8Image;

fn with_previews() -> SaveOptions {
    let mut thumb = Rgba8Image::new(4, 3);
    thumb.pixels.iter_mut().enumerate().for_each(|(i, p)| *p = i as u8);
    let mut composite = Rgba8Image::new(9, 5);
    composite.pixels.iter_mut().enumerate().for_each(|(i, p)| *p = (i as u8).wrapping_mul(13));
    SaveOptions { thumbnail: Some(thumb), composite: Some(composite) }
}

/// Every file of a directory bundle by relative path, except the lock file.
fn tree(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    for sub in ["", "tiles", "blobs", "composite"] {
        for entry in std::fs::read_dir(dir.join(sub)).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().into_owned();
            if entry.file_type().unwrap().is_file() && name != "save.lock" {
                let path = if sub.is_empty() { name } else { format!("{sub}/{name}") };
                out.insert(path, std::fs::read(entry.path()).unwrap());
            }
        }
    }
    out
}

fn load(store: &dyn BundleStore) -> Result<Document> {
    load_bytes_source(store, &LoadOptions::default())
}

#[test]
fn zip_and_directory_stores_write_what_the_writer_paths_write() {
    for (mode, depth) in [(ColorMode::Rgb, SampleType::U8), (ColorMode::Cmyk, SampleType::U16), (ColorMode::Lab, SampleType::F32)] {
        let doc = rich_doc(mode, depth);
        for opts in [SaveOptions::default(), with_previews()] {
            let mut zip = ZipStore::new();
            PcraftWriter::new().save_to(&doc, &mut zip, &opts).unwrap();
            assert_eq!(zip.bytes(), save_to_bytes(&doc, &opts).unwrap(), "{mode:?} {depth:?}");
            assert_eq!(load(&zip).unwrap(), doc);

            let (through_store, through_writer) = (temp_dir("store-dir"), temp_dir("writer-dir"));
            let mut dir = DirStore::new(&through_store);
            PcraftWriter::new().save_to(&doc, &mut dir, &opts).unwrap();
            PcraftWriter::new().save_dir(&doc, &through_writer, &opts).unwrap();
            assert_eq!(tree(&through_store), tree(&through_writer), "{mode:?} {depth:?}");
            assert_eq!(load(&dir).unwrap(), doc);
            assert_eq!(load_path(&through_store).unwrap(), doc);
            std::fs::remove_dir_all(through_store).unwrap();
            std::fs::remove_dir_all(through_writer).unwrap();
        }
    }
}

#[test]
fn incremental_saves_through_a_store_count_what_the_writer_paths_count() {
    let mut doc = rich_doc(ColorMode::Rgb, SampleType::U16);
    let (store_dir, writer_dir) = (temp_dir("store-incremental"), temp_dir("writer-incremental"));
    let (mut via_store, mut via_writer) = (PcraftWriter::new(), PcraftWriter::new());
    let (mut zip, mut dir) = (ZipStore::new(), DirStore::new(&store_dir));
    let mut save = |doc: &Document| {
        let zip_stats = via_store.save_to(doc, &mut zip, &SaveOptions::default()).unwrap();
        let (bytes, writer_zip_stats) = via_writer.save_zip(doc, &SaveOptions::default()).unwrap();
        assert_eq!(zip_stats, writer_zip_stats);
        assert_eq!(zip.bytes(), bytes);
        let dir_stats = via_store.save_to(doc, &mut dir, &SaveOptions::default()).unwrap();
        assert_eq!(dir_stats, via_writer.save_dir(doc, &writer_dir, &SaveOptions::default()).unwrap());
        assert_eq!(tree(&store_dir), tree(&writer_dir));
        (zip_stats, dir_stats)
    };
    let (zip1, dir1) = save(&doc);
    assert_eq!((zip1.tiles_written, dir1.tiles_written), (zip1.tiles_total, dir1.tiles_total));
    let (zip2, dir2) = save(&doc);
    assert_eq!((zip2.tiles_written, dir2.tiles_written), (0, 0));
    // One new tile, then a layer gone: its tiles are removed from the directory.
    let id = doc.layers[1].id;
    doc.layer_mut(id).unwrap().surface_mut().unwrap().write_pixel(3, 3, &[0.5, 0.5, 0.5, 1.0]);
    let (zip3, dir3) = save(&doc);
    assert_eq!((zip3.tiles_written, dir3.tiles_written), (1, 1));
    doc.remove(id);
    let (_, dir4) = save(&doc);
    assert!(dir4.objects_removed > 0, "{dir4:?}");
    assert_eq!(load(&zip).unwrap(), doc);
    assert_eq!(load(&dir).unwrap(), doc);
    std::fs::remove_dir_all(store_dir).unwrap();
    std::fs::remove_dir_all(writer_dir).unwrap();
}

#[test]
fn zip_store_reads_an_existing_archive_and_refuses_reads_before_one() {
    let doc = rich_doc(ColorMode::Grayscale, SampleType::U8);
    let empty = ZipStore::new();
    assert!(matches!(empty.get("manifest.json", usize::MAX), Err(FormatError::Corrupt(_))));
    assert!(load(&empty).is_err());
    let bytes = save_to_bytes(&doc, &with_previews()).unwrap();
    assert!(ZipStore::from_bytes(b"not a zip".to_vec()).is_err());
    let mut store = ZipStore::from_bytes(bytes.clone()).unwrap();
    assert_eq!(load(&store).unwrap(), doc);
    assert_eq!(store.get("thumb.png", usize::MAX).unwrap(), read_thumbnail(&bytes).unwrap().unwrap());
    assert!(matches!(store.get("thumb.png", 3), Err(FormatError::LimitExceeded(_))));
    // A save replaces the archive.
    let other = rich_doc(ColorMode::Rgb, SampleType::U8);
    PcraftWriter::new().save_to(&other, &mut store, &SaveOptions::default()).unwrap();
    assert_eq!(load(&store).unwrap(), other);
    assert!(store.get("thumb.png", usize::MAX).is_err());
    assert_eq!(store.into_bytes(), save_to_bytes(&other, &SaveOptions::default()).unwrap());
}

/// A backend outside this crate: objects and the manifest in a map, every call logged.
#[derive(Default)]
struct MemoryStore {
    files: HashMap<String, Vec<u8>>,
    log: Vec<String>,
    planned: Vec<String>,
    fail: Option<&'static str>,
}

impl MemoryStore {
    fn step(&mut self, step: &str) -> Result<()> {
        self.log.push(step.to_owned());
        match self.fail {
            Some(name) if step.starts_with(name) => Err(FormatError::Io(std::io::Error::other(format!("{name} failed")))),
            _ => Ok(()),
        }
    }
}

impl ByteSource for MemoryStore {
    fn get(&self, path: &str, max: usize) -> Result<Vec<u8>> {
        let bytes = self.files.get(path).ok_or_else(|| FormatError::Corrupt(format!("missing `{path}`")))?;
        if bytes.len() > max {
            return Err(FormatError::LimitExceeded(path.to_owned()));
        }
        Ok(bytes.clone())
    }
}

impl BundleStore for MemoryStore {
    fn begin(&mut self) -> Result<()> {
        self.step("begin")
    }

    fn plan(&mut self, objects: &[BundleObject<'_>]) -> Result<()> {
        self.planned = objects.iter().map(|o| o.path().to_owned()).collect();
        self.step("plan")
    }

    fn object(&mut self, object: &BundleObject<'_>) -> Result<Stored> {
        self.step(&format!("object {}", object.path()))?;
        assert_eq!(object.is_tile(), object.path().starts_with("tiles/"));
        if self.files.contains_key(object.path()) {
            return Ok(Stored::Reused);
        }
        self.files.insert(object.path().to_owned(), object.compressed());
        Ok(Stored::Written)
    }

    fn preview(&mut self, path: &str, png: Vec<u8>) -> Result<()> {
        self.step(&format!("preview {path}"))?;
        self.files.insert(path.to_owned(), png);
        Ok(())
    }

    fn commit(&mut self, manifest: &[u8]) -> Result<usize> {
        self.step("commit")?;
        self.files.insert("manifest.json".to_owned(), manifest.to_vec());
        Ok(0)
    }

    fn end(&mut self) {
        self.log.push("end".to_owned());
    }
}

#[test]
fn a_store_from_outside_the_crate_gets_every_step_in_order() {
    let doc = rich_doc(ColorMode::Rgb, SampleType::U8);
    let mut store = MemoryStore::default();
    let mut writer = PcraftWriter::new();
    let stats = writer.save_to(&doc, &mut store, &with_previews()).unwrap();
    assert_eq!(stats.tiles_written, stats.tiles_total);
    assert_eq!(stats.blobs_written, stats.blobs_total);
    let mut want = vec!["begin".to_owned(), "plan".to_owned()];
    assert!(!store.planned.is_empty() && store.planned.is_sorted(), "{:?}", store.planned);
    want.extend(store.planned.iter().map(|p| format!("object {p}")));
    want.extend(["preview thumb.png", "preview composite/preview.png", "commit", "end"].map(String::from));
    assert_eq!(store.log, want);
    assert_eq!(load(&store).unwrap(), doc);
    // The bytes are those a ZIP bundle holds.
    let zip = ZipStore::from_bytes(save_to_bytes(&doc, &with_previews()).unwrap()).unwrap();
    for (path, bytes) in &store.files {
        assert_eq!(&zip.get(path, usize::MAX).unwrap(), bytes, "{path}");
    }

    // Saved again (here by a fresh writer), nothing is written.
    store.log.clear();
    let stats = PcraftWriter::new().save_to(&doc, &mut store, &SaveOptions::default()).unwrap();
    assert_eq!((stats.tiles_written, stats.blobs_written, stats.tiles_reused), (0, 0, stats.tiles_total));
    assert_eq!(store.log.last().map(String::as_str), Some("end"));
}

#[test]
fn a_failed_save_never_commits_and_always_ends() {
    let doc = rich_doc(ColorMode::Rgb, SampleType::U8);
    for fail in ["begin", "plan", "object", "preview", "commit"] {
        let mut store = MemoryStore { fail: Some(fail), ..Default::default() };
        let e = PcraftWriter::new().save_to(&doc, &mut store, &with_previews()).unwrap_err();
        assert!(e.to_string().contains(&format!("{fail} failed")), "{e}");
        let failed = store.log.iter().position(|s| s.starts_with(fail)).unwrap();
        assert_eq!(store.log.len(), failed + 2, "nothing after the failed step but end: {:?}", store.log);
        assert_eq!(store.log.last().map(String::as_str), Some("end"));
        assert!(!store.files.contains_key("manifest.json"), "{fail}");
    }
}

#[test]
fn a_document_the_writer_refuses_reaches_no_store() {
    let mut doc = rich_doc(ColorMode::Rgb, SampleType::U8);
    doc.layers[1].opacity = f32::NAN;
    let mut store = MemoryStore::default();
    let e = PcraftWriter::new().save_to(&doc, &mut store, &SaveOptions::default()).unwrap_err();
    assert!(matches!(e, FormatError::NonFinite { .. }), "{e}");
    assert_eq!(store.log, ["begin", "end"]);
    assert!(store.files.is_empty());
}
