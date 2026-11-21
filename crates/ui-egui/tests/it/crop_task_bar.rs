//! Crop tool: the contextual task bar under the crop frame (#10890).
//!
//! The bar's place under the frame (follow, grip drag, Reset, Pin) is the framework task bar's;
//! this file keeps the app's rules: Hide bar hides it and unticks Window › Contextual Task Bar;
//! Cancel and Done show only while a crop is pending and do what the options bar's cancel and
//! commit do.
//!
//! The whole app runs in an egui_kittest harness (no GPU needed); the bar's own buttons are
//! pressed with pointer events, so a click must reach the bar and not the canvas under it.

use egui::{Modifiers, PointerButton, Pos2, Rect, Vec2, pos2, vec2};
use egui_kittest::kittest::Queryable;
use photocraft_ui_egui::PhotocraftApp;
use photocraft_ui_egui::control::{ControlRequest, Reply, handle};
use photocraft_ui_egui::crop_ui::{frame, pending};
use serde_json::{Value, json};

type Harness = egui_kittest::Harness<'static, Box<PhotocraftApp>>;

/// The app in English with a 400 × 300 white document at 100 %, the Crop tool picked in Classic
/// Mode (the frame moves over a fixed image, so the frame's screen place is easy to predict).
fn harness() -> Harness {
    let mut h = Harness::builder().with_size(vec2(1100.0, 760.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), Default::default());
        // English labels, whatever the system language.
        app.session.prefs.edit(|p| p.interface.language = "en".into());
        app.run("file.new", json!({"width": 400, "height": 300, "background": "white"})).expect("new");
        app.ui.extras.snap = false;
        app.ui.view.show.smart_guides = false;
        app.ui.tool_options.crop.shield.classic_mode = true;
        app
    });
    h.run_steps(4);
    control(&mut h, "ui.set", json!({"tool": "crop", "zoom": 1.0, "center": [200.0, 150.0]}));
    h.run_steps(6);
    h
}

/// Runs one control-channel request, which must succeed, then lets the app draw.
fn control(h: &mut Harness, method: &str, params: Value) -> Value {
    let ctx = h.ctx.clone();
    let (req, _rx) = ControlRequest::new(method, params.clone());
    let v = match handle(h.state_mut(), &ctx, &req) {
        Reply::Done(v) => {
            assert_eq!(v["ok"], true, "{method} {params}: {v}");
            v
        }
        // Input the canvas takes on its next pass: the frames below deliver it.
        Reply::AfterInput => Value::Null,
        _ => panic!("{method} {params}: not done"),
    };
    h.run_steps(6);
    v
}

/// Draws a new crop frame from `a` to `b` (document px), as an agent does.
fn draw_frame(h: &mut Harness, a: [f64; 2], b: [f64; 2]) {
    let events = json!([
        {"kind": "down", "x": a[0], "y": a[1]},
        {"kind": "move", "x": b[0], "y": b[1]},
        {"kind": "up", "x": b[0], "y": b[1]},
    ]);
    control(h, "ui.pointer", json!({"events": events}));
}

/// The crop frame's rect as `[x0, y0, x1, y1]` (document px).
fn frame_rect(h: &Harness) -> Option<[f64; 4]> {
    frame(h.state()).map(|f| [f.rect.x0, f.rect.y0, f.rect.x1, f.rect.y1])
}

/// The task bar's screen rect, as `ui.inspect` reports it.
fn bar(h: &Harness) -> Option<Rect> {
    h.state().ui.task_bar_shown.map(|[x0, y0, x1, y1]| Rect::from_min_max(pos2(x0, y0), pos2(x1, y1)))
}

fn button(h: &mut Harness, at: Pos2, pressed: bool) {
    h.event(egui::Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    h.run_steps(1);
}

/// A real click at `at`: hover, press, release.
fn click_at(h: &mut Harness, at: Pos2) {
    h.event(egui::Event::PointerMoved(at));
    h.run_steps(2);
    button(h, at, true);
    button(h, at, false);
    h.run_steps(4);
}

/// The centre of the widget labelled `label` (which must exist).
fn widget(h: &Harness, label: &str) -> Pos2 {
    h.get_by_label(label).rect().center()
}

fn has(h: &Harness, label: &str) -> bool {
    h.query_by_label(label).is_some()
}

/// Drags the bar by its grip by `d` screen points.
fn drag_grip(h: &mut Harness, d: Vec2) {
    let from = widget(h, "Move bar");
    h.event(egui::Event::PointerMoved(from));
    h.run_steps(2);
    button(h, from, true);
    for i in 1..=8 {
        h.event(egui::Event::PointerMoved(from + d * (i as f32 / 8.0)));
        h.run_steps(1);
    }
    button(h, from + d, false);
    h.run_steps(4);
}

#[test]
fn shows_only_with_the_crop_tool() {
    let mut h = harness();
    assert!(bar(&h).is_some());
    control(&mut h, "ui.set", json!({"tool": "brush"}));
    assert!(bar(&h).is_none(), "another tool hides the bar");
    assert!(!has(&h, "More options"));
    // No document: no canvas, no bar (and `ui.inspect` doesn't report the last one drawn).
    control(&mut h, "ui.set", json!({"tool": "crop"}));
    assert!(bar(&h).is_some());
    h.state_mut().run("file.close", json!({})).expect("close");
    h.run_steps(6);
    assert!(h.state().session.active().is_none());
    assert!(bar(&h).is_none(), "closing the document clears the bar");
}

/// Runs one control-channel request without drawing after it.
fn request(h: &mut Harness, method: &str, params: Value) -> Value {
    let ctx = h.ctx.clone();
    let (req, _rx) = ControlRequest::new(method, params.clone());
    let Reply::Done(v) = handle(h.state_mut(), &ctx, &req) else { panic!("{method} {params}: not done") };
    v
}

#[test]
fn ui_set_changes_only_the_place_fields_it_names() {
    let mut h = harness();
    draw_frame(&mut h, [40.0, 30.0], [260.0, 200.0]);
    drag_grip(&mut h, vec2(-60.0, -120.0));
    let dropped = bar(&h).expect("bar");
    let at = h.state().ui.panels.task_bar.at;
    // Pinning keeps the dragged place: the bar doesn't jump back under the frame.
    control(&mut h, "ui.set", json!({"panels": {"task_bar": {"pinned": true}}}));
    assert_eq!(h.state().ui.panels.task_bar.at, at);
    assert!(h.state().ui.panels.task_bar.pinned);
    assert_eq!(bar(&h).expect("bar").left_top(), dropped.left_top());
    // Placing it keeps the pin.
    control(&mut h, "ui.set", json!({"panels": {"task_bar": {"at": [12.0, 34.0]}}}));
    assert_eq!(h.state().ui.panels.task_bar, photocraft_ui_egui::state::TaskBarPlace::new(true, Some([12.0, 34.0])));
    // A field the bar doesn't have is refused, and nothing changes.
    let v = request(&mut h, "ui.set", json!({"panels": {"task_bar": {"pinned": false, "nope": 1}}}));
    assert_eq!(v["ok"], false, "{v}");
    assert!(h.state().ui.panels.task_bar.pinned);
    // Reset bar position over the control channel.
    control(&mut h, "ui.set", json!({"panels": {"task_bar": {"pinned": false, "at": null}}}));
    assert_eq!(h.state().ui.panels.task_bar, Default::default());
    let v = request(&mut h, "ui.inspect", json!({}));
    assert_eq!(v["result"]["taskBar"]["at"], Value::Null, "after the reset the bar follows the frame again");
}

#[test]
fn the_bars_preset_dropdown_is_the_options_bars() {
    let mut h = harness();
    let b = bar(&h).expect("bar");
    assert_eq!(frame_rect(&h), Some([0.0, 0.0, 400.0, 300.0]), "the untouched frame is the canvas");
    // Both bars show the Ratio dropdown: open the task bar's.
    let ratio = h.get_all_by_value("Ratio").map(|n| n.rect().center()).find(|p| b.contains(*p)).expect("the task bar's Ratio dropdown");
    click_at(&mut h, ratio);
    h.get_by_label("1 : 1 (Square)").click();
    h.run_steps(6);
    let [x0, y0, x1, y1] = frame_rect(&h).expect("a frame");
    assert!(((x1 - x0) - (y1 - y0)).abs() < 1e-6, "a square frame: {:?}", frame_rect(&h));
    assert_eq!(h.get_all_by_value("1 : 1 (Square)").count(), 2, "the options bar shows the same preset");
}

#[test]
fn the_options_bar_ratio_list_takes_escape_and_enter_before_the_crop() {
    let mut h = harness();
    control(&mut h, "ui.set", json!({"panels": {"context_task_bar": false}}));
    draw_frame(&mut h, [100.0, 50.0], [300.0, 250.0]);
    assert!(pending(h.state()), "the drawn square crop is pending");
    let before = frame_rect(&h).expect("the pending frame");
    let size = |h: &Harness| h.state().session.active().map(|s| (s.doc.size.width, s.doc.size.height));
    let history = |h: &Harness| h.state().session.active().expect("the crop document").history.past_len();
    let steps = history(&h);
    let square = "1 : 1 (Square)";

    // Escape dismisses the list. Enter accepts its first row, the selected free Ratio.
    // Neither key should reach the pending crop while the list is open (#7386).
    for key in [egui::Key::Escape, egui::Key::Enter] {
        assert!(!has(&h, square), "the square row is only in the open list");
        let at = h.get_by_value("Ratio").rect().center();
        click_at(&mut h, at);
        assert!(has(&h, square), "the options bar ratio list opened");
        h.key_press(key);
        h.run_steps(12);
        assert!(!has(&h, square), "{key:?} closed the ratio list");
        assert!(pending(h.state()), "{key:?} left the crop pending");
        assert_eq!(frame_rect(&h), Some(before), "{key:?} left the frame alone");
        assert_eq!(size(&h), Some((400, 300)), "{key:?} did not crop the document");
    }

    // A closed dropdown still has focus. Click inside the frame to return focus to the canvas.
    let at = h.state().last_canvas_rect.center();
    click_at(&mut h, at);
    assert_eq!(frame_rect(&h), Some(before), "the click only moved focus");
    h.key_press(egui::Key::Enter);
    h.run_steps(6);
    assert_eq!(size(&h), Some((200, 200)), "Enter on the canvas commits the pending crop");
    assert_eq!(history(&h), steps + 1, "Enter adds one document history step");
    assert!(!pending(h.state()), "Enter clears the pending Crop session once the document accepts the crop");
}

#[test]
fn inspect_reports_a_bar_hidden_before_the_next_draw_as_hidden() {
    let mut h = harness();
    assert!(bar(&h).is_some());
    for hide in [json!({"panels": {"context_task_bar": false}}), json!({"tool": "brush"})] {
        let v = request(&mut h, "ui.set", hide.clone());
        assert_eq!(v["ok"], true, "{v}");
        let v = request(&mut h, "ui.inspect", json!({}));
        assert_eq!(v["result"]["taskBar"]["visible"], false, "{hide}");
        assert_eq!(v["result"]["taskBar"]["rect"], Value::Null, "{hide}");
        control(&mut h, "ui.set", json!({"tool": "crop", "panels": {"context_task_bar": true}}));
        assert!(bar(&h).is_some());
    }
}

#[test]
fn with_tiled_documents_the_bar_sits_on_the_active_tile() {
    let mut h = harness();
    h.state_mut().run("file.new", json!({"width": 400, "height": 300, "background": "white"})).expect("new");
    h.run_steps(4);
    let ctx = h.ctx.clone();
    photocraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "window.arrange.twoUpVertical", json!({})).expect("2-up");
    h.run_steps(6);
    // Each tile active in turn (side by side, the first one left): the first tile is drawn before
    // the second, so a bar drawn by every tile would end up on the second one both times.
    let mut centres = Vec::new();
    for idx in [0, 1] {
        assert!(h.state_mut().session.set_active(idx));
        h.run_steps(8);
        assert_eq!(h.state().session.active_index(), Some(idx));
        centres.push(bar(&h).unwrap_or_else(|| panic!("document {idx}: no bar")).center().x);
    }
    // The documents are the same size at the same zoom: the bars are one tile width apart.
    assert!(centres[1] - centres[0] > 200.0, "the bar follows the active tile: {centres:?}");
    // A click into the first tile activates its document while the frame is drawn: the second
    // tile, drawn after it as the active one, must not put the new document's bar on itself.
    let y = bar(&h).expect("bar").top() - 120.0;
    let at = pos2(centres[0], y);
    h.event(egui::Event::PointerMoved(at));
    h.run_steps(2);
    button(&mut h, at, true);
    assert_eq!(h.state().session.active_index(), Some(0), "the click activates the first tile");
    if let Some(b) = bar(&h) {
        assert!((b.center().x - centres[0]).abs() < 100.0, "the bar is on the first tile, not {} (the second)", b.center().x);
    }
    button(&mut h, at, false);
}

#[test]
fn cancel_and_done_only_while_a_crop_is_pending() {
    let mut h = harness();
    assert!(!pending(h.state()));
    assert!(has(&h, "More options") && !has(&h, "Done") && !has(&h, "Cancel"), "the untouched frame offers no Cancel / Done");
    draw_frame(&mut h, [40.0, 30.0], [260.0, 200.0]);
    assert!(pending(h.state()));
    assert!(has(&h, "Done") && has(&h, "Cancel"));
    let narrow = bar(&h).expect("bar");
    assert!(narrow.width() > 0.0);
    // Cancel: the frame goes, as with the options bar's cancel.
    let at = widget(&h, "Cancel");
    click_at(&mut h, at);
    assert!(!pending(h.state()));
    assert_eq!(h.state().session.active().map(|s| (s.doc.size.width, s.doc.size.height)), Some((400, 300)));
    // Done commits the crop.
    draw_frame(&mut h, [40.0, 30.0], [260.0, 200.0]);
    let at = widget(&h, "Done");
    click_at(&mut h, at);
    assert_eq!(h.state().session.active().map(|s| (s.doc.size.width, s.doc.size.height)), Some((220, 170)), "Done crops");
    assert!(!pending(h.state()), "Done also clears the pending Crop session");
}

/// The active document's stored zoom.
fn zoom(h: &Harness) -> f32 {
    let app = h.state();
    app.session.active_index().and_then(|i| app.ui.views.get(i)).map(|v| v.zoom).expect("a view")
}

/// Done, like the options bar's commit, fits the cropped document on screen. Fit never zooms past
/// 100 %, so the crop starts at 50 %: the 220 × 170 result fits at 100 %. The bar draws inside the
/// canvas pass, whose write-back of its own camera used to drop that fit.
#[test]
fn done_fits_the_cropped_document() {
    let mut h = harness();
    control(&mut h, "ui.set", json!({"zoom": 0.5, "center": [200.0, 150.0]}));
    assert!((zoom(&h) - 0.5).abs() < 1e-4);
    draw_frame(&mut h, [40.0, 30.0], [260.0, 200.0]);
    let at = widget(&h, "Done");
    click_at(&mut h, at);
    assert_eq!(h.state().session.active().map(|s| (s.doc.size.width, s.doc.size.height)), Some((220, 170)), "Done crops");
    assert!((zoom(&h) - 1.0).abs() < 1e-4, "Done fits the cropped document: zoom {}", zoom(&h));
    let app = h.state();
    let v = app.session.active_index().and_then(|i| app.ui.views.get(i)).expect("a view");
    assert!(
        !v.asks(craft::ui::canvas::ViewCommands::FitOnScreen) && (v.center[0] - 110.0).abs() < 0.5 && (v.center[1] - 85.0).abs() < 0.5,
        "fitted and centred: {:?}",
        v.center
    );
}

#[test]
fn hide_and_place_are_kept_with_the_panel_layout() {
    let mut h = harness();
    // A layout saved before the bar existed: the bar is on and follows the frame.
    let mut old = serde_json::to_value(&h.state().ui.panels).expect("panels");
    old.as_object_mut().map(|o| (o.remove("context_task_bar"), o.remove("task_bar")));
    photocraft_ui_egui::dock::apply(h.state_mut(), &json!({"panels": old}));
    assert!(h.state().ui.panels.context_task_bar);
    assert_eq!(h.state().ui.panels.task_bar, Default::default());
    // What a restart restores.
    let saved = json!({"panels": {"context_task_bar": false, "task_bar": {"pinned": true, "at": [12.0, 34.0]}}});
    let mut panels = serde_json::to_value(&h.state().ui.panels).expect("panels");
    panels.as_object_mut().expect("object").extend(saved["panels"].as_object().expect("object").clone());
    photocraft_ui_egui::dock::apply(h.state_mut(), &json!({"panels": panels}));
    let p = &h.state().ui.panels;
    assert!(!p.context_task_bar);
    assert!(p.task_bar.pinned);
    assert_eq!(p.task_bar.at, Some([12.0, 34.0]));
}
