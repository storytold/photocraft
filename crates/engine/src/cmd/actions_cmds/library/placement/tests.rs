//! Acceptance source for the private prepared-definition prototype; no commands are executed.
use super::*;
use crate::actions_cmds::library;
use photocraft_psd::{
    atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem},
    descriptor::{Descriptor, UnicodeString, Value as D},
};

fn plain() -> ActionState {
    let files: Vec<_> = [Some("A"), Some("B"), None]
        .into_iter()
        .map(|name| AtnFile {
            name: UnicodeString::new_nul("Same"),
            actions: name
                .into_iter()
                .map(|name| AtnAction {
                    name: UnicodeString::new_nul(name),
                    items: vec![AtnItem {
                        event: AtnEvent::Name(b"futureEvent".to_vec()),
                        dictionary: "Opaque".into(),
                        descriptor: Some(Descriptor::new("null").with("futureBytes", D::RawData(vec![0, 255, 7]))),
                        ..Default::default()
                    }],
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        })
        .collect();
    library::decode(&atn::write_sets(&files).unwrap()).unwrap()
}

fn encoded(state: &ActionState) -> Vec<u8> {
    library::encode(state, &[]).unwrap()
}
fn counts(state: &ActionState) -> Vec<usize> {
    (0..state.sets.len()).map(|slot| members(state, &RowBinding::at(state, slot, None).unwrap()).unwrap().len()).collect()
}
fn prepared(state: &ActionState, edit: Edit) -> Prepared {
    prepare(state, &Keys::new(), &Alternates::new(), edit).unwrap()
}

/// Earlier-folder insertion stays in its contiguous block and remaps both registered call forms.
#[test]
fn earlier_folder_insert_preserves_occurrences_calls_raw_events_and_reload_equality() {
    let state = plain();
    let foreign = json!({"actions":[{"steps":[["actions.play",{"action":1}]]}]});
    let caller = Action::new(
        "Caller",
        vec![
            ("actions.play".into(), json!({"action":1})),
            ("actions.conditional".into(), json!({"if":"always","then":0,"else":1})),
            ("file.automate.batch".into(), foreign.clone()),
        ],
    );
    let second = RowBinding::at(&state, 1, Some("Same")).unwrap();
    let with_caller = prepared(&state, Edit::Insert { destination: second, action: Box::new(caller) }).definition;
    let before = encoded(&with_caller);
    let first = RowBinding::at(&with_caller, 0, Some("Same")).unwrap();
    let value = Action::new("New", vec![]);
    let result = prepared(&with_caller, Edit::Insert { destination: first, action: Box::new(value) });
    assert_eq!(result.result["index"], 1);
    assert_eq!(result.definition.list.iter().map(|action| action.name.as_str()).collect::<Vec<_>>(), ["A", "New", "B", "Caller"]);
    let caller = &result.definition.list[3];
    assert_eq!(caller.steps[0].1, json!({"action":2}));
    assert_eq!(caller.steps[1].1, json!({"if":"always","then":0,"else":2}));
    assert_eq!(caller.steps[2].1, foreign);
    assert_eq!(counts(&result.definition), [2, 2, 0]);
    assert_eq!(encoded(&with_caller), before, "preparation does not publish");
    assert_eq!(result.definition.list[0].library_origin, with_caller.list[0].library_origin);
    assert_eq!(result.definition.list[2].library_steps, with_caller.list[1].library_steps);
    let reload = library::decode(&encoded(&result.definition)).unwrap();
    assert_eq!(counts(&reload), [2, 2, 0]);
    assert_eq!(reload.list, result.definition.list, "explicit placement is excluded from ordinary logical equality");
    assert!(reload.list.iter().all(|action| action.library_row.is_none()));
}

/// A contiguous move preserves numeric identities in Play and both Conditional branches.
#[test]
fn assignment_permutation_remaps_all_known_call_keys_and_preserves_raw_source() {
    let state = plain();
    let first = RowBinding::at(&state, 0, None).unwrap();
    let mut state = prepared(&state, Edit::Insert { destination: first, action: Box::new(Action::new("C", vec![])) }).definition;
    state.list[2].steps.push(("actions.play".into(), json!({"action":1})));
    state.list[2].steps.push(("actions.conditional".into(), json!({"if":"always","then":0,"else":1})));
    let original = state.list[0].library_origin.clone();
    let second = RowBinding::at(&state, 1, None).unwrap();
    let result = prepared(&state, Edit::Assign { destination: second, action: 0 });
    assert_eq!(result.definition.list.iter().map(|action| action.name.as_str()).collect::<Vec<_>>(), ["C", "B", "A"]);
    assert_eq!(result.definition.list[1].steps[1].1, json!({"action":0}));
    assert_eq!(result.definition.list[1].steps[2].1, json!({"if":"always","then":2,"else":0}));
    assert_eq!(result.definition.list[2].library_origin, original);
    assert_eq!(counts(&library::decode(&encoded(&result.definition)).unwrap()), [1, 2, 0]);
}

/// A same-caption move patches only SET association; source action payload and raw origin survive.
#[test]
fn row_local_carrier_survives_assign_duplicate_delete_and_both_shortcut_maps() {
    let mut state = library::decode(&sidecar::tests::physical_placement_fixture()).unwrap();
    state.list[1].name = "Only second".into();
    let bytes = encoded(&state);
    let first_payload = sidecar::tests::physical_action_payload(&state.list[0]);
    let payload = sidecar::tests::physical_action_payload(&state.list[1]);
    let first_carrier = sidecar::tests::physical_set_carrier_payload(&state, 0);
    let second_carrier = sidecar::tests::physical_set_carrier_payload(&state, 1);
    let first = RowBinding::at(&state, 0, None).unwrap();
    let moved = prepared(&state, Edit::Assign { destination: first, action: 1 });
    assert_eq!(counts(&moved.definition), [2, 0, 0]);
    assert_eq!(moved.definition.list[1].library_origin, state.list[1].library_origin);
    assert_eq!(sidecar::tests::physical_action_payload(&moved.definition.list[1]), payload);
    let reload = library::decode(&encoded(&moved.definition)).unwrap();
    assert_eq!(counts(&reload), [2, 0, 0]);
    assert_eq!(sidecar::tests::physical_action_payload(&reload.list[1]), payload);
    assert_eq!(sidecar::tests::physical_set_carrier_payload(&reload, 0), first_carrier);
    assert_eq!(sidecar::tests::physical_set_carrier_payload(&reload, 1), second_carrier);
    assert_eq!(sidecar::tests::physical_set_action_payloads(&reload, 0), [first_payload, payload.clone()]);
    assert!(sidecar::tests::physical_set_action_payloads(&reload, 1).is_empty());
    // A new action without action metadata still uses its selected row's opaque SET carrier.
    let second = RowBinding::at(&state, 1, None).unwrap();
    let added = prepared(&state, Edit::Insert { destination: second.clone(), action: Box::new(Action::new("Fresh", vec![])) });
    let reload = library::decode(&encoded(&added.definition)).unwrap();
    assert_eq!(counts(&reload), [1, 2, 0]);
    assert_eq!(sidecar::tests::physical_set_carrier_payload(&reload, 1), second_carrier);
    let entries = sidecar::tests::physical_set_action_payloads(&reload, 1);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0], payload);
    let copied = prepared(&state, Edit::Duplicate { source: second.clone(), name: "Copy".into() });
    assert_eq!(counts(&copied.definition), [1, 1, 1, 0]);
    let reload = library::decode(&encoded(&copied.definition)).unwrap();
    assert_eq!(counts(&reload), [1, 1, 1, 0]);
    assert_eq!(sidecar::tests::physical_set_carrier_payload(&reload, 2), second_carrier);
    assert_eq!(sidecar::tests::physical_set_action_payloads(&reload, 2).as_slice(), std::slice::from_ref(&payload));
    assert_eq!(sidecar::tests::physical_action_payload(&reload.list[2]), payload);
    let keys = Keys::from([("actions.play:Only second".into(), "F4".into()), ("file.save".into(), "Ctrl+S".into())]);
    let alternates = Alternates::from([("actions.play:Only second".into(), vec!["F5".into()])]);
    let deleted = prepare(&state, &keys, &alternates, Edit::Delete { source: second }).unwrap();
    assert_eq!(counts(&deleted.definition), [1, 0]);
    assert!(!deleted.shortcuts.contains_key("actions.play:Only second"));
    assert!(!deleted.alternates.contains_key("actions.play:Only second"));
    assert_eq!(deleted.shortcuts["file.save"], "Ctrl+S");
    assert_eq!(state.sets.len(), 3);
    assert_eq!(keys["actions.play:Only second"], "F4");
    assert_eq!(encoded(&state), bytes);
}

/// Invalid destinations, fragments and changed first-name resolution fail with no publication.
#[test]
fn stale_ambiguous_unrepresentable_and_first_name_moves_refuse_atomically() {
    let state = plain();
    let before = encoded(&state);
    assert!(RowBinding::at(&state, 9, None).is_err());
    assert!(RowBinding::at(&state, 0, Some(" Same")).is_err());
    let first = RowBinding::at(&state, 0, None).unwrap();
    let mut stale = library::definition_copy(&state);
    first.apply(&mut stale.list[0]);
    stale.sets.remove(0);
    stale.library.rows.remove(0);
    assert!(library::encode(&stale, &[]).is_err(), "explicit stale origin never finds another Same");
    let mut ambiguous = library::definition_copy(&state);
    ambiguous.sets.push("Same".into());
    ambiguous.library.rows.push(state.library.rows[0].clone());
    assert!(RowBinding::at(&ambiguous, 0, None).is_err());
    let rawless = ActionState { sets: vec!["Same".into(), "Same".into()], ..Default::default() };
    assert!(RowBinding::at(&rawless, 0, None).is_err());
    let mut split = library::definition_copy(&state);
    split.list.push(split.list[0].clone());
    assert!(prepare(&split, &Keys::new(), &Alternates::new(), Edit::Insert { destination: first, action: Box::new(Action::new("New", vec![])) }).is_err());
    let duplicate_names = library::decode(&sidecar::tests::physical_placement_fixture()).unwrap();
    let second = RowBinding::at(&duplicate_names, 1, None).unwrap();
    assert!(prepare(&duplicate_names, &Keys::new(), &Alternates::new(), Edit::Assign { destination: second, action: 0 }).is_err());
    assert_eq!(encoded(&state), before);
}

/// The parser is pure: conflicting/null selectors fail before any future caller can allocate a take.
#[test]
fn physical_selector_conflicts_are_checked_before_capture_or_draft_consumption() {
    let mut state = plain();
    state.take = 31;
    state.playback_nonce = 43;
    let before = encoded(&state);
    for params in [
        json!({"setSlot":null}),
        json!({"setSlot":-1}),
        json!({"setSlot":0,"set":null}),
        json!({"setSlot":0,"set":" Same"}),
        json!({"setSlot":0,"action":0}),
        json!({"setSlot":0,"step":0}),
    ] {
        assert!(RowBinding::requested(&state, &params, SelectorUse::NewAction).is_err(), "{params}");
    }
    assert!(RowBinding::requested(&state, &json!({"setSlot":0,"action":0}), SelectorUse::Folder).is_err());
    assert!(RowBinding::requested(&state, &json!({"setSlot":0}), SelectorUse::Assign).is_err());
    assert!(RowBinding::requested(&state, &json!({"setSlot":1,"action":0,"set":"Same"}), SelectorUse::Assign).unwrap().is_some());
    assert!(RowBinding::requested(&state, &json!({"set":"Same"}), SelectorUse::NewAction).unwrap().is_none());
    assert_eq!((state.take, state.playback_nonce), (31, 43));
    assert_eq!(encoded(&state), before);
}
