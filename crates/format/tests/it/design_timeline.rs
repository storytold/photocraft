//! A design timeline survives a save: its base document, steps, marker and baked results.
use std::sync::Arc;

use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::design::{Baked, DesignStep, DesignTimeline, StepState};
use photocraft_doc::{Document, IdScope, Layer, Size, Surface};
use photocraft_format::*;
use serde_json::json;

/// The format version as stored, before any migration.
fn stored_version(bytes: &[u8]) -> u64 {
    let raw = zip::ZipReader::new(bytes).unwrap().read_by_name("manifest.json", usize::MAX).unwrap();
    serde_json::from_slice::<serde_json::Value>(&raw).unwrap()["format_version"].as_u64().unwrap()
}

/// A plain recorded step.
fn step(step: u64, command: &str) -> DesignStep {
    DesignStep {
        step,
        command: command.into(),
        params: json!({"opacity": 0.5}),
        label: command.into(),
        prelude: Vec::new(),
        active: None,
        selected: Vec::new(),
        created: Vec::new(),
        changed: Vec::new(),
        suppressed: false,
        joins_previous: false,
        baked: None,
        rerecord: false,
        state: StepState::Ok,
        checkpoint: Default::default(),
    }
}

/// A document with a timeline holding a base, steps, a baked result (with a profile) and a marker.
fn with_timeline() -> Document {
    let _scope = IdScope::new(1).enter();
    let mut base = Document::new("d", Size::new(8, 8), ColorMode::Rgb, SampleType::U8);
    base.layers.push(Layer::raster("Background", PixelFormat::RGBA8));
    let mut doc = base.clone();
    let mut a = Layer::raster("A", PixelFormat::RGBA8);
    a.opacity = 0.5;
    let aid = a.id;
    doc.layers.push(a);
    let mut sel = Surface::new(PixelFormat::GRAY8);
    sel.fill_rect(photocraft_geom::Rect::new(0, 0, 4, 4), &[1.0]);
    doc.selection = Some(sel);
    doc.icc_profile = Some(Arc::new(b"profile".to_vec()));
    let mut first = step(2, "layer.new.layer");
    first.created = vec![aid];
    let mut second = step(3, "select.reselect");
    second.suppressed = true;
    second.baked = Some(Arc::new(Baked::capture(&base, &doc, &[aid], &[])));
    doc.design_timeline = Some(Arc::new(DesignTimeline { base: Arc::new(base), steps: vec![first, second], marker: Some(1) }));
    doc
}

#[test]
fn a_timeline_round_trips_and_marks_the_file_format_2() {
    let doc = with_timeline();
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    assert_eq!(stored_version(&bytes), u64::from(FORMAT_VERSION));
    let back = load_from_bytes(&bytes).unwrap();
    assert_eq!(back, doc);
    let tl = back.design_timeline.unwrap();
    let baked = tl.steps[1].baked.as_deref().unwrap();
    assert!(baked.selection.as_ref().is_some_and(Option::is_some), "the baked selection is kept");
    assert!(baked.color.as_ref().is_some_and(|c| c.2.is_some()), "so is the profile it assigned");
    assert!(tl.steps[0].checkpoint.0.is_some(), "replay resumes from the saved document");
}

#[test]
fn a_file_without_a_timeline_stays_format_1() {
    let mut doc = with_timeline();
    doc.design_timeline = None;
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    assert_eq!(stored_version(&bytes), u64::from(BASE_FORMAT_VERSION));
}

#[test]
fn a_timeline_whose_ids_cannot_be_kept_opens_without_it_and_says_so() {
    let mut doc = with_timeline();
    let mut tl = (**doc.design_timeline.as_ref().unwrap()).clone();
    // An unscoped id far ahead of the counter makes the loader remap, here only in the base.
    let far = photocraft_doc::LayerId((1 << 52) + 50_000_000);
    Arc::make_mut(&mut tl.base).layers[0].id = far;
    doc.design_timeline = Some(Arc::new(tl));
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    // Remapped, the steps would name layers they can't find: the image opens without them, and
    // the user hears of it before a save erases the history.
    let (back, warnings) = load_from_bytes_reporting(&bytes, &LoadOptions::default()).unwrap();
    assert!(back.design_timeline.is_none());
    assert_ne!(back.layers[0].id, doc.layers[0].id, "the document is remapped with its timeline");
    assert!(warnings.iter().any(|w| w.contains("design timeline")), "{warnings:?}");
    // A load that can't show the warning refuses rather than let a save erase the history.
    assert!(matches!(load_from_bytes(&bytes), Err(FormatError::Unsupported(_))));
    // Without a timeline the same ids are remapped with nothing to report.
    doc.design_timeline = None;
    let (_, warnings) = load_from_bytes_reporting(&save_to_bytes(&doc, &SaveOptions::default()).unwrap(), &LoadOptions::default()).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn a_timeline_keyed_to_a_step_near_the_limit_keeps_its_ids() {
    let mut doc = with_timeline();
    let mut tl = (**doc.design_timeline.as_ref().unwrap()).clone();
    let late = {
        let _scope = IdScope::new(photocraft_doc::MAX_STEP - 1).enter();
        photocraft_doc::LayerId::fresh()
    };
    Arc::make_mut(&mut tl.base).layers[0].id = late;
    doc.design_timeline = Some(Arc::new(tl));
    let (back, warnings) = load_from_bytes_reporting(&save_to_bytes(&doc, &SaveOptions::default()).unwrap(), &LoadOptions::default()).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(back.design_timeline.expect("the timeline survives").base.layers[0].id, late);
}
