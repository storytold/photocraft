//! #13776: the Paths to Illustrator dropdown reads Document Bounds, All Paths and Work Path, with
//! separators below Document Bounds and below All Paths, the saved paths before the Work Path
//! and the active layer's path last. A saved path named `all` or `work` is still that path.

use super::*;
use egui::accesskit::Role;
use egui_kittest::{Harness, kittest::Queryable};

const COMMAND: &str = "file.export.pathsToIllustrator";

fn path(x: f64) -> Value {
    json!({"subpaths": [{"closed": true, "knots": [[x, 10.0], [x + 30.0, 10.0], [x + 30.0, 40.0]]}]})
}

/// Saved `Rect A` and `all` (a reserved word as a name), a Work Path and the shape layer
/// `Rectangle 1` active.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    for (name, x) in [("Rect A", 10.0), ("all", 60.0), ("work", 110.0)] {
        app.run("path.set", json!({"name": name, "path": path(x)})).unwrap();
    }
    app.run("shape.create", json!({"kind": "rectangle", "rect": [200, 120, 60, 50]})).unwrap();
    app
}

#[test]
fn entries_labels_and_separators_follow_the_reference_order() {
    let (names, labels, groups) = paths_export_entries(&app());
    assert_eq!(names, ["documentBounds", "all", "Rect A", "path:all", "work", "layer"]);
    let label = |v: &str| labels[v].as_str().unwrap().to_string();
    assert_eq!(
        [label("documentBounds"), label("all"), label("Rect A"), label("path:all"), label("work"), label("layer")],
        ["Document Bounds", "All Paths", "Rect A", "all", "Work Path", "Rectangle 1 Shape Path"]
    );
    assert_eq!(groups, ["all", "Rect A"], "separators above All Paths and above the first path");
}

#[test]
fn a_document_without_paths_offers_document_bounds_alone_without_separators() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let (names, _, groups) = paths_export_entries(&app);
    assert_eq!(names, ["documentBounds"]);
    assert!(groups.is_empty());
}

/// The values stay `all`, `work` and the names; a saved path named `all` goes by `path:all`.
#[test]
fn a_saved_path_named_all_is_exported_as_that_path() {
    let mut app = app();
    let one = app.run(COMMAND, json!({"paths": "path:all"})).unwrap();
    assert_eq!(one["paths"], 1, "only the saved path named all");
    assert!(one["ai"].as_str().unwrap().contains("60.0000"), "its own points");
    let all = app.run(COMMAND, json!({"paths": "all"})).unwrap();
    assert_eq!(all["paths"], 4, "All Paths: Rect A, all, the Work Path and the shape path");
    assert_eq!(app.run(COMMAND, json!({"paths": "work"})).unwrap()["paths"], 1);
}

#[test]
fn the_dropdown_shows_the_labels_in_order() {
    let mut h = Harness::builder().with_size(egui::vec2(900.0, 700.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        app()
    });
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, COMMAND, json!({})).unwrap();
    h.run_steps(6);
    h.get_by_role_and_label(Role::ComboBox, "Paths").click();
    h.run_steps(4);
    let y = |label: &str| h.get_by_label(label).rect().min.y;
    let order = [y("Document Bounds"), y("Rect A"), y("Work Path"), y("Rectangle 1 Shape Path")];
    assert!(order.windows(2).all(|w| w[0] < w[1]), "{order:?}");
    assert!(h.query_by_label("All").is_none() && h.query_by_label("Work").is_none(), "no raw values");
}

/// CodeRabbit on #15051: every entry has its own value. Saved paths called `layer` and `path:all` get
/// `path:` values, and the active layer's path is `layer` whatever a saved path is named, so two rows
/// never send the same value.
#[test]
fn every_entry_has_its_own_value_even_for_awkward_names() {
    let mut app = app();
    for (name, x) in [("layer", 150.0), ("path:all", 180.0), ("Rectangle 1 Shape Path", 210.0)] {
        app.run("path.set", json!({"name": name, "path": path(x)})).unwrap();
    }
    let (names, labels, _) = paths_export_entries(&app);
    let unique: std::collections::HashSet<_> = names.iter().collect();
    assert_eq!(unique.len(), names.len(), "{names:?}");
    assert!(names.contains(&"path:layer".to_string()) && names.contains(&"path:path:all".to_string()) && names.contains(&"layer".to_string()));
    // The layer row and the saved path of the same display name export different geometry.
    let layer_rows: Vec<_> = names.iter().filter(|v| labels[v.as_str()] == json!("Rectangle 1 Shape Path")).collect();
    assert_eq!(layer_rows.len(), 2, "one saved path and the layer, same label: {layer_rows:?}");
    let mut ai = |v: &str| app.run(COMMAND, json!({"paths": v})).unwrap()["ai"].as_str().unwrap().to_string();
    assert!(ai("layer").contains("200.0000") && !ai("layer").contains("210.0000"));
    assert!(ai("Rectangle 1 Shape Path").contains("210.0000"));
}
