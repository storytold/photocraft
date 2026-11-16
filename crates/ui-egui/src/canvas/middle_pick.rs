//! Alt/Option key + middle-click on the canvas, with your pointer on any tool, it will cause you to select the topmost visible layer.
//! The alt key press is tracked in its entirity, and remains active until the middle click is pressed or the alt is let go.

use egui::{Context, Modifiers, PointerButton, Response};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::ViewXform;

/// Disables it when you are also holding the ctrl key.
pub fn is_gesture(mods: Modifiers) -> bool {
    mods.alt && !mods.command && !mods.ctrl && !mods.mac_cmd
}

/// Select the topmost layer that has pixels under your cursiour given x, y point.
pub fn pick_at(app: &mut PhotocraftApp, x: f64, y: f64) -> bool {
    if app.session.active().is_none() || !(x.is_finite() && y.is_finite()) {
        return false;
    }
    match app.run("layer.pickAt", json!({"x": x, "y": y, "target": "layer", "mode": "replace"})) {
        Ok(v) => !v["layer"].is_null(),
        Err(e) => {
            app.ui.status = e;
            app.ui.status_error = true;
            false
        }
    }
}

fn held_id(response: &Response) -> egui::Id {
    response.id.with("pc-alt-middle-pick")
}

/// Runs once per canvas frame. On an Alt + middle press, picks the layer under the pointer.
/// Returns true while that press is held, so the caller doesn't pan.
/// Nothing is picked when `free` is false (another window, a held temporary tool, a stroke being
/// dismissed), when a dialog is open, or while a tool drag is in progress.
pub fn frame(app: &mut PhotocraftApp, ctx: &Context, response: &Response, xf: &ViewXform, free: bool) -> bool {
    let free = free && app.ui.dialogs.is_empty() && app.drag.is_none();
    let id = held_id(response);
    let (pressed, down, origin, mods) =
        ctx.input(|i| (i.pointer.button_pressed(PointerButton::Middle), i.pointer.middle_down(), i.pointer.press_origin(), i.modifiers));
    let mut held = down && ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    let mods = crate::workspace_ui::sticky_mods(app, mods);
    if free
        && pressed
        && is_gesture(mods)
        && let Some(p) = origin.filter(|p| response.rect.contains(*p) && response.hovered())
    {
        let d = xf.to_doc(p);
        pick_at(app, d[0], d[1]);
        held = true;
    }
    ctx.data_mut(|d| d.insert_temp(id, held && down));
    held
}

#[cfg(test)]
mod tests {
    use egui::{Modifiers, Pos2, vec2};
    use egui_kittest::Harness;
    use serde_json::json;

    use super::*;
    use crate::state::Tool;

    /// 200 × 200: a white Background and "Red" (a square in the middle), with the Background
    /// active. Returns the app and the (background, red) ids.
    fn app() -> (PhotocraftApp, u64, u64) {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let bg = app.session.active().unwrap().active_layer.unwrap().0;
        app.run("layer.new.layer", json!({"name": "Red"})).unwrap();
        app.run("select.rect", json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap();
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        let red = app.session.active().unwrap().active_layer.unwrap().0;
        app.run("layer.select", json!({"layer": bg, "mode": "replace"})).unwrap();
        (app, bg, red)
    }

    fn active(app: &PhotocraftApp) -> u64 {
        app.session.active().unwrap().active_layer.unwrap().0
    }

    fn harness(a: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
        let mut a = a;
        a.sync_views();
        let mut h = Harness::builder().with_size(vec2(1000.0, 700.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                let ctx = ui.ctx().clone();
                if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
            },
            a,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::default());
        h.run_steps(4);
        h
    }

    /// A middle press at `from`, dragged to `to`, and released, with `modifiers` held throughout.
    fn middle_drag(h: &mut Harness<'static, PhotocraftApp>, from: Pos2, to: Pos2, modifiers: Modifiers) {
        h.event(egui::Event::ModifiersChanged(modifiers));
        h.event(egui::Event::PointerMoved(from));
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: from, button: PointerButton::Middle, pressed: true, modifiers });
        h.run_steps(1);
        for k in 1..=4 {
            h.event(egui::Event::PointerMoved(from + (to - from) * (k as f32 / 4.0)));
            h.run_steps(1);
        }
        h.event(egui::Event::PointerButton { pos: to, button: PointerButton::Middle, pressed: false, modifiers });
        h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
        h.run_steps(2);
    }

    fn center(h: &Harness<'static, PhotocraftApp>) -> [f32; 2] {
        h.state().ui.views[0].center
    }

    #[test]
    fn alt_middle_click_picks_the_layer_under_the_pointer_with_any_tool() {
        for tool in [Tool::Brush, Tool::RectMarquee, Tool::Move, Tool::Hand] {
            let (mut a, bg, red) = app();
            a.ui.tool = tool;
            let mut h = harness(a);
            let c = h.state().last_canvas_rect.center();
            let before = h.state().session.active().unwrap().history.past_len();
            middle_drag(&mut h, c, c, Modifiers::ALT);
            assert_eq!(active(h.state()), red, "{tool:?}: the Red square is under the centre");
            assert_eq!(h.state().ui.tool, tool, "{tool:?}: the tool stays");
            assert_eq!(h.state().session.active().unwrap().history.past_len(), before, "{tool:?}: nothing was painted");
            // Back to the Background: its corner is outside the square.
            let corner = h.state().last_canvas_rect.center() + vec2(-80.0, -80.0) * (h.state().ui.views[0].zoom / h.ctx.pixels_per_point());
            middle_drag(&mut h, corner, corner, Modifiers::ALT);
            assert_eq!(active(h.state()), bg, "{tool:?}: picked the Background");
        }
    }

    #[test]
    fn a_plain_middle_drag_still_pans_and_picks_nothing() {
        let (mut a, bg, _) = app();
        a.ui.tool = Tool::Brush;
        let mut h = harness(a);
        let c = h.state().last_canvas_rect.center();
        let before = center(&h);
        middle_drag(&mut h, c, c + vec2(60.0, 40.0), Modifiers::NONE);
        assert_eq!(active(h.state()), bg);
        assert_ne!(center(&h), before, "the middle button pans");
    }

    #[test]
    fn an_alt_middle_drag_picks_without_panning() {
        let (mut a, _, red) = app();
        a.ui.tool = Tool::Brush;
        let mut h = harness(a);
        let c = h.state().last_canvas_rect.center();
        let before = center(&h);
        middle_drag(&mut h, c, c + vec2(60.0, 40.0), Modifiers::ALT);
        assert_eq!(active(h.state()), red);
        assert_eq!(center(&h), before, "the pick's drag is swallowed");
    }

    #[test]
    fn command_or_ctrl_with_alt_is_not_the_gesture() {
        assert!(is_gesture(Modifiers::ALT));
        assert!(is_gesture(Modifiers { shift: true, ..Modifiers::ALT }));
        assert!(!is_gesture(Modifiers::NONE));
        assert!(!is_gesture(Modifiers { command: true, ctrl: true, ..Modifiers::ALT }));
        assert!(!is_gesture(Modifiers { command: true, mac_cmd: true, ..Modifiers::ALT }));
        assert!(!is_gesture(Modifiers { ctrl: true, ..Modifiers::ALT }));
    }

    #[test]
    fn a_pick_with_nothing_there_or_no_document_is_handled() {
        let (mut a, bg, _) = app();
        for (x, y) in [(f64::NAN, 1.0), (1.0, f64::INFINITY), (1e300, -1e300), (-5.0, 10.0), (500.0, 500.0)] {
            assert!(!pick_at(&mut a, x, y), "({x}, {y})");
            assert_eq!(active(&a), bg);
        }
        let mut b = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        assert!(!pick_at(&mut b, 1.0, 1.0), "no document");
    }
}
