use super::*;
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer};
use photocraft_engine::jobs::{JobId, Started};
use photocraft_geom::Size;

fn document(name: &str) -> Document {
    let mut doc = Document::new(name, Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
    doc.layers.push(Layer::raster("pixels", PixelFormat::RGBA8));
    doc
}

fn held(doc: Document, name: &str, question: Option<Value>) -> Held {
    Held {
        doc,
        read_only: false,
        initial_view: None,
        damage_pending: false,
        name: name.into(),
        path: Some(format!("/sources/{name}")),
        warnings: Vec::new(),
        bytes: None,
        slot: None,
        order: None,
        question,
        policy_applied: true,
        smart_object: false,
    }
}

#[test]
fn background_open_action_invalidates_saved_path_keeps_pixels_warns_and_continues_queue() {
    let mut session = photocraft_engine::Session::new();
    session.add_document(document("control"), Some("/accepted/control.pcraft".into()));
    let mut app = PhotocraftApp::new(session, crate::Services::default());
    let control = app.session.active().unwrap().doc.clone();
    let identity = {
        let state = app.session.active().unwrap();
        (state.path.clone(), state.revision, state.saved_revision, state.history.past_len())
    };
    app.run(
        "file.scripts.scriptEventsManager",
        json!({"add":{
            "event":"openDocument", "name":"Remove imported selected path",
            "steps":[["document.activate",{"document":1}],["path.delete",{"name":"Selected saved path"}]]
        }}),
    )
    .unwrap();
    arrive(&mut app, held(document("earlier"), "earlier.pcraft", Some(json!({"missing":true,"working":"sRGB"}))));
    let profile_prompt = app.ui.dialogs.iter().find(|dialog| owns(&dialog.fields)).unwrap().id;

    let mut imported = document("embedded original name");
    imported.layers[0].surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(0, 0, 2, 2), &[0.2, 0.4, 0.6, 1.]);
    let pixel = imported.layers[0].surface().unwrap().pixel(0, 0);
    imported.paths.extend(["Retained path", "Selected saved path"].map(|name| photocraft_doc::vector::NamedPath { name: name.into(), ..Default::default() }));
    let selected_layer = imported.layers[0].id;
    let view = photocraft_io::NativeInitialView {
        selected_layers: vec![selected_layer],
        active_layer: Some(selected_layer),
        selected_path: Some(1),
        ..Default::default()
    };
    view.validate(&imported).unwrap();
    // Exercise actual worker completion/application before the shell handles this open result.
    let started = app
        .session
        .start_job_with_spawn(
            photocraft_engine::jobs::OPEN_JOB,
            json!({"name":"renamed-source.xcf"}),
            "Opening renamed-source.xcf",
            false,
            true,
            move |ctx| {
                ctx.check()?;
                Ok((imported, view))
            },
            |session, (doc, view)| {
                let index = session.add_document(doc, None);
                session.active_mut().ok_or(photocraft_engine::EngineError::NoDocument)?.source_read_only = true;
                session.apply_import_initial_view(Some(view))?;
                let id = session.active().ok_or(photocraft_engine::EngineError::NoDocument)?.doc.id;
                Ok(json!({"document":index,"docId":id.0,"warnings":[],"hasInitialView":true,"initialPathSelection":1}))
            },
        )
        .unwrap();
    #[cfg(not(target_arch = "wasm32"))]
    assert!(matches!(&started, Started::Job(_)), "the fixture must complete through the background worker path");
    let (job, result) = match started {
        Started::Job(job) => (job, app.session.wait_job(job).unwrap()),
        Started::Done(result) => (JobId(u64::MAX), result),
    };
    let imported_id = app.session.active().unwrap().doc.id;
    crate::jobs_ui::after_open(&mut app, control.id, "control.pcraft", None, &[], None).unwrap();
    assert_eq!(app.session.active().unwrap().doc.id, imported_id);
    assert_eq!(app.session.active().unwrap().doc.paths.iter().map(|path| path.name.as_str()).collect::<Vec<_>>(), ["Retained path"]);
    app.session.execute("file.scripts.scriptEventsManager", json!({"enabled":false})).unwrap();

    let source_path = "/sources/renamed-source.xcf";
    crate::jobs_ui::finish_open_tab(
        &mut app,
        &crate::jobs_ui::OpenTab { job, name: "renamed-source.xcf".into(), path: Some(source_path.into()), slot: None, order: None },
        &result,
    )
    .unwrap();
    let pending = app.held_opens.back().unwrap();
    assert_eq!(pending.initial_view.as_ref().unwrap().selected_path, Some(1));
    assert!(pending.initial_view.as_ref().unwrap().validate(&pending.doc).unwrap_err().to_string().contains("selected path is absent"));
    let later = document("later");
    let later_id = later.id;
    arrive(&mut app, held(later, "later.pcraft", None));
    crate::dialogs::confirm(&mut app, profile_prompt).unwrap();

    assert_eq!(app.session.documents().len(), 4, "the optional view failure must retain the imported pixels and every queued document");
    let imported = app.session.documents().iter().find(|state| state.doc.id == imported_id).unwrap();
    assert_eq!(imported.doc.layers[0].surface().unwrap().pixel(0, 0), pixel);
    assert!(imported.initial_path_selection.is_none());
    assert!(imported.source_read_only);
    assert!(imported.path.is_none());
    assert_eq!(imported.history.past_len(), 0);
    assert_eq!(app.session.active().unwrap().doc.id, later_id, "the view warning does not stop later queued opens");
    assert!(app.held_opens.is_empty());
    assert!(app.prompt_order.is_empty());
    let state = app.session.documents().iter().find(|state| state.doc.id == control.id).unwrap();
    assert!(Arc::ptr_eq(&control, &state.doc), "an optional view failure cannot change an accepted document");
    assert_eq!((state.path.clone(), state.revision, state.saved_revision, state.history.past_len()), identity);
    assert!(app.ui.recent_files.iter().any(|path| path == source_path), "the pixels opened successfully from this source");
    assert!(app.ui.notices.iter().all(|notice| !notice.error));
    let warnings: Vec<_> =
        app.ui.notices.iter().filter(|notice| notice.lines.iter().any(|line| line.contains("Native initial view was not applied"))).collect();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].title.contains("Opened renamed-source.xcf"), "{}", warnings[0].title);
    assert!(warnings[0].lines.iter().any(|line| line.contains("selected path is absent")));
    assert!(!warnings[0].title.contains("embedded original name"), "report the source filename rather than its embedded document name");
    let notices = app.ui.notices.clone();
    app.sync_views();
    assert_eq!(app.ui.notices, notices, "the rejected requeued hint is consumed once");
}
