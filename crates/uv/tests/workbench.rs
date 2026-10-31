use photocraft_uv::{
    preview::{self, Camera},
    topology::{MaskOptions, Topology},
    *,
};
use std::collections::BTreeSet;

fn fixture(name: &str) -> Scene {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    import::load(name, &std::fs::read(root.join(name)).unwrap(), 0, |name| std::fs::read(root.join(name)).map_err(|e| Error(e.to_string()))).unwrap()
}
fn options(material: usize, shell: usize) -> MaskOptions {
    MaskOptions {
        width: 128,
        height: 128,
        material,
        tile: [0, 0],
        selected: BTreeSet::from([shell]),
        guard: 8.0,
        horizontal: 0.0,
        vertical: 0.0,
        feather: 0.0,
        open_borders: false,
        padding: 0.0,
    }
}

#[test]
fn outward_padding_fills_only_empty_space_around_the_selected_shell() {
    let scene = fixture("two-shells.obj");
    let topology = Topology::build(&scene).unwrap();
    let shell = topology.pick_uv(&scene, [0.2, 0.5], topology.shells[0].material)[0];
    let material = topology.shells[shell].material;
    let occupancy = topology.occupancy_mask(&scene, 128, 128, material, [0, 0]).unwrap();
    let mut padding_options = options(material, shell);
    padding_options.padding = 3.0;
    let padding = topology.outward_padding_mask(&padding_options, shell, &occupancy).unwrap();
    let mut options = options(material, shell);
    options.guard = 0.0;
    options.padding = 3.0;
    let mask = topology.mask_with_occupancy(&scene, &options, &occupancy).unwrap();
    let mut added = 0usize;
    padding.for_each(|x, y| {
        assert!((0..128).contains(&x) && (0..128).contains(&y));
        assert!(!occupancy.contains(x, y), "padding crossed onto occupied UV geometry at {x},{y}");
        assert_eq!(mask[y as usize * 128 + x as usize], 1.0);
        added += 1;
    });
    assert!(added > 0, "the shell should receive an exterior apron");
}

#[test]
fn fbx_and_obj_preserve_two_connected_shells_and_material_texture() {
    for name in ["two-shells.fbx", "two-shells.obj"] {
        let scene = fixture(name);
        assert_eq!(scene.triangles.len(), 4, "{name}");
        let t = Topology::build(&scene).unwrap();
        assert_eq!(t.shells.len(), 2, "{name}");
        assert_eq!(t.shells[0].neighbors, BTreeSet::from([1]), "{name}");
        assert_eq!(t.shells[1].neighbors, BTreeSet::from([0]), "{name}");
        assert_eq!(t.boundaries.iter().filter(|e| e.shared).count(), 2, "{name}");
        assert!(scene.materials.iter().any(|m| m.base_color.as_deref().is_some_and(|s| s.ends_with("checker.png"))), "{name}");
    }
}

#[test]
fn shell_pick_uses_uv_geometry_not_texture_color() {
    let scene = fixture("two-shells.fbx");
    let t = Topology::build(&scene).unwrap();
    let material = t.shells[0].material;
    assert_eq!(t.pick_uv(&scene, [0.2, 0.5], material).len(), 1);
    assert_eq!(t.pick_uv(&scene, [0.75, 0.5], material).len(), 1);
    assert!(t.pick_uv(&scene, [0.5, 0.5], material).is_empty());
}

#[test]
fn guard_protects_only_shared_3d_seam_and_feathers_inward() {
    let scene = fixture("two-shells.obj");
    let t = Topology::build(&scene).unwrap();
    let shell = t.pick_uv(&scene, [0.2, 0.5], t.shells[0].material)[0];
    let mut o = options(t.shells[shell].material, shell);
    let mask = t.mask(&scene, &o).unwrap();
    assert_eq!(mask[64 * 128 + 52], 0.0); // seam at x=53.76, inside its guard.
    assert_eq!(mask[64 * 128 + 30], 1.0);
    assert_eq!(mask[64 * 128 + 12], 1.0); // the open outer border is intentionally not protected.
    assert_eq!(mask[64 * 128 + 90], 0.0); // other UV shell.
    o.feather = 8.0;
    let mask = t.mask(&scene, &o).unwrap();
    assert_eq!(mask[64 * 128 + 52], 0.0);
    assert!(mask[64 * 128 + 42] > 0.0 && mask[64 * 128 + 42] < 1.0);
    o.open_borders = true;
    assert_eq!(t.mask(&scene, &o).unwrap()[64 * 128 + 12], 0.0);
}

#[test]
fn all_selected_islands_still_protect_both_sides_of_seam() {
    let scene = fixture("two-shells.obj");
    let t = Topology::build(&scene).unwrap();
    let mut o = options(t.shells[0].material, 0);
    o.selected = BTreeSet::from([0, 1]);
    let mask = t.mask(&scene, &o).unwrap();
    assert_eq!(mask[64 * 128 + 52], 0.0);
    assert_eq!(mask[64 * 128 + 75], 0.0);
    assert_eq!(mask[64 * 128 + 30], 1.0);
    assert_eq!(mask[64 * 128 + 95], 1.0);
}

#[test]
fn coincident_disconnected_vertices_are_not_automatically_welded() {
    let mut scene = fixture("two-shells.obj");
    let offset = scene.positions.len();
    scene.positions.extend(scene.positions.clone());
    let copy: Vec<Triangle> = scene
        .triangles
        .iter()
        .cloned()
        .map(|mut t| {
            for c in &mut t.corners {
                c.vertex += offset;
            }
            t
        })
        .collect();
    scene.triangles.extend(copy);
    let t = Topology::build(&scene).unwrap();
    assert_eq!(t.shells.len(), 4);
    assert_eq!(t.pick_uv(&scene, [0.2, 0.5], t.shells[0].material).len(), 2);
    assert!(t.shells.iter().all(|s| s.neighbors.len() == 1));
}

#[test]
fn non_manifold_edges_are_reported() {
    let mut scene = fixture("two-shells.obj");
    scene.triangles.push(scene.triangles[0].clone());
    assert!(Topology::build(&scene).unwrap().non_manifold_edges > 0);
}

#[test]
fn mask_limits_and_empty_selection_fail_closed() {
    let scene = fixture("two-shells.obj");
    let t = Topology::build(&scene).unwrap();
    let mut o = options(t.shells[0].material, 0);
    o.selected.clear();
    assert!(t.mask(&scene, &o).unwrap().iter().all(|v| *v == 0.0));
    o.width = u32::MAX;
    assert!(t.mask(&scene, &o).is_err());
    o.width = 128;
    o.guard = f64::NAN;
    assert!(t.mask(&scene, &o).is_err());
    o.guard = 8.0;
    o.selected.insert(usize::MAX);
    assert!(t.mask(&scene, &o).is_err());
}

#[test]
fn nonsquare_texture_guard_is_in_real_pixels() {
    let scene = fixture("two-shells.obj");
    let t = Topology::build(&scene).unwrap();
    let mut o = options(t.shells[0].material, 0);
    o.width = 256;
    let mask = t.mask(&scene, &o).unwrap();
    assert_eq!(mask[64 * 256 + 102], 0.0);
    assert_eq!(mask[64 * 256 + 92], 1.0);
}

#[test]
fn malformed_models_return_errors() {
    for (name, bytes) in [
        ("bad.fbx", b"Kaydara FBX Binary".as_slice()),
        ("bad.obj", b"v 0 0 0\nf 1/1 7/1 8/1"),
        ("ascii.fbx", b"; FBX 7.4"),
        ("bad.obj", b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3"),
    ] {
        assert!(import::load(name, bytes, 0, |_| Err(Error("missing".into()))).is_err());
    }
}

#[test]
fn reference_preview_picks_front_triangle_and_handles_empty_textures() {
    let scene = fixture("two-shells.obj");
    let frame = preview::render(&scene, Camera::default(), &[], &[], 128, 128).unwrap();
    assert_eq!(frame.rgba.len(), 128 * 128);
    assert!(frame.triangles.iter().any(Option::is_some));
    let camera = Camera { yaw: f64::NAN, ..Camera::default() };
    assert!(preview::render(&scene, camera, &[], &[], 128, 128).is_err());
}

#[test]
fn depth_pick_is_independent_of_triangle_submission_order() {
    for reversed in [false, true] {
        let mut scene = Scene {
            positions: vec![[-1.0, -1.0, 0.0], [1.0, -1.0, 0.0], [0.0, 1.0, 0.0], [-1.0, -1.0, 1.0], [1.0, -1.0, 1.0], [0.0, 1.0, 1.0]],
            materials: vec![Material::default()],
            ..Scene::default()
        };
        let triangle = |base| Triangle {
            material: 0,
            corners: [
                Corner { vertex: base, uv: [0.0, 0.0], normal: None },
                Corner { vertex: base + 1, uv: [1.0, 0.0], normal: None },
                Corner { vertex: base + 2, uv: [0.5, 1.0], normal: None },
            ],
        };
        scene.triangles = if reversed { vec![triangle(3), triangle(0)] } else { vec![triangle(0), triangle(3)] };
        let camera = Camera { yaw: 0.0, pitch: 0.0, ..Camera::default() };
        let frame = preview::render(&scene, camera, &[], &[], 128, 128).unwrap();
        assert_eq!(frame.triangles[64 * 128 + 64], Some(if reversed { 0 } else { 1 }));
    }
}

#[test]
fn embedded_fbx_diffuse_texture_is_available_without_external_files() {
    let scene = fixture("two-shells-embedded.fbx");
    assert!(scene.materials.iter().any(|m| m.embedded.as_deref().is_some_and(|bytes| bytes.starts_with(b"\x89PNG\r\n\x1a\n"))));
    assert_eq!(Topology::build(&scene).unwrap().shells.len(), 2);
}

#[test]
fn triangulation_preserves_winding_in_every_projection_plane_and_obj_normals() {
    for axis in 0..3 {
        for reversed in [false, true] {
            let vertices: Vec<[f64; 3]> = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]]
                .map(|p| match axis {
                    0 => [0.0, p[0], p[1]],
                    1 => [p[0], 0.0, p[1]],
                    _ => [p[0], p[1], 0.0],
                })
                .to_vec();
            let mut text = String::new();
            for p in &vertices {
                text.push_str(&format!("v {} {} {}\n", p[0], p[1], p[2]));
            }
            text.push_str("vt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nvn 0 0 1\n");
            text.push_str(if reversed { "f 4/4/1 3/3/1 2/2/1 1/1/1\n" } else { "f 1/1/1 2/2/1 3/3/1 4/4/1\n" });
            let scene = photocraft_uv::import::load("quad.obj", text.as_bytes(), 0, |_| unreachable!()).unwrap();
            let expected = (glam::DVec3::from_array(vertices[1]) - glam::DVec3::from_array(vertices[0]))
                .cross(glam::DVec3::from_array(vertices[2]) - glam::DVec3::from_array(vertices[0]))
                * if reversed { -1.0 } else { 1.0 };
            for triangle in &scene.triangles {
                let [a, b, c] = triangle.corners.map(|c| glam::DVec3::from_array(scene.positions[c.vertex]));
                assert!((b - a).cross(c - a).dot(expected) > 0.0);
                assert!(triangle.corners.iter().all(|c| c.normal == Some([0.0, 0.0, 1.0])));
            }
        }
    }
    assert!(fixture("two-shells.fbx").triangles.iter().flat_map(|t| t.corners).any(|c| c.normal.is_some()));
}

#[test]
fn normals_normal_maps_and_hidden_materials_affect_the_visible_surface() {
    use photocraft_uv::preview::{Image, Materials};
    let mut scene =
        photocraft_uv::import::load("face.obj", b"v -1 -1 0\nv 1 -1 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt .5 1\nf 1/1 2/2 3/3\n", 0, |_| unreachable!()).unwrap();
    let camera = Camera { yaw: 0.0, pitch: 0.0, shaded: true, ..Camera::default() };
    let draw = |scene: &Scene, camera: Camera, maps: &Materials<'_>| preview::render_materials(scene, camera, maps, &[], 64, 64).unwrap();
    let center = 32 * 64 + 32;
    let baseline = draw(&scene, camera, &Materials::default());
    assert_eq!(baseline.triangles[center], Some(0));
    let mut maps = Materials::default();
    maps.hidden.insert(0);
    assert!(draw(&scene, camera, &maps).triangles.iter().all(Option::is_none));
    scene.triangles[0].corners.swap(1, 2);
    assert!(draw(&scene, camera, &Materials::default()).triangles.iter().all(Option::is_none));
    assert!(draw(&scene, Camera { flip_faces: true, ..camera }, &Materials::default()).triangles[center].is_some());
    scene.triangles[0].corners.swap(1, 2);
    let flat = vec![Some(Image { width: 1, height: 1, rgba: vec![[0.5, 0.5, 1.0, 1.0]] })];
    assert_eq!(baseline.rgba, draw(&scene, camera, &Materials { normals: &flat, ..Default::default() }).rgba);
    let green = vec![Some(Image { width: 1, height: 1, rgba: vec![[0.5, 0.85, 0.85, 1.0]] })];
    let maps = Materials { normals: &green, ..Default::default() };
    assert!(draw(&scene, camera, &maps).rgba[center][0] > draw(&scene, Camera { flip_normal_y: true, ..camera }, &maps).rgba[center][0]);
    for corner in &mut scene.triangles[0].corners {
        corner.normal = Some([0.0, -1.0, 0.1]);
    }
    assert!(draw(&scene, camera, &Materials::default()).rgba[center][0] < baseline.rgba[center][0]);
    assert_eq!(baseline.rgba, draw(&scene, Camera { authored_normals: false, ..camera }, &Materials::default()).rgba);
}

#[test]
fn reflected_nonuniform_fbx_preserves_outward_normals_and_normal_texture() {
    let scene = fixture("normals-reflected.fbx");
    assert!(scene.materials.iter().any(|m| m.normal_map.as_deref().is_some_and(|s| s.ends_with("normal.png"))));
    let center = scene.positions.iter().map(|p| glam::DVec3::from_array(*p)).sum::<glam::DVec3>() / scene.positions.len() as f64;
    for triangle in &scene.triangles {
        let [a, b, c] = triangle.corners.map(|c| glam::DVec3::from_array(scene.positions[c.vertex]));
        let face = (b - a).cross(c - a).normalize();
        assert!(face.dot((a + b + c) / 3.0 - center) > 0.0, "reflected winding must remain outward");
        for corner in triangle.corners {
            let normal = glam::DVec3::from_array(corner.normal.unwrap());
            assert!(normal.dot(face) > 0.6, "inverse-transposed normal must match the surface");
            assert!((normal.length() - 1.0).abs() < 1e-8);
        }
    }
}

#[test]
fn obj_bump_height_map_is_not_misinterpreted_as_a_normal_map() {
    let obj = b"mtllib test.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nusemtl Surface\nf 1/1 2/2 3/3\n";
    let scene = import::load("test.obj", obj, 0, |_| Ok(b"newmtl Surface\nmap_Bump height.png\n".to_vec())).unwrap();
    assert!(scene.materials.iter().all(|m| m.normal_map.is_none()));
    assert!(scene.warnings.iter().any(|w| w.contains("bump/height")));
    let scene = import::load("test.obj", obj, 0, |_| Ok(b"newmtl Surface\nnorm surface-normal.png\n".to_vec())).unwrap();
    assert!(scene.materials.iter().any(|m| m.normal_map.as_deref() == Some("surface-normal.png")));
}

#[test]
fn default_camera_sees_the_front_of_y_up_fbx_and_z_up_obj() {
    for name in ["two-shells.fbx", "two-shells.obj"] {
        let scene = fixture(name);
        let frame = preview::render(&scene, Camera::default(), &[], &[], 128, 128).unwrap();
        assert!(frame.triangles.iter().any(Option::is_some), "default camera must be above the surface: {name}");
    }
}

#[test]
fn high_resolution_preview_and_camera_target_keep_picking_and_limits() {
    let scene = fixture("two-shells.obj");
    let frame = preview::render(&scene, Camera::default(), &[], &[], 1920, 1080).unwrap();
    assert_eq!(frame.rgba.len(), 1920 * 1080);
    assert!(frame.triangles.iter().any(Option::is_some));
    assert!(preview::render(&scene, Camera::default(), &[], &[], 4097, 4096).is_err());
    assert!(preview::render(&scene, Camera { target: Some([f64::NAN, 0.0, 0.0]), ..Default::default() }, &[], &[], 10, 10).is_err());
    let shifted = preview::render(&scene, Camera { target: Some([1000.0; 3]), ..Default::default() }, &[], &[], 64, 64).unwrap();
    assert!(shifted.triangles.iter().all(Option::is_none));
}

#[test]
fn single_map_views_ignore_lighting_material_tint_and_normal_strength() {
    use photocraft_uv::preview::{Channel, Image, Materials};
    let mut scene =
        photocraft_uv::import::load("face.obj", b"v -1 -1 0\nv 1 -1 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt .5 1\nf 1/1 2/2 3/3\n", 0, |_| unreachable!()).unwrap();
    scene.materials[0].color = [0.1, 0.3, 0.6, 1.0];
    let colors = [Some(Image { width: 1, height: 1, rgba: vec![[0.8, 0.4, 0.2, 1.0]] })];
    let normals = [Some(Image { width: 1, height: 1, rgba: vec![[0.25, 0.75, 1.0, 1.0]] })];
    let maps = Materials { colors: &colors, normals: &normals, ..Default::default() };
    for perspective in [false, true] {
        let camera = Camera { perspective, yaw: 0.0, pitch: 0.0, shaded: true, normal_strength: 4.0, flip_normal_y: true, ..Default::default() };
        for (channel, expected) in [(Channel::Color, [204, 102, 51, 255]), (Channel::Normal, [64, 191, 255, 255])] {
            let frame = preview::render_materials(&scene, Camera { channel, ..camera }, &maps, &[], 64, 64).unwrap();
            assert_eq!(frame.rgba[32 * 64 + 32], expected, "{channel:?}, perspective={perspective}");
        }
        let frame = preview::render_materials(&scene, camera, &maps, &[], 64, 64).unwrap();
        assert!(frame.rgba[32 * 64 + 32][0] < 30, "Material must retain tint and lighting");
    }
}

#[test]
fn directional_margins_follow_texture_axes_and_add_to_the_general_margin() {
    let mut scene = fixture("two-shells.obj");
    for rotated in [false, true] {
        let t = Topology::build(&scene).unwrap();
        let mut o = options(t.shells[0].material, 0);
        o.guard = 0.0;
        o.horizontal = 8.0;
        let index = if rotated { 52 * 128 + 64 } else { 64 * 128 + 52 };
        let horizontal = t.mask(&scene, &o).unwrap()[index];
        o.horizontal = 0.0;
        o.vertical = 8.0;
        let vertical = t.mask(&scene, &o).unwrap()[index];
        assert_eq!((horizontal, vertical), if rotated { (1.0, 0.0) } else { (0.0, 1.0) });
        for triangle in &mut scene.triangles {
            for c in &mut triangle.corners {
                c.uv = [c.uv[1], 1.0 - c.uv[0]];
            }
        }
    }
    let scene = fixture("two-shells.obj");
    let t = Topology::build(&scene).unwrap();
    let mut o = options(t.shells[0].material, 0);
    o.guard = 4.0;
    assert_eq!(t.mask(&scene, &o).unwrap()[64 * 128 + 47], 1.0);
    o.horizontal = 4.0;
    assert_eq!(t.mask(&scene, &o).unwrap()[64 * 128 + 47], 0.0);
    o.horizontal = f64::NAN;
    assert!(t.mask(&scene, &o).is_err());
}

#[test]
fn redundant_polygon_corners_do_not_abort_valid_meshes() {
    let obj = b"v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nf 1/1 2/2 2/2 3/3 4/4 1/1\nf 1/1 1/1 1/1\n";
    let scene = import::load("redundant.obj", obj, 0, |_| Err(Error("no files".into()))).unwrap();
    assert_eq!(scene.triangles.len(), 2);
    assert!(!scene.warnings.is_empty());
}
#[test]
fn tangent_chart_crosses_a_fold_and_uv_seam_without_touching_disconnected_faces() {
    let scene = fixture("two-shells.obj");
    let mesh = wrap::Mesh::new(&scene).unwrap();
    let start = scene
        .triangles
        .iter()
        .position(|t| {
            t.corners.iter().all(|c| c.vertex < 4)
                && topology::barycentric(
                    [-0.02, 0.0],
                    t.corners.map(|c| {
                        let p = scene.positions[c.vertex];
                        [p[0], p[1]]
                    }),
                )
                .is_some()
        })
        .unwrap();
    let patches = mesh.chart(&scene, start, [-0.02, 0.0, 0.0], [128, 128], 12.0).unwrap();
    let mut left = 0;
    let mut right = 0;
    let mut outside = 0;
    for patch in patches {
        patch
            .rasterize(&scene, [128, 128], 12.0, |x, _, p| {
                if p[0].hypot(p[1]) < 10.0 {
                    if x < 55 {
                        left += 1;
                    } else if x > 73 {
                        right += 1;
                    } else {
                        outside += 1;
                    }
                }
            })
            .unwrap();
    }
    assert!(left > 0 && right > 0, "wrap should reach both disjoint atlas islands");
    assert_eq!(outside, 0, "the empty UV gap is never painted");
}

#[test]
fn tangent_chart_emits_only_cached_shell_apron_pixels_outside_its_uv_triangles() {
    let scene = fixture("two-shells.obj");
    let topology = Topology::build(&scene).unwrap();
    let material = topology.shells[0].material;
    let occupancy = topology.occupancy_mask(&scene, 128, 128, material, [0, 0]).unwrap();
    let mesh = wrap::Mesh::new(&scene).unwrap();
    let start = scene
        .triangles
        .iter()
        .position(|t| {
            t.corners.iter().all(|c| c.vertex < 4)
                && topology::barycentric(
                    [-0.02, 0.0],
                    t.corners.map(|c| {
                        let p = scene.positions[c.vertex];
                        [p[0], p[1]]
                    }),
                )
                .is_some()
        })
        .unwrap();
    let shell = topology.triangle_shell[start];
    let mut padding_options = options(material, shell);
    padding_options.padding = 3.0;
    let apron = topology.outward_padding_mask(&padding_options, shell, &occupancy).unwrap();
    let patches = mesh.chart(&scene, start, [-0.02, 0.0, 0.0], [128, 128], 12.0).unwrap();
    let (mut base, mut padded) = (BTreeSet::new(), BTreeSet::new());
    for patch in patches {
        patch
            .rasterize(&scene, [128, 128], 12.0, |x, y, _| {
                base.insert((x, y));
            })
            .unwrap();
        patch
            .rasterize_with_padding(&scene, [128, 128], 12.0, Some(&apron), 3.0, |x, y, _| {
                padded.insert((x, y));
            })
            .unwrap();
    }
    let added: Vec<_> = padded.difference(&base).copied().collect();
    assert!(!added.is_empty(), "wrapped brush coverage should extend over the shell border");
    for (x, y) in added {
        assert!(apron.contains(x, y));
        assert!(!occupancy.contains(x, y), "wrap apron touched neighboring UV geometry");
    }
}
