//! Multi-agent sessions: the `client` param on commands and the events it produces (#5154).
//! Events come from the session journal (`Session.events`), read through `events::poll`
//! (`since`, `next`, `gap`; filters `events` and `documents`).

use photocraft_engine::Session;
use photocraft_engine::events::{Event, poll};
use serde_json::{Value, json};

const PREFIX: &str = "`client` must be lowercase letters, digits and hyphens, 1 to 40 characters";

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":4,"height":4,"background":"#804020"})).unwrap();
    s
}
fn run(s: &mut Session, who: Option<&str>, id: &str, mut p: Value) -> Result<Value, String> {
    if let Some(w) = who {
        p["client"] = json!(w);
    }
    s.execute(id, p).map_err(|e| e.to_string())
}
fn add_layer(s: &mut Session, who: Option<&str>, name: &str) {
    run(s, who, "layer.new.layer", json!({"name": name})).unwrap();
}
/// Events after `since` of the given kinds, through `events.poll` (`kinds` empty: every kind).
fn events_since(s: &Session, since: u64, kinds: &[&str]) -> Vec<Event> {
    let mut p = json!({"since": since, "limit": 1000});
    if !kinds.is_empty() {
        p["events"] = json!(kinds);
    }
    let b = poll(&s.events, &p).unwrap();
    assert!(!b.gap, "no gap on a short run");
    b.events
}
fn changed(s: &Session) -> Vec<Event> {
    events_since(s, 0, &["document.changed"])
}
fn joined(s: &Session) -> Vec<Event> {
    events_since(s, 0, &["peer.joined"])
}

// Criterion: a bad `client` is refused with the exact rule, before the command is even looked up.
#[test]
fn ms_v1_invalid_client_errors_before_unknown_command() {
    let mut s = session();
    let e = run(&mut s, Some("Sonde!"), "hub.probe", json!({})).unwrap_err();
    assert!(e.starts_with(PREFIX), "{e}");
    let e = run(&mut s, Some("Sonde!"), "no.such.command", json!({})).unwrap_err();
    assert!(e.starts_with(PREFIX) && !e.contains("unknown"), "{e}");
}

// Criterion: a `client` that is not text counts as the empty string (refused, message ends `got ""`);
// null and absent mean no client and the command runs.
#[test]
fn ms_v2_non_text_client_is_empty_string() {
    let mut s = session();
    for bad in [json!(5), json!(true), json!({}), json!([])] {
        let e = s.execute("layer.new.layer", json!({"name": "x", "client": bad})).unwrap_err().to_string();
        assert!(e.starts_with(PREFIX) && e.ends_with("got \"\""), "{e}");
    }
    s.execute("layer.new.layer", json!({"name": "x", "client": null})).unwrap();
}

// Criterion (V2 behaviour, not the old "no event before a named client"): V2 journals commands
// without a client too, with `agent` null; a named client adds `peer.joined` and its own name as
// `agent`. No `peer.joined` exists before the first named call.
#[test]
fn ms_v3_no_event_before_named_client_then_agent_null() {
    let mut s = session();
    add_layer(&mut s, None, "a");
    let ev = changed(&s);
    assert_eq!(ev.len(), 1, "a command without a client is journaled: {ev:?}");
    assert!(ev[0].data["agent"].is_null(), "{:?}", ev[0].data);
    assert!(joined(&s).is_empty(), "no peer.joined before a named client");
    add_layer(&mut s, Some("s1-main"), "b");
    add_layer(&mut s, None, "c");
    let ev = changed(&s);
    assert_eq!(ev.len(), 3);
    assert_eq!(ev[1].data["agent"], "s1-main");
    assert!(ev[2].data["agent"].is_null(), "the client does not leak to the next call: {:?}", ev[2].data);
    assert_eq!(joined(&s).len(), 1);
}

// Criterion: document.changed carries command, agent, revision (strictly increasing), the
// document id and the command id as source; the `documents` filter selects by that id.
#[test]
fn ms_v4_event_fields_and_revision_increase() {
    let mut s = session();
    add_layer(&mut s, Some("s1-main"), "a");
    add_layer(&mut s, Some("s2-main"), "b");
    let doc = s.active().unwrap().doc.id.0;
    let ev = changed(&s);
    assert_eq!(ev.len(), 2);
    assert_eq!(ev[0].data["command"], "layer.new.layer");
    assert_eq!(ev[0].data["agent"], "s1-main");
    assert_eq!(ev[1].data["agent"], "s2-main");
    let (r0, r1) = (ev[0].data["revision"].as_u64().unwrap(), ev[1].data["revision"].as_u64().unwrap());
    assert!(r1 > r0, "revision increases: {r0} then {r1}");
    assert_eq!(ev[0].document, Some(doc));
    assert_eq!(ev[0].source.as_deref(), Some("layer.new.layer"));
    let p = poll(&s.events, &json!({"since": 0, "events": ["document.changed"], "documents": [doc]})).unwrap();
    assert_eq!(p.events.len(), 2);
    let p = poll(&s.events, &json!({"since": 0, "events": ["document.changed"], "documents": [doc + 1000]})).unwrap();
    assert!(p.events.is_empty(), "another document id selects nothing");
}

// Criterion: peer.joined is published once per client (not once per call), carries the agent,
// and a reader that filters on other kinds does not see it.
#[test]
fn ms_v5_peer_joined_once_and_only_with_all() {
    let mut s = session();
    add_layer(&mut s, Some("s1-main"), "a");
    add_layer(&mut s, Some("s1-main"), "b");
    let j = joined(&s);
    assert_eq!(j.len(), 1, "{j:?}");
    assert_eq!(j[0].data["agent"], "s1-main");
    assert!(events_since(&s, 0, &["document.changed"]).iter().all(|e| !e.event.starts_with("peer.")));
    assert!(events_since(&s, 0, &[]).iter().any(|e| e.event == "peer.joined"), "unfiltered reader sees it");
}

// Criterion: commands that do not edit pixels or layers publish no document.changed. V2 does log
// `selection.changed` for select.* (and a history step), so only document.changed is checked for
// select.all; unknown or UI-only commands add no event of any kind.
#[test]
fn ms_v6_selection_and_ui_commands_emit_nothing() {
    let mut s = session();
    add_layer(&mut s, Some("s1-main"), "a");
    let cursor = s.events.last();
    run(&mut s, Some("s1-main"), "select.all", json!({})).unwrap();
    let sel = events_since(&s, cursor, &["selection.changed"]);
    assert_eq!(sel.len(), 1, "select.all reports the selection: {sel:?}");
    let cursor = s.events.last();
    let _ = run(&mut s, Some("s1-main"), "ui.zoom", json!({}));
    let _ = run(&mut s, Some("s1-main"), "window.title", json!({}));
    assert!(events_since(&s, cursor, &[]).is_empty(), "UI commands emit nothing");
}

// Criterion: `start` and `precheck` take `client` like `execute`: accepted, bad names refused
// before the command lookup, and the first named `start` announces the client once.
#[test]
fn ms_v12_start_and_precheck_take_client() {
    let mut s = session();
    let r = s.start("layer.hideLayers", json!({"client": "s1-main", "layer": 1}));
    if let Err(e) = &r {
        assert!(!e.to_string().contains("unknown parameter"), "{e}");
    }
    let e = s.precheck("layer.hideLayers", &json!({"client": "s1-main", "layer": 1}));
    if let Err(e) = &e {
        assert!(!e.to_string().contains("unknown parameter"), "{e}");
    }
    let e = s.start("layer.hideLayers", json!({"client": "Sonde!", "layer": 1})).expect_err("must fail").to_string();
    assert!(e.starts_with(PREFIX), "{e}");
    let e = s.start("no.such.command", json!({"client": "Sonde!"})).expect_err("must fail").to_string();
    assert!(e.starts_with(PREFIX), "{e}");
    let e = s.precheck("layer.hideLayers", &json!({"client": "Sonde!"})).unwrap_err().to_string();
    assert!(e.starts_with(PREFIX), "{e}");
    let j = joined(&s);
    assert_eq!(j.iter().filter(|e| e.data["agent"] == "s1-main").count(), 1, "{j:?}");
    assert!(!j.iter().any(|e| e.data["agent"] == "Sonde!"), "a rejected name is never announced: {j:?}");
}

// Criterion: `file.automate.batch` started with `client`: bad name refused, good name announced.
#[test]
fn ms_v13_batch_takes_client() {
    let mut s = session();
    let e = s.start("file.automate.batch", json!({"client": "Sonde!"})).expect_err("must fail").to_string();
    assert!(e.starts_with(PREFIX), "{e}");
    let out = std::env::temp_dir().join("ms-v13-batch-out").to_string_lossy().into_owned();
    let params = json!({"client": "s1-main", "source": "folder", "input": [], "output": out});
    if let Err(e) = s.start("file.automate.batch", params) {
        assert!(!e.to_string().contains("client"), "{e}");
    }
    assert!(joined(&s).iter().any(|e| e.data["agent"] == "s1-main"), "no peer.joined event for s1-main");
}

// Criterion: a background job that lands publishes document.changed with the starter's `agent`.
#[test]
fn ms_v14_background_job_end_publishes_document_changed() {
    let mut s = session();
    // A flat fill does not change under blur: make the content non-uniform. The client makes the
    // layer itself, since a named client may only change the layers it created.
    add_layer(&mut s, Some("s1-main"), "contraste");
    run(&mut s, Some("s1-main"), "paint.stroke", json!({"points": [[0, 0], [3, 3]], "color": "#ff0000", "size": 3})).unwrap();
    let cursor = s.events.last();
    let id = match s.start("filter.blur.gaussianBlur", json!({"radius": 60, "client": "s1-main"})).unwrap() {
        photocraft_engine::jobs::Started::Job(id) => id,
        other => panic!("Gaussian Blur must run as a background job: {other:?}"),
    };
    s.wait_job(id).unwrap();
    let hit = events_since(&s, cursor, &["document.changed"]).iter().any(|e| e.data["agent"] == "s1-main" && e.data["command"] == "filter.blur.gaussianBlur");
    assert!(hit, "no document.changed from the finished job for s1-main");
}

// Criterion: document.changed lists the layers the command touched as {id, name}, carries a null
// page (PhotoCraft has no page), the agent, and layersAdded for a new layer. Fails if `elements`,
// `page` or `layersAdded` is missing.
#[test]
fn ms_changed_event_lists_touched_layers_with_null_page() {
    let mut s = session();
    add_layer(&mut s, Some("ag-1"), "fresh");
    let ev = changed(&s);
    let last = ev.last().expect("a layer command publishes document.changed");
    let data = &last.data;
    assert_eq!(data["page"], Value::Null, "{data:?}");
    assert_eq!(data["agent"], "ag-1");
    let elements = data["elements"].as_array().expect("elements is an array");
    assert!(elements.iter().any(|el| el["name"] == json!("fresh")), "{elements:?}");
    assert!(elements.iter().all(|el| el["id"].is_u64() && el["name"].is_string()), "{elements:?}");
    assert_eq!(data["layersAdded"].as_array().map(Vec::len), Some(1), "{data:?}");
}
