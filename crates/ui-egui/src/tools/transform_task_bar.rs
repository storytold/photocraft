//! The Free Transform contextual task bar: while a transform box is open on the canvas
//! (via Cmd+T, Ctrl+Shift+T, or the Free Transform button), a floating bar appears below the
//! transformed quad with shortcuts matching Photoshop's Contextual Task Bar:
//!
//! 1. the grip (widgets::task_bar);
//! 2. Rotate 90° Counter Clockwise (`rotate-ccw`);
//! 3. Rotate 90° Clockwise (`rotate-cw`);
//! 4. Flip Horizontal (`flip-horizontal`);
//! 5. Flip Vertical (`flip-vertical`);
//! 6. Cancel (cancels and exits transform mode);
//! 7. Done (commits and exits transform mode).

use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::ViewXform;
use crate::widgets::task_bar;

// ── guards ──────────────────────────────────────────────────────────────────

/// All conditions for the bar to show: Window › Contextual Task Bar is enabled, no dialog
/// is open, an active document is open, and a transform session is active.
fn wanted(app: &PhotocraftApp) -> bool {
    app.ui.panels.context_task_bar && app.ui.dialogs.is_empty() && app.ui.transform.is_some() && app.session.active().is_some()
}

/// Whether the bar can show now — called from `ui.inspect` so a bar hidden by `ui.set` before
/// the next draw no longer reports last frame's rect.
pub fn eligible(app: &PhotocraftApp) -> bool {
    wanted(app)
}

// ── anchor ──────────────────────────────────────────────────────────────────

/// The screen-space bounding box to anchor the bar below: all 4 corners of the transform
/// quad mapped through the view transform.
fn anchor(app: &PhotocraftApp, xf: &ViewXform) -> Option<egui::Rect> {
    let t = app.ui.transform.as_ref()?;
    let pts = t.quad.map(|[x, y]| xf.to_screen(x as f32, y as f32));
    let r = egui::Rect::from_points(&pts);
    if r.is_finite() && r.is_positive() {
        Some(r)
    } else {
        None
    }
}

// ── draw ─────────────────────────────────────────────────────────────────────

/// Draws the bar under the transform frame (the canvas overlay pass calls it on the active
/// document's tile).
pub fn draw(app: &mut PhotocraftApp, ui: &mut egui::Ui, xf: &ViewXform) {
    if !wanted(app) {
        return;
    }
    let Some(anchor_rect) = anchor(app, xf) else { return };

    let ctx = ui.ctx().clone();
    let mut place = app.ui.panels.task_bar;
    let shown = task_bar::show(&ctx, &mut place, anchor_rect, xf.rect, |ui, more| {
        contents(app, ui, more);
    });
    app.ui.panels.task_bar = place;

    let Some(shown) = shown else { return };
    if shown.hide {
        app.ui.panels.context_task_bar = false;
        return;
    }
    let r = shown.rect;
    app.ui.task_bar_shown = Some([r.min.x, r.min.y, r.max.x, r.max.y]);

    // A right-click over the transform task bar opens the transform context menu, as on the canvas.
    if ctx.input(|i| i.pointer.button_clicked(egui::PointerButton::Secondary))
        && let Some(p) = ctx.input(|i| i.pointer.interact_pos())
        && r.contains(p)
    {
        crate::canvas_tool_menu::open_transform(app, &ctx, [p.x, p.y]);
    }
}

// ── contents ─────────────────────────────────────────────────────────────────

fn contents(app: &mut PhotocraftApp, ui: &mut egui::Ui, more: &mut task_bar::MoreMenu) {
    // The Transform contextual task bar in the reference application does not have a ⋯ menu.
    more.suppress();

    let can_preset = app.ui.transform.as_ref().is_some_and(|t| t.warp.is_none()) && crate::transform_tool::committing(app).is_none();

    // 1. Rotate 90° CCW
    let ccw = ui
        .add_enabled_ui(can_preset, |ui| {
            crate::icons::button_with_icon_size_tip(ui, "rotate-ccw", task_bar::BUTTON, 14.0, false, tl!("Rotate 90° Counter Clockwise"), false)
        })
        .inner;
    if ccw.clicked()
        && let Err(e) = app.run("edit.transform.rotate90Ccw", json!({}))
    {
        crate::notices::error(app, e);
    }

    // 2. Rotate 90° CW
    let cw = ui
        .add_enabled_ui(can_preset, |ui| {
            crate::icons::button_with_icon_size_tip(ui, "rotate-cw", task_bar::BUTTON, 14.0, false, tl!("Rotate 90° Clockwise"), false)
        })
        .inner;
    if cw.clicked()
        && let Err(e) = app.run("edit.transform.rotate90Cw", json!({}))
    {
        crate::notices::error(app, e);
    }

    // 3. Flip Horizontal
    let flip_h = ui
        .add_enabled_ui(can_preset, |ui| {
            crate::icons::button_with_icon_size_tip(ui, "flip-horizontal", task_bar::BUTTON, 14.0, false, tl!("Flip Horizontal"), false)
        })
        .inner;
    if flip_h.clicked()
        && let Err(e) = app.run("edit.transform.flipHorizontal", json!({}))
    {
        crate::notices::error(app, e);
    }

    // 4. Flip Vertical
    let flip_v = ui
        .add_enabled_ui(can_preset, |ui| {
            crate::icons::button_with_icon_size_tip(ui, "flip-vertical", task_bar::BUTTON, 14.0, false, tl!("Flip Vertical"), false)
        })
        .inner;
    if flip_v.clicked()
        && let Err(e) = app.run("edit.transform.flipVertical", json!({}))
    {
        crate::notices::error(app, e);
    }

    // 5. Cancel
    if ui.add(egui::Button::new(tl!("Cancel")).min_size(egui::vec2(0.0, task_bar::BUTTON))).clicked() {
        crate::transform_tool::cancel(app);
    }

    // 6. Done
    if ui.add(egui::Button::new(tl!("Done")).min_size(egui::vec2(0.0, task_bar::BUTTON))).clicked() {
        crate::transform_tool::commit(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn app_with_doc() -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width": 400, "height": 200})).unwrap();
        app.sync_views();
        app
    }

    #[test]
    fn eligible_false_when_context_task_bar_off() {
        let mut app = app_with_doc();
        app.ui.panels.context_task_bar = false;
        assert!(!eligible(&app));
    }

    #[test]
    fn eligible_false_when_no_transform() {
        let app = app_with_doc();
        assert!(!eligible(&app));
    }

    #[test]
    fn eligible_true_during_transform() {
        let mut app = app_with_doc();
        app.ui.panels.context_task_bar = true;
        app.ui.transform = Some(crate::state::TransformSession {
            session: 1,
            layer: 0,
            rect: [10.0, 10.0, 110.0, 110.0],
            quad: [[10.0, 10.0], [110.0, 10.0], [110.0, 110.0], [10.0, 110.0]],
            pivot: [60.0, 60.0],
            interpolation: "nearest".into(),
            warp: None,
            selection: false,
            target: None,
            path: None,
            made: None,
            mode: Default::default(),
            content_aware: None,
            skew_v: 0.0,
            layers: Vec::new(),
        });
        assert!(eligible(&app));
    }

    #[test]
    fn anchor_maps_quad_points() {
        let mut app = app_with_doc();
        app.ui.transform = Some(crate::state::TransformSession {
            session: 1,
            layer: 0,
            rect: [0.0, 0.0, 100.0, 100.0],
            quad: [[10.0, 20.0], [110.0, 20.0], [110.0, 120.0], [10.0, 120.0]],
            pivot: [60.0, 70.0],
            interpolation: "nearest".into(),
            warp: None,
            selection: false,
            target: None,
            path: None,
            made: None,
            mode: Default::default(),
            content_aware: None,
            skew_v: 0.0,
            layers: Vec::new(),
        });
        let xf = ViewXform {
            rect: egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0)),
            zoom: 1.0,
            center: [200.0, 100.0],
            flip: false,
            rotation: 0.0,
            aspect: 1.0,
        };
        let r = anchor(&app, &xf).expect("anchor rect");
        assert_eq!(r.min.x.round(), 210.0);
        assert_eq!(r.min.y.round(), 220.0);
        assert_eq!(r.max.x.round(), 310.0);
        assert_eq!(r.max.y.round(), 320.0);
    }

    #[test]
    fn rotate_and_flip_transform_quad() {
        let mut app = app_with_doc();
        app.ui.panels.context_task_bar = true;
        app.ui.transform = Some(crate::state::TransformSession {
            session: 1,
            layer: 0,
            rect: [0.0, 0.0, 100.0, 100.0],
            quad: [[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]],
            pivot: [50.0, 50.0],
            interpolation: "nearest".into(),
            warp: None,
            selection: false,
            target: None,
            path: None,
            made: None,
            mode: Default::default(),
            content_aware: None,
            skew_v: 0.0,
            layers: Vec::new(),
        });

        // Rotate 90 CW
        app.run("edit.transform.rotate90Cw", json!({})).unwrap();
        let q = app.ui.transform.as_ref().unwrap().quad;
        assert_eq!([q[0][0].round(), q[0][1].round()], [100.0, 0.0]);
        assert_eq!([q[1][0].round(), q[1][1].round()], [100.0, 100.0]);

        // Rotate 90 CCW returns to start
        app.run("edit.transform.rotate90Ccw", json!({})).unwrap();
        let q = app.ui.transform.as_ref().unwrap().quad;
        assert_eq!([q[0][0].round(), q[0][1].round()], [0.0, 0.0]);
        assert_eq!([q[1][0].round(), q[1][1].round()], [100.0, 0.0]);

        // Flip Horizontal
        app.run("edit.transform.flipHorizontal", json!({})).unwrap();
        let q = app.ui.transform.as_ref().unwrap().quad;
        assert_eq!([q[0][0].round(), q[0][1].round()], [100.0, 0.0]);
        assert_eq!([q[1][0].round(), q[1][1].round()], [0.0, 0.0]);

        // Flip Horizontal again returns to start
        app.run("edit.transform.flipHorizontal", json!({})).unwrap();
        let q = app.ui.transform.as_ref().unwrap().quad;
        assert_eq!([q[0][0].round(), q[0][1].round()], [0.0, 0.0]);
        assert_eq!([q[1][0].round(), q[1][1].round()], [100.0, 0.0]);

        // Flip Vertical
        app.run("edit.transform.flipVertical", json!({})).unwrap();
        let q = app.ui.transform.as_ref().unwrap().quad;
        assert_eq!([q[0][0].round(), q[0][1].round()], [0.0, 100.0]);
        assert_eq!([q[1][0].round(), q[1][1].round()], [100.0, 100.0]);

        // Flip Vertical again returns to start
        app.run("edit.transform.flipVertical", json!({})).unwrap();
        let q = app.ui.transform.as_ref().unwrap().quad;
        assert_eq!([q[0][0].round(), q[0][1].round()], [0.0, 0.0]);
        assert_eq!([q[1][0].round(), q[1][1].round()], [100.0, 0.0]);
    }

    #[test]
    fn cancel_and_commit_exits_transform() {
        let mut app = app_with_doc();
        app.ui.panels.context_task_bar = true;
        app.ui.transform = Some(crate::state::TransformSession {
            session: 1,
            layer: 0,
            rect: [0.0, 0.0, 100.0, 100.0],
            quad: [[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]],
            pivot: [50.0, 50.0],
            interpolation: "nearest".into(),
            warp: None,
            selection: false,
            target: None,
            path: None,
            made: None,
            mode: Default::default(),
            content_aware: None,
            skew_v: 0.0,
            layers: Vec::new(),
        });
        assert!(eligible(&app));
        crate::transform_tool::cancel(&mut app);
        assert!(app.ui.transform.is_none());
        assert!(!eligible(&app));
    }

    #[test]
    fn transition_between_place_action_bar_and_transform_task_bar() {
        let mut app = app_with_doc();
        app.session.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        app.sync_views();
        app.ui.panels.context_task_bar = true;

        // Initially place action bar is eligible, transform task bar is not
        assert!(crate::tools::place_action_bar::eligible(&app));
        assert!(!eligible(&app));

        // Start Free Transform
        crate::menus::invoke(&mut app, &egui::Context::default(), "edit.freeTransform", json!({})).unwrap();
        assert!(app.ui.transform.is_some());

        // Now place action bar is hidden, transform task bar is eligible
        assert!(!crate::tools::place_action_bar::eligible(&app));
        assert!(eligible(&app));

        // Cancel Free Transform
        crate::transform_tool::cancel(&mut app);
        assert!(app.ui.transform.is_none());

        // Back to place action bar eligible
        assert!(crate::tools::place_action_bar::eligible(&app));
        assert!(!eligible(&app));
    }
}
