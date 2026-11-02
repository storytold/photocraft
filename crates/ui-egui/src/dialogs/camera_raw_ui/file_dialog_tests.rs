use super::*;
use crate::file_dialog::{FileDialogAnswer, FileDialogReply, FileDialogRequest};
use std::cell::RefCell;
use std::rc::Rc;

type Requests = Rc<RefCell<Vec<(FileDialogRequest, egui::ViewportId, FileDialogReply)>>>;

fn app() -> (PhotocraftApp, Requests) {
    let requests: Requests = Rc::default();
    let recorded = requests.clone();
    let services = crate::Services {
        file_dialog: Some(Box::new(move |request, parent, reply| recorded.borrow_mut().push((request, parent.viewport, reply)))),
        write: Some(Box::new(|_, _| Ok(()))),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
    app.run("edit.fill", json!({"color": "#808080"})).unwrap();
    let ctx = egui::Context::default();
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": 0.25}}})).unwrap().unwrap();
    (app, requests)
}

#[test]
fn settings_pickers_keep_the_requesting_window_when_polled_from_root() {
    for embedded in [false, true] {
        for save in [false, true] {
            let (mut app, requests) = app();
            let ctx = egui::Context::default();
            ctx.set_embed_viewports(embedded);
            let doc = app.session.active().unwrap().doc.clone();
            let history = app.session.active().unwrap().history.past_len();
            if save { sidecar::save_settings(&mut app) } else { sidecar::load_settings(&mut app) }.unwrap();
            assert!(requests.borrow().is_empty(), "the picker must remain asynchronous");
            assert_eq!(ctx.viewport_id(), egui::ViewportId::ROOT);
            app.poll_file_dialog(&ctx, None);
            let (request, parent, reply) = requests.borrow_mut().pop().unwrap();
            assert_eq!(parent, if embedded { egui::ViewportId::ROOT } else { viewport_id() });
            if save {
                assert!(matches!(request, FileDialogRequest::Save { .. }));
            } else {
                assert!(matches!(request, FileDialogRequest::Open { extensions: Some(exts), .. } if exts == ["xmp"]));
            }
            app.poll_file_dialog(&ctx, None);
            assert!(requests.borrow().is_empty(), "later frames must not open a second picker");
            reply.send(None);
            app.poll_file_dialog(&ctx, None);
            assert!(!app.file_dialog_open());
            assert_eq!(app.camera_raw.as_ref().unwrap().params.exposure, 0.25);
            assert!(std::sync::Arc::ptr_eq(&doc, &app.session.active().unwrap().doc));
            assert_eq!(app.session.active().unwrap().history.past_len(), history);
            assert!(!app.ui.status_error, "Cancel is not an error");
        }
    }
}

#[test]
fn failed_or_dropped_picker_and_bad_xmp_keep_camera_raw_and_its_document() {
    let answers =
        [Some(FileDialogAnswer::Unavailable("test picker unavailable".into())), Some(FileDialogAnswer::Contents("bad.xmp".into(), b"<broken".to_vec())), None];
    for answer in answers {
        let (mut app, requests) = app();
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        let doc = app.session.active().unwrap().doc.clone();
        let history = app.session.active().unwrap().history.past_len();
        sidecar::load_settings(&mut app).unwrap();
        app.poll_file_dialog(&ctx, None);
        let (_, parent, reply) = requests.borrow_mut().pop().unwrap();
        assert_eq!(parent, viewport_id());
        if let Some(answer) = answer {
            reply.send(Some(answer));
        } else {
            drop(reply);
        }
        app.poll_file_dialog(&ctx, None);
        assert!(!app.file_dialog_open());
        assert_eq!(app.camera_raw.as_ref().unwrap().params.exposure, 0.25);
        assert!(std::sync::Arc::ptr_eq(&doc, &app.session.active().unwrap().doc));
        assert_eq!(app.session.active().unwrap().history.past_len(), history);
    }
}

#[test]
fn rejected_settings_request_does_not_reparent_an_existing_root_picker() {
    let (mut app, requests) = app();
    let ctx = egui::Context::default();
    ctx.set_embed_viewports(false);
    app.open_dialog_file().unwrap();
    assert!(sidecar::load_settings(&mut app).is_err());
    assert!(sidecar::save_settings(&mut app).is_err());
    app.poll_file_dialog(&ctx, None);
    let (_, parent, reply) = requests.borrow_mut().pop().unwrap();
    assert_eq!(parent, egui::ViewportId::ROOT);
    reply.send(None);
    app.poll_file_dialog(&ctx, None);
}

#[test]
fn queued_and_open_settings_pickers_keep_their_owner_alive_until_consumed() {
    for already_shown in [false, true] {
        for save in [false, true] {
            let (mut app, requests) = app();
            let ctx = egui::Context::default();
            ctx.set_embed_viewports(false);
            let doc = app.session.active().unwrap().doc.clone();
            let history = app.session.active().unwrap().history.past_len();
            if save { sidecar::save_settings(&mut app) } else { sidecar::load_settings(&mut app) }.unwrap();
            if already_shown {
                app.poll_file_dialog(&ctx, None);
            }

            for params in [json!({}), json!({"ui": {"cancel": true}}), json!({"ui": {"commit": true}}), json!({"ui": {"set": {"exposure": 2}}})] {
                assert!(menu(&mut app, &ctx, "filter.cameraRaw", &params).unwrap().is_err());
            }
            assert!(controls::commit(&mut app).is_err());
            assert!(open::open(&mut app, &ctx).is_err());
            open::cancel(&mut app);
            let mut output = ctx.run_ui(Default::default(), |ui| {
                ui.ctx().show_viewport_deferred(viewport_id(), Default::default(), |_, _| {});
                controls::request_close(&mut app, ui.ctx());
            });
            assert!(output.viewport_output.get(&viewport_id()).unwrap().commands.contains(&egui::ViewportCommand::CancelClose));
            output.textures_delta.clear();
            assert_eq!(app.camera_raw.as_ref().unwrap().params.exposure, 0.25);
            assert!(std::sync::Arc::ptr_eq(&doc, &app.session.active().unwrap().doc));
            assert_eq!(app.session.active().unwrap().history.past_len(), history);

            app.poll_file_dialog(&ctx, None);
            let (_, _, reply) = requests.borrow_mut().pop().unwrap();
            let loaded = CameraRaw { exposure: 0.75, ..Default::default() };
            reply.send(Some(if save {
                FileDialogAnswer::SaveTo("look.xmp".into())
            } else {
                FileDialogAnswer::Contents("look.xmp".into(), photocraft_io::camera_raw_xmp::save(&loaded).into_bytes())
            }));
            assert!(controls::commit(&mut app).is_err(), "a sent reply still needs to be consumed");
            app.poll_file_dialog(&ctx, None);
            assert_eq!(app.camera_raw.as_ref().unwrap().params.exposure, if save { 0.25 } else { 0.75 });
            controls::commit(&mut app).unwrap();
            assert!(app.camera_raw.is_none());
            open::open(&mut app, &ctx).unwrap();
            open::cancel(&mut app);
            assert!(app.camera_raw.is_none(), "normal closing works again after the picker finishes");
        }
    }
}

#[test]
fn normalized_settings_save_keeps_its_replacement_question_with_camera_raw() {
    for replace in [false, true] {
        let (mut app, requests) = app();
        let written = Rc::new(RefCell::new(Vec::new()));
        let log = written.clone();
        app.services.write = Some(Box::new(move |path, _| {
            log.borrow_mut().push(path.to_string());
            Ok(())
        }));
        app.services.file_stamp = Some(Box::new(|p| (p == "Look.xmp").then_some(crate::FileStamp { len: 1, modified: None })));
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        sidecar::save_settings(&mut app).unwrap();
        app.poll_file_dialog(&ctx, None);
        let (_, _, reply) = requests.borrow_mut().pop().unwrap();
        reply.send(Some(FileDialogAnswer::SaveTo("Look.XMP".into())));
        app.poll_file_dialog(&ctx, None);
        assert_eq!(app.file_dialog_prompt_viewport(), Some(viewport_id()));
        let prompt = app.ui.dialogs.iter().find(|d| crate::replace_prompt::owns(&d.fields)).unwrap().id;
        assert!(written.borrow().is_empty(), "normalizing a name must never silently overwrite");
        open::cancel(&mut app);
        assert!(app.camera_raw.is_some(), "the question still belongs to this instance");
        if replace { crate::dialogs::confirm(&mut app, prompt) } else { crate::dialogs::cancel(&mut app, prompt) }.unwrap();
        assert!(!app.file_dialog_open());
        assert!(!app.ui.dialogs.iter().any(|d| d.id == prompt));
        let expected = if replace { vec!["Look.xmp".to_string()] } else { Vec::new() };
        assert_eq!(*written.borrow(), expected);
        open::cancel(&mut app);
        assert!(app.camera_raw.is_none());
    }
}
