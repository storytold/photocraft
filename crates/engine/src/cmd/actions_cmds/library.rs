//! The sole durable Actions library. Runtime origins refer to physical ATN records, never names.
use std::{ops::Range, sync::Arc};

use craft_psd::{
    atn::{self, AtnAction, AtnFile, SpannedAtnArchive},
    descriptor::{Descriptor, UnicodeString, Value as D},
};

use super::{Action, ActionState, color_to_atn};
use crate::atn_cmds::{self, sidecar};

pub(crate) mod identity_manifest;
use identity_manifest::{GroupKey, IdentityManifest};

type Result<T> = std::result::Result<T, String>;
const MAX_BYTES: usize = crate::preset_store::MAX_ACTIONS_BYTES as usize;

/// Check the encoded action-count ceiling before a definition edit is published.
/// The metadata carrier consumes one real ATN action even though it is hidden in the panel.
pub(crate) fn check_action_count<'a>(state: &ActionState, actions: impl IntoIterator<Item = &'a Action>) -> Result<()> {
    let mut count = 0usize;
    let mut carrier = state.library.sidecar.is_some() || state.library.legacy.is_some() || !state.atn_set_rows.is_empty() || !state.sets.is_empty();
    for action in actions {
        count = count.checked_add(1).ok_or("Actions count overflow")?;
        carrier |= sidecar::action_needs_metadata(action)
            || action.action_id.is_some()
            || action.legacy_shortcut_owner
            || (action.set.is_some() && action.set_slot.is_none());
    }
    if count.checked_add(usize::from(carrier)).is_none_or(|total| total > atn::MAX_TOTAL_ACTIONS) {
        return Err("actions archive leaves no space for the action and its metadata sidecar".into());
    }
    Ok(())
}

/// Copy only persisted definitions and row provenance for pre-publication encoding.
/// Recording, editor ownership and playback counters remain in the live state.
pub(crate) fn definition_copy(state: &ActionState) -> ActionState {
    ActionState {
        list: state.list.clone(),
        library: state.library.clone(),
        sets: state.sets.clone(),
        set_rows: state.set_rows.clone(),
        set_rows_revision: state.set_rows_revision,
        atn_set_rows: state.atn_set_rows.clone(),
        ..Default::default()
    }
}

/// Use the durable encoder's complete limits before publishing a proposed definition.
/// This checks eligibility, while backend I/O retains its ordinary warning/retry semantics.
pub(crate) fn check_definition(state: &ActionState, shortcuts: &std::collections::BTreeMap<String, String>) -> Result<()> {
    state
        .list
        .iter()
        .try_fold(0usize, |total, action| total.checked_add(action.steps.len()))
        .filter(|total| *total <= atn::MAX_TOTAL_ITEMS)
        .ok_or("too many native steps in Actions library")?;
    let keys: Vec<_> = shortcuts.iter().filter(|(key, _)| super::is_action_key(key)).map(|(key, value)| (key.clone(), value.clone())).collect();
    encode(state, &keys).map(|_| ())
}

#[derive(Debug)]
struct Archive {
    bytes: Arc<[u8]>,
    parsed: SpannedAtnArchive,
    hidden: Option<usize>,
}

impl PartialEq for Archive {
    /// Archive equality denotes the same parsed input, not equal bytes from a separate load.
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }
}

/// An action's original record and the native projection used to detect actual step edits.
#[derive(Clone, Debug, PartialEq)]
pub struct ActionOrigin {
    row: RowOrigin,
    action: usize,
    baseline: Arc<Action>,
    cohorts: Vec<Range<usize>>,
}

/// One native part of one original event. Step remapping moves this with the step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepOrigin {
    item: usize,
    part: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RowOrigin {
    archive: Arc<Archive>,
    set: usize,
    logical_name: String,
    metadata: Option<Arc<sidecar::SetMetadata>>,
}

impl RowOrigin {
    /// Transient physical row identity; repeated display names remain distinct.
    pub(crate) fn key(&self) -> (usize, usize) {
        (Arc::as_ptr(&self.archive) as usize, self.set)
    }
    /// Resolve this physical row in its retained archive, rejecting an invalid ordinal.
    fn file(&self) -> Result<&AtnFile> {
        self.archive.parsed.sets.get(self.set).ok_or_else(|| "missing original set".into())
    }
}

/// Receipt inside the ATN sidecar, sufficient to retire only the exact legacy input on restart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LegacyOrigin {
    bytes: usize,
    digest: String,
}

impl LegacyOrigin {
    /// Bind a bounded legacy input by byte count and digest before recording a migration receipt.
    pub(crate) fn new(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_BYTES {
            return Err("legacy Actions library exceeds byte limit".into());
        }
        Ok(Self { bytes: bytes.len(), digest: blake3::hash(bytes).to_hex().to_string() })
    }
    /// Require the exact original legacy bytes, including for same-size replacement inputs.
    pub(crate) fn matches(&self, bytes: &[u8]) -> bool {
        bytes.len() == self.bytes && blake3::hash(bytes).to_hex().as_str() == self.digest
    }
    /// Encode the receipt version, bounded byte count and digest into the library sidecar.
    pub(crate) fn descriptor(&self) -> Result<Descriptor> {
        Ok(Descriptor::new("null")
            .with("version", D::Integer(1))
            .with("bytes", D::Integer(i32::try_from(self.bytes).map_err(|_| "legacy length overflow")?))
            .with("digest", D::Text(UnicodeString::new(&self.digest))))
    }
    /// Validate the receipt schema, unique fields, byte ceiling and lowercase digest before use.
    pub(crate) fn from_descriptor(value: &Descriptor) -> Result<Self> {
        if !value.class_id.is("null") || !value.name.0.is_empty() || value.items.len() != 3 {
            return Err("invalid legacy Actions origin".into());
        }
        let field = |key: &str| {
            let mut values = value.items.iter().filter(|(id, _)| id.is(key));
            let value = values.next().map(|(_, value)| value).ok_or("missing legacy origin field")?;
            if values.next().is_some() {
                return Err("duplicate legacy origin field");
            }
            Ok(value)
        };
        if field("version")? != &D::Integer(1) {
            return Err("unsupported legacy Actions origin".into());
        }
        let D::Integer(length) = field("bytes")? else {
            return Err("invalid legacy origin length".into());
        };
        let bytes = usize::try_from(*length).ok().filter(|length| *length <= MAX_BYTES).ok_or("invalid legacy origin length")?;
        let D::Text(text) = field("digest")? else {
            return Err("invalid legacy origin digest".into());
        };
        let digest = String::from_utf16(&text.0).map_err(|_| "invalid legacy origin digest")?;
        if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) {
            return Err("invalid legacy origin digest".into());
        }
        Ok(Self { bytes, digest })
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Library {
    source: Option<Arc<Archive>>,
    pub(crate) rows: Vec<Option<RowOrigin>>,
    other_rows: Vec<(GroupKey, RowOrigin)>,
    order: Vec<GroupKey>,
    sidecar: Option<Arc<sidecar::Source>>,
    pub(crate) legacy: Option<LegacyOrigin>,
}

pub(crate) struct RowIndex<'a> {
    first: std::collections::HashMap<&'a str, RowOrigin>,
    exact: std::collections::HashMap<(&'a str, (usize, usize)), RowOrigin>,
}
impl RowIndex<'_> {
    /// Prefer the action's exact surviving physical row; otherwise use the first logical namesake.
    pub(crate) fn source(&self, action: &Action) -> Option<RowOrigin> {
        let name = atn_cmds::logical_set_name(action.set.as_deref());
        action.library_origin.as_ref().and_then(|origin| self.exact.get(&(name, origin.row.key()))).or_else(|| self.first.get(name)).cloned()
    }
}

impl Library {
    pub(crate) fn has_implicit_rows(&self) -> bool {
        self.other_rows.iter().any(|(key, _)| matches!(key, GroupKey::Implicit(_)))
    }
    /// Index surviving rows once, retaining both first-name resolution and exact provenance matches.
    pub(crate) fn row_index<'a>(&self, names: &'a [String]) -> RowIndex<'a> {
        let mut index = RowIndex { first: Default::default(), exact: Default::default() };
        for (name, row) in names.iter().zip(&self.rows) {
            if let Some(row) = row {
                index.first.entry(name.as_str()).or_insert_with(|| row.clone());
                index.exact.entry((name.as_str(), row.key())).or_insert_with(|| row.clone());
            }
        }
        index
    }
    pub(crate) fn row_source(&self, names: &[String], action: &Action) -> Option<RowOrigin> {
        action
            .set_slot
            .and_then(|slot| self.rows.get(slot).cloned().flatten())
            .or_else(|| action.library_origin.as_ref().map(|origin| origin.row.clone()))
            .or_else(|| self.row_index(names).source(action))
    }
}

/// A strict complete load; failure returns no partial library and executes no commands.
pub(crate) fn decode(bytes: &[u8]) -> Result<ActionState> {
    decode_inner(bytes, true)
}

/// External imports retain raw provenance but never trust local identity metadata.
pub(crate) fn decode_import(bytes: &[u8]) -> Result<ActionState> {
    decode_inner(bytes, false)
}

fn decode_inner(bytes: &[u8], trusted: bool) -> Result<ActionState> {
    if bytes.len() > MAX_BYTES {
        return Err("Actions library exceeds byte limit".into());
    }
    let parsed = atn::parse_sets_with_spans(bytes).map_err(|error| error.to_string())?;
    let raw_outer = parsed
        .sets
        .iter()
        .enumerate()
        .find_map(|(index, file)| sidecar::is_wrapper(file).then(|| parsed.spans.get(index)?.actions.first()?.items.first()?.descriptor.clone()).flatten())
        .and_then(|range| bytes.get(range));
    let mut metadata = sidecar::decode_library(&parsed.sets, raw_outer)?;
    let legacy = sidecar::legacy_origin(metadata.source.as_deref())?;
    let identity_bytes = if trusted { sidecar::identity_bytes(metadata.source.as_deref())? } else { None };
    let physical_rows = parsed.sets.len() - usize::from(metadata.hidden.is_some());
    let action_count = parsed.sets.iter().enumerate().filter(|(i, _)| Some(*i) != metadata.hidden).map(|(_, set)| set.actions.len()).sum();
    let manifest = identity_bytes.as_deref().map(|bytes| IdentityManifest::decode(bytes, action_count, physical_rows)).transpose()?;
    let archive = Arc::new(Archive { bytes: bytes.into(), parsed, hidden: metadata.hidden });
    let mut state = ActionState::default();
    state.library.source = Some(archive.clone());
    state.library.sidecar = metadata.source;
    state.library.legacy = legacy;
    let mut fingerprints = Vec::new();
    let mut total = 0usize;
    let mut set_names = atn_cmds::UniqueNames::new(
        std::iter::once(atn_cmds::UNGROUPED_SET.to_string()).chain(
            archive
                .parsed
                .sets
                .iter()
                .enumerate()
                .filter(|(index, _)| Some(*index) != archive.hidden)
                .map(|(_, file)| file.name.to_string_lossy())
                .filter(|name| !name.is_empty()),
        ),
    );
    let literal_ungrouped = set_names.claim(atn_cmds::UNGROUPED_SET);
    let mut scopes = std::collections::HashMap::new();
    let mut physical = 0usize;
    for (set, file) in archive.parsed.sets.iter().enumerate().filter(|(index, _)| Some(*index) != archive.hidden) {
        let wire_name = file.name.to_string_lossy();
        let logical_name = if wire_name == atn_cmds::UNGROUPED_SET {
            literal_ungrouped.as_str()
        } else {
            atn_cmds::logical_set_name((!wire_name.is_empty()).then_some(wire_name.as_str()))
        };
        let key = match manifest.as_ref().and_then(|manifest| manifest.groups.get(physical)) {
            Some(key) => key.clone(),
            None if wire_name.is_empty() => GroupKey::Implicit(physical),
            None => GroupKey::Folder(state.set_rows.len()),
        };
        let logical_name = manifest.as_ref().and_then(|manifest| manifest.group_name(&key)).unwrap_or(logical_name);
        let source = metadata.sets.get_mut(set).and_then(Option::take);
        let row = RowOrigin { archive: archive.clone(), set, logical_name: logical_name.to_string(), metadata: source.clone() };
        if let GroupKey::Folder(slot) = key {
            // The manifest may reorder physical records; canonical slots remain independent.
            if state.set_rows.len() <= slot {
                state.set_rows.resize(slot + 1, String::new());
                state.library.rows.resize(slot + 1, None);
            }
            state.set_rows[slot] = logical_name.to_string();
            state.library.rows[slot] = Some(row.clone());
            if let Some(source) = source {
                state.atn_set_rows.push(sidecar::SetRowMetadata { slot, row: logical_name.to_string(), source });
            }
        } else {
            state.library.other_rows.push((key.clone(), row.clone()));
        }
        state.library.order.push(key.clone());
        scopes.insert(wire_name.clone(), logical_name.to_string());
        physical += 1;
        for (index, original) in file.actions.iter().enumerate() {
            let (mut action, cohorts) = atn_cmds::library_action(file, original, if wire_name.is_empty() { "" } else { logical_name })?;
            total = total.checked_add(action.steps.len()).filter(|count| *count <= atn::MAX_TOTAL_ITEMS).ok_or("too many native steps in Actions library")?;
            if let Some(metadata) = metadata.actions.get_mut(set).and_then(|actions| actions.get_mut(index)).and_then(Option::take) {
                sidecar::apply_local(&mut action, metadata, identity_bytes.is_some())?;
            }
            action.set_slot = match key {
                GroupKey::Folder(slot) => Some(slot),
                _ => None,
            };
            action.set = match &key {
                GroupKey::Implicit(_) => None,
                _ => Some(logical_name.into()),
            };
            if manifest.is_some() {
                fingerprints.push(wire_fingerprint(file, original, &action, row.metadata.as_deref())?);
            }
            let baseline = Arc::new(action.clone());
            action.library_steps = vec![None; action.steps.len()];
            for (item, cohort) in cohorts.iter().enumerate() {
                for (part, position) in cohort.clone().enumerate() {
                    *action.library_steps.get_mut(position).ok_or("invalid native event projection")? = Some(StepOrigin { item, part });
                }
            }
            action.library_origin = Some(ActionOrigin { row: row.clone(), action: index, baseline, cohorts });
            state.list.push(action);
        }
    }
    for action in &mut state.list {
        sidecar::remap_scopes(action, &scopes);
    }
    if let Some(manifest) = manifest {
        manifest.restore(&mut state, &fingerprints)?;
    }
    Ok(state)
}

/// External import may normalize names, while retaining the original event cohorts and bytes.
pub(crate) fn bind_import(action: &mut Action, original: &Action) {
    // Keep the wire projection as the baseline. Normalized call names/scopes
    // must be encoded when their target headers changed during import.
    action.library_origin = original.library_origin.clone();
    action.library_steps = original.library_steps.clone();
    action.atn_metadata = original.atn_metadata.clone();
}

/// Attach one imported physical row to its destination slot without replacing an existing sidecar source.
pub(crate) fn import_row(state: &mut ActionState, imported: &ActionState, index: usize, slot: usize) {
    state.library.rows.resize(state.set_rows.len(), None);
    if let Some(row) = state.library.rows.get_mut(slot) {
        *row = projected_groups(imported).ok().and_then(|groups| groups.get(index).and_then(|(_, _, selection)| selection.raw.clone()));
    }
    if state.library.sidecar.is_none() {
        if imported.library.sidecar.is_some() || state.library.source.is_none() {
            state.library.source = imported.library.source.clone();
        }
        state.library.sidecar = imported.library.sidecar.clone();
    }
}

/// Attach complete physical state. Edits made while asynchronous startup ran survive, as do
/// every disk occurrence and empty row; name collisions do not discard either definition.
pub(crate) fn attach(live: &mut ActionState, mut disk: ActionState) {
    super::shift_action_calls(&mut disk.list, 0, live.list.len());
    let offset = live.set_rows.len();
    live.library.rows.resize(offset, None);
    live.library.rows.extend(disk.library.rows);
    if disk.library.sidecar.is_some() || live.library.sidecar.is_none() {
        live.library.source = disk.library.source;
        live.library.sidecar = disk.library.sidecar;
    }
    live.library.legacy = disk.library.legacy;
    for action in &mut disk.list {
        if let Some(slot) = action.set_slot.as_mut() {
            *slot += offset;
        }
    }
    let mut remap = Vec::new();
    for (key, row) in disk.library.other_rows {
        let original = key.clone();
        let mut key = key;
        if live.library.other_rows.iter().any(|(existing, _)| existing == &key) {
            let next = (0..=atn::MAX_SETS).find_map(|serial| {
                let candidate = match &key {
                    GroupKey::Legacy(name, _) => GroupKey::Legacy(name.clone(), serial),
                    GroupKey::Implicit(_) => GroupKey::Implicit(serial),
                    GroupKey::Folder(_) => return None,
                };
                (!live.library.other_rows.iter().any(|(existing, _)| existing == &candidate)).then_some(candidate)
            });
            if let Some(next) = next {
                key = next;
            }
        }
        remap.push((original, key.clone()));
        live.library.other_rows.push((key, row));
    }
    disk.library.order =
        disk.library.order.into_iter().map(|key| remap.iter().find(|(old, _)| old == &key).map(|(_, new)| new.clone()).unwrap_or(key)).collect();
    live.library.order.extend(disk.library.order.into_iter().map(|key| match key {
        GroupKey::Folder(slot) => GroupKey::Folder(slot + offset),
        other => other,
    }));
    live.list.extend(disk.list);
    super::identity::invalidate_duplicate_ids(&mut live.list);
    live.set_rows.extend(disk.set_rows);
    for name in disk.sets {
        if !live.sets.contains(&name) {
            live.sets.push(name);
        }
    }
    live.atn_set_rows.extend(disk.atn_set_rows.into_iter().map(|mut row| {
        row.slot += offset;
        row
    }));
    live.set_rows_revision = live.set_rows_revision.wrapping_add(1);
    live.rev = live.rev.saturating_add(1).max(1);
}

/// Unique names can be attached to the existing shortcut preferences; ambiguous headers stay raw.
pub(crate) fn function_keys(state: &ActionState) -> Vec<(String, String)> {
    header_commands(state)
        .into_iter()
        .filter_map(|(row, command)| {
            let action = &state.list[row];
            let origin = action.library_origin.as_ref()?;
            let original = original_action(origin).ok()?;
            (original.function_key > 0).then(|| (command, atn_cmds::function_key_name(original.function_key, original.shift, original.command)))
        })
        .collect()
}

pub(crate) fn header_commands(state: &ActionState) -> Vec<(usize, String)> {
    let mut counts = std::collections::HashMap::new();
    for action in &state.list {
        *counts.entry(action.name.as_str()).or_insert(0usize) += 1;
    }
    state
        .list
        .iter()
        .enumerate()
        .filter_map(|(row, action)| {
            if let Some(id) = action.action_id.as_deref().filter(|id| super::valid_action_id(id)) {
                Some((row, super::occurrence_key(id)))
            } else if counts.get(action.name.as_str()) == Some(&1) {
                Some((row, format!("{}{}", super::PLAY_SHORTCUT_PREFIX, action.name)))
            } else {
                None
            }
        })
        .collect()
}

struct ItemOrigin {
    row: RowOrigin,
    action: usize,
    item: usize,
    flags_changed: bool,
}

struct OutputAction {
    origin: Option<ActionOrigin>,
    items: Vec<Option<ItemOrigin>>,
}

/// Resolve a physical source record, rejecting stale row/action indices.
fn original_action(origin: &ActionOrigin) -> Result<&AtnAction> {
    origin.row.file()?.actions.get(origin.action).ok_or_else(|| "missing original action".into())
}

/// Rebuild edited headers and steps while retaining provenance for unchanged event spans.
/// Function keys bind only unique action names; opaque source headers remain available.
fn action_record(action: &Action, set: &str, keys: &[(String, String)], unique_name: bool) -> Result<(AtnAction, OutputAction)> {
    let origin = action.library_origin.as_ref();
    let original = origin.map(original_action).transpose()?;
    let mut output = original.cloned().unwrap_or_default();
    if origin.is_none_or(|origin| action.name != origin.baseline.name) {
        output.name = UnicodeString::new_nul(&action.name);
    }
    output.color = color_to_atn(&action.color).ok_or("invalid Actions library color")?;
    if unique_name || action.action_id.as_deref().is_some_and(super::valid_action_id) {
        let command = action
            .action_id
            .as_deref()
            .filter(|id| super::valid_action_id(id))
            .map(super::occurrence_key)
            .unwrap_or_else(|| format!("{}{}", super::PLAY_SHORTCUT_PREFIX, action.name));
        let key = keys
            .iter()
            .find(|(name, _)| name == &command)
            .or_else(|| {
                keys.iter().find(|(name, _)| {
                    (unique_name || action.legacy_shortcut_owner)
                        && (name == &action.name || name == &format!("{}{}", super::PLAY_SHORTCUT_PREFIX, action.name))
                })
            })
            .map(|(_, key)| key.as_str())
            .filter(|key| !key.is_empty());
        let previous = original.filter(|old| old.function_key > 0).map(|old| atn_cmds::function_key_name(old.function_key, old.shift, old.command));
        if key != previous.as_deref() {
            let (number, shift, command) = key.and_then(atn_cmds::parse_function_key).unwrap_or((0, false, false));
            output.function_key = number;
            output.shift = shift;
            output.command = command;
        }
    }
    output.items.clear();
    let mut origins = Vec::new();
    let mut position = 0usize;
    while position < action.steps.len() {
        let cohort = origin.and_then(|origin| {
            let step = action.library_steps.get(position)?.as_ref()?;
            if step.part != 0 {
                return None;
            }
            let range = origin.cohorts.get(step.item)?;
            if range.is_empty() {
                return None;
            }
            let end = position.checked_add(range.len())?;
            if end > action.steps.len() {
                return None;
            }
            let unchanged = range.clone().enumerate().all(|(part, old)| {
                let new = position + part;
                action.library_steps.get(new).and_then(Option::as_ref) == Some(&StepOrigin { item: step.item, part })
                    && action.steps.get(new) == origin.baseline.steps.get(old)
            });
            let first_flags = (action.disabled.contains(&position), action.dialogs.contains(&position));
            let uniform = (position..end).all(|new| (action.disabled.contains(&new), action.dialogs.contains(&new)) == first_flags);
            (unchanged && uniform).then_some((step.item, range.clone(), end))
        });
        if let (Some(origin), Some((item, range, end))) = (origin, cohort) {
            let mut record = original.ok_or("missing original action")?.items.get(item).cloned().ok_or("missing original event")?;
            let flags_changed = action.disabled.contains(&position) != origin.baseline.disabled.contains(&range.start)
                || action.dialogs.contains(&position) != origin.baseline.dialogs.contains(&range.start);
            if flags_changed {
                record.enabled = !action.disabled.contains(&position);
                record.with_dialog = action.dialogs.contains(&position);
            }
            output.items.push(record);
            origins.push(Some(ItemOrigin { row: origin.row.clone(), action: origin.action, item, flags_changed }));
            position = end;
        } else {
            let (id, params) = action.steps.get(position).ok_or("missing action step")?;
            for mut record in atn_cmds::library_items(id, params, set)? {
                record.enabled = !action.disabled.contains(&position);
                record.with_dialog = action.dialogs.contains(&position);
                output.items.push(record);
                origins.push(None);
            }
            position += 1;
        }
        if output.items.len() > atn::MAX_ITEMS {
            return Err("too many events in action".into());
        }
    }
    Ok((output, OutputAction { origin: origin.cloned(), items: origins }))
}

/// Copy a validated source span while enforcing the complete durable-library byte ceiling.
fn append(out: &mut Vec<u8>, bytes: &[u8], range: Range<usize>) -> Result<()> {
    let value = bytes.get(range).ok_or("invalid Actions library byte span")?;
    if out.len().checked_add(value.len()).is_none_or(|length| length > MAX_BYTES) {
        return Err("Actions library exceeds byte limit".into());
    }
    out.extend_from_slice(value);
    Ok(())
}

/// Reuse an original field only when unchanged; otherwise copy the canonical replacement span.
fn field(out: &mut Vec<u8>, canonical: &[u8], new: Range<usize>, source: Option<(&Archive, Range<usize>)>, unchanged: bool) -> Result<()> {
    if let Some((source, old)) = source.filter(|_| unchanged) { append(out, &source.bytes, old) } else { append(out, canonical, new) }
}

/// Canonical encoding supplies boundaries; unchanged original fields/events are then spliced back.
/// No second ATN grammar or persisted shadow registry is involved.
pub(crate) fn encode(state: &ActionState, keys: &[(String, String)]) -> Result<Vec<u8>> {
    let mut text_bytes = 0usize;
    for text in state
        .sets
        .iter()
        .chain(&state.set_rows)
        .map(String::as_str)
        .chain(state.list.iter().flat_map(|action| [&*action.name, &*action.description, &*action.icon]))
    {
        text_bytes = text_bytes
            .checked_add(text.encode_utf16().count().checked_mul(2).ok_or("Actions text length overflow")?)
            .filter(|bytes| *bytes <= MAX_BYTES)
            .ok_or("Actions text exceeds library byte limit")?;
    }
    let projected = projected_groups(state)?;
    let group_keys: Vec<_> = projected.iter().map(|(key, _, _)| key.clone()).collect();
    let groups: Vec<_> = projected.into_iter().map(|(_, name, selection)| (name, selection)).collect();
    let mut wire_names: Vec<String> = groups
        .iter()
        .map(|(name, selection)| match selection.raw.as_ref() {
            Some(row) if row.logical_name == *name => Ok(row.file()?.name.to_string_lossy()),
            _ => Ok(atn_cmds::wire_set_name(name).to_string()),
        })
        .collect::<Result<_>>()?;
    let mut available =
        atn_cmds::UniqueNames::new(std::iter::once(atn_cmds::UNGROUPED_SET.to_string()).chain(wire_names.iter().filter(|name| !name.is_empty()).cloned()));
    let literal_alias = available.claim(atn_cmds::UNGROUPED_SET);
    for ((name, selection), wire) in groups.iter().zip(&mut wire_names) {
        if selection.raw.as_ref().is_none_or(|row| row.logical_name != *name) || (*wire == atn_cmds::UNGROUPED_SET && literal_alias != *name) {
            *wire = atn_cmds::wire_set_name(name).to_string();
        }
    }
    let scopes = groups.iter().zip(&wire_names).map(|((name, _), wire)| (name.to_string(), wire.clone())).collect();
    let mut names = std::collections::HashMap::new();
    for action in &state.list {
        *names.entry(action.name.as_str()).or_insert(0usize) += 1;
    }
    let mut files = Vec::new();
    let mut records = Vec::new();
    for ((_, selection), wire) in groups.iter().zip(&wire_names) {
        let original = selection.raw.as_ref().map(RowOrigin::file).transpose()?;
        let mut file = original.cloned().unwrap_or_default();
        if original.is_none_or(|file| file.name.to_string_lossy() != *wire) {
            file.name = UnicodeString::new_nul(wire);
        }
        file.actions.clear();
        let mut actions = Vec::new();
        for action in &selection.actions {
            let mut wire_action = (**action).clone();
            sidecar::remap_scopes(&mut wire_action, &scopes);
            let (record, origin) = action_record(&wire_action, wire, keys, names.get(action.name.as_str()) == Some(&1))?;
            file.actions.push(record);
            actions.push(origin);
        }
        files.push(file);
        records.push(actions);
    }
    let selected: Vec<_> = groups
        .iter()
        .map(|(_, selection)| sidecar::Selection { actions: selection.actions.clone(), source: selection.source.clone(), raw: selection.raw.clone() })
        .collect();
    let wire_actions: Vec<_> = selected.iter().flat_map(|selection| selection.actions.iter().copied()).collect();
    let same_order = wire_actions.iter().map(|action| std::ptr::from_ref(*action)).eq(state.list.iter().map(std::ptr::from_ref));
    let mut expected_slot = 0usize;
    let canonical_rows = group_keys.iter().all(|key| match key {
        GroupKey::Legacy(_, _) => false,
        GroupKey::Implicit(_) => true,
        GroupKey::Folder(slot) => {
            let correct = *slot == expected_slot;
            expected_slot += 1;
            correct
        }
    });
    let needs_identity = group_keys.iter().zip(&wire_names).any(|(key, wire)| matches!(key, GroupKey::Folder(_)) && wire.is_empty())
        || !same_order
        || !canonical_rows
        || !state.sets.is_empty()
        || state.list.iter().any(|action| action.action_id.is_some() || action.legacy_shortcut_owner);
    // An unchanged pre-identity v16 archive remains byte-for-byte intact, including its lack of a carrier.
    let identities = if needs_identity {
        let mut fingerprints = Vec::with_capacity(wire_actions.len());
        for (file, (_, selection)) in files.iter().zip(&groups) {
            for (record, action) in file.actions.iter().zip(&selection.actions) {
                fingerprints.push(wire_fingerprint(file, record, action, selection.source.as_deref())?);
            }
        }
        Some(IdentityManifest::capture(state, group_keys, &wire_actions, &fingerprints)?.encode()?)
    } else {
        None
    };
    let outer = sidecar::append_library(&mut files, &selected, state.library.sidecar.clone(), state.library.legacy.as_ref(), identities.as_deref())?;
    let canonical = atn::write_sets(&files).map_err(|error| error.to_string())?;
    if canonical.len() > MAX_BYTES {
        return Err("Actions library exceeds byte limit".into());
    }
    let encoded = atn::parse_sets_with_spans(&canonical).map_err(|error| error.to_string())?;
    let mut out = Vec::with_capacity(canonical.len());
    append(&mut out, &canonical, 0..4)?;
    for (index, file) in files.iter().enumerate() {
        let span = encoded.spans.get(index).ok_or("missing output set span")?;
        if index >= groups.len() {
            if let (Some(outer), Some(source)) = (&outer, &state.library.source)
                && let Some(old) = source.hidden.and_then(|hidden| source.parsed.spans.get(hidden))
            {
                let descriptor = old
                    .actions
                    .first()
                    .and_then(|action| action.items.first())
                    .and_then(|item| item.descriptor.clone())
                    .ok_or("missing original sidecar span")?;
                append(&mut out, &source.bytes, old.range.start..descriptor.start)?;
                append(&mut out, outer, 0..outer.len())?;
                append(&mut out, &source.bytes, descriptor.end..old.range.end)?;
                continue;
            }
            // Newly created wrapper uses the canonical header and the exact patched descriptor.
            if let Some(outer) = &outer {
                let descriptor = span
                    .actions
                    .first()
                    .and_then(|action| action.items.first())
                    .and_then(|item| item.descriptor.clone())
                    .ok_or("missing output sidecar span")?;
                append(&mut out, &canonical, span.range.start..descriptor.start)?;
                append(&mut out, outer, 0..outer.len())?;
                append(&mut out, &canonical, descriptor.end..span.range.end)?;
            } else {
                append(&mut out, &canonical, span.range.clone())?;
            }
            continue;
        }
        let row = groups.get(index).and_then(|(_, selection)| selection.raw.as_ref());
        let old = row.map(RowOrigin::file).transpose()?;
        let raw = row.and_then(|row| row.archive.parsed.spans.get(row.set));
        field(
            &mut out,
            &canonical,
            span.name.clone(),
            row.zip(raw).map(|(row, raw)| (row.archive.as_ref(), raw.name.clone())),
            old.is_some_and(|old| old.name == file.name),
        )?;
        field(
            &mut out,
            &canonical,
            span.expanded.clone(),
            row.zip(raw).map(|(row, raw)| (row.archive.as_ref(), raw.expanded.clone())),
            old.is_some_and(|old| old.expanded == file.expanded),
        )?;
        field(
            &mut out,
            &canonical,
            span.count.clone(),
            row.zip(raw).map(|(row, raw)| (row.archive.as_ref(), raw.count.clone())),
            old.is_some_and(|old| old.actions.len() == file.actions.len()),
        )?;
        for (at, action) in file.actions.iter().enumerate() {
            let span = span.actions.get(at).ok_or("missing output action span")?;
            let record = records.get(index).and_then(|actions| actions.get(at)).ok_or("missing output action origin")?;
            let origin = record.origin.as_ref();
            let old = origin.map(original_action).transpose()?;
            let raw = origin.and_then(|origin| origin.row.archive.parsed.spans.get(origin.row.set)?.actions.get(origin.action));
            macro_rules! header {
                ($field:ident) => {
                    field(
                        &mut out,
                        &canonical,
                        span.$field.clone(),
                        origin.zip(raw).map(|(origin, raw)| (origin.row.archive.as_ref(), raw.$field.clone())),
                        old.is_some_and(|old| old.$field == action.$field),
                    )?;
                };
            }
            header!(function_key);
            header!(shift);
            header!(command);
            header!(color);
            header!(name);
            header!(expanded);
            field(
                &mut out,
                &canonical,
                span.count.clone(),
                origin.zip(raw).map(|(origin, raw)| (origin.row.archive.as_ref(), raw.count.clone())),
                old.is_some_and(|old| old.items.len() == action.items.len()),
            )?;
            for (item, span) in span.items.iter().enumerate() {
                if let Some(origin) = record.items.get(item).and_then(Option::as_ref) {
                    let raw = origin
                        .row
                        .archive
                        .parsed
                        .spans
                        .get(origin.row.set)
                        .and_then(|row| row.actions.get(origin.action))
                        .and_then(|action| action.items.get(origin.item))
                        .ok_or("missing original item span")?;
                    if origin.flags_changed {
                        let original =
                            origin.row.file()?.actions.get(origin.action).and_then(|action| action.items.get(origin.item)).ok_or("missing original item")?;
                        let current = action.items.get(item).ok_or("missing output item")?;
                        let same = [
                            original.expanded == current.expanded,
                            original.enabled == current.enabled,
                            original.with_dialog == current.with_dialog,
                            original.dialog_options == current.dialog_options,
                        ];
                        for (offset, same) in same.into_iter().enumerate() {
                            field(
                                &mut out,
                                &canonical,
                                span.flags.start + offset..span.flags.start + offset + 1,
                                Some((origin.row.archive.as_ref(), raw.flags.start + offset..raw.flags.start + offset + 1)),
                                same,
                            )?;
                        }
                        append(&mut out, &origin.row.archive.bytes, raw.flags.end..raw.range.end)?;
                    } else {
                        append(&mut out, &origin.row.archive.bytes, raw.range.clone())?;
                    }
                } else {
                    append(&mut out, &canonical, span.range.clone())?;
                }
            }
        }
    }
    Ok(out)
}

/// Project canonical folder identities onto native physical rows without a second definition model.
pub(crate) fn projected_groups(state: &ActionState) -> Result<Vec<(GroupKey, &str, sidecar::Selection<'_>)>> {
    let mut origin_ids = std::collections::HashMap::new();
    for action in &state.list {
        if let Some(origin) = &action.library_origin {
            let next = origin_ids.len();
            origin_ids.entry(origin.row.key()).or_insert(next);
        }
    }
    let retained_rows: std::collections::HashMap<_, _> = state
        .library
        .other_rows
        .iter()
        .filter_map(|(key, row)| match key {
            GroupKey::Legacy(name, _) => Some(((row.key(), Some(name.as_str())), key)),
            GroupKey::Implicit(_) => Some(((row.key(), None), key)),
            GroupKey::Folder(_) => None,
        })
        .collect();
    let action_key = |action: &Action| {
        if let Some(slot) = action.set_slot {
            return GroupKey::Folder(slot);
        }
        let retained = action
            .library_origin
            .as_ref()
            .and_then(|origin| retained_rows.get(&(origin.row.key(), action.set.as_deref().filter(|name| !name.is_empty()))).copied());
        if let Some(name) = action.set.as_ref().filter(|name| !name.is_empty()) {
            if let Some(GroupKey::Legacy(previous, serial)) = retained {
                if previous == name {
                    return GroupKey::Legacy(name.clone(), *serial);
                }
            }
            GroupKey::Legacy(
                name.clone(),
                action.library_origin.as_ref().and_then(|origin| origin_ids.get(&origin.row.key())).copied().unwrap_or(atn::MAX_SETS),
            )
        } else {
            retained.filter(|key| matches!(key, GroupKey::Implicit(_))).cloned().unwrap_or(GroupKey::Implicit(atn::MAX_SETS))
        }
    };
    let mut keys = state.library.order.clone();
    for slot in 0..state.set_rows.len() {
        if !keys.contains(&GroupKey::Folder(slot)) {
            keys.push(GroupKey::Folder(slot));
        }
    }
    for name in &state.sets {
        if !keys.iter().any(|key| matches!(key, GroupKey::Legacy(existing, _) if existing == name))
            && !state.list.iter().any(|action| action.set_slot.is_none() && action.set.as_ref() == Some(name))
        {
            keys.push(GroupKey::Legacy(name.clone(), atn::MAX_SETS));
        }
    }
    let mut seen: std::collections::HashSet<_> = keys.iter().cloned().collect();
    let mut buckets = std::collections::HashMap::<GroupKey, Vec<&Action>>::new();
    for action in &state.list {
        let key = action_key(action);
        if seen.insert(key.clone()) {
            keys.push(key.clone());
        }
        buckets.entry(key).or_default().push(action);
    }
    let mut out = Vec::new();
    for key in keys {
        let (name, mut raw) = match &key {
            GroupKey::Folder(slot) => {
                let Some(name) = state.set_rows.get(*slot) else { continue };
                (name.as_str(), state.library.rows.get(*slot).cloned().flatten())
            }
            GroupKey::Legacy(name, _) => {
                let Some(name) = state
                    .sets
                    .iter()
                    .chain(state.list.iter().filter(|action| action.set_slot.is_none()).filter_map(|action| action.set.as_ref()))
                    .find(|candidate| *candidate == name)
                else {
                    continue;
                };
                (name.as_str(), state.library.other_rows.iter().find(|(candidate, _)| candidate == &key).map(|(_, row)| row.clone()))
            }
            GroupKey::Implicit(_) => {
                (atn_cmds::UNGROUPED_SET, state.library.other_rows.iter().find(|(candidate, _)| candidate == &key).map(|(_, row)| row.clone()))
            }
        };
        let actions = buckets.remove(&key).unwrap_or_default();
        if raw.is_none() {
            raw = actions.first().and_then(|action| action.library_origin.as_ref().map(|origin| origin.row.clone()));
        }
        if actions.is_empty() && matches!(key, GroupKey::Implicit(_)) && raw.is_none() {
            continue;
        }
        let source = raw.as_ref().and_then(|row| row.metadata.clone()).or_else(|| actions.first().and_then(|action| sidecar::selection(state, action).source));
        out.push((key, name, sidecar::Selection { actions, source, raw }));
        if out.len() > atn::MAX_SETS {
            return Err("too many physical rows in local Actions library".into());
        }
    }
    Ok(out)
}

/// Insert exactly one folder occurrence and move every physical carrier with its canonical slot.
pub(crate) fn insert_folder(state: &mut ActionState, at: usize, name: &str, copy: Option<usize>) {
    state.library.rows.resize(state.set_rows.len(), None);
    let origin = copy.and_then(|slot| state.library.rows.get(slot).cloned().flatten());
    let metadata = copy.and_then(|slot| state.atn_set_rows.iter().find(|entry| entry.slot == slot).map(|entry| entry.source.clone()));
    for key in &mut state.library.order {
        if let GroupKey::Folder(slot) = key {
            if *slot >= at {
                *slot += 1;
            }
        }
    }
    for action in &mut state.list {
        if let Some(slot) = &mut action.set_slot {
            if *slot >= at {
                *slot += 1;
            }
        }
    }
    for row in &mut state.atn_set_rows {
        if row.slot >= at {
            row.slot += 1;
        }
    }
    state.set_rows.insert(at, name.into());
    state.library.rows.insert(at, origin);
    let order_at = copy
        .and_then(|source| {
            let source = source + usize::from(source >= at);
            state.library.order.iter().position(|key| *key == GroupKey::Folder(source)).map(|index| index + 1)
        })
        .or_else(|| state.library.order.iter().position(|key| matches!(key, GroupKey::Folder(slot) if *slot > at)))
        .unwrap_or(state.library.order.len());
    state.library.order.insert(order_at, GroupKey::Folder(at));
    if let Some(source) = metadata {
        state.atn_set_rows.push(sidecar::SetRowMetadata { slot: at, row: name.into(), source });
    }
    state.set_rows_revision = state.set_rows_revision.wrapping_add(1);
}

/// A caller supplies the old slot inventory, before changing names or rows.
pub(crate) fn retain_folders(state: &mut ActionState, keep: &[bool]) {
    state.library.rows.resize(state.set_rows.len(), None);
    let mut next = 0usize;
    let map: Vec<_> = keep
        .iter()
        .map(|keep| {
            if *keep {
                let slot = next;
                next += 1;
                Some(slot)
            } else {
                None
            }
        })
        .collect();
    if next == state.set_rows.len() {
        return;
    }
    let old = std::mem::take(&mut state.set_rows);
    state.set_rows = old.into_iter().enumerate().filter_map(|(slot, name)| map.get(slot).copied().flatten().map(|_| name)).collect();
    let old = std::mem::take(&mut state.library.rows);
    state.library.rows = old.into_iter().enumerate().filter_map(|(slot, row)| map.get(slot).copied().flatten().map(|_| row)).collect();
    state.library.order.retain_mut(|key| match key {
        GroupKey::Folder(slot) => match map.get(*slot).copied().flatten() {
            Some(new) => {
                *slot = new;
                true
            }
            None => false,
        },
        _ => true,
    });
    state.atn_set_rows.retain_mut(|entry| match map.get(entry.slot).copied().flatten() {
        Some(slot) => {
            entry.slot = slot;
            true
        }
        None => false,
    });
    for action in &mut state.list {
        action.set_slot = action.set_slot.and_then(|slot| map.get(slot).copied().flatten());
    }
    state.set_rows_revision = state.set_rows_revision.wrapping_add(1);
}

/// Whole portable replacement makes every archive row explicit, without adopting private IDs.
pub(crate) fn replacement_provenance(imported: &ActionState, names: &[String]) -> Result<(Library, Vec<sidecar::SetRowMetadata>)> {
    let groups = projected_groups(imported)?;
    if groups.len() != names.len() {
        return Err("replacement physical provenance does not match its converted sets".into());
    }
    let mut library = Library { source: imported.library.source.clone(), sidecar: imported.library.sidecar.clone(), ..Default::default() };
    let mut metadata = Vec::new();
    for (slot, ((_, _, selection), name)) in groups.into_iter().zip(names).enumerate() {
        library.rows.push(selection.raw);
        library.order.push(GroupKey::Folder(slot));
        if let Some(source) = selection.source {
            metadata.push(sidecar::SetRowMetadata { slot, row: name.clone(), source });
        }
    }
    Ok((library, metadata))
}

/// Caption-only copies retain each empty physical source as a private native occurrence.
pub(crate) fn copy_legacy_rows(state: &mut ActionState, source: &str, name: &str) {
    let rows: Vec<_> = state
        .set_rows
        .iter()
        .enumerate()
        .filter(|(_, caption)| caption.as_str() == source)
        .filter_map(|(slot, _)| state.library.rows.get(slot).cloned().flatten())
        .chain(
            state
                .library
                .other_rows
                .iter()
                .filter(|(key, _)| match key {
                    GroupKey::Legacy(name, _) => name == source,
                    GroupKey::Implicit(_) => source == atn_cmds::UNGROUPED_SET,
                    _ => false,
                })
                .map(|(_, row)| row.clone()),
        )
        .collect();
    for (serial, row) in rows.into_iter().enumerate() {
        let key = GroupKey::Legacy(name.into(), serial);
        state.library.other_rows.push((key.clone(), row));
        state.library.order.push(key);
    }
}

pub(crate) fn retire_legacy_group(state: &mut ActionState, name: &str) {
    let keep = |key: &GroupKey| !match key {
        GroupKey::Legacy(caption, _) => caption == name,
        GroupKey::Implicit(_) => name == atn_cmds::UNGROUPED_SET,
        _ => false,
    };
    state.library.other_rows.retain(|(key, _)| keep(key));
    state.library.order.retain(keep);
}

/// Hash the existing canonical ATN projection and exact opaque metadata, never a shadow step codec.
fn wire_fingerprint(file: &AtnFile, record: &AtnAction, action: &Action, metadata: Option<&sidecar::SetMetadata>) -> Result<String> {
    let mut singleton = file.clone();
    singleton.actions = vec![record.clone()];
    let record = atn::write(&singleton).map_err(|error| error.to_string())?;
    let entry = sidecar::identity_entry(action)?;
    let set = sidecar::identity_set(metadata)?;
    let mut digest = blake3::Hasher::new();
    for bytes in [&record, &entry, &set] {
        digest.update(&(bytes.len() as u64).to_le_bytes());
        digest.update(bytes);
    }
    Ok(digest.finalize().to_hex().to_string())
}
