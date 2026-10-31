//! Saved layer targeting: PSD resource 1024 names a raw record, while 1069 names `lyid`s.
//! Folder dividers occupy record slots but can never be selected document layers.

use std::collections::{HashMap, HashSet, hash_map::Entry};

use photocraft_doc::{Document, Layer, LayerContent, LayerId, SavedLayerSelection};
use photocraft_psd::resources::{ImageResource, ResourceData, ids};
use photocraft_psd::{LayerNode, LayerRecord, PsdFile, SectionType};

/// Pairs the reconstructed tree with imported layers while retaining original record slots.
/// Divider slots stay empty, and recursion uses the same depth limit as layer reconstruction.
fn record_targets(nodes: &[LayerNode], layers: &[Layer], targets: &mut [Option<LayerId>], depth: usize) {
    for (node, layer) in nodes.iter().zip(layers) {
        if let Some(target) = targets.get_mut(node.index()) {
            *target = Some(layer.id);
        }
        if depth < photocraft_doc::MAX_GROUP_DEPTH
            && let LayerContent::Group(group) = &layer.content
        {
            record_targets(node.children(), &group.children, targets, depth + 1);
        }
    }
}

/// Maps only unambiguous `lyid` values to imported layers, including zero as a valid ID.
/// A collision with any raw record, even a divider, invalidates that ID for selection.
fn unique_ids(file: &PsdFile, targets: &[Option<LayerId>]) -> HashMap<u32, Option<LayerId>> {
    let mut ids = HashMap::new();
    for (index, record) in file.layers().iter().enumerate() {
        let Some(id) = record.layer_id() else { continue };
        match ids.entry(id) {
            Entry::Vacant(entry) => {
                entry.insert(targets.get(index).copied().flatten());
            }
            Entry::Occupied(mut entry) => {
                entry.insert(None);
            }
        }
    }
    ids
}

/// Resolves the first resource 1024 through raw record slots, warning on unusable metadata.
/// Malformed, out-of-range and divider targets leave ordinary document fallback available.
fn target_layer(file: &PsdFile, targets: &[Option<LayerId>], warnings: &mut Vec<String>) -> Option<LayerId> {
    let resource = file.resource(ids::LAYER_STATE)?;
    match resource.parsed() {
        Some(Ok(ResourceData::LayerState(index))) => {
            let target = targets.get(usize::from(index)).copied().flatten();
            if target.is_none() {
                warnings.push("saved target layer (resource 1024) does not name an imported layer; ignored".into());
            }
            target
        }
        Some(Err(error)) => {
            warnings.push(format!("saved target layer (resource 1024) could not be read ({error}); ignored"));
            None
        }
        _ => None,
    }
}

/// Reads the first resource 1069 without conflating an explicit empty list with missing data.
/// Optional metadata errors become warnings so otherwise usable image content still opens.
fn selection_ids(file: &PsdFile, warnings: &mut Vec<String>) -> Option<Vec<u32>> {
    let resource = file.resource(ids::LAYER_SELECTION_IDS)?;
    match resource.parsed() {
        Some(Ok(ResourceData::LayerSelectionIds(ids))) => Some(ids),
        Some(Err(error)) => {
            warnings.push(format!("saved layer selection (resource 1069) could not be read ({error}); ignored"));
            None
        }
        _ => None,
    }
}

/// Uses a valid legacy target as a singleton, retaining absent metadata when none resolves.
fn singleton(active: Option<LayerId>) -> Option<SavedLayerSelection> {
    active.map(|id| SavedLayerSelection { active: Some(id), selected: vec![id] })
}

/// Applies saved targeting only to actual imported records, never to a merged-image stand-in.
/// Resource 1069 defines the selected set, including explicit deselection; 1024 supplies the
/// primary only when it belongs to that set. PhotoCraft recovers an inconsistent primary
/// with the last surviving selected ID, or falls back to 1024 when no selected IDs resolve.
pub(crate) fn import(file: &PsdFile, nodes: &[LayerNode], doc: &mut Document, warnings: &mut Vec<String>) {
    if file.layers().is_empty() || doc.layers.is_empty() || (file.resource(ids::LAYER_STATE).is_none() && file.resource(ids::LAYER_SELECTION_IDS).is_none()) {
        return;
    }
    let mut targets = vec![None; file.layers().len()];
    record_targets(nodes, &doc.layers, &mut targets, 0);
    let active = target_layer(file, &targets, warnings);
    let Some(ids) = selection_ids(file, warnings) else {
        doc.saved_layer_selection = singleton(active);
        return;
    };
    // Photoshop also keeps a cached 1024 when no layers are selected (empty-group.psd).
    if ids.is_empty() {
        doc.saved_layer_selection = Some(SavedLayerSelection { active: None, selected: Vec::new() });
        return;
    }
    let by_id = unique_ids(file, &targets);
    let mut seen = HashSet::new();
    let mut selected = Vec::new();
    let mut invalid = false;
    for id in ids {
        match by_id.get(&id).copied().flatten() {
            Some(layer) if seen.insert(layer) => selected.push(layer),
            Some(_) => {}
            None => invalid = true,
        }
    }
    if invalid {
        warnings.push("saved layer selection (resource 1069) contains missing, ambiguous or non-selectable layer ids; ignored those ids".into());
    }
    if selected.is_empty() {
        doc.saved_layer_selection = singleton(active);
        return;
    }
    let primary = active.filter(|id| seen.contains(id));
    if active.is_some() && primary.is_none() {
        warnings.push("saved target layer (resource 1024) is outside the saved selection; used the last selected layer".into());
    }
    doc.saved_layer_selection = Some(SavedLayerSelection { active: primary.or_else(|| selected.last().copied()), selected });
}

/// Regenerates targeting from the save snapshot and the records/ids that this export emitted.
/// Resource 1024 counts divider slots; resource 1069 uses finalized `lyid` assignments.
/// Explicit deselection emits only an empty 1069. Missing targeting and layerless output emit
/// neither resource; values outside PSD's 16-bit count/index limits are omitted with warnings.
pub(crate) fn export(doc: &Document, layer_ids: &HashMap<LayerId, u32>, records: &[LayerRecord], warnings: &mut Vec<String>) -> Vec<ImageResource> {
    let Some(saved) = doc.saved_layer_selection.clone() else { return Vec::new() };
    if records.is_empty() {
        return Vec::new();
    }
    let saved = saved.resolve(doc);
    let record_indices: HashMap<_, _> = records
        .iter()
        .enumerate()
        .filter(|(_, record)| record.section_type() != SectionType::BoundingDivider)
        .filter_map(|(index, record)| record.layer_id().map(|id| (id, index)))
        .collect();
    let selected: Vec<_> = saved.selected.iter().filter_map(|id| layer_ids.get(id).copied()).filter(|id| record_indices.contains_key(id)).collect();
    let Ok(count) = u16::try_from(selected.len()) else {
        warnings.push("saved layer selection exceeds PSD's 65535 selected-layer limit; selection metadata omitted".into());
        return Vec::new();
    };
    let mut data = count.to_be_bytes().to_vec();
    data.extend(selected.iter().flat_map(|id| id.to_be_bytes()));
    let mut resources = vec![ImageResource::new(ids::LAYER_SELECTION_IDS, data)];
    let index = saved.active.and_then(|id| layer_ids.get(&id)).and_then(|id| record_indices.get(id)).copied();
    if let Some(index) = index {
        match u16::try_from(index) {
            Ok(index) => resources.push(ImageResource::new(ids::LAYER_STATE, index.to_be_bytes().to_vec())),
            Err(_) => warnings.push("saved target layer index exceeds PSD's 65535 index limit; target metadata omitted".into()),
        }
    }
    resources
}
