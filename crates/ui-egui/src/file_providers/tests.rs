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
    tick(app, &egui::Context::default());
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
    let document = app.session.active().unwrap().doc.as_ref().clone();
    app.services.import = Some(Box::new(move |_, _, _| Ok((document.clone(), Vec::new()))));
    app.background_jobs = true;
    let request = host.request_open("resource-one").unwrap();
    host.take_events();
    host.open(request.clone(), "loaded.png".into(), vec![1]).unwrap();
    assert!(host.busy());
    tick_once(&mut app);
    assert!(app.background_jobs, "the provider-only synchronous path restores the platform policy");
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
