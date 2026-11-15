//! #13774: Paths to Illustrator lists and exports the active layer's shape path or vector mask
//! after the Work Path, by its Paths panel name, and includes it in All Paths.

use super::*;
use photocraft_doc::{NamedPath, Path, Subpath, VectorMask};

const BEGIN: &str = "%Adobe_Photoshop_Path_Begin";

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Subpath {
    Subpath::polygon(&[(x0, y0), (x1, y0), (x1, y1), (x0, y1)])
}

/// 300 x 200 at 72 ppi with saved paths `Rect A` and `Curve B` (two subpaths) and a Work Path.
fn session(work: bool) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 300, "height": 200, "name": "Layers.psd"})).unwrap();
    s.edit("paths", |d, _| {
        d.resolution_dpi = 72.0;
        d.paths.push(NamedPath { name: "Rect A".into(), path: Path::new(vec![rect(10.0, 10.0, 50.0, 40.0)]), psd_raw: None });
        d.paths.push(NamedPath { name: "Curve B".into(), path: Path::new(vec![rect(60.0, 10.0, 90.0, 40.0), rect(100.0, 10.0, 120.0, 40.0)]), psd_raw: None });
        if work {
            d.work_path = Some(Path::new(vec![rect(130.0, 10.0, 160.0, 40.0)]));
        }
        Ok(())
    })
    .unwrap();
    s
}

fn export(s: &mut Session, which: &str) -> std::result::Result<(String, u64), String> {
    let r = s.execute("file.export.pathsToIllustrator", json!({"paths": which})).map_err(|e| e.to_string())?;
    Ok((r["ai"].as_str().unwrap().replace('\r', "\n"), r["paths"].as_u64().unwrap()))
}

/// The x of each path block's first `m` (moveto), in file order.
fn first_points(ai: &str) -> Vec<f64> {
    ai.split(BEGIN).skip(1).map(|b| b.lines().find(|l| l.ends_with(" m")).unwrap().split(' ').next().unwrap().parse().unwrap()).collect()
}

#[test]
fn a_shape_layer_path_is_the_last_entry_and_exports_alone() {
    let mut s = session(true);
    s.execute("shape.create", json!({"kind": "rectangle", "rect": [200, 120, 60, 50]})).unwrap();
    let doc = s.active().unwrap().doc.clone();
    let active = s.active().unwrap().active_layer;
    assert_eq!(active_layer_path(&doc, active).map(|(n, _)| n).as_deref(), Some("Rectangle 1 Shape Path"));
    let (ai, n) = export(&mut s, "Rectangle 1 Shape Path").unwrap();
    assert_eq!(n, 1);
    assert_eq!(first_points(&ai), [200.0], "only the shape's path");
}

#[test]
fn all_paths_writes_saved_paths_then_the_work_path_then_the_layer_path() {
    let mut s = session(true);
    s.execute("shape.create", json!({"kind": "rectangle", "rect": [200, 120, 60, 50]})).unwrap();
    let (ai, n) = export(&mut s, "all").unwrap();
    assert_eq!(n, 4);
    assert_eq!(first_points(&ai), [10.0, 60.0, 130.0, 200.0], "Rect A, Curve B, Work Path, Rectangle 1 Shape Path");
}

#[test]
fn a_vector_mask_is_listed_by_its_panel_name() {
    let mut s = session(false);
    s.execute("layer.new.layer", json!({"name": "Masked"})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    s.edit("mask", |d, _| {
        d.layer_mut(id).unwrap().vector_mask = Some(VectorMask::new(Path::new(vec![rect(220.0, 60.0, 280.0, 90.0)])));
        Ok(())
    })
    .unwrap();
    let (ai, n) = export(&mut s, "Masked Vector Mask").unwrap();
    assert_eq!((n, first_points(&ai)), (1, vec![220.0]));
    assert_eq!(first_points(&export(&mut s, "all").unwrap().0), [10.0, 60.0, 220.0]);
}

#[test]
fn an_inactive_layers_path_is_neither_listed_nor_exported() {
    let mut s = session(true);
    s.execute("shape.create", json!({"kind": "rectangle", "rect": [200, 120, 60, 50]})).unwrap();
    let bg = s.active().unwrap().doc.layers[0].id;
    s.execute("layer.select", json!({"layer": bg.0})).unwrap();
    let doc = s.active().unwrap().doc.clone();
    assert_eq!(active_layer_path(&doc, s.active().unwrap().active_layer), None);
    assert_eq!(first_points(&export(&mut s, "all").unwrap().0), [10.0, 60.0, 130.0], "Rect A, Curve B and the Work Path only");
    assert!(export(&mut s, "Rectangle 1 Shape Path").unwrap_err().contains("no paths match"));
}

#[test]
fn a_layer_path_alone_works_without_saved_or_work_paths() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 300, "height": 200})).unwrap();
    s.execute("shape.create", json!({"kind": "rectangle", "rect": [40, 40, 60, 50]})).unwrap();
    assert!(s.is_enabled("file.export.pathsToIllustrator"));
    assert_eq!(export(&mut s, "all").unwrap().1, 1);
}

fn export_saved(s: &mut Session, name: &str) -> std::result::Result<(String, u64), String> {
    let r = s.execute("file.export.pathsToIllustrator", json!({"savedPath": name})).map_err(|e| e.to_string())?;
    Ok((r["ai"].as_str().unwrap().to_string(), r["paths"].as_u64().unwrap()))
}

/// CodeRabbit on #15051: a saved path is chosen by `savedPath`, its exact name, so one called `all` or
/// `path:all` is never taken for another. The older `paths` calls keep their reading: exact name first.
#[test]
fn a_saved_path_is_chosen_by_its_exact_name_and_paths_keeps_its_older_reading() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 300, "height": 200})).unwrap();
    s.edit("paths", |d, _| {
        d.resolution_dpi = 72.0;
        d.paths.push(NamedPath { name: "all".into(), path: Path::new(vec![rect(10.0, 10.0, 50.0, 40.0)]), psd_raw: None });
        d.paths.push(NamedPath { name: "path:all".into(), path: Path::new(vec![rect(70.0, 10.0, 90.0, 40.0)]), psd_raw: None });
        Ok(())
    })
    .unwrap();
    assert_eq!(first_points(&export_saved(&mut s, "all").unwrap().0), [10.0], "the path called all, not All Paths");
    assert_eq!(first_points(&export_saved(&mut s, "path:all").unwrap().0), [70.0], "the path called path:all");
    assert!(export_saved(&mut s, "work").is_err(), "no saved path called work: an error, not the Work Path");
    // Older calls: `all` is All Paths, `path:all` is the path called that when it exists.
    assert_eq!(first_points(&export(&mut s, "all").unwrap().0), [10.0, 70.0]);
    assert_eq!(first_points(&export(&mut s, "path:all").unwrap().0), [70.0], "legacy exact name first");
    // With only `all` saved, `path:all` is still the prefix form for it.
    s.edit("one", |d, _| {
        d.paths.remove(1);
        Ok(())
    })
    .unwrap();
    assert_eq!(first_points(&export(&mut s, "path:all").unwrap().0), [10.0]);
}

/// CodeRabbit on #15051: the active layer's path has its own selector, so a saved path with the same
/// display name is not exported in its place.
#[test]
fn the_active_layers_path_has_a_selector_apart_from_a_saved_path_of_its_name() {
    let mut s = session(true);
    s.edit("clash", |d, _| {
        d.paths.push(NamedPath { name: "Rectangle 1 Shape Path".into(), path: Path::new(vec![rect(250.0, 10.0, 280.0, 40.0)]), psd_raw: None });
        Ok(())
    })
    .unwrap();
    s.execute("shape.create", json!({"kind": "rectangle", "rect": [200, 120, 60, 50]})).unwrap();
    assert_eq!(first_points(&export(&mut s, "layer").unwrap().0), [200.0], "the layer's path");
    assert_eq!(first_points(&export_saved(&mut s, "Rectangle 1 Shape Path").unwrap().0), [250.0], "the saved path of the same name");
    // An old call by the name finds the saved path first, as before.
    assert_eq!(first_points(&export(&mut s, "Rectangle 1 Shape Path").unwrap().0), [250.0]);
    s.edit("named layer", |d, _| {
        d.paths.push(NamedPath { name: "layer".into(), path: Path::new(vec![rect(300.0, 10.0, 320.0, 40.0)]), psd_raw: None });
        Ok(())
    })
    .unwrap();
    assert_eq!(first_points(&export_saved(&mut s, "layer").unwrap().0), [300.0], "a saved path called layer");
}
