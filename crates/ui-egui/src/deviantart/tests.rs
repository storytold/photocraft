use super::*;

#[test]
fn menu_submission_with_null_or_empty_params_only_opens_the_dialog() {
    for params in [Value::Null, json!({})] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let result = crate::menus::invoke(&mut app, &egui::Context::default(), photocraft_engine::deviantart_cmds::COMMAND, params).unwrap();
        assert!(result.get("dialog").is_some());
        assert_eq!(app.ui.dialogs.iter().filter(|d| owns(&d.fields)).count(), 1);
        assert!(app.deviantart_job.is_none());
    }
}

#[test]
fn menu_submission_rejects_non_object_parameters_instead_of_opening_a_dialog() {
    for params in [json!([]), json!(true), json!(42), json!("png")] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        assert!(crate::menus::invoke(&mut app, &egui::Context::default(), photocraft_engine::deviantart_cmds::COMMAND, params).is_err());
        assert!(app.ui.dialogs.is_empty());
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn saved_file_dialog_submits_the_pending_or_active_disk_copy() {
    use std::sync::{Arc, Mutex};
    let dir = tempfile::tempdir().unwrap();
    let active = dir.path().join("active.png");
    let pending = dir.path().join("pending.png");
    std::fs::write(&active, b"ACTIVE").unwrap();
    std::fs::write(&pending, b"PENDING").unwrap();
    for explicit in [false, true] {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let copy = seen.clone();
        let services = crate::Services {
            deviantart_submit: Some(Arc::new(move |_, _, _, _, _, name, bytes| {
                copy.lock().unwrap().push((name.to_string(), bytes.to_vec()));
                Ok(r#"{"itemid":42}"#.into())
            })),
            save_deviantart: Some(Box::new(|_| Ok(()))),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        app.run("file.new", json!({"width":2,"height":2})).unwrap();
        app.session.active_mut().unwrap().path = Some(active.display().to_string());
        app.deviantart.client_id = "99".into();
        app.deviantart.refresh_token = "synthetic".into();
        let id = open_dialog(&mut app, explicit.then(|| pending.display().to_string()));
        assert_eq!(app.ui.dialog_mut(id).unwrap().fields.get("format"), Some(&json!("file")));
        crate::dialogs::confirm(&mut app, id).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while app.deviantart_job.is_some() && std::time::Instant::now() < deadline {
            poll(&mut app, &egui::Context::default());
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(app.deviantart_job.is_none());
        assert!(!app.ui.status_error, "{}", app.ui.status);
        let expected = if explicit { ("pending.png".to_string(), b"PENDING".to_vec()) } else { ("active.png".to_string(), b"ACTIVE".to_vec()) };
        assert_eq!(*seen.lock().unwrap(), vec![expected]);
    }
}

#[test]
fn a_rejected_refresh_token_is_recognized() {
    assert!(!super::errors::login_was_rejected("The refresh token is invalid"));
    assert!(super::errors::login_was_rejected("refresh-grant-rejected: invalid_grant"));
    assert!(!super::errors::login_was_rejected("invalid_token"));
    assert!(!super::errors::login_was_rejected("invalid_grant"));
    assert!(!super::errors::login_was_rejected("HTTP 500"));
}

#[test]
fn settings_drop_the_refresh_token() {
    let text = settings_json(&Settings { client_id: "99".into(), refresh_token: "secret-token".into(), title: "Hill".into(), ..Settings::default() });
    let (disk, token) = without_token(&text).unwrap();
    assert_eq!(token, "secret-token");
    assert!(!disk.contains("secret-token"));
    assert!(disk.contains("99"));
    let back = settings_from_json(&with_token(&disk, &token));
    assert_eq!(back.refresh_token, "secret-token");
    assert_eq!(back.title, "Hill");
}

#[test]
fn tags_keep_letters_numbers_and_underscores() {
    assert_eq!(clean_tags("night sky, night, café, a-b"), vec!["night".to_string(), "sky".to_string(), "caf".to_string(), "ab".to_string()]);
}

#[test]
fn submit_body_carries_the_file_and_not_a_token() {
    let (_, body) = multipart("Hill", "notes", &["night".into()], "hill.png", b"PNG");
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("name=\"title\""));
    assert!(text.contains("Hill"));
    assert!(text.contains("name=\"tags[0]\""));
    assert!(text.contains("filename=\"hill.png\""));
    assert!(text.contains("PNG"));
    assert!(!text.contains("refresh"));
    assert_eq!(item_id(r#"{"status":"success","itemid":42}"#), Some(42));
    assert_eq!(stash_url(2730074306229899).as_deref(), Some("https://www.deviantart.com/stash/0qvqatlhzez"));
    assert_eq!(stash_url(0), None);
    let (_, quoted) = multipart("Hill", "notes", &[], "my \"best\" sketch.png", b"x");
    let quoted = String::from_utf8_lossy(&quoted);
    assert!(quoted.contains("filename=\"my _best_ sketch.png\""));
    assert!(!quoted.contains("filename=\"my \"best\""));
}

#[test]
fn artist_comments_omit_the_local_path() {
    let text = comments("hill.png", 1200);
    assert!(text.contains("hill.png"));
    assert!(text.contains("PNG"));
    assert!(!text.contains('/'));
    assert!(!text.contains("home"));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn automatic_submissions_wait_for_the_active_job_and_run_in_order() {
    use std::sync::{Arc, Mutex, mpsc};
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.png");
    let second = dir.path().join("second.png");
    std::fs::write(&first, b"first").unwrap();
    std::fs::write(&second, b"second").unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sent = seen.clone();
    let mut app = PhotocraftApp::new(
        photocraft_engine::Session::new(),
        crate::Services {
            deviantart_submit: Some(Arc::new(move |_, _, _, _, _, filename, _| {
                sent.lock().unwrap().push(filename.to_string());
                Ok(r#"{"itemid":42}"#.into())
            })),
            save_deviantart: Some(Box::new(|_| Ok(()))),
            ..Default::default()
        },
    );
    app.deviantart = Settings { client_id: "42".into(), refresh_token: "token".into(), send_on_export: true, ..Default::default() };
    let (tx, rx) = mpsc::channel();
    app.deviantart_job = Some(Job { rx, kind: JobKind::Submit, path: String::new(), temporary: false, name: "active".into() });
    after_written(&mut app, &first.to_string_lossy(), WriteKind::Export);
    after_written(&mut app, &second.to_string_lossy(), WriteKind::Export);
    assert_eq!(app.deviantart_pending.len(), 2);
    assert!(seen.lock().unwrap().is_empty());
    tx.send(Ok(json!({"body":"{\"itemid\":1}","warnings":[]}).to_string())).unwrap();
    let ctx = egui::Context::default();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while (app.deviantart_job.is_some() || !app.deviantart_pending.is_empty()) && std::time::Instant::now() < deadline {
        poll(&mut app, &ctx);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(app.deviantart_pending.is_empty());
    assert!(app.deviantart_job.is_none());
    assert_eq!(*seen.lock().unwrap(), vec!["first.png", "second.png"]);
}

#[test]
fn disconnect_drops_pending_automatic_submissions() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.deviantart_pending.push_back("private.png".into());
    errors::disconnect(&mut app);
    assert!(app.deviantart_pending.is_empty());
}

#[test]
fn multipart_boundary_avoids_file_and_text_delimiters() {
    let (content_type, _) = multipart("--PcDeviantArt0", "\r\n--PcDeviantArt1--\r\n", &[], "a.png", b"\r\n--PcDeviantArt2\r\n");
    assert_eq!(content_type, "multipart/form-data; boundary=PcDeviantArt3");
}

#[test]
fn access_token_error_does_not_discard_the_refresh_token() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.deviantart.refresh_token = "still-valid".into();
    errors::report_deviantart_error(&mut app, "invalid_token: access expired", false);
    assert_eq!(app.deviantart.refresh_token, "still-valid");
    errors::report_deviantart_error(&mut app, "refresh-grant-rejected: invalid_grant", false);
    assert!(app.deviantart.refresh_token.is_empty());
}

#[test]
fn disconnected_automatic_exports_keep_every_pending_path() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.deviantart.send_on_export = true;
    after_written(&mut app, "first.png", WriteKind::Export);
    after_written(&mut app, "second.png", WriteKind::Export);
    poll(&mut app, &egui::Context::default());
    assert_eq!(app.deviantart_pending.iter().map(String::as_str).collect::<Vec<_>>(), vec!["first.png", "second.png"]);
    assert_eq!(app.ui.dialogs.iter().filter(|dialog| owns(&dialog.fields)).count(), 1);
}

#[test]
fn window_close_waits_for_the_active_upload() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let (_tx, rx) = std::sync::mpsc::channel();
    app.deviantart_job = Some(Job { rx, kind: JobKind::Submit, path: String::new(), temporary: false, name: String::new() });
    app.jobs.close_after_saves = true;
    let ctx = egui::Context::default();
    crate::jobs_ui::close_after_saves(&mut app, &ctx);
    assert!(app.jobs.close_after_saves);
    app.deviantart_job = None;
    crate::jobs_ui::close_after_saves(&mut app, &ctx);
    assert!(!app.jobs.close_after_saves);
}

#[test]
fn automation_cannot_confirm_a_denied_deviantart_dialog() {
    let services = crate::Services {
        automation_command: Some(Box::new(|id, _| {
            assert_eq!(id, "file.submitDeviantArt");
            Err("network denied".into())
        })),
        ..crate::Services::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    let id = open_dialog(&mut app, None);
    app.automation_input = true;
    assert_eq!(crate::dialogs::confirm(&mut app, id), Err("network denied".into()));
    assert!(app.ui.dialog_mut(id).is_some());
}

#[test]
fn automatic_deviantart_submissions_default_off() {
    for settings in [Settings::default(), settings_from_json("{}"), settings_from_json("broken")] {
        assert!(!settings.send_on_save && !settings.send_on_save_as && !settings.send_on_save_copy && !settings.send_on_export);
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn menu_submission_with_params_uses_the_engine_dispatcher() {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), b"PNG").unwrap();
    let services = crate::Services {
        deviantart_submit: Some(std::sync::Arc::new(|_, _, _, _, _, _, bytes| {
            assert_eq!(bytes, b"PNG");
            Ok(r#"{"itemid":42}"#.into())
        })),
        ..crate::Services::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.deviantart.client_id = "99".into();
    app.deviantart.refresh_token = "private-token".into();
    configure_engine(&mut app);
    let result = crate::menus::invoke(&mut app, &egui::Context::default(), photocraft_engine::deviantart_cmds::COMMAND, json!({"path":file.path()})).unwrap();
    assert_eq!(result["body"], r#"{"itemid":42}"#);
    assert!(app.ui.dialogs.is_empty());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn engine_binding_loads_with_credentials_and_clears_on_failed_disconnect_save() {
    let services = crate::Services {
        load_deviantart: Some(Box::new(|| Some(r#"{"clientId":"99","refreshToken":"private"}"#.into()))),
        deviantart_submit: Some(std::sync::Arc::new(|_, _, _, _, _, _, _| Ok(r#"{"itemid":42}"#.into()))),
        save_deviantart: Some(Box::new(|_| Err("disk full".into()))),
        ..crate::Services::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), b"PNG").unwrap();
    let params = json!({"path":file.path()});
    assert!(app.session.execute(photocraft_engine::deviantart_cmds::COMMAND, params.clone()).is_ok());
    errors::disconnect(&mut app);
    assert!(app.session.deviantart_submit.is_none());
    assert!(app.session.execute(photocraft_engine::deviantart_cmds::COMMAND, params).is_err());
}

#[test]
fn deviantart_answered_save_arms_the_original_document_for_all_save_kinds() {
    for (command, kind) in [("file.save", WriteKind::Save), ("file.saveAs", WriteKind::SaveAs), ("file.saveACopy", WriteKind::SaveCopy)] {
        for background in [false, true] {
            let mut app = PhotocraftApp::new(
                photocraft_engine::Session::new(),
                crate::Services {
                    export: Some(Box::new(|_, _, _| Ok((vec![1], vec![])))),
                    write: Some(Box::new(|_, _| Ok(()))),
                    save_file: Some(std::sync::Arc::new(|_, _, _, _| Ok(vec![]))),
                    ..Default::default()
                },
            );
            app.session.execute("file.new", json!({"width":2,"height":2,"name":"source"})).unwrap();
            let source = app.active_doc_id().unwrap();
            app.session.execute("file.new", json!({"width":2,"height":2,"name":"other"})).unwrap();
            let other = app.active_doc_id().unwrap();
            app.deviantart.send_on_save = true;
            app.deviantart.send_on_save_as = true;
            app.deviantart.send_on_save_copy = true;
            app.background_jobs = background;
            app.write_answered_save(
                &egui::Context::default(),
                source,
                "source.jpg".into(),
                &crate::ExportSettings::default(),
                command == "file.saveACopy",
                (command, json!({})),
            );
            assert_eq!(app.active_doc_id().unwrap(), other);
            if background {
                assert_eq!(app.jobs.saves.last().unwrap().notify_deviantart, Some(kind));
                assert_eq!(app.jobs.saves.last().unwrap().doc, source);
                assert!(!dialog_open(&app));
            } else {
                assert!(dialog_open(&app));
                assert_eq!(app.deviantart_pending.back().map(String::as_str), Some("source.jpg"));
            }
        }
    }
}

#[test]
fn deviantart_tiff_prompt_arms_save_as_after_confirmation() {
    let mut app = PhotocraftApp::new(
        photocraft_engine::Session::new(),
        crate::Services { export: Some(Box::new(|_, _, _| Ok((vec![1], vec![])))), write: Some(Box::new(|_, _| Ok(()))), ..Default::default() },
    );
    app.session.execute("file.new", json!({"width":2,"height":2})).unwrap();
    app.deviantart.send_on_save_as = true;
    crate::tiff_options_ui::park(&mut app, "layers.tif".into()).unwrap();
    let ctx = egui::Context::default();
    let input = egui::RawInput {
        events: vec![egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }],
        ..Default::default()
    };
    ctx.run_ui(input, |ui| crate::tiff_options_ui::show(&mut app, ui.ctx())).textures_delta.clear();
    assert!(app.tiff_options.is_none());
    assert!(dialog_open(&app));
    assert_eq!(app.session.journal.last().unwrap().0, "file.saveAs");
}

#[test]
fn deviantart_direct_actions_honor_both_automation_gates() {
    for session_gate in [false, true] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        if session_gate {
            app.session.authorize = Some(|id, _| {
                assert_eq!(id, photocraft_engine::deviantart_cmds::COMMAND);
                Err(photocraft_engine::EngineError::Other("network denied".into()))
            });
        } else {
            app.automation_input = true;
            app.services.automation_command = Some(Box::new(|id, _| {
                assert_eq!(id, photocraft_engine::deviantart_cmds::COMMAND);
                Err("network denied".into())
            }));
        }
        for result in [start_connect(&mut app), start_prepared_submit(&mut app, "png", None)] {
            assert!(result.unwrap_err().contains("network denied"));
            assert!(app.deviantart_job.is_none());
        }
    }
}

#[test]
fn deviantart_only_mutating_jobs_delay_close() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    {
        let kind = JobKind::Connect;
        let (_tx, rx) = std::sync::mpsc::channel();
        app.deviantart_job = Some(Job { rx, kind, path: String::new(), temporary: false, name: String::new() });
        assert!(!busy(&app));
        app.jobs.close_after_saves = true;
        crate::jobs_ui::close_after_saves(&mut app, &egui::Context::default());
        assert!(!app.jobs.close_after_saves);
    }
    {
        let kind = JobKind::Submit;
        let (_tx, rx) = std::sync::mpsc::channel();
        app.deviantart_job = Some(Job { rx, kind, path: String::new(), temporary: false, name: String::new() });
        assert!(busy(&app));
    }
}
