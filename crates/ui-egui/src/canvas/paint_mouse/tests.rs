use crate::app::framework::BuildApp as _;
use egui::{Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;

use super::*;
use crate::canvas::{ToolEvent, tool_event};

mod right_click;

pub(super) fn harness(prefs: Option<&str>) -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("tools.setColors", json!({"foreground": "#0000ff"})).unwrap();
    app.run("tools.setBrush", json!({"brush": {"size": 20, "hardness": 1.0}})).unwrap();
    if let Some(v) = prefs {
        app.run("prefs.set", json!({"path": "tools.rightClickWithPaintingTools", "value": v})).unwrap();
    }
    app.ui.tool = Tool::Brush;
    app.sync_views();
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1200.0, 800.0)).with_step_dt(1.0 / 60.0).build_app(
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

pub(super) fn press(h: &mut Harness<'static, Box<PhotocraftApp>>, pos: Pos2, button: PointerButton, pressed: bool) {
    h.event(egui::Event::PointerButton { pos, button, pressed, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

/// Drag across the canvas centre with `button`; returns the document points it covered.
pub(super) fn drag(h: &mut Harness<'static, Box<PhotocraftApp>>, button: PointerButton) -> (Pos2, Pos2) {
    let c = h.state().last_canvas_rect.center();
    let (a, b) = (c - vec2(80.0, 0.0), c + vec2(80.0, 0.0));
    h.event(egui::Event::PointerMoved(a));
    h.run_steps(1);
    press(h, a, button, true);
    for i in 1..=8 {
        h.event(egui::Event::PointerMoved(a + (b - a) * (i as f32 / 8.0)));
        h.run_steps(1);
    }
    press(h, b, button, false);
    (a, b)
}

fn alpha_at(h: &Harness<'static, Box<PhotocraftApp>>, screen: Pos2) -> f32 {
    let app = h.state();
    let v = app.ui.views[0].clone();
    let r = app.last_canvas_rect;
    let d = (screen - r.center()) / v.zoom;
    let (x, y) = ((d.x + v.center[0]) as i32, (d.y + v.center[1]) as i32);
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y)[3]
}

fn strokes(h: &Harness<'static, Box<PhotocraftApp>>) -> Vec<serde_json::Value> {
    h.state().session.journal.iter().filter(|(id, _)| id == "paint.stroke").map(|(_, p)| p.clone()).collect()
}

/// Where the Shift+right-click mode menu is open, if it is.
fn mode_menu_at(h: &Harness<'static, Box<PhotocraftApp>>) -> Option<Pos2> {
    h.state()
        .ui
        .canvas_tool_menu
        .as_ref()
        .filter(|m| matches!(m.kind, crate::canvas_tool_menu::MenuKind::BrushMode { .. }))
        .map(|m| egui::pos2(m.pos[0], m.pos[1]))
}

/// Where the Brush Preset picker is open, if it is.
fn picker_at(h: &Harness<'static, Box<PhotocraftApp>>) -> Option<Pos2> {
    crate::brush_picker::opened_at(h.state())
}

fn mode_right_click(h: &mut Harness<'static, Box<PhotocraftApp>>, mods: Modifiers) {
    let p = h.state().last_canvas_rect.center();
    h.event(egui::Event::ModifiersChanged(mods));
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: true, modifiers: mods });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: false, modifiers: mods });
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
}

#[test]
fn shift_right_mode_picker_selects_journals_and_clear_erases() {
    use egui_kittest::kittest::Queryable;
    let mut h = harness(Some("erase"));
    let history = h.state().session.active().unwrap().history.past_len();
    let journal = h.state().session.journal.len();
    mode_right_click(&mut h, Modifiers::SHIFT);
    assert!(mode_menu_at(&h).is_some());
    assert!(picker_at(&h).is_none() && h.state().live_stroke.is_none() && strokes(&h).is_empty());
    assert_eq!(h.state().session.journal.len(), journal);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
    h.get_by_label("Multiply").click();
    h.run_steps(2);
    assert!(mode_menu_at(&h).is_none());
    assert_eq!(h.state().session.tools.brush.mode, photocraft_doc::BlendMode::Multiply);
    assert_eq!(h.state().session.journal.last().unwrap(), &("tools.setBrush".into(), json!({"brush": {"mode": "Multiply", "behind": false}})));
    let (a, b) = drag(&mut h, PointerButton::Primary);
    let mid = a + (b - a) * 0.5;
    assert!(alpha_at(&h, mid) > 0.9);
    mode_right_click(&mut h, Modifiers::SHIFT);
    h.get_by_label("Clear").click();
    h.run_steps(2);
    assert!(h.state().ui.tool_options.brush_clear);
    assert_eq!(h.state().session.tools.brush.mode, photocraft_doc::BlendMode::Multiply, "Clear keeps the underlying mode, as the options bar does");
    drag(&mut h, PointerButton::Primary);
    assert_eq!(alpha_at(&h, mid), 0.0);
    assert!(strokes(&h).last().unwrap()["erase"].as_bool().unwrap());
}

#[test]
fn mode_control_can_pick_a_row_with_ui_click() {
    use crate::control::{ControlRequest, handle};
    use egui_kittest::kittest::Queryable;
    let mut h = harness(Some("erase"));
    let ctx = h.ctx.clone();
    let before = h.state().session.journal.len();
    let (req, _) = ControlRequest::new(
        "ui.pointer",
        json!({"button": "right", "shift": true,
        "events": [{"kind": "down", "x": 200, "y": 150}, {"kind": "up", "x": 200, "y": 150}]}),
    );
    // `ui.pointer` queues window input; the passes after the call act on it.
    let _ = handle(h.state_mut(), &ctx, &req);
    h.run_steps(5);
    assert!(mode_menu_at(&h).is_some());
    assert_eq!(h.state().session.journal.len(), before);
    let at = h.get_by_label("Darken").rect().center();
    let (req, _) = ControlRequest::new("ui.click", json!({"x": at.x, "y": at.y}));
    let _ = handle(h.state_mut(), &ctx, &req);
    // The window host injects the move, the press and the release one pass each.
    h.run_steps(5);
    assert!(mode_menu_at(&h).is_none());
    assert_eq!(h.state().session.tools.brush.mode, photocraft_doc::BlendMode::Darken);
    assert_eq!(h.state().session.journal.last().unwrap(), &("tools.setBrush".into(), json!({"brush": {"mode": "Darken", "behind": false}})));
}

#[test]
fn shift_right_mode_picker_cancels_and_other_gestures_keep_their_routes() {
    let mut h = harness(None);
    let journal = h.state().session.journal.len();
    mode_right_click(&mut h, Modifiers::SHIFT);
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(mode_menu_at(&h).is_none());
    mode_right_click(&mut h, Modifiers::SHIFT);
    let outside = h.state().last_canvas_rect.min + vec2(2.0, 2.0);
    h.event(egui::Event::PointerMoved(outside));
    press(&mut h, outside, PointerButton::Secondary, true);
    press(&mut h, outside, PointerButton::Secondary, false);
    assert!(mode_menu_at(&h).is_none());
    mode_right_click(&mut h, Modifiers::SHIFT);
    h.state_mut().ui.tool = Tool::Pencil;
    h.run_steps(2);
    assert!(mode_menu_at(&h).is_none());
    // Spot Healing has no Shift+right-click menu here: its right-click keeps the picker.
    h.state_mut().ui.tool = Tool::SpotHealing;
    h.run_steps(1);
    mode_right_click(&mut h, Modifiers::SHIFT);
    assert!(mode_menu_at(&h).is_none() && picker_at(&h).is_some());
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    h.state_mut().ui.tool = Tool::Brush;
    mode_right_click(&mut h, Modifiers::NONE);
    assert!(picker_at(&h).is_some() && mode_menu_at(&h).is_none());
    assert_eq!(h.state().session.journal.len(), journal, "opening/cancelling never changes brush or pixels");
}

#[test]
fn shift_released_before_the_right_button_still_opens_the_mode_picker() {
    // egui reports `secondary_clicked` on release. With Erase on, a click that lost Shift a
    // frame early must open the menu, not erase a dab.
    let mut h = harness(Some("erase"));
    let history = h.state().session.active().unwrap().history.past_len();
    let journal = h.state().session.journal.len();
    let p = h.state().last_canvas_rect.center();
    h.event(egui::Event::ModifiersChanged(Modifiers::SHIFT));
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::SHIFT });
    h.run_steps(1);
    // Shift goes up in a frame of its own while the right button is still down.
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert!(mode_menu_at(&h).is_some(), "the press decided the gesture");
    assert!(h.state().live_stroke.is_none() && strokes(&h).is_empty());
    assert_eq!(h.state().session.journal.len(), journal, "no erase dab");
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
}

#[test]
fn shift_click_connects_to_the_previous_brush_stroke() {
    use egui::{Event, Modifiers};

    let mut h = harness(None);
    let (_, end) = drag(&mut h, PointerButton::Primary);
    let previous = h
        .state()
        .session
        .journal
        .iter()
        .rev()
        .find(|(id, _)| id == "paint.stroke")
        .map(|(_, p)| p["points"].as_array().unwrap().last().unwrap().clone())
        .unwrap();
    let target = end + vec2(120.0, 80.0);
    h.event(Event::PointerMoved(target));
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: target, button: PointerButton::Primary, pressed, modifiers: Modifiers::SHIFT });
        h.run_steps(2);
    }

    let strokes = strokes(&h);
    assert_eq!(strokes.len(), 2, "a Shift-click commits one connected stroke");
    let points = strokes[1]["points"].as_array().unwrap();
    // The previous endpoint, then the clicked one. While the button is held, smoothing that is
    // still catching up repeats the clicked point with later times.
    assert_eq!(points[0][0], previous[0]);
    assert_eq!(points[0][1], previous[1]);
    let clicked = &points[1];
    assert!(points[1..].iter().all(|p| p[0] == clicked[0] && p[1] == clicked[1]), "{points:?}");
    assert!(alpha_at(&h, end + vec2(60.0, 40.0)) > 0.9, "the segment between the strokes is painted");
}

#[test]
fn right_drag_erases_with_the_erase_preference() {
    let mut h = harness(Some("erase"));
    let (a, b) = drag(&mut h, PointerButton::Primary);
    let mid = a + (b - a) * 0.5;
    assert!(alpha_at(&h, mid) > 0.9);
    let undo = h.state().session.active().unwrap().history.past_len();
    drag(&mut h, PointerButton::Secondary);
    let s = strokes(&h);
    assert_eq!(s.len(), 2);
    assert_eq!(s[1]["erase"], json!(true), "the right stroke erases");
    assert_eq!(alpha_at(&h, mid), 0.0, "erased to transparency");
    assert_eq!(h.state().session.active().unwrap().history.past_len(), undo + 1, "one undo step");
    assert_eq!(picker_at(&h), None, "no picker in erase mode");
    // The next left stroke paints again.
    drag(&mut h, PointerButton::Primary);
    assert_eq!(strokes(&h)[2]["erase"], json!(false));
    assert!(alpha_at(&h, mid) > 0.9);
}

/// A secondary-button stroke across the canvas centre with the pen reporting 0.4 at the press and
/// rising to 0.8 at the release.
fn secondary_pen_stroke(h: &mut Harness<'static, Box<PhotocraftApp>>) {
    use crate::stylus::PenSample;
    let pen = |p: f32| Some(PenSample { pressure: p, ..Default::default() });
    let c = h.state().last_canvas_rect.center();
    let (a, b) = (c - vec2(80.0, 0.0), c + vec2(80.0, 0.0));
    h.state().stylus.feed.set(pen(0.4));
    h.event(egui::Event::PointerMoved(a));
    h.run_steps(1);
    press(h, a, PointerButton::Secondary, true);
    for i in 1..=8 {
        h.state().stylus.feed.set(pen(0.4 + 0.4 * i as f32 / 8.0));
        h.event(egui::Event::PointerMoved(a + (b - a) * (i as f32 / 8.0)));
        h.run_steps(1);
    }
    press(h, b, PointerButton::Secondary, false);
    h.state().stylus.feed.set(None);
}

#[test]
fn right_stroke_erases_with_pen_pressure_through_the_control_channel_path() {
    let mut h = harness(Some("erase"));
    secondary_pen_stroke(&mut h);
    let s = strokes(&h);
    assert_eq!(s.len(), 1, "one erasing stroke");
    assert_eq!(s[0]["erase"], json!(true));
    let first = s[0]["points"][0][2].as_f64().unwrap();
    assert!((first - 0.4).abs() < 1e-3, "pressure reaches the erasing stroke: {first}");
    // With the default preference the right button opens the picker and nothing paints.
    let mut h = harness(None);
    secondary_pen_stroke(&mut h);
    assert!(strokes(&h).is_empty(), "nothing paints");
    let c = h.state().last_canvas_rect.center();
    h.event(egui::Event::PointerMoved(c));
    h.run_steps(1);
    press(&mut h, c, PointerButton::Secondary, true);
    press(&mut h, c, PointerButton::Secondary, false);
    assert!(picker_at(&h).is_some() && strokes(&h).is_empty());
}

#[test]
fn shift_alt_right_through_the_control_channel_respects_the_erase_preference() {
    // As on the canvas, with the right-erase preference the erase stroke wins over the HUD
    // for Brush/Eraser; without it, Shift+Alt+right opens the HUD (#6098).
    let shift_alt = Modifiers::SHIFT | Modifiers::ALT;
    let gesture = |h: &mut Harness<'static, Box<PhotocraftApp>>| {
        let c = h.state().last_canvas_rect.center();
        h.event(egui::Event::ModifiersChanged(shift_alt));
        h.event(egui::Event::PointerMoved(c));
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: c, button: PointerButton::Secondary, pressed: true, modifiers: shift_alt });
        h.run_steps(2);
        let opened = crate::color_hud::is_open(&h.state().ui);
        for i in 1..=4 {
            h.event(egui::Event::PointerMoved(c + vec2(3.0 * i as f32, 0.0)));
            h.run_steps(1);
        }
        h.event(egui::Event::PointerButton { pos: c + vec2(12.0, 0.0), button: PointerButton::Secondary, pressed: false, modifiers: shift_alt });
        h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
        h.run_steps(2);
        opened
    };
    let mut h = harness(Some("erase"));
    assert!(!gesture(&mut h), "no HUD with the erase preference");
    let s = strokes(&h);
    assert_eq!(s.len(), 1, "the press arms one erasing stroke");
    assert_eq!(s[0]["erase"], json!(true));
    // With the default preference the same gesture opens the HUD instead.
    let mut h = harness(None);
    assert!(gesture(&mut h), "the HUD opened");
    assert!(strokes(&h).is_empty());
    assert!(!crate::color_hud::is_open(&h.state().ui), "the release closes it");
}

#[test]
fn the_brush_is_a_per_tool_option_that_presets_keep() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    let amount = |app: &PhotocraftApp| app.session.tools.brush.smoothing.amount;
    app.ui.tool = Tool::Brush;
    sync_tool_brush(&mut app);
    assert_eq!(amount(&app), 0.1, "Photoshop's default");
    // Picking a preset (whose own smoothing is 0) keeps the tool's smoothing.
    let preset = app.session.tools.presets.iter().find(|p| p.brush.smoothing.amount == 0.0).map(|p| p.name.clone()).unwrap();
    app.run("tools.setBrush", json!({"preset": preset})).unwrap();
    assert_eq!(amount(&app), 0.1);
    // A user value (the options-bar field) survives a preset too, mode included.
    app.session.tools.brush.smoothing.amount = 0.42;
    app.session.tools.brush.smoothing.pulled_string = true;
    app.run("tools.setBrush", json!({"preset": preset})).unwrap();
    assert_eq!((amount(&app), app.session.tools.brush.smoothing.pulled_string), (0.42, true));
    // The Eraser has its own (0 % at first, #11936); switching back restores the Brush's.
    app.ui.tool = Tool::Eraser;
    sync_tool_brush(&mut app);
    assert_eq!(amount(&app), 0.0);
    app.session.tools.brush.smoothing.amount = 0.0;
    app.ui.tool = Tool::Brush;
    sync_tool_brush(&mut app);
    assert_eq!((amount(&app), app.session.tools.brush.smoothing.pulled_string), (0.42, true));
    // Other tools leave it alone; the Eraser kept its 0 %.
    app.ui.tool = Tool::Move;
    sync_tool_brush(&mut app);
    assert_eq!(amount(&app), 0.42);
    app.ui.tool = Tool::Eraser;
    // A stroke picks up the tool's brush even without a frame in between.
    tool_event(&mut app, ToolEvent::Down { x: 5.0, y: 5.0, pressure: 1.0 }, Modifiers::NONE);
    assert_eq!(amount(&app), 0.0);
    tool_event(&mut app, ToolEvent::Up { x: 30.0, y: 5.0 }, Modifiers::NONE);
}

/// #10263: the Art History Brush starts at full opacity the first time it is picked, with its own
/// tip rather than the old tool's (#12631; other tools still take its opacity).
#[test]
fn art_history_brush_starts_at_full_opacity() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.ui.tool = Tool::Brush;
    sync_tool_brush(&mut app);
    app.run("tools.setBrush", json!({"brush": {"size": 40, "opacity": 0.3}})).unwrap();
    app.ui.tool = Tool::ArtHistoryBrush;
    sync_tool_brush(&mut app);
    assert_eq!(app.session.tools.brush.opacity, 1.0, "Art History starts at 100%");
    assert_eq!(app.session.tools.brush.size, 21.0, "with its own 21 px tip");
    app.ui.tool = Tool::HistoryBrush;
    sync_tool_brush(&mut app);
    assert_eq!(app.session.tools.brush.opacity, 1.0, "the History Brush takes the Art History brush's opacity");
    app.ui.tool = Tool::Brush;
    sync_tool_brush(&mut app);
    app.ui.tool = Tool::Eraser;
    sync_tool_brush(&mut app);
    assert_eq!(app.session.tools.brush.opacity, 0.3, "other tools still inherit opacity");
}

/// #218: Photoshop keeps a brush per painting tool, so the size, hardness, mode, opacity and
/// dynamics of one tool never leak into another's, and each tool gets its own back.
#[test]
fn each_tool_keeps_its_own_brush() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    let brush = |app: &PhotocraftApp| (*app.session.tools.brush).clone();
    let size = |b: &BrushSettings| b.size;
    let hardness = |b: &BrushSettings| b.hardness;
    let opacity = |b: &BrushSettings| b.opacity;
    let mode = |b: &BrushSettings| b.mode;

    app.ui.tool = Tool::Brush;
    sync_tool_brush(&mut app);
    app.run("tools.setBrush", json!({"brush": {"size": 40, "hardness": 0.2, "opacity": 0.7, "mode": "Multiply"}})).unwrap();
    assert_eq!(size(&brush(&app)), 40.0);

    // A tool seen for the first time starts from the brush the old tool left (Photoshop's
    // 10 % smoothing), so nothing is lost by switching.
    app.ui.tool = Tool::Blur;
    sync_tool_brush(&mut app);
    assert_eq!(size(&brush(&app)), 40.0, "a new tool inherits the current brush");

    // The Blur tool edits its own size; the Brush keeps 40 px.
    app.run("tools.setBrush", json!({"brush": {"size": 8, "hardness": 1.0}})).unwrap();
    app.ui.tool = Tool::Brush;
    sync_tool_brush(&mut app);
    assert_eq!(size(&brush(&app)), 40.0, "the Brush keeps its own size");
    assert_eq!(hardness(&brush(&app)), 0.2, "and its hardness");
    app.ui.tool = Tool::Blur;
    sync_tool_brush(&mut app);
    assert_eq!(size(&brush(&app)), 8.0, "the Blur tool keeps its own size");
    assert_eq!(hardness(&brush(&app)), 1.0, "and its hardness");

    // Mode, opacity, flow and the dynamics sections are per tool too.
    app.ui.tool = Tool::CloneStamp;
    sync_tool_brush(&mut app);
    assert_eq!(mode(&brush(&app)), photocraft_color::BlendMode::Normal, "a new tool starts in Normal, not the Brush's Multiply");
    app.run("tools.setBrush", json!({"brush": {"mode": "Screen", "opacity": 0.3}})).unwrap();
    app.ui.tool = Tool::Brush;
    sync_tool_brush(&mut app);
    assert_eq!((mode(&brush(&app)), opacity(&brush(&app))), (photocraft_color::BlendMode::Multiply, 0.7), "the Brush's own mode and opacity");
    app.ui.tool = Tool::CloneStamp;
    sync_tool_brush(&mut app);
    assert_eq!((mode(&brush(&app)), opacity(&brush(&app))), (photocraft_color::BlendMode::Screen, 0.3), "the Clone Stamp keeps its own");

    // The colours, the erase flag and the seed belong to the session, so a swap keeps what is
    // live now instead of restoring another tool's copy (a picked colour reaches every tool).
    app.run("tools.setBrush", json!({"brush": {"color": [0.0, 1.0, 0.0, 1.0], "background": [1.0, 0.0, 1.0, 1.0], "erase": true, "seed": 42}})).unwrap();
    app.ui.tool = Tool::Brush;
    sync_tool_brush(&mut app);
    let b = brush(&app);
    assert_eq!(
        (b.color, b.background, b.erase, b.seed),
        ([0.0, 1.0, 0.0, 1.0], [1.0, 0.0, 1.0, 1.0], true, 42),
        "the session's own brush fields survive the swap"
    );

    // Tools with no brush of their own (the Move) leave the brush alone.
    let before = brush(&app);
    app.ui.tool = Tool::Move;
    sync_tool_brush(&mut app);
    assert_eq!(brush(&app), before);
}

#[test]
fn eraser_tool_on_the_background_paints_the_background_colour() {
    // #76: the Eraser on the Background (locked transparency) paints the background colour.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 120, "height": 80})).unwrap();
    app.run("tools.setColors", json!({"background": "#ff0000"})).unwrap();
    app.run("tools.setBrush", json!({"brush": {"size": 12, "hardness": 1.0}})).unwrap();
    app.ui.tool = Tool::Eraser;
    let m = Modifiers::NONE;
    tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 40.0, pressure: 1.0 }, m);
    tool_event(&mut app, ToolEvent::Move { x: 100.0, y: 40.0, pressure: 1.0 }, m);
    tool_event(&mut app, ToolEvent::Up { x: 100.0, y: 40.0 }, m);
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.layers[0].name, "Background");
    assert_eq!(st.doc.layers[0].surface().unwrap().rgba(50, 40), [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(st.history.undo_label(), Some("Eraser"));
    assert!(!app.ui.status_error, "{}", app.ui.status);
}

/// Shift+right-click with `tool` and the labels of the menu it opened (empty: none opened).
fn open_tool_menu(h: &mut Harness<'static, Box<PhotocraftApp>>, tool: Tool) -> Vec<&'static str> {
    h.state_mut().ui.tool = tool;
    h.run_steps(1);
    mode_right_click(h, Modifiers::SHIFT);
    if mode_menu_at(h).is_none() { Vec::new() } else { crate::brush_mode_menu::labels(tool) }
}

#[test]
fn shift_right_menu_lists_each_tools_own_rows_in_order() {
    use crate::brush_mode_menu::{Item, items};
    use egui_kittest::kittest::Queryable;
    let blend: Vec<&'static str> = photocraft_doc::BlendMode::LAYER_MODES.iter().map(|m| m.label()).collect();
    let with = |extra: &[&'static str]| {
        let mut v = blend.clone();
        for (i, l) in extra.iter().enumerate() {
            v.insert(2 + i, l);
        }
        v
    };
    let edit = |rest: Vec<&'static str>| std::iter::once("Edit Brush…").chain(rest).collect::<Vec<_>>();
    for (tool, want) in [
        (Tool::Brush, edit(with(&["Behind", "Clear"]))),
        (Tool::Pencil, edit(with(&["Behind", "Clear"]))),
        (Tool::CloneStamp, edit(with(&[]))),
        (Tool::Eraser, vec!["Brush", "Pencil", "Block"]),
        (Tool::Dodge, edit(vec!["Shadows", "Midtones", "Highlights"])),
        (Tool::Sponge, edit(vec!["Desaturate", "Saturate"])),
        (Tool::PaintBucket, with(&["Clear"])),
        (Tool::MixerBrush, vec!["Edit Brush…"]),
        (Tool::Healing, vec!["Normal", "Multiply", "Screen", "Darken", "Lighten", "Color", "Luminosity"]),
        (Tool::SpotHealing, vec![]),
        (Tool::MagicEraser, vec![]),
    ] {
        let mut h = harness(None);
        let journal = h.state().session.journal.len();
        assert_eq!(open_tool_menu(&mut h, tool), want, "{tool:?}");
        assert_eq!(h.state().session.journal.len(), journal, "{tool:?}: opening changes nothing");
        if want.is_empty() {
            continue;
        }
        assert!(picker_at(&h).is_none(), "{tool:?}");
        // A separator after Edit Brush…, and the blend modes in their groups.
        let rows = items(tool);
        if rows.first() == Some(&Item::EditBrush) && rows.len() > 1 {
            assert_eq!(rows.get(1), Some(&Item::Separator), "{tool:?}");
        }
        if want.contains(&"Darken") && want.contains(&"Dissolve") {
            let at = rows.iter().position(|i| *i == Item::Blend(photocraft_doc::BlendMode::Darken)).unwrap();
            assert_eq!(rows[at - 1], Item::Separator, "{tool:?}");
        }
        // The rows the menu shows first are drawn, top to bottom, in that order.
        let tops: Vec<f32> = want.iter().take(4).map(|l| h.get_by_label(l).rect().top()).collect();
        assert!(tops.windows(2).all(|w| w[0] < w[1]), "{tool:?}: {tops:?}");
    }
}

#[test]
fn behind_and_clear_are_greyed_on_the_background_layer() {
    use crate::brush_mode_menu::{Item, enabled, find};
    use egui_kittest::kittest::Queryable;
    let mut h = harness(None);
    let (bg, top) = {
        let doc = &h.state().session.active().unwrap().doc;
        (doc.layers[0].id, doc.layers[1].id)
    };
    h.state_mut().session.active_mut().unwrap().set_active_layer(Some(bg));
    for tool in [Tool::Brush, Tool::Pencil, Tool::PaintBucket] {
        open_tool_menu(&mut h, tool);
        assert!(!enabled(h.state(), Item::Clear), "{tool:?}");
        assert!(find(h.state(), "Clear").is_err(), "{tool:?}");
        assert!(enabled(h.state(), Item::Blend(photocraft_doc::BlendMode::Multiply)));
        let journal = h.state().session.journal.len();
        h.get_by_label("Clear").click();
        h.run_steps(2);
        let o = &h.state().ui.tool_options;
        assert!(!o.brush_clear && !o.pencil_clear && !o.bucket_clear, "{tool:?}");
        assert_eq!(h.state().session.journal.len(), journal, "{tool:?}: a greyed row does nothing");
        h.key_press(egui::Key::Escape);
        h.run_steps(2);
    }
    assert!(!enabled(h.state(), Item::Behind));
    // On a normal layer they are available again.
    h.state_mut().session.active_mut().unwrap().set_active_layer(Some(top));
    assert!(enabled(h.state(), Item::Behind) && enabled(h.state(), Item::Clear));
}

/// #11897: Clear chosen on a pixel layer, then the Background activated: a Brush stroke there
/// paints the foreground, and the Mode reads Normal afterwards.
#[test]
fn chosen_clear_paints_normally_on_the_background_and_the_mode_reads_normal() {
    let mut h = harness(None);
    h.state_mut().ui.tool_options.brush_clear = true;
    let bg = h.state().session.active().unwrap().doc.background().unwrap().id;
    h.state_mut().run("layer.select", json!({"layer": bg.0})).unwrap();
    h.run_steps(2);
    let (a, b) = drag(&mut h, PointerButton::Primary);
    assert!(!h.state().ui.tool_options.brush_clear, "the Mode reads Normal");
    let last = strokes(&h).last().cloned().unwrap();
    assert!(last.get("mode").is_none() && !last["erase"].as_bool().unwrap(), "{last}");
    let app = h.state();
    let v = app.ui.views[0].clone();
    let d = ((a + (b - a) * 0.5) - app.last_canvas_rect.center()) / v.zoom;
    let st = app.session.active().unwrap();
    let px = st.doc.layer(bg).unwrap().surface().unwrap().rgba((d.x + v.center[0]) as i32, (d.y + v.center[1]) as i32);
    assert!(px[2] > 0.9 && px[0] < 0.1, "the foreground blue, not the background colour: {px:?}");
}

/// #11897: a Background with Lock All refuses the stroke, so a chosen Clear stays chosen.
#[test]
fn chosen_clear_stays_on_a_locked_background() {
    let mut h = harness(None);
    h.state_mut().ui.tool_options.brush_clear = true;
    let bg = h.state().session.active().unwrap().doc.background().unwrap().id;
    h.state_mut().run("layer.select", json!({"layer": bg.0})).unwrap();
    h.state_mut().run("layer.lockLayers", json!({"all": true})).unwrap();
    h.run_steps(2);
    drag(&mut h, PointerButton::Primary);
    assert!(h.state().ui.tool_options.brush_clear, "the Mode still reads Clear");
}

/// #11897: a held `~` on the Background keeps erasing to the background colour.
#[test]
fn held_tilde_on_the_background_paints_the_background_colour() {
    let mut h = harness(None);
    h.state_mut().run("tools.setColors", json!({"background": "#00ff00"})).unwrap();
    let bg = h.state().session.active().unwrap().doc.background().unwrap().id;
    h.state_mut().run("layer.select", json!({"layer": bg.0})).unwrap();
    h.run_steps(2);
    h.event(egui::Event::Key { key: egui::Key::Backtick, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(1);
    let (a, b) = drag(&mut h, PointerButton::Primary);
    h.event(egui::Event::Key { key: egui::Key::Backtick, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(1);
    assert_eq!(strokes(&h).last().unwrap()["mode"], json!("clear"));
    let app = h.state();
    let v = app.ui.views[0].clone();
    let d = ((a + (b - a) * 0.5) - app.last_canvas_rect.center()) / v.zoom;
    let st = app.session.active().unwrap();
    let px = st.doc.layer(bg).unwrap().surface().unwrap().rgba((d.x + v.center[0]) as i32, (d.y + v.center[1]) as i32);
    assert!(px[1] > 0.9 && px[0] < 0.1 && px[2] < 0.1, "the background green, not the foreground blue: {px:?}");
}
