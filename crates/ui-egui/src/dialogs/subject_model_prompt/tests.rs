use super::*;
use craft::ui::dialog::{DialogId, StackDialog as _};
use photocraft_engine::Session;
use photocraft_engine::subject_model::{ModelError, ModelOffer, SubjectImage, SubjectModel};
use std::sync::{Arc, Mutex};

/// A model that is offered until it is installed, covering all it sees.
#[derive(Default)]
struct StandIn {
    installed: Mutex<bool>,
}

impl SubjectModel for StandIn {
    fn status(&self) -> ModelStatus {
        if *self.installed.lock().unwrap() {
            ModelStatus::Ready { model: "Stand-in".into() }
        } else {
            ModelStatus::Offer(ModelOffer {
                model: "Stand-in".into(),
                download_bytes: 367_120_444,
                folder: "D:\\Models\\inspyrenet".into(),
                terms: "The terms".into(),
            })
        }
    }
    fn matte(&self, image: &SubjectImage, _: &dyn Fn() -> bool) -> Result<Vec<u8>, ModelError> {
        Ok(vec![255; (image.width * image.height) as usize])
    }
    fn install(&self, _: &dyn Fn(f32, &str), _: &dyn Fn() -> bool) -> Result<(), ModelError> {
        *self.installed.lock().unwrap() = true;
        Ok(())
    }
}

fn make_app(model: Option<Arc<StandIn>>) -> Box<PhotocraftApp> {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":16,"height":16})).unwrap();
    let model = model.map(|m| m as photocraft_engine::subject_model::SharedSubjectModel);
    PhotocraftApp::new(s, crate::Services { subject_model: model, ..Default::default() })
}

fn mask_exists(app: &PhotocraftApp) -> bool {
    let d = app.session.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().mask.is_some()
}

/// The open offer.
fn offer(app: &PhotocraftApp) -> Option<DialogId> {
    alerts::prompt_tagged::<SubjectModelAsk>(app)
}

/// Answers the offer `id` with `choice` (and "Don't show again" as `skip`) the way `ui.dialog.confirm`
/// does, so the answer acts inside the call.
fn answer_with(app: &mut PhotocraftApp, id: DialogId, choice: &str, skip: bool) -> Result<(), String> {
    let ctx = app.ctx().clone();
    app.framework.dialogs.set_json_field(&ctx, id, "choice", json!(choice)).unwrap();
    app.framework.dialogs.set_json_field(&ctx, id, "skip", json!(skip)).unwrap();
    crate::dialogs::confirm(app, &ctx, id)
}

#[test]
fn without_a_model_nothing_is_offered() {
    let mut app = make_app(None);
    let _ = app.run("select.subject", json!({})).unwrap();
    assert!(offer(&app).is_none());
}

#[test]
fn the_offer_waits_for_an_answer_and_built_in_is_the_default() {
    let mut app = make_app(Some(Arc::default()));
    let result = app.run("layer.removeBackground", json!({})).unwrap();
    assert_eq!(result["confirmationPending"], true);
    assert!(offer(&app).is_some());
    let prompt = app.framework.dialogs.get::<StandardPrompt>().unwrap();
    let fields = prompt.json_fields().unwrap();
    assert_eq!(fields["choice"], json!("proceed"), "Enter takes the built-in method, never the terms");
    assert_eq!(fields["skip"], json!(false));
    assert!(!mask_exists(&app), "nothing runs before the answer");
}

#[test]
fn cancel_runs_nothing_and_asks_again() {
    let mut app = make_app(Some(Arc::default()));
    let _ = app.run("layer.removeBackground", json!({})).unwrap();
    let id = offer(&app).unwrap();
    answer_with(&mut app, id, "cancel", false).unwrap();
    assert!(offer(&app).is_none());
    assert!(!mask_exists(&app));
    let _ = app.run("layer.removeBackground", json!({})).unwrap();
    assert!(offer(&app).is_some());
}

#[test]
fn download_installs_the_model_and_goes_on_with_the_command() {
    let model = Arc::new(StandIn::default());
    let mut app = make_app(Some(model.clone()));
    let _ = app.run("layer.removeBackground", json!({})).unwrap();
    let id = offer(&app).unwrap();
    answer_with(&mut app, id, "alternate", false).unwrap();
    assert!(*model.installed.lock().unwrap());
    assert!(mask_exists(&app));
}

#[test]
fn a_background_download_locks_the_document_and_then_goes_on_with_the_command() {
    let mut app = make_app(Some(Arc::default()));
    app.background_jobs = true;
    let _ = app.run("layer.removeBackground", json!({})).unwrap();
    let id = offer(&app).unwrap();
    answer_with(&mut app, id, "alternate", false).unwrap();
    assert_eq!(app.session.active_job().unwrap().command, INSTALL_CMD, "this is what opens the progress window");
    let ctx = app.ctx().clone();
    for _ in 0..2000 {
        crate::jobs_ui::tick(&mut app, &ctx);
        if mask_exists(&app) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("the command did not go on after the download");
}

#[test]
fn the_built_in_method_leaves_the_model_out_and_can_be_remembered() {
    let model = Arc::new(StandIn::default());
    let mut app = make_app(Some(model.clone()));
    let _ = app.run("select.subject", json!({})).unwrap();
    let id = offer(&app).unwrap();
    answer_with(&mut app, id, "proceed", true).unwrap();
    assert!(!*model.installed.lock().unwrap());
    let _ = app.run("select.subject", json!({})).unwrap();
    assert!(offer(&app).is_none(), "it does not ask again");
}

#[test]
fn the_offer_stays_with_the_document_it_was_asked_on() {
    for choice in ["proceed", "alternate"] {
        let model = Arc::new(StandIn::default());
        let mut app = make_app(Some(model.clone()));
        let _ = app.run("layer.removeBackground", json!({})).unwrap();
        let id = offer(&app).unwrap();
        app.session.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        answer_with(&mut app, id, choice, false).unwrap();
        assert!(app.ui.status_error, "{choice}: the change is reported");
        assert!(!mask_exists(&app));
        assert!(!*model.installed.lock().unwrap(), "{choice}");
    }
}

#[test]
fn a_command_that_fails_after_the_download_keeps_its_error() {
    let mut app = make_app(Some(Arc::default()));
    let document = app.session.active().unwrap().doc.id;
    let layer = app.session.active().unwrap().active_layer.map(|id| id.0);
    app.jobs.subject_model_pending =
        Some(Pending { command: "layer.removeBackground".into(), params: json!({"layer": 999_999}), target: Target { document: document.0, layer }, job: 0 });
    let event = install_event(0, document, photocraft_engine::jobs::JobOutcome::Done(json!({"installed": true})));
    assert!(on_job_event(&mut app, &event), "the caller keeps the error");
    assert!(app.ui.status_error);
}

/// The end of the download job `id`.
fn install_event(id: u64, document: photocraft_doc::DocId, outcome: photocraft_engine::jobs::JobOutcome) -> photocraft_engine::jobs::JobEvent {
    photocraft_engine::jobs::JobEvent {
        id: photocraft_engine::jobs::JobId(id),
        command: INSTALL_CMD.into(),
        label: "Download Subject Model".into(),
        document: Some(document),
        outcome,
        error: None,
    }
}

#[test]
fn only_its_own_download_goes_on_with_the_command_and_a_failed_one_drops_it() {
    let mut app = make_app(Some(Arc::default()));
    let document = app.session.active().unwrap().doc.id;
    let layer = app.session.active().unwrap().active_layer.map(|id| id.0);
    app.jobs.subject_model_pending =
        Some(Pending { command: "layer.removeBackground".into(), params: json!({}), target: Target { document: document.0, layer }, job: 7 });
    let done = || photocraft_engine::jobs::JobOutcome::Done(json!({"installed": true}));
    assert!(!on_job_event(&mut app, &install_event(8, document, done())));
    assert!(app.jobs.subject_model_pending.is_some(), "another download leaves it waiting");
    assert!(!on_job_event(&mut app, &install_event(7, document, photocraft_engine::jobs::JobOutcome::Failed("offline".into()))));
    assert!(app.jobs.subject_model_pending.is_none(), "a failed download drops it");
    assert!(!on_job_event(&mut app, &install_event(9, document, done())));
    assert!(!mask_exists(&app), "a later download runs nothing");
}

#[test]
fn automation_is_not_asked() {
    let mut app = make_app(Some(Arc::default()));
    app.automation_input = true;
    let _ = app.run("select.subject", json!({})).unwrap();
    assert!(offer(&app).is_none());
}

#[test]
fn an_explicit_method_is_not_asked() {
    let mut app = make_app(Some(Arc::default()));
    let _ = app.run("select.subject", json!({"method": "classic"})).unwrap();
    assert!(offer(&app).is_none());
}
