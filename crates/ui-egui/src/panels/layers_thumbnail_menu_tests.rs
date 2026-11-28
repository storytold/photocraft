//! #16777: right-click on a layer thumbnail opens the short thumbnail menu for every layer kind,
//! not the whole layer menu. The entries, their order and the greyed states are pinned; frames,
//! artboards and the name area keep the full layer menu as before.

use craft::layers::ui::panel::{LayersEvent, MenuTarget};
use craft::layers::ui::tree::{LayerPart, LayerRowKey, LayerTree};
use craft::ui::contract::Entry;
use craft::ui::list::ThumbSize;
use photocraft_doc::LayerId;
use photocraft_engine::Session;
use serde_json::json;

use crate::PhotocraftApp;
use crate::panels::layers_events::apply_layers_events;
use crate::panels::layers_tree::{CLIP_THUMBS, PhotoLayers, SELECT_PIXELS, THUMB_SIZE, row_menu};

const BLEND: &str = "layer.layerStyle.blendingOptions";
const EDIT_ADJ: &str = "layer.layerContentOptions";
const LOAD_PIXELS: &str = "layersPanel:loadPixels:";
const SEP: &str = "―";

/// The thumbnail menu's items and separators in order; an entry that is not an item is `None`.
fn plan() -> Vec<Option<String>> {
    let size = |s: ThumbSize| format!("{THUMB_SIZE}{}", crate::layers_panel_ui::size_key(s));
    vec![
        Some(BLEND.into()),
        Some(EDIT_ADJ.into()),
        None,
        Some(SELECT_PIXELS.into()),
        None,
        Some(format!("{LOAD_PIXELS}add")),
        Some(format!("{LOAD_PIXELS}subtract")),
        Some(format!("{LOAD_PIXELS}intersect")),
        None,
        Some(size(ThumbSize::None)),
        Some(size(ThumbSize::Small)),
        Some(size(ThumbSize::Medium)),
        Some(size(ThumbSize::Large)),
        None,
        Some(format!("{CLIP_THUMBS}true")),
        Some(format!("{CLIP_THUMBS}false")),
    ]
}

/// The expected pins: every item enabled except `greyed`.
fn want(greyed: &[&str]) -> Vec<String> {
    plan()
        .into_iter()
        .map(|e| match e {
            None => SEP.to_string(),
            Some(v) => format!("{}{}", if greyed.contains(&v.as_str()) { "!" } else { "=" }, v),
        })
        .collect()
}

/// One pin per entry: `"―"` for a separator, else `=`/`!` (enabled or greyed) and the entry value.
fn pins(menu: &[Entry<String>]) -> Vec<String> {
    menu.iter()
        .map(
            |e| {
                if e.is_separator() { SEP.to_string() } else { format!("{}{}", if e.is_enabled() { "=" } else { "!" }, e.value().cloned().unwrap_or_default()) }
            },
        )
        .collect()
}

struct Kinds {
    pixel: LayerId,
    shape: LayerId,
    smart: LayerId,
    typed: LayerId,
    adjust: LayerId,
    group: LayerId,
    frame: LayerId,
    artboard: LayerId,
    background: LayerId,
}

/// A 60 x 40 document with one layer of every kind on top of a white Background.
fn kinds() -> (Box<PhotocraftApp>, Kinds) {
    let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 60, "height": 40, "background": "white"})).unwrap();
    let background = app.session.active().unwrap().doc.layers.first().unwrap().id;
    let created = |app: &mut PhotocraftApp| app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.new.layer", json!({"name": "Pix"})).unwrap();
    let pixel = created(&mut app);
    app.run("select.rect", json!({"x": 2, "y": 2, "width": 10, "height": 10})).unwrap();
    app.run("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.run("shape.create", json!({"kind": "rect", "rect": [30, 4, 12, 8], "fill": "#00ff00"})).unwrap();
    let shape = created(&mut app);
    app.run("type.create", json!({"x": 6, "y": 28, "text": "Hi", "size": 10})).unwrap();
    let typed = created(&mut app);
    app.run("layer.new.layer", json!({"name": "Smart"})).unwrap();
    app.run("select.rect", json!({"x": 44, "y": 20, "width": 10, "height": 8})).unwrap();
    app.run("edit.fill", json!({"contents": "color", "color": "#0000ff"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let smart = created(&mut app);
    app.run("layer.newAdjustmentLayer.levels", json!({})).unwrap();
    let adjust = created(&mut app);
    // The new groups and the frame go in with the pixel layer active: with a group, artboard or
    // frame active, `layer.new.group` and `layer.new.frame` land inside it instead of at top level.
    activate(&mut app, pixel);
    app.run("layer.new.group", json!({"name": "G"})).unwrap();
    let group = created(&mut app);
    activate(&mut app, pixel);
    app.run("layer.new.group", json!({"name": "AB"})).unwrap();
    let artboard = created(&mut app);
    app.run("layer.new.artboardFromGroup", json!({"layer": artboard.0})).unwrap();
    activate(&mut app, pixel);
    app.run("layer.new.frame", json!({"x": 4, "y": 4, "width": 40, "height": 30})).unwrap();
    let frame = created(&mut app);
    (app, Kinds { pixel, shape, smart, typed, adjust, group, frame, artboard, background })
}

fn activate(app: &mut PhotocraftApp, id: LayerId) {
    app.run("layer.select", json!({"layer": id.0, "mode": "replace"})).unwrap();
}

/// The menu a right-click on `id`'s thumbnail builds for the panel.
fn thumb_menu(app: &PhotocraftApp, id: LayerId) -> Vec<Entry<String>> {
    let st = app.session.active().unwrap();
    let tree = PhotoLayers::new(app, st, &Default::default(), false, None);
    let mut target = MenuTarget::new(Some(LayerRowKey::Layer(id)));
    target.part = Some(LayerPart::Pixels);
    row_menu(app, &tree, &target)
}

fn enabled_of(menu: &[Entry<String>], value: &str) -> bool {
    menu.iter().find(|e| e.value().is_some_and(|v| v == value)).is_some_and(|e| e.is_enabled())
}

/// A pixel layer's thumbnail menu, without a document selection: the full list, Edit Adjustment…
/// and the three Transparency Mask rows greyed, and the new rows carry their measured labels.
#[test]
fn a_pixel_thumbnail_lists_the_full_menu_and_greys_the_mask_rows_without_a_selection() {
    let (mut app, k) = kinds();
    activate(&mut app, k.pixel);
    let menu = thumb_menu(&app, k.pixel);
    assert_eq!(pins(&menu), want(&[EDIT_ADJ, &format!("{LOAD_PIXELS}add"), &format!("{LOAD_PIXELS}subtract"), &format!("{LOAD_PIXELS}intersect")]));
    let label = |value: &str| menu.iter().find(|e| e.value().is_some_and(|v| v == value)).map(|e| e.label().as_str().to_string()).expect("entry shown");
    assert_eq!(label(BLEND), "Blending Options…");
    assert_eq!(label(EDIT_ADJ), "Edit Adjustment…");
    for (op, text) in [("add", "Add Transparency Mask"), ("subtract", "Subtract Transparency Mask"), ("intersect", "Intersect Transparency Mask")] {
        assert_eq!(label(&format!("{LOAD_PIXELS}{op}")), text, "{op}");
    }
}

/// With a selection, only Edit Adjustment… stays greyed; the three Transparency Mask rows come live.
#[test]
fn a_selection_only_greys_edit_adjustment_on_a_pixel_thumbnail() {
    let (mut app, k) = kinds();
    activate(&mut app, k.pixel);
    app.run("select.rect", json!({"x": 20, "y": 20, "width": 10, "height": 10})).unwrap();
    let menu = thumb_menu(&app, k.pixel);
    assert_eq!(pins(&menu), want(&[EDIT_ADJ]));
}

/// Shape, smart object, type, adjustment, group and Background thumbnails open the same list,
/// in the same order, as the pixel layer's thumbnail.
#[test]
fn every_measured_kind_gets_the_same_thumbnail_menu_list() {
    let (mut app, k) = kinds();
    let values: Vec<String> = plan().into_iter().flatten().collect();
    for (case, id) in
        [("shape", k.shape), ("smart object", k.smart), ("type", k.typed), ("adjustment", k.adjust), ("group", k.group), ("Background", k.background)]
    {
        activate(&mut app, id);
        let got: Vec<String> = thumb_menu(&app, id).into_iter().filter_map(|e| e.value().cloned()).collect();
        assert_eq!(got, values, "{case}");
    }
}

/// The greyed states follow the measured table for every kind and both selection states.
#[test]
fn the_greyed_states_follow_the_measured_table() {
    let (mut app, k) = kinds();
    let ops: Vec<String> = ["add", "subtract", "intersect"].iter().map(|op| format!("{LOAD_PIXELS}{op}")).collect();
    let size = |s: ThumbSize| format!("{THUMB_SIZE}{}", crate::layers_panel_ui::size_key(s));

    // Pixel, no selection: Edit Adjustment… and the three mask rows greyed, the rest live.
    activate(&mut app, k.pixel);
    let m = thumb_menu(&app, k.pixel);
    assert!(enabled_of(&m, BLEND) && enabled_of(&m, SELECT_PIXELS));
    assert!(!enabled_of(&m, EDIT_ADJ));
    for op in &ops {
        assert!(!enabled_of(&m, op), "pixel without a selection greys {op}");
    }

    // Adjustment layer, no selection: Edit Adjustment… is the only live new row among these.
    activate(&mut app, k.adjust);
    let m = thumb_menu(&app, k.adjust);
    assert!(enabled_of(&m, BLEND) && enabled_of(&m, SELECT_PIXELS) && enabled_of(&m, EDIT_ADJ));
    for op in &ops {
        assert!(!enabled_of(&m, op), "adjustment without a selection greys {op}");
    }

    // Group: Edit Adjustment…, Select Pixels and the three mask rows greyed; Blending Options… live.
    activate(&mut app, k.group);
    let m = thumb_menu(&app, k.group);
    assert!(enabled_of(&m, BLEND));
    assert!(!enabled_of(&m, EDIT_ADJ) && !enabled_of(&m, SELECT_PIXELS));
    for op in &ops {
        assert!(!enabled_of(&m, op), "a group greys {op}");
    }

    // Background, even with a selection: everything except the four sizes and the two clip rows.
    activate(&mut app, k.background);
    app.run("select.rect", json!({"x": 20, "y": 20, "width": 10, "height": 10})).unwrap();
    let m = thumb_menu(&app, k.background);
    assert!(!enabled_of(&m, BLEND), "Background greys Blending Options…");
    assert!(!enabled_of(&m, EDIT_ADJ) && !enabled_of(&m, SELECT_PIXELS));
    for op in &ops {
        assert!(!enabled_of(&m, op), "Background greys {op}");
    }
    for s in [ThumbSize::None, ThumbSize::Small, ThumbSize::Medium, ThumbSize::Large] {
        assert!(enabled_of(&m, &size(s)), "Background keeps the {s:?} size row live");
    }
    assert!(enabled_of(&m, &format!("{CLIP_THUMBS}true")) && enabled_of(&m, &format!("{CLIP_THUMBS}false")), "Background keeps the clip rows live");
}

/// Picking a Transparency Mask row runs `select.loadSelection` on the clicked layer's transparency:
/// the intersect leaves the layer's pixels, subtracting them again empties the selection, and a far
/// rectangle survives an add/subtract pair of the same layer.
#[test]
fn picking_the_transparency_rows_loads_the_layers_pixels() {
    let (mut app, k) = kinds();
    let ctx = app.ctx().clone();
    activate(&mut app, k.pixel);
    let target = MenuTarget::new(Some(LayerRowKey::Layer(k.pixel)));
    let pick = |app: &mut PhotocraftApp, op: &str| {
        apply_layers_events(app, &ctx, vec![LayersEvent::menu(target.clone(), format!("{LOAD_PIXELS}{op}"))]);
    };

    app.run("select.all", json!({})).unwrap();
    let steps = app.session.active().unwrap().history.past_len();
    pick(&mut app, "intersect");
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1, "one history step");
    assert!(app.session.active().unwrap().doc.selection.is_some(), "the intersect leaves the layer's pixels");
    pick(&mut app, "subtract");
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 2, "one history step each");
    assert!(app.session.active().unwrap().doc.selection.is_none(), "subtracting the same pixels leaves nothing");

    app.run("select.rect", json!({"x": 30, "y": 30, "width": 10, "height": 5})).unwrap();
    pick(&mut app, "add");
    pick(&mut app, "subtract");
    assert!(app.session.active().unwrap().doc.selection.is_some(), "the far rectangle survives an add/subtract pair");
}

/// Frames and artboards were not measured and keep today's full layer menu, as does the name area.
#[test]
fn frames_artboards_and_the_name_area_keep_the_full_layer_menu() {
    let (mut app, k) = kinds();
    for (case, id, part) in [("frame", k.frame, Some(LayerPart::Pixels)), ("artboard", k.artboard, Some(LayerPart::Pixels)), ("name area", k.pixel, None)] {
        activate(&mut app, id);
        let menu = {
            let st = app.session.active().unwrap();
            let tree = PhotoLayers::new(&app, st, &Default::default(), false, None);
            let mut target = MenuTarget::new(Some(LayerRowKey::Layer(id)));
            target.part = part;
            row_menu(&app, &tree, &target)
        };
        let values: Vec<String> = menu.into_iter().filter_map(|e| e.value().cloned()).collect();
        assert!(values.iter().any(|v| v == "layer.duplicate"), "{case}: the full layer menu");
        assert!(!values.iter().any(|v| v.starts_with(LOAD_PIXELS)), "{case}: no thumbnail mask rows");
    }
}

/// The menu is built from the clicked row: a thumbnail press on a layer the panel has not made
/// active yet still lists that layer's thumbnail menu (its kind decides the list, not the active
/// layer's; the engine enablement behind two greyed flags is read from the session as in today's
/// row menus).
#[test]
fn the_menu_of_a_row_that_is_not_active_still_lists_the_clicked_layer() {
    let (mut app, k) = kinds();
    activate(&mut app, k.pixel);
    let got: Vec<String> = thumb_menu(&app, k.adjust).into_iter().filter_map(|e| e.value().cloned()).collect();
    assert_eq!(got, plan().into_iter().flatten().collect::<Vec<String>>(), "the adjustment's own list, not the active pixel layer's");
}

/// The size and clip rows still follow the panel's current settings: only the current size is
/// checked, and the clip row matching the preference.
#[test]
fn the_size_and_clip_rows_follow_the_current_settings() {
    let (app, k) = kinds();
    let menu = thumb_menu(&app, k.pixel);
    let current = app.ui.layers_settings.options.thumbnail_size;
    for s in [ThumbSize::None, ThumbSize::Small, ThumbSize::Medium, ThumbSize::Large] {
        let v = format!("{THUMB_SIZE}{}", crate::layers_panel_ui::size_key(s));
        let entry = menu.iter().find(|e| e.value().is_some_and(|x| *x == v)).expect("the size row is shown");
        assert_eq!(entry.is_checked(), current == s, "only the current size is checked ({s:?})");
    }
    let clip = app.session.prefs().interface.clip_thumbnails_to_layer_bounds;
    for to_layer in [true, false] {
        let v = format!("{CLIP_THUMBS}{to_layer}");
        let entry = menu.iter().find(|e| e.value().is_some_and(|x| *x == v)).expect("the clip row is shown");
        assert_eq!(entry.is_checked(), clip == to_layer, "the clip row matching the preference is checked");
    }
}

/// The thumbnail menu ends with the row's Color submenu, like the row menu.
#[test]
fn the_thumbnail_menu_offers_the_rows_colour_submenu() {
    let (app, k) = kinds();
    let st = app.session.active().unwrap();
    let tree = PhotoLayers::new(&app, st, &Default::default(), false, None);
    let mut target = MenuTarget::new(Some(LayerRowKey::Layer(k.pixel)));
    target.part = Some(LayerPart::Pixels);
    assert!(tree.label_colors_in_menu(&target), "the thumbnail menu carries the colour submenu");
    let empty = MenuTarget::new(None);
    assert!(!tree.label_colors_in_menu(&empty), "the empty-space menu does not");
}

/// The issue lists a hidden, locked or clipped row as not measured and keeps it on today's menu
/// (#16777): the pixel layer, which had the short menu in every state, keeps it, and a layer of any
/// other kind keeps the full layer menu its measured twin no longer gets.
#[test]
fn a_hidden_locked_or_clipped_row_keeps_todays_menu() {
    let (mut app, k) = kinds();
    let set = |app: &mut PhotocraftApp, id: LayerId, key: &str, on: bool| app.run("layer.setProps", json!({"layer": id.0, key: on})).unwrap();
    // `state` is the value that puts the row into the unmeasured state, `plain` the value it had before.
    for (key, state, plain, case) in [("visible", false, true, "hidden"), ("locked", true, false, "locked"), ("clipped", true, false, "clipped")] {
        for (kind, id) in [("pixel", k.pixel), ("shape", k.shape)] {
            set(&mut app, id, key, state);
            let values: Vec<String> = thumb_menu(&app, id).into_iter().filter_map(|e| e.value().cloned()).collect();
            let short = values.iter().any(|v| v.starts_with(LOAD_PIXELS));
            let full = values.iter().any(|v| v == "layer.duplicate");
            assert_eq!((short, full), (kind == "pixel", kind != "pixel"), "a {case} {kind} thumbnail keeps today's menu");
            set(&mut app, id, key, plain);
        }
    }
}

/// The measured Background row carries the Background's own transparency and position locks, and
/// those are the only locks its exemption covers. A lock it does not carry by itself - all or
/// artboard - is the locked state the issue lists as not measured, so that row keeps the full layer
/// menu it had before. Locking the pixels is no such case: that clears the transparency lock, and
/// the layer stops being a Background.
#[test]
fn a_background_with_an_extra_lock_keeps_todays_layer_menu() {
    let (mut app, k) = kinds();
    let shown = |app: &PhotocraftApp| -> (bool, bool) {
        let values: Vec<String> = thumb_menu(app, k.background).into_iter().filter_map(|e| e.value().cloned()).collect();
        (values.iter().any(|v| v.starts_with(LOAD_PIXELS)), values.iter().any(|v| v == "layer.duplicate"))
    };
    let own_locks = photocraft_doc::Locks { transparency: true, position: true, ..Default::default() };
    assert_eq!(shown(&app), (true, false), "the Background with its own locks opens the short menu");
    for key in ["all", "artboard"] {
        app.run("layer.setProps", json!({"layer": k.background.0, "locks": {key: true}})).unwrap();
        assert!(app.session.active().unwrap().doc.is_background(k.background), "the {key} lock leaves the row a Background");
        assert_eq!(shown(&app), (false, true), "a Background with the {key} lock keeps today's full layer menu");
        app.run("layer.setProps", json!({"layer": k.background.0, "locks": {key: false}})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.layers.first().unwrap().locks, own_locks, "only the Background's own locks are left after the {key} case");
        assert_eq!(shown(&app), (true, false), "the Background is back on the short menu with the {key} lock off");
    }
}
