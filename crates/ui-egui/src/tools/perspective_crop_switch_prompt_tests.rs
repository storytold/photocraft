//! #11969: with a Perspective Crop quad pending, a toolbar click on another tool asks "Crop the
//! image?" with Crop (the default), Cancel and Don't Crop, in that order. Crop commits and
//! switches, Don't Crop drops the quad and switches, Cancel keeps the tool and the quad. Without a
//! quad the click switches at once, as before.

use egui::{Modifiers, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::*;
use crate::control::{ControlRequest, Outcome, handle};
use crate::panels::toolbar::{TOOL_SECTIONS, toolbar};

const QUAD: [[f64; 2]; 4] = [[20.0, 15.0], [150.0, 20.0], [140.0, 110.0], [25.0, 100.0]];

/// A 160 × 120 document with the Perspective Crop tool and, with `quad`, a quad pending.
fn setup(quad: bool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 160, "height": 120})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::PerspectiveCrop;
    if quad {
        app.ui.perspective_crop = Some(QUAD);
    }
    app
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.entries().len()
}

fn size(app: &PhotocraftApp) -> (u32, u32) {
    let d = &app.session.active().unwrap().doc;
    (d.size.width, d.size.height)
}

fn prompts(app: &PhotocraftApp) -> Vec<u64> {
    app.ui.dialogs.iter().filter(|d| owns_switch(&d.fields)).map(|d| d.id).collect()
}

/// One toolbar frame with `events`.
fn frame(app: &mut PhotocraftApp, ctx: &egui::Context, time: f64, events: Vec<egui::Event>) {
    let raw =
        egui::RawInput { time: Some(time), events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 1800.0))), ..Default::default() };
    ctx.run_ui(raw, |ui| toolbar(app, ui)).textures_delta.clear();
}

/// A plain click on the toolbar slot holding `tool`, through the pointer.
fn click_slot(app: &mut PhotocraftApp, tool: Tool) {
    click_slot_n(app, tool, 1);
}

/// `clicks` plain clicks in a row on the toolbar slot holding `tool`.
fn click_slot_n(app: &mut PhotocraftApp, tool: Tool, clicks: u32) {
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
    frame(app, &ctx, 0.0, vec![]);
    frame(app, &ctx, 0.1, vec![]);
    let bx = if crate::theme::Tokens::get(&ctx).pro { 30.0 } else { 36.0 };
    let mut buttons: Vec<egui::Rect> = ctx.viewport(|v| {
        v.prev_pass
            .widgets
            .layers()
            .flat_map(|(_, w)| w.iter())
            .filter(|w| w.rect.size() == egui::Vec2::splat(bx) && w.sense.senses_click())
            .map(|w| w.rect)
            .collect()
    });
    buttons.sort_by(|a, b| a.top().total_cmp(&b.top()));
    let index = TOOL_SECTIONS.iter().flat_map(|s| s.iter()).position(|slot| slot.contains(&tool)).unwrap();
    let at = buttons[index].center();
    let pointer = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    for k in 0..clicks {
        let t = 1.0 + f64::from(k) * 0.1;
        frame(app, &ctx, t, vec![egui::Event::PointerMoved(at), pointer(true)]);
        frame(app, &ctx, t + 0.05, vec![pointer(false)]);
    }
    frame(app, &ctx, 1.0 + f64::from(clicks) * 0.1, vec![]);
}

#[test]
fn a_toolbar_click_with_a_quad_asks_first() {
    let mut app = setup(true);
    click_slot(&mut app, Tool::Brush);
    let open = prompts(&app);
    assert_eq!(open.len(), 1, "the prompt opens");
    let d = app.ui.dialogs.iter().find(|d| d.id == open[0]).unwrap();
    assert_eq!(d.fields["message"], "Crop the image?");
    assert_eq!((app.ui.tool, app.ui.perspective_crop), (Tool::PerspectiveCrop, Some(QUAD)), "nothing switches yet");
    // Another click while it is up leaves the one prompt.
    click_slot(&mut app, Tool::Brush);
    assert_eq!(prompts(&app).len(), 1);
}

#[test]
fn without_a_quad_the_click_switches_at_once() {
    let mut app = setup(false);
    click_slot(&mut app, Tool::Brush);
    assert!(prompts(&app).is_empty());
    assert_eq!(app.ui.tool, Tool::Brush);
    // Clicked corners without a quad (not measured): no prompt either.
    let mut app = setup(false);
    app.pcrop.clicks = vec![[10.0, 10.0], [50.0, 12.0]];
    click_slot(&mut app, Tool::Brush);
    assert!(prompts(&app).is_empty());
    assert_eq!(app.ui.tool, Tool::Brush);
}

#[test]
fn crop_commits_and_switches() {
    let mut app = setup(true);
    let before = steps(&app);
    assert!(ask_before_switch(&mut app, Tool::Brush));
    let id = prompts(&app)[0];
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(steps(&app), before + 1, "one history state");
    assert_eq!(app.session.active().unwrap().history.entries().last().map(|e| e.to_string()), Some("Perspective Crop".to_string()));
    assert_ne!(size(&app), (160, 120), "the image was cropped");
    assert_eq!((app.ui.tool, app.ui.perspective_crop), (Tool::Brush, None));
}

#[test]
fn dont_crop_drops_the_quad_and_switches() {
    let mut app = setup(true);
    let before = steps(&app);
    assert!(ask_before_switch(&mut app, Tool::Brush));
    let id = prompts(&app)[0];
    app.ui.dialog_mut(id).unwrap().fields.insert("choice".into(), json!("dontCrop"));
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!((steps(&app), size(&app)), (before, (160, 120)), "the document is untouched");
    assert_eq!((app.ui.tool, app.ui.perspective_crop), (Tool::Brush, None));
}

#[test]
fn cancel_keeps_the_tool_and_the_quad() {
    let mut app = setup(true);
    let before = steps(&app);
    assert!(ask_before_switch(&mut app, Tool::Brush));
    let id = prompts(&app)[0];
    crate::dialogs::cancel(&mut app, id).unwrap();
    assert!(prompts(&app).is_empty());
    assert_eq!((app.ui.tool, app.ui.perspective_crop, steps(&app)), (Tool::PerspectiveCrop, Some(QUAD), before));
}

#[test]
fn the_prompt_is_listed_and_answered_over_the_control_channel() {
    let ctx = egui::Context::default();
    let mut app = setup(true);
    assert!(ask_before_switch(&mut app, Tool::Lasso));
    let (inspect, _rx) = ControlRequest::new("ui.inspect", json!({}));
    let Outcome::Done(inspect) = handle(&mut app, &ctx, &inspect) else { panic!("ui.inspect replies at once") };
    let dialog = inspect["result"]["dialogs"].as_array().unwrap().iter().find(|d| d["fields"]["message"] == "Crop the image?").cloned().expect("listed");
    let (set, _rx) = ControlRequest::new("ui.dialog.set", json!({"dialog": dialog["id"], "field": "choice", "value": "dontCrop"}));
    assert!(matches!(handle(&mut app, &ctx, &set), Outcome::Done(v) if v["ok"] == true));
    let (confirm, _rx) = ControlRequest::new("ui.dialog.confirm", json!({"dialog": dialog["id"]}));
    assert!(matches!(handle(&mut app, &ctx, &confirm), Outcome::Done(v) if v["ok"] == true));
    assert_eq!((app.ui.tool, app.ui.perspective_crop), (Tool::Lasso, None));
    // And ui.dialog.cancel keeps the session.
    let mut app = setup(true);
    assert!(ask_before_switch(&mut app, Tool::Lasso));
    let id = prompts(&app)[0];
    let (cancel, _rx) = ControlRequest::new("ui.dialog.cancel", json!({"dialog": id}));
    assert!(matches!(handle(&mut app, &ctx, &cancel), Outcome::Done(v) if v["ok"] == true));
    assert_eq!((app.ui.tool, app.ui.perspective_crop), (Tool::PerspectiveCrop, Some(QUAD)));
}

#[test]
fn the_buttons_read_crop_cancel_dont_crop_and_enter_crops() {
    let mut app = setup(true);
    assert!(ask_before_switch(&mut app, Tool::Brush));
    let mut h = Harness::builder().with_size(vec2(900.0, 700.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                crate::dialogs::show(app, ui.ctx());
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    if !h.ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
        return;
    }
    let x = |label: &str| h.get_by_label(label).rect().center().x;
    let (crop, cancel, dont) = (x("Crop"), x("Cancel"), x("Don't Crop"));
    // Crop is the default button, Cancel the cancel button and Don't Crop the other answer, laid
    // out in the platform's order as every three-button prompt (Rasterize / Cancel / Convert to
    // Smart Object, Save / Cancel / Don't Save): the default first elsewhere, last on macOS.
    let mut order = [("Crop", crop), ("Cancel", cancel), ("Don't Crop", dont)];
    order.sort_by(|a, b| a.1.total_cmp(&b.1));
    let order: Vec<&str> = order.iter().map(|o| o.0).collect();
    let mac = h.ctx.os() == egui::os::OperatingSystem::Mac;
    let want: &[&str] = if mac { &["Don't Crop", "Cancel", "Crop"] } else { &["Crop", "Don't Crop", "Cancel"] };
    assert_eq!(order, want);
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert_eq!(h.state().ui.tool, Tool::Brush, "Enter is Crop");
    assert!(h.state().ui.perspective_crop.is_none());
}

/// CodeRabbit on #14785: a double-click on the Hand or Zoom slot with a quad pending only asks. It
/// doesn't also fit the view or set 100 %, or switch the tool, behind the prompt.
#[test]
fn a_double_click_on_hand_or_zoom_only_asks() {
    for tool in [Tool::Hand, Tool::Zoom] {
        let mut app = setup(true);
        let zoom = app.ui.views.first().map(|v| v.zoom);
        click_slot_n(&mut app, tool, 2);
        assert_eq!(prompts(&app).len(), 1, "{tool:?}: one prompt");
        assert_eq!(app.ui.tool, Tool::PerspectiveCrop, "{tool:?}: the tool waits for the answer");
        assert_eq!(app.ui.views.first().map(|v| v.zoom), zoom, "{tool:?}: the view is as it was");
    }
}

/// CodeRabbit on #14785: the prompt asks about one document's quad. When another document becomes
/// active the quad and the prompt both go, so a later quad on the new document is not answered by
/// the old prompt.
#[test]
fn the_prompt_goes_with_its_quad_when_another_document_is_active() {
    let mut app = setup(true);
    click_slot(&mut app, Tool::Brush);
    assert_eq!(prompts(&app).len(), 1);
    app.run("file.new", json!({"width": 90, "height": 70})).unwrap();
    cancel_stale(&mut app);
    assert!(app.ui.perspective_crop.is_none(), "the old quad is dropped");
    assert!(prompts(&app).is_empty(), "and its prompt with it");
}
