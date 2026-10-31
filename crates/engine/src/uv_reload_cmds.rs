//! Reimport geometry without reopening, replacing or modifying texture documents.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use photocraft_doc::{DocId, Document, mask_paint::EffectKind};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

/// Prepare derived mask-cache updates for a proposed UV topology/binding set. The documents are
/// cloned first and the caller publishes them only after every bounded recomposition succeeds.
pub(crate) fn prepare_bound_mask_cache_updates(
    scene: &photocraft_uv::Scene,
    topology: &photocraft_uv::topology::Topology,
    bindings: &[Option<DocId>],
    normal_bindings: &[Option<DocId>],
    map_bindings: &BTreeMap<photocraft_uv::preview::Channel, Vec<Option<DocId>>>,
    docs: &[crate::DocState],
) -> Result<Vec<(DocId, Document)>> {
    let mut materials_by_doc = HashMap::<DocId, std::collections::BTreeSet<usize>>::new();
    for channel in photocraft_uv::preview::Channel::MAPS {
        let slots = match channel {
            photocraft_uv::preview::Channel::Normal => normal_bindings,
            photocraft_uv::preview::Channel::Color | photocraft_uv::preview::Channel::Material => bindings,
            _ => map_bindings.get(&channel).map_or(&[] as &[Option<DocId>], Vec::as_slice),
        };
        for (material, id) in slots.iter().enumerate() {
            if let Some(id) = id {
                materials_by_doc.entry(*id).or_default().insert(material);
            }
        }
    }

    let mut updates = Vec::new();
    for state in docs {
        let Some(materials) = materials_by_doc.get(&state.doc.id) else { continue };
        let mask_ids: Vec<_> =
            state.doc.walk().iter().filter_map(|(_, _, layer)| layer.mask.as_ref().filter(|mask| mask.paint.is_some()).map(|_| layer.id)).collect();
        if mask_ids.is_empty() {
            continue;
        }
        let mut updated = (*state.doc).clone();
        let material_ids: Vec<_> = materials.iter().copied().collect();
        refresh_bound_mask_cache_for_document(&mut updated, scene, topology, &material_ids)?;
        updates.push((state.doc.id, updated));
    }
    Ok(updates)
}

pub(crate) fn refresh_bound_mask_cache_for_document(
    document: &mut Document,
    scene: &photocraft_uv::Scene,
    topology: &photocraft_uv::topology::Topology,
    materials: &[usize],
) -> Result<()> {
    let mask_ids: Vec<_> =
        document.walk().iter().filter_map(|(_, _, layer)| layer.mask.as_ref().filter(|mask| mask.paint.is_some()).map(|_| layer.id)).collect();
    if mask_ids.is_empty() {
        return Ok(());
    }
    let canvas = document.bounds();
    let mut shell_ids = None;
    for id in mask_ids {
        let Some(mask) = document.layer_mut(id).and_then(|layer| layer.mask.as_mut()) else { continue };
        let has_blur = mask
            .paint
            .as_ref()
            .is_some_and(|stack| stack.layers.iter().any(|effect| effect.visible && effect.opacity > 0.0 && matches!(effect.kind, EffectKind::Blur { .. })));
        if has_blur {
            if shell_ids.is_none() {
                shell_ids = Some(
                    topology
                        .shell_id_map_for_materials(scene, canvas.width() as u32, canvas.height() as u32, materials, [0, 0])
                        .map_err(|error| EngineError::Other(format!("Could not prepare UV shell ownership for mask Blur: {error}")))?,
                );
            }
            let Some(ids) = shell_ids.as_deref() else {
                return Err(EngineError::Other("Could not prepare UV shell ownership for mask Blur".into()));
            };
            let shell_map = photocraft_compose::mask_paint::ShellIdMap::new(canvas, ids)
                .ok_or_else(|| EngineError::Other("Could not prepare UV shell ownership for mask Blur".into()))?;
            if !photocraft_compose::mask_paint::refresh_in_canvas_with_shell_ids(mask, canvas, shell_map) {
                return Err(EngineError::Other("Mask Blur exceeded the bounded UV shell compositor region".into()));
            }
        } else {
            photocraft_compose::mask_paint::refresh_in_canvas(mask, canvas);
        }
    }
    Ok(())
}

pub(crate) fn apply_bound_mask_cache_updates(s: &mut Session, updates: Vec<(DocId, Document)>) {
    let mut published = Vec::new();
    {
        let docs = s.uv_documents_mut();
        for (id, document) in updates {
            if let Some(state) = docs.iter_mut().find(|state| state.doc.id == id) {
                state.doc = Arc::new(document);
                // The cache refresh changes the rendered pixels without making the document
                // content dirty or adding an undo step. Move the UI revision and its view-only
                // portion together so texture-export revisions stay unchanged.
                let was_clean = state.saved_revision == state.revision;
                state.revision += 1;
                state.view_revision = state.view_revision.wrapping_add(1);
                if was_clean {
                    state.saved_revision = state.revision;
                }
                state.last_damage = None;
                published.push(Arc::clone(&state.doc));
            }
        }
    }
    for document in published {
        if let Some(context) = s.uv.mask_cache_context(&document) {
            context.remember(&document);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceStamp {
    size: u64,
    modified_ns: u64,
}
impl SourceStamp {
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn read(path: &str) -> Result<Option<Self>> {
        let metadata = std::fs::metadata(path).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
        let time = metadata.modified().map_err(|e| EngineError::Other(e.to_string()))?;
        let ns = time.duration_since(std::time::UNIX_EPOCH).map_err(|e| EngineError::Other(e.to_string()))?.as_nanos();
        Ok(Some(Self { size: metadata.len(), modified_ns: u64::try_from(ns).map_err(|_| EngineError::Other("Invalid source timestamp".into()))? }))
    }
    #[cfg(target_arch = "wasm32")]
    pub(super) fn read(_: &str) -> Result<Option<Self>> {
        Err(EngineError::Other("Source-file checking is available in native builds only".into()))
    }
}
fn enabled(s: &Session) -> std::result::Result<(), String> {
    if s.uv.scene.is_some() && !s.uv.source.is_empty() { Ok(()) } else { Err("Import a model with a source file first".into()) }
}
fn check(s: &mut Session, _: &Value) -> Result<Value> {
    match SourceStamp::read(&s.uv.source) {
        Ok(now) => {
            let changed = s.uv.source_stamp.is_some() && now != s.uv.source_stamp;
            if s.uv.source_stamp.is_none() {
                s.uv.source_stamp = now;
            }
            Ok(json!({"changed":changed,"tracked":s.uv.source_stamp.is_some(),"available":true}))
        }
        Err(error) => Ok(json!({"changed":false,"available":false,"error":error.to_string()})),
    }
}
fn reload(s: &mut Session, p: &Value) -> Result<Value> {
    let old = s.uv.scene.as_ref().ok_or_else(|| EngineError::Other("Import a model first".into()))?;
    let uv_set = match p.get("uvSet") {
        None => s.uv.uv_set,
        Some(value) => {
            value.as_u64().and_then(|v| usize::try_from(v).ok()).filter(|v| *v <= 31).ok_or_else(|| EngineError::Other("uvSet must be in 0..31".into()))?
        }
    };
    let (mut scene, stamp) = crate::uv_cmds::load_scene(&s.uv.source, uv_set)?;
    let topology = photocraft_uv::topology::Topology::build(&scene).map_err(|e| EngineError::Other(e.to_string()))?;
    let uv_changed = s.uv.uv_set != uv_set
        || old.triangles.len() != scene.triangles.len()
        || old.triangles.iter().zip(&scene.triangles).any(|(a, b)| a.corners.iter().zip(b.corners).any(|(a, b)| a.vertex != b.vertex || a.uv != b.uv));
    let mut old_names = BTreeMap::<&str, Vec<usize>>::new();
    let mut new_names = BTreeMap::<&str, usize>::new();
    for (id, material) in old.materials.iter().enumerate() {
        old_names.entry(&material.name).or_default().push(id);
    }
    for material in &scene.materials {
        *new_names.entry(&material.name).or_default() += 1;
    }
    let mapping: Vec<_> = scene
        .materials
        .iter()
        .map(|m| old_names.get(m.name.as_str()).filter(|ids| ids.len() == 1 && new_names.get(m.name.as_str()) == Some(&1)).and_then(|ids| ids.first()).copied())
        .collect();
    let bindings: Vec<_> = mapping.iter().map(|id| id.and_then(|i| s.uv.bindings.get(i).copied().flatten())).collect();
    let normal_bindings: Vec<_> = mapping.iter().map(|id| id.and_then(|i| s.uv.normal_bindings.get(i).copied().flatten())).collect();
    let map_bindings =
        s.uv.map_bindings.iter().map(|(kind, slots)| (*kind, mapping.iter().map(|id| id.and_then(|i| slots.get(i).copied().flatten())).collect())).collect();
    let hidden_materials = mapping.iter().enumerate().filter_map(|(new, old)| old.filter(|old| s.uv.hidden_materials.contains(old)).map(|_| new)).collect();
    let material = mapping.iter().position(|old| *old == Some(s.uv.material)).unwrap_or(0);
    for (id, old) in mapping.iter().enumerate() {
        if old.is_none() {
            scene.warnings.push(format!(
                "{}: new or ambiguous material; assign its texture documents manually",
                scene.materials.get(id).map_or("?", |m| m.name.as_str())
            ));
        }
    }
    let mask_updates = prepare_bound_mask_cache_updates(&scene, &topology, &bindings, &normal_bindings, &map_bindings, s.uv_documents())?;
    if uv_changed {
        scene.warnings.push("UV/topology changed. Texture documents and existing masks were retained; review seam-protection masks.".into());
    }
    // Everything above is fallible. Publish the new geometry only after it has fully validated.
    let scene = Arc::new(scene);
    let wrap_mesh = photocraft_uv::wrap::Mesh::new(&scene).ok().map(Arc::new);
    s.uv.clear_mask_shell_ids_cache();
    s.uv.wrap_mesh = wrap_mesh;
    s.uv.wrapped_stroke = None;
    s.uv.scene = Some(scene);
    s.uv.topology = Some(Arc::new(topology));
    s.uv.bindings = bindings;
    s.uv.normal_bindings = normal_bindings;
    s.uv.map_bindings = map_bindings;
    s.uv.hidden_materials = hidden_materials;
    s.uv.material = material;
    s.uv.uv_set = uv_set;
    s.uv.source_stamp = stamp;
    s.uv.selected.clear();
    s.uv.generation = s.uv.generation.wrapping_add(1);
    apply_bound_mask_cache_updates(s, mask_updates);
    Ok(json!({"reimported":true,"uvChanged":uv_changed,"triangles":s.uv.scene.as_ref().map_or(0,|s|s.triangles.len())}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bound_mask_cache_refresh_invalidates_views_without_dirtying_documents_or_exports() {
        let mut session = Session::new();
        session.execute("file.new", json!({"width":32,"height":32})).unwrap();
        let id = session.active().unwrap().doc.id;

        let initial_content_revision = {
            let state = session.active().unwrap();
            state.revision.saturating_sub(state.view_revision)
        };
        session.uv.exported_revisions.insert(id, initial_content_revision);
        session.uv.saved_revisions.insert(id, initial_content_revision);
        session.uv.scene = Some(Arc::new(photocraft_uv::Scene::default()));
        session.uv.saved_generation = session.uv.generation;

        let before = session.active().unwrap();
        let clean_revision = before.revision;
        let clean_saved_revision = before.saved_revision;
        let clean_view_revision = before.view_revision;
        let mut refreshed = (*before.doc).clone();
        refreshed.name.push_str(" refreshed");
        apply_bound_mask_cache_updates(&mut session, vec![(id, refreshed)]);

        let clean = session.active().unwrap();
        assert_eq!(clean.revision, clean_revision + 1);
        assert_eq!(clean.saved_revision, clean_saved_revision + 1);
        assert_eq!(clean.view_revision, clean_view_revision + 1);
        assert!(!clean.is_dirty(), "cache-only refresh keeps a clean document clean");
        assert!(!session.uv.needs_export(clean), "view revision is excluded from texture export state");
        assert!(!session.uv.has_unsaved_project(session.uv_documents()), "view revision is excluded from project save state");
        assert!(!clean.history.can_undo(), "cache-only refresh does not add an undo step");

        let state = session.active_mut().unwrap();
        state.revision += 1;
        let dirty_revision = state.revision;
        let dirty_saved_revision = state.saved_revision;
        let dirty_view_revision = state.view_revision;
        let mut refreshed = (*state.doc).clone();
        refreshed.name.push_str(" again");
        apply_bound_mask_cache_updates(&mut session, vec![(id, refreshed)]);

        let dirty = session.active().unwrap();
        assert_eq!(dirty.revision, dirty_revision + 1);
        assert_eq!(dirty.saved_revision, dirty_saved_revision, "a dirty document stays dirty");
        assert_eq!(dirty.view_revision, dirty_view_revision + 1);
        assert!(dirty.is_dirty());
        assert!(session.uv.needs_export(dirty), "cache refresh preserves a pending texture export");
        assert!(session.uv.has_unsaved_project(session.uv_documents()), "cache refresh preserves the unsaved project state");
        assert!(!dirty.history.can_undo());
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "uv.model.reimport",
            label: "Reimport model geometry",
            params: r#"{"uvSet":int?}"#,
            enabled,
            run: reload,
            menu: &[],
            shortcut: None,
            journal: true,
        },
        CommandSpec {
            id: "uv.model.checkSource",
            label: "Check source model for changes",
            params: "{}",
            enabled,
            run: check,
            menu: &[],
            shortcut: None,
            journal: false,
        },
    ]
}
