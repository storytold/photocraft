//! #10723: File › Package, File › Import › Notes and Image › Apply Data Set steps as
//! `docs/atn.md` ("Recorded with source files") measures them.

use super::*;

fn load(items: Vec<AtnItem>) -> (Session, Value) {
    let file = AtnFile {
        name: UnicodeString::new_nul("Menus"),
        actions: vec![AtnAction { name: UnicodeString::new_nul("A"), items, ..Default::default() }],
        ..Default::default()
    };
    let mut s = Session::new();
    let r = s.execute("actions.load", json!({"data": photocraft_paint::tile::b64_encode(&atn::write(&file).unwrap())})).unwrap();
    (s, r)
}

fn named(event: &str, label: &str, d: Descriptor) -> AtnItem {
    AtnItem { event: AtnEvent::name(event), dictionary: label.as_bytes().to_vec(), descriptor: Some(d), ..Default::default() }
}

fn saved(s: &mut Session) -> Vec<AtnItem> {
    let r = s.execute("actions.save", json!({"action": "A"})).unwrap();
    assert_eq!(r["skipped"], json!([]), "{r}");
    atn::parse(&photocraft_paint::tile::b64_decode(r["data"].as_str().unwrap()).unwrap()).unwrap().actions.remove(0).items
}

/// The three steps load as their commands and save back with the same events and descriptors.
#[test]
fn package_notes_and_data_set_steps_load_and_save_back() {
    let package = desc("null").with("T   ", path_value("/work/packages"));
    let notes = desc("null").with("Usng", path_value("/work/review.psd"));
    let data_set = desc("null").with("null", name_ref("dataSetClass", "Summer sale"));
    let items = vec![
        named("packageFile", "Package", package.clone()),
        named("importAnnots", "Import Notes", notes.clone()),
        named("apply", "Apply Data Set", data_set.clone()),
    ];
    let (mut s, r) = load(items.clone());
    assert_eq!(r["unsupported"], json!([]), "{r}");
    assert_eq!(
        s.actions.list[0].steps,
        [
            ("file.package".to_string(), json!({"dir": "/work/packages"})),
            ("file.import.notes".to_string(), json!({"path": "/work/review.psd"})),
            ("image.applyDataSet".to_string(), json!({"name": "Summer sale"})),
        ]
    );
    let back = saved(&mut s);
    for (b, original) in back.iter().zip(&items) {
        assert_eq!((&b.event, &b.descriptor), (&original.event, &original.descriptor));
    }
}

/// Package plays: the document and its folder are written where the step says.
#[test]
fn a_loaded_package_step_plays() {
    let dir = std::env::temp_dir().join(format!("pc-10723-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("pkg").to_string_lossy().replace('\\', "/");
    let (mut s, _) = load(vec![named("packageFile", "Package", desc("null").with("T   ", path_value(&out)))]);
    s.execute("file.new", json!({"width": 8, "height": 8, "name": "Poster"})).unwrap();
    let played = s.execute("actions.play", json!({"action": "A"})).unwrap();
    assert_eq!(played["ran"], 1, "{played}");
    assert!(std::fs::read_dir(&out).unwrap().next().is_some(), "the package folder holds the document");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A step with a key the event does not have, or an `apply` of something other than a data set
/// by name, keeps the Photoshop step.
#[test]
fn other_forms_keep_the_photoshop_step() {
    let kept = |m: Mapped| matches!(m, Mapped::Unsupported { harmless: false });
    assert!(kept(steps_file_menu::package_from(&desc("null").with("T   ", path_value("/a")).with("Fldr", D::Boolean(true)))));
    assert!(kept(steps_file_menu::notes_from(&desc("null"))));
    assert!(kept(steps_file_menu::apply_data_set_from(&desc("null").with("null", target_ref("dataSetClass")))));
    assert!(kept(steps_file_menu::apply_data_set_from(&desc("null").with("null", name_ref("Lyr ", "Summer sale")))));
}

/// What the events cannot hold refuses the step: a package format, a data set by index.
#[test]
fn settings_without_a_key_are_refused() {
    assert!(export_item("file.package", &json!({"dir": "/a", "format": "psd"}), "S").is_none());
    assert!(export_item("image.applyDataSet", &json!({"index": 2}), "S").is_none());
    assert!(export_item("file.import.notes", &json!({}), "S").is_none());
}

/// An `apply` naming a data set with an empty name has no `image.applyDataSet` form that saves
/// back, so it keeps the Photoshop step and round-trips unchanged (CodeRabbit review on
/// steps_file_menu.rs:54).
#[test]
fn an_empty_data_set_name_keeps_the_photoshop_step() {
    let empty = desc("null").with("null", name_ref("dataSetClass", ""));
    let items = vec![named("apply", "Apply Data Set", empty.clone())];
    let (mut s, _) = load(items);
    assert_ne!(s.actions.list[0].steps[0].0, "image.applyDataSet", "{:?}", s.actions.list[0].steps);
    let back = saved(&mut s);
    assert_eq!((back[0].event.id(), back[0].descriptor.as_ref()), ("apply".to_string(), Some(&empty)));
}

/// Import Notes plays only from the files `file.import.notes` reads (.psd, .psb, .pcraft). Notes
/// recorded from a PDF or FDF keep the Photoshop step, so the load reports them as unsupported
/// rather than failing on playback (CodeRabbit review on steps_file_menu.rs:20-39).
#[test]
fn notes_from_a_pdf_keep_the_photoshop_step() {
    let kept = |m: Mapped| matches!(m, Mapped::Unsupported { harmless: false });
    for path in ["/work/review.pdf", "/work/review.PDF", "/work/review.fdf", "/work/notes"] {
        assert!(kept(steps_file_menu::notes_from(&desc("null").with("Usng", path_value(path)))), "{path}");
    }
    for path in ["/work/review.psd", "/work/review.PSB", "/work/review.pcraft"] {
        assert!(!kept(steps_file_menu::notes_from(&desc("null").with("Usng", path_value(path)))), "{path}");
    }
}
