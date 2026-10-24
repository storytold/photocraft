//! Native render-readiness prerequisites, independent of an XCF decoder or private corpus.
use std::path::PathBuf;

use photocraft_automation::Headless;
use photocraft_doc::{Document, LayerContent, SmartFilter};
use photocraft_engine::Session;
use serde_json::json;

struct Fixture(PathBuf);

impl Fixture {
    fn new(doc: &Document) -> Self {
        Self::bytes("source.pcraft", &photocraft_format::save_to_bytes(doc, &Default::default()).unwrap())
    }

    fn bytes(name: &str, bytes: &[u8]) -> Self {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("pc-native-preparation-{}-{stamp}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join(name), bytes).unwrap();
        Self(dir)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for name in ["source.pcraft", "source.xcf", "alias.pcraft", "converted.pcraft", "first.png"] {
            let _ = std::fs::remove_file(self.0.join(name));
        }
        let _ = std::fs::remove_dir(&self.0);
    }
}

fn pending_native_smart() -> Document {
    let mut session = Session::new();
    session.execute("file.new", json!({"width": 1, "height": 1, "background": "white"})).unwrap();
    session.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let mut doc = (*session.active().unwrap().doc).clone();
    let LayerContent::Smart(object) = &mut doc.layers[0].content else { panic!("native smart conversion expected") };
    object.cache = None;
    doc
}

#[test]
fn headless_open_prepares_native_smart_pixels_before_export() {
    let fixture = Fixture::new(&pending_native_smart());
    let mut headless = Headless::trusted_local();
    headless.open(&fixture.0.join("source.pcraft")).unwrap();
    let doc = &headless.session.active().unwrap().doc;
    assert_eq!(doc.layers[0].surface().unwrap().read_region(doc.bounds()), vec![1.0, 1.0, 1.0, 1.0]);
    assert!(!headless.session.active().unwrap().is_dirty());
}

#[test]
fn native_preparation_obeys_the_sessions_filter_authorization_before_installing() {
    let mut doc = pending_native_smart();
    if let LayerContent::Smart(object) = &mut doc.layers[0].content {
        object.smart_filters.push(SmartFilter {
            command: "filter.blur.gaussianBlur".into(),
            params: json!({"radius": 1}),
            blend: photocraft_doc::BlendMode::Normal,
            opacity: 1.0,
            visible: true,
        });
    }
    let fixture = Fixture::new(&doc);
    let mut headless = Headless::trusted_local();
    headless.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("stored native filter refused".into())));
    assert!(headless.open(&fixture.0.join("source.pcraft")).is_err());
    assert!(headless.session.documents().is_empty());
}

#[test]
fn headless_open_exposes_the_inert_report_summary_without_changing_native_writability() {
    use photocraft_doc::import_conversion::{ConversionReport, ImportConversion};
    let mut doc = pending_native_smart();
    doc.metadata.import_conversion =
        Some(ImportConversion::Report(ConversionReport { source_format: "xcf".into(), source_version: Some(11), policy_version: 1, ..Default::default() }));
    let fixture = Fixture::new(&doc);
    let mut headless = Headless::trusted_local();
    let opened = headless.open(&fixture.0.join("source.pcraft")).unwrap();
    assert_eq!(opened["importConversion"]["sourceFormat"], "xcf");
    assert_eq!(opened["importConversion"]["entryCount"], 0);
    assert!(!headless.session.active().unwrap().source_read_only);
    assert!(headless.session.active().unwrap().path.is_some());
    assert_eq!(headless.session.execute("document.importReport", json!({})).unwrap()["entries"], json!([]));
}

#[test]
fn source_protection_and_native_save_reopen_work_with_trusted_and_scoped_filesystems() {
    let mut bytes = b"gimp xcf v003\0".to_vec();
    for value in [1u32, 1, 0, 0, 0, 0, 0] {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    for scoped in [false, true] {
        let fixture = Fixture::bytes("source.xcf", &bytes);
        std::fs::hard_link(fixture.0.join("source.xcf"), fixture.0.join("alias.pcraft")).unwrap();
        let mut headless = if scoped {
            Headless::with_workspace(photocraft_automation::workspace::AuthorizedWorkspace::new(Some(&fixture.0), Some(&fixture.0)).unwrap())
        } else {
            Headless::trusted_local()
        };
        let path = |name: &str| if scoped { PathBuf::from(name) } else { fixture.0.join(name) };
        // Byte sniffing must also win in the capability-scoped/native-suffix route.
        headless.open(&path("alias.pcraft")).unwrap();
        assert!(headless.session.active().unwrap().source_read_only);
        let opts = photocraft_io::ExportOptions::default();
        assert!(headless.save(None, None, None, &opts).is_err(), "Save requires a native/export destination");
        // Indexed saves protect the document being saved, even with an unrelated tab active.
        headless.session.execute("file.new", json!({"width": 2, "height": 2})).unwrap();
        let active = headless.session.active().unwrap().doc.id;
        for destination in ["source.xcf", "alias.pcraft"] {
            assert!(headless.save(Some(0), Some(&path(destination)), Some("pcraft"), &opts).is_err());
            assert_eq!(headless.session.active().unwrap().doc.id, active);
            assert_eq!(std::fs::read(fixture.0.join("source.xcf")).unwrap(), bytes);
            assert_eq!(std::fs::read(fixture.0.join("alias.pcraft")).unwrap(), bytes);
        }
        headless.session.set_active(0);
        headless.session.execute("layer.new.layer", json!({"name": "Native edit"})).unwrap();
        headless.session.execute("layer.setProps", json!({"opacity": 0.6})).unwrap();
        headless.save(None, Some(&path("converted.pcraft")), None, &opts).unwrap();
        assert!(!headless.session.active().unwrap().source_read_only);
        let report = headless.session.active().unwrap().doc.metadata.import_conversion.clone();
        std::fs::remove_file(fixture.0.join("source.xcf")).unwrap();
        headless.save(None, None, None, &opts).unwrap();
        headless.open(&path("converted.pcraft")).unwrap();
        let state = headless.session.active().unwrap();
        assert!(!state.source_read_only && state.path.is_some());
        assert!(state.protected_import_source.is_none());
        assert_eq!(state.doc.metadata.import_conversion, report);
        headless.session.execute("layer.setProps", json!({"opacity": 0.4})).unwrap();
        headless.session.execute("edit.undo", json!({})).unwrap();
        headless.session.execute("edit.redo", json!({})).unwrap();
        headless.save(None, None, None, &opts).unwrap();
    }
}

/// Independent public XCF18 framing; no corpus or GIMP runtime needed.
struct XcfBytes(Vec<u8>);
impl XcfBytes {
    fn u32(&mut self, value: u32) {
        self.0.extend(value.to_be_bytes());
    }
    fn string(&mut self, value: &str) {
        self.u32(value.len() as u32 + 1);
        self.0.extend(value.as_bytes());
        self.0.push(0);
    }
    fn prop(&mut self, kind: u32, data: &[u8]) {
        self.u32(kind);
        self.u32(data.len() as u32);
        self.0.extend(data);
    }
    fn end(&mut self) {
        self.u32(0);
        self.u32(0);
    }
    fn pointer(&mut self, value: usize) {
        self.0.extend((value as u64).to_be_bytes());
    }
    fn slot(&mut self) -> usize {
        let at = self.0.len();
        self.pointer(0);
        at
    }
    fn patch(&mut self, at: usize, value: usize) {
        self.0[at..at + 8].copy_from_slice(&(value as u64).to_be_bytes());
    }
    fn hierarchy(&mut self, channels: u32, pixels: &[u8]) -> usize {
        let at = self.0.len();
        for value in [4, 3, channels] {
            self.u32(value);
        }
        let level = self.slot();
        self.pointer(0);
        let position = self.0.len();
        self.patch(level, position);
        self.u32(4);
        self.u32(3);
        let tile = self.slot();
        self.pointer(0);
        let position = self.0.len();
        self.patch(tile, position);
        self.0.extend(pixels);
        at
    }
    fn channel(&mut self, name: &str, active: bool) -> usize {
        let at = self.0.len();
        self.u32(4);
        self.u32(3);
        self.string(name);
        if active {
            self.prop(3, &[]);
        }
        self.prop(8, &1u32.to_be_bytes());
        self.end();
        let h = self.slot();
        let pixels = self.hierarchy(1, &[128; 12]);
        self.patch(h, pixels);
        at
    }
}
fn source_view_xcf(active_channel: bool, profile: Option<&[u8]>) -> Vec<u8> {
    let mut w = XcfBytes(b"gimp xcf v018\0".to_vec());
    for value in [4, 3, 0, 150] {
        w.u32(value);
    }
    w.prop(17, &[0]);
    let mut vectors = XcfBytes(Vec::new());
    for value in [1, 0, 1] {
        vectors.u32(value);
    }
    vectors.string("Saved empty path");
    for value in [7, 0, 0, 0, 0] {
        vectors.u32(value);
    }
    w.prop(25, &vectors.0);
    let mut samplers = Vec::new();
    for value in [1i32, 2] {
        samplers.extend(value.to_be_bytes());
    }
    samplers.extend(0u32.to_be_bytes());
    samplers.extend([0; 8]);
    w.prop(39, &samplers);
    w.prop(22, &2u32.to_be_bytes());
    w.prop(9001, &[1, 3, 5, 7]);
    if let Some(profile) = profile {
        let mut parasite = XcfBytes(Vec::new());
        parasite.string("icc-profile");
        parasite.u32(1);
        parasite.u32(profile.len() as u32);
        parasite.0.extend(profile);
        w.prop(21, &parasite.0);
    }
    w.end();
    let selected = w.slot();
    let other = w.slot();
    w.pointer(0);
    let overlay = w.slot();
    w.pointer(0);
    w.pointer(0);
    for (slot, name, masked) in [(selected, "Selected source", true), (other, "Other source", false)] {
        let at = w.0.len();
        w.patch(slot, at);
        for value in [4, 3, 1] {
            w.u32(value);
        }
        w.string(name);
        w.prop(8, &1u32.to_be_bytes());
        w.prop(7, &if masked { 28u32 } else { 5u32 }.to_be_bytes());
        if masked {
            w.prop(2, &[]);
            w.prop(12, &1u32.to_be_bytes());
            w.prop(13, &1u32.to_be_bytes());
        }
        w.end();
        let h = w.slot();
        let mask = w.slot();
        let pixels = w.hierarchy(4, &[20, 40, 60, 255].repeat(12));
        w.patch(h, pixels);
        if masked {
            let at = w.0.len();
            w.patch(mask, at);
            w.u32(4);
            w.u32(3);
            w.string("Selected mask");
            w.end();
            let h = w.slot();
            let pixels = w.hierarchy(1, &[192; 12]);
            w.patch(h, pixels);
        }
    }
    let at = w.channel("Visible alpha", active_channel);
    w.patch(overlay, at);
    w.0
}
fn assert_source_view(session: &Session, alpha: bool) {
    use photocraft_engine::channel_cmds::ChannelTarget;
    let state = session.active().unwrap();
    let active = state.active_layer.unwrap();
    assert_eq!(state.doc.layer(active).unwrap().name, "Selected source");
    assert_eq!(state.selected_layers, vec![active]);
    assert_eq!(state.color_samplers, vec![[1.0, 2.0]]);
    assert_eq!(state.initial_path_selection, Some(0));
    assert_eq!(state.channel_view.target, if alpha { ChannelTarget::Alpha(0) } else { ChannelTarget::LayerMask });
    assert!(state.channel_view.alpha_shown(0));
    assert!(state.channel_view.layer_mask.is_some());
    assert!(!state.is_dirty());
    assert!(!state.history.can_undo());
    assert_eq!(state.revision, state.saved_revision);
}

#[test]
fn files_and_headless_routes_preserve_complete_native_view_and_inert_report_once() {
    use photocraft_automation::{AuthorizedWorkspace, files};
    for alpha in [false, true] {
        let bytes = source_view_xcf(alpha, None);
        let decoded = files::open_bytes("source.xcf", &bytes).unwrap();
        assert!(decoded.source_read_only);
        assert!(!decoded.warnings.is_empty());
        decoded.initial_view.as_ref().unwrap().validate(&decoded.document).unwrap();
        let mut direct = Session::new();
        direct.add_document(decoded.document, None);
        direct.apply_import_initial_view(decoded.initial_view).unwrap();
        assert_source_view(&direct, alpha);
        for (scoped, rpc) in [(false, false), (true, false), (false, true), (true, true)] {
            let fixture = Fixture::bytes("source.xcf", &bytes);
            let mut headless = if scoped {
                Headless::with_workspace(AuthorizedWorkspace::new(Some(&fixture.0), Some(&fixture.0)).unwrap())
            } else {
                Headless::trusted_local()
            };
            let path = if scoped { PathBuf::from("source.xcf") } else { fixture.0.join("source.xcf") };
            let opened = if rpc { headless.handle("doc.open", json!({"path":path.to_str().unwrap()})).unwrap() } else { headless.open(&path).unwrap() };
            assert_source_view(&headless.session, alpha);
            assert!(headless.session.active().unwrap().source_read_only);
            assert!(headless.session.active().unwrap().path.is_none());
            assert_eq!(opened["importConversion"]["sourceFormat"], "xcf");
            assert!(!opened["warnings"].as_array().unwrap().is_empty());
            assert!(opened["color"]["action"].is_string());
            headless.command_run("view.colorSamplers.clear", json!({})).unwrap();
            // The ordinary native sampler command may advance view/document revision.
            // Querying historical import metadata must leave that resulting state alone.
            let before = headless.session.active().unwrap().revision;
            let page = headless.handle("engine.execute", json!({"command":"document.importReport","params":{"offset":0,"limit":1}})).unwrap();
            assert_eq!(page["entries"].as_array().unwrap().len(), 1);
            assert_eq!(page["summary"], opened["importConversion"]);
            assert_eq!(headless.session.active().unwrap().revision, before);
            assert!(headless.session.active().unwrap().color_samplers.is_empty());
            let output = if scoped { PathBuf::from("converted.pcraft") } else { fixture.0.join("converted.pcraft") };
            headless.save(None, Some(&output), None, &Default::default()).unwrap();
            headless.open(&output).unwrap();
            let reopened = headless.session.active().unwrap();
            assert!(reopened.initial_path_selection.is_none());
            assert!(reopened.color_samplers.is_empty());
            assert!(reopened.channel_view.layer_mask.is_none());
            assert!(!reopened.channel_view.alpha_shown(0));
            assert_eq!(reopened.channel_view.target, photocraft_engine::channel_cmds::ChannelTarget::Composite);
            assert!(!reopened.source_read_only);
            assert!(reopened.doc.metadata.import_conversion.is_some());
        }
    }
}

#[test]
fn headless_uses_shared_immediate_color_policy_and_failed_save_open_are_atomic() {
    let mut tagged = Session::new();
    tagged.execute("file.new", json!({"width":4,"height":3})).unwrap();
    tagged.execute("edit.assignProfile", json!({"profile":"display-p3"})).unwrap();
    let profile = tagged.active().unwrap().doc.icc_profile.clone().unwrap();
    let bytes = source_view_xcf(true, Some(&profile));
    let fixture = Fixture::bytes("source.xcf", &bytes);
    let mut h = Headless::trusted_local();
    h.command_run("edit.colorSettings", json!({"policyRgb":"off","askOnMismatch":true})).unwrap();
    let reply = h.open(&fixture.0.join("source.xcf")).unwrap();
    assert_eq!(reply["color"]["action"], "discarded");
    assert!(reply.get("held").is_none(), "synchronous headless applies configured policy; GUI async prompt is a distinct native route");
    assert!(h.session.active().unwrap().doc.icc_profile.is_none());
    assert_source_view(&h.session, true);
    let before = h.session.active().unwrap().clone();
    let active = h.session.active_index();
    assert!(h.save(None, Some(&fixture.0.join("source.xcf/converted.pcraft")), Some("pcraft"), &Default::default()).is_err());
    assert!(h.open(&fixture.0.join("absent.xcf")).is_err());
    assert_eq!(h.session.active_index(), active);
    let after = h.session.active().unwrap();
    assert!(std::sync::Arc::ptr_eq(&before.doc, &after.doc));
    assert_eq!(before.revision, after.revision);
    assert_eq!(before.source_read_only, after.source_read_only);
    assert_eq!(before.protected_import_source, after.protected_import_source);
    assert_eq!(before.channel_view, after.channel_view);
    assert_eq!(before.color_samplers, after.color_samplers);
    assert_eq!(std::fs::read(fixture.0.join("source.xcf")).unwrap(), bytes);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn async_held_color_handoff_is_taken_once_and_cancelled_open_publishes_nothing() {
    use photocraft_engine::jobs::{OpenSource, Started};
    let mut tagged = Session::new();
    tagged.execute("file.new", json!({"width":4,"height":3})).unwrap();
    tagged.execute("edit.assignProfile", json!({"profile":"display-p3"})).unwrap();
    let profile = tagged.active().unwrap().doc.icc_profile.clone().unwrap();
    let bytes = source_view_xcf(false, Some(&profile));
    let mut h = Headless::trusted_local();
    h.command_run("edit.colorSettings", json!({"askOnMismatch":true})).unwrap();
    let Started::Job(job) = h.session.start_open("source.xcf", OpenSource::Bytes(std::sync::Arc::new(bytes.clone()))).unwrap() else {
        panic!("native worker job")
    };
    let reply = h.session.wait_job(job).unwrap();
    let held = reply["held"].as_u64().unwrap();
    assert!(h.session.documents().is_empty());
    assert_eq!(reply["importConversion"]["sourceFormat"], "xcf");
    assert!(!reply["warnings"].as_array().unwrap().is_empty());
    let (mut doc, read_only, view) = h.session.take_held_open_with_view(held).unwrap();
    assert!(read_only);
    assert!(h.session.take_held_open_with_view(held).is_none());
    h.session.color.resolve_open(&mut doc, "preserve").unwrap();
    h.session.open_imported_document(doc, None, &Default::default()).unwrap();
    h.session.apply_import_initial_view(view).unwrap();
    assert_source_view(&h.session, false);
    let before = h.session.active().unwrap().clone();
    let count = h.session.documents().len();
    let Started::Job(job) = h.session.start_open("cancel.xcf", OpenSource::Bytes(std::sync::Arc::new(bytes))).unwrap() else { panic!("native worker job") };
    // Cancellation must win before the RPC synchronization boundary, even if work finished.
    let cancelled = h.handle("jobs.cancel", json!({"job":job.0})).unwrap();
    assert_eq!(cancelled["cancelled"], 1);
    let _ = h.handle("jobs.list", json!({})).unwrap();
    assert!(h.session.wait_job(job).is_err());
    h.session.join_cancelled_jobs();
    assert_eq!(h.session.documents().len(), count);
    let after = h.session.active().unwrap();
    assert!(std::sync::Arc::ptr_eq(&before.doc, &after.doc));
    assert_eq!(before.channel_view, after.channel_view);
    assert_eq!(before.color_samplers, after.color_samplers);
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_xcf_open_report_paging_native_view_and_source_protection_share_headless_results() {
    use photocraft_automation::{AuthorizedWorkspace, PhotocraftMcp};
    use rmcp::model::{CallToolRequestParams, CallToolResult, ClientConfig};
    use rmcp::service::RunningService;
    use rmcp::{ClientHandler, RoleClient, ServiceExt};
    use serde_json::Value;
    #[derive(Clone, Default)]
    struct Client;
    impl ClientHandler for Client {
        fn get_info(&self) -> ClientConfig {
            ClientConfig::default()
        }
    }
    async fn call(client: &RunningService<RoleClient, Client>, name: &str, args: Value) -> CallToolResult {
        let request = CallToolRequestParams::new(name.to_owned()).with_arguments(args.as_object().unwrap().clone());
        client.call_tool(request).await.unwrap()
    }
    fn value(reply: &CallToolResult) -> Value {
        let text = reply.content.iter().filter_map(|entry| entry.as_text()).map(|text| text.text.as_str()).collect::<Vec<_>>().join("\n");
        assert_ne!(reply.is_error, Some(true), "{text}");
        serde_json::from_str(&text).unwrap()
    }
    let bytes = source_view_xcf(true, None);
    let fixture = Fixture::bytes("source.xcf", &bytes);
    let workspace = AuthorizedWorkspace::new(Some(&fixture.0), Some(&fixture.0)).unwrap();
    let server = PhotocraftMcp::headless_with_workspace(workspace);
    let (server_io, client_io) = tokio::io::duplex(1 << 20);
    let server_task = tokio::spawn(async move {
        let running = server.serve(server_io).await.unwrap();
        let _ = running.waiting().await;
    });
    let client = Client.serve(client_io).await.unwrap();
    let opened = value(&call(&client, "doc_open", json!({"path":"source.xcf"})).await);
    assert_eq!(opened["importConversion"]["sourceFormat"], "xcf");
    assert!(!opened["warnings"].as_array().unwrap().is_empty());
    assert!(opened["color"]["action"].is_string());
    let inspected = value(&call(&client, "doc_inspect", json!({})).await);
    assert_eq!(inspected["importConversion"], opened["importConversion"]);
    assert_eq!(inspected["sourceReadOnly"], true);
    assert_eq!(inspected["channels"]["target"], json!({"kind":"alpha","index":0}));
    assert_eq!(inspected["channels"]["alpha"][0]["visible"], true);
    assert!(!inspected["layerMaskView"].is_null());
    assert_eq!(inspected["selectedLayers"].as_array().unwrap().len(), 1);
    let sampler = value(&call(&client, "command_run", json!({"id":"view.colorSamplers.list","params":{}})).await);
    assert_eq!(sampler["samplers"][0]["position"], json!([1.0, 2.0]));
    let traversal = call(&client, "doc_open", json!({"path":"../source.xcf"})).await;
    assert_eq!(traversal.is_error, Some(true));
    let ambient = call(&client, "command_run", json!({"id":"file.openAs","params":{"path":fixture.0.join("source.xcf").to_str().unwrap()}})).await;
    assert_eq!(ambient.is_error, Some(true), "stored/imported context does not grant ambient command paths");
    assert_eq!(value(&call(&client, "doc_inspect", json!({})).await)["revision"], inspected["revision"]);
    let first = value(&call(&client, "command_run", json!({"id":"document.importReport","params":{"offset":0,"limit":1}})).await);
    assert_eq!(first["summary"], opened["importConversion"]);
    assert_eq!(first["entries"].as_array().unwrap().len(), 1);
    if let Some(offset) = first["nextOffset"].as_u64() {
        let next = value(&call(&client, "command_run", json!({"id":"document.importReport","params":{"offset":offset,"limit":1}})).await);
        assert_eq!(next["offset"], offset);
        assert_eq!(next["summary"], first["summary"]);
    }
    let denied = call(&client, "doc_save", json!({"path":"source.xcf","format":"pcraft"})).await;
    assert_eq!(denied.is_error, Some(true));
    assert_eq!(std::fs::read(fixture.0.join("source.xcf")).unwrap(), bytes);
    let _ = value(&call(&client, "doc_save", json!({"path":"converted.pcraft"})).await);
    let _ = value(&call(&client, "doc_open", json!({"path":"converted.pcraft"})).await);
    let reopened = value(&call(&client, "doc_inspect", json!({})).await);
    assert_eq!(reopened["importConversion"], opened["importConversion"]);
    assert_eq!(reopened["sourceReadOnly"], false);
    assert_eq!(reopened["channels"]["target"], json!({"kind":"composite"}));
    assert_eq!(reopened["channels"]["alpha"][0]["visible"], false);
    assert!(reopened["layerMaskView"].is_null());
    let sampler = value(&call(&client, "command_run", json!({"id":"view.colorSamplers.list","params":{}})).await);
    assert!(sampler["samplers"].as_array().unwrap().is_empty());
    let revision = reopened["revision"].clone();
    let _ = value(&call(&client, "command_run", json!({"id":"document.importReport","params":{"limit":1}})).await);
    assert_eq!(value(&call(&client, "doc_inspect", json!({})).await)["revision"], revision);
    client.cancel().await.unwrap();
    server_task.await.unwrap();
}

#[cfg(not(target_arch = "wasm32"))]
#[tokio::test(flavor = "multi_thread")]
async fn mcp_cancel_discards_an_exited_workers_queued_xcf_result_before_publication() {
    use photocraft_automation::{Backend, PhotocraftMcp, files};
    use photocraft_engine::jobs::Started;
    use rmcp::model::{CallToolRequestParams, ClientConfig};
    use rmcp::{ClientHandler, ServiceExt};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };
    struct ExitSignal(Arc<AtomicBool>);
    impl Drop for ExitSignal {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }
    thread_local! {static EXIT_SIGNAL:std::cell::RefCell<Option<ExitSignal>>=const{std::cell::RefCell::new(None)};}
    #[derive(Clone, Default)]
    struct Client;
    impl ClientHandler for Client {
        fn get_info(&self) -> ClientConfig {
            ClientConfig::default()
        }
    }
    for generic in [false, true] {
        let headless = Arc::new(Mutex::new(Headless::new()));
        let server = PhotocraftMcp::with_backend(Backend::Headless(headless.clone()));
        let (server_io, client_io) = tokio::io::duplex(1 << 20);
        let server_task = tokio::spawn(async move {
            let running = server.serve(server_io).await.unwrap();
            let _ = running.waiting().await;
        });
        let client = Client.serve(client_io).await.unwrap();
        let exited = Arc::new(AtomicBool::new(false));
        let signal = exited.clone();
        let bytes = source_view_xcf(false, None);
        let Started::Job(job) = headless
            .lock()
            .unwrap()
            .session
            .start_job(
                "test.nativeXcfOpen",
                json!({}),
                "Queued native XCF",
                false,
                move |_| {
                    // Engine queues the result before its dedicated worker returns. Rust drops this
                    // thread-local guard at thread exit, so this signal proves the result is queued.
                    // Atomic store alone avoids blocking/locking in a Windows TLS destructor.
                    EXIT_SIGNAL.with(|slot| *slot.borrow_mut() = Some(ExitSignal(signal)));
                    files::open_bytes("queued.xcf", &bytes).map_err(|e| photocraft_engine::EngineError::Other(e.to_string()))
                },
                |session, opened| {
                    let (index, color) = session.open_imported_document(opened.document, None, &Default::default())?;
                    session.apply_import_initial_view(opened.initial_view)?;
                    Ok(json!({"document":index,"color":color}))
                },
            )
            .unwrap()
        else {
            panic!("dedicated native worker")
        };
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !exited.load(Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("worker exited after queueing its source result");
        assert!(headless.lock().unwrap().session.documents().is_empty());
        let (tool, args) = if generic { ("command_run", json!({"id":"jobs.cancel","params":{"job":job.0}})) } else { ("jobs_cancel", json!({"job":job.0})) };
        let request = CallToolRequestParams::new(tool.to_owned()).with_arguments(args.as_object().unwrap().clone());
        let reply = client.call_tool(request).await.unwrap();
        let text = reply.content.iter().filter_map(|entry| entry.as_text()).map(|entry| entry.text.as_str()).collect::<Vec<_>>().join("\n");
        assert_ne!(reply.is_error, Some(true), "{text}");
        let result: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(result["cancelled"], 1);
        {
            let mut h = headless.lock().unwrap();
            h.sync_jobs();
            h.session.join_cancelled_jobs();
            assert!(h.session.documents().is_empty());
            assert!(h.session.jobs_with_recent().iter().any(|job| job.state == "cancelled"));
        }
        client.cancel().await.unwrap();
        server_task.await.unwrap();
    }
}

/// Independently authored v26 framing, original pixels, and one typed stored
/// adjustment. No native document bytes or external corpus define this input.
fn stored_native_effect_xcf(operation: &str) -> Vec<u8> {
    struct W(Vec<u8>);
    impl W {
        fn word(&mut self, v: u32) {
            self.0.extend(v.to_be_bytes());
        }
        fn ptr(&mut self, v: usize) {
            self.0.extend((v as u64).to_be_bytes());
        }
        fn slot(&mut self) -> usize {
            let at = self.0.len();
            self.ptr(0);
            at
        }
        fn patch(&mut self, at: usize) {
            let value = self.0.len() as u64;
            self.0[at..at + 8].copy_from_slice(&value.to_be_bytes());
        }
        fn text(&mut self, v: &str) {
            self.word(v.len() as u32 + 1);
            self.0.extend(v.as_bytes());
            self.0.push(0);
        }
        fn prop(&mut self, kind: u32, data: &[u8]) {
            self.word(kind);
            self.word(data.len() as u32);
            self.0.extend(data);
        }
        fn end(&mut self) {
            self.word(0);
            self.word(0);
        }
    }
    let mut w = W(b"gimp xcf v026\0".to_vec());
    for value in [4, 3, 0, 150] {
        w.word(value);
    }
    w.prop(17, &[0]);
    w.prop(9001, &[1, 3, 5, 7]);
    w.end();
    let layer = w.slot();
    w.ptr(0);
    w.ptr(0);
    w.ptr(0);
    w.patch(layer);
    for value in [4, 3, 1] {
        w.word(value);
    }
    w.text("Stored filter owner");
    w.end();
    let hierarchy = w.slot();
    w.ptr(0);
    let effect = w.slot();
    w.ptr(0);
    w.patch(hierarchy);
    for value in [4, 3, 4] {
        w.word(value);
    }
    let level = w.slot();
    w.ptr(0);
    w.patch(level);
    w.word(4);
    w.word(3);
    let tile = w.slot();
    w.ptr(0);
    w.patch(tile);
    w.0.extend([64, 128, 192, 128].repeat(12));
    w.patch(effect);
    w.text("Original stored adjustment");
    w.text("");
    w.text(operation);
    w.text(if operation == "gegl:exposure" { "1:0" } else { "0:0" });
    if operation == "gegl:exposure" {
        let mut arg = W(Vec::new());
        arg.text("exposure");
        arg.word(3);
        arg.0.extend(1f32.to_be_bytes());
        w.prop(45, &arg.0);
    }
    w.end();
    w.ptr(0);
    w.0
}
/// A fresh ordinary native command chain on the original pixels is the oracle;
/// import output, report data and source-operation dispatch are never consulted.
fn independently_native_stored_filter_samples(operation: &str) -> Vec<f32> {
    let mut native = Session::new();
    native.execute("file.new", json!({"width":4,"height":3,"background":"transparent"})).unwrap();
    native.execute("edit.assignProfile", json!({"profile":"srgb"})).unwrap();
    let doc = std::sync::Arc::make_mut(&mut native.active_mut().unwrap().doc);
    let bounds = doc.bounds();
    doc.layers[0].surface_mut().unwrap().write_region(bounds, &[64u8, 128, 192, 128].repeat(12).iter().map(|v| f32::from(*v) / 255.).collect::<Vec<_>>());
    native.execute("filter.convertForSmartFilters", json!({})).unwrap();
    if operation == "gegl:exposure" {
        native.execute("image.adjustments.exposure", json!({"exposure":1.,"offset":0.,"gamma":1.})).unwrap();
    } else {
        native.execute("image.adjustments.invert", json!({})).unwrap();
    }
    let doc = &native.active().unwrap().doc;
    assert!(doc.metadata.import_conversion.is_none());
    doc.layers[0].surface().unwrap().read_region(doc.bounds())
}
fn assert_stored_filter_pixels(actual: &[f32], expected: &[f32], route: &str) {
    assert_eq!(actual.len(), expected.len(), "{route}: sample count");
    let max = actual.iter().zip(expected).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
    assert!(actual.iter().all(|v| v.is_finite()) && max <= 2. / 255., "{route}: native counterpart max sample error {max}");
}

#[test]
fn actual_stored_xcf_adjustments_trusted_and_scoped_headless_match_native_commands() {
    use photocraft_doc::import_conversion::{ConversionDisposition, ImportConversion};
    for operation in ["gegl:invert-gamma", "gegl:exposure"] {
        for scoped in [false, true] {
            let bytes = stored_native_effect_xcf(operation);
            let fixture = Fixture::bytes("source.xcf", &bytes);
            let mut h = if scoped {
                Headless::with_workspace(photocraft_automation::AuthorizedWorkspace::new(Some(&fixture.0), Some(&fixture.0)).unwrap())
            } else {
                Headless::trusted_local()
            };
            let path = |name: &str| if scoped { PathBuf::from(name) } else { fixture.0.join(name) };
            let opened = h.open(&path("source.xcf")).unwrap();
            assert_eq!(opened["importConversion"]["sourceFormat"], "xcf");
            let expected = independently_native_stored_filter_samples(operation);
            let state = h.session.active().unwrap();
            assert!(state.source_read_only);
            let LayerContent::Smart(smart) = &state.doc.layers[0].content else { panic!("newly emitted native smart source") };
            assert!(smart.cache.is_some());
            assert_stored_filter_pixels(&state.doc.layers[0].surface().unwrap().read_region(state.doc.bounds()), &expected, operation);
            let Some(ImportConversion::Report(report)) = &state.doc.metadata.import_conversion else { panic!("source-only report") };
            assert!(report.entries.iter().any(|row| row.feature_code == "xcf.layer_effect" && row.disposition == ConversionDisposition::NativeApproximation));
            assert!(
                report.entries.iter().any(|row| row.feature_code == "xcf.source_property.9001" && row.disposition == ConversionDisposition::RetainedUnapplied)
            );
            let report = state.doc.metadata.import_conversion.clone();
            let opts = photocraft_io::ExportOptions::default();
            h.save(None, Some(&path("first.png")), Some("png"), &opts).unwrap();
            let png = photocraft_io::import("first.png", &std::fs::read(fixture.0.join("first.png")).unwrap()).unwrap().document;
            assert_stored_filter_pixels(&png.layers[0].surface().unwrap().read_region(png.bounds()), &expected, operation);
            h.save(None, Some(&path("converted.pcraft")), None, &opts).unwrap();
            assert_eq!(std::fs::read(fixture.0.join("source.xcf")).unwrap(), bytes);
            std::fs::remove_file(fixture.0.join("source.xcf")).unwrap();
            drop(bytes);
            h.open(&path("converted.pcraft")).unwrap();
            let state = h.session.active().unwrap();
            assert!(!state.source_read_only);
            assert_eq!(state.doc.metadata.import_conversion, report);
            assert_stored_filter_pixels(&state.doc.layers[0].surface().unwrap().read_region(state.doc.bounds()), &expected, operation);
            h.session.execute("layer.setProps", json!({"opacity":0.25})).unwrap();
            h.session.execute("edit.undo", json!({})).unwrap();
            h.save(None, None, None, &opts).unwrap();
            assert_stored_filter_pixels(
                &h.session.active().unwrap().doc.layers[0].surface().unwrap().read_region(h.session.active().unwrap().doc.bounds()),
                &expected,
                operation,
            );
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[tokio::test(flavor = "multi_thread")]
async fn actual_stored_xcf_adjustments_mcp_first_output_and_source_independent_native_reopen() {
    use photocraft_automation::{AuthorizedWorkspace, Backend, PhotocraftMcp};
    use rmcp::model::{CallToolRequestParams, CallToolResult, ClientConfig};
    use rmcp::service::RunningService;
    use rmcp::{ClientHandler, RoleClient, ServiceExt};
    use serde_json::Value;
    use std::sync::{Arc, Mutex};
    #[derive(Clone, Default)]
    struct Client;
    impl ClientHandler for Client {
        fn get_info(&self) -> ClientConfig {
            ClientConfig::default()
        }
    }
    async fn call(client: &RunningService<RoleClient, Client>, name: &str, args: Value) -> CallToolResult {
        client.call_tool(CallToolRequestParams::new(name.to_owned()).with_arguments(args.as_object().unwrap().clone())).await.unwrap()
    }
    fn value(reply: &CallToolResult) -> Value {
        let text = reply.content.iter().filter_map(|entry| entry.as_text()).map(|entry| entry.text.as_str()).collect::<Vec<_>>().join("\n");
        assert_ne!(reply.is_error, Some(true), "{text}");
        serde_json::from_str(&text).unwrap()
    }
    for operation in ["gegl:invert-gamma", "gegl:exposure"] {
        for scoped in [false, true] {
            let bytes = stored_native_effect_xcf(operation);
            let fixture = Fixture::bytes("source.xcf", &bytes);
            let h = if scoped {
                Headless::with_workspace(AuthorizedWorkspace::new(Some(&fixture.0), Some(&fixture.0)).unwrap())
            } else {
                Headless::trusted_local()
            };
            let backend = Arc::new(Mutex::new(h));
            let server = PhotocraftMcp::with_backend(Backend::Headless(backend.clone()));
            let (server_io, client_io) = tokio::io::duplex(1 << 20);
            let server_task = tokio::spawn(async move {
                let running = server.serve(server_io).await.unwrap();
                let _ = running.waiting().await;
            });
            let client = Client.serve(client_io).await.unwrap();
            let path = |name: &str| if scoped { name.to_owned() } else { fixture.0.join(name).to_string_lossy().into_owned() };
            let opened = value(&call(&client, "doc_open", json!({"path":path("source.xcf")})).await);
            assert_eq!(opened["importConversion"]["sourceFormat"], "xcf");
            let inspected = value(&call(&client, "doc_inspect", json!({})).await);
            assert_eq!(inspected["sourceReadOnly"], true);
            let expected = independently_native_stored_filter_samples(operation);
            let report = {
                let h = backend.lock().unwrap();
                let state = h.session.active().unwrap();
                let LayerContent::Smart(smart) = &state.doc.layers[0].content else { panic!("new native smart source") };
                assert!(smart.cache.is_some());
                assert_stored_filter_pixels(&state.doc.layers[0].surface().unwrap().read_region(state.doc.bounds()), &expected, operation);
                state.doc.metadata.import_conversion.clone()
            };
            let page = value(&call(&client, "command_run", json!({"id":"document.importReport","params":{"limit":250}})).await);
            assert!(
                page["entries"].as_array().unwrap().iter().any(|row| row["featureCode"] == "xcf.layer_effect" && row["disposition"] == "native_approximation")
            );
            assert!(
                page["entries"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|row| row["featureCode"] == "xcf.source_property.9001" && row["disposition"] == "retained_unapplied")
            );
            let denied = call(&client, "doc_save", json!({"path":path("source.xcf"),"format":"pcraft"})).await;
            assert_eq!(denied.is_error, Some(true));
            let _ = value(&call(&client, "doc_save", json!({"path":path("first.png"),"format":"png"})).await);
            let png = photocraft_io::import("first.png", &std::fs::read(fixture.0.join("first.png")).unwrap()).unwrap().document;
            assert_stored_filter_pixels(&png.layers[0].surface().unwrap().read_region(png.bounds()), &expected, operation);
            let _ = value(&call(&client, "doc_save", json!({"path":path("converted.pcraft")})).await);
            assert_eq!(std::fs::read(fixture.0.join("source.xcf")).unwrap(), bytes);
            std::fs::remove_file(fixture.0.join("source.xcf")).unwrap();
            drop(bytes);
            let _ = value(&call(&client, "doc_open", json!({"path":path("converted.pcraft")})).await);
            let reopened = value(&call(&client, "doc_inspect", json!({})).await);
            assert_eq!(reopened["sourceReadOnly"], false);
            assert_eq!(reopened["importConversion"], opened["importConversion"]);
            {
                let h = backend.lock().unwrap();
                let state = h.session.active().unwrap();
                assert_eq!(state.doc.metadata.import_conversion, report);
                assert_stored_filter_pixels(&state.doc.layers[0].surface().unwrap().read_region(state.doc.bounds()), &expected, operation);
            }
            let _ = value(&call(&client, "command_run", json!({"id":"layer.setProps","params":{"opacity":0.25}})).await);
            let _ = value(&call(&client, "command_run", json!({"id":"edit.undo","params":{}})).await);
            let _ = value(&call(&client, "doc_save", json!({})).await);
            client.cancel().await.unwrap();
            server_task.await.unwrap();
        }
    }
}
