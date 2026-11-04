//! The sole durable Actions library. Runtime origins refer to physical ATN records, never names.
use std::{ops::Range, sync::Arc};

use photocraft_psd::{
    atn::{self, AtnAction, AtnFile, SpannedAtnArchive},
    descriptor::{Descriptor, UnicodeString, Value as D},
};

use super::{Action, ActionState, color_to_atn};
use crate::atn_cmds::{self, sidecar};

type Result<T> = std::result::Result<T, String>;
const MAX_BYTES: usize = crate::preset_store::MAX_ACTIONS_BYTES as usize;

#[derive(Debug)]
struct Archive {
    bytes: Arc<[u8]>,
    parsed: SpannedAtnArchive,
    hidden: Option<usize>,
}

impl PartialEq for Archive {
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
}

impl RowOrigin {
    pub(crate) fn key(&self) -> (usize, usize) {
        (Arc::as_ptr(&self.archive) as usize, self.set)
    }
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
    pub(crate) fn new(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_BYTES {
            return Err("legacy Actions library exceeds byte limit".into());
        }
        Ok(Self { bytes: bytes.len(), digest: blake3::hash(bytes).to_hex().to_string() })
    }
    pub(crate) fn matches(&self, bytes: &[u8]) -> bool {
        bytes.len() == self.bytes && blake3::hash(bytes).to_hex().as_str() == self.digest
    }
    pub(crate) fn descriptor(&self) -> Result<Descriptor> {
        Ok(Descriptor::new("null")
            .with("version", D::Integer(1))
            .with("bytes", D::Integer(i32::try_from(self.bytes).map_err(|_| "legacy length overflow")?))
            .with("digest", D::Text(UnicodeString::new(&self.digest))))
    }
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
    sidecar: Option<Arc<sidecar::Source>>,
    pub(crate) legacy: Option<LegacyOrigin>,
}

pub(crate) struct RowIndex<'a> {
    first: std::collections::HashMap<&'a str, RowOrigin>,
    exact: std::collections::HashMap<(&'a str, (usize, usize)), RowOrigin>,
}
impl RowIndex<'_> {
    pub(crate) fn source(&self, action: &Action) -> Option<RowOrigin> {
        let name = atn_cmds::logical_set_name(action.set.as_deref());
        action.library_origin.as_ref().and_then(|origin| self.exact.get(&(name, origin.row.key()))).or_else(|| self.first.get(name)).cloned()
    }
}

impl Library {
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
        self.row_index(names).source(action)
    }
}

/// A strict complete load; failure returns no partial library and executes no commands.
pub(crate) fn decode(bytes: &[u8]) -> Result<ActionState> {
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
    let archive = Arc::new(Archive { bytes: bytes.into(), parsed, hidden: metadata.hidden });
    let mut state = ActionState::default();
    state.library.source = Some(archive.clone());
    state.library.sidecar = metadata.source;
    state.library.legacy = legacy;
    let mut total = 0usize;
    for (set, file) in archive.parsed.sets.iter().enumerate().filter(|(index, _)| Some(*index) != archive.hidden) {
        let wire_name = file.name.to_string_lossy();
        let row = RowOrigin { archive: archive.clone(), set };
        let slot = state.sets.len();
        state.sets.push(atn_cmds::logical_set_name((!wire_name.is_empty()).then_some(wire_name.as_str())).to_string());
        state.library.rows.push(Some(row.clone()));
        if let Some(source) = metadata.sets.get_mut(set).and_then(Option::take) {
            state.atn_set_rows.push(sidecar::SetRowMetadata { slot, row: state.sets.last().cloned().ok_or("missing loaded row")?, source });
        }
        for (index, original) in file.actions.iter().enumerate() {
            let (mut action, cohorts) = atn_cmds::library_action(file, original, &wire_name)?;
            total = total.checked_add(action.steps.len()).filter(|count| *count <= atn::MAX_TOTAL_ITEMS).ok_or("too many native steps in Actions library")?;
            if let Some(metadata) = metadata.actions.get_mut(set).and_then(|actions| actions.get_mut(index)).and_then(Option::take) {
                sidecar::apply(&mut action, metadata);
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
    Ok(state)
}

/// External import may normalize names, while retaining the original event cohorts and bytes.
pub(crate) fn bind_import(action: &mut Action, original: &Action) {
    let Some(mut origin) = original.library_origin.clone() else { return };
    let mut baseline = original.library_origin.as_ref().map(|origin| (*origin.baseline).clone()).unwrap_or_default();
    baseline.steps = action.steps.clone();
    origin.baseline = Arc::new(baseline);
    action.library_origin = Some(origin);
    action.library_steps = original.library_steps.clone();
    action.atn_metadata = original.atn_metadata.clone();
}

pub(crate) fn import_row(state: &mut ActionState, imported: &ActionState, index: usize, slot: usize) {
    state.library.rows.resize(state.sets.len(), None);
    if let Some(row) = state.library.rows.get_mut(slot) {
        *row = imported.library.rows.get(index).cloned().flatten();
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
pub(crate) fn attach(live: &mut ActionState, disk: ActionState) {
    let offset = live.sets.len();
    live.library.rows.resize(offset, None);
    live.library.rows.extend(disk.library.rows);
    if disk.library.sidecar.is_some() || live.library.sidecar.is_none() {
        live.library.source = disk.library.source;
        live.library.sidecar = disk.library.sidecar;
    }
    live.library.legacy = disk.library.legacy;
    live.list.extend(disk.list);
    live.sets.extend(disk.sets);
    live.atn_set_rows.extend(disk.atn_set_rows.into_iter().map(|mut row| {
        row.slot += offset;
        row
    }));
    live.rev = live.rev.saturating_add(1).max(1);
}

/// Unique names can be attached to the existing shortcut preferences; ambiguous headers stay raw.
pub(crate) fn function_keys(state: &ActionState) -> Vec<(String, String)> {
    let mut counts = std::collections::HashMap::new();
    for action in &state.list {
        *counts.entry(action.name.as_str()).or_insert(0usize) += 1;
    }
    state
        .list
        .iter()
        .filter(|action| counts.get(action.name.as_str()) == Some(&1))
        .filter_map(|action| {
            let origin = action.library_origin.as_ref()?;
            let header = origin.row.file().ok()?.actions.get(origin.action)?;
            (header.function_key != 0).then(|| (action.name.clone(), atn_cmds::function_key_name(header.function_key, header.shift, header.command)))
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

fn original_action(origin: &ActionOrigin) -> Result<&AtnAction> {
    origin.row.file()?.actions.get(origin.action).ok_or_else(|| "missing original action".into())
}

fn action_record(action: &Action, set: &str, keys: &[(String, String)], unique_name: bool) -> Result<(AtnAction, OutputAction)> {
    let origin = action.library_origin.as_ref();
    let original = origin.map(original_action).transpose()?;
    let mut output = original.cloned().unwrap_or_default();
    if origin.is_none_or(|origin| action.name != origin.baseline.name) {
        output.name = UnicodeString::new_nul(&action.name);
    }
    output.color = color_to_atn(&action.color).ok_or("invalid Actions library color")?;
    if unique_name {
        let key = keys.iter().find(|(name, _)| name == &action.name).map(|(_, key)| key.as_str()).filter(|key| !key.is_empty());
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

fn append(out: &mut Vec<u8>, bytes: &[u8], range: Range<usize>) -> Result<()> {
    let value = bytes.get(range).ok_or("invalid Actions library byte span")?;
    if out.len().checked_add(value.len()).is_none_or(|length| length > MAX_BYTES) {
        return Err("Actions library exceeds byte limit".into());
    }
    out.extend_from_slice(value);
    Ok(())
}

fn field(out: &mut Vec<u8>, canonical: &[u8], new: Range<usize>, source: Option<(&Archive, Range<usize>)>, unchanged: bool) -> Result<()> {
    if let Some((source, old)) = source.filter(|_| unchanged) { append(out, &source.bytes, old) } else { append(out, canonical, new) }
}

/// Canonical encoding supplies boundaries; unchanged original fields/events are then spliced back.
/// No second ATN grammar or persisted shadow registry is involved.
pub(crate) fn encode(state: &ActionState, keys: &[(String, String)]) -> Result<Vec<u8>> {
    let mut text_bytes = 0usize;
    for text in state.sets.iter().map(String::as_str).chain(state.list.iter().flat_map(|action| [&*action.name, &*action.description, &*action.icon])) {
        text_bytes = text_bytes
            .checked_add(text.encode_utf16().count().checked_mul(2).ok_or("Actions text length overflow")?)
            .filter(|bytes| *bytes <= MAX_BYTES)
            .ok_or("Actions text exceeds library byte limit")?;
    }
    let groups = sidecar::groups(state, None)?;
    let mut names = std::collections::HashMap::new();
    for action in &state.list {
        *names.entry(action.name.as_str()).or_insert(0usize) += 1;
    }
    let mut files = Vec::new();
    let mut records = Vec::new();
    for (name, selection) in &groups {
        let original = selection.raw.as_ref().map(RowOrigin::file).transpose()?;
        let mut file = original.cloned().unwrap_or_default();
        let wire = atn_cmds::wire_set_name(name);
        if original.is_none_or(|old| {
            let source_name = old.name.to_string_lossy();
            atn_cmds::logical_set_name((!source_name.is_empty()).then_some(source_name.as_str())) != *name
        }) {
            file.name = UnicodeString::new_nul(wire);
        }
        file.actions.clear();
        let mut actions = Vec::new();
        for action in &selection.actions {
            let (record, origin) = action_record(action, name, keys, names.get(action.name.as_str()) == Some(&1))?;
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
    let outer = sidecar::append_library(&mut files, &selected, state.library.sidecar.clone(), state.library.legacy.as_ref())?;
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
            if let (Some(outer), Some(source)) = (&outer, &state.library.source) {
                if let Some(old) = source.hidden.and_then(|hidden| source.parsed.spans.get(hidden)) {
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
