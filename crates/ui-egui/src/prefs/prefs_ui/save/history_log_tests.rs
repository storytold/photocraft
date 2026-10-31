use super::*;
use crate::Services;
use photocraft_engine::Session;
use serde_json::json;
use std::cell::RefCell;
use std::rc::Rc;

type Appends = Rc<RefCell<Vec<(String, String)>>>;

fn app(destination: prefs::LogDestination) -> (PhotocraftApp, Appends) {
    let calls = Appends::default();
    let sink = calls.clone();
    let services = Services {
        append_text: Some(Box::new(move |path, text| {
            sink.borrow_mut().push((path.into(), text.into()));
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(Session::new(), services);
    app.run("file.new", json!({"width": 4, "height": 3})).unwrap();
    history_log(&mut app); // Consume setup while the default preference is disabled.
    app.session.edit_prefs(|p| {
        p.history_log.enabled = true;
        p.history_log.destination = destination;
        p.history_log.file_path = "history.txt".into();
    });
    (app, calls)
}

#[test]
fn failed_append_reports_the_path_and_error_without_undoing_the_edit_or_replaying_it() {
    for destination in [prefs::LogDestination::TextFile, prefs::LogDestination::Both] {
        let (mut app, calls) = app(destination);
        let sink = calls.clone();
        app.services.append_text = Some(Box::new(move |path, text| {
            sink.borrow_mut().push((path.into(), text.into()));
            // A sink can have written part of this text before returning an error.
            Err("disk full".into())
        }));
        let result = app.run("edit.fill", json!({"color": "#ff0000"}));
        assert!(result.is_ok());
        let edited = app.session.active().unwrap();
        assert!(edited.is_dirty());
        let mut pixel = [0.0; 4];
        edited.doc.layers[0].surface().unwrap().read_pixel(1, 1, &mut pixel);
        assert_eq!(pixel, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(edited.history.undo_label(), Some("Fill"));
        let (doc, revision, history_len) = (edited.doc.clone(), edited.revision, edited.history.past_len());
        let journal = app.session.journal.clone();
        assert_eq!(journal.last().unwrap().0, "edit.fill");

        // Exercise the same upkeep entry point as the running application.
        tick(&mut app, &egui::Context::default());
        assert_eq!(&*calls.borrow(), &[("history.txt".into(), "Fill\n".into())]);
        assert!(app.ui.status_error);
        for part in ["History Log", "history.txt", "disk full"] {
            assert!(app.ui.status.contains(part), "{}", app.ui.status);
        }
        let edited = app.session.active().unwrap();
        assert!(std::sync::Arc::ptr_eq(&edited.doc, &doc), "logging must not replace the successful edit");
        assert_eq!((edited.revision, edited.history.past_len()), (revision, history_len));
        assert_eq!(app.session.journal, journal);

        // Clearing the visible message makes any repeat error event observable.
        app.ui.status = "another status".into();
        app.ui.status_error = false;
        for _ in 0..3 {
            tick(&mut app, &egui::Context::default());
            assert_eq!(app.ui.status, "another status");
            assert!(!app.ui.status_error);
        }
        assert_eq!(calls.borrow().len(), 1);

        let sink = calls.clone();
        app.services.append_text = Some(Box::new(move |path, text| {
            sink.borrow_mut().push((path.into(), text.into()));
            Ok(())
        }));
        app.run("layer.renameLayer", json!({"name": "Renamed"})).unwrap();
        tick(&mut app, &egui::Context::default());
        tick(&mut app, &egui::Context::default());
        assert_eq!(&*calls.borrow(), &[("history.txt".into(), "Fill\n".into()), ("history.txt".into(), "Rename Layer\n".into())]);
        assert_eq!(app.prefs_rt.log_len, app.session.journal.len());
        assert_eq!(app.session.active().unwrap().doc.layers[0].name, "Renamed");
    }
}

#[test]
fn failed_append_uses_the_translated_history_log_label_and_keeps_literal_details() {
    let (mut app, _) = app(prefs::LogDestination::Both);
    app.session.edit_prefs(|p| p.history_log.file_path = "history-{error}.txt".into());
    app.services.append_text = Some(Box::new(|_, _| Err("cannot write {path}".into())));
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    crate::i18n::with_language(crate::i18n::Lang::from_code("fr").unwrap(), || history_log(&mut app));
    assert_eq!(app.ui.status, "Journal de l'historique (history-{error}.txt): cannot write {path}");
    assert!(app.ui.status_error);
}

#[test]
fn successful_append_preserves_unrelated_status() {
    for error in [false, true] {
        let (mut app, calls) = app(prefs::LogDestination::TextFile);
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        app.ui.status = "unrelated status".into();
        app.ui.status_error = error;
        history_log(&mut app);
        assert_eq!(calls.borrow().len(), 1);
        assert_eq!(app.ui.status, "unrelated status");
        assert_eq!(app.ui.status_error, error);
    }
}

#[test]
fn skipped_batches_preserve_status_and_are_not_appended_later() {
    for skip in ["disabled", "metadata", "empty path", "no service", "sessions only", "empty journal"] {
        let (mut app, calls) = app(prefs::LogDestination::TextFile);
        if skip != "empty journal" {
            app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        }
        app.session.edit_prefs(|p| match skip {
            "disabled" => p.history_log.enabled = false,
            "metadata" => p.history_log.destination = prefs::LogDestination::Metadata,
            "empty path" => p.history_log.file_path.clear(),
            "sessions only" => p.history_log.detail = prefs::LogDetail::SessionsOnly,
            _ => {}
        });
        let service = if skip == "no service" { app.services.append_text.take() } else { None };
        app.ui.status = "unrelated error".into();
        app.ui.status_error = true;
        history_log(&mut app);
        assert!(calls.borrow().is_empty(), "{skip}");
        assert_eq!(app.ui.status, "unrelated error", "{skip}");
        assert!(app.ui.status_error, "{skip}");
        assert_eq!(app.prefs_rt.log_len, app.session.journal.len());
        app.session.edit_prefs(|p| {
            p.history_log.enabled = true;
            p.history_log.destination = prefs::LogDestination::Both;
            p.history_log.file_path = "history.txt".into();
            p.history_log.detail = prefs::LogDetail::Concise;
        });
        if let Some(service) = service {
            app.services.append_text = Some(service);
        }
        history_log(&mut app);
        assert!(calls.borrow().is_empty(), "a skipped batch is consumed: {skip}");
        assert_eq!(app.ui.status, "unrelated error");
        assert!(app.ui.status_error);
    }
}

#[test]
fn detail_levels_keep_the_existing_labels_and_session_filter() {
    let entries = [
        ("file.new", json!({"width": 4})),
        ("edit.fill", json!({"color": "#ff0000"})),
        ("file.open", json!({"path": "input.png"})),
        ("file.close", json!({})),
        ("file.openAs", json!({"path": "input.psd"})),
        ("file.closeAll", json!({})),
        ("test.unknown", json!({"amount": 2})),
    ];
    for destination in [prefs::LogDestination::TextFile, prefs::LogDestination::Both] {
        for (detail, expected) in [
            (prefs::LogDetail::SessionsOnly, "New\nfile.open\nClose\nOpen As\n"),
            (prefs::LogDetail::Concise, "New\nFill\nfile.open\nClose\nOpen As\nClose All\ntest.unknown\n"),
            (
                prefs::LogDetail::Detailed,
                concat!(
                    "New  file.new {\"width\":4}\n",
                    "Fill  edit.fill {\"color\":\"#ff0000\"}\n",
                    "file.open  file.open {\"path\":\"input.png\"}\n",
                    "Close  file.close {}\n",
                    "Open As  file.openAs {\"path\":\"input.psd\"}\n",
                    "Close All  file.closeAll {}\n",
                    "test.unknown  test.unknown {\"amount\":2}\n",
                ),
            ),
        ] {
            let (mut app, calls) = app(destination);
            app.session.edit_prefs(|p| p.history_log.detail = detail);
            app.session.journal.extend(entries.iter().map(|(id, params)| ((*id).into(), params.clone())));
            history_log(&mut app);
            assert_eq!(&*calls.borrow(), &[("history.txt".into(), expected.into())], "{destination:?} {detail:?}");
        }
    }
}
