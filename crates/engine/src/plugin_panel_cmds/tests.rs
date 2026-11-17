//! `plugin.panel.*` end to end: a panel opens, its view is kept for the UI, a click reaches it, and
//! the operations it answers with are carried out — or refused, but always answered.
//!
//! Each fixture's `pc_event` answers with a fixed script, and then **echoes the event it is given**
//! straight back inside a `storage.set`. That makes the event the host delivered readable from a
//! test, byte for byte — which is how the timer fires and the refusals are checked. The one event a
//! scripted fixture does not treat that way is a tick's `doc.changed`, which another test's session
//! can deliver at any moment: see [`tick_guard`].
//!
//! The installed plug-ins *and* the open panels are process-wide, so one id belongs to one test: a
//! second test using it installs over the first one's instance, and the panel it is clicking becomes
//! someone else's. A tick reaches every panel in the process too, so the tests that tick take
//! [`ticking`] first.

use super::*;

/// Held by the tests that call `plugin.panel.tick`, for the length of the test.
///
/// `tick` advances *every* open panel in the process, so one test's tick lands in the middle of
/// another test's assertions: it fires the other panel's timers, re-points the document the panel
/// believes it is showing and grows its event counter. That is the host as it is, and it has no owner
/// to appeal to — the application itself holds one session, and the tick *is* its frame — so a panel
/// being ticked by a session that did not open it is a thing these tests have to live with rather
/// than a bug to fix here. Taking this lock is how they live with it deterministically: two tests
/// that tick cannot interleave, so each one's own ticks are the only ones inside its assertions.
/// (Whatever a test's own tick does to a panel another test owns is still its own doing — which is
/// why no assertion here speaks for the whole process, only for the panel it opened.)
static TICKING: Mutex<()> = Mutex::new(());

/// See [`TICKING`]: held for the length of a test that ticks.
fn ticking() -> MutexGuard<'static, ()> {
    TICKING.lock().unwrap_or_else(|e| e.into_inner())
}

/// Where the fixture's pieces live. The echo buffer sits high in the initial eight pages, below
/// where `pc_alloc` starts handing out blocks (8 pages = 524,288).
const REPLIES_AT: i32 = 4096;
const REPLY_STRIDE: i32 = 4096;
/// The reply's opening text, and the address the event is copied to just after it.
const ECHO_AT: i32 = 262_144;
/// The reply's closing `}]`, kept *before* the buffer: after it would be overwritten by the event.
const SUFFIX_AT: i32 = 262_000;
/// A reply of this in a fixture means "trap here" rather than "answer with this JSON".
const TRAP: &str = "@trap";

/// A v1 filter module built here, used only as "a module that is not a panel". It is deliberately
/// not the example Invert: the plug-in registry is process-wide, so two tests installing one id race
/// — `plugin_cmds::tests::example_invert…` owns `org.photocraft.example.invert` and reads back what
/// it lists the moment it installs, and this test removing it there fails that read.
const FILTER_ID: &str = "test.panel.notfilter";

/// The example panel from `examples/plugins/panel-rs`, as its `build.sh` leaves it. The ABI's real
/// conformance test is that the artifact in the repository runs: not a fixture written to match the
/// host, but the plug-in a person would install.
const EXAMPLE: &[u8] = include_bytes!("../../../plugins/tests/fixtures/panel.wasm");
const EXAMPLE_ID: &str = "org.photocraft.example.panel";

const VIEW: &str = r#"[{"op": "ui.view", "view": {"type": "column", "children": [
    {"type": "text", "id": "script", "value": "", "multiline": true},
    {"type": "button", "id": "go", "label": "Go", "primary": true}]}}]"#;

/// A view whose inputs declare values: what `values` must carry before anything is set or typed.
const VIEW_WITH_VALUES: &str = r#"[{"op": "ui.view", "view": {"type": "column", "children": [
    {"type": "text", "id": "script", "value": "declared line", "multiline": true},
    {"type": "toggle", "id": "bold", "value": true, "label": "Bold"},
    {"type": "button", "id": "go", "label": "Go", "primary": true}]}}]"#;

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")
}

/// The echo's wrapper: the event JSON goes into a `storage.set` as-is, because a stored value is any
/// JSON value, so an event full of quotes needs no escaping.
const ECHO_PREFIX: &str = r#"[{"op": "storage.set", "key": "last", "value": "#;
const ECHO_SUFFIX: &str = r#"}]"#;

/// The `pc_event` body that copies the event it was given into the echo buffer and answers with all
/// of it, as one operation.
fn echo_body() -> String {
    let (p, s) = (ECHO_PREFIX.len() as i32, ECHO_SUFFIX.len() as i32);
    format!(
        "(memory.copy (i32.const {}) (local.get $p) (local.get $len))
         (memory.copy (i32.add (i32.const {}) (local.get $len)) (i32.const {SUFFIX_AT}) (i32.const {s}))
         (i64.or (i64.shl (i64.extend_i32_u (i32.add (i32.const {}) (local.get $len))) (i64.const 32)) (i64.const {ECHO_AT}))",
        ECHO_AT + p,
        ECHO_AT + p,
        p + s,
    )
}

/// The reply a scripted fixture answers a `doc.changed` with: the view it already sent.
///
/// Panels are process-wide, so while one test runs, every other test in this binary ticks *its* panel
/// too, and each of their frames delivers a `doc.changed` naming their session's document. A fixture
/// that answered every call from one counter would have the reply meant for a click eaten by a tick,
/// and the click would hear the echo of its own event instead — a flake no assertion can repair,
/// because the test cannot see which call it got, nor when another session ticked past it. A tick
/// gets the view back instead, which is idempotent (it is what the panel already sent) and writes
/// nothing: the script then lines up with the events *this* test sends, and `storage["last"]` is only
/// ever written by its own.
fn tick_guard() -> String {
    format!(
        "(if (call $is_doc_changed (local.get $p) (local.get $len))
           (then (return (i64.or (i64.shl (i64.const {}) (i64.const 32)) (i64.const {REPLIES_AT})))))",
        VIEW.len()
    )
}

/// A panel module for `id`: `pc_event` answers with `replies` in order and echoes the event back
/// once the script runs out. A reply of [`TRAP`] traps instead of answering.
fn panel(id: &str, replies: &[&str]) -> Vec<u8> {
    panel_with(&panel_manifest(id), replies)
}

/// [`panel`] with a manifest of its own, for the tests that vary what a panel asks for: what it may
/// run follows from `capabilities`, so a panel that asks for nothing must be able to do nothing.
fn panel_with(manifest: &str, replies: &[&str]) -> Vec<u8> {
    // The first reply is the view the panel sends on `init`, and it is also what a tick is answered
    // with (see [`tick_guard`]) — so it is not one of the scripted answers below, and it is never a
    // trap.
    assert!(replies.first().is_some_and(|r| r.contains("ui.view")), "a fixture's script starts with the view its panel sends");
    let mut data = String::new();
    for (i, reply) in replies.iter().enumerate() {
        if *reply != TRAP {
            data.push_str(&format!("(data (i32.const {}) \"{}\")\n", REPLIES_AT + i as i32 * REPLY_STRIDE, escape(reply)));
        }
    }
    data.push_str(&format!("(data (i32.const {ECHO_AT}) \"{}\")\n", escape(ECHO_PREFIX)));
    data.push_str(&format!("(data (i32.const {SUFFIX_AT}) \"{}\")\n", escape(ECHO_SUFFIX)));
    // The script, as a chain of tests on the call counter. Past the end of it, the panel echoes.
    let mut body = echo_body();
    for (i, reply) in replies.iter().enumerate().rev() {
        let then = if *reply == TRAP {
            "(unreachable)".to_string()
        } else {
            format!("(i64.or (i64.shl (i64.const {}) (i64.const 32)) (i64.const {}))", reply.len(), REPLIES_AT + i as i32 * REPLY_STRIDE)
        };
        body = format!("(if (result i64) (i32.eq (local.get $n) (i32.const {i})) (then {then}) (else {body}))");
    }
    module_with(
        manifest,
        &format!("(local.set $n (global.get $calls))\n  {}\n  (global.set $calls (i32.add (local.get $n) (i32.const 1)))\n  {body}", tick_guard()),
        &data,
    )
}

/// What the fixtures' panels declare: reading the document and adding layers to it, and a store.
fn panel_manifest(id: &str) -> String {
    format!(
        r#"{{"id":"{id}","name":"Panel {id}","version":"2.0","kind":"panel",
            "panel":{{"title":"{id}","minWidth":200}},
            "capabilities":{{"document":["read","addLayers"],"storage":true}}}}"#
    )
}

/// A panel that asks for no document capability at all: it may keep a store and nothing else.
fn closed_manifest(id: &str) -> String {
    format!(
        r#"{{"id":"{id}","name":"Panel {id}","version":"2.0","kind":"panel",
            "panel":{{"title":"{id}","minWidth":200}},
            "capabilities":{{"storage":true}}}}"#
    )
}

/// A fixture that echoes **every** event back into its store, `doc.changed` included — unlike
/// [`panel`], whose script answers a tick with its view so a test can read the events it sent.
/// A test using one holds [`ticking`], so no other test's tick writes into the same store.
fn echoing_panel(manifest: &str) -> Vec<u8> {
    let mut data = format!("(data (i32.const {ECHO_AT}) \"{}\")\n", escape(ECHO_PREFIX));
    data.push_str(&format!("(data (i32.const {SUFFIX_AT}) \"{}\")\n", escape(ECHO_SUFFIX)));
    module_with(manifest, &echo_body(), &data)
}

/// A panel that answers every event with the same reply, forever.
fn panel_always(id: &str, reply: &str) -> Vec<u8> {
    let (body, data) = if reply == TRAP {
        ("(unreachable)".to_string(), String::new())
    } else {
        (
            format!("(i64.or (i64.shl (i64.const {}) (i64.const 32)) (i64.const {REPLIES_AT}))", reply.len()),
            format!("(data (i32.const {REPLIES_AT}) \"{}\")\n", escape(reply)),
        )
    };
    module(id, &body, &data)
}

/// The module both fixtures share: the manifest at 16, eight pages, the calling convention's exports.
fn module(id: &str, event_body: &str, data: &str) -> Vec<u8> {
    module_with(&panel_manifest(id), event_body, data)
}

/// [`module`] with a manifest of its own (see [`panel_with`]).
fn module_with(manifest: &str, event_body: &str, data: &str) -> Vec<u8> {
    let src = format!(
        r#"(module
  (memory (export "memory") 8)
  (data (i32.const 16) "{manifest}")
  {data}
  (global $calls (mut i32) (i32.const 0))
  (func (export "pc_abi_version") (result i32) (i32.const 2))
  (func (export "pc_manifest") (result i64)
    (i64.or (i64.shl (i64.const {mlen}) (i64.const 32)) (i64.const 16)))
  (func (export "pc_alloc") (param $n i32) (result i32) (local $old i32)
    (local.set $old (memory.grow (i32.shr_u (i32.add (local.get $n) (i32.const 65535)) (i32.const 16))))
    (if (result i32) (i32.eq (local.get $old) (i32.const -1))
      (then (i32.const 0)) (else (i32.mul (local.get $old) (i32.const 65536)))))
  (func (export "pc_free") (param $p i32) (param $n i32))
  (func $is_doc_changed (param $p i32) (param $len i32) (result i32) (local $i i32) (local $end i32)
    (local.set $end (i32.sub (local.get $len) (i32.const 8)))
    (block $out
      (loop $scan
        (br_if $out (i32.gt_s (local.get $i) (local.get $end)))
        ;; `doc.chan`: the frame's own `"type": "doc.changed"`, which only a tick sends.
        (if (i32.and
              (i32.eq (i32.load (i32.add (local.get $p) (local.get $i))) (i32.const 0x2E636F64))
              (i32.eq (i32.load (i32.add (local.get $p) (i32.add (local.get $i) (i32.const 4)))) (i32.const 0x6E616863)))
          (then (return (i32.const 1))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $scan)))
    (i32.const 0))
  (func (export "pc_event") (param $p i32) (param $len i32) (result i64) (local $n i32)
    {event_body})
)"#,
        manifest = escape(manifest),
        mlen = manifest.len(),
    );
    wat::parse_str(&src).unwrap_or_else(|e| panic!("bad test WAT: {e}\n{src}"))
}

/// A v1 `kind: "filter"` module: the whole of what the host asks of one — `pc_abi_version`,
/// `pc_manifest`, `pc_alloc`, `pc_filter`. Nothing here runs the filter, so its `pc_filter` is a
/// signature and an `i32.const 0`.
fn filter_module(id: &str) -> Vec<u8> {
    let manifest = format!(r#"{{"id":"{id}","name":"Filter {id}","version":"1","kind":"filter","params":{{}}}}"#);
    let src = format!(
        r#"(module
  (memory (export "memory") 1)
  (data (i32.const 16) "{manifest}")
  (func (export "pc_abi_version") (result i32) (i32.const 1))
  (func (export "pc_manifest") (result i64)
    (i64.or (i64.shl (i64.const {mlen}) (i64.const 32)) (i64.const 16)))
  (func (export "pc_alloc") (param $n i32) (result i32) (local $old i32)
    (local.set $old (memory.grow (i32.shr_u (i32.add (local.get $n) (i32.const 65535)) (i32.const 16))))
    (if (result i32) (i32.eq (local.get $old) (i32.const -1))
      (then (i32.const 0)) (else (i32.mul (local.get $old) (i32.const 65536)))))
  (func (export "pc_filter") (param i32 i32 i32 i32 i32 i32 i32 i32) (result i32) (i32.const 0))
)"#,
        manifest = escape(&manifest),
        mlen = manifest.len(),
    );
    wat::parse_str(&src).unwrap_or_else(|e| panic!("bad test WAT: {e}\n{src}"))
}

fn b64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            out.push(if i <= c.len() { T[(n >> (18 - 6 * i)) as usize & 63] as char } else { '=' });
        }
    }
    out
}

/// A session with a 40×28 document and the panel installed, and nothing open yet.
fn app(id: &str, bytes: &[u8]) -> Session {
    let mut s = Session::new();
    s.execute("plugin.install", json!({"data": b64(bytes)})).unwrap();
    s.execute("file.new", json!({"width": 40, "height": 28})).unwrap();
    assert_eq!(s.execute("plugin.panel.inspect", json!({"id": id})).unwrap()["state"], "closed");
    s
}

fn open(s: &mut Session, id: &str) -> Value {
    s.execute("plugin.panel.open", json!({"id": id})).unwrap()
}

fn click(s: &mut Session, id: &str, widget: &str) -> Result<Value> {
    s.execute("plugin.panel.event", json!({"id": id, "event": {"type": "ui.click", "id": widget}}))
}

fn inspect(s: &mut Session, id: &str) -> Value {
    s.execute("plugin.panel.inspect", json!({"id": id})).unwrap()
}

/// The last event the panel was given, which the echo fixture stored for us.
fn last_event(s: &mut Session, id: &str) -> Value {
    inspect(s, id)["storage"]["last"].clone()
}

fn cleanup(s: &mut Session, id: &str) {
    s.execute("plugin.remove", json!({"id": id})).unwrap();
}

#[test]
fn a_panel_opens_with_its_view_values_and_grants() {
    let id = "test.panel.view";
    let mut s = app(id, &panel(id, &[VIEW]));
    let st = open(&mut s, id);
    assert_eq!(st["state"], "running");
    assert_eq!(st["abi"], 2);
    assert_eq!(st["visible"], true);
    assert_eq!(st["view"]["type"], "column");
    assert_eq!(st["view"]["children"][1]["id"], "go");
    // The manifest asks to read the document and add layers, and this host carries both out:
    // `doc.info` reads, `doc.command` runs the lettering commands `permissions` lists. Pixels
    // (`doc.capture`/`doc.commit`) and the network are still refused, and no command grants them.
    assert_eq!(st["capabilities"]["document"], json!(["read", "addLayers"]));
    assert_eq!(st["grants"]["document"], json!(["read", "addLayers"]));
    assert_eq!(st["grants"]["storage"], true);
    assert_eq!(st["grants"]["network"], Value::Null);
    // The exact commands, not the namespaces it asked for: a panel offers what it was granted.
    let commands = st["grants"]["commands"].as_array().unwrap();
    assert!(commands.iter().any(|c| c == "type.create"), "{commands:?}");
    assert!(commands.iter().any(|c| c == "type.info"), "{commands:?}");
    assert!(!commands.iter().any(|c| c == "file.open"), "nothing that touches the disk");
    assert!(!commands.iter().any(|c| c == "plugin.panel.event"), "and no way back into the panel host");
    assert!(st["error"].is_null());
    // Opening a panel that is already open is not a second instance: it is only told it is visible.
    let again = open(&mut s, id);
    assert_eq!(again["events"], 2, "`init`, then the visibility change");
    assert_eq!(last_event(&mut s, id)["type"], "panel.visible");
    assert_eq!(last_event(&mut s, id)["visible"], true);
    cleanup(&mut s, id);
}

#[test]
fn a_click_reaches_the_panel_and_its_operations_are_carried_out() {
    let id = "test.panel.click";
    let reply = r#"[{"op": "ui.notice", "level": "warning", "text": "pressed"},
                   {"op": "ui.progress", "fraction": 0.5, "text": "half", "cancellable": true},
                   {"op": "ui.set", "values": {"script": "line one"}},
                   {"op": "timer.set", "id": "poll", "ms": 50},
                   {"op": "storage.set", "key": "count", "value": 7},
                   {"op": "timer.clear", "id": "poll"}]"#;
    let mut s = app(id, &panel(id, &[VIEW, reply]));
    open(&mut s, id);
    let st = click(&mut s, id, "go").unwrap();
    assert_eq!(st["notices"][0]["level"], "warning");
    assert_eq!(st["notices"][0]["text"], "pressed");
    assert_eq!(st["progress"]["fraction"], 0.5);
    assert_eq!(st["progress"]["cancellable"], true);
    assert_eq!(st["values"]["script"], "line one");
    assert_eq!(st["storage"]["count"], 7);
    assert_eq!(st["timers"], 0, "the same batch cleared the timer it set");
    // A further click runs past the script, and the fixture echoes the event it was given.
    click(&mut s, id, "other").unwrap();
    let seen = last_event(&mut s, id);
    assert_eq!(seen["type"], "ui.click");
    assert_eq!(seen["id"], "other");
    assert!(seen["now"].as_u64().unwrap_or(0) > 0, "the host supplies the clock, so a panel cannot forge one");
    assert_eq!(seen["values"]["script"], "line one", "the values the host holds go with the event");
    // An event naming a widget the panel never declared is the panel's to judge, not the host's; an
    // event the ABI does not have is not.
    assert!(click(&mut s, id, "not-a-widget").is_ok());
    let error = s.execute("plugin.panel.event", json!({"id": id, "event": {"type": "ui.poke"}})).expect_err("unknown event");
    assert!(error.to_string().contains("ui.poke"), "{error}");
    cleanup(&mut s, id);
}

#[test]
fn a_click_carries_the_values_the_view_declares() {
    let _tick = ticking();
    let id = "test.panel.declaredvalues";
    let mut s = app(id, &panel_with(&panel_manifest(id), &[VIEW_WITH_VALUES]));
    open(&mut s, id);
    click(&mut s, id, "go").unwrap();
    let event = last_event(&mut s, id);
    assert_eq!(event["type"], "ui.click", "{event}");
    assert_eq!(event["values"]["script"], "declared line", "`values` is every input's current value: {event}");
    assert_eq!(event["values"]["bold"], true);
    cleanup(&mut s, id);
}

#[test]
fn a_committed_change_is_the_value_the_next_click_carries() {
    let _tick = ticking();
    let id = "test.panel.change";
    let mut s = app(id, &panel_with(&panel_manifest(id), &[VIEW_WITH_VALUES]));
    open(&mut s, id);
    s.execute("plugin.panel.event", json!({"id": id, "event": {"type": "ui.change", "id": "script", "value": "typed"}})).unwrap();
    click(&mut s, id, "go").unwrap();
    let event = last_event(&mut s, id);
    assert_eq!(event["type"], "ui.click", "{event}");
    assert_eq!(event["values"]["script"], "typed", "what the user typed is the input's current value: {event}");
    cleanup(&mut s, id);
}

#[test]
fn input_values_are_capped_and_a_panel_that_overflows_them_is_stopped() {
    let id = "test.panel.valuecap";
    // One `ui.set` with more keys than the host keeps: the operation is refused, the batch with it.
    let keys: Vec<String> = (0..=MAX_VALUE_KEYS).map(|i| format!(r#""k{i}": {i}"#)).collect();
    let reply = format!(r#"[{{"op": "ui.set", "values": {{{}}}}}]"#, keys.join(","));
    let mut s = app(id, &panel(id, &[VIEW, &reply]));
    open(&mut s, id);
    let error = click(&mut s, id, "go").expect_err("a panel may keep only so many input values");
    assert!(error.to_string().contains("input values"), "{error}");
    let st = inspect(&mut s, id);
    assert_eq!(st["state"], "stopped", "{st}");
    assert_eq!(st["values"].as_object().map(serde_json::Map::len), Some(MAX_VALUE_KEYS), "nothing past the cap was kept: {st}");
    cleanup(&mut s, id);
}

/// Another session's tick must not eat this panel's answers.
///
/// Panels are process-wide and `tick` announces the *calling* session's document to all of them, so a
/// second session in the same process — another test in this binary, or a client on the control
/// channel while the user works — hands this panel a `doc.changed` naming *its* document at any
/// moment. The script answers the events this test sends, so the click still hears the reply written
/// for it, and the document in it is the one the panel's own session holds.
#[test]
fn another_sessions_tick_does_not_eat_a_panels_answers() {
    let _ticking = ticking();
    let id = "test.panel.shared";
    let mut s = app(id, &panel(id, &[VIEW, r#"[{"op": "doc.info", "id": "d1"}]"#]));
    open(&mut s, id);
    // A second session, with a document of its own, ticks every panel in the process — including
    // this one, which is not its own. (`panels` is that count, not one: the other tests' panels are
    // open too, and one tick reaches all of them.)
    let mut other = Session::new();
    other.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    let ticked = other.execute("plugin.panel.tick", json!({})).unwrap();
    assert!(ticked["panels"].as_u64().unwrap_or(0) >= 1, "{ticked}");
    // It answers for the panels it was told about and no more: the `errors` of a process-wide tick
    // are the whole process's, and a panel of another test's that fails on its own is not this one's
    // to answer for — only this id's own entries would be.
    let errors = ticked["errors"].as_array().cloned().unwrap_or_default();
    assert!(!errors.iter().any(|e| e["id"] == id), "ticking this panel from another session is not an error: {ticked}");
    // The click is answered by the script's reply for it, not by the echo of its own event.
    click(&mut s, id, "go").unwrap();
    let answer = last_event(&mut s, id);
    assert_eq!(answer["type"], "result", "the click heard the reply the script holds for it: {answer}");
    assert_eq!(answer["id"], "d1");
    assert_eq!(answer["ok"], true, "{answer}");
    assert_eq!(answer["width"], 40, "and `doc.info` read the panel's own session's document: {answer}");
    cleanup(&mut s, id);
}

#[test]
fn a_panel_without_read_is_told_the_document_changed_without_its_contents() {
    let _tick = ticking();
    let id = "test.panel.docchanged.noread";
    let mut s = app(id, &echoing_panel(&closed_manifest(id)));
    open(&mut s, id);
    // A second document is a new active one, which the tick announces. The panel asked for no
    // `read` grant, and `doc.changed` must not hand it what `doc.info` would refuse.
    s.execute("file.new", json!({"width": 40, "height": 28})).unwrap();
    s.execute("plugin.panel.tick", json!({"id": id})).unwrap();
    let event = last_event(&mut s, id);
    assert_eq!(event["type"], "doc.changed", "{event}");
    assert!(event["doc"].is_null(), "no read grant, no document contents: {event}");
    assert!(event["revision"].as_u64().is_some_and(|r| r > 0), "the change *is* announced: {event}");
    cleanup(&mut s, id);
}

#[test]
fn a_panel_with_read_is_told_the_document_changed_with_its_contents() {
    let _tick = ticking();
    let id = "test.panel.docchanged.read";
    let mut s = app(id, &echoing_panel(&panel_manifest(id)));
    open(&mut s, id);
    s.execute("file.new", json!({"width": 40, "height": 28})).unwrap();
    s.execute("plugin.panel.tick", json!({"id": id})).unwrap();
    let event = last_event(&mut s, id);
    assert_eq!(event["type"], "doc.changed", "{event}");
    assert_eq!(event["doc"]["width"], 40, "a granted read gets the document, as `doc.info` does: {event}");
    assert_eq!(event["doc"]["height"], 28);
    cleanup(&mut s, id);
}

#[test]
fn a_timer_fires_as_the_result_of_the_operation_that_armed_it() {
    let _ticking = ticking();
    let id = "test.panel.timer";

    let mut s = app(id, &panel(id, &[VIEW, r#"[{"op": "timer.set", "id": "poll", "ms": 16}]"#]));
    open(&mut s, id);
    click(&mut s, id, "go").unwrap();
    assert_eq!(inspect(&mut s, id)["timers"], 1);
    let early = s.execute("plugin.panel.tick", json!({"id": id})).unwrap();
    assert_eq!(early["timers"], 0, "it was armed for 16 ms, and no time has passed");
    std::thread::sleep(std::time::Duration::from_millis(30));
    let ticked = s.execute("plugin.panel.tick", json!({"id": id})).unwrap();
    assert_eq!(ticked["timers"], 1, "one timer came due");
    assert_eq!(inspect(&mut s, id)["timers"], 0, "and a fired timer is gone");
    let fired = last_event(&mut s, id);
    assert_eq!(fired["type"], "result");
    assert_eq!(fired["id"], "poll");
    assert_eq!(fired["ok"], true);
    cleanup(&mut s, id);
}

#[test]
fn the_document_and_network_operations_are_answered_even_though_they_are_refused() {
    let id = "test.panel.refused";
    let commit = r#"[{"op": "doc.commit", "id": "c1", "doc": 1, "label": "Lettering", "layers": [{"blob": "b:1"}]}]"#;
    let mut s = app(id, &panel(id, &[VIEW, commit]));
    open(&mut s, id);
    click(&mut s, id, "go").unwrap();
    let refused = last_event(&mut s, id);
    assert_eq!(refused["id"], "c1");
    assert_eq!(refused["ok"], false);
    assert!(refused["error"].as_str().unwrap_or_default().contains("doc.commit"), "{refused}");

    // A capture and a loopback request are refused the same way, and each reason names its operation.
    for (i, (op, expected)) in [
        (r#"{"op": "doc.capture", "id": "s1", "source": "composite", "rect": "canvas"}"#, "doc.capture"),
        (r#"{"op": "http.request", "id": "h1", "method": "GET", "url": "http://127.0.0.1:8188/x", "response": "json"}"#, "http.request"),
    ]
    .into_iter()
    .enumerate()
    {
        let id = format!("{id}.{i}");
        let mut s2 = app(&id, &panel(&id, &[VIEW, &format!("[{op}]")]));
        open(&mut s2, &id);
        click(&mut s2, &id, "go").unwrap();
        let answer = last_event(&mut s2, &id);
        assert_eq!(answer["ok"], false, "{expected}");
        assert!(answer["error"].as_str().unwrap_or_default().contains(expected), "{answer}");
        cleanup(&mut s2, &id);
    }
    cleanup(&mut s, id);
}

#[test]
fn doc_info_answers_with_the_open_document() {
    let id = "test.panel.docinfo";
    let mut s = app(id, &panel(id, &[VIEW, r#"[{"op": "doc.info", "id": "d1"}]"#]));
    open(&mut s, id);
    click(&mut s, id, "go").unwrap();
    let answer = last_event(&mut s, id);
    assert_eq!(answer["ok"], true);
    assert_eq!(answer["id"], "d1");
    assert_eq!(answer["width"], 40);
    assert_eq!(answer["height"], 28);
    assert_eq!(answer["layers"].as_array().map(Vec::len), Some(1));
    cleanup(&mut s, id);
}

#[test]
fn a_bad_operation_stops_the_panel_instead_of_leaving_half_the_batch_applied() {
    let id = "test.panel.badop";
    // The 33rd `timer.set` is one past the limit: the operations before it are carried out, this one
    // is not. The ABI says the batch is rejected and the panel stopped, so nothing it armed survives.
    let ops: Vec<String> = (0..=photocraft_plugins::instance::MAX_TIMERS).map(|i| format!(r#"{{"op": "timer.set", "id": "t{i}", "ms": 100}}"#)).collect();
    let reply = format!("[{}]", ops.join(","));
    let mut s = app(id, &panel(id, &[VIEW, &reply]));
    open(&mut s, id);
    let error = click(&mut s, id, "go").expect_err("a reply with too many timers must fail");
    assert!(error.to_string().contains("timers"), "{error}");
    let st = inspect(&mut s, id);
    assert_eq!(st["state"], "stopped", "{st}");
    assert!(st["error"].as_str().unwrap_or_default().contains("timers"), "{st}");
    assert_eq!(st["timers"], 0, "the timers the earlier operations armed went with the instance: {st}");
    // And it stays stopped, as for any other failure.
    assert!(click(&mut s, id, "go").is_err());
    cleanup(&mut s, id);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn one_host_call_gets_one_wall_clock_budget_not_one_per_event() {
    // The web build has no monotonic clock in `std`, so there is nothing to expire; fuel bounds it.
    let id = "test.panel.budget";
    let mut s = app(id, &panel(id, &[VIEW]));
    open(&mut s, id);
    // A budget that has already run out stops the panel instead of pumping it event by event, each
    // with a fresh 2 s deadline of its own.
    {
        let mut all = panels();
        let panel = all.get_mut(id).expect("the panel was just opened");
        let ctx = HostCtx { now: now_ms(), doc: None };
        let error = drive(panel, ops::panel_visible(ctx.now, true), &ctx, &Budget::after_ms(0)).expect_err("an expired budget must stop the panel");
        assert!(error.to_string().contains("time budget"), "{error}");
        assert!(panel.stopped().is_some());
    }
    assert_eq!(inspect(&mut s, id)["state"], "stopped");
    cleanup(&mut s, id);
}

#[test]
fn a_panel_that_traps_stops_and_keeps_its_reason_until_restarted() {
    let id = "test.panel.trap";
    let mut s = app(id, &panel(id, &[VIEW, TRAP]));
    open(&mut s, id);
    let error = click(&mut s, id, "go").expect_err("a trapping panel must fail");
    assert!(error.to_string().contains("trap"), "{error}");
    let st = inspect(&mut s, id);
    assert_eq!(st["state"], "stopped");
    assert!(st["error"].as_str().unwrap_or_default().contains("trap"));
    // What it held went with it, and it stays stopped.
    assert_eq!(st["timers"], 0);
    assert_eq!(st["queued"], 0);
    assert!(click(&mut s, id, "go").is_err());
    let error = s.execute("plugin.panel.open", json!({"id": id})).expect_err("reopening does not quietly restart it");
    assert!(error.to_string().contains("plugin.panel.restart"), "{error}");
    // `restart` is the user's way out, and it starts a fresh instance at `init`.
    let restarted = s.execute("plugin.panel.restart", json!({"id": id})).unwrap();
    assert_eq!(restarted["state"], "running");
    assert_eq!(restarted["events"], 1);
    assert!(restarted["error"].is_null());
    cleanup(&mut s, id);
}

#[test]
fn a_panel_that_fails_at_init_three_times_stays_down_until_restarted() {
    let id = "test.panel.init";
    let mut s = app(id, &panel_always(id, TRAP));
    for i in 0..photocraft_plugins::instance::MAX_FAILURES {
        let error = s.execute("plugin.panel.open", json!({"id": id})).expect_err("a trapping init must fail");
        assert!(error.to_string().contains("trap"), "attempt {i}: {error}");
    }
    let error = s.execute("plugin.panel.open", json!({"id": id})).expect_err("three failures leave it stopped");
    assert!(error.to_string().contains("repeatedly"), "{error}");
    // The explicit restart clears the window — and fails here, because this module always traps.
    assert!(s.execute("plugin.panel.restart", json!({"id": id})).is_err());
    let error = s.execute("plugin.panel.open", json!({"id": id})).expect_err("this module always traps");
    assert!(!error.to_string().contains("repeatedly"), "the restart cleared the window: {error}");
    // Reinstalling the same id runs new code from a clean slate: the old module is gone.
    s.execute("plugin.install", json!({"data": b64(&panel_always(id, VIEW))})).unwrap();
    assert_eq!(open(&mut s, id)["state"], "running");
    cleanup(&mut s, id);
}

#[test]
fn a_panel_that_answers_every_answer_forever_is_stopped() {
    let id = "test.panel.loop";
    // Every reply asks about the document, and every answer is another event: without a limit this
    // would not return.
    let mut s = app(id, &panel_always(id, r#"[{"op": "doc.info", "id": "d"}]"#));
    let error = s.execute("plugin.panel.open", json!({"id": id})).expect_err("a runaway panel must fail");
    assert!(error.to_string().contains("without settling"), "{error}");
    cleanup(&mut s, id);
}

/// A command a panel asked for before it stopped must never run — not with the reply that asked for
/// it, and not later.
///
/// The reply below asks the host to run `type.create` and then fails while being carried out: 513
/// input values against a 512-key cap. The command is queued rather than run as the reply is read
/// (see `apply`), and the failure stops the panel, so it never runs. What made that worth a test is
/// the second half: the queued command stayed in the dropped panel's place until something ran it,
/// and `plugin.panel.close` did — the command the UI sends when the user closes a panel's window,
/// which is the way out of a stopped panel — putting a layer on the document on behalf of a panel
/// that had already failed. Closing a stopped panel now closes it and nothing else.
#[test]
fn a_command_a_panel_asked_for_before_it_stopped_is_dropped() {
    let id = "test.panel.staleask";
    let mut values = Map::new();
    for i in 0..=MAX_VALUE_KEYS {
        values.insert(format!("k{i}"), json!(i));
    }
    let reply = json!([
        {"op": "doc.command", "id": "made", "command": "type.create",
         "params": {"x": 4, "y": 8, "text": "Never made", "size": 12}},
        {"op": "ui.set", "values": values}
    ])
    .to_string();
    let mut s = app(id, &panel(id, &[VIEW, reply.as_str()]));
    open(&mut s, id);
    let error = click(&mut s, id, "go").expect_err("the 513th input value is past the cap");
    assert!(error.to_string().contains("input values"), "{error}");
    assert_eq!(inspect(&mut s, id)["state"], "stopped");
    let layers = |s: &mut Session| s.execute("document.inspect", json!({})).unwrap()["layers"].as_array().map(Vec::len);
    assert_eq!(layers(&mut s), Some(1), "the command the failed reply asked for did not run");
    // The one way left out of a stopped panel: closing its window. It closes the panel, and it is
    // not the moment the dropped command runs.
    let closed = s.execute("plugin.panel.close", json!({"id": id})).unwrap();
    assert_eq!(closed["visible"], false);
    assert_eq!(inspect(&mut s, id)["state"], "stopped", "and the panel is still the panel it was");
    assert_eq!(layers(&mut s), Some(1), "nor does closing the panel run it");
    cleanup(&mut s, id);
}

#[test]
fn a_filter_is_not_a_panel_and_a_panel_is_not_a_filter() {
    let id = "test.panel.kinds";
    let mut s = app(id, &panel(id, &[VIEW]));
    let error = s.execute("plugin.run", json!({"id": id})).expect_err("a panel is not run as a filter");
    assert!(error.to_string().contains("plugin.panel.open"), "{error}");
    s.execute("plugin.install", json!({"data": b64(&filter_module(FILTER_ID))})).unwrap();
    let error = s.execute("plugin.panel.open", json!({"id": FILTER_ID})).expect_err("a filter is not a panel");
    assert!(error.to_string().contains("filter, not a panel"), "{error}");
    assert_eq!(inspect(&mut s, FILTER_ID)["state"], "closed");
    // `plugin.list` says which is which, so a UI can put each in the right menu.
    let listed = s.execute("plugin.list", json!({})).unwrap();
    let find = |id: &str| listed["plugins"].as_array().and_then(|a| a.iter().find(|p| p["id"] == id)).cloned().unwrap_or(Value::Null);
    assert_eq!(find(id)["kind"], "panel");
    assert_eq!(find(id)["abi"], 2);
    assert_eq!(find(id)["panel"]["minWidth"], 200);
    assert_eq!(find(id)["capabilities"]["storage"], true);
    assert_eq!(find(id)["capabilities"]["network"], Value::Null, "no network was asked for");
    assert_eq!(find(FILTER_ID)["kind"], "filter");
    assert_eq!(find(FILTER_ID)["panel"], Value::Null);
    s.execute("plugin.remove", json!({"id": FILTER_ID})).unwrap();
    cleanup(&mut s, id);
}

/// A panel that asks the host to run a command: the command really runs, the document changes, and
/// the panel is told what came back — the whole point of `doc.command`.
#[test]
fn a_panel_runs_a_command_the_host_allows_and_hears_the_answer() {
    let id = "test.panel.command";
    let ask = r#"[{"op": "doc.command", "id": "made", "command": "type.create",
        "params": {"x": 4, "y": 8, "text": "Balloon", "size": 12}}]"#;
    let mut s = app(id, &panel(id, &[VIEW, ask]));
    open(&mut s, id);
    click(&mut s, id, "go").unwrap();
    // The event the panel holds next is the result of the command it asked for: the host ran
    // `type.create` — the same command the UI's Type tool runs — and answered with what it returned.
    let result = last_event(&mut s, id);
    assert_eq!(result["type"], "result");
    assert_eq!(result["id"], "made");
    assert_eq!(result["ok"], true, "{result}");
    assert!(result["layer"].is_u64(), "the new layer's id came back: {result}");
    let doc = s.execute("document.inspect", json!({})).unwrap();
    let layers = doc["layers"].as_array().unwrap();
    assert_eq!(layers.len(), 2, "the background and the new type layer");
    assert_eq!(layers[0]["name"], "Balloon");
    assert_eq!(layers[0]["kind"], "Type");
    // One command, one undo step, like any other edit: the panel did not get a private history.
    s.execute("edit.undo", json!({})).unwrap();
    let doc = s.execute("document.inspect", json!({})).unwrap();
    assert_eq!(doc["layers"].as_array().unwrap().len(), 1);
    cleanup(&mut s, id);
}

/// The refusals are answers too: a command the host does not let a panel run, and one the manifest
/// did not ask for the capability for, both come back as `ok: false` with the reason.
#[test]
fn a_panel_is_told_why_a_command_was_refused() {
    // Not the id above: one id belongs to one test. The registry is process-wide, and a second test
    // installing the same id replaces the instance the first one is clicking — which is how this
    // test failed five runs in thirteen when it shared `test.panel.refused`.
    let id = "test.panel.denied";
    let ask = |command: &str| format!(r#"[{{"op": "doc.command", "id": "nope", "command": "{command}"}}]"#);
    // `file.open` is not in the host's list, however the manifest is written.
    let open_ask = ask("file.open");
    let mut s = app(id, &panel(id, &[VIEW, &open_ask]));
    open(&mut s, id);
    click(&mut s, id, "go").unwrap();
    let result = last_event(&mut s, id);
    assert_eq!(result["ok"], false);
    assert!(result["error"].as_str().unwrap().contains("does not let a panel run"), "{result}");
    assert_eq!(s.execute("document.inspect", json!({})).unwrap()["layers"].as_array().unwrap().len(), 1, "and nothing happened");
    cleanup(&mut s, id);

    // A well-known command is refused too when the manifest asks for nothing.
    let id = "test.panel.nocaps";
    let manifest = |id: &str| {
        format!(
            r#"{{"id":"{id}","name":"Panel {id}","version":"2.0","kind":"panel",
            "panel":{{"title":"{id}","minWidth":200}},"capabilities":{{"storage":false}}}}"#
        )
    };
    let info_ask = ask("type.info");
    let mut s = app(id, &panel_with(&manifest(id), &[VIEW, &info_ask]));
    open(&mut s, id);
    click(&mut s, id, "go").unwrap();
    let result = last_event(&mut s, id);
    assert_eq!(result["ok"], false);
    assert!(result["error"].as_str().unwrap().contains("read"), "{result}");
    // `doc.info` is a capability too, not a free read.
    assert_eq!(inspect(&mut s, id)["grants"]["document"], json!([]));
    cleanup(&mut s, id);
}

/// Asking for a command on every answer never settles. The panel is stopped with a reason, rather
/// than the host running engine commands for it forever.
#[test]
fn a_panel_that_asks_for_a_command_forever_is_stopped() {
    let id = "test.panel.commandloop";
    let ask = r#"[{"op": "doc.command", "id": "again", "command": "type.info"}]"#;
    let mut s = app(id, &panel_always(id, ask));
    // The fixture asks again on *every* event, so it never settles on `init` either: the very first
    // event is enough to stop it.
    let error = s.execute("plugin.panel.open", json!({"id": id})).expect_err("a panel that never settles must fail");
    assert!(error.to_string().contains("without settling"), "{error}");
    let st = inspect(&mut s, id);
    assert_eq!(st["state"], "stopped");
    assert!(st["error"].as_str().unwrap().contains("without settling"), "{st}");
    cleanup(&mut s, id);
}

#[test]
fn removing_or_replacing_a_plug_in_ends_its_instance() {
    let id = "test.panel.lifecycle";
    let mut s = app(id, &panel(id, &[VIEW]));
    open(&mut s, id);
    click(&mut s, id, "go").unwrap();
    assert_eq!(last_event(&mut s, id)["type"], "ui.click");
    // Hiding the panel keeps its instance and everything it held: it is only told it is hidden.
    s.execute("plugin.panel.close", json!({"id": id})).unwrap();
    let st = inspect(&mut s, id);
    assert_eq!(st["visible"], false);
    assert_eq!(st["state"], "running");
    assert_eq!(st["storage"]["last"]["type"], "panel.visible");
    assert_eq!(st["storage"]["last"]["visible"], false);
    // Reinstalling the same id runs new code, so the old instance goes with the old module.
    s.execute("plugin.install", json!({"data": b64(&panel(id, &[VIEW]))})).unwrap();
    let st = inspect(&mut s, id);
    assert_eq!(st["state"], "closed");
    assert!(st["storage"].is_null(), "a closed panel has no state to report");
    // Removing it leaves the panel closed, and it cannot be opened again.
    open(&mut s, id);
    s.execute("plugin.remove", json!({"id": id})).unwrap();
    assert_eq!(inspect(&mut s, id)["state"], "closed");
    assert!(s.execute("plugin.panel.open", json!({"id": id})).is_err(), "it is not installed any more");
}

/// The example panel in the repository, driven the way the UI drives one: draw it, press its buttons,
/// type into its field, and read back what the host was told.
#[test]
fn the_example_panel_plug_in_works_end_to_end() {
    let _ticking = ticking();
    let id = EXAMPLE_ID;
    let mut s = app(id, EXAMPLE);
    let st = open(&mut s, id);
    assert_eq!(st["name"], "Document Info (WebAssembly)");
    // The title the window is drawn under is the one the module declares, and `plugin.list` reports
    // it before anything is opened.
    let listed = s.execute("plugin.list", json!({})).unwrap();
    let declared = listed["plugins"].as_array().and_then(|ps| ps.iter().find(|p| p["id"] == id)).cloned().unwrap_or(Value::Null);
    assert_eq!(declared["panel"]["title"], "Document Info");
    assert_eq!(declared["kind"], "panel");
    assert_eq!(declared["abi"], 2);
    assert_eq!(st["view"]["children"][0]["text"], "Ready", "the panel's first view");
    assert_eq!(st["view"]["children"][1]["id"], "note");
    assert_eq!(st["view"]["children"][2]["children"][0]["id"], "read");
    // Read Document: the panel asks the host about the document, and the answer comes back to it
    // within the same call, so its next view already says what it learned.
    let st = click(&mut s, id, "read").unwrap();
    assert_eq!(st["view"]["children"][0]["text"], "Document 40 x 28");
    assert_eq!(st["view"]["children"][4]["text"], "Reads 1, ticks 0");
    // Watch Document arms a slow timer; pressing it again clears it.
    let st = click(&mut s, id, "watch").unwrap();
    assert_eq!(st["timers"], 1);
    assert_eq!(st["view"]["children"][2]["children"][1]["label"], "Stop Watching");
    let st = click(&mut s, id, "watch").unwrap();
    assert_eq!(st["timers"], 0, "the same button clears the timer it set");
    // A committed text edit is stored under the plug-in's own key.
    s.execute("plugin.panel.event", json!({"id": id, "event": {"type": "ui.change", "id": "note", "value": "chapter 1"}})).unwrap();
    let st = inspect(&mut s, id);
    assert_eq!(st["storage"]["note"], "chapter 1");
    assert_eq!(st["view"]["children"][1]["value"], "chapter 1");
    // It follows the document rather than polling for it: `plugin.panel.tick` tells it, once, when
    // what it can see has changed.
    let before = inspect(&mut s, id)["view"]["children"][0]["text"].clone();
    let quiet = s.execute("plugin.panel.tick", json!({"id": id})).unwrap();
    assert_eq!(quiet["timers"], 0);
    assert_eq!(inspect(&mut s, id)["view"]["children"][0]["text"], before, "no change, no event");
    s.execute("file.new", json!({"width": 64, "height": 32})).unwrap();
    s.execute("plugin.panel.tick", json!({"id": id})).unwrap();
    let st = inspect(&mut s, id);
    assert_eq!(st["view"]["children"][0]["text"], "Document 64 x 32");
    let events = st["events"].as_u64().unwrap_or(0);
    s.execute("plugin.panel.tick", json!({"id": id})).unwrap();
    assert_eq!(inspect(&mut s, id)["events"], events, "the document did not change again");
    // Storage belongs to the plug-in, not to a run of it: a panel stopped and opened again — and one
    // restarted after a failure — reads back what the last instance stored.
    s.execute("plugin.panel.close", json!({"id": id, "stop": true})).unwrap();
    let again = open(&mut s, id);
    assert_eq!(again["view"]["children"][1]["value"], "chapter 1", "the note came back with `init`");
    assert_eq!(again["view"]["children"][0]["text"], "Ready", "but nothing else did");
    // Clearing the field is a delete, not an empty string: a `null` value takes the key out, so the
    // next instance finds no note at all rather than an empty one. (A store that could only grow
    // would be a one-way door once a plug-in reached its key limit.)
    s.execute("plugin.panel.event", json!({"id": id, "event": {"type": "ui.change", "id": "note", "value": ""}})).unwrap();
    let st = inspect(&mut s, id);
    assert_eq!(st["storage"].get("note"), None, "an empty note is no note");
    assert_eq!(st["view"]["children"][1]["value"], "");
    s.execute("plugin.panel.close", json!({"id": id, "stop": true})).unwrap();
    assert_eq!(open(&mut s, id)["view"]["children"][1]["value"], "", "and it stays gone");
    // Undoing the install forgets it: the same id reinstalled starts clean.
    s.execute("plugin.remove", json!({"id": id})).unwrap();
    s.execute("plugin.install", json!({"data": b64(EXAMPLE)})).unwrap();
    assert_eq!(open(&mut s, id)["view"]["children"][1]["value"], "");
    cleanup(&mut s, id);
}

#[test]
fn the_store_is_capped_at_one_mib_and_a_refused_value_is_not_kept() {
    let id = "test.panel.storagecap";
    let mut s = app(id, &panel(id, &[VIEW]));
    open(&mut s, id);
    let ctx = HostCtx { now: now_ms(), doc: None };
    {
        let mut all = panels();
        let panel = all.get_mut(id).unwrap();
        // Each value is under the 1 MiB reply cap, so the store's own cap is what refuses the second.
        let big = "x".repeat(600 * 1024);
        apply(panel, &ctx, Operation::StorageSet { key: "a".into(), value: Value::String(big.clone()) }).unwrap();
        let e = apply(panel, &ctx, Operation::StorageSet { key: "b".into(), value: Value::String(big) }).unwrap_err();
        assert!(e.to_string().contains("stored values"), "{e}");
        assert_eq!(panel.storage.len(), 1, "the refused value was not kept");
        assert_eq!(storages().get(id).map(Map::len), Some(1), "and the written-through store matches");
        // Replacing a key is measured by the difference, not by the sum of old and new.
        apply(panel, &ctx, Operation::StorageSet { key: "a".into(), value: Value::String("small".into()) }).unwrap();
        assert_eq!(storages().get(id).unwrap().get("a"), Some(&json!("small")));
        apply(panel, &ctx, Operation::StorageSet { key: "a".into(), value: Value::Null }).unwrap();
        assert!(panel.storage.is_empty());
        assert!(storages().get(id).is_none_or(|m| !m.contains_key("a")), "a delete reaches the store too");
    }
    cleanup(&mut s, id);
}

#[test]
fn an_armed_timer_tells_the_ui_when_to_draw_again() {
    let _ticking = ticking();
    let id = "test.panel.nexttimer";
    let mut s = app(id, &panel(id, &[VIEW, r#"[{"op": "timer.set", "id": "poll", "ms": 60000}]"#]));
    open(&mut s, id);
    assert_eq!(next_timer_in(), None, "no timer is armed, so there is nothing to wake for");
    click(&mut s, id, "go").unwrap();
    let wait = next_timer_in().expect("a visible panel armed a timer");
    assert!(wait <= 60_000, "the nearest timer is 60 s away, not {wait} ms");
    // A closed panel has no window to repaint for: its timers wait until it is shown again.
    s.execute("plugin.panel.close", json!({"id": id})).unwrap();
    assert_eq!(next_timer_in(), None);
    cleanup(&mut s, id);
}
