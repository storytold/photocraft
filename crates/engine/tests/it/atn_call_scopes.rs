//! Import resolves explicit set names before normalizing duplicate action names.
use photocraft_engine::{
    Session,
    actions_cmds::Action,
    preset_store::{self, ACTIONS_FILE, MAX_ACTIONS_BYTES, MemBackend, PresetBackend},
};
use photocraft_psd::{
    atn::{self, AtnFile},
    descriptor::UnicodeString,
};
use serde_json::json;

/// The durable writer preserves explicit set scopes through its exact native fallback.
/// Portable Play/Conditional reference scopes have separate CLI and import controls.
fn row(name: &str, mut actions: Vec<Action>) -> AtnFile {
    if actions.is_empty() {
        return AtnFile { name: UnicodeString::new_nul(name), ..Default::default() };
    }
    let store = MemBackend::default();
    let mut session = Session::new();
    assert!(session.attach_preset_store(preset_store::open(Box::new(store.clone()))).is_empty());
    for action in &mut actions {
        action.set = Some(name.to_string());
    }
    session.actions.list = actions;
    session.actions.rev += 1;
    session.sync_preset_store();
    let bytes = store.read(ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap();
    let mut rows = atn::parse_sets(&bytes).unwrap();
    assert_eq!(rows.len(), 1, "metadata-free fixture has exactly its real set row");
    rows.remove(0)
}

fn finish(marker: &str) -> Action {
    Action::new("Finish", vec![("layer.new.layer".into(), json!({"name":marker}))])
}

/// Play and Conditional use the first named physical set, retaining legacy fallback rules.
#[test]
fn imported_named_set_calls_keep_their_target_through_duplicate_action_renaming() {
    for conditional in [false, true] {
        for local in [false, true] {
            for scope in [Some("B"), None, Some("Missing"), Some("Empty")] {
                let mut params = if conditional { json!({"if":"always","then":"Finish"}) } else { json!({"action":"Finish"}) };
                if let Some(scope) = scope {
                    params["set"] = json!(scope);
                }
                let command = if conditional { "actions.conditional" } else { "actions.play" };
                let mut caller = vec![Action::new("Caller", vec![(command.into(), params)])];
                if local {
                    caller.push(finish("From C"));
                }
                let files = vec![
                    row("A", vec![finish("From A")]),
                    row("B", vec![finish("From B first")]),
                    row("B", vec![finish("From B second")]),
                    row("C", caller),
                    row("Empty", Vec::new()),
                ];
                if let Some(scope) = scope {
                    assert!(files[3].actions.iter().any(|action| action.name.to_string_lossy() == "Caller"));
                    let bytes = atn::write(&files[3]).unwrap();
                    let imported = photocraft_engine::atn_cmds::import(&bytes).unwrap();
                    assert_eq!(imported.actions[0].steps[0].1["set"], scope);
                }
                let bytes = atn::write_sets(&files).unwrap();
                let mut session = Session::new();
                session.execute("file.new", json!({"width":8,"height":8})).unwrap();
                let before = session.actions.clone();
                let result = session.execute("actions.load", json!({"data":photocraft_engine::paint::tile::b64_encode(&bytes),"replace":false}));
                let expected = if scope == Some("B") {
                    Some("From B first")
                } else if scope == Some("Empty") {
                    None
                } else if local {
                    Some("From C")
                } else {
                    None
                };
                if let Some(expected) = expected {
                    result.unwrap_or_else(|error| panic!("conditional={conditional}, local={local}, scope={scope:?}: {error}"));
                    let caller = session.actions.list.iter().find(|action| action.name == "Caller").unwrap();
                    let target = caller.steps[0].1[if conditional { "then" } else { "action" }].as_str().unwrap();
                    let resolved = session.actions.list.iter().find(|action| action.name == target).unwrap();
                    assert_eq!(resolved.steps[0].1["name"], expected);
                    let played = session.execute("actions.play", json!({"action":"Caller"})).unwrap();
                    assert!(played.get("failed").is_none(), "{played}");
                    assert_eq!(session.active().unwrap().doc.layers.last().unwrap().name, expected);
                } else {
                    let error = result.unwrap_err().to_string();
                    assert!(error.contains("ambiguous cross-set"), "{error}");
                    assert_eq!(session.actions, before, "refusal retains the original definitions");
                }
            }
        }
    }
}

/// Portable Conditional references may name a different physical set for each branch.
#[test]
fn portable_conditional_branches_retain_independent_set_scopes() {
    use photocraft_engine::atn_cmds;
    for (condition, expected) in [("hasLayers", "From A"), ("hasSelection", "From B")] {
        let caller =
            Action::new("Caller", vec![("actions.conditional".into(), json!({"if":condition,"then":"Finish","thenSet":"A","else":"Finish","elseSet":"B"}))]);
        let mut rows = Vec::new();
        let first = finish("From A");
        let second = finish("From B");
        for (name, action) in [("A", &first), ("B", &second), ("C", &caller)] {
            let bytes = atn_cmds::export(name, &[action], &[], false).unwrap().0;
            rows.push(atn::parse(&bytes).unwrap());
        }
        let bytes = atn::write_sets(&rows).unwrap();
        let projected = atn_cmds::import(&bytes).unwrap();
        assert_eq!(projected.actions[2].steps[0].1["thenSet"], "A");
        assert_eq!(projected.actions[2].steps[0].1["elseSet"], "B");
        if condition == "hasLayers" {
            let mut inherited = projected.clone();
            inherited.actions[2].steps[0].1["thenSet"] = serde_json::Value::Null;
            inherited.actions[2].steps[0].1["set"] = json!("A");
            atn_cmds::normalize_import(&mut inherited, std::iter::empty(), false).unwrap();
            assert_eq!(inherited.actions[2].steps[0].1["then"], "Finish", "null branch scope inherits the shared set, like the writer");
        }
        let mut session = Session::new();
        session.execute("file.new", json!({"width":4,"height":4})).unwrap();
        session.execute("layer.new.layer", json!({"name":"Existing working layer"})).unwrap();
        session.execute("actions.load", json!({"data":photocraft_engine::paint::tile::b64_encode(&bytes)})).unwrap();
        let result = session.execute("actions.play", json!({"action":"Caller"})).unwrap();
        assert!(result.get("failed").is_none(), "{result}");
        assert_eq!(session.active().unwrap().doc.layers.last().unwrap().name, expected);
    }
}

/// Retaining set references must not invent per-branch fields for the existing shared-set API.
#[test]
fn portable_conditional_shared_scope_keeps_the_existing_command_shape() {
    use photocraft_engine::atn_cmds;
    for other in [serde_json::Value::Null, json!("Else")] {
        let params = json!({"if":"always","then":"Then","else":other,"set":"S"});
        let action = Action::new("Caller", vec![("actions.conditional".into(), params.clone())]);
        let bytes = atn_cmds::export("S", &[&action], &[], false).unwrap().0;
        let imported = atn_cmds::import(&bytes).unwrap();
        assert_eq!(imported.actions[0].steps[0].1, params);
    }
}

/// An out-of-archive scope still targets an existing library action after local names collide.
#[test]
fn imported_external_set_call_is_not_rewritten_to_an_archive_local_namesake() {
    use photocraft_engine::atn_cmds;
    let caller = Action::new("Caller", vec![("actions.play".into(), json!({"action":"Finish","set":"External"}))]);
    let local = finish("From B");
    let bytes = atn_cmds::export("B", &[&local, &caller], &[], false).unwrap().0;
    let mut deferred = atn_cmds::import(&bytes).unwrap();
    atn_cmds::normalize_import(&mut deferred, ["Finish".to_string()], true).unwrap();
    assert_ne!(deferred.actions[0].name, "Finish");
    assert_eq!(deferred.actions[1].steps[0].1, json!({"action":"Finish","set":"External"}), "deferred reference remains intact");
    let mut session = Session::new();
    session.execute("file.new", json!({"width":4,"height":4})).unwrap();
    let mut external = finish("From External");
    external.set = Some("External".into());
    session.actions.list.push(external);
    session.execute("actions.load", json!({"data":photocraft_engine::paint::tile::b64_encode(&bytes)})).unwrap();
    let result = session.execute("actions.play", json!({"action":"Caller"})).unwrap();
    assert!(result.get("failed").is_none(), "{result}");
    assert!(session.active().unwrap().doc.layers.iter().any(|layer| layer.name == "From External"));
    assert!(!session.active().unwrap().doc.layers.iter().any(|layer| layer.name == "From B"));
}
