use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

fn host() -> Host {
    let host = Host::default();
    host.set_session("provider-test-session-0001".into()).unwrap();
    host.connect("test-provider".into(), "Test files".into(), "account-one".into()).unwrap();
    host
}

fn connection(host: &Host) -> String {
    host.snapshot()["connectionId"].as_str().unwrap_or("").into()
}

fn app() -> (PhotocraftApp, Host, DocId) {
    let host = host();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services { file_provider: Some(host.clone()), ..Default::default() });
    app.run("file.new", json!({"width":2,"height":2,"name":"Drawing"})).unwrap();
    let state = app.session.active_mut().unwrap();
    state.revision = 7;
    state.saved_revision = 2;
    let doc = state.doc.id;
    (app, host, doc)
}

fn begin(host: &Host, doc: DocId, copy: bool) -> String {
    host.begin(doc, 7, "drawing.psd", vec![1, 2, 3], Vec::new(), copy).unwrap();
    let events = host.take_events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].metadata["providerId"], "test-provider");
    assert_eq!(events[0].metadata["namespace"], "account-one");
    assert_eq!(events[0].bytes.as_deref(), Some(&[1, 2, 3][..]));
    events[0].metadata["requestId"].as_str().unwrap().into()
}

fn tick_once(app: &mut PhotocraftApp) {
    let ctx = egui::Context::default();
    tick(app, &ctx);
    crate::discard_ui::finish_provider_save(app, &ctx);
}

fn request_deferred_open(app: &mut PhotocraftApp, host: &Host) -> (String, Arc<AtomicUsize>) {
    let document = app.session.active().unwrap().doc.as_ref().clone();
    let imports = Arc::new(AtomicUsize::new(0));
    let decoder_calls = imports.clone();
    app.services.import = Some(Box::new(move |name, bytes, _| {
        assert_eq!(name, "loaded.png");
        assert_eq!(bytes, &[1][..]);
        decoder_calls.fetch_add(1, Ordering::Relaxed);
        Ok((document.clone(), Vec::new()))
    }));
    let request = host.request_open("resource-one").unwrap();
    host.take_events();
    (request, imports)
}

fn assert_deferred_open_rejected(app: &PhotocraftApp, host: &Host, doc: DocId, request: &str) {
    assert_eq!(app.session.documents().len(), 1, "a rejected delivery must not admit another document");
    assert_eq!(app.session.active().unwrap().doc.id, doc);
    assert!(app.ui.status_error);
    let events = host.take_events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].metadata["type"], "openFailed");
    assert_eq!(events[0].metadata["requestId"], request);
    assert_eq!(events[0].metadata["resourceId"], "resource-one");
    assert_eq!(events[0].metadata["connectionId"], connection(host));
    assert_eq!(events[0].metadata["namespace"], "account-one");
    assert!(!host.busy(), "the failed delivery retires only its own read");
}

#[test]
fn default_has_no_storage_authority_and_registration_is_explicit() {
    let host = Host::default();
    assert!(!host.connected());
    assert!(!host.enabled());
    assert!(!host.busy());
    assert_eq!(host.snapshot()["providerEnabled"], false);
    assert!(host.snapshot()["providerId"].is_null());
    assert!(host.connect("provider".into(), "Files".into(), "account".into()).is_err());
    assert!(host.set_enabled(true).is_err());
    assert!(host.request_open("resource").is_err());
    assert!(host.begin(DocId(1), 1, "file.psd", vec![1], Vec::new(), false).is_err());
    assert!(host.set_resources(&connection(&host), Vec::new()).is_err());
}

#[test]
fn identity_validation_and_idempotent_registration_preserve_disabled_state() {
    let host = Host::default();
    assert!(host.set_session("short".into()).is_err());
    host.set_session("provider-test-session-0001".into()).unwrap();
    for (id, label, namespace) in [("../provider", "Files", "account"), ("provider", "\n", "account"), ("provider", "Files", "account/email")] {
        assert!(host.connect(id.into(), label.into(), namespace.into()).is_err());
    }
    host.connect("provider".into(), "Files".into(), "account-one".into()).unwrap();
    host.set_enabled(false).unwrap();
    host.connect("provider".into(), "Files".into(), "account-one".into()).unwrap();
    assert!(!host.enabled());
    assert!(host.connect("provider".into(), "Files".into(), "account-two".into()).is_err());
    assert!(host.connect("other".into(), "Files".into(), "account-one".into()).is_err());
    assert!(host.set_session("provider-test-session-0002".into()).is_err());
}

#[test]
fn acknowledgement_marks_only_the_captured_revision() {
    for current in [7, 8] {
        let (mut app, host, doc) = app();
        let request = begin(&host, doc, false);
        app.session.active_mut().unwrap().revision = current;
        host.result(request, "saved".into(), "resource-one".into()).unwrap();
        tick_once(&mut app);
        let state = app.session.active().unwrap();
        assert_eq!(state.saved_revision, 7);
        assert_eq!(state.is_dirty(), current != 7);
        assert!(state.path.is_none(), "provider identities are not filesystem paths");
        assert!(host.bound(doc));
        assert_eq!(host.snapshot()["documents"][0]["resourceId"], "resource-one");
    }
}

#[test]
fn replacing_a_resource_invalidates_its_other_tabs_but_not_local_files() {
    let (mut app, host, saver) = app();
    app.session.active_mut().unwrap().revision = 8;
    app.run("file.new", json!({"width":2,"height":2,"name":"Other provider tab"})).unwrap();
    let alias = app.session.active().unwrap().doc.id;
    app.session.active_mut().unwrap().revision = 7;
    app.session.active_mut().unwrap().saved_revision = 7;
    app.run("file.new", json!({"width":2,"height":2,"name":"Physical copy"})).unwrap();
    let physical = app.session.active().unwrap().doc.id;
    {
        let state = app.session.active_mut().unwrap();
        state.path = Some("/device/drawing.psd".into());
        state.revision = 9;
        state.saved_revision = 9;
    }
    app.run("file.new", json!({"width":2,"height":2,"name":"Local homonym"})).unwrap();
    let homonym = app.session.active().unwrap().doc.id;
    {
        let state = app.session.active_mut().unwrap();
        state.path = Some("/elsewhere/drawing.psd".into());
        state.revision = 4;
        state.saved_revision = 4;
    }
    {
        let mut state = host.state();
        for doc in [saver, alias, physical] {
            state.bindings.insert(doc, "resource-one".into());
            state.provider_names.insert(doc, "drawing.psd".into());
            state.provider_revisions.insert(doc, 7);
        }
    }
    host.begin(saver, 8, "drawing.psd", vec![1], Vec::new(), false).unwrap();
    let request = host.take_events()[0].metadata["requestId"].as_str().unwrap().to_string();
    host.result(request, "saved".into(), "resource-one".into()).unwrap();
    tick_once(&mut app);
    let documents = app.session.documents();
    let saved = documents.iter().find(|state| state.doc.id == saver).unwrap();
    assert_eq!(saved.saved_revision, 8);
    assert!(!saved.is_dirty());
    assert!(host.bound(saver));
    let replaced = documents.iter().find(|state| state.doc.id == alias).unwrap();
    assert!(replaced.is_dirty(), "a replaced provider resource is no longer this tab's saved copy");
    assert!(!host.bound(alias));
    host.intent(alias, "save");
    host.state().destinations.insert(alias, "provider".into());
    assert!(!host.unchanged_provider_save(alias, 7, "drawing.psd"));
    let local = documents.iter().find(|state| state.doc.id == physical).unwrap();
    assert_eq!(local.path.as_deref(), Some("/device/drawing.psd"));
    assert_eq!(local.saved_revision, 9);
    assert!(!local.is_dirty(), "an independently saved physical file remains durable");
    assert!(!host.bound(physical), "the physical copy no longer represents the provider's bytes");
    let local = documents.iter().find(|state| state.doc.id == homonym).unwrap();
    assert_eq!(local.path.as_deref(), Some("/elsewhere/drawing.psd"));
    assert_eq!(local.saved_revision, 4);
    assert!(!local.is_dirty());
    assert_eq!(app.session.active().unwrap().doc.id, homonym, "alias invalidation does not select another tab");
}

#[test]
fn copies_invalidate_existing_bindings_only_when_they_replace_the_same_resource() {
    for same_resource in [false, true] {
        let (mut app, host, doc) = app();
        app.session.active_mut().unwrap().saved_revision = 7;
        {
            let mut state = host.state();
            state.bindings.insert(doc, "resource-one".into());
            state.provider_names.insert(doc, "drawing.psd".into());
            state.provider_revisions.insert(doc, 7);
        }
        let request = begin(&host, doc, true);
        host.result(request, "saved".into(), if same_resource { "resource-one" } else { "copy-resource" }.into()).unwrap();
        tick_once(&mut app);
        assert_eq!(host.bound(doc), !same_resource);
        assert_eq!(app.session.active().unwrap().is_dirty(), same_resource);
        if !same_resource {
            assert_eq!(app.session.active().unwrap().saved_revision, 7);
            assert_eq!(host.bound_filename(doc).as_deref(), Some("drawing.psd"));
        }
    }
}

#[test]
fn a_device_download_cannot_close_a_document_edited_after_its_capture() {
    for edited in [false, true] {
        let (mut app, host, doc) = app();
        app.services.write_unconfirmed = true;
        assert!(crate::discard_ui::intercept(&mut app, "file.close", &json!({})));
        host.state().destinations.insert(doc, "device".into());
        let request = begin(&host, doc, false);
        if edited {
            app.session.active_mut().unwrap().revision = 8;
        }
        host.result(request, "download".into(), String::new()).unwrap();
        tick_once(&mut app);
        if edited {
            assert_eq!(app.session.active().unwrap().doc.id, doc);
            assert_eq!(app.session.active().unwrap().saved_revision, 2);
            assert!(app.session.active().unwrap().is_dirty());
            assert!(crate::discard_ui::inspect(&app).is_some());
            assert!(!host.pending_for(doc), "the download completed, but did not include the later edit");
        } else {
            assert!(app.session.documents().is_empty(), "the unchanged capture retains the browser's existing download-and-close behaviour");
        }
    }
}

#[test]
fn a_deferred_close_cannot_use_the_download_exception_after_its_resource_is_overwritten() {
    let (mut app, host, first) = app();
    app.services.write_unconfirmed = true;
    let first_save = begin(&host, first, false);
    app.run("file.new", json!({"width":2,"height":2,"name":"Other"})).unwrap();
    let other = app.session.active().unwrap().doc.id;
    app.session.active_mut().unwrap().revision = 7;
    let other_save = begin(&host, other, false);
    {
        let mut state = host.state();
        state.bindings.insert(first, "shared-resource".into());
        state.bindings.insert(other, "shared-resource".into());
    }
    app.session.set_active(0);
    assert!(crate::discard_ui::intercept(&mut app, "file.close", &json!({})));
    host.result(first_save, "saved".into(), "shared-resource".into()).unwrap();
    host.result(other_save, "saved".into(), "shared-resource".into()).unwrap();
    tick_once(&mut app);
    let first = app.session.documents().iter().find(|state| state.doc.id == first).unwrap();
    assert!(first.is_dirty(), "the other tab replaced this tab's only durable checkpoint");
    assert!(!host.bound(first.doc.id));
    assert!(crate::discard_ui::inspect(&app).is_some());
}

#[test]
fn an_overwritten_alias_cannot_be_cleaned_or_closed_by_its_older_acknowledgement() {
    for (destination, status, resource, stale) in
        [("provider", "saved", "resource-one", true), ("device", "download", "", true), ("provider", "saved", "new-resource", false)]
    {
        assert_overwritten_alias_acknowledgement(destination, status, resource, stale);
    }
}

fn assert_overwritten_alias_acknowledgement(destination: &str, status: &str, resource: &str, stale: bool) {
    let (mut app, host, first) = app();
    app.services.write_unconfirmed = true;
    let new_doc = |app: &mut PhotocraftApp, name: &str, revision: u64, saved: u64, path: Option<&str>| {
        app.run("file.new", json!({"width":2,"height":2,"name":name})).unwrap();
        let state = app.session.active_mut().unwrap();
        state.revision = revision;
        state.saved_revision = saved;
        state.path = path.map(str::to_string);
        state.doc.id
    };
    let writer = new_doc(&mut app, "Newer writer", 8, 7, None);
    let physical = new_doc(&mut app, "Physical copy", 11, 11, Some("/device/drawing.psd"));
    let other_alias = new_doc(&mut app, "Other resource", 9, 9, None);
    let homonym = new_doc(&mut app, "Local homonym", 13, 13, Some("/elsewhere/drawing.psd"));
    {
        let mut state = host.state();
        for (doc, checkpoint) in [(first, 2), (writer, 7), (physical, 9), (other_alias, 9)] {
            state.bindings.insert(doc, if doc == other_alias { "new-resource" } else { "resource-one" }.into());
            state.provider_names.insert(doc, "drawing.psd".into());
            state.provider_revisions.insert(doc, checkpoint);
        }
        state.destinations.insert(first, destination.into());
    }
    if !stale {
        host.intent(first, "saveAs");
    }
    let begin_at = |doc, revision| {
        host.begin(doc, revision, "drawing.psd", vec![1, 2, 3], Vec::new(), false).unwrap();
        host.take_events().into_iter().find(|event| event.metadata["type"] == "save").unwrap().metadata["requestId"].as_str().unwrap().to_string()
    };
    let older = begin_at(first, 7);
    let newer = begin_at(writer, 8);
    let local_upload = begin_at(physical, 11);
    if stale {
        assert!(crate::discard_ui::intercept(&mut app, "file.close", &json!({"document":0})));
    }
    host.result(newer, "saved".into(), "resource-one".into()).unwrap();
    tick_once(&mut app);
    assert!(host.pending_for(first), "retiring checkpoint authority must not abandon the upload");
    assert!(host.pending_for(physical));
    assert!(host.disconnect().is_err());
    host.result(older, status.into(), resource.into()).unwrap();
    tick_once(&mut app);
    let writer_after_first = app.session.documents().iter().find(|state| state.doc.id == writer).unwrap();
    assert_eq!(writer_after_first.is_dirty(), stale && status == "saved", "only a write to this same resource invalidates its checkpoint");
    assert_eq!(host.bound(writer), !(stale && status == "saved"));
    host.result(local_upload, "saved".into(), "resource-one".into()).unwrap();
    tick_once(&mut app);

    let documents = app.session.documents();
    let first_state = documents.iter().find(|state| state.doc.id == first).unwrap();
    assert_eq!(first_state.is_dirty(), stale);
    assert_eq!(host.bound(first), !stale);
    if !stale {
        assert_eq!(host.state().bindings.get(&first).map(String::as_str), Some("new-resource"));
    }
    assert_eq!(crate::discard_ui::inspect(&app).is_some(), stale, "an older reply cannot release the close prompt");
    let writer_state = documents.iter().find(|state| state.doc.id == writer).unwrap();
    assert!(writer_state.is_dirty(), "the later physical-copy upload can have replaced the same resource too");
    assert_eq!(writer_state.revision, 8);
    assert!(!host.bound(writer));
    assert_independent_alias_checkpoints(&app, &host, physical, other_alias, homonym, stale);
    assert!(!host.pending_for(first));
    assert!(!host.pending_for(physical));
}

fn assert_independent_alias_checkpoints(app: &PhotocraftApp, host: &Host, physical: DocId, other_alias: DocId, homonym: DocId, stale: bool) {
    let documents = app.session.documents();
    let physical_state = documents.iter().find(|state| state.doc.id == physical).unwrap();
    assert_eq!(physical_state.path.as_deref(), Some("/device/drawing.psd"));
    assert_eq!(physical_state.saved_revision, 11);
    assert_eq!(physical_state.revision, 11);
    assert!(!physical_state.is_dirty(), "the independently saved physical copy remains durable");
    assert!(!host.bound(physical), "the retired upload cannot detach the physical file and rebind it");
    let other_state = documents.iter().find(|state| state.doc.id == other_alias).unwrap();
    assert_eq!(other_state.is_dirty(), !stale, "only a successful Save As to this resource invalidates its alias");
    assert_eq!(host.bound(other_alias), stale);
    let local = documents.iter().find(|state| state.doc.id == homonym).unwrap();
    assert_eq!(local.path.as_deref(), Some("/elsewhere/drawing.psd"));
    assert_eq!(local.saved_revision, 13);
    assert!(!local.is_dirty());
    assert_eq!(app.session.active().unwrap().doc.id, homonym);
}

#[test]
fn delayed_reply_does_not_change_the_selected_tab_or_a_homonymous_local_file() {
    let (mut app, host, doc) = app();
    let request = begin(&host, doc, false);
    app.run("file.new", json!({"width":2,"height":2,"name":"Other"})).unwrap();
    let other = app.session.active().unwrap().doc.id;
    app.session.active_mut().unwrap().path = Some("drawing.psd".into());
    host.result(request, "saved".into(), "resource-one".into()).unwrap();
    tick_once(&mut app);
    assert_eq!(app.session.active().unwrap().doc.id, other);
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some("drawing.psd"));
    let saved = app.session.documents().iter().find(|state| state.doc.id == doc).unwrap();
    assert_eq!(saved.saved_revision, 7);
    assert!(saved.path.is_none());
    assert!(app.ui.recent_files.is_empty());
}

#[test]
fn a_reply_for_a_closed_document_cannot_save_another_document() {
    let (mut app, host, doc) = app();
    let request = begin(&host, doc, false);
    app.session.close(0).unwrap();
    capture(&mut app);
    assert!(host.result(request, "saved".into(), "resource-one".into()).is_err());
    app.run("file.new", json!({"width":2,"height":2,"name":"Replacement"})).unwrap();
    let new_doc = app.session.active().unwrap().doc.id;
    assert_ne!(doc, new_doc);
    tick_once(&mut app);
    assert!(!host.bound(new_doc));
}

#[test]
fn failed_saves_remain_pending_and_retry_uses_the_same_capture() {
    let (mut app, host, doc) = app();
    let request = begin(&host, doc, false);
    host.result(request.clone(), "failed".into(), String::new()).unwrap();
    assert!(host.result(request.clone(), "saved".into(), "resource-one".into()).is_err(), "a queued result cannot be replaced");
    tick_once(&mut app);
    assert!(host.pending_for(doc));
    assert_eq!(app.session.active().unwrap().saved_revision, 2);
    assert!(host.take_events().is_empty(), "retry does not recapture newer content");
    app.session.active_mut().unwrap().revision = 8;
    host.result(request.clone(), "saved".into(), "resource-one".into()).unwrap();
    tick_once(&mut app);
    assert_eq!(app.session.active().unwrap().saved_revision, 7);
    assert!(app.session.active().unwrap().is_dirty());
    assert!(host.result(request, "saved".into(), "resource-one".into()).is_err());
}

#[test]
fn copies_and_unconfirmed_results_do_not_advance_saved_state() {
    for (status, copy) in [("failed", false), ("download", false), ("cancel", false), ("saved", true)] {
        let (mut app, host, doc) = app();
        if status == "download" {
            host.state().destinations.insert(doc, "device".into());
        }
        let request = begin(&host, doc, copy);
        host.result(request, status.into(), if status == "saved" { "copy-resource".into() } else { String::new() }).unwrap();
        tick_once(&mut app);
        assert_eq!(app.session.active().unwrap().saved_revision, 2);
        assert!(!host.bound(doc));
        assert!(app.session.active().unwrap().is_dirty());
    }
}

#[test]
fn a_device_download_cannot_acknowledge_provider_persistence() {
    let (mut app, host, doc) = app();
    host.state().destinations.insert(doc, "device".into());
    let request = begin(&host, doc, false);
    assert!(host.result(request.clone(), "saved".into(), "resource-one".into()).is_err());
    host.state().commands.push(Command::Result(request, "saved".into(), "resource-one".into()));
    tick_once(&mut app);
    assert_eq!(app.session.active().unwrap().saved_revision, 2);
    assert!(!host.bound(doc));
}

#[test]
fn a_provider_upload_cannot_close_a_dirty_document_as_an_unconfirmed_download() {
    let (mut app, host, doc) = app();
    app.services.write_unconfirmed = true;
    assert!(crate::discard_ui::intercept(&mut app, "file.close", &json!({})));
    let request = begin(&host, doc, false);
    assert!(host.result(request.clone(), "download".into(), String::new()).is_err());
    assert!(host.pending_for(doc));
    // Also defend the application boundary if an invalid result bypasses API admission.
    host.state().commands.push(Command::Result(request, "download".into(), String::new()));
    tick_once(&mut app);
    assert_eq!(app.session.active().unwrap().doc.id, doc);
    assert_eq!(app.session.active().unwrap().saved_revision, 2);
    assert!(app.session.active().unwrap().is_dirty());
    assert!(crate::discard_ui::inspect(&app).is_some());
}

#[test]
fn requested_and_queued_provider_opens_are_busy_but_not_saving() {
    let (mut app, host, _) = app();
    host.set_resources(&connection(&host), vec![Resource { id: "one".into(), name: "one.psd".into(), updated_at: String::new() }]).unwrap();
    let request = host.request_open("one").unwrap();
    assert!(host.busy());
    assert!(!host.saving());
    assert!(!app.saving());
    host.open(request, "one.psd".into(), vec![1, 2, 3]).unwrap();
    assert!(host.busy());
    assert!(!host.saving());
    assert!(!app.saving());
    tick_once(&mut app);
    assert!(!host.busy());
}

#[test]
fn request_ids_are_unique_across_sessions_and_counters_do_not_wrap() {
    let (_, one, doc) = app();
    let first = begin(&one, doc, false);
    let two = Host::default();
    two.set_session("provider-test-session-0002".into()).unwrap();
    two.connect("test-provider".into(), "Test files".into(), "account-one".into()).unwrap();
    let second = begin(&two, doc, false);
    assert_ne!(first, second);
    assert!(two.result(first, "saved".into(), "resource-one".into()).is_err());
    let exhausted = host();
    exhausted.state().serial = u64::MAX;
    assert!(exhausted.begin(doc, 7, "drawing.psd", vec![1], Vec::new(), false).is_err());
    assert!(exhausted.request_open("resource-one").is_err());
    assert!(exhausted.take_events().is_empty());
}

#[test]
fn unregister_waits_for_operations_and_removes_previous_identity() {
    let (mut app, host, doc) = app();
    let request = begin(&host, doc, false);
    assert!(host.disconnect().is_err());
    host.set_enabled(false).unwrap();
    host.result(request, "saved".into(), "resource-one".into()).unwrap();
    tick_once(&mut app);
    host.set_resources(&connection(&host), vec![Resource { id: "resource-one".into(), name: "drawing.psd".into(), updated_at: String::new() }]).unwrap();
    host.disconnect().unwrap();
    assert!(!host.bound(doc));
    assert!(host.resources().is_empty());
    assert!(host.take_events().is_empty());
    let snapshot = host.snapshot();
    assert!(snapshot["providerId"].is_null());
    assert!(snapshot["providerLabel"].is_null());
    assert!(snapshot["namespace"].is_null());
    assert!(snapshot["documents"].as_array().unwrap().is_empty());
    host.connect("second-provider".into(), "Other files".into(), "account-two".into()).unwrap();
    assert_eq!(host.snapshot()["namespace"], "account-two");
}

#[test]
fn resource_lists_are_bounded_unique_and_independent_of_local_recents() {
    let (mut app, host, _) = app();
    app.push_recent("/local/drawing.psd");
    let entry = Resource { id: "resource-one".into(), name: "drawing.psd".into(), updated_at: "today".into() };
    host.set_resources(&connection(&host), vec![entry.clone()]).unwrap();
    assert_eq!(app.ui.recent_files, ["/local/drawing.psd"]);
    assert!(host.set_resources(&connection(&host), vec![entry.clone(), entry.clone()]).is_err());
    assert!(host.set_resources(&connection(&host), (0..101).map(|i| Resource { id: format!("resource-{i}"), ..entry.clone() }).collect()).is_err());
    assert!(host.set_resources(&connection(&host), vec![Resource { name: "../drawing.psd".into(), ..entry.clone() }]).is_err());
    assert!(host.set_resources(&connection(&host), vec![Resource { id: "a/b".into(), ..entry.clone() }]).is_err());
    assert!(host.set_resources(&connection(&host), vec![Resource { updated_at: "x".repeat(101), ..entry }]).is_err());
    assert_eq!(host.resources().len(), 1, "rejected updates do not clear the last good list");
}

#[test]
fn requested_opens_have_nonce_ids_and_cancelled_callbacks_cannot_reach_a_new_account() {
    let host = host();
    let request = host.request_open("resource-one").unwrap();
    let events = host.take_events();
    assert_eq!(events[0].metadata["requestId"], request);
    assert_eq!(events[0].metadata["resourceId"], "resource-one");
    assert!(host.busy());
    assert!(host.disconnect().is_err());
    assert!(host.request_open("resource-one").is_err());
    host.cancel_open(request.clone(), String::new()).unwrap();
    assert_eq!(host.take_events()[0].metadata["type"], "openCancelled");
    host.disconnect().unwrap();
    host.connect("test-provider".into(), "Test files".into(), "account-two".into()).unwrap();
    assert!(host.open(request.clone(), "drawing.psd".into(), vec![1]).is_err());
    assert!(host.cancel_open(request, "late failure".into()).is_err());
    assert!(!host.busy());
}

#[test]
fn late_resource_lists_cannot_cross_accounts_or_reconnections() {
    let host = host();
    let original = connection(&host);
    assert_eq!(host.connect("test-provider".into(), "Test files".into(), "account-one".into()).unwrap(), original);
    host.disconnect().unwrap();
    let next = host.connect("test-provider".into(), "Test files".into(), "account-two".into()).unwrap();
    assert_ne!(original, next);
    let current = Resource { id: "current".into(), name: "current.psd".into(), updated_at: String::new() };
    host.set_resources(&next, vec![current.clone()]).unwrap();
    assert!(host.set_resources(&original, vec![Resource { id: "old".into(), name: "old.psd".into(), updated_at: String::new() }]).is_err());
    assert_eq!(host.resources().as_slice(), std::slice::from_ref(&current));
    host.disconnect().unwrap();
    let again = host.connect("test-provider".into(), "Test files".into(), "account-two".into()).unwrap();
    assert_ne!(next, again, "even reconnecting the same account changes asynchronous authority");
    assert!(host.set_resources(&next, vec![current]).is_err());
    assert!(host.resources().is_empty());
}

#[test]
fn provider_open_uses_the_decoder_then_acknowledges_the_admitted_document() {
    let (mut app, host, original) = app();
    let original_snapshot = app.session.active().unwrap().doc.clone();
    let (request, imports) = request_deferred_open(&mut app, &host);
    app.background_jobs = true;
    assert!(app.session.active().unwrap().is_dirty(), "an idle dirty document does not block opening another tab");
    host.open(request.clone(), "loaded.png".into(), vec![1]).unwrap();
    assert!(host.busy());
    tick_once(&mut app);
    assert!(app.background_jobs, "the provider-only synchronous path restores the platform policy");
    assert_eq!(imports.load(Ordering::Relaxed), 1);
    assert_eq!(app.session.documents().len(), 2);
    assert!(Arc::ptr_eq(&app.session.documents().iter().find(|state| state.doc.id == original).unwrap().doc, &original_snapshot));
    let events = host.take_events();
    let opened = events.iter().find(|e| e.metadata["type"] == "opened").unwrap();
    let state = app.session.active().unwrap();
    assert_ne!(state.doc.id, original, "imported IDs are not reused as ACK authority");
    assert_eq!(opened.metadata["documentId"], state.doc.id.0.to_string());
    assert_eq!(opened.metadata["requestId"], request);
    assert_eq!(opened.metadata["resourceId"], "resource-one");
    assert!(state.path.is_none());
    assert!(app.ui.recent_files.is_empty());
    assert!(!host.busy());
}

#[test]
fn provider_open_accepts_idle_pointer_motion_and_leave_in_a_real_input_frame() {
    for events in [
        vec![egui::Event::PointerMoved(egui::pos2(20.0, 20.0))],
        vec![egui::Event::MouseMoved(egui::vec2(4.0, 2.0))],
        vec![egui::Event::PointerMoved(egui::pos2(20.0, 20.0)), egui::Event::PointerGone],
    ] {
        let ctx = egui::Context::default();
        PhotocraftApp::setup_context(&ctx, Default::default());
        let (mut app, host, original) = app();
        let source = app.session.active().unwrap().doc.clone();
        let (request, imports) = request_deferred_open(&mut app, &host);
        host.open(request.clone(), "loaded.png".into(), vec![1]).unwrap();
        let mut output = ctx.run_ui(egui::RawInput { focused: true, events, ..Default::default() }, |ui| {
            assert!(!ui.input(|input| input.pointer.any_down()));
            assert!(!ui.ctx().text_edit_focused());
            tick(&mut app, ui.ctx());
        });
        output.textures_delta.clear(); // This input/protocol fixture does not paint.
        assert_eq!(imports.load(Ordering::Relaxed), 1, "idle hover must not require a second open request");
        assert_eq!(app.session.documents().len(), 2);
        let original_state = app.session.documents().iter().find(|state| state.doc.id == original).unwrap();
        assert!(Arc::ptr_eq(&original_state.doc, &source));
        let active = app.session.active().unwrap().doc.id;
        assert_ne!(active, original);
        assert!(host.bound(active));
        let events = host.take_events();
        let opened = events.iter().find(|event| event.metadata["type"] == "opened").unwrap();
        assert_eq!(opened.metadata["requestId"], request);
        assert_eq!(opened.metadata["resourceId"], "resource-one");
        assert_eq!(opened.metadata["documentId"], active.0.to_string());
        assert!(!events.iter().any(|event| event.metadata["type"] == "openFailed"));
        assert!(!host.busy());
    }
}

#[test]
fn provider_open_still_rejects_pointer_motion_while_a_button_is_held() {
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, Default::default());
    let (mut app, host, doc) = app();
    let source = app.session.active().unwrap().doc.clone();
    let (request, imports) = request_deferred_open(&mut app, &host);
    let down =
        egui::Event::PointerButton { pos: egui::pos2(10.0, 10.0), button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE };
    let mut output = ctx.run_ui(egui::RawInput { focused: true, events: vec![down], ..Default::default() }, |_| {});
    output.textures_delta.clear();
    host.open(request.clone(), "loaded.png".into(), vec![1]).unwrap();
    let raw = egui::RawInput { focused: true, events: vec![egui::Event::PointerMoved(egui::pos2(20.0, 20.0))], ..Default::default() };
    let mut output = ctx.run_ui(raw, |ui| {
        assert!(ui.input(|input| input.pointer.any_down()));
        tick(&mut app, ui.ctx());
        assert_deferred_open_rejected(&app, &host, doc, &request);
    });
    output.textures_delta.clear();
    assert_eq!(imports.load(Ordering::Relaxed), 0);
    assert!(Arc::ptr_eq(&app.session.active().unwrap().doc, &source));
}

#[test]
fn deferred_provider_open_does_not_retarget_a_live_filter_dialog() {
    for confirm in [false, true] {
        let (mut app, host, doc) = app();
        app.run("layer.new.layer", json!({})).unwrap();
        let (request, imports) = request_deferred_open(&mut app, &host);
        let dialog = crate::dialogs::open_command_dialog(&mut app, "filter.blur.gaussianBlur", "Gaussian Blur");
        app.ui.dialog_mut(dialog).unwrap().fields.insert("radius".into(), json!(2.0));
        let dialogs = app.ui.dialogs.clone();
        let source = app.session.active().unwrap().doc.clone();
        let checkpoint = app.session.active().unwrap().saved_revision;
        host.open(request.clone(), "loaded.png".into(), vec![1]).unwrap();
        tick_once(&mut app);
        assert_eq!(imports.load(Ordering::Relaxed), 0, "reject before decoding or admitting the incoming file");
        assert_eq!(app.ui.dialogs, dialogs, "the existing dialog and its edited fields remain intact");
        assert!(Arc::ptr_eq(&app.session.active().unwrap().doc, &source));
        assert_eq!(app.session.active().unwrap().saved_revision, checkpoint);
        assert_deferred_open_rejected(&app, &host, doc, &request);
        if confirm {
            crate::dialogs::confirm(&mut app, dialog).unwrap();
        } else {
            crate::dialogs::cancel(&mut app, dialog).unwrap();
            assert!(Arc::ptr_eq(&app.session.active().unwrap().doc, &source), "cancelling must not edit the original document");
        }
        assert_eq!(app.session.documents().len(), 1);
        assert_eq!(app.session.active().unwrap().doc.id, doc, "the original dialog still targets its own document");
        assert!(app.ui.dialogs.is_empty());
    }
}

#[test]
fn deferred_provider_open_preserves_waiting_remove_strokes_until_explicit_cancel() {
    use crate::canvas::{ToolEvent, tool_event};

    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, Default::default());
    let (mut app, host, doc) = app();
    app.run("layer.new.layer", json!({"name":"Remove source"})).unwrap();
    let (request, imports) = request_deferred_open(&mut app, &host);
    let source = app.session.active().unwrap().doc.clone();
    let revision = app.session.active().unwrap().revision;
    let checkpoint = app.session.active().unwrap().saved_revision;
    app.ui.tool = crate::state::Tool::Remove;
    app.ui.tool_options.remove_after_each_stroke = false;
    let modifiers = egui::Modifiers::NONE;
    tool_event(&mut app, ToolEvent::Down { x: 0.0, y: 0.0, pressure: 1.0 }, modifiers);
    tool_event(&mut app, ToolEvent::Move { x: 1.0, y: 1.0, pressure: 1.0 }, modifiers);
    tool_event(&mut app, ToolEvent::Up { x: 1.0, y: 1.0 }, modifiers);
    assert!(app.drag.is_none() && app.live_stroke.is_none(), "the stroke has ended, rather than remaining a held gesture");
    assert!(app.ui.dialogs.is_empty());
    assert_eq!(app.remove_pending.len(), 1);
    assert!(app.remove_trail.is_some());
    let waiting = app.remove_pending.clone();
    let target = app.remove_pending_target.clone();
    host.open(request.clone(), "loaded.png".into(), vec![1]).unwrap();
    let mut output = ctx.run_ui(egui::RawInput { focused: true, ..Default::default() }, |ui| {
        assert!(ui.input(|input| input.events.is_empty() && !input.pointer.any_down() && input.keys_down.is_empty()));
        tick(&mut app, ui.ctx());
        assert_deferred_open_rejected(&app, &host, doc, &request);
    });
    output.textures_delta.clear(); // This input/protocol fixture does not paint.
    assert_eq!(imports.load(Ordering::Relaxed), 0);
    assert!(Arc::ptr_eq(&app.session.active().unwrap().doc, &source));
    assert_eq!(app.session.active().unwrap().revision, revision);
    assert_eq!(app.session.active().unwrap().saved_revision, checkpoint);
    assert_eq!(app.remove_pending, waiting);
    assert!(app.remove_pending_target == target);
    assert!(app.remove_trail.is_some());
    assert!(crate::retouch_ui::remove_pending_live(&mut app), "the waiting stroke still belongs to its original target");
    assert_eq!(app.ui.status, crate::i18n::t("Finish or cancel the current edit before opening a provider file."));

    assert!(crate::retouch_ui::remove_pending_cancel(&mut app));
    assert!(app.remove_pending.is_empty() && app.remove_pending_target.is_none() && app.remove_trail.is_none());
    let next = host.request_open("resource-one").unwrap();
    assert_ne!(next, request);
    let events = host.take_events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].metadata["type"], "openRequest");
    assert_eq!(events[0].metadata["requestId"], next);
    host.open(next.clone(), "loaded.png".into(), vec![1]).unwrap();
    tick_once(&mut app);
    assert_eq!(imports.load(Ordering::Relaxed), 1);
    assert_eq!(app.session.documents().len(), 2);
    let active = app.session.active().unwrap().doc.id;
    assert_ne!(active, doc);
    assert!(host.bound(active));
    let original = app.session.documents().iter().find(|state| state.doc.id == doc).unwrap();
    assert!(Arc::ptr_eq(&original.doc, &source));
    assert_eq!(original.revision, revision);
    let events = host.take_events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].metadata["type"], "opened");
    assert_eq!(events[0].metadata["requestId"], next);
    assert_eq!(events[0].metadata["resourceId"], "resource-one");
    assert_eq!(events[0].metadata["documentId"], active.0.to_string());
    assert!(!host.busy());
}

#[test]
fn deferred_provider_open_preserves_a_live_gradient_until_its_user_commits_it() {
    let (mut app, host, doc) = app();
    app.run("layer.new.layer", json!({})).unwrap();
    let (request, imports) = request_deferred_open(&mut app, &host);
    app.ui.tool = crate::state::Tool::Gradient;
    app.ui.tool_options.gradient_classic = false;
    assert!(crate::gradient_ui::pointer(&mut app, crate::canvas::ToolEvent::Down { x: 0.0, y: 0.0, pressure: 1.0 }, egui::Modifiers::NONE));
    assert!(crate::gradient_ui::pointer(&mut app, crate::canvas::ToolEvent::Move { x: 10.0, y: 10.0, pressure: 1.0 }, egui::Modifiers::NONE));
    assert!(crate::gradient_ui::editing(&app));
    let source = app.session.active().unwrap().doc.clone();
    let revision = app.session.active().unwrap().revision;
    assert!(app.ui.dialogs.is_empty(), "this uncommitted tool preview has no modal dialog");
    host.open(request.clone(), "loaded.png".into(), vec![1]).unwrap();
    tick_once(&mut app);
    assert_eq!(imports.load(Ordering::Relaxed), 0);
    assert!(crate::gradient_ui::editing(&app));
    assert!(Arc::ptr_eq(&app.session.active().unwrap().doc, &source));
    assert_eq!(app.session.active().unwrap().revision, revision);
    assert_deferred_open_rejected(&app, &host, doc, &request);
    assert!(crate::gradient_ui::pointer(&mut app, crate::canvas::ToolEvent::Up { x: 10.0, y: 10.0 }, egui::Modifiers::NONE));
    assert!(!crate::gradient_ui::editing(&app));
    assert_eq!(app.session.active().unwrap().doc.id, doc);
    assert_ne!(app.session.active().unwrap().revision, revision, "only the user's later commit changes the document");
}

#[test]
fn deferred_provider_open_does_not_preempt_type_input_from_the_same_frame() {
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, Default::default());
    let (mut app, host, doc) = app();
    let layer = app.run("type.create", json!({"text":"Before", "size":12})).unwrap()["layer"].as_u64().unwrap();
    crate::type_tool::edit_active(&mut app).unwrap();
    let edit = app.ui.text_edit.as_mut().unwrap();
    edit.anchor = edit.caret;
    let (request, imports) = request_deferred_open(&mut app, &host);
    host.open(request.clone(), "loaded.png".into(), vec![1]).unwrap();
    let raw = egui::RawInput { focused: true, events: vec![egui::Event::Text(" after".into())], ..Default::default() };
    let mut output = ctx.run_ui(raw, |ui| {
        tick(&mut app, ui.ctx());
        assert_eq!(app.session.active().unwrap().doc.id, doc);
        assert_deferred_open_rejected(&app, &host, doc, &request);
        assert!(crate::type_tool::handle_keys(&mut app, ui.ctx()));
        app.finish_type_reset_pass(ui.ctx());
    });
    output.textures_delta.clear(); // This input/protocol fixture does not paint.
    assert_eq!(imports.load(Ordering::Relaxed), 0);
    let state = app.session.active().unwrap();
    let photocraft_doc::LayerContent::Text(text) = &state.doc.layer(photocraft_doc::LayerId(layer)).unwrap().content else { panic!("text layer") };
    assert_eq!(text.text, "Before after");
    assert_eq!(app.session.documents().len(), 1);
    assert!(!host.busy());
}

#[test]
fn deferred_provider_open_keeps_a_focused_text_field_draft_without_new_input() {
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, Default::default());
    let (mut app, host, doc) = app();
    let (request, imports) = request_deferred_open(&mut app, &host);
    let field = egui::Id::unique("provider-open-text-draft");
    let mut draft = "1e-".to_string();
    ctx.memory_mut(|memory| memory.request_focus(field));
    let mut output = ctx.run_ui(egui::RawInput { focused: true, ..Default::default() }, |ui| {
        assert!(ui.add(egui::TextEdit::singleline(&mut draft).id(field)).has_focus());
    });
    output.textures_delta.clear();
    assert!(ctx.text_edit_focused());
    let source = app.session.active().unwrap().doc.clone();
    host.open(request.clone(), "loaded.png".into(), vec![1]).unwrap();
    let mut output = ctx.run_ui(egui::RawInput { focused: true, ..Default::default() }, |ui| {
        assert!(ui.input(|input| input.events.is_empty()));
        tick(&mut app, ui.ctx());
        assert!(ui.add(egui::TextEdit::singleline(&mut draft).id(field)).has_focus());
    });
    output.textures_delta.clear();
    assert_eq!(draft, "1e-");
    assert_eq!(imports.load(Ordering::Relaxed), 0);
    assert!(Arc::ptr_eq(&app.session.active().unwrap().doc, &source));
    assert_deferred_open_rejected(&app, &host, doc, &request);
}

#[test]
fn malformed_opens_fail_without_creating_bindings_and_leave_no_busy_request() {
    let (mut app, host, original) = app();
    app.services.import = Some(Box::new(|_, _, _| Err("bad document".into())));
    let request = host.request_open("resource-one").unwrap();
    host.take_events();
    host.open(request.clone(), "bad.png".into(), vec![1]).unwrap();
    tick_once(&mut app);
    let events = host.take_events();
    assert!(events.iter().any(|e| e.metadata["type"] == "openFailed" && e.metadata["requestId"] == request));
    assert_eq!(app.session.active().unwrap().doc.id, original);
    assert!(!host.bound(original));
    assert!(!host.busy());
}

#[test]
fn an_edit_between_admission_and_acknowledgement_is_not_a_provider_checkpoint() {
    let (mut app, host, doc) = app();
    host.state().opening = Some(Opening { request: "opening-request".into(), resource: "resource-one".into(), name: "drawing.psd".into() });
    host.opening_document(doc);
    {
        let state = app.session.active_mut().unwrap();
        state.saved_revision = 7;
        state.revision = 8;
    }
    finish_opens(&app, &host);
    assert_eq!(host.state().provider_revisions.get(&doc), Some(&7));
    host.state().destinations.insert(doc, "provider".into());
    host.intent(doc, "save");
    assert!(!host.unchanged_provider_save(doc, 8, "drawing.psd"));
    capture(&mut app);
    assert_eq!(host.snapshot()["documents"][0]["dirty"], true);
}

#[test]
fn a_remapped_or_cancelled_open_keeps_its_request_identity() {
    let host = host();
    let old = DocId(41);
    let new = DocId(42);
    host.state().opening = Some(Opening { request: "opening-request".into(), resource: "resource-one".into(), name: "drawing.psd".into() });
    host.opening_document(old);
    host.remap_opening_document(old, new);
    assert!(!host.state().opening_docs.contains_key(&old));
    assert!(host.state().opening_docs.contains_key(&new));
    host.abandon_opening_document(new, None);
    let events = host.take_events();
    assert_eq!(events[0].metadata["type"], "openCancelled");
    assert_eq!(events[0].metadata["requestId"], "opening-request");
    assert_eq!(events[0].metadata["resourceId"], "resource-one");
    assert!(!host.busy());
    host.abandon_opening_document(new, Some("late failure"));
    assert!(host.take_events().is_empty());
}

#[test]
fn decoded_open_errors_are_bounded_without_slicing_unicode() {
    let host = host();
    host.state().opening = Some(Opening { request: "opening-request".into(), resource: "resource-one".into(), name: "drawing.psd".into() });
    host.opening_document(DocId(1));
    host.abandon_opening_document(DocId(1), Some(&"é".repeat(1000)));
    let event = host.take_events().pop().unwrap();
    assert_eq!(event.metadata["type"], "openFailed");
    assert_eq!(event.metadata["error"].as_str().unwrap().len(), 1024);
}

#[test]
fn cancelled_or_failed_fetches_are_terminal_bounded_and_do_not_mutate_the_list() {
    let host = host();
    let request = host.request_open("resource-one").unwrap();
    host.take_events();
    assert!(host.cancel_open(request.clone(), "x".repeat(1025)).is_err());
    assert!(host.busy());
    host.cancel_open(request.clone(), "not available".into()).unwrap();
    let events = host.take_events();
    assert_eq!(events[0].metadata["type"], "openFailed");
    assert_eq!(events[0].metadata["requestId"], request);
    assert!(!host.busy());
    assert!(host.open(request, "late.png".into(), vec![1]).is_err());
}

#[test]
fn queued_open_bytes_and_captured_save_events_have_aggregate_limits() {
    let host = host();
    let size = MAX_BYTES / 2 + 1;
    host.begin(DocId(1), 7, "one.psd", vec![0; size], Vec::new(), false).unwrap();
    assert!(host.begin(DocId(2), 7, "two.psd", vec![0; size], Vec::new(), false).is_err());
    assert_eq!(host.state().event_bytes(), size);
    host.take_events();
    let one = host.request_open("one").unwrap();
    let two = host.request_open("two").unwrap();
    host.open(one, "one.png".into(), vec![0; size]).unwrap();
    assert!(host.open(two.clone(), "two.png".into(), vec![0; size]).is_err());
    assert!(host.state().requested_opens.contains_key(&two), "a capacity rejection leaves a cancellable fetch");
}

#[test]
fn invalid_names_and_oversized_files_never_enter_the_open_queue() {
    let host = host();
    let request = host.request_open("one").unwrap();
    for name in ["../one.psd", "one/other.psd", "one\\other.psd", "\0.psd", "", " "] {
        assert!(host.open(request.clone(), name.into(), vec![1]).is_err());
    }
    assert!(host.open(request.clone(), "one.psd".into(), Vec::new()).is_err());
    assert!(host.open(request.clone(), "one.psd".into(), vec![0; MAX_BYTES + 1]).is_err());
    assert!(host.state().commands.is_empty());
    assert!(host.state().requested_opens.contains_key(&request));
}

#[test]
fn clean_noop_requires_a_resolved_provider_choice_with_the_same_binding_and_revision() {
    let host = host();
    let doc = DocId(1);
    {
        let mut state = host.state();
        state.bindings.insert(doc, "one".into());
        state.provider_names.insert(doc, "one.psd".into());
        state.provider_revisions.insert(doc, 7);
        state.intents.insert(doc, "save".into());
    }
    assert!(!host.unchanged_provider_save(doc, 7, "one.psd"));
    host.state().destinations.insert(doc, "device".into());
    assert!(!host.unchanged_provider_save(doc, 7, "one.psd"));
    host.state().destinations.insert(doc, "provider".into());
    assert!(host.unchanged_provider_save(doc, 7, "one.psd"));
    assert!(!host.unchanged_provider_save(doc, 8, "one.psd"));
    assert!(!host.unchanged_provider_save(doc, 7, "renamed.psd"));
    host.intent(doc, "saveAs");
    assert!(!host.unchanged_provider_save(doc, 7, "one.psd"));
}
