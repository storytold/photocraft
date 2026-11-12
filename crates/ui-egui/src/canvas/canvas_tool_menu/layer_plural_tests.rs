//! #12358: with several layers selected, the canvas layer menu's Duplicate, Delete and Rasterize
//! rows read in the plural, as the Layers panel menu does; with one layer selected they stay
//! singular. The labels come from the panel's rule and the command ids don't change.

use serde_json::{Value, json};

use super::count_tests::{call, right_click};
use super::*;

/// The rows whose label names one layer or several, with their singular and plural labels.
const COUNTED: [(&str, &str, &str); 3] = [
    ("layer.duplicate", "Duplicate Layer…", "Duplicate Layers…"),
    ("layer.delete", "Delete Layer", "Delete Layers"),
    ("layer.rasterize.layer", "Rasterize Layer", "Rasterize Layers"),
];

/// The Count tool's layer menu ids in the reference order (#11772), "―" for separators.
const WANT: [&str; 17] = [
    "layer.layerStyle.blendingOptions",
    "―",
    "layer.duplicate",
    "layer.delete",
    "layer.new.groupFromLayers",
    "―",
    "layer.smartObjects.convertToSmartObject",
    "layer.smartObjects.convertToLayers",
    "―",
    "layer.rasterize.layer",
    "layer.rasterize.layerStyle",
    "―",
    "edit.freeTransform",
    "―",
    "select.colorRange",
    "select.loadSelection",
    "select.reselect",
];

/// A 64 × 48 document with two pixel layers above the background, the top one active; with
/// `both`, the other one is selected too. The Count tool is current.
fn app(both: bool) -> (PhotocraftApp, egui::Context, [u64; 2]) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    let layer = |r: Value| r["layer"].as_u64().unwrap();
    let ink = layer(app.run("layer.new.layer", json!({"name": "Ink"})).unwrap());
    let paint = layer(app.run("layer.new.layer", json!({"name": "Paint"})).unwrap());
    if both {
        app.run("layer.select", json!({"layer": ink, "mode": "add"})).unwrap();
    }
    app.sync_views();
    app.ui.tool = Tool::Count;
    (app, egui::Context::default(), [ink, paint])
}

/// The open menu's `ui.inspect` entries as (id, label, enabled), "―" for separators.
fn inspected(app: &mut PhotocraftApp, ctx: &egui::Context) -> Vec<(String, String, bool)> {
    let r = call(app, ctx, "ui.inspect", json!({}));
    let entries = r.pointer("/result/canvasToolMenu/entries").and_then(Value::as_array).cloned().unwrap_or_default();
    entries
        .iter()
        .map(|e| match e["id"].as_str() {
            Some(id) => (id.to_string(), e["label"].as_str().unwrap_or_default().to_string(), e["enabled"].as_bool().unwrap_or_default()),
            None => ("―".to_string(), String::new(), false),
        })
        .collect()
}

fn selected(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().selected_layers().len()
}

#[test]
fn with_two_layers_selected_the_layer_rows_read_in_the_plural() {
    let (mut app, ctx, _) = app(true);
    assert_eq!(selected(&app), 2);
    right_click(&mut app, &ctx, 50.0, 40.0);
    let entries = inspected(&mut app, &ctx);
    assert_eq!(entries.iter().map(|e| e.0.as_str()).collect::<Vec<_>>(), WANT, "the command ids and their order don't change");
    for (id, _, plural) in COUNTED {
        let label = entries.iter().find(|e| e.0 == id).map(|e| e.1.as_str());
        assert_eq!(label, Some(plural), "{id}");
    }
    let labels: Vec<&str> = entries.iter().filter(|e| e.0 != "―").map(|e| e.1.as_str()).collect();
    assert_eq!(
        labels,
        [
            "Blending Options…",
            "Duplicate Layers…",
            "Delete Layers",
            "Group from Layers…",
            "Convert to Smart Object",
            "Convert to Layers",
            "Rasterize Layers",
            "Rasterize Layer Style",
            "Free Transform",
            "Color Range…",
            "Load Selection…",
            "Reselect",
        ],
        "the measured list"
    );
    // Greyed in the reference with two pixel layers and no pixel selection; the plural rows act on
    // the whole selection and stay available. Blending Options, Color Range… and Load Selection…
    // (also greyed there) are left out: the rows keep following `menus::is_enabled`, which enables
    // them here, and #12358 changes labels only.
    let greyed = ["layer.smartObjects.convertToLayers", "layer.rasterize.layer", "layer.rasterize.layerStyle"];
    let enabled = ["layer.duplicate", "layer.delete", "layer.new.groupFromLayers", "layer.smartObjects.convertToSmartObject", "edit.freeTransform"];
    let wrong: Vec<(&str, Option<bool>)> = greyed
        .iter()
        .map(|id| (*id, false))
        .chain(enabled.iter().map(|id| (*id, true)))
        .filter_map(|(id, want)| {
            let got = entries.iter().find(|e| e.0 == id).map(|e| e.2);
            (got != Some(want)).then_some((id, got))
        })
        .collect();
    assert!(wrong.is_empty(), "rows enabled otherwise than in the reference: {wrong:?}");
}

#[test]
fn with_one_layer_selected_the_layer_rows_stay_singular() {
    let (mut app, ctx, _) = app(false);
    assert_eq!(selected(&app), 1);
    right_click(&mut app, &ctx, 50.0, 40.0);
    let entries = inspected(&mut app, &ctx);
    assert_eq!(entries.iter().map(|e| e.0.as_str()).collect::<Vec<_>>(), WANT);
    for (id, singular, _) in COUNTED {
        let label = entries.iter().find(|e| e.0 == id).map(|e| e.1.as_str());
        assert_eq!(label, Some(singular), "{id}");
    }
}

/// The canvas rows and the Layers panel menu take their labels from the same rule.
#[test]
fn the_labels_match_the_layers_panel_menu() {
    for multi in [false, true] {
        let (mut app, ctx, [_, paint]) = app(multi);
        right_click(&mut app, &ctx, 50.0, 40.0);
        let entries = inspected(&mut app, &ctx);
        let st = app.session.active().unwrap();
        let layer = st.doc.layer(photocraft_doc::LayerId(paint)).unwrap();
        let panel = crate::layer_menu_ui::entries(layer, multi, false);
        for (id, _, _) in COUNTED {
            let canvas = entries.iter().find(|e| e.0 == id).map(|e| e.1.as_str());
            let panel = panel.iter().flatten().find(|e| e.1 == id).map(|e| e.0);
            assert_eq!(canvas, panel, "{id}, several layers: {multi}");
        }
    }
}

/// Delete Layers still runs `layer.delete`, which removes every selected layer.
#[test]
fn delete_layers_still_deletes_the_selected_layers() {
    let (mut app, ctx, [ink, paint]) = app(true);
    right_click(&mut app, &ctx, 50.0, 40.0);
    assert_eq!(call(&mut app, &ctx, "ui.context.choose", json!({"id": "layer.delete"}))["ok"], true);
    let st = app.session.active().unwrap();
    for id in [ink, paint] {
        assert!(st.doc.layer(photocraft_doc::LayerId(id)).is_none(), "layer {id} deleted");
    }
}
