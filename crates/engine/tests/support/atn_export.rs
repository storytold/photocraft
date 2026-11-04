//! Assertions for the shared ATN library's native fallback and opt-in Photoshop omission.
use photocraft_engine::{actions_cmds::Action, atn_cmds};
use photocraft_psd::{
    atn::{self, AtnItem},
    descriptor::Value as D,
};

pub fn preserved(set: &str, action: &Action) -> Vec<AtnItem> {
    let (bytes, skipped) = atn_cmds::export(set, &[action], &[], false).unwrap();
    assert!(skipped.is_empty());
    let loaded = atn_cmds::import(&bytes).unwrap();
    assert!(loaded.unsupported.is_empty());
    assert_eq!(loaded.actions[0].steps, action.steps);
    atn::parse(&bytes).unwrap().actions[0].items.clone()
}

/// A one-step command cannot be emitted as a lossy Photoshop approximation.
pub fn fallback(set: &str, action: &Action) {
    assert_eq!(action.steps.len(), 1);
    let items = preserved(set, action);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].event, atn::AtnEvent::name("photocraftCommand"));
    let descriptor = items[0].descriptor.as_ref().unwrap();
    let Some(D::Text(command)) = descriptor.get("command") else { panic!("missing command TEXT") };
    let Some(D::Text(params)) = descriptor.get("params") else { panic!("missing params TEXT") };
    assert_eq!(command.to_string_lossy(), action.steps[0].0);
    assert_eq!(serde_json::from_str::<serde_json::Value>(&params.to_string_lossy()).unwrap(), action.steps[0].1);
    let (_, skipped) = atn_cmds::export(set, &[action], &[], true).unwrap();
    assert_eq!(skipped, [atn_cmds::Skipped { action: action.name.clone(), step: 0, id: action.steps[0].0.clone() }]);
}
