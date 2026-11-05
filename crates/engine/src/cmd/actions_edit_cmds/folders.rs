//! A panel projection of existing physical rows; captions never become occurrence identities.
use crate::{Session, atn_cmds};

/// One retained ATN folder or one native logical folder not represented by a retained row.
#[derive(Clone, Debug, PartialEq)]
pub struct SetOccurrence {
    pub name: String,
    pub slot: Option<usize>,
    /// The canonical archive/ordinal key is transient and is never serialized by this view.
    pub origin: Option<(usize, usize)>,
    pub actions: Vec<usize>,
    // Keep the canonical archive alive while a transient UI key is retained; pointer reuse
    // must not turn a removed folder's selection into a different parsed occurrence.
    _source: Option<crate::actions_cmds::library::RowOrigin>,
}

/// Resolve actions with the library's existing exact-origin/first-logical-name rule.
/// Empty and repeated same-caption rows remain separate and keep their physical slot order.
pub fn set_occurrences(s: &Session) -> Result<Vec<SetOccurrence>, String> {
    let state = &s.actions;
    crate::actions_cmds::library::placement::validate(state)?;
    let mut rows: Vec<_> = state
        .sets
        .iter()
        .enumerate()
        .map(|(slot, name)| SetOccurrence {
            name: name.clone(),
            slot: Some(slot),
            origin: state.library.rows.get(slot).and_then(Option::as_ref).map(|row| row.key()),
            actions: Vec::new(),
            _source: state.library.rows.get(slot).cloned().flatten(),
        })
        .collect();
    let mut first = std::collections::HashMap::new();
    let mut physical = std::collections::HashMap::new();
    for (slot, name) in state.sets.iter().enumerate() {
        first.entry(name.as_str()).or_insert(slot);
        if let Some(origin) = rows.get(slot).and_then(|row| row.origin) {
            physical.entry((name.as_str(), origin)).or_insert(slot);
        }
    }
    let index = state.library.row_index(&state.sets);
    for (action_index, action) in state.list.iter().enumerate() {
        let name = atn_cmds::logical_set_name(action.set.as_deref());
        let source = index.source(action)?.map(|row| row.key());
        let row = source.and_then(|source| physical.get(&(name, source)).copied()).or_else(|| first.get(name).copied());
        if let Some(row) = row.and_then(|row| rows.get_mut(row)) {
            row.actions.push(action_index);
        } else {
            first.insert(name, rows.len());
            rows.push(SetOccurrence { name: name.into(), slot: None, origin: None, actions: vec![action_index], _source: None });
        }
    }
    Ok(rows)
}
