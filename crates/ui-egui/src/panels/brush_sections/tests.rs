use super::*;
use crate::theme::AppTheme;
use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};

/// What a trial draws: the app (preset library, open curves), the working brush and the active tool.
struct State {
    app: Box<PhotocraftApp>,
    brush: BrushSettings,
    tool: Tool,
}

fn harness(section: usize, brush: BrushSettings, tool: Tool) -> Harness<'static, State> {
    let mut ready = false;
    let state = State { app: PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default()), brush, tool };
    let mut h = Harness::builder().with_context(state.app.ctx().clone()).with_size(vec2(340.0, 1400.0)).build_ui_state(
        move |ui, state: &mut State| {
            if !ready {
                PhotocraftApp::setup_context(ui.ctx(), AppTheme::pro_medium());
                ready = true;
                return;
            }
            section_body(&mut state.app, ui, &mut state.brush, section, state.tool);
        },
        state,
    );
    h.run_steps(3);
    h
}

/// The number field showing `value`.
fn spin<'h>(h: &'h Harness<'static, State>, value: f64) -> egui_kittest::Node<'h> {
    h.query_all_by_role(Role::SpinButton)
        .find(|n| n.accesskit_node().numeric_value() == Some(value))
        .unwrap_or_else(|| panic!("a number field showing {value}"))
}

/// Presses `key` in the field showing `value`.
fn step(h: &mut Harness<'static, State>, value: f64, key: egui::Key) {
    spin(h, value).click();
    h.key_press(key);
    h.run_steps(2);
}

fn minimum_brush() -> BrushSettings {
    let mut b = BrushSettings::default();
    b.shape_dynamics.size.minimum = 0.2;
    b.shape_dynamics.roundness.minimum = 0.35;
    b.transfer.opacity.minimum = 0.4;
    b.transfer.flow.minimum = 0.55;
    b.transfer.wetness.minimum = 0.6;
    b.transfer.mix.minimum = 0.75;
    b
}

fn transfer_mixer_brush() -> BrushSettings {
    let mut b = BrushSettings::default();
    b.transfer.enabled = true;
    for (dynamic, jitter, minimum, fade_steps) in [
        (&mut b.transfer.opacity, 0.13, 0.17, 119),
        (&mut b.transfer.flow, 0.27, 0.29, 229),
        (&mut b.transfer.wetness, 0.41, 0.43, 339),
        (&mut b.transfer.mix, 0.63, 0.67, 449),
    ] {
        *dynamic = Dynamic { control: Control::Fade, jitter, minimum, fade_steps, ..Default::default() };
    }
    b
}

/// #3825: Mixer-only blocks follow the tool, including sliders, minimums and Fade steps; changing the tool changes
/// only availability.
#[test]
fn transfer_mixer_controls_follow_the_tool() {
    let mut h = harness(6, transfer_mixer_brush(), Tool::Brush);
    let before = h.state().brush.clone();
    for tool in Tool::ALL {
        h.state_mut().tool = tool;
        h.run_steps(2);
        for (mixer_only, values) in [(false, [13.0, 17.0, 119.0, 27.0, 29.0, 229.0]), (true, [41.0, 43.0, 339.0, 63.0, 67.0, 449.0])] {
            for value in values {
                assert_eq!(spin(&h, value).accesskit_node().is_disabled(), mixer_only && tool != Tool::MixerBrush, "{tool:?}, {value}");
            }
        }
        for (index, node) in h.query_all_by_role(Role::ComboBox).enumerate() {
            assert_eq!(node.accesskit_node().is_disabled(), index >= 2 && tool != Tool::MixerBrush, "{tool:?}: controller {index}");
        }
        assert_eq!(h.state().brush, before, "changing tool context only changes availability");
    }
    h.state_mut().tool = Tool::MixerBrush;
    h.state_mut().brush.transfer.wetness.control = Control::Off;
    h.run_steps(2);
    assert!(spin(&h, 43.0).accesskit_node().is_disabled(), "Control Off still disables the applicable minimum");
}

fn set_controls(b: &mut BrushSettings, control: Control) {
    for d in [
        &mut b.shape_dynamics.size,
        &mut b.shape_dynamics.roundness,
        &mut b.transfer.opacity,
        &mut b.transfer.flow,
        &mut b.transfer.wetness,
        &mut b.transfer.mix,
    ] {
        d.control = control;
    }
}

/// #3825: all six minimum fields follow their own Control; jitter remains editable.
#[test]
fn minimum_fields_are_disabled_only_when_their_control_is_off() {
    for (section, values) in [(1, vec![20.0, 35.0]), (6, vec![40.0, 55.0, 60.0, 75.0])] {
        let mut h = harness(section, minimum_brush(), Tool::MixerBrush);
        for (control, _) in CONTROLS {
            set_controls(&mut h.state_mut().brush, control);
            h.run_steps(2);
            for value in &values {
                assert_eq!(spin(&h, *value).accesskit_node().is_disabled(), control == Control::Off, "section {section}, minimum {value}, {control:?}");
            }
            assert!(
                h.query_all_by_role(Role::SpinButton).any(|n| n.accesskit_node().numeric_value() == Some(0.0) && !n.accesskit_node().is_disabled()),
                "jitter stays editable"
            );
        }
        set_controls(&mut h.state_mut().brush, Control::PenPressure);
        if section == 1 {
            h.state_mut().brush.shape_dynamics.size.control = Control::Off;
        } else {
            h.state_mut().brush.transfer.opacity.control = Control::Off;
        }
        h.run_steps(2);
        for (index, value) in values.iter().enumerate() {
            assert_eq!(spin(&h, *value).accesskit_node().is_disabled(), index == 0, "each minimum follows its own control");
        }
    }
}

/// Spacing shows 1 to 1000 % and Scatter 0 to 1000 %: a key step changes the stored fraction by 0.01, the ends hold,
/// a toggle keeps the saved value and only the edited field changes.
#[test]
fn percent_fields_store_whole_percents_as_fractions_in_range() {
    let near = |a: f32, b: f32| (a - b).abs() < 1e-6;

    let mut h = harness(0, BrushSettings { spacing: 0.42, spacing_enabled: false, ..Default::default() }, Tool::Brush);
    assert!(spin(&h, 42.0).accesskit_node().is_disabled(), "Spacing Off disables its field");
    h.get_by_role_and_label(Role::CheckBox, "Spacing").click();
    h.run_steps(2);
    assert!(h.state().brush.spacing_enabled);
    assert_eq!(h.state().brush.spacing, 0.42, "enabling keeps the saved value");
    step(&mut h, 42.0, egui::Key::ArrowUp);
    assert!(near(h.state().brush.spacing, 0.43), "a step is one percent: {}", h.state().brush.spacing);
    for (spacing, value, key) in [(0.01, 1.0, egui::Key::ArrowDown), (10.0, 1000.0, egui::Key::ArrowUp)] {
        h.state_mut().brush.spacing = spacing;
        h.run_steps(2);
        step(&mut h, value, key);
        assert!(near(h.state().brush.spacing, spacing), "Spacing stays within 1..1000 %: {}", h.state().brush.spacing);
    }

    for section in [2, 4] {
        let mut b = BrushSettings::default();
        b.scattering.scatter.jitter = 2.137;
        b.scattering.count = 5;
        b.dual_brush.scatter = 2.137;
        b.dual_brush.count = 7;
        let mut h = harness(section, b.clone(), Tool::Brush);
        h.get_by_role_and_label(Role::CheckBox, "Both Axes").click();
        h.run_steps(2);
        if section == 2 {
            b.scattering.both_axes = true;
        } else {
            b.dual_brush.both_axes = true;
        }
        assert_eq!(h.state().brush, b, "a toggle does not round scatter or change other fields");
        step(&mut h, 214.0, egui::Key::ArrowUp);
        let got = if section == 2 { h.state().brush.scattering.scatter.jitter } else { h.state().brush.dual_brush.scatter };
        assert!(near(got, 2.15), "section {section}: {got}");
        for (value, key) in [(0.0, egui::Key::ArrowDown), (10.0, egui::Key::ArrowUp)] {
            if section == 2 {
                h.state_mut().brush.scattering.scatter.jitter = value;
            } else {
                h.state_mut().brush.dual_brush.scatter = value;
            }
            h.run_steps(2);
            step(&mut h, f64::from(value) * 100.0, key);
            let got = if section == 2 { h.state().brush.scattering.scatter.jitter } else { h.state().brush.dual_brush.scatter };
            assert!(near(got, value), "section {section}: Scatter stays within 0..1000 %: {got}");
        }
    }
}
