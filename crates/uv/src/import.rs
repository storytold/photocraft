//! Mesh import entry points.

mod fbx;
use crate::{Corner, Error, MAX_FILE_BYTES, MAX_TRIANGLES, MAX_VERTICES, Material, Result, Scene, Triangle};
use std::collections::BTreeMap;
use std::path::Path;

// Earcut's worst-case work is quadratic in the number of polygon corners. Keep
// both a per-face cap and an aggregate budget so many moderate n-gons cannot
// multiply that work across an otherwise valid model.
const MAX_FACE_CORNERS: usize = 4_096;
const MAX_TRIANGULATION_WORK: usize = 64 * 1024 * 1024;

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
    let mut uvs: Vec<[f64; 2]> = Vec::new();
    let mut normals = Vec::new();
    let mut material = 0;
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
                let path = parts.collect::<Vec<_>>().join(" ");
                match resolve(&path) {
                    Ok(bytes) => {
                        let (materials, _) = tobj::load_mtl_buf(&mut std::io::BufReader::new(bytes.as_slice())).map_err(|e| Error(e.to_string()))?;
                        for m in materials {
                            if m.normal_texture.is_some() {
                                scene.warnings.push(format!("{}: MTL bump/height map is not a tangent-space normal map; assign a normal map manually", m.name));
                            }
                            let c = m.diffuse.unwrap_or([1.0; 3]);
                            let texture = m.diffuse_texture.map(|s| Path::new(&path).parent().unwrap_or(Path::new("")).join(s).to_string_lossy().into_owned());
                            material_names.insert(m.name.clone(), scene.materials.len());
                            scene.materials.push(Material {
                                name: m.name,
                                color: [c[0], c[1], c[2], m.dissolve.unwrap_or(1.0)],
                                base_color: texture,
                                embedded: None,
                                normal_map: m
                                    .unknown_param
                                    .get("norm")
                                    .map(|s| Path::new(&path).parent().unwrap_or(Path::new("")).join(s).to_string_lossy().into_owned()),
                                embedded_normal: None,
                                maps: [
                                    (crate::preview::Channel::Emissive, "map_Ke"),
                                    (crate::preview::Channel::Metallic, "map_Pm"),
                                    (crate::preview::Channel::Roughness, "map_Pr"),
                                ]
                                .into_iter()
                                .filter_map(|(kind, key)| {
                                    m.unknown_param
                                        .get(key)
                                        .map(|s| (kind, Path::new(&path).parent().unwrap_or(Path::new("")).join(s).to_string_lossy().into_owned()))
                                })
                                .collect(),
                            });
                        }
                    }
                    Err(e) => scene.warnings.push(format!("MTL {path}: {e}; assign textures manually")),
                }
            }
            "usemtl" => {
                let name = parts.collect::<Vec<_>>().join(" ");
                material = *material_names.entry(name.clone()).or_insert_with(|| {
                    let id = scene.materials.len();
                    scene.materials.push(Material { name, ..Material::default() });
                    id
                });
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
        source.push_str("f");
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
}
