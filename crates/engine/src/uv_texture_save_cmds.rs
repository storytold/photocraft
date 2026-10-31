//! Explicit, dialog-free writeback of modified textures bound to the current UV scene.
use crate::{Result, Session, commands::CommandSpec};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};

fn save(s: &mut Session, _: &Value) -> Result<Value> {
    let bound: HashSet<_> = s.uv.all_bindings().flatten().copied().collect();
    let mut paths = BTreeMap::<String, usize>::new();
    for st in s.documents().iter().filter(|st| bound.contains(&st.doc.id)) {
        if let Some(path) = &st.path {
            *paths.entry(path.replace('\\', "/").to_lowercase()).or_default() += 1;
        }
    }
    let (mut saved, mut unsaved) = (0, 0);
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    for index in 0..s.documents().len() {
        let Some(st) = s.documents().get(index).filter(|st| bound.contains(&st.doc.id) && s.uv.needs_export(st)) else { continue };
        if s.uv.blocked_exports.contains(&st.doc.id) {
            errors.push("External texture destination restored from project: explicitly rebind the texture before exporting".into());
            continue;
        }
        let Some(path) = st.path.clone().filter(|p| !p.is_empty()) else {
            unsaved += 1;
            continue;
        };
        if paths.get(&path.replace('\\', "/").to_lowercase()).copied().unwrap_or(0) > 1 {
            errors.push(format!("{path}: several bound documents target the same file"));
            continue;
        }
        // The user explicitly chose writeback: flat maps retain their original format,
        // encoding, depth and profile through the normal export path. Layers live in .pcuv.
        let _scope = crate::doc_path::scope(Some(std::path::Path::new(&path)));
        match crate::file_cmds::save_doc(&st.doc, &path, crate::file_cmds::SaveOpts::default()) {
            Ok(w) => {
                warnings.extend(w);
                if let Some(st) = s.docs.get_mut(index) {
                    s.uv.exported_revisions.insert(st.doc.id, st.revision.saturating_sub(st.view_revision));
                }
                saved += 1;
            }
            Err(e) => errors.push(e.to_string()),
        }
    }
    Ok(json!({"saved":saved,"unsaved":unsaved,"errors":errors,"warnings":warnings}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "file.uv.saveAllTextures",
        label: "Export Textures",
        menu: &["File", "3D Import"],
        shortcut: None,
        params: "{}; writes modified bound documents to their existing paths, skips unsaved textures",
        enabled: |s| if s.uv.scene.is_some() { Ok(()) } else { Err("Import a model first".into()) },
        run: save,
        journal: false,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uv_save_all_writes_only_dirty_bound_maps_deduplicates_and_retains_failed_work() {
        let mut s = Session::new();
        assert!(s.execute("file.uv.saveAllTextures", json!({})).is_err());
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/two-shells.obj");
        s.execute("uv.model.import", json!({"path":fixture})).unwrap();
        s.execute("uv.texture.create", json!({"width":32,"height":32})).unwrap();
        let id = s.active().unwrap().doc.id;
        s.uv.normal_bindings = s.uv.bindings.clone(); // Same texture in two slots is saved once.
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("map.png").to_string_lossy().into_owned();
        s.active_mut().unwrap().saved_to(path.clone());
        s.execute("edit.fill", json!({"color":"#ff0000"})).unwrap();
        let result = s.execute("file.uv.saveAllTextures", json!({})).unwrap();
        assert_eq!(result["saved"], 1);
        assert!(!s.uv.needs_export(s.active().unwrap()));
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(s.execute("file.uv.saveAllTextures", json!({})).unwrap()["saved"], 0);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        s.execute("edit.fill", json!({"color":"#00ff00"})).unwrap();
        assert_eq!(s.execute("file.uv.saveAllTextures", json!({})).unwrap()["saved"], 1);
        let image = crate::file_cmds::import("map.png", &std::fs::read(&path).unwrap()).unwrap();
        let px = photocraft_compose::render(&image, photocraft_geom::Rect::from_xywh(2, 2, 1, 1)).px[0];
        assert!(px[1] > 0.99 && px[0] < 0.01);
        s.active_mut().unwrap().path = Some(temp.path().join("invalid.unsupported").to_string_lossy().into_owned());
        s.execute("edit.fill", json!({"color":"#0000ff"})).unwrap();
        let result = s.execute("file.uv.saveAllTextures", json!({})).unwrap();
        assert_eq!(result["errors"].as_array().unwrap().len(), 1);
        assert!(s.active().unwrap().is_dirty());

        s.active_mut().unwrap().path = Some(path.clone());
        let project = temp.path().join("scene.pcuv");
        s.execute("uv.project.save", json!({"path":project})).unwrap();
        let mut restored = Session::new();
        restored.execute("uv.project.load", json!({"path":project})).unwrap();
        assert!(!restored.active().unwrap().is_dirty());
        assert!(restored.uv.needs_export(restored.active().unwrap()));
        assert_eq!(restored.execute("file.uv.saveAllTextures", json!({})).unwrap()["saved"], 1);

        restored.execute("uv.project.save", json!({"path":project})).unwrap();
        let bytes = std::fs::read(&project).unwrap();
        let zip = photocraft_format::zip::ZipReader::new(&bytes).unwrap();
        let mut manifest: Value = serde_json::from_slice(&zip.read_by_name("project.json", 1024 * 1024).unwrap()).unwrap();
        manifest.as_object_mut().unwrap().remove("dirty_documents");
        let mut writer = photocraft_format::zip::ZipWriter::new();
        for entry in &zip.entries {
            let bytes = if entry.name == "project.json" { serde_json::to_vec(&manifest).unwrap() } else { zip.read(entry, 1024 * 1024).unwrap() };
            writer.add(&entry.name, &bytes).unwrap();
        }
        std::fs::write(&project, writer.finish().unwrap()).unwrap();
        let mut legacy = Session::new();
        legacy.execute("uv.project.load", json!({"path":project})).unwrap();
        assert!(!legacy.active().unwrap().is_dirty());
        assert!(legacy.uv.needs_export(legacy.active().unwrap()), "old embedded maps need their first explicit export");
        assert_eq!(legacy.execute("file.uv.saveAllTextures", json!({})).unwrap()["saved"], 1);
        s.active_mut().unwrap().path = None;
        assert_eq!(s.execute("file.uv.saveAllTextures", json!({})).unwrap()["unsaved"], 1);
        assert_eq!(s.active().unwrap().doc.id, id);
    }
}
