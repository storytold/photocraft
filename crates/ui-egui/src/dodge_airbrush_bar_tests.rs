//! #12682: the Dodge options bar's Airbrush toggle, and what a pointer held still does with it on.
//!
//! The toggle sits between Exposure and Angle, is off by default and off again after Reset Tool, and
//! it writes the Dodge tool's own brush's Build-up - the setting the Brush bar's Airbrush button
//! uses, and the route this bar's Angle and "Pressure for Size" cells already take. The reference
//! application's measured pixels (132 held with it off, 179 held for 3 s) are not asserted here:
//! #12681's per-stroke limit and the epic's tone-curve items are not in `main` yet, so this document
//! would not give those numbers. What is pinned is the order - held longer is strictly lighter with
//! the toggle on, and a held pointer changes nothing after the first dab with it off - and that
//! Burn's and Sponge's bars do not get the toggle (Burn's is #12665).

use egui::accesskit::{Role, Toggled};
use egui::{Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::{PhotocraftApp, Tool};

type H = Harness<'static, Box<PhotocraftApp>>;

const AIRBRUSH: &str = "Enable airbrush-style build-up effects";

/// The options bar over a grey 128 document, with `tool` picked and its own brush swapped in.
fn bar(tool: Tool) -> H {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 120, "depth": 8})).unwrap();
    app.run("edit.fill", json!({"color": "#808080"})).unwrap();
    crate::theme::install_fonts(app.ctx());
    crate::theme::apply(app.ctx(), &crate::theme::AppTheme::pro());
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1800.0, 300.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut Box<PhotocraftApp>| {
            crate::panels::options_bar(app, ui);
            crate::dialogs::run(app, ui);
        },
        app,
    );
    h.state_mut().ui.tool = tool;
    h.run_steps(4);
    h
}

fn on(h: &H) -> bool {
    h.get_by_role_and_label(Role::Button, AIRBRUSH).accesskit_node().toggled() == Some(Toggled::True)
}

fn click_airbrush(h: &mut H) {
    h.get_by_role_and_label(Role::Button, AIRBRUSH).click();
    h.run_steps(2);
}

#[test]
fn the_dodge_bar_shows_airbrush_between_exposure_and_angle() {
    let h = bar(Tool::Dodge);
    let exposure = h.get_by_role_and_label(Role::SpinButton, "Exposure").rect();
    let button = h.get_by_role_and_label(Role::Button, AIRBRUSH);
    let angle = h.get_by_role_and_label(Role::SpinButton, "Angle").rect();
    assert!(exposure.right() <= button.rect().left() && button.rect().right() <= angle.left(), "Range, Exposure, Airbrush, Angle");
    assert_eq!(button.accesskit_node().label().as_deref(), Some(AIRBRUSH), "the reference's sentence is the control's own name");
    assert!(!on(&h), "default off");
    assert!(!h.state().session.tools.brush.build_up, "and the tool's brush says so");
}

#[test]
fn the_burn_and_sponge_bars_have_no_airbrush_toggle() {
    // Burn's own toggle is #12665, and the Sponge bar is its own: neither shows this one.
    for tool in [Tool::Burn, Tool::Sponge] {
        let h = bar(tool);
        assert!(h.query_all_by_label(AIRBRUSH).next().is_none(), "{tool:?} shows no Airbrush toggle");
    }
}

#[test]
fn the_toggle_writes_the_brush_and_reset_tool_turns_it_off() {
    let mut h = bar(Tool::Dodge);
    h.state_mut().run("tools.setBrush", json!({"brush": {"size": 40.0, "hardness": 1.0}})).unwrap();
    h.run_steps(2);
    click_airbrush(&mut h);
    assert!(on(&h), "the button reads on");
    assert!(h.state().session.tools.brush.build_up, "and the tool's brush holds it");
    click_airbrush(&mut h);
    assert!(!on(&h) && !h.state().session.tools.brush.build_up, "a second click turns it off");
    click_airbrush(&mut h);

    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "tool.reset", json!({})).unwrap();
    h.run_steps(3);
    let brush = &h.state().session.tools.brush;
    assert!(!on(&h), "Reset Tool turns Airbrush off");
    assert!(!brush.build_up, "and the brush with it");
    assert_eq!(brush.size, 65.0, "with the rest of the tool's starting brush, unchanged by this item");
}

#[test]
fn the_airbrush_is_dodge_s_own_brush() {
    // Every painting tool keeps its own brush, so Dodge's Airbrush is Dodge's own: the Brush tool
    // never picks it up, and Dodge keeps it across a tool switch.
    let mut h = bar(Tool::Dodge);
    h.state_mut().run("tools.setBrush", json!({"brush": {"size": 40.0, "hardness": 1.0}})).unwrap();
    h.state_mut().ui.tool = Tool::Brush;
    h.run_steps(2);
    h.state_mut().ui.tool = Tool::Dodge;
    h.run_steps(2);
    click_airbrush(&mut h);
    assert!(h.state().session.tools.brush.build_up, "Dodge's brush took it");
    h.state_mut().ui.tool = Tool::Brush;
    h.run_steps(3);
    assert!(!h.state().session.tools.brush.build_up, "the Brush's own brush did not");
    h.state_mut().ui.tool = Tool::Dodge;
    h.run_steps(3);
    assert!(h.state().session.tools.brush.build_up, "and Dodge still has it");
}

/// The canvas at 1:1 over a grey 128 document, `tool` picked, its brush a 40 px hard round with
/// Airbrush `build_up` and Exposure 50 %.
fn canvas(tool: Tool, build_up: bool) -> H {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 120, "depth": 8})).unwrap();
    app.run("edit.fill", json!({"color": "#808080"})).unwrap();
    app.ui.tool = tool;
    crate::paint_mouse::sync_tool_brush(&mut app);
    app.run("tools.setBrush", json!({"brush": {"size": 40.0, "hardness": 1.0, "buildUp": build_up, "smoothing": {"amount": 0.0}}})).unwrap();
    app.ui.tool_options.exposure = 50.0;
    app.sync_views();
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(900.0, 600.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut Box<PhotocraftApp>| {
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
    let v = &mut h.state_mut().ui.views[0];
    v.zoom = 1.0;
    v.center = [100.0, 60.0];
    v.clear_pending();
    h.run_steps(2);
    h
}

fn screen(h: &H, x: f32, y: f32) -> Pos2 {
    crate::canvas::testing::to_screen(h.state(), [f64::from(x), f64::from(y)])
}

fn button(h: &mut H, down: bool) {
    let p = screen(h, 100.0, 60.0);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: down, modifiers: Modifiers::NONE });
    h.run_steps(1);
}

/// The grey under the press after a pointer pressed at the document centre and held still for
/// `frames` frames before it was released.
fn held(h: &mut H, frames: usize) -> i32 {
    let p = screen(h, 100.0, 60.0);
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
    button(h, true);
    h.run_steps(frames);
    button(h, false);
    h.run_steps(3);
    let app = h.state();
    let st = app.session.active().unwrap();
    let c = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(100, 60);
    (c[0] * 255.0).round() as i32
}

#[test]
fn a_held_dodge_with_airbrush_on_keeps_lightening() {
    let each: Vec<i32> = [0usize, 12, 24, 36].map(|frames| held(&mut canvas(Tool::Dodge, true), frames)).to_vec();
    for pair in each.windows(2) {
        assert!(pair[1] > pair[0], "held longer lightens strictly more: {each:?}");
    }
    assert!(each.last().unwrap() <= &255, "and never past white: {each:?}");
}

#[test]
fn a_held_dodge_with_airbrush_off_adds_nothing_after_the_first_dab() {
    let each: Vec<i32> = [0usize, 12, 24, 36].map(|frames| held(&mut canvas(Tool::Dodge, false), frames)).to_vec();
    assert!(each[0] > 128, "the press itself tones the spot: {each:?}");
    assert!(each.windows(2).all(|w| w[0] == w[1]), "off: a held pointer adds nothing more: {each:?}");
}

#[test]
fn a_held_dodge_stroke_is_one_history_state() {
    for build_up in [true, false] {
        let mut h = canvas(Tool::Dodge, build_up);
        let before = h.state().session.active().unwrap().history.past_len();
        let journal = h.state().session.journal.len();
        let _ = held(&mut h, 20);
        let app = h.state();
        assert_eq!(app.session.active().unwrap().history.past_len(), before + 1, "one Dodge Tool state, Airbrush {build_up}");
        assert_eq!(app.session.journal.len(), journal + 1, "one command, Airbrush {build_up}");
        assert_eq!(app.session.journal.iter().next_back().unwrap().0, "paint.dodge", "Airbrush {build_up}");
    }
}
