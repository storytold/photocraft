//! Capability-scoped XCF sample interpretation through real RPC and MCP controls.
//! The tiny source below is authored from the public XCF framing specification.
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use photocraft_automation::{AuthorizedWorkspace, Backend, Headless, PhotocraftMcp};
use rmcp::model::{CallToolRequestParams, CallToolResult, ClientConfig};
use rmcp::service::RunningService;
use rmcp::{ClientHandler, RoleClient, ServiceExt};
use serde_json::{Value, json};

struct Fixture {
    root: PathBuf,
    read: PathBuf,
    outside: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!("pc-xcf-controls-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        let fixture = Self { read: root.join("read"), outside: root.join("outside"), root };
        std::fs::create_dir(&fixture.root).unwrap();
        for folder in [&fixture.read, &fixture.outside] {
            std::fs::create_dir(folder).unwrap();
            std::fs::write(folder.join("source.xcf"), historical_rgba16()).unwrap();
        }
        fixture
    }
    fn headless(&self) -> Headless {
        let mut headless = Headless::with_workspace(AuthorizedWorkspace::new(Some(&self.read), Some(&self.read)).unwrap());
        headless.command_run("file.new", json!({"width":2,"height":2,"background":"white"})).unwrap();
        headless
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        // Only files created by this fixture; never recursively remove a caller path.
        for folder in [&self.read, &self.outside] {
            for name in ["source.xcf", "replay.pcraft"] {
                let _ = std::fs::remove_file(folder.join(name));
            }
            let _ = std::fs::remove_dir(folder);
        }
        let _ = std::fs::remove_dir(&self.root);
    }
}

/// A single raw RGBA16 tile in a version-10 file. Structural words are always BE;
/// the intentionally asymmetric sample bytes let the command's explicit choice be observed.
fn historical_rgba16() -> Vec<u8> {
    fn word(out: &mut Vec<u8>, value: u32) {
        out.extend_from_slice(&value.to_be_bytes());
    }
    fn patch(out: &mut [u8], slot: usize, target: usize) {
        out[slot..slot + 4].copy_from_slice(&(target as u32).to_be_bytes());
    }
    let mut out = b"gimp xcf v010\0".to_vec();
    for value in [1, 1, 0, 250, 17, 1] {
        word(&mut out, value);
    }
    out.push(0); // no compression
    for value in [0, 0] {
        word(&mut out, value);
    }
    let layer = out.len();
    for value in [0, 0, 0] {
        word(&mut out, value);
    }
    let at = out.len();
    patch(&mut out, layer, at);
    for value in [1, 1, 1, 2] {
        word(&mut out, value);
    }
    out.extend_from_slice(b"L\0");
    for value in [0, 0] {
        word(&mut out, value);
    }
    let hierarchy = out.len();
    for value in [0, 0] {
        word(&mut out, value);
    }
    let at = out.len();
    patch(&mut out, hierarchy, at);
    for value in [1, 1, 8] {
        word(&mut out, value);
    }
    let level = out.len();
    for value in [0, 0] {
        word(&mut out, value);
    }
    let at = out.len();
    patch(&mut out, level, at);
    for value in [1, 1] {
        word(&mut out, value);
    }
    let tile = out.len();
    for value in [0, 0] {
        word(&mut out, value);
    }
    let at = out.len();
    patch(&mut out, tile, at);
    out.extend_from_slice(&[0x12, 0x34, 0x23, 0x45, 0x34, 0x56, 0xff, 0xff]);
    out
}

fn assert_pixel(headless: &Headless, order: &str) {
    let expected = if order == "big-endian" { [0x1234u16, 0x2345, 0x3456] } else { [0x3412u16, 0x4523, 0x5634] };
    let state = headless.session.active().unwrap();
    let pixel = state.doc.layers[0].surface().unwrap().pixel(0, 0);
    assert_eq!(pixel, [f32::from(expected[0]) / 65535., f32::from(expected[1]) / 65535., f32::from(expected[2]) / 65535., 1.], "{order}");
    assert_eq!(state.doc.depth, photocraft_color::SampleType::U16);
}

#[test]
fn scoped_rpc_explicit_order_preserves_exact_pixels_warnings_and_native_replay() {
    let fixture = Fixture::new();
    let bytes = std::fs::read(fixture.read.join("source.xcf")).unwrap();
    let mut headless = fixture.headless();
    for order in ["little-endian", "big-endian"] {
        let params = json!({"path":"source.xcf","historicalXcfSampleOrder":order,"approveHighMemory":false});
        let expected =
            photocraft_io::import_with_options("source.xcf", &bytes, &photocraft_automation::files::import_options_from_params(&params).unwrap()).unwrap();
        let result = headless.handle("doc.open", params).unwrap();
        assert_eq!(result["warnings"], json!(expected.warnings));
        assert_pixel(&headless, order);
        assert!(headless.session.active().unwrap().source_read_only);
        assert!(headless.session.active().unwrap().path.is_none());
        headless.handle("doc.save", json!({"path":"replay.pcraft"})).unwrap();
        headless.handle("doc.open", json!({"path":"replay.pcraft"})).unwrap();
        assert_pixel(&headless, order);
        assert!(!headless.session.active().unwrap().source_read_only);
        assert_eq!(std::fs::read(fixture.read.join("source.xcf")).unwrap(), bytes);
    }
}

#[test]
fn scoped_rpc_denials_and_unanswered_orders_leave_the_active_document_unchanged() {
    let fixture = Fixture::new();
    let mut headless = fixture.headless();
    let before = headless.session.active().unwrap().doc.clone();
    let listing = headless.session_list();
    for params in [json!({"path":"source.xcf"}), json!({"path":"source.xcf","historicalXcfSampleOrder":null})] {
        let error = headless.handle("doc.open", params).unwrap_err();
        assert!(matches!(error, photocraft_automation::AutomationError::Import(photocraft_io::IoError::HistoricalXcfSampleOrderRequired(_))));
        assert!(Arc::ptr_eq(&before, &headless.session.active().unwrap().doc));
        assert_eq!(headless.session_list(), listing);
    }
    for params in [
        json!({"path":"../outside/source.xcf","historicalXcfSampleOrder":"little-endian"}),
        json!({"path":fixture.outside.join("source.xcf"),"historicalXcfSampleOrder":"big-endian"}),
        json!({"path":"missing.xcf","historicalXcfSampleOrder":"native"}),
        json!({"path":"missing.xcf","historicalXcfSampleOrder":1}),
        json!({"path":"missing.xcf","approveHighMemory":"true"}),
    ] {
        assert!(headless.handle("doc.open", params).is_err());
        assert!(Arc::ptr_eq(&before, &headless.session.active().unwrap().doc));
        assert_eq!(headless.session_list(), listing);
    }
    for invalid in [json!("native"), json!(1), json!(false)] {
        let error = headless.handle("doc.open", json!({"path":"nonexistent-order-control.xcf","historicalXcfSampleOrder":invalid})).unwrap_err();
        assert!(
            matches!(error,photocraft_automation::AutomationError::Engine(photocraft_engine::EngineError::BadParams { ref cmd, ref msg }) if cmd=="doc.open" && msg.contains("historicalXcfSampleOrder"))
        );
        assert!(Arc::ptr_eq(&before, &headless.session.active().unwrap().doc));
    }
    // An explicit value cannot grant filesystem authority to an otherwise denied host.
    let mut denied = Headless::new();
    assert!(denied.handle("doc.open", json!({"path":fixture.read.join("source.xcf"),"historicalXcfSampleOrder":"little-endian"})).is_err());
    assert!(denied.session.documents().is_empty());
    assert_eq!(std::fs::read(fixture.outside.join("source.xcf")).unwrap(), historical_rgba16());
}

#[derive(Clone, Default)]
struct Client;
impl ClientHandler for Client {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::default()
    }
}
async fn connect(server: PhotocraftMcp) -> RunningService<RoleClient, Client> {
    let (server_io, client_io) = tokio::io::duplex(1 << 20);
    tokio::spawn(async move {
        if let Ok(server) = server.serve(server_io).await {
            let _ = server.waiting().await;
        }
    });
    Client.serve(client_io).await.unwrap()
}
async fn tool(client: &RunningService<RoleClient, Client>, name: &str, args: Value) -> CallToolResult {
    client.call_tool(CallToolRequestParams::new(name.to_owned()).with_arguments(args.as_object().unwrap().clone())).await.unwrap()
}
async fn call(client: &RunningService<RoleClient, Client>, args: Value) -> CallToolResult {
    tool(client, "doc_open", args).await
}
fn reply(result: &CallToolResult) -> Value {
    let text = result.content.iter().filter_map(|content| content.as_text()).map(|content| content.text.as_str()).collect::<String>();
    serde_json::from_str(&text).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn scoped_mcp_order_parameter_uses_the_same_authority_pixels_and_warnings_as_rpc() {
    let fixture = Fixture::new();
    let state = Arc::new(Mutex::new(fixture.headless()));
    let client = connect(PhotocraftMcp::with_backend(Backend::Headless(state.clone()))).await;
    for order in ["little-endian", "big-endian"] {
        let result = call(&client, json!({"path":"source.xcf","historical_xcf_sample_order":order,"approve_high_memory":true})).await;
        assert_ne!(result.is_error, Some(true), "{result:?}");
        let mut direct = fixture.headless();
        let expected = direct.handle("doc.open", json!({"path":"source.xcf","historicalXcfSampleOrder":order,"approveHighMemory":true})).unwrap();
        assert_eq!(reply(&result)["warnings"], expected["warnings"]);
        assert_pixel(&state.lock().unwrap(), order);
    }
    let before = state.lock().unwrap().session.active().unwrap().doc.clone();
    for params in [
        json!({"path":"source.xcf"}),
        json!({"path":"source.xcf","historical_xcf_sample_order":null}),
        json!({"path":"../outside/source.xcf","historical_xcf_sample_order":"little-endian"}),
        json!({"path":"missing.xcf","historical_xcf_sample_order":"native"}),
    ] {
        let result = call(&client, params).await;
        assert_eq!(result.is_error, Some(true), "{result:?}");
        assert!(Arc::ptr_eq(&before, &state.lock().unwrap().session.active().unwrap().doc));
    }
    assert_eq!(std::fs::read(fixture.read.join("source.xcf")).unwrap(), historical_rgba16());
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn rpc_and_mcp_publish_the_same_typed_question_without_inventing_a_held_capability() {
    let fixture = Fixture::new();
    let state = Arc::new(Mutex::new(fixture.headless()));
    let before = state.lock().unwrap().session.active().unwrap().doc.clone();
    let request = json!({"id":17,"method":"doc.open","params":{"path":"source.xcf"}}).to_string();
    let rpc = photocraft_automation::rpc::respond(&state, &request);
    assert_eq!(rpc["ok"], false);
    let question = json!({"version":10,"bitsPerSample":16});
    assert_eq!(rpc["historicalXcfSampleOrderRequired"], question);
    assert!(rpc.get("heldHistoricalXcfOpen").is_none());
    let client = connect(PhotocraftMcp::with_backend(Backend::Headless(state.clone()))).await;
    let mcp = call(&client, json!({"path":"source.xcf"})).await;
    assert_eq!(mcp.is_error, Some(true));
    let mcp = reply(&mcp);
    assert_eq!(mcp["historicalXcfSampleOrderRequired"], question);
    assert_eq!(mcp["error"], rpc["error"]);
    assert!(mcp.get("heldHistoricalXcfOpen").is_none());
    assert!(Arc::ptr_eq(&before, &state.lock().unwrap().session.active().unwrap().doc));
    client.cancel().await.unwrap();
}

/// Another producer can place an engine-owned hold in a shared session. Merely
/// knowing its identity does not grant the headless backend authority to answer it.
fn outside_backend_hold(headless: &mut Headless, bytes: Vec<u8>) -> u64 {
    let options = photocraft_io::ImportOptions { historical_xcf_order: photocraft_io::HistoricalXcfOrder::RequireExplicit, ..Default::default() };
    let started = headless
        .session
        .start_open_for_user_with_options("external-choice.xcf", photocraft_engine::jobs::OpenSource::Bytes(Arc::new(bytes)), &options, false)
        .unwrap();
    let photocraft_engine::jobs::Started::Done(question) = started else { panic!("inline question expected") };
    let id = question["heldHistoricalXcfOpen"].as_u64().unwrap();
    assert!(headless.session.holds_historical_xcf_open(id));
    id
}

#[tokio::test(flavor = "multi_thread")]
async fn automation_cannot_answer_external_or_foreign_holds_or_use_outside_roots() {
    let fixture = Fixture::new();
    let mut headless = fixture.headless();
    // The source is deliberately outside this backend's granted read root, as a
    // GUI/user-origin source could be. Only the test's external producer reads it.
    let external = outside_backend_hold(&mut headless, std::fs::read(fixture.outside.join("source.xcf")).unwrap());
    let mut foreign_session = fixture.headless();
    let foreign = outside_backend_hold(&mut foreign_session, historical_rgba16());
    let before = headless.session.active().unwrap().doc.clone();
    for id in [external, foreign, u64::MAX] {
        for params in [json!({"id":id,"historicalXcfSampleOrder":"big-endian","background":false}), json!({"id":id,"cancel":true})] {
            assert!(headless.command_run("file.resolveHistoricalXcfOpen", params).is_err());
            assert!(headless.session.holds_historical_xcf_open(external));
            assert!(Arc::ptr_eq(&before, &headless.session.active().unwrap().doc));
        }
    }
    let error = headless.handle("doc.open", json!({"path":fixture.outside.join("source.xcf"),"historicalXcfSampleOrder":"big-endian"})).unwrap_err();
    assert!(matches!(error, photocraft_automation::AutomationError::BadRequest(_)), "outside roots fail before import");
    let state = Arc::new(Mutex::new(headless));
    let client = connect(PhotocraftMcp::with_backend(Backend::Headless(state.clone()))).await;
    for params in [json!({"id":external,"historicalXcfSampleOrder":"little-endian","background":false}), json!({"id":external,"cancel":true})] {
        let result = tool(&client, "command_run", json!({"id":"file.resolveHistoricalXcfOpen","params":params,"wait":true})).await;
        assert_eq!(result.is_error, Some(true));
        assert!(state.lock().unwrap().session.holds_historical_xcf_open(external));
        assert!(Arc::ptr_eq(&before, &state.lock().unwrap().session.active().unwrap().doc));
    }
    let denied = call(&client, json!({"path":fixture.outside.join("source.xcf"),"historical_xcf_sample_order":"little-endian"})).await;
    assert_eq!(denied.is_error, Some(true));
    assert!(reply(&denied).get("historicalXcfSampleOrderRequired").is_none(), "no source question is exposed without read authority");
    assert!(foreign_session.session.holds_historical_xcf_open(foreign));
    assert_eq!(std::fs::read(fixture.outside.join("source.xcf")).unwrap(), historical_rgba16());
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn bridge_keeps_held_identity_question_and_warnings_through_existing_dialog_controls() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    // Transport coverage only: the remote host remains responsible for its queue
    // and authorization. A held response never becomes a headless capability.
    let id = 9_007_199_254_740_999u64;
    let question =
        json!({"heldHistoricalXcfOpen":id,"historicalXcfSampleOrderRequired":{"version":10,"bitsPerSample":16},"warnings":["source choice is pending"]});
    let dialog = "0011223344556677";
    let finished = json!({"id":id,"result":{"docId":83,"width":1,"height":1,"warnings":["explicit big-endian source"]}});
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let bridge =
        photocraft_automation::BridgeClient::new(listener.local_addr().unwrap().to_string(), TOKEN).unwrap().with_timeout(std::time::Duration::from_secs(5));
    let expected_question = question.clone();
    let expected_finished = finished.clone();
    let app = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let (read, mut write) = socket.into_split();
        let mut lines = tokio::io::BufReader::new(read).lines();
        let mut handshake = photocraft_automation::security::ServerHandshake::new();
        loop {
            let line = lines.next_line().await.unwrap().unwrap();
            let (reply, step) = handshake.reply(&line, TOKEN);
            write.write_all(format!("{reply}\n").as_bytes()).await.unwrap();
            if step != photocraft_automation::security::AuthStep::Challenged {
                assert_eq!(step, photocraft_automation::security::AuthStep::Authenticated);
                break;
            }
        }
        let requests = [
            ("app.open", json!({"path":"source.xcf"}), expected_question),
            ("ui.inspect", json!({}), json!({"dialogs":[{"id":dialog,"fields":{"request":id,"historicalXcfSampleOrder":""}}]})),
            ("ui.dialog.set", json!({"dialog":dialog,"field":"historicalXcfSampleOrder","value":"big-endian"}), Value::Null),
            ("ui.dialog.confirm", json!({"dialog":dialog}), expected_finished),
        ];
        for (method, params, result) in requests {
            let request: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(request["method"], method);
            assert_eq!(request["params"], params);
            let reply = json!({"id":request["id"],"ok":true,"result":result});
            write.write_all(format!("{reply}\n").as_bytes()).await.unwrap();
        }
    });
    let client = connect(PhotocraftMcp::with_backend(Backend::Bridge(Arc::new(bridge)))).await;
    let opened = call(&client, json!({"path":"source.xcf"})).await;
    assert_ne!(opened.is_error, Some(true));
    assert_eq!(reply(&opened), question);
    let inspected = tool(&client, "control_call", json!({"method":"ui.inspect"})).await;
    let actual_dialog = reply(&inspected)["dialogs"][0]["id"].clone();
    assert_eq!(reply(&inspected)["dialogs"][0]["fields"]["request"], id);
    let set = tool(
        &client,
        "control_call",
        json!({"method":"ui.dialog.set","params":{"dialog":actual_dialog,"field":"historicalXcfSampleOrder","value":"big-endian"}}),
    )
    .await;
    assert_ne!(set.is_error, Some(true));
    let confirmed = tool(&client, "control_call", json!({"method":"ui.dialog.confirm","params":{"dialog":actual_dialog}})).await;
    assert_ne!(confirmed.is_error, Some(true));
    assert_eq!(reply(&confirmed), finished);
    client.cancel().await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), app).await.unwrap().unwrap();
}

fn native_masked_source() -> Vec<u8> {
    photocraft_io::native_xcf_tones::fixtures::masked_exposure(Default::default()).bytes
}
fn native_owner(document: &photocraft_doc::Document) -> &photocraft_doc::SmartObject {
    let photocraft_doc::LayerContent::Smart(owner) = &document.layers[0].content else { panic!("authorized effect remains independently editable") };
    assert_eq!(owner.smart_filters.len(), 1);
    assert_eq!(owner.smart_filters[0].command, "image.adjustments.exposure");
    assert!(owner.filter_mask.is_some());
    owner
}
fn report_without_native_ids(document: &photocraft_doc::Document) -> photocraft_doc::import_conversion::ConversionReport {
    use photocraft_doc::import_conversion::ImportConversion;
    let Some(ImportConversion::Report(report)) = document.metadata.import_conversion.as_deref() else { panic!("source report required") };
    let mut report = report.clone();
    // Separate imports allocate separate native identities; every other source,
    // mapping, attachment and native target fact must agree.
    for entry in &mut report.entries {
        for target in &mut entry.targets {
            target.layer_id = None;
        }
    }
    report
}

#[test]
fn rooted_headless_native_effects_match_engine_open_and_survive_native_replay() {
    let fixture = Fixture::new();
    let source = native_masked_source();
    std::fs::write(fixture.read.join("source.xcf"), &source).unwrap();
    let options = photocraft_automation::files::import_options_from_params(&json!({})).unwrap();
    let mut engine = photocraft_engine::Session::new();
    engine.execute("edit.colorSettings", json!({"askOnMismatch":false,"askOnMissing":false,"policyRgb":"preserve"})).unwrap();
    let reference = photocraft_engine::file_cmds::open_bytes_as_with_import_options(&mut engine, "source.xcf", &source, None, None, &options).unwrap();
    let expected = engine.active().unwrap().doc.clone();
    let expected_owner = native_owner(&expected);
    let expected_pixels = expected_owner.cache.as_ref().unwrap().to_interleaved(expected.bounds());
    let mut headless = fixture.headless();
    let preparations = photocraft_io::native_xcf_tones::preparation_count();
    let opened = headless.handle("doc.open", json!({"path":"source.xcf"})).unwrap();
    assert!(photocraft_io::native_xcf_tones::preparation_count() > preparations);
    let document = headless.session.active().unwrap().doc.clone();
    let owner = native_owner(&document);
    assert_eq!(opened["warnings"], reference["warnings"]);
    assert_eq!(owner.smart_filters, expected_owner.smart_filters);
    assert_eq!(owner.filter_mask, expected_owner.filter_mask);
    assert_eq!(owner.transform, expected_owner.transform);
    assert_eq!(owner.cache.as_ref().unwrap().to_interleaved(document.bounds()), expected_pixels);
    assert_eq!(report_without_native_ids(&document), report_without_native_ids(&expected));
    assert!(headless.session.active().unwrap().source_read_only);
    assert!(headless.session.active().unwrap().path.is_none());
    headless.handle("doc.save", json!({"path":"replay.pcraft"})).unwrap();
    let reopened = photocraft_format::load_from_bytes(&std::fs::read(fixture.read.join("replay.pcraft")).unwrap()).unwrap();
    let reopened_owner = native_owner(&reopened);
    assert_eq!(reopened_owner.smart_filters, owner.smart_filters);
    assert_eq!(reopened_owner.filter_mask, owner.filter_mask);
    assert_eq!(reopened_owner.cache.as_ref().unwrap().to_interleaved(reopened.bounds()), expected_pixels);
    assert_eq!(reopened.metadata.import_conversion, document.metadata.import_conversion);
    let mut uncached = reopened_owner.clone();
    uncached.cache = None;
    let rendered = photocraft_engine::smart_cmds::render(&reopened, &uncached).unwrap().unwrap();
    assert_eq!(rendered.to_interleaved(reopened.bounds()), expected_pixels, "saved pixels must be reproducible from native controls");
    assert_eq!(std::fs::read(fixture.read.join("source.xcf")).unwrap(), source);
    // A completed host import must not turn sessionless helper calls into an
    // executable route, including trusted CLI's separate low-level contract.
    let before = photocraft_io::native_xcf_tones::preparation_count();
    let direct = photocraft_automation::files::open_bytes_with_options("source.xcf", &source, &options).unwrap();
    assert!(matches!(direct.document.layers[0].content, photocraft_doc::LayerContent::Raster(_)));
    assert_eq!(photocraft_io::native_xcf_tones::preparation_count(), before);
}

fn deny_native_exposure(command: &str, params: &Value) -> photocraft_engine::Result<()> {
    photocraft_automation::workspace::authorize_engine_step(command, params)?;
    if command == "image.adjustments.exposure" { Err(photocraft_engine::EngineError::Other("native exposure denied".into())) } else { Ok(()) }
}
fn cancel_native_exposure(command: &str, params: &Value) -> photocraft_engine::Result<()> {
    photocraft_automation::workspace::authorize_engine_step(command, params)?;
    if command == "image.adjustments.exposure" { Err(photocraft_engine::EngineError::Cancelled) } else { Ok(()) }
}

#[test]
fn rooted_native_effect_denial_and_cancellation_precede_preparation_and_admission() {
    let fixture = Fixture::new();
    let source = native_masked_source();
    std::fs::write(fixture.read.join("source.xcf"), &source).unwrap();
    for (gate, cancelled) in [(deny_native_exposure as fn(&str, &Value) -> photocraft_engine::Result<()>, false), (cancel_native_exposure, true)] {
        let mut headless = fixture.headless();
        headless.session.authorize = Some(gate);
        let document = headless.session.active().unwrap().doc.clone();
        let listing = headless.session_list();
        let preparations = photocraft_io::native_xcf_tones::preparation_count();
        let error = headless.handle("doc.open", json!({"path":"source.xcf"})).unwrap_err();
        if cancelled {
            assert!(matches!(error, photocraft_automation::AutomationError::Import(photocraft_io::IoError::Cancelled)));
        } else {
            assert!(
                matches!(error,photocraft_automation::AutomationError::Import(photocraft_io::IoError::NativeXcfToneDenied { ref command }) if command=="image.adjustments.exposure")
            );
        }
        assert_eq!(photocraft_io::native_xcf_tones::preparation_count(), preparations);
        assert!(Arc::ptr_eq(&document, &headless.session.active().unwrap().doc));
        assert_eq!(headless.session_list(), listing);
        assert_eq!(std::fs::read(fixture.read.join("source.xcf")).unwrap(), source);
        let direct = photocraft_automation::files::open_bytes("source.xcf", &source).unwrap();
        assert!(matches!(direct.document.layers[0].content, photocraft_doc::LayerContent::Raster(_)), "failed scopes restore inert IO");
    }
}

#[test]
fn rooted_native_authority_cannot_admit_data_from_outside_or_absent_read_roots() {
    let fixture = Fixture::new();
    let source = native_masked_source();
    std::fs::write(fixture.outside.join("source.xcf"), &source).unwrap();
    let mut headless = fixture.headless();
    let before = headless.session.active().unwrap().doc.clone();
    let preparations = photocraft_io::native_xcf_tones::preparation_count();
    for path in [fixture.outside.join("source.xcf"), PathBuf::from("../outside/source.xcf")] {
        let error = headless.open(&path).unwrap_err();
        assert!(matches!(error, photocraft_automation::AutomationError::BadRequest(_)));
        assert!(Arc::ptr_eq(&before, &headless.session.active().unwrap().doc));
        assert_eq!(photocraft_io::native_xcf_tones::preparation_count(), preparations);
    }
    let mut denied = Headless::new();
    assert!(matches!(denied.open(&fixture.outside.join("source.xcf")), Err(photocraft_automation::AutomationError::BadRequest(_))));
    assert!(denied.session.documents().is_empty());
    assert_eq!(photocraft_io::native_xcf_tones::preparation_count(), preparations);
    assert_eq!(std::fs::read(fixture.outside.join("source.xcf")).unwrap(), source);
}
