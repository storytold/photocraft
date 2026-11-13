use super::descriptors::{desc, get_obj, jstr};
use super::events::{event_and_descriptor, event_for};
use super::fields::Mapped;
use super::{Action, AtnAction, AtnEvent, AtnFile, AtnItem, D, Descriptor, PHOTOSHOP_STEP, UnicodeString, Value, atn, json};
use crate::actions_cmds::{color_from_atn, color_to_atn};

// ------------------------------------------------------------------ import

/// Portable files admit the recorder's command surface and playback controls; unknown IDs stay opaque.
fn portable_native(id: &str) -> bool {
    crate::commands::find(id).is_none()
        || crate::actions_cmds::replayable(id)
        || matches!(id, super::STOP_MESSAGE | super::PHOTOSHOP_STEP | "actions.conditional")
}

/// A step with no PhotoCraft equivalent.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Unsupported {
    pub action: String,
    pub step: usize,
    pub event: String,
    pub name: String,
}

/// What a `.atn` file imports to.
#[derive(Clone, Debug, Default)]
pub struct Imported {
    pub set: String,
    pub actions: Vec<Action>,
    /// Real physical sets in file order, indexing `actions`; the recognized sidecar is absent.
    pub sets: Vec<ImportedSet>,
    /// Function keys by action position, avoiding duplicate-name association during load.
    pub action_keys: Vec<Option<String>>,
    /// Function keys the file assigns, as `(action name, key)`.
    pub function_keys: Vec<(String, String)>,
    pub unsupported: Vec<Unsupported>,
    pub warnings: Vec<String>,
    /// The encoding the file's 8-bit step names were read in (`"GBK"`, `"Shift_JIS"`,
    /// `"macintosh"`…); `None` when none needed one.
    pub encoding: Option<String>,
}

/// One real physical set and its positional range in the converted action list.
#[derive(Clone, Debug)]
pub struct ImportedSet {
    /// Logical display name after the reserved empty-wire category rule.
    pub name: String,
    /// Original archive identity, resolved before logical display aliases.
    pub(crate) wire_name: String,
    /// This physical set's actions before session collision renaming.
    pub actions: std::ops::Range<usize>,
    pub(crate) metadata: Option<std::sync::Arc<super::sidecar::SetMetadata>>,
}

/// Give imported actions globally unique playback names while keeping calls bound to their physical set.
/// Explicit archive scopes never fall back to another set; external scopes stay unrewritten.
/// The caller owns the uncommitted import, so a refusal never changes a live library.
/// `allow_external_calls` is false for a standalone CLI archive and true when importing into a session namespace.
pub fn normalize_import(
    imp: &mut Imported,
    reserved: impl IntoIterator<Item = String>,
    allow_external_calls: bool,
) -> std::result::Result<Vec<String>, String> {
    let mut set_for = vec![0usize; imp.actions.len()];
    for (index, set) in imp.sets.iter().enumerate() {
        set_for.get_mut(set.actions.clone()).ok_or_else(|| "invalid imported set range".to_string())?.fill(index);
    }
    if imp.action_keys.len() != imp.actions.len() {
        return Err("invalid imported shortcut positions".to_string());
    }
    let mut taken = UniqueNames::new(reserved);
    let mut names = Vec::with_capacity(imp.actions.len());
    let mut renamed: Vec<std::collections::HashMap<String, String>> = vec![Default::default(); imp.sets.len()];
    let mut global: std::collections::HashMap<String, Option<String>> = std::collections::HashMap::new();
    let mut added = Vec::with_capacity(imp.actions.len());
    for (index, mut a) in std::mem::take(&mut imp.actions).into_iter().enumerate() {
        let name = taken.claim(&a.name);
        let group = set_for.get(index).and_then(|set| renamed.get_mut(*set)).ok_or_else(|| "invalid imported action position".to_string())?;
        group.entry(a.name.clone()).or_insert_with(|| name.clone());
        global
            .entry(a.name.clone())
            .and_modify(|entry| {
                if entry.as_ref() != Some(&name) {
                    *entry = None;
                }
            })
            .or_insert(Some(name.clone()));
        a.name = name.clone();
        names.push(name);
        added.push(a);
    }
    // Resolve imported calls within their physical set first; never use another set's first
    // duplicate display name for metadata or shortcuts. Ambiguous cross-set calls fail before commit.
    for (index, a) in added.iter_mut().enumerate() {
        let local = set_for.get(index).and_then(|set| renamed.get(*set)).ok_or_else(|| "invalid imported action position".to_string())?;
        let normalize = |id: &str, params: &mut Value| -> std::result::Result<(), String> {
            let keys: &[&str] = match id {
                "actions.play" | "file.automate.batch" | "file.automate.createDroplet" => &["action"],
                "actions.conditional" => &["then", "else"],
                _ => &[],
            };
            // All branches resolve in the original wire namespace before publishing
            // their logical display aliases.
            let shared_scope = params.get("set").and_then(Value::as_str).map(str::to_owned);
            let mut shared_scopes = Vec::new();
            for key in keys {
                if let Some(target) = params.get(*key).and_then(Value::as_str) {
                    let branch_scope = format!("{key}Set");
                    let scope_key = if params.get(&branch_scope).is_some_and(Value::is_string) { branch_scope.as_str() } else { "set" };
                    let scope = params.get(&branch_scope).and_then(Value::as_str).or(shared_scope.as_deref());
                    let to = if let Some(scope) = scope {
                        scoped_call_target(&imp.sets, &renamed, scope, target, allow_external_calls)?.map(|(to, set)| (to.clone(), Some(set.clone())))
                    } else if let Some(to) = local.get(target) {
                        Some((to.clone(), None))
                    } else {
                        match global.get(target) {
                            Some(Some(to)) => Some((to.clone(), None)),
                            Some(None) => return Err("ambiguous cross-set action call; source set is required".to_string()),
                            None => None,
                        }
                    };
                    if let Some((to, scope)) = to {
                        params[*key] = json!(to);
                        if let Some(scope) = scope {
                            if scope_key == "set" {
                                shared_scopes.push((*key, scope));
                            } else {
                                params[scope_key] = json!(scope);
                            }
                        }
                    }
                }
            }
            // Repeated wire sets can give the two branches different logical aliases.
            // Keep a common set only when it still names both resolved destinations.
            if shared_scopes.windows(2).any(|pair| pair[0].1 != pair[1].1) {
                for (key, scope) in shared_scopes {
                    params[format!("{key}Set")] = json!(scope);
                }
            } else if let Some((_, scope)) = shared_scopes.pop() {
                params["set"] = json!(scope);
            }
            Ok(())
        };
        let mut result = Ok(());
        crate::actions_cmds::visit_calls(&mut a.steps, &mut |id, params, _| {
            if result.is_ok() {
                result = normalize(id, params);
            }
        });
        result?;
    }
    imp.actions = added;
    imp.function_keys = names.iter().zip(&imp.action_keys).filter_map(|(name, key)| key.as_ref().map(|key| (name.clone(), key.clone()))).collect();
    Ok(names)
}

/// Resolve wire names before logical aliases; repeated rows require one unique named target.
fn scoped_call_target<'a>(
    sets: &'a [ImportedSet],
    renamed: &'a [std::collections::HashMap<String, String>],
    scope: &str,
    target: &str,
    allow_external_calls: bool,
) -> std::result::Result<Option<(&'a String, &'a String)>, String> {
    // A literal wire name wins over an alias created for an empty-name set.
    let wire = sets.iter().any(|set| set.wire_name == scope);
    let mut matching = sets.iter().enumerate().filter(|(_, set)| if wire { set.wire_name == scope } else { set.name == scope }).peekable();
    if matching.peek().is_none() {
        return if allow_external_calls { Ok(None) } else { Err(format!("action call references missing set `{scope}`")) };
    }
    // Interleaved durable rows can share one logical/wire name. Resolve the named
    // target across those rows, while genuinely duplicate targets remain ambiguous.
    let mut targets = matching.filter_map(|(index, set)| renamed.get(index)?.get(target).map(|target| (target, &set.name)));
    let found = targets.next().ok_or_else(|| format!("action call target `{target}` is missing from set `{scope}`"))?;
    if targets.next().is_some() {
        return Err(format!("ambiguous action call target `{target}` in set `{scope}`"));
    }
    Ok(Some(found))
}

/// Retain an unsupported event's ID kind and descriptor for later export without executing it.
fn photoshop_step(item: &AtnItem, label: String) -> (String, Value) {
    let descriptor = item.descriptor.as_ref().map(|d| photocraft_paint::tile::b64_encode(&d.to_bytes()));
    let event_kind = match &item.event {
        AtnEvent::Code(_) => "code",
        AtnEvent::Name(_) => "name",
    };
    (PHOTOSHOP_STEP.into(), json!({"event": item.event.id(), "eventKind": event_kind, "name": label, "descriptor": descriptor}))
}

/// Integral floats become integers (`2.0` → `2`), as the commands' own docs and recordings
/// use, so an imported step reads and compares like a recorded one.
pub(super) fn normalise(v: &mut Value) {
    match v {
        Value::Number(n) => {
            if let Some(f) = n.as_f64()
                && f.fract() == 0.0
                && f.abs() < 9.0e15
                && n.as_i64().is_none()
            {
                *v = Value::from(f as i64);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(normalise),
        Value::Object(o) => o.values_mut().for_each(normalise),
        _ => {}
    }
}

/// Longest action or set name kept (Photoshop's own limit is 255). A file's names are lengths it
/// chooses, and each is copied into every action and every unsupported-step diagnostic.
pub(super) const MAX_NAME_CHARS: usize = 1024;

pub(super) fn clip_name(s: &str) -> String {
    s.chars().take(MAX_NAME_CHARS).collect()
}

/// The durable library retains all native bytes; external files accept the recorder's command surface.
#[derive(Clone, Copy)]
enum NativeInput {
    Library,
    Portable,
}

/// Project events under archive-wide step limits, retaining flags and optional original-event cohorts.
fn convert_action(
    file: &AtnFile,
    a: &AtnAction,
    set: &str,
    unsupported: &mut Vec<Unsupported>,
    total: &mut usize,
    mut cohorts: Option<&mut Vec<std::ops::Range<usize>>>,
    native_input: NativeInput,
) -> std::result::Result<Action, String> {
    let name = clip_name(&a.name.to_string_lossy());
    let mut action = Action::new(name.clone(), Vec::new());
    action.set = Some(clip_name(set));
    action.color = color_from_atn(a.color);
    for (i, item) in a.items.iter().enumerate() {
        let native = read_native(item, native_input)?;
        let is_native = native.is_some();
        let mapped = if let Some(step) = native {
            Mapped::Steps(vec![step])
        } else if super::sidecar::has_payload(item) {
            // An unrecognized metadata-bearing Stop stays visible/opaque, never silently loses its payload.
            Mapped::Unsupported { harmless: false }
        } else {
            let menu = item.descriptor.is_none().then(|| super::menu_item::from_event(&item.event)).flatten();
            match (menu, event_for(&item.event), &item.descriptor) {
                (Some(params), _, _) => Mapped::Steps(vec![(super::MENU_ITEM.into(), params)]),
                (_, Some(e), Some(d)) => (e.from)(d),
                (_, Some(e), None) => (e.from)(&Descriptor::default()),
                _ => Mapped::Unsupported { harmless: false },
            }
        };
        let (new_steps, enabled) = match mapped {
            Mapped::Steps(mut s) => {
                if !is_native {
                    super::steps_gallery::remember_event(item, &mut s);
                    super::steps_stack::remember_item(item, &mut s);
                    super::steps_video::remember(item, &mut s);
                }
                (s, item.enabled)
            }
            Mapped::Unsupported { harmless } => {
                let label = file.item_label(item);
                unsupported.push(Unsupported { action: name.clone(), step: i, event: item.event.id(), name: label.clone() });
                (vec![photoshop_step(item, label)], item.enabled && !harmless)
            }
        };
        let start = action.steps.len();
        for mut s in new_steps {
            *total = total.checked_add(1).filter(|total| *total <= atn::MAX_TOTAL_ITEMS).ok_or("too many converted steps in actions archive")?;
            if action.steps.len() >= atn::MAX_ITEMS {
                return Err("too many converted steps in one action".into());
            }
            if !is_native {
                normalise(&mut s.1);
            }
            let idx = action.steps.len();
            action.steps.push(s);
            if !enabled {
                action.disabled.insert(idx);
            }
            if item.with_dialog {
                action.dialogs.insert(idx);
            }
        }
        if let Some(cohorts) = cohorts.as_deref_mut() {
            cohorts.push(start..action.steps.len());
        }
    }
    Ok(action)
}

/// Strict library projection retains full display strings; raw origins retain exact UTF16 bytes.
pub(crate) fn library_action(file: &AtnFile, action: &AtnAction, set: &str) -> std::result::Result<(Action, Vec<std::ops::Range<usize>>), String> {
    let mut cohorts = Vec::new();
    let mut unsupported = Vec::new();
    let mut total = 0;
    let mut value = convert_action(file, action, set, &mut unsupported, &mut total, Some(&mut cohorts), NativeInput::Library)?;
    value.name = action.name.to_string_lossy();
    value.set = (!set.is_empty()).then(|| set.to_string());
    // The durable library reflects its wire flags. The external importer may deliberately
    // switch harmless unsupported steps off; bind_import treats that as an actual flag edit.
    for (item, cohort) in action.items.iter().zip(&cohorts) {
        for index in cohort.clone() {
            if item.enabled {
                value.disabled.remove(&index);
            } else {
                value.disabled.insert(index);
            }
        }
    }
    Ok((value, cohorts))
}

const NATIVE_EVENT: &str = "photocraftCommand";

/// Read one bounded, uniquely named UTF16 TEXT field from a native event descriptor.
fn native_text(descriptor: &Descriptor, key: &str) -> std::result::Result<String, String> {
    let mut values = descriptor.items.iter().filter(|(id, _)| id.is(key));
    let Some((_, D::Text(text))) = values.next() else { return Err(format!("native action step needs TEXT {key}")) };
    if values.next().is_some() || text.0.len() > crate::preset_store::MAX_ACTIONS_BYTES as usize {
        return Err(format!("duplicate or oversized native action {key}"));
    }
    let units = text.0.strip_suffix(&[0]).unwrap_or(&text.0);
    String::from_utf16(units).map_err(|_| format!("invalid UTF16 native action {key}"))
}

/// Decode only the reserved native event with its exact two-field schema; never dispatch its command.
fn read_native(item: &AtnItem, input: NativeInput) -> std::result::Result<Option<(String, Value)>, String> {
    if item.event != AtnEvent::name(NATIVE_EVENT) {
        return Ok(None);
    }
    let descriptor = item.descriptor.as_ref().ok_or("missing native action descriptor")?;
    if !descriptor.class_id.is("null") || descriptor.items.len() != 2 {
        return Err("unsupported native action descriptor".into());
    }
    let id = native_text(descriptor, "command")?;
    if id.is_empty() {
        return Err("empty native action command".into());
    }
    // Use the same command eligibility as recording, plus the explicit playback controls.
    // Configuration/install commands cannot be smuggled through a portable Actions file.
    // Unknown/future IDs remain preservable and fail normal dispatch without a side effect.
    // Re-importing after an upgrade checks any newly registered command against this policy.
    // Local persistence retains every native step without executing it.
    if matches!(input, NativeInput::Portable) && !portable_native(&id) {
        return Err(format!("native action command `{id}` is not permitted in a portable Actions file"));
    }
    let params = native_text(descriptor, "params")?;
    // serde's bounded recursion remains enabled; load never dispatches or resolves the command.
    let params = serde_json::from_str(&params).map_err(|error| format!("invalid native action JSON: {error}"))?;
    Ok(Some((id, params)))
}

/// Encode an unmapped native command and its complete JSON without losing unsupported options.
pub(crate) fn native_item(id: &str, params: &Value) -> std::result::Result<AtnItem, String> {
    let params = serde_json::to_string(params).map_err(|error| error.to_string())?;
    if id.is_empty() || id.len() > crate::preset_store::MAX_ACTIONS_BYTES as usize || params.len() > crate::preset_store::MAX_ACTIONS_BYTES as usize {
        return Err("empty or oversized native action step".into());
    }
    Ok(AtnItem {
        event: AtnEvent::name(NATIVE_EVENT),
        dictionary: id.as_bytes().to_vec(),
        descriptor: Some(desc("null").with("command", D::Text(UnicodeString::new(id))).with("params", D::Text(UnicodeString::new(&params)))),
        ..Default::default()
    })
}

/// Use a Photoshop event only when it preserves this exact native step shape/options.
pub(crate) fn library_items(id: &str, params: &Value, set: &str) -> std::result::Result<Vec<AtnItem>, String> {
    if id == PHOTOSHOP_STEP {
        return export_items(id, params, set).ok_or_else(|| "invalid kept Photoshop step".to_string());
    }
    if let Some(items) = export_items(id, params, set) {
        let file = AtnFile { name: UnicodeString::new_nul(set), ..Default::default() };
        let action = AtnAction { items: items.clone(), ..Default::default() };
        if let Ok((roundtrip, _)) = library_action(&file, &action, set)
            && roundtrip.steps == [(id.to_string(), params.clone())]
        {
            return Ok(items);
        }
    }
    Ok(vec![native_item(id, params)?])
}

/// Convert a portable `.atn` file without dispatching its steps. Reject malformed files and
/// registered native commands that recording forbids; keep unknown Photoshop events opaque.
pub fn import(bytes: &[u8]) -> std::result::Result<Imported, String> {
    import_with_encoding(bytes, None)
}

/// [`import`], reading 8-bit step names in `encoding` (a label such as `"shift_jis"` or
/// `"gbk"`) instead of working it out. An unknown label fails.
pub fn import_with_encoding(bytes: &[u8], encoding: Option<&str>) -> std::result::Result<Imported, String> {
    let files = atn::parse_sets_with_encoding(bytes, encoding).map_err(|error| error.to_string())?;
    // A legacy decoder may return a valid prefix with unread bytes; publishing it would lose the tail.
    if bytes.starts_with(&12u32.to_be_bytes()) && files.iter().any(|file| file.warnings.iter().any(|warning| warning.ends_with(" trailing bytes ignored"))) {
        return Err("trailing bytes after legacy action set".into());
    }
    let mut metadata = super::sidecar::decode(&files)?;
    let mut out = Imported::default();
    let mut total = 0usize;
    let reserved = std::iter::once(super::UNGROUPED_SET.to_string()).chain(
        files
            .iter()
            .enumerate()
            .filter(|(index, _)| metadata.hidden != Some(*index))
            .map(|(_, file)| clip_name(&file.name.to_string_lossy()))
            .filter(|name| !name.is_empty() && name != super::UNGROUPED_SET),
    );
    let mut set_names = UniqueNames::new(reserved);
    for (index, file) in files.iter().enumerate() {
        out.warnings.extend(file.warnings.iter().cloned());
        if out.encoding.is_none() {
            out.encoding = file.encoding.clone();
        }
        if metadata.hidden == Some(index) {
            continue;
        }
        let wire_name = file.name.to_string_lossy();
        let set = wire_name.clone();
        let set = if set.is_empty() {
            super::UNGROUPED_SET.to_string()
        } else if set == super::UNGROUPED_SET {
            let renamed = set_names.claim(&set);
            out.warnings.push(format!("set `{set}` imported as `{renamed}` to keep the ungrouped category distinct"));
            renamed
        } else {
            clip_name(&set)
        };
        if out.sets.is_empty() {
            out.set = set.clone();
        }
        let start = out.actions.len();
        for (at, action) in file.actions.iter().enumerate() {
            let mut converted = convert_action(file, action, &set, &mut out.unsupported, &mut total, None, NativeInput::Portable)?;
            if let Some(value) = metadata.actions.get_mut(index).and_then(|actions| actions.get_mut(at)).and_then(Option::take) {
                super::sidecar::apply(&mut converted, value);
            }
            let key = (action.function_key > 0).then(|| function_key_name(action.function_key, action.shift, action.command));
            if let Some(key) = &key {
                out.function_keys.push((converted.name.clone(), key.clone()));
            }
            out.action_keys.push(key);
            out.actions.push(converted);
        }
        let source = metadata.sets.get_mut(index).and_then(Option::take);
        out.sets.push(ImportedSet { name: set, wire_name, actions: start..out.actions.len(), metadata: source });
    }
    Ok(out)
}

// ------------------------------------------------------------------ export

/// A step Save Actions could not express as a Photoshop event.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Skipped {
    pub action: String,
    pub step: usize,
    pub id: String,
}

/// Project one kept or mapped Photoshop event, supplying the caller set for unscoped action references.
pub(super) fn export_item(id: &str, params: &Value, set: &str) -> Option<AtnItem> {
    if id == super::MENU_ITEM {
        return super::menu_item::export(params);
    }
    if id == PHOTOSHOP_STEP {
        let event = jstr(params, "event")?;
        let event = match params.get("eventKind") {
            Some(Value::String(kind)) if kind == "code" => AtnEvent::Code(event.as_bytes().try_into().ok()?),
            Some(Value::String(kind)) if kind == "name" => AtnEvent::name(event),
            Some(_) => return None,
            // Older PhotoCraft action data did not retain the event kind. Keep its previous
            // best-effort export behavior for compatibility.
            None => match event.as_bytes() {
                [a, b, c, d] => AtnEvent::Code([*a, *b, *c, *d]),
                _ => AtnEvent::name(event),
            },
        };
        let descriptor = match jstr(params, "descriptor") {
            Some(b64) => Some(Descriptor::from_bytes(&photocraft_paint::tile::b64_decode(b64)?).ok()?),
            None => None,
        };
        return Some(AtnItem { event, dictionary: jstr(params, "name").unwrap_or("").as_bytes().to_vec(), descriptor, ..Default::default() });
    }
    let mut params = params.clone();
    if (id == "actions.play" || id == "actions.conditional") && params.get("set").is_none_or(Value::is_null) {
        if params.is_null() {
            params = json!({});
        }
        params.as_object_mut()?.insert("set".into(), json!(set));
    }
    let (e, descriptor) = event_and_descriptor(id, &params)?;
    // A Filter Gallery effect loaded from its own event saves back as that event (#10592).
    if e.code == "GEfc"
        && let Some((event, name)) = super::steps_gallery::recorded_event(id, &params)
    {
        return Some(AtnItem { event, dictionary: name, descriptor: Some(descriptor), ..Default::default() });
    }
    let event = if id == "path.set" || e.code.is_empty() { AtnEvent::name(e.name) } else { AtnEvent::Code(e.code.as_bytes().try_into().ok()?) };
    let omit_descriptor = super::steps_stack::without_descriptor(id, &params) || super::steps_video::omitted(id, &params);
    let descriptor = (!omit_descriptor).then_some(descriptor);
    Some(AtnItem { event, dictionary: e.label.as_bytes().to_vec(), descriptor, ..Default::default() })
}

/// Photoshop applies only the rename when a Set mixes it with opacity or blend mode (#8304):
/// write each layer property separately, renaming last so named references still resolve. A
/// `.atn` reference is also one target, so a `layer.setProps` step that named `layers` exports
/// as one Set per layer, each referenced by Photoshop's layer id: an edit that accepted the
/// list must not replay on the active layer alone (#7246). The list is split into one-target
/// params before the step maps, so the keyed check that refuses the keys the `.atn` adapters
/// cannot write (#7196) sees a mapped one. A step unable to represent the whole list exactly
/// (an id beyond `Idnt`'s u32; a rename of several layers, which PhotoCraft itself refuses)
/// comes back as `None`, to refuse or skip the source step.
pub(super) fn export_items(id: &str, params: &Value, set: &str) -> Option<Vec<AtnItem>> {
    if id == "layer.setProps" {
        if let Some(Value::Array(ids)) = params.get("layers") {
            return layer_set_items(ids, params, set);
        }
        if params.get("layers").map(|v| !v.is_null()) == Some(true) {
            return None;
        }
    }
    let item = export_item(id, params, set)?;
    if id != "layer.setProps" {
        return Some(vec![item]);
    }
    item_property_split(item)
}

/// One Set per layer of an explicit `layers` list, by Photoshop's ever-present layer id.
fn layer_set_items(ids: &[Value], params: &Value, set: &str) -> Option<Vec<AtnItem>> {
    if ids.is_empty() || (params.get("name").is_some() && ids.len() > 1) {
        return None;
    }
    let mut items = Vec::new();
    for v in ids {
        let layer = v.as_u64().filter(|n| *n <= u32::MAX as u64)?;
        let mut p = params.clone();
        let obj = p.as_object_mut()?;
        obj.remove("layers");
        obj.insert("layer".into(), Value::Number(serde_json::Number::from(layer)));
        items.extend(item_property_split(export_item("layer.setProps", &p, set)?)?);
    }
    Some(items)
}

/// One item per property of one `Lyr ` target, renaming last so named references resolve
/// when the whole Set replays (#8304).
fn item_property_split(item: AtnItem) -> Option<Vec<AtnItem>> {
    let descriptor = item.descriptor.as_ref()?;
    let properties = get_obj(descriptor, "T   ")?;
    if properties.items.len() <= 1 {
        return Some(vec![item]);
    }
    let target = descriptor.get("null")?.clone();
    let mut items = Vec::with_capacity(properties.items.len());
    for (key, value) in properties.items.iter().filter(|(k, _)| !k.is("Nm  ")).chain(properties.items.iter().filter(|(k, _)| k.is("Nm  "))) {
        let mut property = properties.clone();
        property.items = vec![(key.clone(), value.clone())];
        let descriptor = desc("null").with("null", target.clone()).with("T   ", D::Descriptor(property));
        items.push(AtnItem { descriptor: Some(descriptor), ..item.clone() });
    }
    Some(items)
}

/// An action's key as a shortcut string: `F5`, with Photoshop's Shift and Command options
/// `Shift+F5`, `Cmd+F5` or `Cmd+Shift+F5` (#7646).
pub fn function_key_name(key: u16, shift: bool, command: bool) -> String {
    format!("{}{}F{key}", if command { "Cmd+" } else { "" }, if shift { "Shift+" } else { "" })
}

/// The `.atn` function key (1…15), Shift and Command of a shortcut string, or `None` for
/// anything else.
pub fn parse_function_key(key: &str) -> Option<(u16, bool, bool)> {
    let (mut shift, mut command) = (false, false);
    let mut rest = key;
    loop {
        if let Some(r) = rest.strip_prefix("Cmd+").filter(|_| !command) {
            (command, rest) = (true, r);
        } else if let Some(r) = rest.strip_prefix("Shift+").filter(|_| !shift) {
            (shift, rest) = (true, r);
        } else {
            break;
        }
    }
    let n = rest.strip_prefix('F')?.parse::<u16>().ok().filter(|n| (1..=15).contains(n))?;
    Some((n, shift, command))
}

/// The `.atn` bytes of `actions` as the set `set`. `function_keys` gives each action's key
/// (`F1`…`F15`, with `Shift+` and `Cmd+` as [`function_key_name`] writes them). PhotoCraft-only steps
/// use the reserved native event if portable import permits them, otherwise export fails.
/// With `skip_unsupported`, native steps are omitted and reported.
pub fn export(
    set: &str,
    actions: &[&Action],
    function_keys: &[(String, String)],
    skip_unsupported: bool,
) -> std::result::Result<(Vec<u8>, Vec<Skipped>), String> {
    if actions.len() > atn::MAX_TOTAL_ACTIONS {
        return Err("too many actions in actions archive".into());
    }
    let selection = super::sidecar::Selection { actions: actions.to_vec(), source: super::sidecar::source_for(actions)?, raw: None };
    export_groups(&[(set, selection)], function_keys, skip_unsupported)
}

/// Validate selected archive counts, export each group and append its positional metadata carrier.
pub(super) fn export_groups(
    groups: &[(&str, super::sidecar::Selection<'_>)],
    function_keys: &[(String, String)],
    skip_unsupported: bool,
) -> std::result::Result<(Vec<u8>, Vec<Skipped>), String> {
    if groups.is_empty() || groups.len() > atn::MAX_SETS {
        return Err("invalid number of sets in actions archive".into());
    }
    let mut actions = 0usize;
    let mut steps = 0usize;
    for (_, group) in groups {
        if group.actions.len() > atn::MAX_ACTIONS {
            return Err("too many actions in one set".into());
        }
        actions = actions.checked_add(group.actions.len()).filter(|count| *count <= atn::MAX_TOTAL_ACTIONS).ok_or("too many actions in actions archive")?;
        for action in &group.actions {
            if action.steps.len() > atn::MAX_ITEMS {
                return Err("too many steps in one action".into());
            }
            steps = steps.checked_add(action.steps.len()).filter(|count| *count <= atn::MAX_TOTAL_ITEMS).ok_or("too many steps in actions archive")?;
        }
    }
    let mut skipped = Vec::new();
    let mut files = Vec::with_capacity(groups.len().min(atn::MAX_SETS));
    let mut written = 0usize;
    for (set, selection) in groups {
        files.push(export_set(set, &selection.actions, function_keys, skip_unsupported, &mut skipped, &mut written)?);
    }
    let selected: Vec<_> = groups
        .iter()
        .map(|(_, selection)| super::sidecar::Selection { actions: selection.actions.clone(), source: selection.source.clone(), raw: selection.raw.clone() })
        .collect();
    super::sidecar::append(&mut files, &selected)?;
    let bytes = atn::write_sets(&files).map_err(|error| error.to_string())?;
    Ok((bytes, skipped))
}

/// Preserve step order and flags; report explicit omissions or retain unmapped commands as native events.
fn export_set(
    set: &str,
    actions: &[&Action],
    function_keys: &[(String, String)],
    skip_unsupported: bool,
    skipped: &mut Vec<Skipped>,
    written: &mut usize,
) -> std::result::Result<AtnFile, String> {
    let mut file =
        AtnFile { name: UnicodeString::new_nul(super::wire_set_name(set)), expanded: true, actions: Vec::new(), warnings: Vec::new(), ..Default::default() };
    for a in actions {
        let color = color_to_atn(&a.color).ok_or_else(|| format!("action `{}` has an invalid colour", a.name))?;
        let mut out = AtnAction { name: UnicodeString::new_nul(&a.name), expanded: false, color, ..Default::default() };
        if let Some((n, shift, command)) = function_keys.iter().find(|(n, _)| n == &a.name).and_then(|(_, key)| parse_function_key(key)) {
            (out.function_key, out.shift, out.command) = (n, shift, command);
        }
        for (i, (id, params)) in a.steps.iter().enumerate() {
            // Portable export keeps the existing Photoshop mapping contract (including its
            // explicit defaults). The durable library uses the stricter library_items path.
            let items = match export_items(id, params, set) {
                Some(items) => items,
                None if id == PHOTOSHOP_STEP => {
                    if skip_unsupported {
                        skipped.push(Skipped { action: a.name.clone(), step: i, id: id.clone() });
                        continue;
                    }
                    return Err("invalid kept Photoshop step".into());
                }
                None if !portable_native(id) => {
                    if skip_unsupported {
                        skipped.push(Skipped { action: a.name.clone(), step: i, id: id.clone() });
                        continue;
                    }
                    return Err(format!("native action command `{id}` is not permitted in a portable Actions file"));
                }
                None => vec![native_item(id, params)?],
            };
            // A kept Photoshop step can itself wrap a native event. Validate the
            // returned items too, before publishing any part of this source step.
            if let Some(error) = items.iter().find_map(|item| read_native(item, NativeInput::Portable).err()) {
                if skip_unsupported {
                    skipped.push(Skipped { action: a.name.clone(), step: i, id: id.clone() });
                    continue;
                }
                return Err(error);
            }
            if skip_unsupported && items.iter().any(|item| item.event == AtnEvent::name(NATIVE_EVENT)) {
                skipped.push(Skipped { action: a.name.clone(), step: i, id: id.clone() });
                continue;
            }
            for mut item in items {
                *written = written.checked_add(1).filter(|count| *count <= atn::MAX_TOTAL_ITEMS).ok_or("too many output steps in actions archive")?;
                item.enabled = a.step_enabled(i);
                item.with_dialog = a.dialogs.contains(&i);
                item.dialog_options = if item.with_dialog { 2 } else { 0 };
                out.items.push(item);
            }
        }
        file.actions.push(out);
    }
    Ok(file)
}

// ------------------------------------------------------------------ commands

/// Assigns names not already used, as `name`, `name 2`, `name 3`… A plain linear scan of the
/// taken list per candidate is cubic when an action file repeats one base name many times, so
/// membership uses a set and the next suffix to try is cached per base name.
pub(crate) struct UniqueNames {
    taken: std::collections::HashSet<String>,
    next_suffix: std::collections::HashMap<String, u32>,
}

impl UniqueNames {
    pub(crate) fn new<I: IntoIterator<Item = String>>(existing: I) -> Self {
        UniqueNames { taken: existing.into_iter().collect(), next_suffix: std::collections::HashMap::new() }
    }

    /// Reserves and returns a unique name derived from `name`, recording it as taken.
    pub(crate) fn claim(&mut self, name: &str) -> String {
        if self.taken.insert(name.to_string()) {
            return name.to_string();
        }
        let mut i = self.next_suffix.get(name).copied().unwrap_or(2);
        loop {
            let candidate = format!("{name} {i}");
            i += 1;
            if self.taken.insert(candidate.clone()) {
                self.next_suffix.insert(name.to_string(), i);
                return candidate;
            }
        }
    }
}
