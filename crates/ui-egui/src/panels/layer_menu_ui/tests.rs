use super::*;
use craft::layers::ui::panel::MenuTarget;
use craft::ui::contract::Entry;
use craft::ui::list::ThumbSize;
use photocraft_doc::LayerId;
use photocraft_engine::Session;
use serde_json::json;

/// The active layer after `s`.
fn active_layer(s: &Session) -> Layer {
    let st = s.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().clone()
}

fn plain(_: &mut Session) {}

fn masked(s: &mut Session) {
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
}

fn smart(s: &mut Session) {
    s.execute("paint.stroke", json!({"points": [[12, 12]], "size": 8})).unwrap();
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
}

fn typed(s: &mut Session) {
    s.execute("type.create", json!({"text": "Hi", "size": 24, "x": 8, "y": 32})).unwrap();
}

fn grouped(s: &mut Session) {
    s.execute("layer.groupLayers", json!({"name": "G"})).unwrap();
}

/// What the right-click menu offers for each kind of target: the kind's own items, the trimming for a multi-selection,
/// and the empty space's thumbnail size and clipping choices.
#[test]
fn row_menu_entries_follow_the_target() {
    struct Case {
        case: &'static str,
        setup: fn(&mut Session),
        multi: bool,
        selection: bool,
        present: &'static [&'static str],
        absent: &'static [&'static str],
    }
    let cases = [
        Case {
            case: "a plain layer",
            setup: plain,
            multi: false,
            selection: false,
            present: &["layer.layerStyle.blendingOptions", "layer.mergeDown", "layer.layerMask.revealAll"],
            absent: &["layer.mergeLayers", "layer.smartObjects.editContents", "layer.layerMask.enabled"],
        },
        Case {
            case: "a plain layer with a selection: the mask is the selection",
            setup: plain,
            multi: false,
            selection: true,
            present: &["layer.layerMask.revealSelection"],
            absent: &["layer.layerMask.revealAll"],
        },
        Case {
            case: "a plain multi-selection merges",
            setup: plain,
            multi: true,
            selection: false,
            present: &["layer.mergeLayers"],
            absent: &["layer.mergeDown"],
        },
        Case {
            case: "a masked layer offers its mask items",
            setup: masked,
            multi: false,
            selection: false,
            present: &["layer.layerMask.enabled", "layer.layerMask.apply", "layer.layerMask.delete"],
            absent: &[],
        },
        Case {
            case: "a masked multi-selection has no per-layer mask items",
            setup: masked,
            multi: true,
            selection: false,
            present: &["layer.layerMask.revealAll"],
            absent: &["layer.layerMask.enabled", "layer.layerMask.apply", "layer.layerMask.delete"],
        },
        Case {
            case: "a smart object offers its contents",
            setup: smart,
            multi: false,
            selection: false,
            present: &[
                "layer.smartObjects.editContents",
                "layer.smartObjects.resetTransform",
                "layer.smartObjects.replaceContents",
                "layer.smartObjects.relinkToFile",
                "layer.smartObjects.convertToLayers",
                "layer.rasterize.smartObject",
            ],
            absent: &["layer.rasterize.layer"],
        },
        Case {
            case: "a smart object in a multi-selection rasterizes the selection",
            setup: smart,
            multi: true,
            selection: false,
            present: &["layer.rasterize.layer"],
            absent: &[
                "layer.rasterize.smartObject",
                "layer.smartObjects.editContents",
                "layer.smartObjects.resetTransform",
                "layer.smartObjects.convertToLayers",
            ],
        },
        Case {
            case: "a type layer converts to a shape",
            setup: typed,
            multi: false,
            selection: false,
            present: &["layer.rasterize.type", "type.createWorkPath", "type.convertToShape"],
            absent: &[],
        },
        Case {
            case: "a type layer in a multi-selection keeps one Rasterize item",
            setup: typed,
            multi: true,
            selection: false,
            present: &["layer.rasterize.layer"],
            absent: &["type.convertToShape"],
        },
        Case { case: "a group merges itself", setup: grouped, multi: false, selection: false, present: &["layer.mergeGroup"], absent: &["layer.mergeDown"] },
    ];
    for c in cases {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 32, "height": 24})).unwrap();
        s.execute("layer.new.layer", json!({"name": "Source"})).unwrap();
        (c.setup)(&mut s);
        let l = active_layer(&s);
        let ids: Vec<&str> = entries(&l, c.multi, c.selection).into_iter().flatten().map(|e| e.1).collect();
        for id in c.present {
            assert!(ids.contains(id), "{}: {id} is offered", c.case);
        }
        for id in c.absent {
            assert!(!ids.contains(id), "{}: {id} is not offered", c.case);
        }
        assert_eq!(ids.first(), Some(&"layer.layerStyle.blendingOptions"), "{}", c.case);
    }
    // Rasterize Layers and Merge Group are labelled for what they do.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 10, "height": 10})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    grouped(&mut s);
    let e: Vec<(&str, &str)> = entries(&active_layer(&s), false, false).into_iter().flatten().collect();
    assert!(e.contains(&("Merge Group", "layer.mergeGroup")), "{e:?}");
    // The empty space: four sizes, a separator, two clip choices, the current ones checked.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    let app = crate::PhotocraftApp::new(s, Default::default());
    let st = app.session.active().unwrap();
    let tree = crate::panels::layers_tree::PhotoLayers::new(&app, st, &Default::default(), false, None);
    let target = MenuTarget::<LayerId>::new(None);
    let menu = crate::panels::layers_tree::row_menu(&tree, &target);
    let size = |size: ThumbSize, label: &str| {
        let value = format!("{}{}", crate::panels::layers_tree::THUMB_SIZE, crate::layers_panel_ui::size_key(size));
        Entry::new(value, label.to_string()).checked(size == ThumbSize::Medium)
    };
    let clip = |to_layer: bool, label: &str| Entry::new(format!("{}{to_layer}", crate::panels::layers_tree::CLIP_THUMBS), label.to_string()).checked(!to_layer);
    let want: Vec<Entry<String>> = vec![
        size(ThumbSize::None, "No Thumbnails"),
        size(ThumbSize::Small, "Small Thumbnails"),
        size(ThumbSize::Medium, "Medium Thumbnails"),
        size(ThumbSize::Large, "Large Thumbnails"),
        Entry::separator(),
        clip(true, "Clip Thumbnails to Layer Bounds"),
        clip(false, "Clip Thumbnails to Document Bounds"),
    ];
    assert_eq!(menu, want);
}

/// Every entry of the multi-layer menu is either known to act on the selection or greyed by
/// [`single_layer_only`]; a new single-layer command added to the menu must be classified.
#[test]
fn single_layer_only_covers_the_per_layer_commands() {
    for id in
        ["layer.layerStyle.blendingOptions", "layer.layerStyle.copyLayerStyle", "layer.layerMask.enabled", "layer.layerMask.apply", "layer.layerMask.delete"]
    {
        assert!(single_layer_only(id), "{id}");
    }
    for id in [
        "layer.quickExportAsPng",
        "layer.exportAs",
        "layer.duplicate",
        "layer.delete",
        "layer.new.groupFromLayers",
        "layer.rasterize.layer",
        "layer.smartObjects.convertToSmartObject",
        "layer.createClippingMask",
        "layer.linkLayers",
        "layer.mergeLayers",
        "layer.layerMask.revealAll",
        "layer.layerMask.hideSelection",
        "layer.layerStyle.pasteLayerStyle",
        "layer.layerStyle.clear",
    ] {
        assert!(!single_layer_only(id), "{id}");
    }
}

#[test]
fn add_mask_uses_the_selection() {
    assert_eq!(add_mask_command(false, false), "layer.layerMask.revealAll");
    assert_eq!(add_mask_command(false, true), "layer.layerMask.hideAll");
    assert_eq!(add_mask_command(true, true), "layer.layerMask.hideSelection");
    // With a selection the new mask is the selection: inside revealed, outside hidden.
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 10, "height": 10})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 4, "height": 10})).unwrap();
    s.execute(add_mask_command(true, false), json!({})).unwrap();
    let st = s.active().unwrap();
    let mask = &st.doc.layer(st.active_layer.unwrap()).unwrap().mask.as_ref().unwrap().surface;
    assert!(mask.pixel(1, 5)[0] > 0.99 && mask.pixel(8, 5)[0] < 0.01);
}

/// #2075: clicking the mask button again (with or without ⌥) used to replace the layer's
/// mask, discarding what was painted into it. Like Photoshop, it now adds a vector mask, and
/// does nothing once the layer has both.
#[test]
fn mask_button_never_replaces_a_mask() {
    for alt in [false, true] {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 10, "height": 10})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        let layer = |s: &photocraft_engine::Session| {
            let st = s.active().unwrap();
            st.doc.layer(st.active_layer.unwrap()).unwrap().clone()
        };
        let first = mask_button_command(Some(&layer(&s)), false, alt).unwrap();
        assert_eq!(first, if alt { "layer.layerMask.hideAll" } else { "layer.layerMask.revealAll" });
        s.execute(first, json!({})).unwrap();
        // Paint a stroke into the mask.
        s.edit("paint mask", |doc, active| {
            let id = (*active).ok_or(photocraft_engine::EngineError::NoDocument)?;
            let m = doc.layer_mut(id).and_then(|l| l.mask.as_mut()).ok_or(photocraft_engine::EngineError::NoDocument)?;
            m.surface.fill_rect(photocraft_geom::Rect::new(2, 2, 5, 5), &[0.5]);
            Ok(())
        })
        .unwrap();
        let painted = layer(&s).mask.unwrap().surface.pixel(3, 3)[0];
        assert!((painted - 0.5).abs() < 0.01);
        // Second click: a vector mask (⌥ hides all), the layer mask is untouched.
        let second = mask_button_command(Some(&layer(&s)), false, alt).unwrap();
        assert_eq!(second, if alt { "layer.vectorMask.hideAll" } else { "layer.vectorMask.revealAll" });
        s.execute(second, json!({})).unwrap();
        let l = layer(&s);
        assert!(l.vector_mask.is_some(), "alt={alt}");
        let mask = l.mask.as_ref().unwrap();
        assert!((mask.surface.pixel(3, 3)[0] - painted).abs() < 1e-6, "alt={alt}: the painted mask was replaced");
        // Third click: nothing left to add.
        assert_eq!(mask_button_command(Some(&l), false, alt), None);
        assert_eq!(mask_button_command(Some(&l), true, alt), None);
    }
    // No layer: nothing to do.
    assert_eq!(mask_button_command(None, false, false), None);
    // A shape layer's path is already its vector mask: after the layer mask, nothing more.
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 10, "height": 10})).unwrap();
    s.execute("shape.create", json!({"kind": "rect", "rect": [0, 0, 5, 5]})).unwrap();
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    let st = s.active().unwrap();
    let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
    assert_eq!(mask_button_command(Some(l), false, false), None);
}

#[test]
fn every_entry_is_a_known_command() {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 10, "height": 10})).unwrap();
    let l = s.active().unwrap().doc.layers[0].clone();
    for (_, id) in entries(&l, false, true).into_iter().chain(entries(&l, false, false)).chain(entries(&l, true, false)).chain(effects_entries()).flatten() {
        assert!(crate::menu_catalog::CATALOG.iter().any(|m| m.3 == id) || photocraft_engine::commands::find(id).is_some(), "{id}");
    }
}
