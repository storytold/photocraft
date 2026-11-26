//! #12501, as measured by the maintainers on 2026-11-26: the gear right of the Warp style dropdown
//! opens a popup whose Color is a dropdown of named entries (Classic, Default picked, then twelve
//! names, a separator after the pair), whose Opacity holds whole percents 1-100 from 70 and whose
//! Density holds whole lines 1-10 from 2, drawn in every patch of the mesh, plus the three Show
//! Guides rows - Auto gating the lines on a running drag. What each choice hands the framework's
//! `WarpGrid` and how the four values are remembered is pinned here; the lines themselves are the
//! framework's paint, pinned in storytold/craft-libs' `tests/it/warp.rs`.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::kittest::{NodeT, Queryable};

use super::*;
use crate::canvas::document_display::active_view;
use crate::state::{Tool, ToolOptions, WarpGuideColor, WarpGuideOptions, WarpGuideShow};

/// The options bar of an open Warp over a 640 px document.
fn warp_bar() -> Harness {
    let mut app = app_with_square(640, photocraft_geom::Rect::new(80, 80, 560, 560));
    let ctx = app.ctx().clone();
    begin_warp(&mut app, &ctx).unwrap();
    Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1800.0, 400.0)).build_ui_state(|ui, app| options_bar(app, ui), app)
}

/// The same bar with its guide-line popup open, tall enough for the whole colour list at once.
fn guide_popup() -> Harness {
    let mut app = app_with_square(640, photocraft_geom::Rect::new(80, 80, 560, 560));
    let ctx = app.ctx().clone();
    begin_warp(&mut app, &ctx).unwrap();
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1800.0, 900.0)).build_ui_state(|ui, app| options_bar(app, ui), app);
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

/// The number the popup's field named `caption` shows, as a person reads it off the field.
fn number_shown(h: &Harness, caption: &str) -> f32 {
    h.query_all_by_role(Role::SpinButton)
        .find(|n| n.accesskit_node().label().as_deref() == Some(caption))
        .expect(caption)
        .accesskit_node()
        .value()
        .unwrap_or_default()
        .parse()
        .unwrap_or(f32::NAN)
}

#[test]
fn the_gear_on_the_warp_bar_opens_the_guide_line_popup() {
    let mut h = warp_bar();
    h.run_steps(4);
    assert!(h.query_by_label("Set warp guide line display options").is_some(), "the gear sits on the Warp bar");
    for (role, label) in [(Role::ComboBox, "Color"), (Role::SpinButton, "Opacity"), (Role::SpinButton, "Density"), (Role::RadioButton, "Always Show Guides")] {
        assert!(!shown_as(&h, role, label), "{label} waits for the gear");
    }
    h.query_by_label("Set warp guide line display options").expect("the gear").click();
    h.run_steps(3);
    for (role, label) in [
        (Role::ComboBox, "Color"),
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
fn the_colour_dropdown_holds_the_measured_entries_in_measured_order_with_default_picked() {
    let mut h = guide_popup();
    assert_eq!(h.state().ui.tool_options.warp_guides.color, WarpGuideColor::Default, "Default is picked at the start");
    h.get_by_role_and_label(Role::ComboBox, "Color").click();
    h.run_steps(3);
    // Every measured name is a row, and the rows run down the list in the measured order. A missing
    // row fails the get_by_label with the whole tree in its message.
    let names = ["Classic", "Default", "Green", "Chartreuse", "Yellow", "Orange", "Red", "Magenta", "Purple", "Indigo", "Fuchsia", "Seafoam", "White", "Black"];
    let tops: Vec<(f32, &str)> = names.iter().map(|name| (h.get_by_label(name).rect().top(), *name)).collect();
    for pair in tops.windows(2) {
        assert!(pair[0].0 < pair[1].0, "{} sits above {} in the dropdown", pair[0].1, pair[1].1);
    }
    h.get_by_label("Red").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.tool_options.warp_guides.color, WarpGuideColor::Red, "picking a row writes the option");
}

#[test]
fn the_popup_starts_at_the_measured_values() {
    let h = guide_popup();
    let held = h.state().ui.tool_options.warp_guides;
    assert_eq!((held.opacity, held.density, held.show), (70.0, 2.0, WarpGuideShow::Always), "70 %, 2 lines, Always picked");
    assert_eq!(number_shown(&h, "Opacity"), 70.0, "the Opacity field reads 70 at the start");
    assert_eq!(number_shown(&h, "Density"), 2.0, "and the Density field 2");
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
    assert_eq!(h.state().ui.tool_options.warp_guides.density, 7.0, "so does the Density field");
}

#[test]
fn an_entry_past_the_whole_range_takes_the_closest_value_and_alerts_once() {
    for (caption, typed, want, alerts) in [("Opacity", "150", 100.0, 1), ("Opacity", "0", 1.0, 1), ("Opacity", "50", 50.0, 0), ("Density", "40", 10.0, 1)] {
        let mut h = guide_popup();
        type_into(&mut h, caption, typed);
        let held = h.state().ui.tool_options.warp_guides;
        let got = if caption == "Opacity" { held.opacity } else { held.density };
        assert_eq!(got, want, "{caption}: {typed}");
        assert_eq!(crate::dialogs::alerts::alerts(h.state()).len(), alerts, "{caption}: {typed} gives {alerts} alert(s)");
    }
}

#[test]
fn a_fraction_in_a_whole_field_alerts_and_keeps_the_previous_value() {
    // The field's reader takes the machine's decimal mark - a comma on a German box, a dot elsewhere
    // (the framework reads it from the system region; PhotoCraft's own helper answers None in tests
    // on purpose, so the test asks the framework directly). Typing it the other way commits nothing
    // and proves nothing, so the typed text follows the reader.
    let dec = craft::i18n::Region::system().decimal.filter(|c| matches!(c, '.' | ',')).unwrap_or('.');
    let mut h = guide_popup();
    type_into(&mut h, "Opacity", "40");
    assert_eq!(h.state().ui.tool_options.warp_guides.opacity, 40.0);
    type_into(&mut h, "Opacity", &format!("50{dec}6"));
    assert_eq!(h.state().ui.tool_options.warp_guides.opacity, 40.0, "50{dec}6 keeps the previous value: the field holds whole percents");
    assert_eq!(crate::dialogs::alerts::alerts(h.state()).len(), 1, "and the same range alert answers it");
}

#[test]
fn each_choice_reaches_the_grid_guide_look() {
    let own = draw::guide_look(&WarpGuideOptions::default());
    assert_eq!(own.line, Some(craft::ui::style::DataColor::new(egui::Color32::from_rgb(0x78, 0xAD, 0xF6))), "the Default entry paints its blue");
    assert!((own.share - 0.7).abs() < 1e-6, "70 % scales the line's alpha to seven tenths: {}", own.share);
    assert_eq!(own.density, Some(2), "two lines per cell, the measured start, from the first frame");
    assert_eq!(own.show, craft::imaging::ui::warp::WarpGuidesShow::Always, "and Always draws them everywhere");
    let classic = draw::guide_look(&WarpGuideOptions { color: WarpGuideColor::Classic, ..WarpGuideOptions::default() });
    assert_eq!(classic.line, None, "Classic leaves the theme's own overlay line standing");
    let named = draw::guide_look(&WarpGuideOptions { color: WarpGuideColor::Red, ..WarpGuideOptions::default() });
    assert_eq!(named.line, Some(craft::ui::style::DataColor::new(egui::Color32::from_rgb(0xFF, 0x00, 0x00))), "a named entry paints its colour");
    // The three Show Guides rows: Auto gates the lines on the grid's running drag (the framework
    // hides them at rest and under a hover - pinned by its own screen trial), Always draws always,
    // Never never, leaving the boundaries.
    for (row, want) in [
        (WarpGuideShow::Auto, craft::imaging::ui::warp::WarpGuidesShow::WhileDragging),
        (WarpGuideShow::Always, craft::imaging::ui::warp::WarpGuidesShow::Always),
        (WarpGuideShow::Never, craft::imaging::ui::warp::WarpGuidesShow::Never),
    ] {
        assert_eq!(draw::guide_look(&WarpGuideOptions { show: row, ..WarpGuideOptions::default() }).show, want, "{row:?}");
    }
    // Values outside the fields' ranges are held to them, and a value that is not a number never
    // reaches the grid at all.
    let wild = draw::guide_look(&WarpGuideOptions { opacity: 150.0, density: 40.0, ..WarpGuideOptions::default() });
    assert_eq!((wild.share, wild.density), (1.0, Some(10)), "past the top of either range");
    let low = draw::guide_look(&WarpGuideOptions { opacity: -20.0, density: -3.0, ..WarpGuideOptions::default() });
    assert_eq!((low.share, low.density), (0.0, Some(1)), "past the bottom of either range");
    let broken = draw::guide_look(&WarpGuideOptions { opacity: f32::NAN, density: f32::NAN, ..WarpGuideOptions::default() });
    assert_eq!((broken.share, broken.density), (1.0, Some(2)), "neither broken number shows, and the density falls back to the start");
}

#[test]
fn the_colour_choice_survives_a_save_and_reload() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    app.ui.tool_options.warp_guides = WarpGuideOptions { color: WarpGuideColor::Seafoam, opacity: 40.0, density: 6.0, show: WarpGuideShow::Auto };
    let back: ToolOptions = serde_json::from_value(serde_json::to_value(&app.ui.tool_options).unwrap()).unwrap();
    assert_eq!(back.warp_guides, app.ui.tool_options.warp_guides, "the picked entry and the two whole numbers read back after a save and reload");
}

#[test]
fn reset_tool_and_a_damaged_file_give_back_the_starting_guides() {
    let mut o = ToolOptions {
        warp_guides: WarpGuideOptions { color: WarpGuideColor::Red, opacity: 30.0, density: 9.0, show: WarpGuideShow::Never },
        ..ToolOptions::default()
    };
    crate::tool_reset::reset_options(&mut o, Tool::Move);
    assert_eq!(o.warp_guides, WarpGuideOptions::default(), "Reset Tool restores the guides PhotoCraft starts with");
    let mut o = ToolOptions {
        warp_guides: WarpGuideOptions { color: WarpGuideColor::Black, opacity: 150.0, density: 40.0, show: WarpGuideShow::Always },
        ..ToolOptions::default()
    };
    crate::tool_memory::sanitize(&mut o);
    assert_eq!((o.warp_guides.opacity, o.warp_guides.density), (100.0, 10.0), "a file outside either field's range is clamped");
    assert_eq!(o.warp_guides.color, WarpGuideColor::Black, "a named colour the dropdown can answer is left alone");
    let mut o = ToolOptions {
        warp_guides: WarpGuideOptions { color: WarpGuideColor::Classic, opacity: 50.6, density: 2.4, show: WarpGuideShow::Always },
        ..ToolOptions::default()
    };
    crate::tool_memory::sanitize(&mut o);
    assert_eq!((o.warp_guides.opacity, o.warp_guides.density), (51.0, 2.0), "a file holding fractions gets whole numbers");
    let mut o = ToolOptions {
        warp_guides: WarpGuideOptions { color: WarpGuideColor::Default, opacity: f32::NAN, density: f32::INFINITY, show: WarpGuideShow::Always },
        ..ToolOptions::default()
    };
    crate::tool_memory::sanitize(&mut o);
    assert_eq!((o.warp_guides.opacity, o.warp_guides.density), (70.0, 2.0), "a file holding no number at all takes the starting values");
}

/// Density is a standing whole number now, so the measured start of 2 draws its lines in every
/// patch, also after a split cuts the mesh: the field keeps showing 2 and the grid keeps being
/// handed 2 (the framework's trial pins the per-patch curves themselves).
#[test]
fn density_two_drives_every_patch_after_a_split_too() {
    let mut h = warp_bar();
    h.run_steps(4);
    h.query_by_label("Set warp guide line display options").expect("the guide-line gear").click();
    h.run_steps(3);
    assert_eq!(number_shown(&h, "Density"), 2.0, "an uncut grid shows 2");
    // Arm the Split button and cut once, the way the armed grid reports a click on the canvas.
    h.state_mut().transform_preview.as_mut().unwrap().split = Some(WarpSplit::Vertical);
    let before = session_warp(h.state()).mesh.clone();
    let cut = before.split(WarpSplit::Vertical, 0, 0, 0.5, 0.5);
    let mut events = EditEvents::new();
    events.push(EditEvent::set(Mixed::Same(before), cut, "Split warp".into(), None));
    grid_edit(h.state_mut(), &events, true);
    h.run_steps(4);
    if !shown_as(&h, Role::SpinButton, "Density") {
        h.query_by_label("Set warp guide line display options").expect("the guide-line gear").click();
        h.run_steps(3);
    }
    assert_eq!(number_shown(&h, "Density"), 2.0, "the field still reads 2 on the cut grid");
    assert_eq!(h.state().ui.tool_options.warp_guides.density, 2.0, "and the option stands at 2");
    assert_eq!(draw::guide_look(&h.state().ui.tool_options.warp_guides).density, Some(2), "the cut mesh is handed 2 lines per cell as ever");
}

/// A Cylinder warp's bar carries the same gear, but the cylinder's canvas never reaches the grid:
/// `show_warp` returns after the cylinder's own controls, before a grid is built or handed the
/// guide options. Pinned at the canvas: with the options set to loud values, a drag answers
/// as the cylinder's surface (the whole cylinder follows, its shape untouched) and never as a
/// guide-drawing grid, which would turn the warp Custom.
#[test]
fn the_guide_options_reach_nothing_on_a_cylinder_warp_canvas() {
    let mut app = app_with_square(640, photocraft_geom::Rect::new(80, 80, 560, 560));
    let ctx = app.ctx().clone();
    begin_warp(&mut app, &ctx).unwrap();
    let cyl = cylinder_start(rect_of(open(&app)));
    set_warp(&mut app, cyl);
    settle(&mut app);
    app.ui.tool_options.warp_guides = WarpGuideOptions { color: WarpGuideColor::Red, opacity: 40.0, density: 9.0, show: WarpGuideShow::Never };
    let mut h = harness(app, egui::vec2(1400.0, 900.0));
    h.run_steps(4);
    assert_eq!(session_warp(h.state()).warp.style, WarpStyle::Cylinder, "the session holds a cylinder warp");
    let (before, at) = (cylinder_of(h.state()), active_view(h.state()).unwrap().to_screen(Point::new(320.0, 320.0)));
    h.hover_at(at);
    h.run_steps(1);
    h.drag_at(at);
    h.run_steps(1);
    for k in 1..=4 {
        h.hover_at(at + egui::vec2(10.0 * k as f32, 0.0));
        h.run_steps(1);
    }
    h.drop_at(at + egui::vec2(40.0, 0.0));
    h.run_steps(3);
    assert_eq!(
        session_warp(h.state()).warp.style,
        WarpStyle::Cylinder,
        "the cylinder answered the canvas, not a grid: its bar's guide options reach nothing there"
    );
    let after = cylinder_of(h.state());
    assert!(after[0] > before[0] + 5.0 && after[2] > before[2] + 5.0, "the whole cylinder moved with the drag: {before:?} -> {after:?}");
    for i in 4..7 {
        assert!((after[i] - before[i]).abs() < 1e-6, "its shape stayed: {before:?} -> {after:?}");
    }
}
