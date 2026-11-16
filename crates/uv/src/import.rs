//! Mesh import entry points.

mod fbx;
use crate::{Corner, Error, MAX_FILE_BYTES, MAX_MATERIALS, MAX_TRIANGLES, MAX_VERTICES, Material, Result, Scene, Triangle};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

// Earcut's worst-case work is quadratic in the number of polygon corners. Keep
// both a per-face cap and an aggregate budget so many moderate n-gons cannot
// multiply that work across an otherwise valid model.
const MAX_FACE_CORNERS: usize = 4_096;
const MAX_TRIANGULATION_WORK: usize = 64 * 1024 * 1024;
const MAX_MTL_DEPENDENCY_BYTES: u64 = MAX_FILE_BYTES;

#[derive(Default)]
struct MtlWorkBudget {
    bytes: u64,
}

impl MtlWorkBudget {
    fn account(&mut self, bytes: usize) -> Result<()> {
        self.bytes = self.bytes.checked_add(bytes as u64).ok_or_else(|| Error("Material-library work budget overflow".into()))?;
        if self.bytes > MAX_MTL_DEPENDENCY_BYTES {
            return Err(Error(format!("OBJ material libraries exceed the {MAX_MTL_DEPENDENCY_BYTES}-byte cumulative import limit")));
        }
        Ok(())
    }
}

fn normalize_library_path(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let mut result = std::path::PathBuf::new();
    for component in std::path::Path::new(&normalized).components() {
        use std::path::Component;
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !result.pop() && !result.has_root() {
                    result.push("..");
                }
            }
            other => result.push(other.as_os_str()),
        }
    }
    result.to_string_lossy().replace('\\', "/")
}

#[derive(Default)]
pub(super) struct TriangulationBudget {
    work: usize,
}

impl TriangulationBudget {
    fn account_face(&mut self, corners: usize) -> Result<()> {
        if corners > MAX_FACE_CORNERS {
            return Err(Error(format!("Polygon exceeds the {MAX_FACE_CORNERS}-corner import limit")));
        }
        let face_work = corners.checked_mul(corners).ok_or_else(|| Error("Polygon triangulation budget overflow".into()))?;
        let total = self.work.checked_add(face_work).ok_or_else(|| Error("Model triangulation budget overflow".into()))?;
        if total > MAX_TRIANGULATION_WORK {
            return Err(Error("Model exceeds the bounded polygon triangulation budget".into()));
        }
        self.work = total;
        Ok(())
    }
}

/// Dependencies are supplied by the host; no filesystem access is needed by the parser.
pub fn load(name: &str, bytes: &[u8], uv_set: usize, mut resolve: impl FnMut(&str) -> Result<Vec<u8>>) -> Result<Scene> {
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(Error("Model file exceeds 256 MiB".into()));
    }
    let ext = Path::new(name).extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase();
    let scene = match ext.as_str() {
        "fbx" => fbx::load(bytes, uv_set)?,
        "obj" => {
            if uv_set != 0 {
                return Err(Error("OBJ has only one UV set".into()));
            }
            obj(bytes, &mut resolve)?
        }
        _ => return Err(Error("Supported model formats: FBX (binary 7.4+), OBJ + MTL".into())),
    };
    scene.validate()?;
    let mut scene = scene;
    if scene.triangles.iter().flat_map(|t| t.corners).any(|c| c.uv.iter().any(|v| !(0.0..=1.0).contains(v))) {
        scene.warnings.push("UV coordinates extend outside tile 1001. This version only edits tile 1001; UDIM is not supported".into());
    }
    Ok(scene)
}

fn obj(bytes: &[u8], resolve: &mut impl FnMut(&str) -> Result<Vec<u8>>) -> Result<Scene> {
    let text = std::str::from_utf8(bytes).map_err(|e| Error(e.to_string()))?;
    let mut scene = Scene::default();
    scene.materials.push(Material::default());
    let mut material_names = BTreeMap::new();
    let mut loaded_mtl_paths = BTreeSet::new();
    let mut mtl_work = MtlWorkBudget::default();
    let mut uvs: Vec<[f64; 2]> = Vec::new();
    let mut normals = Vec::new();
    let mut material = 0;
    let mut material_name: Option<String> = None;
    let mut triangulation_budget = TriangulationBudget::default();
    for (line_number, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        let mut parts = line.split_whitespace();
        let Some(kind) = parts.next() else { continue };
        let bad = || Error(format!("Invalid OBJ data at line {}", line_number + 1));
        match kind {
            "vn" => {
                let p: Vec<f64> = parts.take(3).map(str::parse).collect::<std::result::Result<_, _>>().map_err(|_| bad())?;
                let [x, y, z] = p.as_slice() else { return Err(bad()) };
                if normals.len() >= MAX_VERTICES * 3 {
                    return Err(Error("Too many normals".into()));
                }
                normals.push([*x, *y, *z]);
            }
            "v" => {
                let p: Vec<f64> = parts.take(3).map(str::parse).collect::<std::result::Result<_, _>>().map_err(|_| bad())?;
                let [x, y, z] = p.as_slice() else { return Err(bad()) };
                if scene.positions.len() >= MAX_VERTICES {
                    return Err(Error("Too many vertices".into()));
                }
                scene.positions.push([*x, *y, *z]);
            }
            "vt" => {
                let p: Vec<f64> = parts.take(2).map(str::parse).collect::<std::result::Result<_, _>>().map_err(|_| bad())?;
                let [u, v] = p.as_slice() else { return Err(bad()) };
                if uvs.len() >= MAX_VERTICES * 3 {
                    return Err(Error("Too many UV values".into()));
                }
                uvs.push([*u, *v]);
            }
            "mtllib" => {
                let raw_record = line.strip_prefix(kind).unwrap_or("").trim();
                let explicitly_quoted = raw_record.starts_with('"') && raw_record.ends_with('"') && raw_record.len() >= 2;
                let record = if explicitly_quoted { &raw_record[1..raw_record.len() - 1] } else { raw_record };
                let path = normalize_library_path(&record);
                // If `..` crosses a whitespace token boundary, normalizing the complete record
                // can erase an earlier library name. Preserve the full-name lookup for paths
                // with spaces, then still try the original tokens in this ambiguous case.
                let token_fallback_after_full_match =
                    !explicitly_quoted && record.split_whitespace().count() > 1 && record.replace('\\', "/").split('/').any(|part| part == "..");
                let combined_is_new = loaded_mtl_paths.insert(path.clone());
                if !combined_is_new && !token_fallback_after_full_match {
                    continue;
                }
                let combined_error = if combined_is_new {
                    match resolve(&path) {
                        Ok(bytes) => {
                            load_mtl(&path, &bytes, &mut scene, &mut material_names, &mut mtl_work)?;
                            if let Some(index) = material_name.as_ref().and_then(|name| material_names.get(name)).copied() {
                                material = index;
                            }
                            None
                        }
                        Err(error) => Some(error),
                    }
                } else {
                    None
                };
                let combined_succeeded = combined_is_new && combined_error.is_none();
                let mut loaded_any = combined_error.is_none();
                let should_try_tokens = !explicitly_quoted && (token_fallback_after_full_match || combined_error.is_some());
                let candidates: Vec<_> = if should_try_tokens {
                    record.split_whitespace().map(|name| name.trim_matches('"')).filter(|name| !name.is_empty()).collect()
                } else {
                    Vec::new()
                };
                if candidates.len() > 1 {
                    for candidate in candidates {
                        let candidate = normalize_library_path(candidate);
                        if !loaded_mtl_paths.insert(candidate.clone()) {
                            continue;
                        }
                        match resolve(&candidate) {
                            Ok(bytes) => {
                                load_mtl(&candidate, &bytes, &mut scene, &mut material_names, &mut mtl_work)?;
                                if let Some(index) = material_name.as_ref().and_then(|name| material_names.get(name)).copied() {
                                    material = index;
                                }
                                loaded_any = true;
                            }
                            Err(error) if !combined_succeeded => scene.warnings.push(format!("MTL {candidate}: {error}; assign textures manually")),
                            Err(_) => {}
                        }
                    }
                }
                if !loaded_any {
                    if let Some(error) = combined_error {
                        scene.warnings.push(format!("MTL {path}: {error}; assign textures manually"));
                    }
                }
            }
            "usemtl" => {
                let name = parts.collect::<Vec<_>>().join(" ");
                material_name = Some(name.clone());
                if let Some(index) = material_names.get(&name).copied() {
                    material = index;
                } else {
                    if scene.materials.len() >= MAX_MATERIALS {
                        return Err(Error(format!("Model exceeds the {MAX_MATERIALS}-material import limit")));
                    }
                    material = scene.materials.len();
                    scene.materials.push(Material { name: name.clone(), ..Material::default() });
                    material_names.insert(name, material);
                }
            }
            "f" => {
                let mut face = Vec::new();
                for word in parts {
                    if face.len() >= MAX_FACE_CORNERS {
                        return Err(Error(format!("OBJ polygon exceeds the {MAX_FACE_CORNERS}-corner import limit")));
                    }
                    let mut fields = word.split('/');
                    let v = index(fields.next().unwrap_or(""), scene.positions.len())?;
                    let t = index(fields.next().unwrap_or(""), uvs.len())
                        .map_err(|_| Error(format!("Missing UV at OBJ line {}; unwrap the model before import", line_number + 1)))?;
                    let uv = uvs.get(t).copied().ok_or_else(bad)?;
                    let normal = fields
                        .next()
                        .filter(|s| !s.is_empty())
                        .map(|s| index(s, normals.len()).and_then(|i| normals.get(i).copied().ok_or_else(bad)))
                        .transpose()?;
                    face.push(Corner { vertex: v, uv, normal });
                }
                if triangulate(&scene.positions, &face, material, &mut scene.triangles, &mut triangulation_budget)? && scene.warnings.len() < 100 {
                    scene.warnings.push(format!("OBJ line {}: redundant or degenerate polygon corners omitted", line_number + 1));
                }
            }
            _ => {}
        }
    }
    Ok(scene)
}

fn load_mtl(path: &str, bytes: &[u8], scene: &mut Scene, material_names: &mut BTreeMap<String, usize>, work: &mut MtlWorkBudget) -> Result<()> {
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(Error("Material library exceeds 256 MiB".into()));
    }
    work.account(bytes.len())?;
    check_mtl_material_budget(bytes)?;
    let (materials, _) = tobj::load_mtl_buf(&mut std::io::BufReader::new(bytes)).map_err(|e| Error(e.to_string()))?;
    // FBX/OBJ paths are accepted with either separator independent of the host OS. In
    // particular, Path::parent on Unix treats a backslash as an ordinary filename byte.
    let library_path = path.replace('\\', "/");
    let library_parent = Path::new(&library_path).parent().unwrap_or(Path::new(""));
    for m in materials {
        if m.normal_texture.is_some() {
            scene.warnings.push(format!("{}: MTL bump/height map is not a tangent-space normal map; assign a normal map manually", m.name));
        }
        let c = m.diffuse.unwrap_or([1.0; 3]);
        let texture = m.diffuse_texture.map(|s| library_parent.join(s).to_string_lossy().into_owned());
        let index = if let Some(index) = material_names.get(&m.name).copied() {
            index
        } else {
            if scene.materials.len() >= MAX_MATERIALS {
                return Err(Error(format!("Model exceeds the {MAX_MATERIALS}-material import limit")));
            }
            let index = scene.materials.len();
            material_names.insert(m.name.clone(), index);
            scene.materials.push(Material::default());
            index
        };
        let material = Material {
            name: m.name,
            color: [c[0], c[1], c[2], m.dissolve.unwrap_or(1.0)],
            base_color: texture,
            embedded: None,
            normal_map: m.unknown_param.get("norm").map(|s| library_parent.join(s).to_string_lossy().into_owned()),
            embedded_normal: None,
            maps: [
                (crate::preview::Channel::Emissive, "map_Ke"),
                (crate::preview::Channel::Metallic, "map_Pm"),
                (crate::preview::Channel::Roughness, "map_Pr"),
            ]
            .into_iter()
            .filter_map(|(kind, key)| m.unknown_param.get(key).map(|s| (kind, library_parent.join(s).to_string_lossy().into_owned())))
            .collect(),
        };
        if let Some(slot) = scene.materials.get_mut(index) {
            *slot = material;
        }
    }
    Ok(())
}

fn check_mtl_material_budget(bytes: &[u8]) -> Result<()> {
    let declarations = bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| {
            let line = line.iter().position(|byte| !byte.is_ascii_whitespace()).and_then(|start| line.get(start..)).unwrap_or_default();
            line.starts_with(b"newmtl") && line.get(b"newmtl".len()).is_some_and(u8::is_ascii_whitespace)
        })
        // The default material occupies one of the runtime material slots.
        .take(MAX_MATERIALS)
        .count();
    if declarations >= MAX_MATERIALS {
        return Err(Error(format!("MTL exceeds the {MAX_MATERIALS}-material import limit")));
    }
    Ok(())
}

fn index(s: &str, len: usize) -> Result<usize> {
    let n = s.parse::<i64>().map_err(|_| Error("Invalid OBJ index".into()))?;
    let i = if n < 0 { len as i64 + n } else { n - 1 };
    usize::try_from(i).ok().filter(|i| *i < len).ok_or_else(|| Error("OBJ index is out of range".into()))
}

/// Ear clipping in the polygon's dominant 3D plane also handles concave n-gons.
fn triangulate(positions: &[[f64; 3]], face: &[Corner], material: usize, output: &mut Vec<Triangle>, budget: &mut TriangulationBudget) -> Result<bool> {
    if face.len() < 3 {
        return Err(Error("Polygon has fewer than three vertices".into()));
    }
    if face.iter().any(|c| c.uv.iter().any(|v| !v.is_finite() || v.abs() > 100_000.0)) {
        return Err(Error("Polygon contains invalid UV coordinates".into()));
    }
    budget.account_face(face.len())?;
    if output.len().saturating_add(face.len().saturating_sub(2)) > MAX_TRIANGLES {
        return Err(Error("Too many triangles".into()));
    }
    let points: Vec<glam::DVec3> = face
        .iter()
        .map(|c| positions.get(c.vertex).copied().map(glam::DVec3::from_array).ok_or_else(|| Error("Invalid polygon vertex".into())))
        .collect::<Result<_>>()?;
    if points.iter().any(|p| !p.is_finite() || p.abs().max_element() > 1e15) {
        return Err(Error("Polygon contains invalid coordinates".into()));
    }
    let mut normal = glam::DVec3::ZERO;
    for (a, b) in points.iter().zip(points.iter().cycle().skip(1)).take(points.len()) {
        normal += a.cross(*b);
    }
    // Newell's normal can cancel on a folded or repeated contour. Find a stable
    // projection without dividing by a zero vector; earcut still decides coverage.
    if normal.length_squared() < 1e-24 {
        for triple in points.windows(3) {
            if let [a, b, c] = triple {
                let candidate = (*b - *a).cross(*c - *a);
                if candidate.length_squared() > normal.length_squared() {
                    normal = candidate;
                }
            }
        }
    }
    if normal.length_squared() < 1e-24 {
        return Ok(true);
    }
    let axis = normal.abs().max_element();
    let projected: Vec<f64> = points
        .iter()
        .flat_map(|p| {
            if normal.x.abs() == axis {
                [p.y, p.z]
            } else if normal.y.abs() == axis {
                [p.x, p.z]
            } else {
                [p.x, p.y]
            }
        })
        .collect();
    let indices = earcutr::earcut(&projected, &[], 2).map_err(|e| Error(format!("Cannot triangulate polygon: {e}")))?;
    // Earcut legitimately removes repeated/collinear corners. Rejecting a whole
    // model because its output has fewer triangles loses otherwise valid meshes.
    let mut repaired = indices.len() != face.len().saturating_sub(2) * 3;
    for &[a, b, c] in indices.as_chunks::<3>().0 {
        let [Some(a), Some(b), Some(c)] = [a, b, c].map(|i| face.get(i).copied()) else {
            return Err(Error("Invalid triangulation".into()));
        };
        let mut corners = [a, b, c];
        let [pa, pb, pc] = corners.map(|c| glam::DVec3::from_array(positions.get(c.vertex).copied().unwrap_or([0.0; 3])));
        if (pb - pa).cross(pc - pa).length_squared() < 1e-24 {
            repaired = true;
            continue;
        }
        if (pb - pa).cross(pc - pa).dot(normal) < 0.0 {
            corners.swap(1, 2);
        }
        output.push(Triangle { corners, material });
    }
    Ok(repaired)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn obj_rejects_oversized_ngon_before_earcut() {
        let mut source = String::new();
        for index in 1..=MAX_FACE_CORNERS {
            source.push_str(&format!("v {index} 0 0\nvt {index} 0\n"));
        }
        source.push('f');
        for index in 1..=MAX_FACE_CORNERS + 1 {
            source.push_str(&format!(" {index}/{index}"));
        }

        let error = obj(source.as_bytes(), &mut |_| Err(Error("unused resolver".into()))).unwrap_err();
        assert!(error.to_string().contains("4096-corner import limit"));
    }

    #[test]
    fn triangulation_budget_bounds_per_face_and_total_work() {
        let mut budget = TriangulationBudget::default();
        for _ in 0..4 {
            assert!(budget.account_face(MAX_FACE_CORNERS).is_ok());
        }
        assert!(budget.account_face(MAX_FACE_CORNERS).is_err());
        assert!(TriangulationBudget::default().account_face(MAX_FACE_CORNERS + 1).is_err());
    }

    #[test]
    fn mtl_material_budget_rejects_excess_before_material_parser_allocates() {
        let mut source = b"newmtl material_0\n".to_vec();
        for index in 1..MAX_MATERIALS {
            source.extend_from_slice(format!("newmtl material_{index}\n").as_bytes());
        }
        let error = obj(b"mtllib many-materials.mtl\n", &mut |_| Ok(source.clone())).unwrap_err();
        assert!(error.to_string().contains("MTL exceeds the 4096-material import limit"));

        let at_limit = (0..MAX_MATERIALS - 1).map(|i| format!("newmtl material_{i}\n")).collect::<String>();
        assert!(check_mtl_material_budget(at_limit.as_bytes()).is_ok());
    }

    #[test]
    fn repeated_mtl_reference_is_resolved_once() {
        let source = b"mtllib materials.mtl\nmtllib materials.mtl\n";
        let mut resolve_calls = 0;
        let result = obj(source, &mut |_| {
            resolve_calls += 1;
            Ok(b"newmtl fabric\nKd 0.5 0.4 0.3\n".to_vec())
        });
        assert!(result.is_ok());
        assert_eq!(resolve_calls, 1);
    }

    #[test]
    fn normalized_mtl_aliases_are_resolved_once() {
        let source = b"mtllib data/large.mtl\nmtllib data/./large.mtl\n";
        let mut resolve_calls = 0;
        let result = obj(source, &mut |path| {
            assert_eq!(path, "data/large.mtl");
            resolve_calls += 1;
            Ok(b"newmtl fabric\n".to_vec())
        });
        assert!(result.is_ok());
        assert_eq!(resolve_calls, 1);
    }

    #[test]
    fn token_boundaries_survive_combined_path_normalization() {
        let source = b"mtllib second.mtl\nmtllib first.mtl nested/../second.mtl\n";
        let mut resolved = BTreeSet::new();
        let result = obj(source, &mut |path| {
            resolved.insert(path.to_owned());
            match path {
                "first.mtl" => Ok(b"newmtl first\n".to_vec()),
                "second.mtl" => Ok(b"newmtl second\n".to_vec()),
                _ => Err(Error("not found".into())),
            }
        })
        .unwrap();
        assert!(resolved.contains("first.mtl"));
        assert!(resolved.contains("second.mtl"));
        assert!(result.materials.iter().any(|material| material.name == "first"));
        assert!(result.materials.iter().any(|material| material.name == "second"));
    }

    #[test]
    fn one_mtllib_record_still_loads_tokens_erased_by_combined_normalization() {
        let source = b"mtllib first.mtl nested/../second.mtl\n";
        let result = obj(source, &mut |path| match path {
            "first.mtl" => Ok(b"newmtl first\n".to_vec()),
            "second.mtl" => Ok(b"newmtl second\n".to_vec()),
            _ => Err(Error("not found".into())),
        })
        .unwrap();
        assert!(result.materials.iter().any(|material| material.name == "first"));
        assert!(result.materials.iter().any(|material| material.name == "second"));
    }

    #[test]
    fn complete_mtl_filename_with_spaces_is_tried_before_tokens() {
        let source = b"mtllib my materials.mtl\n";
        let mut resolved = Vec::new();
        let result = obj(source, &mut |path| {
            resolved.push(path.to_owned());
            if path == "my materials.mtl" { Ok(b"newmtl full\n".to_vec()) } else { Err(Error("not found".into())) }
        });
        assert!(result.is_ok());
        assert_eq!(resolved, ["my materials.mtl"]);
    }

    #[test]
    fn quoted_mtl_filename_is_not_split_after_lookup_failure() {
        let source = b"mtllib \"my materials.mtl\"\n";
        let mut resolved = Vec::new();
        let result = obj(source, &mut |path| {
            resolved.push(path.to_owned());
            Err(Error("not found".into()))
        })
        .unwrap();
        assert_eq!(resolved, ["my materials.mtl"]);
        assert!(result.warnings.iter().any(|warning| warning.contains("MTL my materials.mtl: not found")));
    }

    #[test]
    fn cumulative_mtl_work_budget_covers_all_libraries() {
        let mut budget = MtlWorkBudget::default();
        assert!(budget.account(MAX_MTL_DEPENDENCY_BYTES as usize).is_ok());
        assert!(budget.account(1).is_err());
    }

    #[test]
    fn mtl_map_paths_use_normalized_library_parent() {
        let source = b"mtllib materials\\paint.mtl\n";
        let result = obj(source, &mut |path| {
            assert_eq!(path, "materials/paint.mtl");
            Ok(b"newmtl fabric\nmap_Kd lacquer.png\n".to_vec())
        })
        .unwrap();
        assert_eq!(result.materials[1].base_color.as_deref(), Some(std::path::Path::new("materials").join("lacquer.png").to_string_lossy().as_ref()));
    }

    #[test]
    fn mtllib_falls_back_to_each_name_when_the_combined_path_is_missing() {
        let source = b"mtllib first.mtl second.mtl\n";
        let result = obj(source, &mut |path| match path {
            "first.mtl" => Ok(b"newmtl first\n".to_vec()),
            "second.mtl" => Ok(b"newmtl second\n".to_vec()),
            _ => Err(Error("not found".into())),
        })
        .unwrap();
        assert!(result.materials.iter().any(|material| material.name == "first"));
        assert!(result.materials.iter().any(|material| material.name == "second"));
    }

    #[test]
    fn mtl_reuses_material_created_by_earlier_usemtl() {
        let source = b"usemtl fabric\nmtllib materials.mtl\n";
        let result = obj(source, &mut |_| Ok(b"newmtl fabric\nKd 0.5 0.4 0.3\n".to_vec())).unwrap();
        assert_eq!(result.materials.len(), 2);
        assert_eq!(result.materials.get(1).map(|material| material.color[..3].to_vec()), Some(vec![0.5, 0.4, 0.3]));
    }

    #[test]
    fn obj_rejects_excess_material_names_as_they_are_added() {
        let source = (0..MAX_MATERIALS).map(|index| format!("usemtl material_{index}\n")).collect::<String>();
        let error = obj(source.as_bytes(), &mut |_| Err(Error("unused resolver".into()))).unwrap_err();
        assert!(error.to_string().contains("4096-material import limit"));
    }
}
