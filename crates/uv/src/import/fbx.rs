//! Binary FBX static meshes, preserving per-corner UV and control-point identities.
use crate::{Corner, Error, MAX_VERTICES, Material, Result, Scene};
use fbxcel::{
    low::v7400::AttributeValue as A,
    tree::{any::AnyTree, v7400::NodeHandle},
};
use glam::{DMat4, DQuat, DVec3, EulerRot};
use std::collections::{BTreeMap, BTreeSet};

// `fbxcel::AnyTree` owns every node and attribute, including arrays which the
// mesh importer may never use. Bound that allocation before constructing it.
const FBX_HEADER_LEN: usize = 27;
const FBX_MAX_DEPTH: usize = 256;
const FBX_MAX_NODES: usize = 1_000_000;
const FBX_MAX_ATTRIBUTES: u64 = 2_000_000;
const FBX_MAX_ARRAY_BYTES: u64 = 128 * 1024 * 1024;
const FBX_MAX_DECODED_BYTES: u64 = 384 * 1024 * 1024;

#[derive(Default)]
struct PreflightBudget {
    nodes: usize,
    attributes: u64,
    decoded_bytes: u64,
}

fn preflight_error() -> Error {
    Error("FBX structure exceeds safe import limits or is malformed".into())
}

fn checked_end(start: usize, length: usize, bound: usize) -> Result<usize> {
    let end = start.checked_add(length).ok_or_else(preflight_error)?;
    if end > bound {
        return Err(preflight_error());
    }
    Ok(end)
}

fn read_u32(bytes: &[u8], offset: usize, bound: usize) -> Result<(u32, usize)> {
    let end = checked_end(offset, 4, bound)?;
    let value = u32::from_le_bytes(bytes.get(offset..end).ok_or_else(preflight_error)?.try_into().map_err(|_| preflight_error())?);
    Ok((value, end))
}

fn account_decoded(budget: &mut PreflightBudget, amount: u64) -> Result<()> {
    budget.decoded_bytes = budget.decoded_bytes.checked_add(amount).ok_or_else(preflight_error)?;
    if budget.decoded_bytes > FBX_MAX_DECODED_BYTES {
        return Err(preflight_error());
    }
    Ok(())
}

fn scan_attributes(bytes: &[u8], mut offset: usize, count: u64, end: usize, budget: &mut PreflightBudget) -> Result<()> {
    budget.attributes = budget.attributes.checked_add(count).ok_or_else(preflight_error)?;
    if budget.attributes > FBX_MAX_ATTRIBUTES {
        return Err(preflight_error());
    }
    for _ in 0..count {
        let code = *bytes.get(offset).filter(|_| offset < end).ok_or_else(preflight_error)?;
        offset = offset.checked_add(1).ok_or_else(preflight_error)?;
        let scalar_size = match code {
            b'C' => Some(1),
            b'Y' => Some(2),
            b'I' | b'F' => Some(4),
            b'L' | b'D' => Some(8),
            _ => None,
        };
        if let Some(size) = scalar_size {
            offset = checked_end(offset, size, end)?;
            account_decoded(budget, size as u64)?;
            continue;
        }
        let element_size = match code {
            b'b' => 1_u64,
            b'i' | b'f' => 4,
            b'l' | b'd' => 8,
            b'R' | b'S' => {
                let (length, data_start) = read_u32(bytes, offset, end)?;
                let data_end = checked_end(data_start, usize::try_from(length).map_err(|_| preflight_error())?, end)?;
                account_decoded(budget, u64::from(length))?;
                offset = data_end;
                continue;
            }
            _ => return Err(preflight_error()),
        };

        // ArrayAttributeHeader is (u32 count, u32 encoding, u32 byte length).
        let (elements, p) = read_u32(bytes, offset, end)?;
        let (encoding, p) = read_u32(bytes, p, end)?;
        let (stored_len, payload) = read_u32(bytes, p, end)?;
        if encoding > 1 {
            return Err(preflight_error());
        }
        let decoded_len = u64::from(elements).checked_mul(element_size).ok_or_else(preflight_error)?;
        if decoded_len > FBX_MAX_ARRAY_BYTES {
            return Err(preflight_error());
        }
        let stored_len_usize = usize::try_from(stored_len).map_err(|_| preflight_error())?;
        let data_end = checked_end(payload, stored_len_usize, end)?;
        match encoding {
            0 if u64::from(stored_len) != decoded_len => return Err(preflight_error()),
            1 if decoded_len != 0 && stored_len == 0 => return Err(preflight_error()),
            _ => {}
        }
        account_decoded(budget, decoded_len)?;
        offset = data_end;
    }
    if offset > end {
        return Err(preflight_error());
    }
    // FBX permits unused bytes in the declared property block; fbxcel skips
    // them after reading the declared attribute count.
    Ok(())
}

fn scan_nodes(bytes: &[u8], mut offset: usize, bound: usize, root: bool, depth: usize, width: usize, budget: &mut PreflightBudget) -> Result<usize> {
    if depth > FBX_MAX_DEPTH {
        return Err(preflight_error());
    }
    while offset < bound {
        let header_end = checked_end(offset, width, bound)?;
        let (node_end, attr_count, attr_len, name_len, header_end) = if width == 13 {
            let (node_end, p) = read_u32(bytes, offset, bound)?;
            let (attrs, p) = read_u32(bytes, p, bound)?;
            let (attr_bytes, p) = read_u32(bytes, p, bound)?;
            let name_len = *bytes.get(p).filter(|_| p < bound).ok_or_else(preflight_error)?;
            (u64::from(node_end), u64::from(attrs), u64::from(attr_bytes), name_len, p + 1)
        } else {
            let end = bytes.get(offset..header_end).ok_or_else(preflight_error)?;
            let node_end = u64::from_le_bytes(end.get(0..8).ok_or_else(preflight_error)?.try_into().map_err(|_| preflight_error())?);
            let attrs = u64::from_le_bytes(end.get(8..16).ok_or_else(preflight_error)?.try_into().map_err(|_| preflight_error())?);
            let attr_bytes = u64::from_le_bytes(end.get(16..24).ok_or_else(preflight_error)?.try_into().map_err(|_| preflight_error())?);
            let name_len = *end.get(24).ok_or_else(preflight_error)?;
            (node_end, attrs, attr_bytes, name_len, header_end)
        };
        if node_end == 0 && attr_count == 0 && attr_len == 0 && name_len == 0 {
            let after_marker = header_end;
            if !root && after_marker != bound {
                return Err(preflight_error());
            }
            return Ok(after_marker);
        }
        budget.nodes = budget.nodes.checked_add(1).ok_or_else(preflight_error)?;
        if budget.nodes > FBX_MAX_NODES {
            return Err(preflight_error());
        }
        let node_end = usize::try_from(node_end).map_err(|_| preflight_error())?;
        if node_end <= offset || node_end > bound {
            return Err(preflight_error());
        }
        let name_end = checked_end(header_end, usize::from(name_len), node_end)?;
        let attr_len = usize::try_from(attr_len).map_err(|_| preflight_error())?;
        let attrs_end = checked_end(name_end, attr_len, node_end)?;
        let estimated_metadata = attr_count.checked_mul(32).and_then(|n| n.checked_add(u64::from(name_len) + 64)).ok_or_else(preflight_error)?;
        account_decoded(budget, estimated_metadata)?;
        scan_attributes(bytes, name_end, attr_count, attrs_end, budget)?;
        let child_end = scan_nodes(bytes, attrs_end, node_end, false, depth + 1, width, budget)?;
        if child_end != node_end {
            return Err(preflight_error());
        }
        offset = node_end;
    }
    if offset != bound {
        return Err(preflight_error());
    }
    if root {
        // A binary FBX document ends its top-level nodes with a null record.
        return Err(preflight_error());
    }
    Ok(offset)
}

fn preflight_binary(bytes: &[u8]) -> Result<()> {
    const MAGIC: &[u8] = b"Kaydara FBX Binary  \x00\x1a\x00";
    if bytes.len() < FBX_HEADER_LEN || !bytes.starts_with(MAGIC) {
        return Err(preflight_error());
    }
    let version = u32::from_le_bytes(bytes.get(23..27).ok_or_else(preflight_error)?.try_into().map_err(|_| preflight_error())?);
    if !(7400..8000).contains(&version) {
        return Err(Error("Unsupported FBX version".into()));
    }
    let width = if version < 7500 { 13 } else { 25 };
    let mut budget = PreflightBudget::default();
    let _ = scan_nodes(bytes, FBX_HEADER_LEN, bytes.len(), true, 0, width, &mut budget)?;
    Ok(())
}

fn child<'a>(n: NodeHandle<'a>, name: &str) -> Option<NodeHandle<'a>> {
    n.first_child_by_name(name)
}
fn string(n: NodeHandle<'_>, index: usize) -> Option<&str> {
    n.attributes().get(index)?.get_string()
}
fn id(n: NodeHandle<'_>) -> Option<i64> {
    n.attributes().first()?.get_i64()
}
fn text<'a>(n: NodeHandle<'a>, name: &str) -> Option<&'a str> {
    string(child(n, name)?, 0)
}
fn floats(n: NodeHandle<'_>, name: &str) -> Result<Vec<f64>> {
    let values = child(n, name).and_then(|n| n.attributes().first());
    match values {
        Some(A::ArrF64(a)) => Ok(a.clone()),
        Some(A::ArrF32(a)) => Ok(a.iter().map(|v| f64::from(*v)).collect()),
        _ => Err(Error(format!("FBX {name} array is missing"))),
    }
}
fn ints(n: NodeHandle<'_>, name: &str) -> Result<Vec<i32>> {
    child(n, name)
        .and_then(|n| n.attributes().first())
        .and_then(A::get_arr_i32)
        .map(|a| a.to_vec())
        .ok_or_else(|| Error(format!("FBX {name} array is missing")))
}
fn property<'a>(n: NodeHandle<'a>, name: &str) -> Option<NodeHandle<'a>> {
    child(n, "Properties70")?.children_by_name("P").find(|p| string(*p, 0) == Some(name))
}
fn vector(n: NodeHandle<'_>, name: &str, default: DVec3) -> DVec3 {
    property(n, name)
        .and_then(|p| {
            let a = p.attributes();
            Some(DVec3::new(a.get(4)?.get_f64()?, a.get(5)?.get_f64()?, a.get(6)?.get_f64()?))
        })
        .unwrap_or(default)
}
fn rotate(v: DVec3, order: i32) -> DMat4 {
    let v = v * std::f64::consts::PI / 180.0;
    let q = match order {
        1 => DQuat::from_euler(EulerRot::YZX, v.y, v.z, v.x),
        2 => DQuat::from_euler(EulerRot::XZY, v.x, v.z, v.y),
        3 => DQuat::from_euler(EulerRot::ZXY, v.z, v.x, v.y),
        4 => DQuat::from_euler(EulerRot::YXZ, v.y, v.x, v.z),
        5 => DQuat::from_euler(EulerRot::XYZ, v.x, v.y, v.z),
        _ => DQuat::from_euler(EulerRot::ZYX, v.z, v.y, v.x),
    };
    DMat4::from_quat(q)
}
fn integer(n: NodeHandle<'_>, name: &str) -> i32 {
    property(n, name).and_then(|p| p.attributes().get(4)).and_then(A::get_i32).unwrap_or(0)
}
fn transform(n: NodeHandle<'_>, geometric: bool) -> DMat4 {
    let v = |name, default| vector(n, name, default);
    let t = |x| DMat4::from_translation(x);
    if geometric {
        return t(v("GeometricTranslation", DVec3::ZERO))
            * rotate(v("GeometricRotation", DVec3::ZERO), 0)
            * DMat4::from_scale(v("GeometricScaling", DVec3::ONE));
    }
    let rp = v("RotationPivot", DVec3::ZERO);
    let sp = v("ScalingPivot", DVec3::ZERO);
    t(v("Lcl Translation", DVec3::ZERO))
        * t(v("RotationOffset", DVec3::ZERO))
        * t(rp)
        * rotate(v("PreRotation", DVec3::ZERO), 0)
        * rotate(v("Lcl Rotation", DVec3::ZERO), integer(n, "RotationOrder"))
        * rotate(v("PostRotation", DVec3::ZERO), 0).inverse()
        * t(-rp)
        * t(v("ScalingOffset", DVec3::ZERO))
        * t(sp)
        * DMat4::from_scale(v("Lcl Scaling", DVec3::ONE))
        * t(-sp)
}

pub(super) fn load(bytes: &[u8], uv_set: usize) -> Result<Scene> {
    if !bytes.starts_with(b"Kaydara FBX Binary") {
        return Err(Error("This build imports binary FBX 7.4+; export FBX as Binary from Maya (ASCII FBX is not supported)".into()));
    }
    preflight_binary(bytes)?;
    let tree = AnyTree::from_seekable_reader(std::io::Cursor::new(bytes)).map_err(|e| Error(format!("FBX: {e}")))?;
    let AnyTree::V7400(_, tree, _) = tree else { return Err(Error("Unsupported FBX version".into())) };
    let root = tree.root();
    let objects = child(root, "Objects").ok_or_else(|| Error("FBX Objects missing".into()))?;
    let connections: Vec<(i64, i64, String)> = child(root, "Connections")
        .map(|c| {
            c.children_by_name("C")
                .filter_map(|n| {
                    let a = n.attributes();
                    Some((a.get(1)?.get_i64()?, a.get(2)?.get_i64()?, a.get(3).and_then(A::get_string).unwrap_or("").to_owned()))
                })
                .collect()
        })
        .unwrap_or_default();
    let by_id: BTreeMap<i64, NodeHandle<'_>> = objects.children().filter_map(|n| Some((id(n)?, n))).collect();
    let mut scene = Scene::default();
    let mut materials = BTreeMap::new();
    for m in objects.children_by_name("Material") {
        let Some(mid) = id(m) else { continue };
        let color = vector(m, "DiffuseColor", DVec3::ONE);
        let texture = connections
            .iter()
            .filter(|(_, to, prop)| *to == mid && (prop == "DiffuseColor" || prop == "Diffuse" || prop == "BaseColor"))
            .find_map(|(from, _, _)| by_id.get(from).copied().filter(|n| n.name() == "Texture"));
        let filename = texture.and_then(|t| text(t, "RelativeFilename").filter(|s| !s.is_empty()).or_else(|| text(t, "FileName"))).map(str::to_owned);
        let embedded = texture.and_then(id).and_then(|tid| {
            connections.iter().find_map(|(from, to, _)| {
                if *to != tid {
                    return None;
                }
                let video = by_id.get(from).copied().filter(|n| n.name() == "Video")?;
                child(video, "Content")?.attributes().first()?.get_binary().map(|b| b.to_vec())
            })
        });
        let normal_texture = connections
            .iter()
            .filter(|(_, to, prop)| *to == mid && (prop == "NormalMap" || prop == "normalCamera"))
            .find_map(|(from, _, _)| by_id.get(from).copied().filter(|n| n.name() == "Texture"));
        let normal_map = normal_texture.and_then(|t| text(t, "RelativeFilename").filter(|s| !s.is_empty()).or_else(|| text(t, "FileName"))).map(str::to_owned);
        let embedded_normal = normal_texture.and_then(id).and_then(|tid| {
            connections.iter().find_map(|(from, to, _)| {
                if *to != tid {
                    return None;
                }
                let video = by_id.get(from).copied().filter(|n| n.name() == "Video")?;
                child(video, "Content")?.attributes().first()?.get_binary().map(|b| b.to_vec())
            })
        });
        let mut maps = BTreeMap::new();
        for (kind, properties) in [
            (crate::preview::Channel::Id, &["MaterialID", "ID"][..]),
            (crate::preview::Channel::Ao, &["AmbientOcclusion", "AmbientOcclusionMap"][..]),
            (crate::preview::Channel::Curvature, &["Curvature"][..]),
            (crate::preview::Channel::Opacity, &["Opacity", "OpacityMap", "TransparentColor"][..]),
            (crate::preview::Channel::Metallic, &["Metalness", "Metallic"][..]),
            (crate::preview::Channel::Roughness, &["Roughness", "DiffuseRoughness"][..]),
            (crate::preview::Channel::Emissive, &["EmissiveColor", "Emissive", "Emission"][..]),
        ] {
            if let Some(texture) = connections
                .iter()
                .filter(|(_, to, prop)| *to == mid && properties.contains(&prop.as_str()))
                .find_map(|(from, _, _)| by_id.get(from).copied().filter(|n| n.name() == "Texture"))
                && let Some(name) = text(texture, "RelativeFilename").filter(|s| !s.is_empty()).or_else(|| text(texture, "FileName"))
            {
                maps.insert(kind, name.to_owned());
            }
        }
        materials.insert(mid, scene.materials.len());
        scene.materials.push(Material {
            name: string(m, 1).unwrap_or("Material").split('\0').next().unwrap_or("Material").trim_start_matches("Material::").to_owned(),
            color: [color.x as f32, color.y as f32, color.z as f32, 1.0],
            base_color: filename,
            embedded,
            normal_map,
            embedded_normal,
            maps,
        });
    }
    let default_material = scene.materials.len();
    scene.materials.push(Material::default());
    let mut warned_skin = false;
    let mut triangulation_budget = super::TriangulationBudget::default();
    for geometry in objects.children_by_name("Geometry").filter(|n| string(*n, 2) == Some("Mesh")) {
        let gid = id(geometry).ok_or_else(|| Error("FBX geometry id missing".into()))?;
        let vertices = floats(geometry, "Vertices")?;
        let polygons = ints(geometry, "PolygonVertexIndex")?;
        if !vertices.len().is_multiple_of(3) || vertices.len() / 3 > MAX_VERTICES {
            return Err(Error("Invalid FBX vertex array".into()));
        }
        let uv = geometry
            .children_by_name("LayerElementUV")
            .nth(uv_set)
            .ok_or_else(|| Error(format!("FBX mesh has no UV set {uv_set}; unwrap it before import")))?;
        let uv_values = floats(uv, "UV")?;
        let uv_indices = if text(uv, "ReferenceInformationType") == Some("IndexToDirect") { Some(ints(uv, "UVIndex")?) } else { None };
        let mapping = text(uv, "MappingInformationType").unwrap_or("ByPolygonVertex");
        let normals = child(geometry, "LayerElementNormal");
        let normal_values = normals.map(|n| floats(n, "Normals")).transpose()?.unwrap_or_default();
        let normal_mapping = normals.and_then(|n| text(n, "MappingInformationType")).unwrap_or("ByPolygonVertex");
        let normal_indices = normals
            .filter(|n| matches!(text(*n, "ReferenceInformationType"), Some("IndexToDirect" | "Index")))
            .map(|n| ints(n, if child(n, "NormalsIndex").is_some() { "NormalsIndex" } else { "NormalIndex" }))
            .transpose()?;
        let material_layer = child(geometry, "LayerElementMaterial");
        let material_indices = material_layer.and_then(|n| ints(n, "Materials").ok()).unwrap_or_default();
        let all_same = material_layer.and_then(|n| text(n, "MappingInformationType")) == Some("AllSame");
        let models: Vec<NodeHandle<'_>> =
            connections.iter().filter(|(from, _, _)| *from == gid).filter_map(|(_, to, _)| by_id.get(to).copied().filter(|n| n.name() == "Model")).collect();
        if models.is_empty() {
            scene.warnings.push("Unconnected FBX geometry skipped".into());
        }
        for model in models {
            let mid = id(model).ok_or_else(|| Error("Missing model id".into()))?;
            let mut matrix = transform(model, false);
            let mut current = mid;
            let mut seen = BTreeSet::new();
            seen.insert(mid);
            while let Some(parent) =
                connections.iter().filter(|(from, _, _)| *from == current).find_map(|(_, to, _)| by_id.get(to).copied().filter(|n| n.name() == "Model"))
            {
                current = id(parent).ok_or_else(|| Error("Missing parent id".into()))?;
                if seen.len() >= 128 || !seen.insert(current) {
                    return Err(Error("Cyclic or overly deep FBX hierarchy".into()));
                }
                matrix = transform(parent, false) * matrix;
            }
            matrix *= transform(model, true);
            let reflected = matrix.determinant() < 0.0;
            let normal_matrix = matrix.inverse().transpose();
            if !normal_matrix.is_finite() {
                return Err(Error("FBX has a singular mesh transform; freeze transforms before importing".into()));
            }
            if integer(model, "InheritType") != 0 {
                scene.warnings.push("FBX nondefault transform inheritance is approximated; freeze mesh transforms for exact placement".into());
            }
            let attached: Vec<usize> = connections.iter().filter(|(_, to, _)| *to == mid).filter_map(|(from, _, _)| materials.get(from).copied()).collect();
            let offset = scene.positions.len();
            if offset.saturating_add(vertices.len() / 3) > MAX_VERTICES {
                return Err(Error("Too many FBX vertices".into()));
            }
            scene.positions.extend(vertices.as_chunks::<3>().0.iter().map(|&[x, y, z]| matrix.transform_point3(DVec3::new(x, y, z)).to_array()));
            let mut face = Vec::new();
            let mut polygon = 0;
            for (corner, &raw) in polygons.iter().enumerate() {
                let vertex = usize::try_from(if raw < 0 { !raw } else { raw }).map_err(|_| Error("Invalid FBX vertex index".into()))?;
                if vertex >= vertices.len() / 3 {
                    return Err(Error("FBX vertex index is out of range".into()));
                }
                let mapped = match mapping {
                    "ByPolygonVertex" => corner,
                    "ByVertice" | "ByVertex" => vertex,
                    "ByPolygon" => polygon,
                    "AllSame" => 0,
                    _ => return Err(Error(format!("Unsupported FBX UV mapping: {mapping}"))),
                };
                let direct = if let Some(indices) = &uv_indices {
                    indices.get(mapped).copied().and_then(|n| usize::try_from(n).ok()).ok_or_else(|| Error("FBX UV index out of range".into()))?
                } else {
                    mapped
                };
                let at = direct.checked_mul(2).ok_or_else(|| Error("FBX UV index overflow".into()))?;
                let (Some(&u), Some(&v)) = (uv_values.get(at), uv_values.get(at + 1)) else { return Err(Error("FBX UV coordinates out of range".into())) };
                let normal = if normals.is_some() {
                    let mapped = match normal_mapping {
                        "ByPolygonVertex" => corner,
                        "ByVertice" | "ByVertex" => vertex,
                        "ByPolygon" => polygon,
                        "AllSame" => 0,
                        _ => return Err(Error("Unsupported FBX normal mapping".into())),
                    };
                    let direct = if let Some(indices) = &normal_indices {
                        indices.get(mapped).and_then(|v| usize::try_from(*v).ok()).ok_or_else(|| Error("FBX normal index out of range".into()))?
                    } else {
                        mapped
                    };
                    let at = direct.checked_mul(3).ok_or_else(|| Error("FBX normal index overflow".into()))?;
                    let xyz = normal_values.get(at..at.saturating_add(3)).ok_or_else(|| Error("FBX normals out of range".into()))?;
                    let &[x, y, z] = xyz else { return Err(Error("Invalid FBX normal".into())) };
                    let n = normal_matrix.transform_vector3(DVec3::new(x, y, z));
                    if !n.is_finite() || n.length_squared() < 1e-24 {
                        return Err(Error("Invalid FBX normal".into()));
                    }
                    Some(n.normalize().to_array())
                } else {
                    None
                };
                if face.len() >= super::MAX_FACE_CORNERS {
                    return Err(Error(format!("FBX polygon exceeds the {}-corner import limit", super::MAX_FACE_CORNERS)));
                }
                face.push(Corner { vertex: vertex + offset, uv: [u, v], normal });
                if raw < 0 {
                    let slot = material_indices.get(if all_same { 0 } else { polygon }).copied().unwrap_or(0);
                    let material = usize::try_from(slot).ok().and_then(|i| attached.get(i)).copied().unwrap_or(default_material);
                    if reflected {
                        face.reverse();
                    }
                    if super::triangulate(&scene.positions, &face, material, &mut scene.triangles, &mut triangulation_budget)? && scene.warnings.len() < 100 {
                        scene.warnings.push("FBX: redundant or degenerate polygon corners omitted during triangulation".into());
                    }
                    face.clear();
                    polygon += 1;
                }
            }
            if !face.is_empty() {
                return Err(Error("Unterminated FBX polygon".into()));
            }
        }
        if objects.children_by_name("Deformer").next().is_some() {
            warned_skin = true;
        }
    }
    if warned_skin {
        scene.warnings.push("Skinning and animation are not evaluated; the original static mesh is shown".into());
    }
    scene.warnings.push("GPU Material supports Color, Normal, Metallic, Roughness, AO and Emissive; height/bump is not evaluated".into());
    scene.warnings.sort();
    scene.warnings.dedup();
    Ok(scene)
}

#[cfg(test)]
mod preflight_tests {
    use super::*;

    fn file_with_attributes(attribute_count: u32, declared_attribute_bytes: u32, attributes: &[u8]) -> Vec<u8> {
        const NODE_HEADER: usize = 13;
        const END_MARKER: usize = NODE_HEADER;
        let node_start = FBX_HEADER_LEN;
        let node_end = node_start + NODE_HEADER + 1 + declared_attribute_bytes as usize;
        let mut bytes = b"Kaydara FBX Binary  \x00\x1a\x00".to_vec();
        bytes.extend_from_slice(&7400_u32.to_le_bytes());
        bytes.extend_from_slice(&(node_end as u32).to_le_bytes());
        bytes.extend_from_slice(&attribute_count.to_le_bytes());
        bytes.extend_from_slice(&declared_attribute_bytes.to_le_bytes());
        bytes.push(1); // name length
        bytes.push(b'N');
        bytes.extend_from_slice(attributes);
        bytes.resize(bytes.len() + END_MARKER, 0); // root end marker
        bytes
    }

    fn nested_file(depth: usize) -> Vec<u8> {
        fn append_node(bytes: &mut Vec<u8>, descendants: usize) {
            let start = bytes.len();
            bytes.resize(start + 14, 0); // 32-bit node header plus one-byte name
            bytes[start + 13] = 1;
            bytes.push(b'N');
            if descendants != 0 {
                append_node(bytes, descendants - 1);
            }
            bytes.resize(bytes.len() + 13, 0); // required node end marker
            let end = bytes.len() as u32;
            bytes[start..start + 4].copy_from_slice(&end.to_le_bytes());
        }

        let mut bytes = b"Kaydara FBX Binary  \x00\x1a\x00".to_vec();
        bytes.extend_from_slice(&7400_u32.to_le_bytes());
        append_node(&mut bytes, depth.saturating_sub(1));
        bytes.resize(bytes.len() + 13, 0); // implicit root end marker
        bytes
    }

    #[test]
    fn bounded_preflight_accepts_normal_scalar_attributes() {
        let mut attributes = vec![b'I'];
        attributes.extend_from_slice(&17_i32.to_le_bytes());
        assert!(preflight_binary(&file_with_attributes(1, attributes.len() as u32, &attributes)).is_ok());
    }

    #[test]
    fn bounded_preflight_rejects_huge_declared_arrays_before_tree_loading() {
        let mut attributes = vec![b'd'];
        attributes.extend_from_slice(&u32::MAX.to_le_bytes());
        attributes.extend_from_slice(&1_u32.to_le_bytes()); // zlib
        attributes.extend_from_slice(&1_u32.to_le_bytes());
        attributes.push(0x78);
        assert!(preflight_binary(&file_with_attributes(1, attributes.len() as u32, &attributes)).is_err());
    }

    #[test]
    fn bounded_preflight_rejects_unknown_codecs_and_truncated_payloads() {
        let mut unknown_codec = vec![b'i'];
        unknown_codec.extend_from_slice(&1_u32.to_le_bytes());
        unknown_codec.extend_from_slice(&9_u32.to_le_bytes());
        unknown_codec.extend_from_slice(&4_u32.to_le_bytes());
        unknown_codec.extend_from_slice(&[0; 4]);
        assert!(preflight_binary(&file_with_attributes(1, unknown_codec.len() as u32, &unknown_codec)).is_err());

        let mut truncated = vec![b'f'];
        truncated.extend_from_slice(&1_u32.to_le_bytes());
        truncated.extend_from_slice(&0_u32.to_le_bytes()); // direct
        truncated.extend_from_slice(&4_u32.to_le_bytes());
        truncated.push(0);
        assert!(preflight_binary(&file_with_attributes(1, truncated.len() as u32, &truncated)).is_err());
    }

    #[test]
    fn bounded_preflight_rejects_excessive_nesting() {
        assert!(preflight_binary(&nested_file(FBX_MAX_DEPTH + 1)).is_err());
    }
}
