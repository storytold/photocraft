//! Layer > Smart Objects menu rows (#11401).

use craft::menu::Row;
use serde_json::json;

use crate::PhotocraftApp;
use crate::menus::menu_rows;

fn app() -> PhotocraftApp {
    PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
}

fn submenu(app: &PhotocraftApp) -> Vec<Row> {
    menu_rows(app).into_iter().filter(|r| r.path == ["Layer", "Smart Objects"]).collect()
}

#[test]
fn embed_all_linked_is_disabled_without_a_linked_smart_object_and_enabled_with_one() {
    let mut app = app();
    app.run("file.new", json!({"width": 32, "height": 24})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("setup", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(2, 2, 10, 10), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    app.sync_views();
    let enabled = |app: &PhotocraftApp, id: &str| submenu(app).iter().find(|r| r.command.id == id).unwrap().enabled;
    assert!(!enabled(&app, "layer.smartObjects.embedAllLinked"));
    let path = std::env::temp_dir().join(format!("pc-so-menu-{}.pcraft", std::process::id()));
    app.run("layer.smartObjects.convertToLinked", json!({"path": path.to_str().unwrap()})).unwrap();
    app.sync_views();
    assert!(enabled(&app, "layer.smartObjects.embedAllLinked"));
    assert!(enabled(&app, "layer.smartObjects.convertToEmbedded"));
    std::fs::remove_file(&path).ok();
}
