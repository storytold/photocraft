//! Photoshop menu enablement the engine's per-command checks don't express: the Background layer
//! (no styles, masks, clipping, arranging, grouping, transforms without a selection), layer-kind
//! specific items (Rasterize ›, Combine Shapes ›), and single-layer documents (Merge Visible,
//! Flatten Image, Load Selection without a saved selection).
//! Applied on top of `menus::is_enabled`, so menus, shortcuts and the Layers context menu grey the
//! same items Photoshop greys.

use photocraft_doc::{Document, Layer, LayerContent, LayerId};

/// Commands Photoshop greys while the Background layer is the active layer.
fn background_blocks(id: &str, has_selection: bool) -> bool {
    let prefixes = ["layer.layerMask.", "layer.vectorMask.", "layer.arrange.", "layer.matting."];
    if prefixes.iter().any(|p| id.starts_with(p)) {
        return true;
    }
    if id.starts_with("layer.layerStyle.") {
        // Global Light and Hide All Effects are document-wide.
        return !matches!(id, "layer.layerStyle.globalLight" | "layer.layerStyle.hideAllEffects" | "layer.layerStyle.copyLayerStyle");
    }
    // Transforms and Content-Aware Scale need a selection on the Background (they then move the
    // selected pixels).
    if !has_selection && (matches!(id, "edit.freeTransform" | "edit.contentAwareScale") || (id.starts_with("edit.transform.") && id != "edit.transform.again"))
    {
        return true;
    }
    matches!(
        id,
        "layer.createClippingMask"
            | "layer.releaseClippingMask"
            | "layer.mergeDown"
            // The Background can't go into a group (#12866, measured on the canvas menu).
            | "layer.new.groupFromLayers"
            | "layer.lockLayers"
            | "layer.maskAllObjects"
            | "edit.puppetWarp"
            | "edit.perspectiveWarp"
    )
}

/// Commands that only apply to some layer kinds.
fn kind_blocks(id: &str, l: &Layer) -> bool {
    let c = &l.content;
    match id {
        "layer.rasterize.type" | "layer.rasterize.rasterizeType" => !matches!(c, LayerContent::Text(_)),
        "layer.rasterize.shape" | "layer.rasterize.rasterizeShape" => !matches!(c, LayerContent::Shape(_)),
        "layer.rasterize.fillContent" => !matches!(c, LayerContent::Fill(_)),
        "layer.rasterize.smartObject" => !matches!(c, LayerContent::Smart(_)),
        "layer.rasterize.vectorMask" => l.vector_mask.is_none(),
        // What the command converts: type, shape, fill and Smart Object layers, and vector masks
        // ("all vector data"). Effects stay live: Rasterize › Layer Style bakes them.
        "layer.rasterize.layer" => {
            l.vector_mask.is_none() && !matches!(c, LayerContent::Text(_) | LayerContent::Shape(_) | LayerContent::Fill(_) | LayerContent::Smart(_))
        }
        "layer.rasterize.layerStyle" => !photocraft_engine::rasterize_style_cmds::can_bake(l),
        "layer.layerStyle.copyLayerStyle" | "layer.layerStyle.clear" => l.effects.items.is_empty(),
        _ => id.starts_with("layer.combineShapes.") && !matches!(c, LayerContent::Shape(_)),
    }
}

/// Is `id` greyed in Photoshop for this document state, regardless of the engine's own check?
pub fn disabled_for(doc: &Document, active: Option<&Layer>, id: &str) -> bool {
    let only_background = doc.layers.len() == 1 && doc.layers.first().is_some_and(|b| crate::doc_props_ui::is_background(doc, b));
    if only_background && matches!(id, "layer.mergeVisible" | "layer.flattenImage" | "layer.mergeLayers" | "layer.rasterize.allLayers") {
        return true;
    }
    // A lone Background has no transparency or mask to load: Load Selection waits for a saved
    // selection or Quick Mask (#13293). The command itself still loads the colour channels, as a
    // Ctrl-click on a channel does.
    if only_background && id == "select.loadSelection" && doc.channels.is_empty() && doc.quick_mask.is_none() {
        return true;
    }
    if id == "layer.delete.hiddenLayers" && doc.walk().iter().all(|(_, _, l)| l.visible) {
        return true;
    }
    let Some(l) = active else { return false };
    if crate::doc_props_ui::is_background(doc, l) && background_blocks(id, doc.selection.is_some()) {
        return true;
    }
    kind_blocks(id, l)
}

/// How a command that acts on the whole layer selection decides it is available.
#[derive(Clone, Copy)]
enum Targets {
    /// Some selected layer can take it.
    Any,
    /// Some selected layer other than the lowest can (Create Clipping Mask clips every selected layer
    /// but the lowest, which becomes the base).
    AllButLowest,
    /// Some selected layer without a layer mask can take one.
    WithoutMask,
}

/// The commands that run on every selected layer, and so are judged by the selection and not by
/// the active layer (the same selection used to grey or enable them depending on which of its
/// layers happened to be active).
fn selection_targets(id: &str) -> Option<Targets> {
    Some(match id {
        "layer.rasterize.layer"
        | "layer.rasterize.layerStyle"
        | "layer.rasterize.type"
        | "layer.rasterize.shape"
        | "layer.rasterize.fillContent"
        | "layer.rasterize.smartObject" => Targets::Any,
        "layer.releaseClippingMask" | "layer.layerStyle.pasteLayerStyle" | "layer.layerStyle.clear" => Targets::Any,
        "layer.createClippingMask" => Targets::AllButLowest,
        "layer.layerMask.revealAll" | "layer.layerMask.hideAll" | "layer.layerMask.revealSelection" | "layer.layerMask.hideSelection" => Targets::WithoutMask,
        _ => return None,
    })
}

/// [`disabled_for`] with the layer selection in mind: with several layers selected, a command that
/// runs on all of them is greyed only when none of them can take it (the active layer alone must
/// not decide). Everything else, and a single selected layer, is judged by the active layer.
pub fn disabled_for_selection(doc: &Document, selected: &[LayerId], active: Option<&Layer>, id: &str) -> bool {
    // The reference application refuses Group from Layers whenever the Background is among the
    // selected layers, not only when it is the active one.
    if id == "layer.new.groupFromLayers" && selected.iter().filter_map(|l| doc.layer(*l)).any(|l| crate::doc_props_ui::is_background(doc, l)) {
        return true;
    }
    let Some(rule) = selection_targets(id).filter(|_| selected.len() > 1) else { return disabled_for(doc, active, id) };
    let layers = selected.iter().filter_map(|l| doc.layer(*l));
    let can = |l: &Layer| !disabled_for(doc, Some(l), id) && (!matches!(rule, Targets::WithoutMask) || l.mask.is_none());
    let any = match rule {
        Targets::AllButLowest => layers.skip(1).any(&can),
        Targets::Any | Targets::WithoutMask => layers.into_iter().any(can),
    };
    !any
}

pub fn disabled(app: &crate::PhotocraftApp, id: &str) -> bool {
    let Some(st) = app.session.active() else { return false };
    let active = st.active_layer.and_then(|a| st.doc.layer(a));
    // The split warps edit the open Warp box's grid, or a smart object's stored warp: without
    // either there is no grid to split (#12494).
    if matches!(
        id,
        "edit.transform.splitWarpCrosswise" | "edit.transform.splitWarpHorizontally" | "edit.transform.splitWarpVertically" | "edit.transform.removeWarpSplit"
    ) && !crate::transform_tool::warps_active_layer(app)
    {
        return true;
    }
    // Remove Warp Split takes out a split line of the open grid; with none (a single patch, as
    // the engine's `Split::Remove` sees it) there is nothing to remove.
    if id == "edit.transform.removeWarpSplit"
        && app.ui.transform.as_ref().and_then(|t| t.session.warp()).map(|w| &w.warp).is_some_and(|w| {
            let m = w.to_mesh(1, 1);
            m.us.len() <= 2 && m.vs.len() <= 2
        })
    {
        return true;
    }
    // Free Transform Path works on the Background too: it moves the path, not the pixels.
    let transform = id == "edit.freeTransform" || (id.starts_with("edit.transform.") && id != "edit.transform.again");
    if transform && crate::vector_ui::free_transform_path(app).is_some() {
        return false;
    }
    // So does a targeted alpha channel or the Quick Mask: it transforms instead of the layer (#2835).
    let target = crate::canvas::paint_target(app);
    if transform && (target.is_object() || target == "quickMask") {
        return false;
    }
    // Several layers selected: one box moves them all (#6501); with the Background among them it
    // refuses as locked when it opens (#11101). It doesn't warp, and doesn't distort while
    // editable type is among them.
    if transform {
        let targets = photocraft_engine::transform_cmds::selection_targets(st);
        // Judged on the layers the box moves, linked ones included (#11217): one linked layer
        // selected is a box over its link group, which doesn't warp either.
        let targets = photocraft_engine::transform_cmds::box_targets(&st.doc, &targets);
        if targets.len() > 1 || targets.first() != st.active_layer.as_ref() {
            let text = targets.iter().any(|t| st.doc.layer(*t).is_some_and(|l| matches!(l.content, LayerContent::Text(_))));
            return match id {
                "edit.transform.warp"
                | "edit.transform.splitWarpCrosswise"
                | "edit.transform.splitWarpHorizontally"
                | "edit.transform.splitWarpVertically"
                | "edit.transform.removeWarpSplit" => true,
                "edit.transform.distort" | "edit.transform.perspective" => text,
                _ => false,
            };
        }
    }
    // Editable type can be scaled, rotated and skewed but not distorted (#2630): Photoshop greys
    // Distort and Perspective until the type is rasterized, and the split warps (Warp on type
    // offers only the warp styles, #3004). A targeted unlinked mask or channel of the type layer
    // is pixels, and can be.
    if matches!(
        id,
        "edit.transform.distort"
            | "edit.transform.perspective"
            | "edit.transform.splitWarpCrosswise"
            | "edit.transform.splitWarpHorizontally"
            | "edit.transform.splitWarpVertically"
            | "edit.transform.removeWarpSplit"
    ) && active.is_some_and(|l| matches!(l.content, LayerContent::Text(_)))
        && photocraft_engine::transform_cmds::lone_target(&st.doc, st.active_layer, &serde_json::json!({ "target": crate::canvas::paint_target(app) }))
            .is_ok_and(|t| t.is_none())
    {
        return true;
    }
    // Only these commands depend on the selection; skip walking the layers for the rest.
    if selection_targets(id).is_some() || id == "layer.new.groupFromLayers" {
        return disabled_for_selection(&st.doc, st.selected_layers(), active, id);
    }
    disabled_for(&st.doc, active, id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn session() -> photocraft_engine::Session {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 20, "height": 20})).unwrap();
        s
    }

    type Setup = fn(&mut photocraft_engine::Session);

    fn layer_of(r: serde_json::Value) -> LayerId {
        LayerId(r["layer"].as_u64().unwrap())
    }

    fn lone_background(_: &mut photocraft_engine::Session) {}

    fn background_with_selection(s: &mut photocraft_engine::Session) {
        s.execute("select.all", json!({})).unwrap();
    }

    /// A targeted alpha channel transforms instead of the Background (#2835).
    fn background_with_alpha_target(s: &mut photocraft_engine::Session) {
        s.execute("channel.new", json!({})).unwrap();
        s.execute("channel.target", json!({"channel": 0})).unwrap();
    }

    fn plain_layer(s: &mut photocraft_engine::Session) {
        s.execute("layer.new.layer", json!({})).unwrap();
    }

    /// A type layer and a raster layer selected, the raster layer active.
    fn type_and_raster(s: &mut photocraft_engine::Session) {
        let raster = layer_of(s.execute("layer.new.layer", json!({})).unwrap());
        let text = layer_of(s.execute("type.create", json!({"text": "Hi", "size": 12, "x": 2, "y": 12})).unwrap());
        s.execute("layer.select", json!({"layer": text.0})).unwrap();
        s.execute("layer.select", json!({"layer": raster.0, "mode": "add"})).unwrap();
    }

    fn background_and_raster(s: &mut photocraft_engine::Session) {
        let bg = s.active().unwrap().doc.layers[0].id;
        let raster = layer_of(s.execute("layer.new.layer", json!({})).unwrap());
        s.execute("layer.select", json!({"layer": bg.0})).unwrap();
        s.execute("layer.select", json!({"layer": raster.0, "mode": "add"})).unwrap();
    }

    /// Three layers, the first two linked, the first selected (#11217).
    fn linked_layer_selected(s: &mut photocraft_engine::Session) -> LayerId {
        let a = layer_of(s.execute("layer.new.layer", json!({})).unwrap());
        let b = layer_of(s.execute("layer.new.layer", json!({})).unwrap());
        let c = layer_of(s.execute("layer.new.layer", json!({})).unwrap());
        s.execute("layer.select", json!({"layer": a.0})).unwrap();
        s.execute("layer.select", json!({"layer": b.0, "mode": "add"})).unwrap();
        assert_eq!(s.execute("layer.linkLayers", json!({})).unwrap()["linked"], true);
        s.execute("layer.select", json!({"layer": a.0})).unwrap();
        c
    }

    fn linked_partner(s: &mut photocraft_engine::Session) {
        linked_layer_selected(s);
    }

    fn unlinked_layer(s: &mut photocraft_engine::Session) {
        let c = linked_layer_selected(s);
        s.execute("layer.select", json!({"layer": c.0})).unwrap();
    }

    /// Document state → command → greyed, through the menu's own `disabled`.
    #[test]
    fn enable_rules_follow_the_document() {
        type Case<'a> = (&'a str, Setup, &'a [(&'a str, bool)]);
        let cases: &[Case<'_>] = &[
            (
                "a lone Background greys styles, masks, arranging, merges and transforms",
                lone_background,
                &[
                    ("layer.layerStyle.dropShadow", true),
                    ("layer.layerMask.revealAll", true),
                    ("layer.arrange.bringForward", true),
                    ("layer.mergeDown", true),
                    ("edit.freeTransform", true),
                    ("edit.transform.scale", true),
                    ("edit.contentAwareScale", true),
                    ("layer.flattenImage", true),
                    ("layer.mergeVisible", true),
                    ("layer.createClippingMask", true),
                    ("layer.rasterize.layer", true),
                    ("layer.combineShapes.unite", true),
                    ("layer.delete.hiddenLayers", true),
                    ("layer.layerStyle.globalLight", false),
                ],
            ),
            ("a selection frees the Background's transforms", background_with_selection, &[("edit.freeTransform", false), ("edit.contentAwareScale", false)]),
            (
                "a targeted alpha channel transforms instead of the Background",
                background_with_alpha_target,
                &[("edit.freeTransform", false), ("edit.transform.flipHorizontal", false)],
            ),
            ("a normal layer takes Content-Aware Scale without a selection", plain_layer, &[("edit.contentAwareScale", false)]),
            (
                "Rasterize follows the whole selection, not the active layer",
                type_and_raster,
                &[("layer.rasterize.layer", false), ("layer.rasterize.type", false), ("layer.rasterize.shape", true), ("layer.rasterize.smartObject", true)],
            ),
            ("nothing selected can be rasterized", background_and_raster, &[("layer.rasterize.layer", true)]),
            (
                "a layer with linked partners does not warp",
                linked_partner,
                &[("edit.transform.warp", true), ("edit.freeTransform", false), ("edit.transform.distort", false)],
            ),
            ("an unlinked layer warps", unlinked_layer, &[("edit.transform.warp", false)]),
            (
                "an ordinary layer keeps its items",
                plain_layer,
                &[
                    ("layer.layerStyle.dropShadow", false),
                    ("layer.layerMask.revealAll", false),
                    ("layer.mergeDown", false),
                    ("edit.freeTransform", false),
                    ("layer.flattenImage", false),
                    ("layer.rasterize.type", true),
                    ("layer.layerStyle.copyLayerStyle", true),
                ],
            ),
        ];
        for (name, setup, wants) in cases {
            let mut s = session();
            setup(&mut s);
            let app = crate::PhotocraftApp::new(s, crate::Services::default());
            for (id, greyed) in *wants {
                assert_eq!(disabled(&app, id), *greyed, "{name}: {id}");
            }
        }
    }

    /// A document with a Background, plain layers, one with a mask, one with a vector mask, a type layer, a shape, a Smart
    /// Object, an adjustment layer and one with a drop shadow; `select` picks a selection (the last
    /// layer is active) and `copied` has a layer style on the clipboard.
    fn world(select: &[&str], copied: bool) -> photocraft_engine::Session {
        use photocraft_doc::LayerId;
        let mut s = session();
        let id = |r: serde_json::Value| LayerId(r["layer"].as_u64().unwrap());
        let mut ids = std::collections::HashMap::new();
        ids.insert("BG", s.active().unwrap().doc.layers[0].id);
        ids.insert("R1", id(s.execute("layer.new.layer", json!({"name": "R1"})).unwrap()));
        ids.insert("R2", id(s.execute("layer.new.layer", json!({"name": "R2"})).unwrap()));
        s.execute("layer.layerMask.revealAll", json!({})).unwrap();
        ids.insert("V", id(s.execute("layer.new.layer", json!({"name": "V"})).unwrap()));
        s.execute("layer.vectorMask.revealAll", json!({})).unwrap();
        ids.insert("T", id(s.execute("type.create", json!({"text": "T", "size": 12, "x": 2, "y": 12})).unwrap()));
        ids.insert("S", id(s.execute("shape.create", json!({"kind": "rect", "rect": [4, 4, 6, 6]})).unwrap()));
        s.execute("layer.new.layer", json!({"name": "M"})).unwrap();
        s.execute("paint.stroke", json!({"points": [[10, 10]], "size": 4})).unwrap();
        ids.insert("M", id(s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap()));
        ids.insert("A", id(s.execute("layer.newAdjustmentLayer.hueSaturation", json!({"hue": 10})).unwrap()));
        ids.insert("FX", id(s.execute("layer.new.layer", json!({"name": "FX"})).unwrap()));
        s.execute("layer.layerStyle.dropShadow", json!({})).unwrap();
        if copied {
            s.execute("layer.layerStyle.copyLayerStyle", json!({"layer": ids["FX"].0})).unwrap();
        }
        s.execute("layer.select", json!({"layer": ids[select[0]].0})).unwrap();
        for name in &select[1..] {
            s.execute("layer.select", json!({"layer": ids[name].0, "mode": "add"})).unwrap();
        }
        s
    }

    /// What a layer command can change: kind, clipping, masks and style of every layer.
    fn fingerprint(s: &photocraft_engine::Session) -> Vec<String> {
        s.active()
            .unwrap()
            .doc
            .walk()
            .iter()
            .map(|(_, _, l)| {
                format!("{:?}|{}|{}|{}|{}", std::mem::discriminant(&l.content), l.clipped, l.mask.is_some(), l.vector_mask.is_some(), l.effects.items.len())
            })
            .collect()
    }

    /// The menu item and the command agree for every selection, whichever of its layers is active:
    /// an enabled item runs, a greyed one would have changed nothing. (The same selection used to be
    /// greyed or not depending on which layer was active, and Rasterize Layer was enabled for layers
    /// it then refused.)
    #[test]
    fn enabled_commands_run_and_greyed_ones_would_do_nothing() {
        // (command, an enabled item must really change something)
        let commands = [
            ("layer.rasterize.layer", true),
            ("layer.rasterize.layerStyle", true),
            ("layer.createClippingMask", true),
            ("layer.layerMask.revealAll", true),
            ("layer.layerMask.hideAll", true),
            ("layer.layerStyle.pasteLayerStyle", true),
            ("layer.layerStyle.clear", true),
            ("layer.releaseClippingMask", false),
        ];
        let selections: &[&[&str]] = &[
            &["R1", "R2"],
            &["R2", "R1"],
            &["BG", "R1"],
            &["R1", "BG"],
            &["T", "S"],
            &["S", "T"],
            &["M", "R1"],
            &["R1", "M"],
            &["BG", "R1", "M", "A"],
            &["R1", "A"],
            &["A", "R1"],
            &["FX", "R1"],
            &["R1", "FX"],
            &["BG", "FX", "T", "S", "M", "A"],
            &["R2", "A"],
            &["BG", "R2"],
            &["T"],
            &["R1"],
            &["BG"],
            &["FX"],
            &["V", "R1"],
            &["R1", "V"],
            &["V"],
            &["V", "FX", "T"],
        ];
        let mut checked = 0;
        for sel in selections {
            for copied in [false, true] {
                // `is_enabled` only reads the app, so one app per world answers for every command; each command then
                // runs on a world of its own. A copied style is read by Paste Layer Style alone (the engine's
                // `style_clipboard`), so the other commands run without it only (#15652).
                let app = crate::PhotocraftApp::new(world(sel, copied), crate::Services::default());
                for (cmd, must_change) in commands {
                    if copied && cmd != "layer.layerStyle.pasteLayerStyle" {
                        continue;
                    }
                    let enabled = crate::menus::is_enabled(&app, cmd);
                    let mut s = world(sel, copied);
                    let before = fingerprint(&s);
                    let r = s.execute(cmd, json!({}));
                    let changed = fingerprint(&s) != before;
                    let what = format!(
                        "{cmd} for {sel:?} (copied style: {copied}), enabled={enabled}, result={:?}",
                        r.as_ref().map(|_| ()).map_err(|e| e.to_string())
                    );
                    if enabled {
                        assert!(r.is_ok(), "an enabled item must run: {what}");
                        assert!(changed || !must_change || sel.len() < 2, "an enabled item must do something: {what}");
                    } else if sel.len() > 1 {
                        // For one layer the greying is the menu's own policy (the engine, for instance,
                        // lets a script clip the Background); several layers must be exact.
                        assert!(r.is_err() || !changed, "a greyed item would have changed something: {what}");
                    }
                    checked += 1;
                }
            }
        }
        assert_eq!(checked, selections.len() * (commands.len() + 1));
    }

    /// #13293: on a lone Background, Group from Layers… and Load Selection… are greyed, as in the
    /// reference. A pixel selection doesn't change that; a saved selection enables Load Selection….
    #[test]
    fn a_lone_background_greys_group_from_layers_and_load_selection() {
        let mut app = crate::PhotocraftApp::new(session(), crate::Services::default());
        for id in ["layer.new.groupFromLayers", "select.loadSelection"] {
            assert!(!crate::menus::is_enabled(&app, id), "{id} is greyed on a lone Background");
        }
        app.session.execute("select.rect", json!({"x": 2, "y": 2, "width": 8, "height": 8})).unwrap();
        for id in ["layer.new.groupFromLayers", "select.loadSelection"] {
            assert!(!crate::menus::is_enabled(&app, id), "{id} is greyed with a pixel selection");
        }
        app.session.execute("select.saveSelection", json!({"name": "Keep"})).unwrap();
        app.session.execute("select.deselect", json!({})).unwrap();
        assert!(crate::menus::is_enabled(&app, "select.loadSelection"), "a saved selection can be loaded");
        assert!(!crate::menus::is_enabled(&app, "layer.new.groupFromLayers"), "the Background still can't be grouped");
        // The command itself still loads, as a Ctrl-click on a channel does.
        let mut s = session();
        s.execute("select.loadSelection", json!({"channel": "red"})).unwrap();
    }

    /// #13293: where they apply, both rows stay live: a plain layer, and a document with more than
    /// the Background or with Quick Mask on. Group from Layers… stays greyed whenever the Background
    /// is among the selected layers (#12866).
    #[test]
    fn group_from_layers_and_load_selection_stay_live_where_they_apply() {
        let mut s = session();
        let bg = s.active().unwrap().doc.layers[0].id;
        let layer = photocraft_doc::LayerId(s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap());
        let mut app = crate::PhotocraftApp::new(s, crate::Services::default());
        let on = |app: &crate::PhotocraftApp, id: &str| crate::menus::is_enabled(app, id);
        assert!(on(&app, "layer.new.groupFromLayers") && on(&app, "select.loadSelection"), "a plain layer");
        app.session.execute("layer.select", json!({"layer": bg.0, "mode": "add"})).unwrap();
        assert_eq!(app.session.active().unwrap().selected_layers().len(), 2);
        assert!(!on(&app, "layer.new.groupFromLayers"), "the Background with another layer selected (#12866)");
        app.session.execute("layer.select", json!({"layer": bg.0})).unwrap();
        assert!(!on(&app, "layer.new.groupFromLayers"), "the Background alone");
        assert!(on(&app, "select.loadSelection"), "another layer's transparency can be loaded");
        app.session.execute("layer.select", json!({"layer": layer.0})).unwrap();
        app.session.execute("layer.delete", json!({})).unwrap();
        assert!(!on(&app, "select.loadSelection"));
        app.session.execute("select.editInQuickMaskMode", json!({"on": true})).unwrap();
        assert!(on(&app, "select.loadSelection"), "Quick Mask keeps today's state");
    }
}

#[cfg(test)]
#[path = "warp_split_enable_tests.rs"]
mod warp_split_enable_tests;
