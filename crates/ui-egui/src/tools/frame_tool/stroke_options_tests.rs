//! #13873: the Frame tool's options bar shows a stroke swatch (No Color by default), a 1 px width
//! field taking 0–288 px with decimals, and the alignment, always; a colour turns the stroke on and
//! the next frame drawn takes it.
use super::*;
use crate::canvas::tool_event;
use egui::{Event, Key, Modifiers, accesskit::Role, vec2};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 600, "height": 400, "background": "white"})).unwrap();
    app.sync_views();
    app.ui.extras.snap = false;
    app.ui.tool = Tool::Frame;
    app
}

fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1100.0, 700.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            ui.horizontal(|ui| options_bar(app, ui, Tool::Frame));
            crate::dialogs::show(app, ui.ctx());
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
    h.run_steps(3);
    h
}

fn width_field(h: &Harness<'_, PhotocraftApp>) -> String {
    h.get_all_by_role(Role::SpinButton)
        .find(|n| n.accesskit_node().label().as_deref() == Some("Set Frame stroke width"))
        .expect("the width field")
        .value()
        .unwrap_or_default()
}

fn type_width(h: &mut Harness<'_, PhotocraftApp>, text: &str) {
    h.get_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some("Set Frame stroke width")).unwrap().click();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(Key::Enter);
    h.run_steps(3);
}

fn draw(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) {
    let m = Modifiers::NONE;
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, m);
}

fn last_frame_stroke(app: &PhotocraftApp) -> Option<photocraft_doc::ShapeStroke> {
    let st = app.session.active().unwrap();
    st.doc.walk().into_iter().filter_map(|(_, _, l)| l.frame.clone()).next_back().expect("a frame").stroke
}

#[test]
fn swatch_width_and_alignment_are_always_shown_with_the_defaults() {
    let h = harness(app());
    assert!(h.query_by_label("Set Frame stroke type").is_some(), "the swatch");
    assert!(h.query_all_by_label("Stroke").next().is_none() || h.get_all_by_role(Role::CheckBox).next().is_none(), "no Stroke checkbox");
    assert_eq!(width_field(&h), "1", "1 px");
    let combos: Vec<String> = h.get_all_by_role(Role::ComboBox).filter_map(|n| n.value()).collect();
    assert!(combos.iter().any(|v| v == "Center"), "Center: {combos:?}");
    assert_eq!(h.state().ui.frame_tool.stroke, None, "No Color: no stroke");
    assert_eq!(h.state().ui.frame_tool.stroke_style.width, 1.0);
    assert_eq!(h.state().ui.frame_tool.stroke_style.align, photocraft_doc::StrokeAlign::Center);
}

#[test]
fn the_width_takes_0_to_288_with_decimals_and_clamps_past_them() {
    let mut h = harness(app());
    type_width(&mut h, "2.5");
    assert_eq!(h.state().ui.frame_tool.stroke_style.width, 2.5, "decimals kept");
    type_width(&mut h, "0.3");
    assert_eq!(h.state().ui.frame_tool.stroke_style.width, 0.3);
    type_width(&mut h, "1000");
    assert_eq!(h.state().ui.frame_tool.stroke_style.width, 288.0);
    let alert = h.state().ui.dialogs.last().expect("the range alert");
    assert_eq!(alert.fields["message"], json!(STROKE_WIDTH_ALERT));
    h.state_mut().ui.dialogs.clear();
    type_width(&mut h, "-3");
    assert_eq!(h.state().ui.frame_tool.stroke_style.width, 0.0);
    assert_eq!(h.state().ui.frame_tool.stroke, None, "the stroke stays off while the width changes");
}

#[test]
fn a_colour_turns_the_stroke_on_and_no_color_turns_it_off_keeping_width_and_alignment() {
    use crate::shape_paint_ui::PaintChange;
    let style = photocraft_doc::ShapeStroke { width: 2.5, align: photocraft_doc::StrokeAlign::Inside, ..default_stroke_style() };
    let on = stroke_after_swatch(&style, &PaintChange::Color([1.0, 0.0, 0.0, 1.0])).expect("a change").expect("on");
    assert_eq!((on.width, on.align), (2.5, photocraft_doc::StrokeAlign::Inside));
    assert_eq!(on.paint, photocraft_doc::Fill::Solid(photocraft_doc::Color::rgba(1.0, 0.0, 0.0, 1.0)));
    assert_eq!(stroke_after_swatch(&on, &PaintChange::None), Some(None), "No Color: off");
    assert_eq!(stroke_after_swatch(&on, &PaintChange::OpenPicker), None, "no change");

    // Through the popover: a swatch turns the stroke on with that colour.
    let mut h = harness(app());
    type_width(&mut h, "2.5");
    h.get_by_label("Set Frame stroke type").click();
    h.run_steps(3);
    if h.query_by_label("RGB Red").is_none() {
        h.get_by_label("RGB").click();
        h.run_steps(2);
    }
    h.get_by_label("RGB Red").click();
    h.run_steps(2);
    let stroke = h.state().ui.frame_tool.stroke.clone().expect("on");
    assert_eq!(stroke.width, 2.5);
    assert_eq!(stroke.paint, photocraft_doc::Fill::Solid(photocraft_doc::Color::rgba(1.0, 0.0, 0.0, 1.0)));
    h.get_by_label("No Color").click();
    h.run_steps(2);
    assert_eq!(h.state().ui.frame_tool.stroke, None);
    assert_eq!(h.state().ui.frame_tool.stroke_style.width, 2.5, "the width is kept while off");
}

#[test]
fn the_next_frame_takes_the_options_bar_stroke() {
    let mut app = app();
    draw(&mut app, [20.0, 20.0], [120.0, 90.0]);
    assert_eq!(last_frame_stroke(&app), None, "off by default");
    let red = photocraft_doc::Fill::Solid(photocraft_doc::Color::rgba(1.0, 0.0, 0.0, 1.0));
    app.ui.frame_tool.stroke =
        Some(photocraft_doc::ShapeStroke { width: 2.5, align: photocraft_doc::StrokeAlign::Outside, paint: red.clone(), ..default_stroke_style() });
    draw(&mut app, [200.0, 150.0], [300.0, 230.0]);
    let s = last_frame_stroke(&app).expect("a stroke");
    assert_eq!((s.width, s.align, s.paint), (2.5, photocraft_doc::StrokeAlign::Outside, red));
}

#[test]
fn the_shape_tools_stroke_default_is_unchanged() {
    assert_eq!(photocraft_doc::ShapeStroke::default().width, 3.0);
}
