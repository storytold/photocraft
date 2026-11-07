use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::json;

use crate::subject_model::{ModelError, ModelOffer, ModelStatus, SubjectImage, SubjectModel};
use crate::{EngineError, Session};

/// A model that covers the left half of what it is shown, with a soft column at its edge.
struct LeftHalf {
    status: Mutex<ModelStatus>,
    calls: AtomicUsize,
    seen: Mutex<Option<(u32, u32)>>,
    answer: Option<ModelError>,
    /// Whether a CMYK profile was active while it read.
    cmyk: Mutex<Option<bool>>,
}

impl LeftHalf {
    fn new(status: ModelStatus) -> Arc<Self> {
        Arc::new(Self { status: Mutex::new(status), calls: AtomicUsize::new(0), seen: Mutex::new(None), answer: None, cmyk: Mutex::new(None) })
    }

    fn failing(answer: ModelError) -> Arc<Self> {
        Arc::new(Self { status: Mutex::new(ready()), calls: AtomicUsize::new(0), seen: Mutex::new(None), answer: Some(answer), cmyk: Mutex::new(None) })
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl SubjectModel for LeftHalf {
    fn status(&self) -> ModelStatus {
        self.status.lock().unwrap().clone()
    }

    fn matte(&self, image: &SubjectImage, _: &dyn Fn() -> bool) -> Result<Vec<u8>, ModelError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        *self.seen.lock().unwrap() = Some((image.width, image.height));
        *self.cmyk.lock().unwrap() = Some(photocraft_color::convert::active_cmyk_space().is_some());
        assert_eq!(image.rgb.len(), image.width as usize * image.height as usize * 3);
        if let Some(e) = &self.answer {
            return Err(e.clone());
        }
        let half = image.width / 2;
        Ok((0..image.width * image.height)
            .map(|i| match i % image.width {
                x if x < half => 255,
                x if x == half => SOFT,
                _ => 0,
            })
            .collect())
    }

    fn install(&self, progress: &dyn Fn(f32, &str), _: &dyn Fn() -> bool) -> Result<(), ModelError> {
        progress(0.5, "Downloading");
        *self.status.lock().unwrap() = ready();
        Ok(())
    }
}

/// The coverage [`LeftHalf`] answers on the column at its edge.
const SOFT: u8 = 102;

fn ready() -> ModelStatus {
    ModelStatus::Ready { model: "Stand-in".into() }
}

fn offer() -> ModelStatus {
    ModelStatus::Offer(ModelOffer { model: "Stand-in".into(), download_bytes: 1000, folder: "Models".into(), terms: "Terms".into() })
}

/// A document with a mid-gray disc on white: the built-in method finds something, and the model
/// answers its half.
fn session(model: Option<Arc<LeftHalf>>) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    s.set_subject_model(model.map(|m| m as crate::subject_model::SharedSubjectModel));
    s
}

fn mask_value(s: &Session, x: i32, y: i32) -> f32 {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().mask.as_ref().unwrap().value(x, y)
}

#[test]
fn remove_background_uses_a_ready_model_and_keeps_its_mask_as_it_is() {
    let model = LeftHalf::new(ready());
    let mut s = session(Some(model.clone()));
    let result = s.execute("layer.removeBackground", json!({})).unwrap();
    assert_eq!(result["method"], "model");
    assert_eq!(model.calls(), 1);
    assert_eq!(*model.seen.lock().unwrap(), Some((64, 48)));
    assert_eq!(mask_value(&s, 10, 10), 1.0);
    assert_eq!(mask_value(&s, 50, 10), 0.0);
    // No edge refinement on top: the boundary stays where the model put it, soft where it is soft.
    assert_eq!(mask_value(&s, 31, 10), 1.0);
    assert!((mask_value(&s, 32, 10) - f32::from(SOFT) / 255.0).abs() < 0.01, "{}", mask_value(&s, 32, 10));
    assert_eq!(mask_value(&s, 33, 10), 0.0);
}

#[test]
fn select_subject_keeps_the_models_soft_coverage() {
    let mut s = session(Some(LeftHalf::new(ready())));
    s.execute("select.subject", json!({})).unwrap();
    let selection = s.active().unwrap().doc.selection.clone().unwrap();
    assert!((selection.sample_channel(32, 10, 0) - f32::from(SOFT) / 255.0).abs() < 0.01);
}

#[test]
fn remove_background_undoes_in_one_step_with_a_model() {
    let mut s = session(Some(LeftHalf::new(ready())));
    let before = s.active().unwrap().history.past_len();
    s.execute("layer.removeBackground", json!({})).unwrap();
    assert_eq!(s.active().unwrap().history.past_len(), before + 1);
    s.undo();
    let d = s.active().unwrap();
    assert!(d.doc.layer(d.active_layer.unwrap()).unwrap().mask.is_none());
}

#[test]
fn select_subject_uses_a_ready_model_and_remembers_its_answer() {
    let model = LeftHalf::new(ready());
    let mut s = session(Some(model.clone()));
    s.execute("select.subject", json!({})).unwrap();
    let selection = s.active().unwrap().doc.selection.clone().unwrap();
    assert_eq!(selection.sample_channel(10, 10, 0), 1.0);
    assert_eq!(selection.sample_channel(50, 10, 0), 0.0);
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("select.subject", json!({})).unwrap();
    assert_eq!(model.calls(), 1, "the same pixels are not read again");
    // Another model (chosen in another Craft app) answers for itself.
    *model.status.lock().unwrap() = ModelStatus::Ready { model: "Another".into() };
    s.execute("select.subject", json!({})).unwrap();
    assert_eq!(model.calls(), 2);
}

#[test]
fn the_selection_reports_the_bounds_of_what_the_model_covered() {
    let mut s = session(Some(LeftHalf::new(ready())));
    assert_eq!(s.execute("select.subject", json!({})).unwrap()["bounds"], json!([0, 0, 33, 48]));
}

#[test]
fn tight_cuts_a_mask_to_what_it_covers_at_the_areas_place() {
    let mut mask = vec![0u8; 5 * 4];
    mask[5 + 2] = 255;
    mask[2 * 5 + 3] = 10;
    let region = super::tight(photocraft_geom::Rect::new(10, 20, 15, 24), &mask);
    assert_eq!(region.bbox, photocraft_geom::Rect::new(12, 21, 14, 23));
    assert_eq!(region.mask, vec![255, 0, 0, 10]);
}

#[test]
fn a_built_in_answer_is_not_taken_for_the_models() {
    let model = LeftHalf::new(ready());
    let mut s = session(Some(model.clone()));
    let _ = s.execute("select.subject", json!({"method": "classic"})).unwrap();
    assert_eq!(model.calls(), 0);
    s.execute("select.subject", json!({})).unwrap();
    assert_eq!(model.calls(), 1);
}

#[test]
fn a_model_that_is_not_ready_is_left_alone_unless_asked_for() {
    for status in [offer(), ModelStatus::Unavailable { reason: "It is off.".into() }] {
        let model = LeftHalf::new(status);
        let mut s = session(Some(model.clone()));
        let _ = s.execute("select.subject", json!({})).unwrap();
        assert_eq!(model.calls(), 0);
        assert!(s.execute("select.subject", json!({"method": "model"})).is_err());
        assert!(s.execute("layer.removeBackground", json!({"method": "model"})).is_err());
        assert_eq!(model.calls(), 0);
    }
}

#[test]
fn without_a_model_the_built_in_method_is_used_and_a_model_cannot_be_asked_for() {
    let mut s = session(None);
    let _ = s.execute("select.subject", json!({})).unwrap();
    assert!(s.execute("select.subject", json!({"method": "model"})).is_err());
    assert!(s.execute(super::INSTALL_CMD, json!({"acceptTerms": true})).is_err());
}

#[test]
fn a_method_that_does_not_exist_is_refused() {
    let mut s = session(None);
    for method in [json!("fast"), json!("")] {
        assert!(matches!(s.execute("select.subject", json!({"method": method})), Err(EngineError::BadParams { .. })));
        assert!(matches!(s.execute("layer.removeBackground", json!({"method": method})), Err(EngineError::BadParams { .. })));
    }
}

#[test]
fn a_failing_model_reports_its_words_and_leaves_the_document() {
    let mut s = session(Some(LeftHalf::failing(ModelError::Failed("The card ran out of memory.".into()))));
    let error = s.execute("layer.removeBackground", json!({})).unwrap_err();
    assert!(error.to_string().contains("ran out of memory"), "{error}");
    let d = s.active().unwrap();
    assert!(d.doc.layer(d.active_layer.unwrap()).unwrap().mask.is_none());
    assert!(s.execute("select.subject", json!({})).is_err());
    assert!(s.active().unwrap().doc.selection.is_none());
}

#[test]
fn a_cancel_inside_the_model_is_a_cancel() {
    let mut s = session(Some(LeftHalf::failing(ModelError::Cancelled)));
    assert!(matches!(s.execute("layer.removeBackground", json!({})), Err(EngineError::Cancelled)));
}

/// A model that answers the wrong number of pixels.
struct WrongSize;

impl SubjectModel for WrongSize {
    fn status(&self) -> ModelStatus {
        ready()
    }

    fn matte(&self, _: &SubjectImage, _: &dyn Fn() -> bool) -> Result<Vec<u8>, ModelError> {
        Ok(vec![255; 3])
    }

    fn install(&self, _: &dyn Fn(f32, &str), _: &dyn Fn() -> bool) -> Result<(), ModelError> {
        Ok(())
    }
}

#[test]
fn a_mask_of_the_wrong_size_is_refused() {
    let mut s = session(None);
    s.set_subject_model(Some(Arc::new(WrongSize)));
    assert!(s.execute("layer.removeBackground", json!({})).is_err());
}

#[test]
fn a_model_that_sees_nothing_finds_no_subject() {
    struct Empty;
    impl SubjectModel for Empty {
        fn status(&self) -> ModelStatus {
            ready()
        }
        fn matte(&self, image: &SubjectImage, _: &dyn Fn() -> bool) -> Result<Vec<u8>, ModelError> {
            Ok(vec![0; (image.width * image.height) as usize])
        }
        fn install(&self, _: &dyn Fn(f32, &str), _: &dyn Fn() -> bool) -> Result<(), ModelError> {
            Ok(())
        }
    }
    let mut s = session(None);
    s.set_subject_model(Some(Arc::new(Empty)));
    assert_eq!(s.execute("select.subject", json!({})).unwrap()["changed"], false);
    assert!(s.execute("layer.removeBackground", json!({})).unwrap_err().to_string().contains("no subject"));
}

#[test]
fn the_download_needs_the_terms_accepted_and_then_the_model_is_used() {
    let model = LeftHalf::new(offer());
    let mut s = session(Some(model.clone()));
    for params in [json!({}), json!({"acceptTerms": false}), json!({"acceptTerms": "yes"})] {
        assert!(matches!(s.execute(super::INSTALL_CMD, params), Err(EngineError::BadParams { .. })));
    }
    assert!(matches!(model.status(), ModelStatus::Offer(_)));
    assert_eq!(s.execute(super::INSTALL_CMD, json!({"acceptTerms": true})).unwrap()["installed"], true);
    assert_eq!(s.execute("layer.removeBackground", json!({})).unwrap()["method"], "model");
}

#[test]
fn the_model_reads_a_cmyk_document_in_its_own_profile() {
    let profile = photocraft_cms::synth::cmyk_profile(&photocraft_cms::synth::CmykParams {
        description: "Subject Test Uncoated".into(),
        tvi: [0.26, 0.26, 0.26, 0.3],
        grid_a2b: 5,
        grid_b2a: 9,
        ..Default::default()
    })
    .to_bytes();
    for command in ["layer.removeBackground", "select.subject"] {
        let model = LeftHalf::new(ready());
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 32, "height": 24, "mode": "cmyk", "background": "white"})).unwrap();
        s.edit("icc", |doc, _| {
            doc.icc_profile = Some(profile.clone());
            Ok(())
        })
        .unwrap();
        s.set_subject_model(Some(model.clone() as crate::subject_model::SharedSubjectModel));
        s.execute(command, json!({})).unwrap();
        assert_eq!(*model.cmyk.lock().unwrap(), Some(true), "{command}");
    }
}

#[test]
fn batch_plays_the_commands_with_the_apps_model() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.png").to_string_lossy().into_owned();
    let mut source = Session::new();
    source.execute("file.new", json!({"width": 16, "height": 12})).unwrap();
    crate::file_cmds::save_doc(&source.active().unwrap().doc, &input, None).unwrap();
    let out = dir.path().join("out");
    std::fs::create_dir_all(&out).unwrap();
    let model = LeftHalf::new(ready());
    let mut s = session(Some(model.clone()));
    let result = s
        .execute("file.automate.batch", json!({"steps": [["select.subject", {"method": "model"}]], "input": [input], "output": out.to_string_lossy()}))
        .unwrap();
    assert_eq!(result["errors"], json!([]), "{result}");
    assert_eq!(model.calls(), 1);
}

#[test]
fn a_model_already_there_is_not_downloaded_again() {
    let mut s = session(Some(LeftHalf::new(ready())));
    assert!(s.execute(super::INSTALL_CMD, json!({"acceptTerms": true})).is_err());
}
