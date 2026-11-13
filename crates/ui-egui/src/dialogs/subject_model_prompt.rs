//! The one-time offer of the subject model: Remove Background and Select Subject ask before their
//! first use, because the download is 367 MB and has terms a person agrees to. Download runs the
//! command when the model is in; Use Built-in Method runs it without the model.
use crate::{PhotocraftApp, state::DialogKind};
use photocraft_engine::subject_model::ModelStatus;
use photocraft_engine::subject_model_cmds::INSTALL_CMD;
use serde_json::{Map, Value, json};
const MARK: &str = "__subjectModel";
const PREF: &str = "select.subjectModel.offer";

/// The command waiting for the model's download.
pub struct Pending {
    command: String,
    params: Value,
    target: Target,
    /// The download's job: only its own end goes on with the command, or drops it.
    job: u64,
}

/// What a command was asked on: the document and its active layer then.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Target {
    document: u64,
    layer: Option<u64>,
}

/// The active document and layer now.
fn target(app: &PhotocraftApp) -> Option<Target> {
    let d = app.session.active()?;
    Some(Target { document: d.doc.id.0, layer: d.active_layer.map(|id| id.0) })
}

/// The document and layer the offer was asked on.
fn recorded(fields: &Map<String, Value>) -> Option<Target> {
    let document = fields.get("__document")?.as_u64()?;
    let layer = match fields.get("__layer")? {
        Value::Null => None,
        v => Some(v.as_u64()?),
    };
    Some(Target { document, layer })
}

/// The message when the document or layer changed under the offer.
fn changed() -> String {
    tl!("The document or layer changed. Please run the command again.").into()
}

/// Whether `fields` are this offer's dialog.
pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

/// Opens the offer before Remove Background or Select Subject when the model can be downloaded
/// and the person has not been asked to stop seeing it. `None` lets the command run as asked.
pub fn intercept(app: &mut PhotocraftApp, command: &str, params: &Value) -> Option<Result<Value, String>> {
    if app.automation_input
        || !matches!(command, "layer.removeBackground" | "select.subject")
        || params.get("method").is_some()
        || app.session.prefs().dialogs.get(PREF).and_then(|v| v.get("skip")).and_then(Value::as_bool) == Some(true)
        || !crate::menus::is_enabled(app, command)
    {
        return None;
    }
    let ModelStatus::Offer(offer) = app.session.subject_model()?.status() else { return None };
    let asked_on = target(app)?;
    if app.ui.dialogs.iter().any(|d| owns(&d.fields)) {
        return Some(Ok(json!({"confirmationPending": true})));
    }
    let size = (offer.download_bytes / 1_000_000).to_string();
    let message = crate::i18n::fmt(
        tl!(
            "{model} finds subjects, hair and fur much better than the built-in method. It runs on this computer, so your pictures stay there. It needs a one-time download of {size} MB."
        ),
        &[("model", &offer.model), ("size", &size)],
    );
    let folder = if offer.folder.is_empty() { String::new() } else { crate::i18n::fmt(tl!("It will be saved in {folder}."), &[("folder", &offer.folder)]) };
    let fields = json!({MARK: true, "__command": command, "__label": "Subject Model", "__document": asked_on.document, "__layer": asked_on.layer, "message": message, "folder": folder, "terms": offer.terms, "params": params, "skip": false});
    let id = app.ui.open_dialog(DialogKind::Command, fields.as_object().cloned().unwrap_or_default());
    Some(Ok(json!({"dialog": id, "confirmationPending": true})))
}

/// The offer: what the model does, where it goes, its terms and the checkbox.
pub fn body(ui: &mut egui::Ui, fields: &mut Map<String, Value>) {
    ui.set_max_width(460.0);
    ui.label(fields.get("message").and_then(Value::as_str).unwrap_or_default());
    if let Some(folder) = fields.get("folder").and_then(Value::as_str).filter(|folder| !folder.is_empty()) {
        ui.add_space(4.0);
        ui.label(folder);
    }
    ui.add_space(8.0);
    egui::ScrollArea::vertical().max_height(120.0).show(ui, |ui| {
        ui.small(fields.get("terms").and_then(Value::as_str).unwrap_or_default());
    });
    ui.add_space(8.0);
    let mut skip = fields.get("skip").and_then(Value::as_bool).unwrap_or(false);
    ui.checkbox(&mut skip, tl!("Don't offer the download again"));
    fields.insert("skip".into(), json!(skip));
}

/// Runs the waiting command without the model, or starts the download it waits for.
pub fn confirm(app: &mut PhotocraftApp, fields: &Map<String, Value>) -> Result<Value, String> {
    let command = fields.get("__command").and_then(Value::as_str).ok_or(tl!("The document is closed"))?;
    if !matches!(command, "layer.removeBackground" | "select.subject") {
        return Err(tl!("The document is closed").into());
    }
    let params = fields.get("params").cloned().unwrap_or_else(|| json!({}));
    // A background open can make another document active while the dialog is open.
    let asked_on = recorded(fields).filter(|asked_on| target(app) == Some(*asked_on)).ok_or_else(changed)?;
    if fields.get("choice").and_then(Value::as_str) == Some("builtIn") {
        if fields.get("skip").and_then(Value::as_bool) == Some(true) {
            app.session.edit_prefs(|prefs| {
                prefs.dialogs.insert(PREF.into(), json!({"skip": true}));
            });
            if let Err(error) = crate::prefs_ui::save_preferences(app) {
                app.ui.status = error;
                app.ui.status_error = true;
            }
        }
        let mut params = params;
        if let Some(map) = params.as_object_mut() {
            map.insert("method".into(), json!("classic"));
        }
        return app.run_command(command, params);
    }
    let started = app.run_command(INSTALL_CMD, json!({"acceptTerms": true}))?;
    if started.get("pending").and_then(Value::as_bool) == Some(true) {
        if let Some(job) = started.get("job").and_then(Value::as_u64) {
            app.jobs.subject_model_pending = Some(Pending { command: command.into(), params, target: asked_on, job });
        }
        return Ok(started);
    }
    app.run_command(command, params)
}

/// The download ended: the command it waited for goes on, on the same document and layer. `true`
/// when that failed and the error is reported, so the download's own message must not replace it.
pub fn on_job_event(app: &mut PhotocraftApp, e: &photocraft_engine::jobs::JobEvent) -> bool {
    if e.command != INSTALL_CMD || app.jobs.subject_model_pending.as_ref().is_none_or(|p| p.job != e.id.0) {
        return false;
    }
    // A failed or cancelled download drops the command with it.
    let Some(pending) = app.jobs.subject_model_pending.take() else { return false };
    if !matches!(e.outcome, photocraft_engine::jobs::JobOutcome::Done(_)) {
        return false;
    }
    let result = if target(app) == Some(pending.target) { app.run_command(&pending.command, pending.params) } else { Err(changed()) };
    match result {
        Ok(_) => false,
        Err(error) => {
            crate::notices::error(app, error);
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    fn make_app(model: Option<Arc<StandIn>>) -> PhotocraftApp {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":16,"height":16})).unwrap();
        let model = model.map(|m| m as photocraft_engine::subject_model::SharedSubjectModel);
        PhotocraftApp::new(s, crate::Services { subject_model: model, ..Default::default() })
    }

    fn mask_exists(app: &PhotocraftApp) -> bool {
        let d = app.session.active().unwrap();
        d.doc.layer(d.active_layer.unwrap()).unwrap().mask.is_some()
    }

    #[test]
    fn without_a_model_nothing_is_offered() {
        let mut app = make_app(None);
        let _ = app.run("select.subject", json!({})).unwrap();
        assert!(app.ui.dialogs.is_empty());
    }

    #[test]
    fn the_offer_names_the_size_and_shows_the_terms() {
        let mut app = make_app(Some(Arc::default()));
        let result = app.run("layer.removeBackground", json!({})).unwrap();
        assert_eq!(result["confirmationPending"], true);
        let fields = &app.ui.dialogs[0].fields;
        assert!(owns(fields));
        assert!(fields["message"].as_str().unwrap().contains("367 MB"));
        assert!(fields["folder"].as_str().unwrap().contains("D:\\Models\\inspyrenet"));
        assert_eq!(fields["terms"], "The terms");
        assert!(!mask_exists(&app), "nothing runs before the answer");
    }

    #[test]
    fn cancel_runs_nothing_and_asks_again() {
        let mut app = make_app(Some(Arc::default()));
        let dialog = app.run("layer.removeBackground", json!({})).unwrap()["dialog"].as_u64().unwrap();
        app.ui.close_dialog(dialog);
        assert!(!mask_exists(&app));
        let _ = app.run("layer.removeBackground", json!({})).unwrap();
        assert_eq!(app.ui.dialogs.len(), 1);
    }

    #[test]
    fn download_installs_the_model_and_goes_on_with_the_command() {
        let model = Arc::new(StandIn::default());
        let mut app = make_app(Some(model.clone()));
        let dialog = app.run("layer.removeBackground", json!({})).unwrap()["dialog"].as_u64().unwrap();
        crate::dialogs::confirm(&mut app, dialog).unwrap();
        assert!(*model.installed.lock().unwrap());
        assert!(mask_exists(&app));
    }

    #[test]
    fn a_background_download_locks_the_document_and_then_goes_on_with_the_command() {
        let mut app = make_app(Some(Arc::default()));
        app.background_jobs = true;
        let dialog = app.run("layer.removeBackground", json!({})).unwrap()["dialog"].as_u64().unwrap();
        crate::dialogs::confirm(&mut app, dialog).unwrap();
        assert_eq!(app.session.active_job().unwrap().command, INSTALL_CMD, "this is what opens the progress window");
        let ctx = egui::Context::default();
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
        let dialog = app.run("select.subject", json!({})).unwrap()["dialog"].as_u64().unwrap();
        let fields = &mut app.ui.dialog_mut(dialog).unwrap().fields;
        fields.insert("choice".into(), json!("builtIn"));
        fields.insert("skip".into(), json!(true));
        crate::dialogs::confirm(&mut app, dialog).unwrap();
        assert!(!*model.installed.lock().unwrap());
        let _ = app.run("select.subject", json!({})).unwrap();
        assert!(app.ui.dialogs.is_empty(), "it does not ask again");
    }

    #[test]
    fn the_offer_stays_with_the_document_it_was_asked_on() {
        for choice in ["builtIn", "download"] {
            let model = Arc::new(StandIn::default());
            let mut app = make_app(Some(model.clone()));
            let dialog = app.run("layer.removeBackground", json!({})).unwrap()["dialog"].as_u64().unwrap();
            app.ui.dialog_mut(dialog).unwrap().fields.insert("choice".into(), json!(choice));
            app.session.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
            assert!(crate::dialogs::confirm(&mut app, dialog).is_err(), "{choice}");
            assert!(!mask_exists(&app));
            assert!(!*model.installed.lock().unwrap(), "{choice}");
        }
    }

    #[test]
    fn a_command_that_fails_after_the_download_keeps_its_error() {
        let mut app = make_app(Some(Arc::default()));
        let document = app.session.active().unwrap().doc.id;
        let layer = app.session.active().unwrap().active_layer.map(|id| id.0);
        app.jobs.subject_model_pending = Some(Pending {
            command: "layer.removeBackground".into(),
            params: json!({"layer": 999_999}),
            target: Target { document: document.0, layer },
            job: 0,
        });
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
        assert!(app.ui.dialogs.is_empty());
    }

    #[test]
    fn an_explicit_method_is_not_asked() {
        let mut app = make_app(Some(Arc::default()));
        let _ = app.run("select.subject", json!({"method": "classic"})).unwrap();
        assert!(app.ui.dialogs.is_empty());
    }
}
