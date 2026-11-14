//! #12747: Convert Point has only the measured hint after the existing preset picker.

use egui::{Rect, Sense, accesskit::Role, epaint::Shape, vec2};
use egui_kittest::{
    Harness,
    kittest::{Queryable, by},
};

use crate::PhotocraftApp;
use crate::i18n::{Lang, with_language};
use crate::state::Tool;
use crate::theme::ThemeKind;

const HINT: &str = "No options for the Convert Point Tool.";

/// Exercise the real vector bar after a reserved picker slot; the caller's picker is unchanged.
fn bar(theme: ThemeKind, width: f32) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.ui.tool = Tool::ConvertPoint;
    app.ui.tool_options.vector_mode = "path".into();
    let mut h = Harness::builder().with_size(vec2(width, 100.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            with_language(Lang::EN, || {
                ui.horizontal_centered(|ui| {
                    // Reserve the space that belongs to the picker, without replacing or
                    // exercising its unmeasured menus in this focused options-bar test.
                    ui.allocate_exact_size(vec2(48.0, 22.0), Sense::hover());
                    let remaining = ui.available_rect_before_wrap();
                    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("convert-point-remaining"), remaining));
                    let tool = app.ui.tool;
                    assert!(super::options_bar(app, ui, tool));
                });
            });
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, theme);
    h.run_steps(4);
    h
}

/// Find laid-out hint glyphs, rather than mistaking a full-width label's hit rect for its text.
fn hint_rect(shape: &Shape) -> Option<Rect> {
    match shape {
        Shape::Text(text) if text.galley.text() == HINT => Some(text.galley.rect.translate(text.pos.to_vec2())),
        Shape::Vec(shapes) => shapes.iter().find_map(hint_rect),
        _ => None,
    }
}

/// The exact English fallback is visible, and Convert Point creates no editable options.
#[test]
fn convert_point_shows_only_the_fixed_hint() {
    let h = bar(ThemeKind::Studio, 1000.0);
    assert!(h.query_all(by().label(HINT).include_labels()).next().is_some());
    for role in [Role::Button, Role::CheckBox, Role::ComboBox, Role::Slider, Role::SpinButton, Role::TextInput] {
        assert!(h.query_all_by_role(role).next().is_none(), "unexpected {role:?} option");
    }
    assert_eq!(h.state().ui.tool_options.vector_mode, "path");
}

/// The actual text, not just its allocation, is centered in the space after the picker slot.
#[test]
fn convert_point_hint_is_centered_in_the_remaining_bar() {
    for theme in [ThemeKind::Pro, ThemeKind::Studio] {
        for width in [640.0, 1200.0] {
            let h = bar(theme, width);
            let remaining = h.ctx.data(|d| d.get_temp::<Rect>(egui::Id::new("convert-point-remaining"))).unwrap();
            let text = h.output().shapes.iter().find_map(|s| hint_rect(&s.shape)).expect("painted Convert Point hint");
            assert!((text.center().x - remaining.center().x).abs() <= 1.0, "horizontal centering: {text:?} in {remaining:?}");
            assert!((text.center().y - remaining.center().y).abs() <= 1.0, "vertical centering: {text:?} in {remaining:?}");
            assert!(remaining.contains_rect(text), "the hint must not overlap the picker slot");
        }
    }
}

/// Switching away restores each Pen-family bar's existing controls or hint, without leaking this hint.
#[test]
fn the_other_pen_tools_keep_their_existing_options() {
    let mut h = bar(ThemeKind::Studio, 1200.0);
    let alt = crate::shortcuts::pretty("Alt");
    let add = format!("Click a segment to add an anchor point · {alt}-click an anchor to delete it");
    let delete = format!("Click an anchor point to delete it · {alt}-click a segment to add one");
    for (tool, marker) in [
        (Tool::Pen, "Auto Add/Delete"),
        (Tool::FreeformPen, "Curve Fit:"),
        (Tool::CurvaturePen, "Path operations"),
        (Tool::AddAnchorPoint, add.as_str()),
        (Tool::DeleteAnchorPoint, delete.as_str()),
    ] {
        h.state_mut().ui.tool = tool;
        h.run_steps(3);
        assert!(h.query_all(by().label(HINT).include_labels()).next().is_none(), "Convert Point hint leaked to {tool:?}");
        assert!(h.query_all(by().label(marker).include_labels()).next().is_some(), "missing existing {tool:?} option: {marker}");
    }
}
