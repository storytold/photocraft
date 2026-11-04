//! Isolated checked physical-placement prototype. No public command adapter is installed here.
//! Prepared candidates own definitions/maps only; callers must retain their normal lifetime gates.
use std::{collections::{BTreeMap, HashMap, HashSet}, sync::Arc};
use super::{Action, ActionState, RowOrigin, Result, atn_cmds, sidecar};
use serde_json::{Value, json};

type Keys = BTreeMap<String, String>;
type Alternates = BTreeMap<String, Vec<String>>;

/// Opaque checked destination. Public naming preserves external Action struct construction.
/// Cloning retains its archive lifetime; it is neither a serialized ID nor raw action provenance.
#[derive(Clone, Debug)]
pub struct RowBinding {
    pub(crate) name: String,
    pub(crate) row: Option<RowOrigin>,
    pub(crate) carrier: Option<Arc<sidecar::SetMetadata>>,
}

/// Existing-action replacement selectors remain forbidden for new recorder/draft adapters.
pub(crate) enum SelectorUse { NewAction, Assign, Folder }

impl RowBinding {
    /// A bare integer selects the CURRENT physical slot; delayed callers separately bind revision.
    pub(crate) fn at(state: &ActionState, slot: usize, caption: Option<&str>) -> Result<Self> {
        let name = state.sets.get(slot).ok_or("the Actions folder slot is out of range")?;
        if caption.is_some_and(|caption| caption != name) { return Err("the Actions folder caption changed".into()); }
        let binding = Self { name: name.clone(), row: state.library.rows.get(slot).cloned().flatten(), carrier: sidecar::empty_source(state, slot) };
        let resolved = state.library.row_index(&state.sets).destination(&binding)?;
        if resolved != slot { return Err("the Actions folder slot is ambiguous".into()); }
        Ok(binding)
    }

    /// Resolve every physical selector before the caller flushes capture or allocates a take.
    /// Without setSlot the existing command's logical selector validation remains authoritative.
    pub(crate) fn requested(state: &ActionState, params: &Value, usage: SelectorUse) -> Result<Option<Self>> {
        let Some(value) = params.get("setSlot") else { return Ok(None) };
        let slot = value.as_u64().and_then(|value| usize::try_from(value).ok()).ok_or("setSlot must be a current unsigned folder slot")?;
        match usage {
            SelectorUse::NewAction if params.get("action").is_some() || params.get("step").is_some() => return Err("existing actions cannot replace their folder through a new-action selector".into()),
            SelectorUse::Folder if params.get("action").is_some() || params.get("step").is_some() => return Err("select either a folder or an action/step".into()),
            SelectorUse::Assign if params.get("action").is_none_or(Value::is_null) || params.get("step").is_some() => return Err("folder assignment requires one action".into()),
            _ => {}
        }
        let caption = match params.get("set") {
            None => None,
            Some(Value::String(name)) => Some(name.as_str()),
            Some(_) => return Err("set with setSlot must be the exact current caption".into()),
        };
        Self::at(state, slot, caption).map(Some)
    }

    pub(crate) fn slot(&self, state: &ActionState) -> Result<usize> {
        let slot = state.library.row_index(&state.sets).destination(self)?;
        if sidecar::empty_source(state, slot) != self.carrier { return Err("the Actions folder metadata changed".into()); }
        Ok(slot)
    }

    fn apply(&self, action: &mut Action) {
        action.set = Some(self.name.clone());
        action.library_row = Some(self.clone());
        sidecar::place_metadata(action, self.carrier.clone());
    }
}

/// All readers use the same checked destination and current row-local carrier.
/// Temporary indexes preserve linear validation even when many actions share a caption.
pub(crate) fn validate(state: &ActionState) -> Result<()> {
    if !state.list.iter().any(|action| action.library_row.is_some()) { return Ok(()) }
    let index = state.library.row_index(&state.sets);
    let mut carriers = HashMap::new();
    for row in &state.atn_set_rows {
        if state.sets.get(row.slot) == Some(&row.row) {
            if let Some(old) = carriers.insert(row.slot, &row.source) {
                if old != &row.source { return Err("the Actions folder metadata is ambiguous".into()); }
            }
        }
    }
    for action in &state.list {
        if let Some(binding) = &action.library_row {
            let slot = index.destination(binding)?;
            index.source(action)?;
            if carriers.get(&slot).copied() != binding.carrier.as_ref() { return Err("the Actions folder metadata changed".into()); }
        }
    }
    Ok(())
}

pub(crate) enum Edit {
    Insert { destination: RowBinding, action: Action },
    Assign { destination: RowBinding, action: usize },
    Duplicate { source: RowBinding, name: String },
    Delete { source: RowBinding },
}

/// No runtime state or Session is owned or replaced by a prepared physical edit.
pub(crate) struct Prepared {
    pub(crate) definition: ActionState,
    pub(crate) shortcuts: Keys,
    pub(crate) alternates: Alternates,
    pub(crate) result: Value,
}

fn members(state: &ActionState, row: &RowBinding) -> Result<Vec<usize>> {
    row.slot(state)?;
    let index = state.library.row_index(&state.sets);
    let mut out = Vec::new();
    for (position, action) in state.list.iter().enumerate() {
        let raw = index.source(action)?;
        if atn_cmds::logical_set_name(action.set.as_deref()) == row.name && raw.as_ref().map(RowOrigin::key) == row.row.as_ref().map(RowOrigin::key) {
            out.push(position);
        }
    }
    Ok(out)
}

/// Insert at a contiguous target block, or at an empty row before the next occupied slot.
/// Interleaved/unrepresentable input is refused, never repaired by relaxing the durable guard.
fn insertion(state: &ActionState, row: &RowBinding) -> Result<usize> {
    let slot = row.slot(state)?;
    let current = members(state, row)?;
    if current.windows(2).any(|pair| pair[1] != pair[0] + 1) { return Err("the Actions folder is not a contiguous block".into()); }
    if let Some(last) = current.last() { return Ok(last + 1); }
    let index = state.library.row_index(&state.sets);
    let mut slots = HashMap::new();
    for (position, name) in state.sets.iter().enumerate() {
        let key = (name.as_str(), state.library.rows.get(position).and_then(Option::as_ref).map(RowOrigin::key));
        if slots.insert(key, position).is_some() { return Err("the Actions folder is ambiguous".into()); }
    }
    for (position, action) in state.list.iter().enumerate() {
        let raw = index.source(action)?;
        let key = (atn_cmds::logical_set_name(action.set.as_deref()), raw.as_ref().map(RowOrigin::key));
        if slots.get(&key).is_some_and(|&next| next > slot) { return Ok(position); }
    }
    Ok(state.list.len())
}

/// Require exactly one emitted group per current physical slot in its retained slot order.
/// This checks the existing writer's actual order decision, including empty rows and carriers.
fn representable(state: &ActionState) -> Result<()> {
    let expected: Vec<_> = state.sets.iter().enumerate().map(|(slot, name)| {
        (name.as_str(), state.library.rows.get(slot).and_then(Option::as_ref).map(RowOrigin::key))
    }).collect();
    let distinct: HashSet<_> = expected.iter().copied().collect();
    if distinct.len() != expected.len() { return Err("the physical Actions rows are ambiguous".into()); }
    let groups = sidecar::library_groups(state)?;
    let emitted: Vec<_> = groups.iter().map(|(name, selection)| (*name, selection.raw.as_ref().map(RowOrigin::key)))
        .filter(|key| distinct.contains(key)).collect();
    if emitted != expected { return Err("the physical Actions edit cannot preserve flat call order and folder occurrences".into()); }
    Ok(())
}

/// Remap only registered action-call fields; embedded foreign libraries are never traversed.
fn permute_calls(actions: &mut [Action], old_to_new: &[usize]) {
    for (id, params) in actions.iter_mut().flat_map(|action| &mut action.steps) {
        for &key in super::super::call_keys(id) {
            let Some(old) = params.get(key).and_then(super::super::num).and_then(|value| usize::try_from(value).ok()) else { continue };
            if let Some(&new) = old_to_new.get(old) {
                if new != old { params[key] = json!(new); }
            }
        }
    }
}

/// A move may remap numeric calls, but must not change which duplicate name resolves first.
fn preserve_first_names(state: &ActionState, order: &[usize]) -> Result<()> {
    let mut original = HashMap::new();
    for (index, action) in state.list.iter().enumerate() { original.entry(action.name.as_str()).or_insert(index); }
    let mut reordered = HashMap::new();
    for &index in order { reordered.entry(state.list[index].name.as_str()).or_insert(index); }
    if original != reordered { return Err("moving this action would change first-name call resolution".into()); }
    Ok(())
}

/// All fallible work completes in owned definitions/maps before a caller can publish anything.
/// This prototype deliberately does not bypass recording, draft, Owner, authorization or revision gates.
pub(crate) fn prepare(state: &ActionState, shortcuts: &Keys, alternates: &Alternates, edit: Edit) -> Result<Prepared> {
    let mut definition = super::definition_copy(state);
    let mut keys = shortcuts.clone();
    let mut other = alternates.clone();
    let result = match edit {
        Edit::Insert { destination, mut action } => {
            if state.list.iter().any(|old| old.name == action.name) { return Err("an action with this name already exists".into()); }
            let at = insertion(state, &destination)?;
            destination.apply(&mut action);
            definition.list.insert(at, action);
            crate::actions_edit_cmds::shift_action_calls(&mut definition.list, at, 1);
            json!({"index":at,"set":destination.name,"slot":destination.slot(state)?})
        }
        Edit::Assign { destination, action } => {
            let mut value = state.list.get(action).cloned().ok_or("the selected action no longer exists")?;
            destination.slot(state)?;
            // Remove from the owned candidate only. Keeping source calls alive is a permutation,
            // not remove_definitions, whose deliberate deletion semantics null those references.
            definition.list.remove(action);
            let at = insertion(&definition, &destination)?;
            destination.apply(&mut value);
            definition.list.insert(at, value);
            let mut order: Vec<_> = (0..state.list.len()).filter(|&index| index != action).collect();
            order.insert(at, action);
            preserve_first_names(state, &order)?;
            let mut inverse = vec![0; order.len()];
            for (new, &old) in order.iter().enumerate() { inverse[old] = new; }
            permute_calls(&mut definition.list, &inverse);
            json!({"index":at,"set":destination.name,"slot":destination.slot(state)?})
        }
        Edit::Duplicate { source, name } => {
            let slot = source.slot(state)?;
            if name.trim().is_empty() || state.sets.iter().any(|old| old == &name) || state.list.iter().any(|action| atn_cmds::logical_set_name(action.set.as_deref()) == name) {
                return Err("the copied folder needs an unused nonblank caption".into());
            }
            let selected = members(state, &source)?;
            let at = insertion(state, &source)?;
            let destination_slot = slot.checked_add(1).ok_or("Actions folder count overflow")?;
            definition.library.rows.resize(definition.sets.len(), None);
            definition.sets.insert(destination_slot, name.clone());
            definition.library.rows.insert(destination_slot, source.row.clone());
            for row in &mut definition.atn_set_rows {
                if row.slot >= destination_slot { row.slot = row.slot.checked_add(1).ok_or("Actions folder count overflow")?; }
            }
            if let Some(carrier) = &source.carrier {
                definition.atn_set_rows.push(sidecar::SetRowMetadata { slot: destination_slot, row: name.clone(), source: carrier.clone() });
            }
            let destination = RowBinding::at(&definition, destination_slot, Some(&name))?;
            let mut names = atn_cmds::UniqueNames::new(state.list.iter().map(|action| action.name.clone()));
            let copied: Vec<_> = selected.iter().map(|&index| {
                let mut value = state.list[index].clone();
                value.name = names.claim(&format!("{} copy", value.name));
                destination.apply(&mut value);
                value
            }).collect();
            let count = copied.len();
            definition.list.splice(at..at, copied);
            crate::actions_edit_cmds::shift_action_calls(&mut definition.list, at, count);
            json!({"index":at,"set":name,"slot":destination_slot,"actions":count})
        }
        Edit::Delete { source } => {
            let slot = source.slot(state)?;
            let selected = members(state, &source)?;
            let (removed, vanished_keys) = super::super::remove_definitions(&mut definition.list, &selected);
            for key in vanished_keys { keys.remove(&key); other.remove(&key); }
            definition.sets.remove(slot);
            definition.library.rows.resize(state.sets.len(), None);
            definition.library.rows.remove(slot);
            definition.atn_set_rows.retain(|row| row.slot != slot);
            for row in &mut definition.atn_set_rows { if row.slot > slot { row.slot -= 1; } }
            json!({"deletedSet":source.name,"deleted":removed,"slot":slot})
        }
    };
    representable(&definition)?;
    super::check_action_count(&definition, &definition.list)?;
    super::check_definition(&definition, &keys)?;
    Ok(Prepared { definition, shortcuts: keys, alternates: other, result })
}

#[cfg(test)]
mod tests;
