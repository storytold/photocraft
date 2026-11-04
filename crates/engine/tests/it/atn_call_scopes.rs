//! Import resolves explicit set names before normalizing duplicate action names.
use photocraft_engine::{Session, actions_cmds::Action, atn_cmds};
use photocraft_psd::{
    atn::{self, AtnFile},
    descriptor::UnicodeString,
};
use serde_json::json;

/// Each call to the public exporter supplies one physical row, including repeated names.
fn row(name: &str, actions: Vec<Action>) -> AtnFile {
    if actions.is_empty() {
        return AtnFile { name: UnicodeString::new_nul(name), ..Default::default() };
    }
    let refs: Vec<_> = actions.iter().collect();
    let (bytes, skipped) = atn_cmds::export(name, &refs, &[], false).unwrap();
    assert!(skipped.is_empty());
    atn::parse(&bytes).unwrap()
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
                    result.unwrap();
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
