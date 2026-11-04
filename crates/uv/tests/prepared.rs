use photocraft_uv::{
    import,
    prepared::Geometry,
    preview::{self, Camera, Image, Materials},
    topology::Topology,
};
use std::collections::BTreeSet;

#[test]
fn bvh_picking_matches_depth_rasterizer_after_navigation_and_visibility_changes() {
    for file in ["two-shells.fbx", "normals-reflected.fbx"] {
        let scene = import::load(file, &std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(file)).unwrap(), 0, |_| {
            Err(photocraft_uv::Error("absent".into()))
        })
        .unwrap();
        let geometry = Geometry::new(&scene, &Topology::build(&scene).unwrap()).unwrap();
        for step in 0..6 {
            let camera = Camera {
                yaw: 0.3 + f64::from(step) * 0.4,
                pitch: 0.4,
                pan: [0.03, 0.01],
                target: Some(scene.positions[0]),
                zoom: 1.25,
                flip_faces: step % 2 == 1,
                ..Default::default()
            };
            let maps = Materials { hidden: if step == 4 { BTreeSet::from([0]) } else { BTreeSet::new() }, ..Default::default() };
            let frame = preview::render_materials(&scene, camera, &maps, &[], 173, 119).unwrap();
            let mut compared = 0;
            for y in (2..117).step_by(5) {
                for x in (2..171).step_by(5) {
                    let expected = frame.triangles[y * 173 + x];
                    let actual = geometry.pick(&scene, camera, &maps, [(x as f64 + 0.5) / 173.0, (y as f64 + 0.5) / 119.0], [173, 119]);
                    assert_eq!(actual, expected, "{file} step={step} pixel={x},{y}");
                    compared += 1;
                }
            }
            assert!(compared > 500);
        }
    }
}
#[test]
fn alpha_cutouts_do_not_occlude_picking_and_invalid_topology_is_rejected() {
    let scene = import::load("test.obj", include_bytes!("fixtures/two-shells.obj"), 0, |_| Err(photocraft_uv::Error("absent".into()))).unwrap();
    let geometry = Geometry::new(&scene, &Topology::build(&scene).unwrap()).unwrap();
    assert!(Geometry::new(&scene, &Topology::default()).is_err());
    let colors: Vec<_> = scene.materials.iter().map(|_| Some(Image { width: 1, height: 1, rgba: vec![[1.0, 0.0, 0.0, 0.0]] })).collect();
    let maps = Materials { colors: &colors, ..Default::default() };
    for y in 0..20 {
        for x in 0..20 {
            assert!(geometry.pick(&scene, Camera::default(), &maps, [f64::from(x) / 20.0, f64::from(y) / 20.0], [200, 200]).is_none());
        }
    }
}

#[test]
fn surface_hit_chooses_nearest_visible_layer_and_retarget_preserves_projection() {
    use photocraft_uv::{Corner, Material, Scene, Triangle};
    let mut scene = Scene { materials: vec![Material::default(); 2], ..Default::default() };
    for (layer, z) in [0.0, 0.4].into_iter().enumerate() {
        scene.positions.extend([[-1.0, -1.0, z], [1.0, -1.0, z], [0.0, 1.0, z]]);
        scene.triangles.push(Triangle { material: layer, corners: std::array::from_fn(|i| Corner { vertex: layer * 3 + i, uv: [0.5, 0.5], normal: None }) });
    }
    let geometry = Geometry::new(&scene, &Topology::build(&scene).unwrap()).unwrap();
    let camera = Camera { yaw: 0.0, pitch: 0.0, zoom: 3.0, pan: [0.15, 0.1], ..Default::default() };
    let size = [800, 600];
    let maps = Materials::default();
    let hit = geometry.hit(&scene, camera, &maps, [0.5, 0.5], size).unwrap();
    assert_eq!(hit.triangle, 1);
    assert!((hit.position[2] - 0.4).abs() < 1e-6);
    let hidden = Materials { hidden: BTreeSet::from([1]), ..Default::default() };
    assert_eq!(geometry.hit(&scene, camera, &hidden, [0.5, 0.5], size).unwrap().triangle, 0);
    let colors = vec![None, Some(Image { width: 1, height: 1, rgba: vec![[1.0, 1.0, 1.0, 0.0]] })];
    assert_eq!(geometry.hit(&scene, camera, &Materials { colors: &colors, ..Default::default() }, [0.5, 0.5], size).unwrap().triangle, 0);
    assert!(geometry.hit(&scene, Camera { flip_faces: true, ..camera }, &maps, [0.5, 0.5], size).is_none());
    // World coordinates, rather than normalized mesh coordinates, including large offsets.
    for camera in [camera, Camera { yaw: 0.9, pitch: 0.45, target: Some([0.1, 0.2, 0.3]), ..camera }] {
        let next = geometry.retarget(camera, hit.position);
        let before = geometry.camera_values(camera, size);
        let after = geometry.camera_values(next, size);
        for i in [4, 5, 6, 7] {
            assert!((before[i] - after[i]).abs() < 1e-6);
        }
        assert_eq!(next.target, Some(hit.position));
        let rotated = Camera { yaw: next.yaw + 0.8, pitch: next.pitch - 0.3, ..next };
        let project = |cam: Camera| {
            let p =
                photocraft_uv::prepared::rotate((glam::DVec3::from_array(hit.position) - glam::DVec3::from_array(geometry.center)) / geometry.diameter, cam);
            let c = geometry.camera_values(cam, size);
            [(p.x + f64::from(c[6])) * f64::from(c[4]), (p.y + f64::from(c[7])) * f64::from(c[5])]
        };
        for (a, b) in project(next).into_iter().zip(project(rotated)) {
            assert!((a - b).abs() < 1e-6, "orbit pivot must stay stationary");
        }
    }
    assert_eq!(geometry.retarget(camera, [f64::NAN, 0.0, 0.0]), camera);
}

#[test]
#[ignore = "local model diagnostic; set PHOTOCRAFT_UV_PIVOT_MODEL"]
fn orbit_surface_picking_on_external_model() {
    let path = std::env::var("PHOTOCRAFT_UV_PIVOT_MODEL").expect("set diagnostic source model");
    let scene = import::load(&path, &std::fs::read(&path).unwrap(), 0, |_| Err(photocraft_uv::Error("diagnostic uses geometry only".into()))).unwrap();
    let geometry = Geometry::new(&scene, &Topology::build(&scene).unwrap()).unwrap();
    let t = &geometry.triangles[0];
    let target = std::array::from_fn(|axis| geometry.center[axis] + geometry.diameter * t.positions.iter().map(|p| f64::from(p[axis])).sum::<f64>() / 3.0);
    let maps = Materials::default();
    let start = std::time::Instant::now();
    let mut hits = 0;
    for i in 0..10000 {
        let camera = Camera { yaw: f64::from(i % 100) * 0.02, pitch: 0.3, target: Some(target), zoom: 8.0, ..Default::default() };
        if let Some(hit) = geometry.hit(&scene, camera, &maps, [0.5, 0.5], [2195, 1318]) {
            let next = geometry.retarget(camera, hit.position);
            let a = geometry.camera_values(camera, [2195, 1318]);
            let b = geometry.camera_values(next, [2195, 1318]);
            assert!((a[6] - b[6]).abs() < 1e-6 && (a[7] - b[7]).abs() < 1e-6);
            hits += 1;
        }
    }
    println!(
        "{} triangles, {hits}/10000 hits; BVH hit + retarget mean {:.3} microseconds (CPU geometry only)",
        scene.triangles.len(),
        start.elapsed().as_secs_f64() * 100.0
    );
    assert!(hits > 5000);
}

#[test]
fn perspective_raster_and_ray_hits_agree_including_near_clipping_and_retarget() {
    let scene = import::load("test.obj", include_bytes!("fixtures/two-shells.obj"), 0, |_| Err(photocraft_uv::Error("unused".into()))).unwrap();
    let g = Geometry::new(&scene, &Topology::build(&scene).unwrap()).unwrap();
    let maps = Materials::default();
    let mut hits = 0;
    for zoom in [1.0, 4.0, 10.0, 20.0] {
        for yaw in [0.0, 0.5, 1.0] {
            let camera = Camera { perspective: true, yaw, pitch: 0.3, zoom, ..Default::default() };
            let size = [173, 119];
            let frame = preview::render_materials(&scene, camera, &maps, &[], 173, 119).unwrap();
            for y in (3..115).step_by(5) {
                for x in (3..169).step_by(5) {
                    let at = [(x as f64 + 0.5) / 173.0, (y as f64 + 0.5) / 119.0];
                    let hit = g.hit(&scene, camera, &maps, at, size);
                    assert_eq!(hit.map(|h| h.triangle), frame.triangles[y * 173 + x], "perspective zoom{zoom} yaw{yaw} at{x},{y}");
                    if let Some(hit) = hit {
                        hits += 1;
                        let next = g.retarget(camera, hit.position);
                        for p in &g.triangles[hit.triangle].positions {
                            if let (Some(a), Some(b)) =
                                (g.project_normalized(camera, p.map(f64::from), size), g.project_normalized(next, p.map(f64::from), size))
                            {
                                for i in 0..2 {
                                    assert!((a[i] - b[i]).abs() < 0.002, "retarget must preserve perspective projection");
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(hits > 1000);
}
