//! The panel ABI v2 end to end: a well-behaved panel, and hostile ones (bad pointers, unparsable
//! operations, traps, infinite loops), which must all fail with an error, quickly, and stop the
//! instance without touching anything else.

use std::sync::Arc;
use std::time::{Duration, Instant};

use photocraft_plugins::instance::Instance;
use photocraft_plugins::ops::{self, Operation};
use photocraft_plugins::{Error, Limits, Plugin};
use serde_json::json;

const PANEL_MANIFEST: &str = r#"{"id":"com.example.panel","name":"Example Panel","version":"2.0","kind":"panel",
    "panel":{"title":"Example","minWidth":260},
    "capabilities":{"document":["read","addLayers"],"storage":true}}"#;
const FILTER_MANIFEST: &str = r#"{"id":"org.example.tone","name":"Tone","kind":"filter"}"#;

/// Where the reply data segment lives: inside the initial four pages, above `pc_alloc`'s first
/// block (page 4 → 262,144).
const REPLY_AT: i32 = 131_072;

/// Escapes a string for a WAT data segment: a literal newline is not legal inside one, but `\n` is,
/// and it decodes to the same single byte, so byte lengths still line up.
fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n").replace('\t', "\\t")
}

/// A module implementing the ABI, with every piece `Plugin::load` inspects swappable: the manifest,
/// the ABI version, an optional reply string, `pc_event`'s body (params `$p`, `$len`), whether
/// `pc_free` is exported, and extra module items (globals, data, more exports).
fn module(manifest: &str, version: i32, reply: Option<&str>, event: Option<&str>, free: bool, extra: &str) -> Vec<u8> {
    let data = match reply {
        Some(r) => format!("(data (i32.const {REPLY_AT}) \"{}\")", escape(r)),
        None => String::new(),
    };
    let event = match event {
        Some(body) => format!("(func (export \"pc_event\") (param $p i32) (param $len i32) (result i64) {body})"),
        None => String::new(),
    };
    let free = if free { "(func (export \"pc_free\") (param $p i32) (param $n i32))" } else { "" };
    let src = format!(
        r#"(module
  (memory (export "memory") 4)
  (data (i32.const 16) "{manifest}")
  {data}
  {extra}
  (func (export "pc_abi_version") (result i32) (i32.const {version}))
  (func (export "pc_manifest") (result i64)
    (i64.or (i64.shl (i64.const {mlen}) (i64.const 32)) (i64.const 16)))
  (func (export "pc_alloc") (param $n i32) (result i32) (local $old i32)
    (local.set $old (memory.grow (i32.shr_u (i32.add (local.get $n) (i32.const 65535)) (i32.const 16))))
    (if (result i32) (i32.eq (local.get $old) (i32.const -1))
      (then (i32.const 0)) (else (i32.mul (local.get $old) (i32.const 65536)))))
  {free}
  {event}
)"#,
        manifest = escape(manifest),
        mlen = manifest.len(),
    );
    wat::parse_str(&src).unwrap_or_else(|e| panic!("bad test WAT: {e}\n{src}"))
}

/// The body of a `pc_event` that replies with the data segment at `REPLY_AT`.
fn reply_body(len: usize) -> String {
    format!("(i64.or (i64.shl (i64.const {len}) (i64.const 32)) (i64.const {REPLY_AT}))")
}

/// A panel that always replies with `reply`.
fn replying(reply: &str) -> Vec<u8> {
    module(PANEL_MANIFEST, 2, Some(reply), Some(&reply_body(reply.len())), true, "")
}

/// A panel that replies "no operations".
fn silent() -> Vec<u8> {
    module(PANEL_MANIFEST, 2, None, Some("(i64.const 0)"), true, "")
}

/// The ABI v1 filter export, for a filter module.
const PC_FILTER: &str = r#"(func (export "pc_filter") (param i32) (param i32) (param i32) (param i32)
    (param i32) (param i32) (param i32) (param i32) (result i32) (i32.const 0))"#;

fn load(bytes: &[u8]) -> Plugin {
    Plugin::load(bytes, Limits::default()).unwrap()
}

fn start(bytes: &[u8]) -> Instance {
    Instance::start(Arc::new(load(bytes))).unwrap()
}

/// The text of the first `ui.notice` in an operation list.
fn notice(ops: &[Operation]) -> &str {
    ops.iter()
        .find_map(|op| match op {
            Operation::UiNotice { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .unwrap_or("<no notice>")
}

/// A panel that is expected to fail: it must fail, stop, stay stopped with the same reason, and do
/// so without hanging.
fn send_err(bytes: &[u8]) -> (Error, Duration) {
    let t = Instant::now();
    let mut instance = start(bytes);
    let error = instance.send(&ops::panel_visible(0, true)).expect_err("a hostile panel must fail");
    assert!(instance.is_stopped(), "a failure stops the instance");
    assert_eq!(instance.stopped_reason(), Some(&error));
    assert_eq!(instance.send(&ops::panel_visible(0, true)).expect_err("still stopped"), error);
    (error, t.elapsed())
}

#[test]
fn a_panel_loads_and_reports_its_manifest() {
    let plugin = load(&silent());
    assert_eq!(plugin.kind(), photocraft_plugins::Kind::Panel);
    assert_eq!(plugin.abi(), 2, "a panel declares ABI 2");
    assert!(plugin.is_panel());
    let m = plugin.manifest();
    assert_eq!((m.panel.as_ref().map(|p| p.title.as_str()), m.panel.as_ref().map(|p| p.min_width)), (Some("Example"), Some(260)));
    assert!(m.capabilities.storage);
    assert_eq!(m.capabilities.document.len(), 2);
    assert!(m.capabilities.network.is_none());
}

#[test]
fn only_abi_2_may_declare_the_panel_kind() {
    // A panel that claims ABI 1 is not a v2 plug-in, whatever its manifest says.
    let v1_panel = module(PANEL_MANIFEST, 1, None, Some("(i64.const 0)"), true, "");
    assert!(matches!(Plugin::load(&v1_panel, Limits::default()), Err(Error::Abi(m)) if m.contains("ABI 2")));
    // A version this host does not implement is refused either way.
    let v3 = module(PANEL_MANIFEST, 3, None, Some("(i64.const 0)"), true, "");
    assert!(matches!(Plugin::load(&v3, Limits::default()), Err(Error::Abi(m)) if m.contains("1 to 2")));
    // A filter may declare either: an ABI 2 filter keeps `pc_filter` and the per-band model.
    let v2_filter = module(FILTER_MANIFEST, 2, None, None, false, PC_FILTER);
    let plugin = load(&v2_filter);
    assert_eq!((plugin.abi(), plugin.is_panel()), (2, false));
    // A v1 filter is untouched.
    let v1_filter = module(FILTER_MANIFEST, 1, None, None, false, PC_FILTER);
    assert_eq!(load(&v1_filter).abi(), 1);
}

#[test]
fn a_panel_must_export_pc_event_and_pc_free() {
    let no_event = module(PANEL_MANIFEST, 2, None, None, true, "");
    assert!(matches!(Plugin::load(&no_event, Limits::default()), Err(Error::Abi(m)) if m.contains("pc_event")));
    let no_free = module(PANEL_MANIFEST, 2, None, Some("(i64.const 0)"), false, "");
    assert!(matches!(Plugin::load(&no_free, Limits::default()), Err(Error::Abi(m)) if m.contains("pc_free")));
}

#[test]
fn a_filter_cannot_be_opened_as_a_panel() {
    let filter = module(FILTER_MANIFEST, 1, None, None, false, PC_FILTER);
    let plugin = Arc::new(load(&filter));
    assert!(matches!(Instance::start(plugin), Err(Error::Abi(m)) if m.contains("filter, not a panel")));
}

#[test]
fn operations_round_trip_through_the_interpreter() {
    let ops = start(&replying(
        r#"[{"op": "ui.view", "view": {"type": "column", "children": [
              {"type": "text", "id": "script", "value": "", "multiline": true},
              {"type": "button", "id": "insert", "label": "Insert", "primary": true}]}},
            {"op": "ui.set", "values": {"script": "line one"}},
            {"op": "timer.set", "id": "poll", "ms": 250},
            {"op": "storage.set", "key": "lastPrefix", "value": "/A"},
            {"op": "doc.info", "id": "d1"},
            {"op": "blob.free", "blob": "b:1"}]"#,
    ))
    .send(&ops::panel_visible(1_000, true))
    .unwrap();
    assert_eq!(ops.iter().map(Operation::name).collect::<Vec<_>>(), ["ui.view", "ui.set", "timer.set", "storage.set", "doc.info", "blob.free"]);
    let Operation::UiView { view } = &ops[0] else { return };
    let photocraft_plugins::view::Widget::Column { children } = view else { return };
    assert_eq!(children.len(), 2);
    assert_eq!(children[1].id(), Some("insert"));
    assert_eq!(ops[4].id().map(ToString::to_string).as_deref(), Some("d1"));
    assert!(ops[0].id().is_none(), "`ui.view` gets no `result` event");
}

#[test]
fn an_empty_reply_means_no_operations() {
    assert!(start(&silent()).send(&ops::panel_visible(0, true)).unwrap().is_empty());
}

#[test]
fn the_event_reaches_the_module_where_the_host_said_it_would() {
    // Answer only if the first byte of the event block is `{`, so an unwritten or misplaced event
    // block shows up as "no operations".
    let reply = r#"[{"op": "ui.notice", "level": "info", "text": "read the event"}]"#;
    let body = format!("(if (result i64) (i32.eq (i32.load8_u (local.get $p)) (i32.const 123)) (then {}) (else (i64.const 0)))", reply_body(reply.len()));
    let mut instance = start(&module(PANEL_MANIFEST, 2, Some(reply), Some(&body), true, ""));
    let ops = instance.send(&ops::panel_visible(1_000, true)).unwrap();
    assert_eq!(notice(&ops), "read the event");
}

#[test]
fn the_host_gives_both_buffers_back() {
    // A panel that reports, in its next reply, whether `pc_free` has been called: the host frees the
    // event block and the previous reply block after every event, so a long-lived instance does not
    // grow without bound.
    let first = r#"[{"op": "ui.notice", "level": "info", "text": "first"}]"#;
    let later = r#"[{"op": "ui.notice", "level": "info", "text": "later"}]"#;
    let src = format!(
        r#"(module
  (memory (export "memory") 4)
  (data (i32.const 16) "{m}")
  (data (i32.const {at}) "{first}")
  (data (i32.const 133120) "{later}")
  (global $frees (mut i32) (i32.const 0))
  (func (export "pc_abi_version") (result i32) (i32.const 2))
  (func (export "pc_manifest") (result i64)
    (i64.or (i64.shl (i64.const {mlen}) (i64.const 32)) (i64.const 16)))
  (func (export "pc_alloc") (param $n i32) (result i32) (local $old i32)
    (local.set $old (memory.grow (i32.shr_u (i32.add (local.get $n) (i32.const 65535)) (i32.const 16))))
    (if (result i32) (i32.eq (local.get $old) (i32.const -1))
      (then (i32.const 0)) (else (i32.mul (local.get $old) (i32.const 65536)))))
  (func (export "pc_free") (param $p i32) (param $n i32)
    (global.set $frees (i32.add (global.get $frees) (i32.const 1))))
  (func (export "pc_event") (param $p i32) (param $len i32) (result i64)
    (if (result i64) (i32.gt_s (global.get $frees) (i32.const 0))
      (then (i64.or (i64.shl (i64.const {llen}) (i64.const 32)) (i64.const 133120)))
      (else (i64.or (i64.shl (i64.const {flen}) (i64.const 32)) (i64.const {at})))))
)"#,
        m = escape(PANEL_MANIFEST),
        mlen = PANEL_MANIFEST.len(),
        at = REPLY_AT,
        first = escape(first),
        flen = first.len(),
        later = escape(later),
        llen = later.len(),
    );
    let bytes = wat::parse_str(&src).unwrap_or_else(|e| panic!("bad test WAT: {e}\n{src}"));
    let mut instance = start(&bytes);
    let event = ops::panel_visible(0, true);
    assert_eq!(notice(&instance.send(&event).unwrap()), "first", "nothing has been freed yet");
    assert_eq!(notice(&instance.send(&event).unwrap()), "later", "the host freed the previous blocks");
}

#[test]
fn hostile_panels_fail_cleanly_and_quickly() {
    let bad_pointer = module(PANEL_MANIFEST, 2, None, Some("(i64.or (i64.shl (i64.const 8) (i64.const 32)) (i64.const 100000000))"), true, "");
    let too_long = module(PANEL_MANIFEST, 2, None, Some("(i64.or (i64.shl (i64.const 1048577) (i64.const 32)) (i64.const 131072))"), true, "");
    let not_json = replying("not json");
    let a_bad_view = replying(r#"[{"op": "ui.view", "view": {"type": "button", "id": "", "label": "x"}}]"#);
    let duplicate_ids = replying(
        r#"[{"op": "ui.view", "view": {"type": "row", "children": [
             {"type": "separator"}, {"type": "toggle", "id": "a", "value": true, "label": "A"},
             {"type": "toggle", "id": "a", "value": false, "label": "B"}]}}]"#,
    );
    let unknown_op = replying(r#"[{"op": "teleport", "id": "x"}]"#);
    let not_an_array = replying(r#"{"op": "ui.notice"}"#);
    let trap = module(PANEL_MANIFEST, 2, None, Some("(unreachable)"), true, "");
    let spin = module(PANEL_MANIFEST, 2, None, Some("(loop $l (br $l)) (i64.const 0)"), true, "");
    let too_many_ops = format!("[{}]", vec![r#"{"op":"ui.notice","level":"info","text":"x"}"#; 65].join(","));

    for (what, bytes, expected) in [
        ("a reply outside memory", bad_pointer, "abi"),
        ("an oversized reply", too_long, "limit"),
        ("unparsable operations", not_json, "ops"),
        ("a bad widget tree", a_bad_view, "ops"),
        ("duplicate widget ids", duplicate_ids, "ops"),
        ("an unknown operation", unknown_op, "ops"),
        ("an operation list that is not a list", not_an_array, "ops"),
        ("a trapping pc_event", trap, "trap"),
        ("an endless pc_event", spin, "limit"),
    ] {
        let (error, took) = send_err(&bytes);
        let got = match error {
            Error::Abi(_) => "abi",
            Error::Limit(_) => "limit",
            Error::Ops(_) => "ops",
            Error::Trap(_) => "trap",
            other => panic!("{what}: unexpected {other:?}"),
        };
        assert_eq!(got, expected, "{what}: {error:?}");
        assert!(took < Duration::from_secs(30), "{what} took {took:?}");
    }

    let (error, _) = send_err(&replying(&too_many_ops));
    assert!(matches!(error, Error::Limit(_)), "{error:?}");
}

#[test]
fn the_instance_owns_the_queue_the_timers_and_the_blobs() {
    let mut instance = start(&replying(
        r#"[{"op": "timer.set", "id": "poll", "ms": 100},
            {"op": "ui.notice", "level": "info", "text": "waiting"}]"#,
    ));
    let ops = instance.send(&ops::panel_visible(1_000, true)).unwrap();
    assert_eq!(ops.iter().map(Operation::name).collect::<Vec<_>>(), ["timer.set", "ui.notice"]);
    // Executing operations is the engine's job; the timers they arm are the instance's.
    if let Operation::TimerSet { id, ms } = &ops[0] {
        instance.timers_mut().set(id, *ms, 1_000).unwrap();
    }
    assert_eq!(instance.timers().len(), 1);
    assert!(instance.timers_mut().take_due(1_099).is_empty());
    assert_eq!(instance.timers_mut().take_due(1_100), ["poll"]);

    let blob = instance.blobs_mut().put(vec![0u8; 32]).unwrap();
    assert!(instance.blobs().get(&blob).is_some());
    assert!(instance.blobs_mut().free(&blob));

    instance.queue_mut().push(json!({"type": "ui.click", "id": "insert"}));
    assert_eq!(instance.queue().kinds(), ["ui.click"]);

    // Stopping drops everything the instance held.
    instance.stop(Error::Failed("the user closed the panel".into()));
    assert!(instance.queue().is_empty() && instance.timers().is_empty() && instance.blobs().is_empty());
    assert!(matches!(instance.send(&ops::panel_visible(0, false)), Err(Error::Failed(_))));
}
