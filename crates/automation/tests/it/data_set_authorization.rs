//! #8586: Pixel Replacement values of variable data sets are image paths. Automation stores and
//! reads them only beneath the workspace's read root: storing an outside path is refused, and
//! exporting data sets reads each image through the root's capability, never an ambient path.
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use photocraft_automation::{AuthorizedWorkspace, Headless};
use serde_json::{Value, json};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let base = std::env::temp_dir().join(format!("pc-data-set-auth-{}-{stamp}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(base.join("workspace")).unwrap();
        std::fs::create_dir_all(base.join("outside")).unwrap();
        let f = Self(base);
        std::fs::write(f.root().join("inside.png"), white_png()).unwrap();
        std::fs::write(f.outside(), white_png()).unwrap();
        f
    }

    fn root(&self) -> PathBuf {
        self.0.join("workspace")
    }
    /// An image outside the workspace, as an absolute path.
    fn outside(&self) -> PathBuf {
        self.0.join("outside").join("photo.png")
    }
    fn headless(&self) -> Headless {
        Headless::with_workspace(AuthorizedWorkspace::new(Some(&self.root()), Some(&self.root())).unwrap())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn white_png() -> Vec<u8> {
    let image = photocraft_codecs::Image::from_u8(4, 4, photocraft_codecs::ChannelLayout::Rgb, vec![255; 4 * 4 * 3]).unwrap();
    photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap()
}

fn run(h: &mut Headless, command: &str, params: Value) -> Result<Value, photocraft_automation::AutomationError> {
    h.handle("engine.execute", json!({"command": command, "params": params}))
}

/// A black 8×8 document whose layer is bound to a Pixel Replacement variable `photo`.
fn template(h: &mut Headless) {
    h.handle("doc.new", json!({"width": 8, "height": 8, "background": "#000000", "name": "card"})).unwrap();
    let layer = h.handle("doc.inspect", json!({})).unwrap()["layers"][0]["id"].as_u64().expect("the layer id");
    run(h, "image.variables.define", json!({"defs": [{"name": "photo", "layer": layer, "type": "pixelReplacement", "method": "conform"}]})).unwrap();
}

fn pixels(path: &str) -> Value {
    json!({"dataSets": [{"name": "swap", "values": [{"variable": "photo", "kind": "pixels", "value": path}]}]})
}

fn stored_sets(h: &Headless) -> usize {
    h.session.active().unwrap().doc.variables.data_sets.len()
}

fn assert_outside_refused<T: std::fmt::Debug>(result: Result<T, impl std::fmt::Display>) {
    match result {
        Err(error) => assert!(error.to_string().contains("must be inside the automation root"), "{error}"),
        Ok(value) => panic!("an image outside the workspace was accepted: {value:?}"),
    }
}

fn exported_files(dir: &Path) -> usize {
    std::fs::read_dir(dir).map(|entries| entries.count()).unwrap_or(0)
}

#[test]
fn data_sets_cannot_store_an_image_path_outside_the_workspace() {
    let f = Fixture::new();
    let mut h = f.headless();
    template(&mut h);
    let outside = f.outside().to_string_lossy().into_owned();
    for path in [outside.as_str(), "../outside/photo.png"] {
        assert!(run(&mut h, "image.variables.dataSets", pixels(path)).is_err(), "{path} was stored");
        assert_eq!(stored_sets(&h), 0, "{path}: nothing was stored");
    }
    assert_outside_refused(run(&mut h, "image.variables.dataSets", pixels(&outside)));
    // A CSV beneath the root may not name an outside image either.
    std::fs::write(f.root().join("sets.csv"), format!("Data Set,photo\nswap,{outside}\n")).unwrap();
    assert_outside_refused(h.handle("doc.importDataSets", json!({"path": "sets.csv"})));
    assert_eq!(stored_sets(&h), 0, "the refused CSV stored nothing");
}

#[test]
fn exporting_a_stored_outside_image_path_is_refused_before_it_is_read() {
    let f = Fixture::new();
    // A document saved elsewhere already holds a data set naming an outside image.
    let mut trusted = Headless::trusted_local();
    template(&mut trusted);
    run(&mut trusted, "image.variables.dataSets", pixels(&f.outside().to_string_lossy())).unwrap();
    trusted.handle("doc.save", json!({"path": f.root().join("stored.pcraft").to_string_lossy()})).unwrap();
    drop(trusted);

    let mut h = f.headless();
    h.open(Path::new("stored.pcraft")).unwrap();
    assert_eq!(stored_sets(&h), 1);
    assert_outside_refused(h.handle("doc.exportDataSets", json!({"dir": "sets", "format": "png"})));
    assert_eq!(exported_files(&f.root().join("sets")), 0, "no file carries the outside image");
}

#[test]
fn data_sets_with_images_beneath_the_root_still_store_import_and_export() {
    let f = Fixture::new();
    let mut h = f.headless();
    template(&mut h);
    run(&mut h, "image.variables.dataSets", pixels("inside.png")).unwrap();
    let absolute = f.root().join("inside.png").to_string_lossy().into_owned();
    run(
        &mut h,
        "image.variables.dataSets",
        json!({"dataSets": [{"name": "abs", "values": [{"variable": "photo", "kind": "pixels", "value": absolute}]}], "append": true}),
    )
    .unwrap();
    assert_eq!(stored_sets(&h), 2);

    let exported = h.handle("doc.exportDataSets", json!({"dir": "sets", "format": "png"})).unwrap();
    assert_eq!(exported["count"].as_u64(), Some(2), "{exported}");
    for name in ["swap.png", "abs.png"] {
        let image = photocraft_codecs::decode(&std::fs::read(f.root().join("sets").join(name)).unwrap()).unwrap().to_rgba8();
        assert!(image.chunks(4).all(|px| px[..3] == [255, 255, 255]), "{name}: the image beneath the root replaced the black pixels");
    }

    std::fs::write(f.root().join("sets.csv"), "Data Set,photo\none,inside.png\n").unwrap();
    let imported = h.handle("doc.importDataSets", json!({"path": "sets.csv"})).unwrap();
    assert_eq!(imported["imported"].as_u64(), Some(1), "{imported}");
}

#[test]
fn trusted_local_exports_still_read_any_image_path() {
    let f = Fixture::new();
    let mut h = Headless::trusted_local();
    template(&mut h);
    run(&mut h, "image.variables.dataSets", pixels(&f.outside().to_string_lossy())).unwrap();
    let out = f.root().join("trusted");
    let exported = h.handle("doc.exportDataSets", json!({"dir": out.to_string_lossy(), "format": "png"})).unwrap();
    assert_eq!(exported["count"].as_u64(), Some(1), "{exported}");
    let image = photocraft_codecs::decode(&std::fs::read(out.join("swap.png")).unwrap()).unwrap().to_rgba8();
    assert!(image.chunks(4).all(|px| px[..3] == [255, 255, 255]));
}
