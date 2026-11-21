use super::*;
use egui::{pos2, vec2};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use photocraft_doc::LayerContent;
use serde_json::json;

/// What the Properties panel is showing the controls of.
#[derive(Clone, Copy)]
enum Target {
    Pixel,
    Adjustment,
    Vector,
    Shape,
}

fn session_for(target: Target) -> photocraft_engine::Session {
    let mut session = photocraft_engine::Session::new();
    session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    match target {
        Target::Pixel | Target::Adjustment => {
            let command = if matches!(target, Target::Adjustment) { "layer.newAdjustmentLayer.curves" } else { "layer.new.layer" };
            session.execute(command, json!({})).unwrap();
            if session.active().unwrap().doc.layer(session.active().unwrap().active_layer.unwrap()).unwrap().mask.is_none() {
                session.execute("layer.layerMask.revealAll", json!({})).unwrap();
            }
        }
        Target::Vector => {
            session.execute("layer.new.layer", json!({})).unwrap();
            session.execute("layer.vectorMask.revealAll", json!({})).unwrap();
        }
        Target::Shape => {
            session.execute("shape.create", json!({"kind": "rect", "rect": [16, 16, 32, 32]})).unwrap();
        }
    }
    session
}

/// The Properties panel over a session, drawn in the Pro theme.
fn harness_of(session: photocraft_engine::Session, mask_target: bool, vector_target: bool) -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = PhotocraftApp::new(session, crate::Services::default());
    app.ui.mask_target = mask_target;
    app.ui.vector_mask_target = vector_target;
    let mut h = Harness::builder()
        .with_context(app.ctx().clone())
        .with_size(vec2(800.0, 1600.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| super::properties::panel(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
    frames(&mut h, 4);
    h
}

/// `n` frames as the app's frame runs them: the command states and catalog are published before the input (a command
/// button's label and enablement), and the commands the frame fired run after it.
fn frames(h: &mut Harness<'_, Box<PhotocraftApp>>, n: usize) {
    for _ in 0..n {
        let ctx = h.ctx.clone();
        h.state_mut().framework_before_input(&ctx);
        h.step();
        h.state_mut().framework_before_input(&ctx);
        crate::shortcuts::handle(h.state_mut(), &ctx);
    }
}

fn harness(target: Target) -> Harness<'static, Box<PhotocraftApp>> {
    harness_of(session_for(target), matches!(target, Target::Pixel | Target::Adjustment), matches!(target, Target::Vector))
}

fn mask(h: &Harness<'_, Box<PhotocraftApp>>) -> photocraft_doc::LayerMask {
    let s = h.state().session.active().unwrap();
    s.doc.layer(s.active_layer.unwrap()).unwrap().mask.clone().unwrap()
}

/// The Density of the active layer's mask, vector mask or shape.
fn density(h: &Harness<'_, Box<PhotocraftApp>>, target: Target) -> f32 {
    let s = h.state().session.active().unwrap();
    let layer = s.doc.layer(s.active_layer.unwrap()).unwrap();
    match target {
        Target::Pixel | Target::Adjustment => layer.mask.as_ref().unwrap().density,
        Target::Vector => layer.vector_mask.as_ref().unwrap().density,
        Target::Shape => match &layer.content {
            LayerContent::Shape(sh) => sh.density,
            other => panic!("not a shape: {}", other.kind_name()),
        },
    }
}

fn history(h: &Harness<'_, Box<PhotocraftApp>>) -> usize {
    h.state().session.active().unwrap().history.past_len()
}

#[test]
fn pixel_mask_properties_follow_target_for_raster_and_adjustment() {
    for target in [Target::Pixel, Target::Adjustment] {
        let adjustment = matches!(target, Target::Adjustment);
        let mut h = harness(target);
        for label in ["Density", "Feather", "Select and Mask…", "Color Range…", "Invert"] {
            assert!(h.query_all_by_label(label).next().is_some(), "adjustment={adjustment}: no {label}");
        }
        h.state_mut().ui.mask_target = false;
        frames(&mut h, 4);
        assert!(h.query_all_by_label("Density").next().is_none());
        assert!(h.query_all_by_label("Select and Mask…").next().is_none(), "the Refine row belongs to the mask target");
        if adjustment {
            assert!(h.query_all_by_label("Reset to defaults").next().is_some(), "Curves controls restored");
        }
        h.state_mut().ui.mask_target = true;
        h.state_mut().ui.vector_mask_target = true;
        frames(&mut h, 4);
        assert!(h.query_all_by_label("Density").next().is_none(), "a vector target must not edit the pixel mask");
    }
}

/// Drags the slider named `label` through `fractions` of its track, pressing at the first and releasing at the last.
fn drag(h: &mut Harness<'_, Box<PhotocraftApp>>, label: &str, fractions: &[f32]) {
    drag_with(h, label, fractions, false);
}

/// The drag of [`drag`]; with `escape` the Escape key is pressed before the release.
fn drag_with(h: &mut Harness<'_, Box<PhotocraftApp>>, label: &str, fractions: &[f32], escape: bool) {
    let r = h
        .query_all_by_role(egui::accesskit::Role::Slider)
        .find(|n| n.accesskit_node().label().as_deref() == Some(label))
        .unwrap_or_else(|| panic!("no {label} slider"))
        .rect();
    let pos = |f: f32| pos2(r.left() + 7.0 + (r.width() - 14.0) * f, r.center().y);
    let start = pos(fractions[0]);
    h.hover_at(start);
    frames(h, 1);
    h.event(egui::Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    frames(h, 1);
    for &f in &fractions[1..] {
        h.hover_at(pos(f));
        frames(h, 1);
    }
    if escape {
        h.key_press(egui::Key::Escape);
        frames(h, 1);
    }
    h.event(egui::Event::PointerButton {
        pos: pos(*fractions.last().unwrap()),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    frames(h, 3);
}

#[test]
fn mask_slider_drags_preview_live_and_undo_as_separate_gestures() {
    let mut h = harness(Target::Adjustment);
    let before = history(&h);
    drag(&mut h, "Density", &[1.0, 0.8, 0.6, 0.5]);
    assert!((mask(&h).density - 0.5).abs() < 0.01);
    assert_eq!(history(&h), before + 1);
    drag(&mut h, "Density", &[0.5, 0.4, 0.25]);
    assert!((mask(&h).density - 0.25).abs() < 0.01);
    assert_eq!(history(&h), before + 2);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert!((mask(&h).density - 0.5).abs() < 0.01);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(mask(&h).density, 1.0);
    h.state_mut().run("edit.redo", json!({})).unwrap();
    assert!((mask(&h).density - 0.5).abs() < 0.01);
    frames(&mut h, 3);
    // #6545: Feather's track is curved like Photoshop's, so a fifth of the way is 8 px, not 200.
    drag(&mut h, "Feather", &[0.0, 0.1, 0.2]);
    assert!((mask(&h).feather - 8.0).abs() < 0.3, "feather {}", mask(&h).feather);
    // Escape during a drag puts the value from before it back.
    drag_with(&mut h, "Density", &[0.5, 0.4, 0.2], true);
    assert!((mask(&h).density - 0.5).abs() < 0.01, "density {} after a cancelled drag", mask(&h).density);
}

/// A Density drag in each mask section runs that section's edit command: one undo step per gesture, and a
/// shape stays a shape (#5846).
#[test]
fn mask_sections_run_their_edit_commands() {
    for target in [Target::Pixel, Target::Vector, Target::Shape] {
        let mut h = harness(target);
        let before = history(&h);
        drag(&mut h, "Density", &[1.0, 0.8, 0.5]);
        assert!((density(&h, target) - 0.5).abs() < 0.02, "density {}", density(&h, target));
        assert_eq!(history(&h), before + 1, "one step per drag");
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(density(&h, target), 1.0);
    }
}

/// Document-wide row of mask coverage (or of the layer's own red channel) across the middle.
fn row(h: &Harness<'_, Box<PhotocraftApp>>, of_mask: bool) -> Vec<f32> {
    let s = h.state().session.active().unwrap();
    let layer = s.doc.layer(s.active_layer.unwrap()).unwrap();
    let surface = if of_mask { &layer.mask.as_ref().unwrap().surface } else { layer.surface().unwrap() };
    (0..64).map(|x| surface.sample_channel(x, 32, 0)).collect()
}

#[test]
fn refine_invert_inverts_the_mask_and_leaves_the_layer_alone() {
    let mut h = harness(Target::Pixel);
    let gradient = |target: &str| json!({"from": [0, 0], "to": [63, 0], "colors": ["#000000", "#ffffff"], "target": target});
    // Pixels and mask both vary, so inverting the wrong surface shows up either way (#1137).
    h.state_mut().session.execute("paint.gradient", gradient("pixels")).unwrap();
    h.state_mut().session.execute("paint.gradient", gradient("mask")).unwrap();
    frames(&mut h, 3);
    let (mask_before, pixels_before) = (row(&h, true), row(&h, false));
    assert!(mask_before[8] < mask_before[56], "the mask must vary across the document");
    let steps = history(&h);
    h.get_by_label("Invert").click();
    frames(&mut h, 3);
    for (x, (before, after)) in mask_before.iter().zip(row(&h, true)).enumerate() {
        assert!((1.0 - before - after).abs() < 0.01, "mask at x={x}: {before} should invert to {}, got {after}", 1.0 - before);
    }
    assert_eq!(row(&h, false), pixels_before, "the layer's pixels must not be touched");
    assert_eq!(history(&h), steps + 1, "one undoable step");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(row(&h, true), mask_before, "Undo restores the mask");
}

#[test]
fn the_background_shows_the_document_properties() {
    let mut session = photocraft_engine::Session::new();
    session.execute("file.new", json!({"width": 64, "height": 64, "background": "white"})).unwrap();
    let h = harness_of(session, false, false);
    for label in ["Canvas", "Rulers & Grids", "Guides"] {
        assert!(h.query_all_by_label(label).next().is_some(), "no {label} section");
    }
}
