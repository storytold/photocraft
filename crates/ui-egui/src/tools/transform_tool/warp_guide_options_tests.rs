//! #12501: the gear right of the Warp style dropdown and its popup of guide-line options - Color,
//! Opacity, Density and the three Show Guides rows - what each choice does to the lines the grid
//! draws, and how the four values are remembered. The lines themselves are the framework's
//! `WarpGrid` paint and are pinned in storytold/craft-libs' `tests/it/warp.rs`; here the popup's
//! wiring and the values that reach the grid are.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::kittest::{NodeT, Queryable};

use super::*;
use crate::state::{Tool, ToolOptions, WarpGuideOptions, WarpGuideShow};

/// The options bar of an open Warp over a 640 px document.
fn warp_bar() -> Harness {
    let mut app = app_with_square(640, photocraft_geom::Rect::new(80, 80, 560, 560));
    let ctx = app.ctx().clone();
    begin_warp(&mut app, &ctx).unwrap();
    Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1800.0, 400.0)).build_ui_state(|ui, app| options_bar(app, ui), app)
}

/// The same bar with its guide-line popup open.
fn guide_popup() -> Harness {
    let mut h = warp_bar();
    h.run_steps(4);
    h.query_by_label("Set warp guide line display options").expect("the guide-line gear").click();
    h.run_steps(3);
    h
}

/// Types `text` into the bar's number field captioned `caption`, then Tabs out of it.
fn type_into(h: &mut Harness, caption: &str, text: &str) {
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some(caption)).expect(caption).click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
}

/// Whether a control named `label` with `role` is shown.
fn shown_as(h: &Harness, role: Role, label: &str) -> bool {
    h.query_all_by_role(role).any(|n| n.accesskit_node().label().as_deref() == Some(label))
}

#[test]
fn the_gear_on_the_warp_bar_opens_the_guide_line_popup() {
    let mut h = warp_bar();
    h.run_steps(4);
    assert!(h.query_by_label("Set warp guide line display options").is_some(), "the gear sits on the Warp bar");
    for (role, label) in [(Role::ColorWell, "Color"), (Role::SpinButton, "Opacity"), (Role::SpinButton, "Density"), (Role::RadioButton, "Always Show Guides")] {
        assert!(!shown_as(&h, role, label), "{label} waits for the gear");
    }
    h.query_by_label("Set warp guide line display options").expect("the gear").click();
    h.run_steps(3);
    for (role, label) in [
        (Role::ColorWell, "Color"),
        (Role::SpinButton, "Opacity"),
        (Role::SpinButton, "Density"),
        (Role::RadioButton, "Auto Show Guides"),
        (Role::RadioButton, "Always Show Guides"),
        (Role::RadioButton, "Never Show Guides"),
    ] {
        assert!(shown_as(&h, role, label), "{label} is in the popup");
    }
}

#[test]
fn a_row_or_a_typed_number_changes_the_guide_options() {
    let mut h = guide_popup();
    h.query_by_label("Never Show Guides").expect("the Never row").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.tool_options.warp_guides.show, WarpGuideShow::Never);
    h.query_by_label("Auto Show Guides").expect("the Auto row").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.tool_options.warp_guides.show, WarpGuideShow::Auto);
    type_into(&mut h, "Opacity", "40");
    assert_eq!(h.state().ui.tool_options.warp_guides.opacity, 40.0, "the Opacity field writes the option");
    type_into(&mut h, "Density", "7");
    assert_eq!(h.state().ui.tool_options.warp_guides.density, Some(7.0), "so does the Density field");
}

#[test]
fn an_entry_past_the_guide_range_takes_the_closest_value_and_alerts_once() {
    for (caption, typed, want, alerts) in [("Opacity", "150", 100.0, 1), ("Opacity", "0", 0.0, 0), ("Density", "40", 10.0, 1)] {
        let mut h = guide_popup();
        type_into(&mut h, caption, typed);
        let held = h.state().ui.tool_options.warp_guides;
        let got = if caption == "Opacity" { held.opacity } else { held.density.unwrap_or_default() };
        assert_eq!(got, want, "{caption}: {typed}");
        assert_eq!(crate::dialogs::alerts::alerts(h.state()).len(), alerts, "{caption}: {typed} gives {alerts} alert(s)");
    }
}

#[test]
fn the_colour_well_opens_the_colour_picker_for_the_guide_line() {
    let mut h = guide_popup();
    h.query_all_by_role(Role::ColorWell).find(|n| n.accesskit_node().label().as_deref() == Some("Color")).expect("the Color well").click();
    h.run_steps(3);
    let opened = crate::color_picker_ui::find(h.state(), |target| matches!(target, crate::color_picker_ui::PickTarget::WarpGuideColor));
    assert!(opened.is_some(), "the well opens the Color Picker on the guide colour");
}

#[test]
fn a_picked_guide_colour_is_kept_and_reaches_the_grid() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    let picker = crate::color_picker_ui::open_for_target(&mut app, "warpGuideColor", "Color Picker", [0.2, 0.4, 0.6]).unwrap();
    crate::color_picker_ui::testing::set_color(&mut app, picker, "#336699");
    crate::dialogs::testing::confirm(&mut app, picker).unwrap();
    let held = app.ui.tool_options.warp_guides;
    assert_eq!(held.color, Some([0.2, 0.4, 0.6]), "OK stores the picked colour in the tool options");
    let back: ToolOptions = serde_json::from_value(serde_json::to_value(&app.ui.tool_options).unwrap()).unwrap();
    assert_eq!(back.warp_guides, held, "and the saved options read back with it");
    assert_eq!(
        draw::guide_look(&held).line,
        Some(craft::ui::style::DataColor::new(egui::Color32::from_rgb(51, 102, 153))),
        "the grid paints the stored colour"
    );
}

#[test]
fn each_choice_reaches_the_grid_guide_look() {
    let own = draw::guide_look(&WarpGuideOptions::default());
    assert_eq!(own.line, None, "no colour picked leaves the theme's own overlay line");
    assert_eq!(own.share, 1.0, "100 % shows the line whole, as the grid always did");
    assert_eq!(own.density, None, "no density picked keeps the grid's own rule");
    assert!(own.visible, "the guides draw, as they always did");
    // Auto Show Guides was not measured, so it keeps today's behaviour: the guides are drawn.
    assert!(draw::guide_look(&WarpGuideOptions { show: WarpGuideShow::Auto, ..WarpGuideOptions::default() }).visible);
    let picked = WarpGuideOptions { color: Some([1.0, 0.0, 0.0]), opacity: 40.0, density: Some(7.0), show: WarpGuideShow::Never };
    let look = draw::guide_look(&picked);
    assert_eq!(look.line, Some(craft::ui::style::DataColor::new(egui::Color32::from_rgb(255, 0, 0))), "the picked colour reaches the lines");
    assert!((look.share - 0.4).abs() < 1e-6, "Opacity scales their alpha: {}", look.share);
    assert_eq!(look.density, Some(7), "and Density says how many lines a cell holds");
    assert!(!look.visible, "Never Show Guides leaves only the patch boundaries");
    // Values outside the fields' ranges are held to them, and a value that is not a number never
    // reaches the grid at all.
    let wild = draw::guide_look(&WarpGuideOptions { opacity: 150.0, density: Some(40.0), ..WarpGuideOptions::default() });
    assert_eq!((wild.share, wild.density), (1.0, Some(10)), "past the top of either range");
    let low = draw::guide_look(&WarpGuideOptions { opacity: -20.0, density: Some(-3.0), ..WarpGuideOptions::default() });
    assert_eq!((low.share, low.density), (0.0, Some(1)), "past the bottom of either range");
    let broken = draw::guide_look(&WarpGuideOptions { opacity: f32::NAN, density: Some(f32::NAN), ..WarpGuideOptions::default() });
    assert_eq!((broken.share, broken.density), (1.0, None), "neither number shows");
}

#[test]
fn reset_tool_and_a_damaged_file_give_back_the_starting_guides() {
    let mut o = ToolOptions {
        warp_guides: WarpGuideOptions { color: Some([1.0, 0.0, 0.0]), opacity: 30.0, density: Some(9.0), show: WarpGuideShow::Never },
        ..ToolOptions::default()
    };
    crate::tool_reset::reset_options(&mut o, Tool::Move);
    assert_eq!(o.warp_guides, WarpGuideOptions::default(), "Reset Tool restores the guides PhotoCraft starts with");
    let mut o = ToolOptions {
        warp_guides: WarpGuideOptions { color: Some([2.0, 0.0, 0.0]), opacity: 150.0, density: Some(40.0), show: WarpGuideShow::Always },
        ..ToolOptions::default()
    };
    crate::tool_memory::sanitize(&mut o);
    assert_eq!((o.warp_guides.opacity, o.warp_guides.density), (100.0, Some(10.0)), "a file outside either field's range is clamped");
    assert_eq!(o.warp_guides.color, None, "a colour outside the channels the picker answers is dropped, so the theme's own line shows");
}
