//! Layer selection per client (`DocState.selected_layers`, `active_layer`), not the pixel selection.
use photocraft_engine::Session;
use photocraft_engine::doc::LayerId;
use photocraft_engine::events::{Event, poll};
use serde_json::{Value, json};

// The side-effect-free command used below is `prefs.get`: a registered command that changes
// neither the document nor its selection.
const PROBE: &str = "interface.showCanvasDots";

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
fn probe(s: &mut Session, who: Option<&str>) {
    run(s, who, "prefs.get", json!({"path": PROBE})).unwrap();
}
/// Creates a layer without a client and returns its id.
fn add_layer(s: &mut Session, name: &str) -> LayerId {
    let r = run(s, None, "layer.new.layer", json!({"name": name})).unwrap();
    LayerId(r["layer"].as_u64().unwrap())
}
/// Selected layer ids of the active document, in document selection order.
fn selected_ids(s: &Session) -> Vec<u64> {
    s.active().map(|st| st.selected_layers.iter().map(|id| id.0).collect()).unwrap_or_default()
}
/// The active layer id of the active document, if any.
fn active_id(s: &Session) -> Option<u64> {
    s.active().and_then(|st| st.active_layer).map(|id| id.0)
}
/// Events of the session journal of the given kinds (the `events` filter), from the start.
fn events(s: &Session, kinds: &[&str]) -> Vec<Event> {
    poll(&s.events, &json!({"since": 0, "events": kinds})).unwrap().events
}

// Criterion: each named client keeps its own layer selection; fails with one selection per
// document, since the last assertion would see [L2] instead of `s1-a`'s own [L1].
#[test]
fn ms_sel_two_clients_keep_distinct_selections() {
    let mut s = session();
    let l1 = add_layer(&mut s, "L1");
    let l2 = add_layer(&mut s, "L2");
    run(&mut s, Some("s1-a"), "layer.select", json!({"layer": l1.0})).unwrap();
    assert_eq!(selected_ids(&s), vec![l1.0]);
    run(&mut s, Some("s1-b"), "layer.select", json!({"layer": l2.0})).unwrap();
    assert_eq!(selected_ids(&s), vec![l2.0]);
    probe(&mut s, Some("s1-a"));
    assert_eq!(selected_ids(&s), vec![l1.0]);
}

// Criterion: a command without a client sees the common selection; fails with one selection
// per document, since it would see [L2] (chosen by `s1-a`) instead of the common [L1].
#[test]
fn ms_sel_unnamed_command_sees_common_selection() {
    let mut s = session();
    let l1 = add_layer(&mut s, "L1");
    let l2 = add_layer(&mut s, "L2");
    run(&mut s, None, "layer.select", json!({"layer": l1.0})).unwrap();
    assert_eq!(selected_ids(&s), vec![l1.0]);
    run(&mut s, Some("s1-a"), "layer.select", json!({"layer": l2.0})).unwrap();
    assert_eq!(selected_ids(&s), vec![l2.0]);
    probe(&mut s, None);
    assert_eq!(selected_ids(&s), vec![l1.0]);
}

// Criterion (C3): a new client starts with exactly the active layer selected; fails if it
// inherits the two layers of `s1-a`, or if it starts with nothing selected.
#[test]
fn ms_sel_new_client_starts_empty() {
    let mut s = session();
    let l1 = add_layer(&mut s, "L1");
    let l2 = add_layer(&mut s, "L2");
    run(&mut s, Some("s1-a"), "layer.select", json!({"layer": l1.0})).unwrap();
    run(&mut s, Some("s1-a"), "layer.select", json!({"layer": l2.0, "mode": "toggle"})).unwrap();
    assert_eq!(selected_ids(&s).len(), 2, "s1-a selects two layers");
    let active = active_id(&s);
    assert!(active.is_some(), "the document has an active layer");
    probe(&mut s, Some("s1-new"));
    assert_eq!(active_id(&s), active, "the new client gets the document's active layer");
    assert_eq!(selected_ids(&s), active.into_iter().collect::<Vec<u64>>(), "the new client selects exactly the active layer");
}

// Criterion: after a document change, the first command of a named client keeps the active
// layer; fails if the slot starts with no active layer.
#[test]
fn ms_sel_first_named_command_after_document_change_keeps_active_layer() {
    let mut s = session();
    add_layer(&mut s, "L1");
    s.execute("file.new", json!({"width":4,"height":4,"background":"#804020"})).unwrap();
    let active = active_id(&s);
    assert!(active.is_some(), "the new document has an active layer");
    probe(&mut s, Some("s1-a"));
    assert_eq!(active_id(&s), active, "the first command of a named client keeps the active layer");
    assert_eq!(selected_ids(&s), active.into_iter().collect::<Vec<u64>>(), "the active layer is selected");
}

// Criterion (C4): `enter_selection_slot` with `peers.doc != doc` starts from `fresh_selection`:
// a command acting on the active layer (no `layer` param) by a named client must reach that
// layer. The client did not create it, so the command is refused naming that very layer; fails
// (NoLayer-type error, active layer None) if the slot is reset to Null.
#[test]
fn ms_sel_document_changed_after_start_first_named_command_acts_on_active_layer() {
    let mut s = session();
    add_layer(&mut s, "L1");
    probe(&mut s, Some("s1-a"));
    // A second document, opened after the slots were kept for the first one.
    s.execute("file.new", json!({"width":4,"height":4,"background":"#204080"})).unwrap();
    let active = active_id(&s).expect("the new document has an active layer");
    let refused = run(&mut s, Some("s1-b"), "layer.renameLayer", json!({"name": "Renamed"})).unwrap_err();
    assert!(refused.contains(&format!("would change layer {active} ")), "the command reached the active layer: {refused}");
    assert_eq!(active_id(&s), Some(active), "the active layer is kept");
    let st = s.active().unwrap();
    assert_ne!(st.doc.layer(LayerId(active)).unwrap().name, "Renamed", "a layer the client did not create is unchanged");
    assert_eq!(selected_ids(&s), vec![active]);
}

// Criterion: the active layer is kept per client; fails if `s1-a` gets `s1-b`'s L2 back.
#[test]
fn ms_sel_active_layer_is_per_client() {
    let mut s = session();
    let l1 = add_layer(&mut s, "L1");
    let l2 = add_layer(&mut s, "L2");
    run(&mut s, Some("s1-a"), "layer.select", json!({"layer": l1.0})).unwrap();
    assert_eq!(active_id(&s), Some(l1.0));
    run(&mut s, Some("s1-b"), "layer.select", json!({"layer": l2.0})).unwrap();
    assert_eq!(active_id(&s), Some(l2.0));
    // After `s1-b` changed its active layer, `s1-a` still has its own.
    probe(&mut s, Some("s1-a"));
    assert_eq!(active_id(&s), Some(l1.0));
    assert_eq!(selected_ids(&s), vec![l1.0]);
    probe(&mut s, Some("s1-b"));
    assert_eq!(active_id(&s), Some(l2.0));
}

// Criterion: `peer.selection` is emitted when a named client's selection changes, visible
// with the `events` filter naming it, not emitted when nothing changes, and absent from other
// kinds' filters. Fails if no event is emitted (first assertion) or one per command.
#[test]
fn ms_sel_peer_selection_on_change_visible_with_events_filter() {
    let mut s = session();
    let l1 = add_layer(&mut s, "sel-one");
    run(&mut s, Some("s1-a"), "layer.select", json!({"layer": l1.0})).unwrap();

    let peer = events(&s, &["peer.selection"]);
    assert_eq!(peer.len(), 1, "{peer:?}");
    assert_eq!(peer[0].event, "peer.selection");
    assert_eq!(peer[0].source.as_deref(), Some("s1-a"));
    assert_eq!(peer[0].data["agent"], json!("s1-a"));
    assert!(peer[0].data["page"].is_null());
    assert_eq!(peer[0].data["selection"], json!([l1.0]));

    let other = events(&s, &["document.changed", "peer.joined"]);
    assert!(other.iter().all(|e| e.event != "peer.selection"));

    // A command that does not change the selection emits no new `peer.selection`.
    probe(&mut s, Some("s1-a"));
    assert_eq!(events(&s, &["peer.selection"]).len(), 1);
}

// Criterion (H6): `layers.selected` carries the current client as source, "ui" without one;
// fails if the source is always "ui" or always the command id.
#[test]
fn ms_sel_layers_selected_source_is_the_current_client() {
    let mut s = session();
    let l1 = add_layer(&mut s, "L1");
    let l2 = add_layer(&mut s, "L2");
    let before = events(&s, &["layers.selected"]).len();
    run(&mut s, Some("s1-a"), "layer.select", json!({"layer": l1.0})).unwrap();
    run(&mut s, None, "layer.select", json!({"layer": l2.0})).unwrap();
    let evs = events(&s, &["layers.selected"]);
    let sources: Vec<Option<&str>> = evs[before..].iter().map(|e| e.source.as_deref()).collect();
    assert!(sources.contains(&Some("s1-a")), "{sources:?}");
    assert_eq!(sources.last(), Some(&Some("ui")), "{sources:?}");
    let mine = evs[before..].iter().find(|e| e.source.as_deref() == Some("s1-a")).unwrap();
    assert_eq!(mine.data["active"], json!(l1.0));
}
