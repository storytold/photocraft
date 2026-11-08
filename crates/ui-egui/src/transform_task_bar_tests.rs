//! Free Transform's contextual task bar through the real canvas and existing commands.

use egui::{Key, Rect, pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::ViewXform;
use crate::control::{ControlRequest, Outcome, handle};
use crate::transform_undo_tests::{begin, harness};

type AppHarness = Harness<'static, PhotocraftApp>;

/// Settle the floating area's measured size before exercising its controls.
fn open_box() -> AppHarness {
    let mut h = harness();
    begin(&mut h);
    h.run_steps(4);
    h
}

/// Send a synchronous command through the same control path as the offscreen example.
fn control(h: &mut AppHarness, method: &str, params: Value) -> Value {
    let ctx = h.ctx.clone();
    let (request, _reply) = ControlRequest::new(method, params);
    let Outcome::Done(reply) = handle(h.state_mut(), &ctx, &request) else { panic!("{method} should reply immediately") };
    assert_eq!(reply["ok"], true, "{method}: {reply}");
    reply["result"].clone()
}

/// Read the bar's current visibility and position through the public inspection route.
fn bar(h: &mut AppHarness) -> Value {
    control(h, "ui.inspect", json!({}))["taskBar"].clone()
}

/// Count committed document edits, excluding changes made only inside the open box.
fn history(h: &AppHarness) -> usize {
    h.state().session.active().unwrap().history.past_len()
}

/// Click the actual accessible widget and allow the canvas to redraw.
fn click(h: &mut AppHarness, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(4);
}

/// Each preset must match the real right-click entry and keep the document history untouched.
#[test]
fn transform_task_bar_presets_match_the_context_menu() {
    for (label, id) in [
        ("Rotate 90° Counter Clockwise", "edit.transform.rotate90Ccw"),
        ("Rotate 90° Clockwise", "edit.transform.rotate90Cw"),
        ("Flip Horizontal", "edit.transform.flipHorizontal"),
        ("Flip Vertical", "edit.transform.flipVertical"),
    ] {
        let mut toolbar = open_box();
        let mut menu = open_box();
        let before = toolbar.state().ui.transform.as_ref().unwrap().quad;
        let (toolbar_history, menu_history) = (history(&toolbar), history(&menu));
        click(&mut toolbar, label);
        control(&mut menu, "ui.pointer", json!({"button": "secondary", "events": [{"kind": "down", "x": 150, "y": 130}, {"kind": "up", "x": 150, "y": 130}]}));
        control(&mut menu, "ui.context.choose", json!({"id": id}));
        let quad = toolbar.state().ui.transform.as_ref().unwrap().quad;
        assert_ne!(quad, before, "{label}");
        assert_eq!(quad, menu.state().ui.transform.as_ref().unwrap().quad, "{label}");
        assert_eq!(history(&toolbar), toolbar_history);
        assert_eq!(history(&menu), menu_history);
    }
}

/// Done commits once like Enter, while Cancel discards the box like Escape.
#[test]
fn transform_task_bar_done_and_cancel_match_the_keys() {
    let pixels = |h: &AppHarness| {
        let st = h.state().session.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().read_region(st.doc.bounds())
    };
    for (label, key, edits) in [("Done", Key::Enter, 1), ("Cancel", Key::Escape, 0)] {
        let mut toolbar = open_box();
        let mut keys = open_box();
        let original = pixels(&toolbar);
        for h in [&mut toolbar, &mut keys] {
            click(h, "Rotate 90° Clockwise");
        }
        let (toolbar_history, key_history) = (history(&toolbar), history(&keys));
        click(&mut toolbar, label);
        keys.key_press(key);
        keys.run_steps(4);
        assert!(toolbar.state().ui.transform.is_none() && keys.state().ui.transform.is_none(), "{label}");
        assert_eq!(history(&toolbar), toolbar_history + edits);
        assert_eq!(history(&keys), key_history + edits);
        let actual = pixels(&toolbar);
        assert!(actual == pixels(&keys), "{label} should produce the same pixels as the key");
        assert_eq!(actual != original, edits != 0, "{label}");
        assert_eq!(bar(&mut toolbar)["visible"], false);
    }
}

/// The unpinned bar stays centered eleven points below the actual quad on screen.
#[test]
fn transform_task_bar_tracks_the_rotated_quad() {
    let mut h = open_box();
    for turn in [false, true] {
        if turn {
            click(&mut h, "Rotate 90° Clockwise");
        }
        let app = h.state();
        let view = &app.ui.views[0];
        let xf = ViewXform {
            rect: crate::rulers::content_rect(app, app.last_canvas_rect),
            zoom: view.zoom,
            center: view.center,
            flip: app.ui.view.flip_horizontal,
            rotation: view.rotation,
            aspect: app.ui.view.display_aspect(),
        };
        let quad = app.ui.transform.as_ref().unwrap().quad.map(|[x, y]| xf.to_screen(x as f32, y as f32));
        let anchor = Rect::from_points(&quad);
        let [x0, y0, x1, y1] = crate::transform_task_bar::shown_rect(app, &h.ctx).unwrap();
        let shown = Rect::from_min_max(pos2(x0, y0), pos2(x1, y1));
        assert!((shown.center().x - anchor.center().x).abs() <= 1.0, "{shown:?} below {anchor:?}");
        assert!((shown.top() - anchor.bottom() - crate::widgets::task_bar::GAP).abs() <= 1.0, "{shown:?} below {anchor:?}");
        assert_eq!(bar(&mut h)["visible"], true);
    }
}

/// Warp and Content-Aware Scale keep their own controls, and the Crop bar still draws.
#[test]
fn transform_task_bar_keeps_warp_content_aware_scale_and_crop_separate() {
    let mut warp = open_box();
    control(&mut warp, "ui.menu.invoke", json!({"id": "edit.transform.warp"}));
    warp.run_steps(4);
    assert!(warp.state().ui.transform.as_ref().unwrap().warp.is_some());
    assert_eq!(bar(&mut warp)["visible"], false);

    let mut scale = harness();
    control(&mut scale, "ui.menu.invoke", json!({"id": "edit.contentAwareScale"}));
    scale.run_steps(4);
    assert!(scale.state().ui.transform.as_ref().unwrap().content_aware.is_some());
    assert_eq!(bar(&mut scale)["visible"], false);

    let mut crop = harness();
    control(&mut crop, "ui.set", json!({"tool": "crop"}));
    crop.state_mut().ui.crop_rect = Some([100.0, 100.0, 200.0, 160.0]);
    crop.run_steps(4);
    assert!(crate::crop_task_bar::eligible(crop.state()));
    assert!(!crate::transform_task_bar::eligible(crop.state()));
    assert_eq!(bar(&mut crop)["visible"], true);
}

/// The shared menu pins, resets and hides the bar, and the Window command restores it.
#[test]
fn transform_task_bar_uses_the_shared_menu_and_window_toggle() {
    let mut h = open_box();
    click(&mut h, "More options");
    click(&mut h, "Pin bar position");
    assert_eq!(bar(&mut h)["pinned"], true);
    assert!(h.state().ui.panels.task_bar.at.is_some());

    click(&mut h, "More options");
    click(&mut h, "Reset bar position");
    assert_eq!(bar(&mut h)["pinned"], false);
    assert!(h.state().ui.panels.task_bar.at.is_none());

    click(&mut h, "More options");
    click(&mut h, "Hide bar");
    assert_eq!(bar(&mut h)["enabled"], false);
    assert_eq!(bar(&mut h)["visible"], false);
    assert!(h.state().ui.transform.is_some());
    control(&mut h, "ui.menu.invoke", json!({"id": "window.panel.contextualTaskBar"}));
    h.run_steps(4);
    assert_eq!(bar(&mut h)["visible"], true);
}

/// A switch before the next canvas pass must not expose the previous document's Done action.
#[test]
fn transform_task_bar_ignores_a_foreign_preview_after_a_document_switch() {
    let mut h = harness();
    h.state_mut().run("file.new", json!({"width": 400, "height": 300})).unwrap();
    h.state_mut().run("document.activate", json!({"document": 0})).unwrap();
    begin(&mut h);
    h.run_steps(4);
    let owner = h.state().transform_preview.as_ref().unwrap().doc.id;
    let quad = h.state().ui.transform.as_ref().unwrap().quad;
    assert_eq!(bar(&mut h)["visible"], true);

    control(&mut h, "ui.menu.invoke", json!({"id": "window.nextDocument"}));
    assert_ne!(h.state().session.active().unwrap().doc.id, owner);
    assert_eq!(h.state().transform_preview.as_ref().unwrap().doc.id, owner);
    assert_eq!(bar(&mut h)["visible"], false);

    assert_eq!(h.state().ui.transform.as_ref().unwrap().quad, quad);
}

#[test]
fn transform_task_bar_hides_while_a_shell_dialog_is_open() {
    let mut h = open_box();
    let session = h.state().ui.transform.as_ref().unwrap().session;
    assert_eq!(bar(&mut h)["visible"], true);
    control(&mut h, "ui.menu.invoke", json!({"id": "window.workspace.newWorkspace"}));
    assert!(h.state().ui.shell.dialog.is_some() && h.state().ui.dialogs.is_empty());
    assert_eq!(bar(&mut h)["visible"], false);
    h.run_steps(2);
    assert!(h.query_by_label("Rotate 90° Clockwise").is_none());
    control(&mut h, "ui.dialog.cancel", json!({"dialog": "shell"}));
    assert!(bar(&mut h)["rect"].is_null(), "the last pass did not draw the bar");
    h.run_steps(4);
    assert_eq!(h.state().ui.transform.as_ref().unwrap().session, session);
    assert_eq!(bar(&mut h)["visible"], true);
}

#[test]
fn transform_task_bar_inspection_keeps_crop_and_free_transform_rects_separate_before_redraw() {
    let mut h = open_box();
    assert_eq!(bar(&mut h)["visible"], true);
    control(
        &mut h,
        "ui.pointer",
        json!({"tool": "crop", "events": [{"kind": "down", "x": 40, "y": 40}, {"kind": "move", "x": 230, "y": 170}, {"kind": "up", "x": 230, "y": 170}]}),
    );
    assert!(crate::crop_task_bar::eligible(h.state()));
    assert!(bar(&mut h)["rect"].is_null(), "Crop must wait for its own draw");
    h.run_steps(4);
    assert_eq!(bar(&mut h)["visible"], true);

    // An untouched Crop frame can leave the tool without committing a document edit.
    let mut h = harness();
    control(&mut h, "ui.set", json!({"tool": "crop"}));
    h.run_steps(4);
    assert_eq!(bar(&mut h)["visible"], true);
    control(&mut h, "ui.set", json!({"tool": "move"}));
    control(&mut h, "ui.menu.invoke", json!({"id": "edit.freeTransform"}));
    assert!(crate::transform_task_bar::eligible(h.state()));
    assert!(bar(&mut h)["rect"].is_null(), "Free Transform must wait for its own draw");
    h.run_steps(4);
    assert_eq!(bar(&mut h)["visible"], true);
}

/// Accessibility can deliver Hide and Done during the same pass.
#[test]
fn transform_task_bar_commits_when_done_and_hide_arrive_together() {
    let mut h = open_box();
    click(&mut h, "Rotate 90° Clockwise");
    let edits = history(&h);
    click(&mut h, "More options");

    // The harness runs queued clicks in separate frames. Deliver both actions in one input.
    let events = ["Hide bar", "Done"].map(|label| {
        let (target_node, target_tree) = h.get_by_label(label).accesskit_node().locate();
        egui::Event::AccessKitActionRequest(egui::accesskit::ActionRequest { action: egui::accesskit::Action::Click, target_node, target_tree, data: None })
    });
    h.input_mut().events.extend(events);
    h.run_steps(4);

    assert!(!h.state().ui.panels.context_task_bar);
    assert!(h.state().ui.transform.is_none());
    assert_eq!(history(&h), edits + 1);
}

/// A Crop frame created synchronously after Cancel cannot inherit the Transform rect.
#[test]
fn transform_task_bar_does_not_report_its_rect_as_crop_after_cancel() {
    let mut h = open_box();
    let prior_rect = crate::transform_task_bar::shown_rect(h.state(), &h.ctx).unwrap();
    let cancel = h.get_by_label("Cancel").rect().center();
    let ctx = h.ctx.clone();
    let mut input = h.input().clone();
    input.events = vec![
        egui::Event::PointerMoved(cancel),
        egui::Event::PointerButton { pos: cancel, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
        egui::Event::PointerButton { pos: cancel, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
    ];
    let app = h.state_mut();
    let view = &app.ui.views[0];
    let xf = ViewXform {
        rect: crate::rulers::content_rect(app, app.last_canvas_rect),
        zoom: view.zoom,
        center: view.center,
        flip: app.ui.view.flip_horizontal,
        rotation: view.rotation,
        aspect: app.ui.view.display_aspect(),
    };
    ctx.run_ui(input, |ui| {
        if app.ui.transform.is_some() {
            assert_eq!(crate::transform_task_bar::shown_rect(app, ui.ctx()), Some(prior_rect));
            crate::transform_task_bar::draw(app, ui, &xf);
        }
    })
    .textures_delta
    .clear();
    assert!(h.state().ui.transform.is_none());

    // No UI pass between the Crop request and inspection.
    control(
        &mut h,
        "ui.pointer",
        json!({"tool": "crop", "events": [{"kind": "down", "x": 40, "y": 40}, {"kind": "move", "x": 230, "y": 170}, {"kind": "up", "x": 230, "y": 170}]}),
    );
    assert!(crate::crop_task_bar::eligible(h.state()));
    let crop_bar = bar(&mut h);
    assert_eq!(crop_bar["visible"], false);
    assert!(crop_bar["rect"].is_null());
}
