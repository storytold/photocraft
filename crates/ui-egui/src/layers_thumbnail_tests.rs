//! What a click or double-click on a Layers panel thumbnail runs in PhotoCraft: the panel reports
//! the part and modifiers (`ThumbClick`, `Activate`), PhotoCraft picks the command.

use craft::layers::ui::tree::LayerPart;
use egui::Modifiers;
use egui_kittest::{Harness, kittest::Queryable};
use photocraft_doc::LayerMask;
use photocraft_engine::Session;
use serde_json::json;

use crate::PhotocraftApp;
use crate::layers_panel_nodes::{click, drag, layer_id, name_point, thumb};

type H = Harness<'static, Box<PhotocraftApp>>;

fn harness(s: Session) -> H {
    let mut h = Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::with_context(cc.egui_ctx.clone(), s, crate::Services::default())
    });
    let ctx = h.ctx.clone();
    let (r, _) = crate::control::ControlRequest::new("ui.set", json!({"dock": {"collapsed": ["color", "properties", "history", "navigator"]}}));
    crate::control::handle(h.state_mut(), &ctx, &r);
    h.run_steps(8);
    h
}

fn active(h: &H) -> Option<String> {
    let st = h.state().session.active().unwrap();
    st.active_layer.map(|id| st.doc.layer(id).unwrap().name.clone())
}

fn selected(h: &H) -> Vec<String> {
    let st = h.state().session.active().unwrap();
    st.selected_layers().iter().copied().map(|id| st.doc.layer(id).unwrap().name.clone()).collect()
}

/// The selection's bounds as (x, y, width, height).
fn selection(h: &H) -> Option<(u32, u32, u32, u32)> {
    let b = h.state().session.active().unwrap().doc.selection.as_ref()?.content_bounds();
    Some((b.x0 as u32, b.y0 as u32, b.width(), b.height()))
}

/// 400 × 300: "Shape" (black pixels at 100, 50, 120 × 100), "Small" (black at 300, 200, 20 × 10),
/// "Badge" (a shape layer at 40, 20, 60 × 40) and "Top", active.
fn stack() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
    for (name, rect) in [("Shape", [100, 50, 120, 100]), ("Small", [300, 200, 20, 10])] {
        s.execute("layer.new.layer", json!({"name": name})).unwrap();
        s.execute("select.rect", json!({"x": rect[0], "y": rect[1], "width": rect[2], "height": rect[3]})).unwrap();
        s.execute("edit.fill", json!({"contents": "black"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
    }
    s.execute("shape.create", json!({"kind": "rect", "rect": [40, 20, 60, 40], "fill": "#cc3333", "name": "Badge"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Top"})).unwrap();
    s
}

/// Cmd-click loads the thumbnail's layer as a selection without selecting the layer (a shape
/// layer's too, as one undo step; a hidden layer's too; with Extras hidden the new selection shows
/// its edges), except on the Background, which has no transparency: there it is a plain click,
/// which selects the Background alone. Alt-click fits the layer in the window.
#[test]
fn modifier_clicks_on_a_layer_thumbnail() {
    type Case = (&'static str, fn(&mut H), &'static str, Modifiers, fn(&mut H));
    let none: fn(&mut H) = |_| {};
    let cases: [Case; 7] = [
        ("Cmd loads the pixels", none, "Shape", Modifiers::COMMAND, |h| {
            assert_eq!(selection(h), Some((100, 50, 120, 100)));
            assert_eq!((active(h).as_deref(), selected(h)), (Some("Top"), vec!["Top".to_string()]), "the selected layer stays");
        }),
        ("Cmd loads a shape layer, as one step", none, "Badge", Modifiers::COMMAND, |h| {
            assert_eq!(selection(h), Some((40, 20, 60, 40)), "{}", h.state().ui.status);
            assert_eq!(active(h).as_deref(), Some("Top"));
            h.state_mut().run("edit.undo", json!({})).unwrap();
            assert_eq!(selection(h), None);
            h.state_mut().run("edit.redo", json!({})).unwrap();
            assert_eq!(selection(h), Some((40, 20, 60, 40)));
        }),
        (
            "Cmd loads a hidden layer",
            |h| {
                let id = layer_id(h.state(), "Shape").0;
                h.state_mut().run("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
                h.run_steps(2);
            },
            "Shape",
            Modifiers::COMMAND,
            |h| assert_eq!(selection(h), Some((100, 50, 120, 100))),
        ),
        (
            "Cmd with Extras hidden shows the new selection's edges",
            |h| {
                let ctx = h.ctx.clone();
                crate::menus::invoke(h.state_mut(), &ctx, "view.extras", json!({})).unwrap();
                assert!(!h.state().ui.view.extras);
            },
            "Badge",
            Modifiers::COMMAND,
            |h| {
                assert_eq!(selection(h), Some((40, 20, 60, 40)));
                assert!(h.state().ui.view.shows(h.state().ui.view.show.selection_edges));
            },
        ),
        ("Cmd on the Background is a plain click", none, "Background", Modifiers::COMMAND, |h| {
            assert_eq!(selection(h), None);
            assert_eq!((active(h).as_deref(), selected(h)), (Some("Background"), vec!["Background".to_string()]));
        }),
        (
            "Cmd on the Background in a selection of two selects it alone",
            |h| {
                let id = layer_id(h.state(), "Background").0;
                h.state_mut().run("layer.select", json!({"layer": id, "mode": "add"})).unwrap();
                h.run_steps(2);
                assert_eq!(selected(h).len(), 2);
            },
            "Background",
            Modifiers::COMMAND,
            |h| assert_eq!(selected(h), ["Background"]),
        ),
        ("Alt fits the layer", none, "Small", Modifiers::ALT, |h| {
            assert_eq!(active(h).as_deref(), Some("Small"));
            assert_eq!(h.state().ui.views[0].center, [310.0, 205.0], "centred on the layer's content");
        }),
    ];
    for (case, before, name, mods, after) in cases {
        let mut h = harness(stack());
        before(&mut h);
        let zoom = h.state().ui.views[0].zoom;
        let at = thumb(&h, name, LayerPart::Pixels).center();
        click(&mut h, at, mods);
        eprintln!("{case}");
        after(&mut h);
        if mods == Modifiers::ALT {
            assert!(h.state().ui.views[0].zoom > zoom * 4.0, "zoomed in on 20 × 10 px");
        }
    }
}

fn made(v: serde_json::Value) -> u64 {
    v["layer"].as_u64().unwrap()
}

/// Edit Text (the type thumbnail's double-click) on no document or a non-type layer changes nothing.
#[test]
fn edit_type_command_rejects_missing_or_non_type_layers_without_changes() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    for has_document in [false, true] {
        if has_document {
            app.run("file.new", json!({"width": 32, "height": 24})).unwrap();
        }
        let tool = app.ui.tool;
        assert!(!crate::menus::is_enabled(&app, "type.editText"));
        assert!(crate::menus::invoke(&mut app, &ctx, "type.editText", json!({})).is_err());
        assert_eq!(app.ui.tool, tool);
        assert!(app.ui.text_edit.is_none());
    }
}

/// On a masked artboard, as on any masked layer: Alt-clicking the mask thumbnail shows the mask,
/// and clicking the board's thumbnail ends the mask view and targets the board.
#[test]
fn a_masked_artboards_thumbnail_retargets_it_and_ends_the_mask_view() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Inside"})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#336699"})).unwrap();
    let board = made(s.execute("layer.new.artboardFromLayers", json!({"name": "Board"})).unwrap());
    s.execute("layer.layerMask.revealAll", json!({"layer": board})).unwrap();
    s.execute("layer.select", json!({"layer": board})).unwrap();
    let mut h = harness(s);
    let viewing = |h: &H| h.state().session.active().and_then(photocraft_engine::mask_view_cmds::current).is_some();
    let at = thumb(&h, "Board", LayerPart::LayerMask).center();
    click(&mut h, at, Modifiers::ALT);
    assert!(viewing(&h), "Alt-click shows the mask");
    let at = thumb(&h, "Board", LayerPart::Pixels).center();
    click(&mut h, at, Modifiers::NONE);
    assert!(!h.state().ui.mask_target, "the board is targeted again");
    assert!(!viewing(&h), "and the mask view ends");
}

fn add_masked_adjustment(s: &mut Session) {
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Ink"})).unwrap();
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 24, "height": 16})).unwrap();
    s.execute("layer.layerMask.revealSelection", json!({})).unwrap();
    s.execute("layer.newAdjustmentLayer.brightnessContrast", json!({"name": "Tone", "brightness": 12})).unwrap();
}

fn source_and_adjustment_masks(h: &H) -> [LayerMask; 2] {
    let doc = &h.state().session.active().unwrap().doc;
    ["Ink", "Tone"].map(|name| doc.layer(layer_id(h.state(), name)).unwrap().mask.as_ref().unwrap().clone())
}

fn drag_mask_to_adjustment(h: &mut H, mods: Modifiers) {
    let from = thumb(h, "Ink", LayerPart::LayerMask).center();
    let to = name_point(h, "Tone");
    drag(h, from, to, mods);
}

#[test]
fn alt_drag_asks_before_replacing_an_adjustment_mask() {
    let mut s = Session::new();
    add_masked_adjustment(&mut s);
    let mut h = harness(s);
    let original = source_and_adjustment_masks(&h);
    assert_ne!(original[0], original[1]);
    let history = |h: &H| h.state().session.active().unwrap().history.past_len();

    let before = history(&h);
    drag_mask_to_adjustment(&mut h, Modifiers::NONE);
    assert!(h.query_by_label("Replace").is_none());
    assert_eq!(source_and_adjustment_masks(&h), original);

    drag_mask_to_adjustment(&mut h, Modifiers::ALT);
    assert_eq!(source_and_adjustment_masks(&h), original);
    assert!(h.query_by_label("Replace").is_some(), "Alt-drag to an existing adjustment mask asks first");
    h.get_by_label("Cancel").click();
    h.run_steps(4);
    assert_eq!(source_and_adjustment_masks(&h), original);
    assert_eq!(history(&h), before, "dismissing the question does not edit the document");

    drag_mask_to_adjustment(&mut h, Modifiers::ALT);
    h.get_by_label("Replace").click();
    h.run_steps(4);
    assert_eq!(source_and_adjustment_masks(&h), [original[0].clone(), original[0].clone()]);
    assert_eq!(history(&h), before + 1);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(source_and_adjustment_masks(&h), original);
}

#[test]
fn alt_drag_replacement_does_not_run_after_a_document_switch_or_edit() {
    for switch_document in [false, true] {
        let mut s = Session::new();
        add_masked_adjustment(&mut s);
        // Opening a saved copy can keep the original's layer IDs. Give both copies the same revision,
        // so a test that forgot to check document identity would replace the mask in the other copy.
        let original = s.active().unwrap();
        let revision = original.revision;
        s.add_document((*original.doc).clone(), None);
        s.active_mut().unwrap().revision = revision;
        assert_ne!(s.documents()[0].doc.id, s.documents()[1].doc.id);
        s.set_active(0);
        let mut h = harness(s);
        let original = source_and_adjustment_masks(&h);
        drag_mask_to_adjustment(&mut h, Modifiers::ALT);
        assert!(h.query_by_label("Replace").is_some());
        if switch_document {
            h.state_mut().session.set_active(1);
        } else {
            let target = layer_id(h.state(), "Tone").0;
            h.state_mut().run("layer.layerMask.enabled", json!({"layer": target, "enabled": false})).unwrap();
        }
        h.run_steps(3);
        let expected = source_and_adjustment_masks(&h);
        h.get_by_label("Replace").click();
        h.run_steps(4);
        assert_eq!(source_and_adjustment_masks(&h), expected);
        if switch_document {
            h.state_mut().session.set_active(0);
            h.run_steps(3);
            assert_eq!(source_and_adjustment_masks(&h), original);
        }
    }
}

#[test]
fn alt_drag_replacement_does_not_run_after_closing_and_reopening() {
    let mut s = Session::new();
    add_masked_adjustment(&mut s);
    let saved = (*s.close(0).unwrap().doc).clone();
    let path = Some("saved-masks.pcraft".to_owned());
    s.add_document(saved.clone(), path.clone());
    let mut h = harness(s);
    let original = source_and_adjustment_masks(&h);
    let identity = |h: &H| {
        let active = h.state().session.active().unwrap();
        (active.doc.id, active.revision)
    };
    let before = identity(&h);
    drag_mask_to_adjustment(&mut h, Modifiers::ALT);
    assert!(h.query_by_label("Replace").is_some());

    let session = &mut h.state_mut().session;
    session.close(0).unwrap();
    session.add_document(saved, path);
    h.run_steps(3);
    assert_eq!(identity(&h), before, "reopening can reuse the same ID and revision");
    h.get_by_label("Replace").click();
    h.run_steps(4);
    assert_eq!(source_and_adjustment_masks(&h), original, "the old approval does not apply to the newly opened document");
}
