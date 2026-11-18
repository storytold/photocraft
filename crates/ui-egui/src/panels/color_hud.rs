//! Photoshop's HUD Color Picker (#6098): with a painting tool, Shift+Alt+right-click on the
//! canvas opens a floating saturation/value square with a hue strip to its right, placed so the
//! current colour sits under the pointer (as the reference application does).
//! While the right button stays down, moving the pointer over the HUD picks the foreground colour
//! live (`tools.setColors` calls sharing the gesture's `coalesce` key, so the gesture is one history step,
//! Rule 1); releasing the button or pressing Escape closes the HUD and keeps the colour picked so
//! far, as in the reference application (Undo takes it back). A press outside closes it too, which is
//! how the control channel ends a gesture it opened with `ui.pointer`.
//!
//! The framework's `ColorHud` draws the HUD in a popover and maps the pass pointer to a colour; this
//! module owns the gesture's state ([`Hud`]) and writes each picked colour to the foreground.

use craft::behavior::EditEvent;
use craft::color_ui::ui::ColorHud;
use craft::theme::Srgba;
use craft::ui::contract::Bind;
use egui::{PointerButton, pos2};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::PhotocraftApp;
use crate::paint_mouse::has_brush_picker;
use crate::state::UiState;

/// The HUD Color Picker while it is open.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hud {
    /// Where the gesture began, in screen points: the HUD opens with the starting colour here.
    pub anchor: [f32; 2],
    /// The foreground colour when the gesture began: where the HUD opens, and the alpha it keeps.
    pub start: [f32; 4],
    /// Begun by a real right-press: it ends when the right button comes up. One opened through
    /// the control channel ends with its Up event instead.
    pub hold: bool,
}

/// Is the HUD open?
pub fn is_open(ui: &UiState) -> bool {
    ui.color_hud.is_some()
}

/// Open the HUD at `at`, seeded from the current foreground colour. `hold` marks a real-mouse
/// gesture (see [`Hud::hold`]).
pub fn open(app: &mut PhotocraftApp, at: [f32; 2], hold: bool) {
    let fg = app.session.tools.foreground;
    app.ui.color_hud = Some(Hud { anchor: at, start: fg, hold });
}

/// Close the HUD, keeping the picked colour.
pub fn close(ui: &mut UiState) {
    ui.color_hud = None;
}

/// Close a control-channel HUD on its Up event; a real-mouse HUD closes on the button release.
pub fn release(ui: &mut UiState) {
    if ui.color_hud.is_some_and(|h| !h.hold) {
        close(ui);
    }
}

/// Draw the HUD and write the colours it picks: called once per frame while a gesture is open. The HUD reports a pick
/// only on a pass the pointer moved in, so every reported pick is written.
pub fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let Some(hud) = app.ui.color_hud else { return };
    if !has_brush_picker(app.ui.tool) {
        close(&mut app.ui);
        return;
    }
    let secondary_down = ui.input(|i| i.pointer.button_down(PointerButton::Secondary));
    let fg = app.session.tools.foreground;
    let mut color = Srgba::rgb(fg[0], fg[1], fg[2]);
    // A control-channel HUD ends through `release`, so its press counts as held.
    let held = if hud.hold { secondary_down } else { true };
    let out = ColorHud::new(tl!("Color"), pos2(hud.anchor[0], hud.anchor[1]), Bind::bound(&mut color))
        .start(Srgba::rgb(hud.start[0], hud.start[1], hud.start[2]))
        .held(held)
        .show(ui);
    for event in out.events.iter() {
        if let Some(picked) = event.value() {
            set_foreground(app, [picked.r, picked.g, picked.b, hud.start[3]], event);
        }
    }
    if out.closed.is_some() {
        close(&mut app.ui);
    }
}

/// One foreground-colour edit of the gesture: coalesced under the gesture's key, so the whole pick is a single
/// history step (the colour is tool state, so it never touches the document).
fn set_foreground(app: &mut PhotocraftApp, rgba: [f32; 4], event: &EditEvent<Srgba>) {
    let mut p = json!({ "foreground": rgba });
    if let Some(key) = crate::history_keys::coalesce("color-hud", event) {
        p["coalesce"] = json!(key);
    }
    if let Err(e) = app.run("tools.setColors", p) {
        app.ui.status = e;
    }
}

#[cfg(test)]
mod tests {
    use egui::{Modifiers, PointerButton, Pos2, Rect, Vec2, vec2};
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use serde_json::json;

    use super::*;
    use crate::state::Tool;

    const SHIFT_ALT: Modifiers = Modifiers { alt: true, ctrl: false, shift: true, mac_cmd: false, command: false };

    fn harness() -> Harness<'static, PhotocraftApp> {
        harness_size(400, 300)
    }

    fn harness_size(width: u32, height: u32) -> Harness<'static, PhotocraftApp> {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": width, "height": height})).unwrap();
        app.run("tools.setColors", json!({"foreground": "#0000ff"})).unwrap();
        app.ui.tool = Tool::Brush;
        app.sync_views();
        let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1200.0, 800.0)).with_step_dt(1.0 / 60.0).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                let ctx = ui.ctx().clone();
                if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
        h.run_steps(4);
        h
    }

    fn button(h: &mut Harness<'static, PhotocraftApp>, pos: Pos2, pressed: bool, modifiers: Modifiers) {
        h.event(egui::Event::ModifiersChanged(modifiers));
        h.event(egui::Event::PointerButton { pos, button: PointerButton::Secondary, pressed, modifiers });
        h.run_steps(2);
    }

    /// The HUD's picking content (the colour square and the hue strip beside it), found by its accessibility node: the
    /// smallest node named "Color", since the popover around it carries the same name. The square is as tall as
    /// the content is high; the strip stands in the content's right part.
    fn content(h: &Harness<'static, PhotocraftApp>) -> Rect {
        h.query_all_by_label("Color").map(|node| node.rect()).min_by(|a, b| a.area().total_cmp(&b.area())).expect("the HUD is shown")
    }

    /// The colour square's rect in `content`.
    fn square(content: Rect) -> Rect {
        Rect::from_min_size(content.min, Vec2::splat(content.height()))
    }

    /// A point on the hue strip in `content`: a quarter of the way in from its right edge across the part right of the
    /// square, which holds the gap and the strip.
    fn strip_point(content: Rect) -> Pos2 {
        let beside = content.width() - content.height();
        pos2(content.right() - beside / 4.0, content.center().y)
    }

    fn foreground(h: &Harness<'static, PhotocraftApp>) -> [f32; 4] {
        h.state().session.tools.foreground
    }

    #[test]
    fn shift_alt_right_press_opens_the_hud_not_the_picker() {
        let mut h = harness();
        let c = h.state().last_canvas_rect.center();
        h.event(egui::Event::PointerMoved(c));
        h.run_steps(1);
        button(&mut h, c, true, SHIFT_ALT);
        let hud = h.state().ui.color_hud.expect("the HUD opened");
        assert!(hud.hold);
        assert_eq!(hud.start, [0.0, 0.0, 1.0, 1.0], "seeded from the foreground");
        assert_eq!(h.state().ui.brush_picker, None, "the Brush Preset picker stays closed");
        // Nothing painted.
        assert!(!h.state().session.journal.iter().any(|(id, _)| id == "paint.stroke"));
    }

    /// Quick Selection's canvas right-click is the selection menu (#7268), so Shift+Alt+right
    /// must not open the HUD from the same gesture (#6098).
    #[test]
    fn quick_selections_shift_alt_right_opens_only_the_selection_menu() {
        let mut h = harness();
        h.state_mut().ui.tool = Tool::QuickSelection;
        h.run_steps(1);
        let c = h.state().last_canvas_rect.center();
        h.event(egui::Event::PointerMoved(c));
        h.run_steps(1);
        button(&mut h, c, true, SHIFT_ALT);
        assert_eq!(h.state().ui.color_hud, None, "no HUD for Quick Selection");
        assert_eq!(h.state().ui.brush_picker, None, "and no Brush Preset picker");
        button(&mut h, c, false, SHIFT_ALT);
        assert_eq!(h.state().ui.color_hud, None, "the release does not open it either");
        assert!(h.state().ui.canvas_tool_menu.is_some(), "the selection menu opens, once");
    }

    #[test]
    fn dragging_over_the_hud_picks_the_foreground_live_and_release_keeps_it() {
        let mut h = harness();
        let c = h.state().last_canvas_rect.center();
        h.event(egui::Event::PointerMoved(c));
        h.run_steps(1);
        button(&mut h, c, true, SHIFT_ALT);
        let j0 = h.state().session.journal.len();
        let content = content(&h);
        let square = square(content);
        // Top-right of the square: full saturation, full value at the current hue (blue).
        let pick = pos2(square.max.x - 1.0, square.min.y + 1.0);
        h.event(egui::Event::PointerMoved(pick));
        h.run_steps(2);
        let fg = foreground(&h);
        assert!(fg[2] > 0.9 && fg[0] < 0.1 && fg[1] < 0.1, "still blue at the top of the square: {fg:?}");
        // Bottom-left: value 0 → black.
        h.event(egui::Event::PointerMoved(square.min + vec2(1.0, square.height() - 1.0)));
        h.run_steps(2);
        let fg = foreground(&h);
        assert!(fg[..3].iter().all(|v| *v < 0.02), "black at the bottom: {fg:?}");
        // The hue strip: its middle is cyan-ish (180°), at the square's current s/v.
        h.event(egui::Event::PointerMoved(pick));
        h.run_steps(2);
        let strip = strip_point(content);
        h.event(egui::Event::PointerMoved(strip));
        h.run_steps(2);
        let fg = foreground(&h);
        assert!(fg[2] > 0.5 && fg[0] < 0.5, "hue moved off red: {fg:?}");
        // The gesture's edits share one coalesce key (one history step, like the brush HUD).
        let sets: Vec<&serde_json::Value> = h.state().session.journal[j0..].iter().filter(|(id, _)| id == "tools.setColors").map(|(_, p)| p).collect();
        assert!(!sets.is_empty(), "the picks are journaled");
        let keys: Vec<&str> = sets.iter().map(|p| p["coalesce"].as_str().unwrap()).collect();
        assert!(keys.windows(2).all(|w| w[0] == w[1]), "one gesture, one key: {keys:?}");
        // Releasing the right button closes the HUD and keeps the colour.
        let kept = foreground(&h);
        button(&mut h, strip, false, SHIFT_ALT);
        assert_eq!(h.state().ui.color_hud, None);
        assert_eq!(foreground(&h), kept);
    }

    #[test]
    fn plain_right_click_still_opens_the_picker_and_alt_drag_still_resizes() {
        let mut h = harness();
        let c = h.state().last_canvas_rect.center();
        h.event(egui::Event::PointerMoved(c));
        h.run_steps(1);
        button(&mut h, c, true, Modifiers::NONE);
        button(&mut h, c, false, Modifiers::NONE);
        assert!(h.state().ui.brush_picker.is_some() && h.state().ui.color_hud.is_none());
        // Alt+right-drag (no Shift) resizes the brush, as before.
        h.state_mut().ui.brush_picker = None;
        h.run_steps(1);
        h.event(egui::Event::ModifiersChanged(Modifiers::ALT));
        h.event(egui::Event::PointerMoved(c));
        h.run_steps(1);
        button(&mut h, c, true, Modifiers::ALT);
        h.event(egui::Event::PointerMoved(c + vec2(40.0, 0.0)));
        h.run_steps(2);
        assert!(h.state().brush_resize.active(), "Alt+right still resizes");
        assert!(h.state().ui.color_hud.is_none());
        button(&mut h, c + vec2(40.0, 0.0), false, Modifiers::ALT);
    }

    #[test]
    fn shift_alt_right_drag_does_not_resize_the_brush() {
        let mut h = harness();
        let size0 = h.state().session.tools.brush.size;
        let c = h.state().last_canvas_rect.center();
        h.event(egui::Event::PointerMoved(c));
        h.run_steps(1);
        button(&mut h, c, true, SHIFT_ALT);
        for k in 1..=6 {
            h.event(egui::Event::PointerMoved(c + vec2(20.0 * k as f32, 0.0)));
            h.run_steps(1);
        }
        assert!(!h.state().brush_resize.active(), "the HUD gesture is not a resize");
        assert_eq!(h.state().session.tools.brush.size, size0);
        button(&mut h, c + vec2(120.0, 0.0), false, SHIFT_ALT);
    }

    #[test]
    fn the_control_channel_opens_and_closes_the_hud() {
        let mut h = harness();
        crate::canvas::testing::window(h.state_mut(), vec2(1200.0, 800.0));
        // The document's centre, in document points as `ui.pointer` takes them.
        let c = [200.0, 150.0];
        let pointer = |h: &mut Harness<'static, PhotocraftApp>, events: serde_json::Value| {
            crate::canvas::testing::control(h.state_mut(), "ui.pointer", json!({"button": "right", "shift": true, "alt": true, "events": events}));
        };
        pointer(&mut h, json!([{"kind": "down", "x": c[0], "y": c[1]}, {"kind": "up", "x": c[0], "y": c[1]}]));
        assert!(h.state().ui.color_hud.is_none(), "the Up event closed it");
        assert!(h.state().ui.brush_picker.is_none(), "the picker stayed closed");
        // A down without its up leaves the HUD open for the next frame's edits.
        pointer(&mut h, json!([{"kind": "down", "x": c[0], "y": c[1]}]));
        assert!(h.state().ui.color_hud.is_some(), "a press without its release leaves the HUD open");
        assert!(h.state().ui.brush_picker.is_none(), "the picker stayed closed");
    }
}
