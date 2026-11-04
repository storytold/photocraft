//! The measured Actions sidecar. Opaque bytes travel with their action, never with its display name.
use std::{ops::Range, sync::Arc};

use photocraft_psd::{
    atn,
    descriptor::{Descriptor, Id, UnicodeString, Value as D},
};

use super::{Action, AtnAction, AtnEvent, AtnFile, AtnItem};
use crate::actions_cmds::ActionState;

const MODERN: &str = "modernizedActionData";
const MAX_BYTES: usize = crate::preset_import_cmds::MAX_FILE_BYTES as usize;
type Result<T> = std::result::Result<T, String>;

fn extend(out: &mut Vec<u8>, bytes: &[u8]) -> Result<()> {
    if out.len().checked_add(bytes.len()).is_none_or(|length| length > MAX_BYTES) {
        return Err("sidecar exceeds the action-file byte limit".into());
    }
    out.extend_from_slice(bytes);
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RawDescriptor {
    source: Arc<[u8]>,
    span: Range<usize>,
}

struct Field {
    key: Id,
    value: D,
    span: Range<usize>,
    key_len: usize,
}

fn key_len(key: &Id) -> usize {
    match key {
        Id::Code(_) => 8,
        Id::Str(bytes) => 4usize.saturating_add(bytes.len()),
    }
}

fn value_bytes(value: D) -> Result<Vec<u8>> {
    let header = Descriptor::new("null").to_bytes().len();
    let bytes = Descriptor::new("null").with("value", value).to_bytes();
    bytes.get(header.saturating_add(9)..).map(<[u8]>::to_vec).ok_or_else(|| "invalid encoded sidecar value".into())
}

impl RawDescriptor {
    fn new(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_BYTES {
            return Err("sidecar exceeds the action-file byte limit".into());
        }
        Descriptor::from_bytes(bytes).map_err(|error| error.to_string())?;
        Ok(Self { source: Arc::from(bytes), span: 0..bytes.len() })
    }

    fn bytes(&self) -> &[u8] {
        self.source.get(self.span.clone()).unwrap_or(&[])
    }

    fn parsed(&self) -> Result<Descriptor> {
        Descriptor::from_bytes(self.bytes()).map_err(|error| error.to_string())
    }

    fn fields(&self) -> Result<(usize, Vec<Field>)> {
        let descriptor = self.parsed()?;
        let header = Descriptor { name: descriptor.name.clone(), class_id: descriptor.class_id.clone(), items: Vec::new() }.to_bytes().len();
        let mut cursor = self.span.start.checked_add(header).ok_or("sidecar offset overflow")?;
        let mut fields = Vec::with_capacity(descriptor.items.len().min(32));
        for (key, value) in descriptor.items {
            // Serialization supplies only the parsed item's LENGTH. Original bytes, including
            // unknown bool encodings/NaN payloads, are copied from the source ranges below.
            let size = key_len(&key).checked_add(value_bytes(value.clone())?.len()).ok_or("sidecar field length overflow")?;
            let end = cursor.checked_add(size).filter(|end| *end <= self.span.end).ok_or("invalid sidecar field span")?;
            fields.push(Field { key_len: key_len(&key), key, value, span: cursor..end });
            cursor = end;
        }
        if cursor != self.span.end {
            return Err("sidecar descriptor length changed while locating fields".into());
        }
        Ok((header, fields))
    }

    fn list(&self, name: &str) -> Result<Vec<Self>> {
        let (_, fields) = self.fields()?;
        let mut matching = fields.into_iter().filter(|field| field.key.is(name));
        let field = matching.next().ok_or_else(|| format!("missing sidecar {name}"))?;
        if matching.next().is_some() {
            return Err(format!("duplicate sidecar {name}"));
        }
        let D::List(values) = field.value else {
            return Err(format!("sidecar {name} must be a list"));
        };
        if values.len() > atn::MAX_TOTAL_ACTIONS {
            return Err("too many sidecar list entries".into());
        }
        let mut cursor = field.span.start.checked_add(field.key_len).and_then(|offset| offset.checked_add(8)).ok_or("sidecar offset overflow")?;
        let mut out = Vec::with_capacity(values.len().min(256));
        for value in values {
            let D::Descriptor(descriptor) = value else {
                return Err(format!("sidecar {name} must contain objects"));
            };
            let start = cursor.checked_add(4).ok_or("sidecar offset overflow")?;
            let end = start.checked_add(descriptor.to_bytes().len()).filter(|end| *end <= field.span.end).ok_or("invalid sidecar object span")?;
            if self.source.get(cursor..start) != Some(b"Objc") {
                return Err("sidecar list object has an unsupported kind".into());
            }
            out.push(Self { source: self.source.clone(), span: start..end });
            cursor = end;
        }
        if cursor != field.span.end {
            return Err("sidecar list has trailing bytes".into());
        }
        Ok(out)
    }

    /// Replace only named fields; untouched source field bytes and key encodings stay exact.
    fn patch(&self, changes: &[(&str, Option<Vec<u8>>)]) -> Result<Vec<u8>> {
        let (header, fields) = self.fields()?;
        let mut seen = vec![false; changes.len()];
        let mut items = Vec::new();
        let mut count = 0usize;
        for field in fields {
            let found = changes.iter().position(|(key, _)| field.key.is(key));
            if let Some(index) = found {
                if seen.get(index).copied() == Some(true) {
                    return Err("cannot edit a duplicate sidecar field".into());
                }
                if let Some(seen) = seen.get_mut(index) {
                    *seen = true;
                }
                if let Some(Some(value)) = changes.get(index).map(|(_, value)| value.as_ref()) {
                    let end = field.span.start.checked_add(field.key_len).ok_or("sidecar offset overflow")?;
                    extend(&mut items, self.source.get(field.span.start..end).ok_or("invalid sidecar key span")?)?;
                    extend(&mut items, value)?;
                    count = count.checked_add(1).ok_or("sidecar count overflow")?;
                }
            } else {
                extend(&mut items, self.source.get(field.span).ok_or("invalid sidecar field span")?)?;
                count = count.checked_add(1).ok_or("sidecar count overflow")?;
            }
        }
        for (index, (key, value)) in changes.iter().enumerate() {
            if !seen.get(index).copied().unwrap_or(false)
                && let Some(value) = value
            {
                let key = Id::new(key);
                match key {
                    Id::Code(bytes) => {
                        extend(&mut items, &0u32.to_be_bytes())?;
                        extend(&mut items, &bytes)?;
                    }
                    Id::Str(bytes) => {
                        let len = u32::try_from(bytes.len()).map_err(|_| "sidecar key is too long")?;
                        extend(&mut items, &len.to_be_bytes())?;
                        extend(&mut items, &bytes)?;
                    }
                }
                extend(&mut items, value)?;
                count = count.checked_add(1).ok_or("sidecar count overflow")?;
            }
        }
        let prefix = header.checked_sub(4).ok_or("invalid sidecar header")?;
        let mut out = self.bytes().get(..prefix).ok_or("invalid sidecar header")?.to_vec();
        extend(&mut out, &u32::try_from(count).map_err(|_| "too many sidecar fields")?.to_be_bytes())?;
        extend(&mut out, &items)?;
        Ok(out)
    }
}

fn objects(entries: &[Vec<u8>]) -> Result<Vec<u8>> {
    if entries.len() > atn::MAX_TOTAL_ACTIONS {
        return Err("too many sidecar objects".into());
    }
    let mut out = b"VlLs".to_vec();
    out.extend_from_slice(&u32::try_from(entries.len()).map_err(|_| "too many sidecar objects")?.to_be_bytes());
    for entry in entries {
        let len = out.len().checked_add(4).and_then(|len| len.checked_add(entry.len())).ok_or("sidecar size overflow")?;
        if len > MAX_BYTES {
            return Err("sidecar exceeds the action-file byte limit".into());
        }
        out.extend_from_slice(b"Objc");
        out.extend(entry);
    }
    Ok(out)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Source {
    wrapper: Vec<u8>,
    outer: RawDescriptor,
    root: RawDescriptor,
    signature: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SetMetadata {
    raw: RawDescriptor,
    source: Arc<Source>,
    ordinal: usize,
}

/// Runtime preservation data. The public protocol still edits only description/icon strings.
/// Bytes, not floating-point descriptor equality, define clone/dirty/stale comparisons.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionMetadata {
    raw: RawDescriptor,
    set: Option<Arc<SetMetadata>>,
    source: Arc<Source>,
    description: String,
    icon: String,
    thumbnail: i32,
    reset_icon: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SetRowMetadata {
    pub(crate) slot: usize,
    pub(crate) row: String,
    pub(crate) source: Arc<SetMetadata>,
}

/// Root's editor.update hook calls this after validating an explicitly supplied icon string.
pub(crate) fn explicit_icon(action: &mut Action) {
    if let Some(metadata) = action.atn_metadata.as_mut()
        && action.icon.is_empty()
        && !matches!(metadata.thumbnail, 0 | 3)
    {
        metadata.reset_icon = true;
    }
}

pub(super) fn empty_source(state: &ActionState, slot: usize) -> Option<Arc<SetMetadata>> {
    state.atn_set_rows.iter().find(|entry| entry.slot == slot && state.sets.get(slot) == Some(&entry.row)).map(|entry| entry.source.clone())
}

pub(crate) fn install_row(state: &mut ActionState, name: &str, source: Option<Arc<SetMetadata>>) {
    let slot = state.sets.len();
    state.library.rows.resize(slot, None);
    state.library.rows.push(None);
    state.sets.push(name.to_string());
    if let Some(source) = source {
        state.atn_set_rows.push(SetRowMetadata { slot, row: name.to_string(), source });
    }
}

/// Copy every physical row of one logical folder, retaining empty-set metadata and moving
/// surviving row carriers with the insertion. Action clones already retain their raw entries.
pub(crate) fn duplicate_set_rows(state: &mut ActionState, source: &str, name: &str) {
    let rows: Vec<_> = state.sets.iter().enumerate().filter_map(|(slot, row)| (row == source).then_some(slot)).collect();
    let at = rows.first().map_or(state.sets.len(), |slot| slot + 1);
    let mut copied: Vec<_> = rows.iter().map(|&slot| empty_source(state, slot)).collect();
    if copied.is_empty() {
        copied.push(group_source(state, source));
    }
    let count = copied.len();
    state.library.rows.resize(state.sets.len(), None);
    let raw: Vec<_> = if rows.is_empty() { vec![None] } else { rows.iter().map(|&slot| state.library.rows.get(slot).cloned().flatten()).collect() };
    state.library.rows.splice(at..at, raw);
    for entry in &mut state.atn_set_rows {
        if entry.slot >= at {
            entry.slot = entry.slot.saturating_add(count);
        }
    }
    state.sets.splice(at..at, std::iter::repeat_n(name.to_string(), count));
    for (offset, source) in copied.into_iter().enumerate() {
        if let Some(source) = source {
            state.atn_set_rows.push(SetRowMetadata { slot: at + offset, row: name.to_string(), source });
        }
    }
}

/// Retire selected CURRENT rows and move surviving carriers with their rows.
pub(crate) fn retire_sets(state: &mut ActionState, names: &std::collections::HashSet<String>) {
    let metadata: std::collections::BTreeMap<usize, SetRowMetadata> =
        std::mem::take(&mut state.atn_set_rows).into_iter().map(|entry| (entry.slot, entry)).collect();
    let rows = std::mem::take(&mut state.sets);
    let origins = std::mem::take(&mut state.library.rows);
    for (old, row) in rows.into_iter().enumerate() {
        if names.contains(&row) {
            continue;
        }
        let slot = state.sets.len();
        if let Some(entry) = metadata.get(&old).filter(|entry| entry.row == row) {
            state.atn_set_rows.push(SetRowMetadata { slot, row: row.clone(), source: entry.source.clone() });
        }
        state.library.rows.push(origins.get(old).cloned().flatten());
        state.sets.push(row);
    }
}

fn group_source(state: &ActionState, name: &str) -> Option<Arc<SetMetadata>> {
    let actions: Vec<&Action> = state.list.iter().filter(|action| super::logical_set_name(action.set.as_deref()) == name).collect();
    let mut source = match source_for(&actions) {
        Ok(source) => source,
        Err(_) => return None,
    };
    for (slot, _) in state.sets.iter().enumerate().filter(|(_, row)| row.as_str() == name) {
        if let Some(candidate) = empty_source(state, slot) {
            if let Some(current) = &source {
                if !same_set(current, &candidate) {
                    return None;
                }
            } else {
                source = Some(candidate);
            }
        }
    }
    source
}

fn same_set(left: &Arc<SetMetadata>, right: &Arc<SetMetadata>) -> bool {
    Arc::ptr_eq(&left.source, &right.source) && left.ordinal == right.ordinal
}

/// Small assignSet hook: preserve the action entry but retire its old set binding on a real move.
pub(crate) fn assign_set_metadata(state: &mut ActionState, action: usize, target: Option<&str>) {
    let Some(current) = state.list.get(action) else { return };
    if super::logical_set_name(current.set.as_deref()) == super::logical_set_name(target) {
        return;
    }
    let source = group_source(state, super::logical_set_name(target));
    if let Some(metadata) = state.list.get_mut(action).and_then(|action| action.atn_metadata.as_mut()) {
        metadata.set = source;
    }
}

pub(crate) fn selection<'a>(state: &ActionState, action: &'a Action) -> Selection<'a> {
    let source =
        action.atn_metadata.as_ref().and_then(|metadata| metadata.set.clone()).or_else(|| group_source(state, super::logical_set_name(action.set.as_deref())));
    Selection { actions: vec![action], source, raw: state.library.row_source(&state.sets, action) }
}

pub(crate) fn groups<'a>(state: &'a ActionState, only: Option<&str>) -> Result<Vec<(&'a str, Selection<'a>)>> {
    if state.list.len() > atn::MAX_TOTAL_ACTIONS || state.sets.len() > atn::MAX_SETS {
        return Err("action library exceeds archive limits".into());
    }
    let raw_rows = state.library.row_index(&state.sets);
    let mut names = std::collections::HashSet::new();
    let mut out: Vec<(&str, Selection<'a>)> = Vec::new();
    for name in state.list.iter().map(|action| super::logical_set_name(action.set.as_deref())).chain(state.sets.iter().map(String::as_str)) {
        if only.is_some_and(|only| only != name) || !names.insert(name) {
            continue;
        }
        let fallback = group_source(state, name);
        let before = out.len();
        for action in state.list.iter().filter(|action| super::logical_set_name(action.set.as_deref()) == name) {
            let source = action.atn_metadata.as_ref().and_then(|metadata| metadata.set.clone()).or_else(|| fallback.clone());
            let raw = raw_rows.source(action);
            let existing = out.last_mut().filter(|(previous, selection)| {
                *previous == name
                    && selection.raw == raw
                    && match (&selection.source, &source) {
                        (None, None) => true,
                        (Some(left), Some(right)) => same_set(left, right),
                        _ => false,
                    }
            });
            if let Some((_, selection)) = existing {
                selection.actions.push(action);
            } else {
                if out.len() >= atn::MAX_SETS {
                    return Err("too many sets in actions archive".into());
                }
                out.push((name, Selection { actions: vec![action], source, raw }));
            }
        }
        for (slot, _) in state.sets.iter().enumerate().filter(|(_, row)| row.as_str() == name) {
            let raw = state.library.rows.get(slot).cloned().flatten();
            if let Some(raw) = raw.as_ref()
                && !out.iter().skip(before).any(|(_, selection)| selection.raw.as_ref() == Some(raw))
            {
                if out.len() >= atn::MAX_SETS {
                    return Err("too many sets in actions archive".into());
                }
                out.push((name, Selection { actions: Vec::new(), source: empty_source(state, slot), raw: Some(raw.clone()) }));
            }
            if let Some(source) = empty_source(state, slot)
                && !out.iter().skip(before).any(|(_, selection)| selection.source.as_ref().is_some_and(|existing| same_set(existing, &source)))
            {
                if out.len() >= atn::MAX_SETS {
                    return Err("too many sets in actions archive".into());
                }
                out.push((name, Selection { actions: Vec::new(), source: Some(source), raw: state.library.rows.get(slot).cloned().flatten() }));
            }
        }
        if out.len() == before {
            if out.len() >= atn::MAX_SETS {
                return Err("too many sets in actions archive".into());
            }
            out.push((name, Selection { actions: Vec::new(), source: fallback, raw: None }));
        }
    }
    // Preserve physical rows only when doing so preserves the panel's group/action order.
    // An explicit move can change that projection, including S1/S2/S1 runs within one group.
    let mut slots = std::collections::HashMap::new();
    for row in &state.atn_set_rows {
        if state.sets.get(row.slot) == Some(&row.row) {
            let key = (row.row.as_str(), Arc::as_ptr(&row.source.source), row.source.ordinal);
            slots.entry(key).and_modify(|slot: &mut usize| *slot = (*slot).min(row.slot)).or_insert(row.slot);
        }
    }
    let raw_slots: std::collections::HashMap<_, _> =
        state.library.rows.iter().enumerate().filter_map(|(slot, source)| Some(((state.sets.get(slot)?.as_str(), source.as_ref()?.key()), slot))).collect();
    let mut physical: Vec<_> = (0..out.len()).collect();
    physical.sort_by_cached_key(|&index| {
        let (name, selection) = &out[index];
        selection
            .raw
            .as_ref()
            .and_then(|raw| raw_slots.get(&(*name, raw.key())).copied())
            .or_else(|| selection.source.as_ref().and_then(|source| slots.get(&(*name, Arc::as_ptr(&source.source), source.ordinal)).copied()))
            .unwrap_or(usize::MAX)
    });
    if panel_projection(&out, 0..out.len()) != panel_projection(&out, physical.iter().copied()) {
        return Ok(out);
    }
    Ok(physical
        .into_iter()
        .map(|index| {
            let (name, selection) = &out[index];
            (*name, Selection { actions: selection.actions.clone(), source: selection.source.clone(), raw: selection.raw.clone() })
        })
        .collect())
}

/// Temporary pointer identities compare panel order without comparing action/descriptor values.
fn panel_projection<'a>(groups: &[(&'a str, Selection<'a>)], order: impl IntoIterator<Item = usize>) -> Vec<(&'a str, Vec<*const Action>)> {
    let mut actions = std::collections::HashMap::<&str, Vec<*const Action>>::new();
    let mut all = Vec::new();
    let mut nonempty = Vec::new();
    for index in order {
        let (name, selection) = &groups[index];
        let members = actions.entry(*name).or_insert_with(|| {
            all.push(*name);
            Vec::new()
        });
        if members.is_empty() && !selection.actions.is_empty() {
            nonempty.push(*name);
        }
        members.extend(selection.actions.iter().map(|action| std::ptr::from_ref(*action)));
    }
    all.retain(|name| actions.get(name).is_some_and(Vec::is_empty));
    nonempty.extend(all);
    nonempty.into_iter().map(|name| (name, actions.remove(name).unwrap_or_default())).collect()
}

pub(crate) struct Decoded {
    pub(crate) hidden: Option<usize>,
    pub(crate) sets: Vec<Option<Arc<SetMetadata>>>,
    pub(crate) actions: Vec<Vec<Option<ActionMetadata>>>,
    pub(crate) source: Option<Arc<Source>>,
}

fn unique<'a>(descriptor: &'a Descriptor, key: &str) -> Result<&'a D> {
    let mut fields = descriptor.items.iter().filter(|(id, _)| id.is(key));
    let value = fields.next().map(|(_, value)| value).ok_or_else(|| format!("missing sidecar {key}"))?;
    if fields.next().is_some() {
        return Err(format!("duplicate sidecar {key}"));
    }
    Ok(value)
}

pub(super) fn has_payload(item: &AtnItem) -> bool {
    (item.event.is("stop") || item.event.is("Stop")) && item.descriptor.as_ref().is_some_and(|descriptor| descriptor.get(MODERN).is_some())
}

fn payload(file: &AtnFile) -> Option<&[u8]> {
    if file.name.to_string_lossy() != "Action sidecar" || file.expanded {
        return None;
    }
    let [action] = file.actions.as_slice() else { return None };
    if action.name.to_string_lossy() != "Play for details" || action.expanded || action.function_key != 0 || action.shift || action.command || action.color != 0
    {
        return None;
    }
    let [item] = action.items.as_slice() else { return None };
    if item.expanded || !item.enabled || !item.with_dialog || item.dialog_options != 2 || item.dictionary != b"Stop" || !has_payload(item) {
        return None;
    }
    let descriptor = item.descriptor.as_ref()?;
    if !descriptor.class_id.is("null") || !descriptor.name.0.is_empty() {
        return None;
    }
    if !matches!(descriptor.get("Msge"), Some(D::Text(_))) || descriptor.get("Cntn") != Some(&D::Boolean(true)) {
        return None;
    }
    let D::RawData(bytes) = descriptor.get(MODERN)? else { return None };
    if bytes.get(..4) != Some(&16u32.to_be_bytes()) {
        return None;
    }
    bytes.get(4..)
}

pub(crate) fn is_wrapper(file: &AtnFile) -> bool {
    payload(file).is_some()
}

pub(crate) fn legacy_origin(source: Option<&Source>) -> Result<Option<crate::actions_cmds::library::LegacyOrigin>> {
    let Some(source) = source else { return Ok(None) };
    let root = source.root.parsed()?;
    let mut fields = root.items.iter().filter(|(id, _)| id.is("photocraftLegacyJsonV1"));
    let Some((_, value)) = fields.next() else { return Ok(None) };
    if fields.next().is_some() {
        return Err("duplicate legacy Actions origin".into());
    }
    let D::Descriptor(value) = value else { return Err("invalid legacy Actions origin".into()) };
    crate::actions_cmds::library::LegacyOrigin::from_descriptor(value).map(Some)
}

pub(super) fn decode(files: &[AtnFile]) -> Result<Decoded> {
    decode_inner(files, false, None)
}

pub(crate) fn decode_library(files: &[AtnFile], raw_outer: Option<&[u8]>) -> Result<Decoded> {
    decode_inner(files, true, raw_outer)
}

fn decode_inner(files: &[AtnFile], library: bool, raw_outer: Option<&[u8]>) -> Result<Decoded> {
    let mut result =
        Decoded { hidden: None, sets: vec![None; files.len()], actions: files.iter().map(|file| vec![None; file.actions.len()]).collect(), source: None };
    for (hidden, file) in files.iter().enumerate() {
        let Some(bytes) = payload(file) else { continue };
        if hidden.checked_add(1) != Some(files.len()) {
            return Err("Actions sidecar must be the final physical set".into());
        }
        if result.hidden.is_some() {
            return Err("ambiguous multiple Actions sidecars".into());
        }
        let root = RawDescriptor::new(bytes)?;
        let parsed = root.parsed()?;
        if !parsed.class_id.is("null") || !parsed.name.0.is_empty() {
            return Err("unsupported Actions sidecar root".into());
        }
        let sets = root.list("sets")?;
        if sets.is_empty() && !library {
            return Err("Actions sidecar requires at least one real set".into());
        }
        if sets.len().checked_add(1) != Some(files.len()) {
            return Err("sidecar set count does not match the action file".into());
        }
        let descriptor =
            file.actions.first().and_then(|action| action.items.first()).and_then(|item| item.descriptor.as_ref()).ok_or("missing sidecar descriptor")?;
        unique(descriptor, MODERN)?;
        unique(descriptor, "Msge")?;
        unique(descriptor, "Cntn")?;
        let outer = match raw_outer {
            Some(bytes) => RawDescriptor::new(bytes)?,
            None => RawDescriptor::new(&descriptor.to_bytes())?,
        };
        let outer_signature = outer.patch(&[(MODERN, Some(value_bytes(D::RawData(Vec::new()))?))])?;
        let mut signature_file = file.clone();
        let signature_item = signature_file.actions.first_mut().and_then(|action| action.items.first_mut()).ok_or("missing sidecar signature item")?;
        signature_item.descriptor = Some(Descriptor::from_bytes(&outer_signature).map_err(|error| error.to_string())?);
        let mut signature = atn::write(&signature_file).map_err(|error| error.to_string())?;
        signature.extend(root.patch(&[("sets", Some(objects(&[])?)), ("photocraftLegacyJsonV1", None)])?);
        let source = Arc::new(Source { wrapper: atn::write(file).map_err(|error| error.to_string())?, outer, root, signature });
        result.source = Some(source.clone());
        let mut ordinal = 0usize;
        for (index, real) in files.iter().enumerate().filter(|(index, _)| *index != hidden) {
            let raw = sets.get(ordinal).ok_or("missing positional sidecar set")?.clone();
            let parsed = raw.parsed()?;
            if !parsed.class_id.is("null") || !parsed.name.0.is_empty() || !matches!(unique(&parsed, "setColorIndex")?, D::Integer(_)) {
                return Err("invalid positional sidecar set".into());
            }
            let entries = raw.list("actions")?;
            if entries.len() != real.actions.len() {
                return Err("sidecar action count does not match its set".into());
            }
            let set = Arc::new(SetMetadata { raw, source: source.clone(), ordinal });
            if let Some(slot) = result.sets.get_mut(index) {
                *slot = Some(set.clone());
            }
            for (at, raw) in entries.into_iter().enumerate() {
                let parsed = raw.parsed()?;
                if !parsed.class_id.is("null") || !parsed.name.0.is_empty() {
                    return Err("invalid sidecar action object".into());
                }
                let D::Text(description) = unique(&parsed, "actionDescriptionKey")? else {
                    return Err("invalid action description".into());
                };
                let D::Integer(thumbnail) = unique(&parsed, "actionThumbnailTypeKey")? else {
                    return Err("invalid action thumbnail type".into());
                };
                if !matches!(unique(&parsed, "actionPreviewKey")?, D::Boolean(_))
                    || !matches!(unique(&parsed, "actionGenerativeKey")?, D::Boolean(_))
                    || !matches!(unique(&parsed, "actionPreconditionVal")?, D::Integer(_))
                {
                    return Err("invalid measured action metadata fields".into());
                }
                let icon = if *thumbnail == 3 {
                    let D::Text(icon) = unique(&parsed, "actionIconKey")? else {
                        return Err("invalid action icon".into());
                    };
                    icon.to_string_lossy()
                } else {
                    String::new()
                };
                // Unknown values of the other measured properties remain opaque and unchanged.
                let metadata = ActionMetadata {
                    raw,
                    set: Some(set.clone()),
                    source: source.clone(),
                    description: description.to_string_lossy(),
                    icon,
                    thumbnail: *thumbnail,
                    reset_icon: false,
                };
                if let Some(slot) = result.actions.get_mut(index).and_then(|actions| actions.get_mut(at)) {
                    *slot = Some(metadata);
                }
            }
            ordinal = ordinal.checked_add(1).ok_or("sidecar position overflow")?;
        }
        result.hidden = Some(hidden);
    }
    Ok(result)
}

pub(crate) fn apply(action: &mut Action, metadata: ActionMetadata) {
    action.description = metadata.description.clone();
    action.icon = metadata.icon.clone();
    action.atn_metadata = Some(metadata);
}

fn entry(action: &Action) -> Result<Vec<u8>> {
    // Bound strings before UnicodeString allocates their UTF16 representation.
    let units = action.description.encode_utf16().count().checked_add(action.icon.encode_utf16().count()).ok_or("metadata string length overflow")?;
    if units.checked_mul(2).is_none_or(|bytes| bytes > MAX_BYTES.saturating_sub(1024)) {
        return Err("action metadata strings exceed the action-file byte limit".into());
    }
    if let Some(metadata) = &action.atn_metadata {
        let mut changes = Vec::new();
        if action.description != metadata.description {
            changes.push(("actionDescriptionKey", Some(value_bytes(D::Text(UnicodeString::new(&action.description)))?)));
        }
        if action.icon != metadata.icon || metadata.reset_icon {
            changes.push(("actionThumbnailTypeKey", Some(value_bytes(D::Integer(if action.icon.is_empty() { 0 } else { 3 }))?)));
            changes.push(("actionIconKey", if action.icon.is_empty() { None } else { Some(value_bytes(D::Text(UnicodeString::new(&action.icon)))?) }));
        }
        return if changes.is_empty() { Ok(metadata.raw.bytes().to_vec()) } else { metadata.raw.patch(&changes) };
    }
    let mut descriptor = Descriptor::new("null")
        .with("actionDescriptionKey", D::Text(UnicodeString::new(&action.description)))
        .with("actionThumbnailTypeKey", D::Integer(if action.icon.is_empty() { 0 } else { 3 }));
    if !action.icon.is_empty() {
        descriptor = descriptor.with("actionIconKey", D::Text(UnicodeString::new(&action.icon)));
    }
    Ok(descriptor
        .with("actionPreviewKey", D::Boolean(false))
        .with("actionGenerativeKey", D::Boolean(false))
        .with("actionPreconditionVal", D::Integer(-1))
        .to_bytes())
}

pub(crate) struct Selection<'a> {
    pub(crate) actions: Vec<&'a Action>,
    pub(crate) source: Option<Arc<SetMetadata>>,
    pub(crate) raw: Option<crate::actions_cmds::library::RowOrigin>,
}

pub(super) fn source_for(actions: &[&Action]) -> Result<Option<Arc<SetMetadata>>> {
    let mut source: Option<Arc<SetMetadata>> = None;
    for candidate in actions.iter().filter_map(|action| action.atn_metadata.as_ref()).filter_map(|metadata| metadata.set.as_ref()) {
        if let Some(source) = &source {
            if !Arc::ptr_eq(&source.source, &candidate.source) || source.ordinal != candidate.ordinal {
                return Err("selected actions have conflicting source-set metadata; export their sets separately".into());
            }
        } else {
            source = Some(candidate.clone());
        }
    }
    Ok(source)
}

pub(super) fn append(files: &mut Vec<AtnFile>, selected: &[Selection<'_>]) -> Result<()> {
    append_inner(files, selected, false, None, None).map(|_| ())
}

pub(crate) fn append_library(
    files: &mut Vec<AtnFile>,
    selected: &[Selection<'_>],
    source: Option<Arc<Source>>,
    receipt: Option<&crate::actions_cmds::library::LegacyOrigin>,
) -> Result<Option<Vec<u8>>> {
    let force = files.is_empty() || receipt.is_some() || source.is_some();
    append_inner(files, selected, force, source, receipt)
}

fn append_inner(
    files: &mut Vec<AtnFile>,
    selected: &[Selection<'_>],
    force: bool,
    source: Option<Arc<Source>>,
    receipt: Option<&crate::actions_cmds::library::LegacyOrigin>,
) -> Result<Option<Vec<u8>>> {
    if files.len() != selected.len() {
        return Err("sidecar export selection mismatch".into());
    }
    let needed = selected.iter().any(|selection| {
        selection.source.is_some()
            || selection.actions.iter().any(|action| action.atn_metadata.is_some() || !action.description.is_empty() || !action.icon.is_empty())
    });
    if !needed && !force {
        return Ok(None);
    }
    let actions = files.iter().try_fold(1usize, |count, file| count.checked_add(file.actions.len())).ok_or("sidecar action count overflow")?;
    let steps = files
        .iter()
        .flat_map(|file| &file.actions)
        .try_fold(1usize, |count, action| count.checked_add(action.items.len()))
        .ok_or("sidecar step count overflow")?;
    if files.len() >= atn::MAX_SETS || actions > atn::MAX_TOTAL_ACTIONS || steps > atn::MAX_TOTAL_ITEMS {
        return Err("actions archive leaves no space for its metadata sidecar".into());
    }
    let mut source: Option<Arc<Source>> = source;
    let mut sets = Vec::with_capacity(selected.len());
    let mut set_bytes = 8usize;
    for selection in selected {
        let set_source = selection.source.clone().or(source_for(&selection.actions)?);
        let sources = set_source
            .iter()
            .map(|set| &set.source)
            .chain(selection.actions.iter().filter_map(|action| action.atn_metadata.as_ref()).map(|metadata| &metadata.source));
        for candidate in sources {
            if let Some(source) = &source {
                if source.signature != candidate.signature {
                    return Err("source sidecar notes or archive fields conflict; export separately".into());
                }
            } else {
                source = Some(candidate.clone());
            }
        }
        let mut entries = Vec::with_capacity(selection.actions.len().min(256));
        let mut entry_bytes = 8usize;
        for action in &selection.actions {
            let entry = entry(action)?;
            entry_bytes = entry_bytes
                .checked_add(4)
                .and_then(|count| count.checked_add(entry.len()))
                .filter(|count| *count <= MAX_BYTES)
                .ok_or("sidecar action list exceeds the action-file byte limit")?;
            entries.push(entry);
        }
        let actions = objects(&entries)?;
        let set = match set_source {
            Some(set) => set.raw.patch(&[("actions", Some(actions))])?,
            None => {
                let raw = RawDescriptor::new(&Descriptor::new("null").with("setColorIndex", D::Integer(0)).with("actions", D::List(Vec::new())).to_bytes())?;
                raw.patch(&[("actions", Some(actions))])?
            }
        };
        set_bytes = set_bytes
            .checked_add(4)
            .and_then(|count| count.checked_add(set.len()))
            .filter(|count| *count <= MAX_BYTES)
            .ok_or("sidecar set list exceeds the action-file byte limit")?;
        sets.push(set);
    }
    let root = match &source {
        Some(source) => source.root.patch(&[("sets", Some(objects(&sets)?))])?,
        None => RawDescriptor::new(&Descriptor::new("null").with("sets", D::List(Vec::new())).to_bytes())?.patch(&[("sets", Some(objects(&sets)?))])?,
    };
    let root = if let Some(receipt) = receipt {
        RawDescriptor::new(&root)?.patch(&[("photocraftLegacyJsonV1", Some(value_bytes(D::Descriptor(receipt.descriptor()?))?))])?
    } else {
        root
    };
    let mut payload = 16u32.to_be_bytes().to_vec();
    extend(&mut payload, &root)?;
    let outer = match &source {
        Some(source) => source.outer.patch(&[(MODERN, Some(value_bytes(D::RawData(payload))?))])?,
        None => Descriptor::new("null")
            .with("Msge", D::Text(UnicodeString::new("This set stores additional action details. Older editors may ignore it.")))
            .with("Cntn", D::Boolean(true))
            .with(MODERN, D::RawData(payload))
            .to_bytes(),
    };
    let descriptor = Descriptor::from_bytes(&outer).map_err(|error| error.to_string())?;
    let mut wrapper = if let Some(source) = source {
        atn::parse(&source.wrapper).map_err(|error| error.to_string())?
    } else {
        AtnFile {
            name: UnicodeString::new_nul("Action sidecar"),
            actions: vec![AtnAction {
                name: UnicodeString::new_nul("Play for details"),
                items: vec![AtnItem {
                    event: AtnEvent::name("stop"),
                    dictionary: b"Stop".to_vec(),
                    enabled: true,
                    with_dialog: true,
                    dialog_options: 2,
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }
    };
    let item = wrapper.actions.first_mut().and_then(|action| action.items.first_mut()).ok_or("missing output sidecar wrapper")?;
    item.descriptor = Some(descriptor);
    files.push(wrapper);
    Ok(Some(outer))
}

#[cfg(test)]
#[path = "sidecar_tests.rs"]
mod tests;
