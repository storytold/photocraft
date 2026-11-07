//! The Brushes view has one scrollbar: its preset list (#11709).

use egui::vec2;
use egui_kittest::{Harness, kittest::Queryable};

use crate::PhotocraftApp;

/// A footer button, always drawn and always enabled.
const FOOTER: &str = "Create new brush from the current settings";

/// The dock with only the Brushes panel shown, so its group is the column's filler and its height
/// follows the window.
fn harness(size: egui::Vec2, ppp: f32) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.edit_prefs(|p| p.general.auto_show_home_screen = false);
    app.ui.panels = crate::state::Panels { brushes: true, layers: false, properties: false, color: false, ..Default::default() };
    let mut h = Harness::builder().with_size(size).with_pixels_per_point(ppp).with_max_steps(64).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::panels::dock_area(app, ui);
            egui::CentralPanel::default().show(ui, |_| {});
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(6);
    h
}

fn footer_bottom(h: &Harness<'_, PhotocraftApp>) -> Option<f32> {
    h.query_by_label(FOOTER).map(|n| n.rect().max.y)
}

fn scroll(h: &mut Harness<'_, PhotocraftApp>, at: egui::Pos2, notches: i32) {
    h.hover_at(at);
    h.run_steps(2);
    for _ in 0..notches {
        h.event(egui::Event::MouseWheel {
            source: egui::MouseWheelSource::Unknown,
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, -1.0),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
        });
        h.run_steps(1);
    }
    h.run_steps(2);
}

/// The panel's bottom section stays in the panel whatever the window height: the list is bounded
/// by what is left after it. The dock wrapped the whole body in its own scroll area, and the list
/// asked for all the remaining height, so the body was taller than its group by about the footer's
/// height and that outer area drew a second bar that only scrolled the footer out of view.
#[test]
fn the_brushes_footer_stays_inside_its_panel() {
    // Heights where the list used to ask for everything the group left it, so the body overflowed
    // by the footer: at 100 % and at 125 % scaling, short and tall.
    for (ppp, height) in [(1.0, 420.0), (1.25, 420.0), (1.0, 700.0), (1.25, 900.0)] {
        let h = harness(vec2(1100.0, height), ppp);
        let panel = crate::panel_docking::probe::panel_rect(&h.ctx, "brushes").expect("the Brushes panel is docked");
        let footer = footer_bottom(&h).unwrap_or_else(|| panic!("{ppp}x, {height} high: the footer is drawn"));
        assert!(footer <= panel.max.y + 0.5, "{ppp}x, {height} high: the footer ends at {footer}, the panel at {}", panel.max.y);
    }
}

/// Scrolling in the Brushes view moves only its list: the footer keeps its place, and a wheel over
/// the footer (outside the list) moves nothing at all.
#[test]
fn scrolling_the_brushes_list_does_not_move_the_footer() {
    let mut h = harness(vec2(1100.0, 700.0), 1.0);
    let panel = crate::panel_docking::probe::panel_rect(&h.ctx, "brushes").expect("the Brushes panel is docked");
    let name = h.state().session.tools.presets[0].name.clone();
    let rest = footer_bottom(&h).expect("the footer shows");
    let first_row = h.query_by_label(&name).expect("the first preset").rect();
    scroll(&mut h, first_row.center(), 20);
    assert_eq!(footer_bottom(&h), Some(rest), "the footer stayed put while the list scrolled");
    let scrolled = h.query_by_label(&name).map(|n| n.rect()) != Some(first_row);
    assert!(scrolled || h.query_by_label(&name).is_none(), "the list itself scrolled");
    // A wheel over the footer is outside the list, and the panel has nothing else to scroll.
    let footer_rect = h.query_by_label(FOOTER).expect("the footer").rect();
    scroll(&mut h, footer_rect.center(), 10);
    assert_eq!(footer_bottom(&h), Some(rest), "the footer did not scroll out of the panel");
    assert!(footer_bottom(&h).is_some_and(|y| y <= panel.max.y + 0.5), "and it is still inside it");
}

/// A rename bar above the list is part of the panel too: the footer stays in view while a rename is
/// open, and it is the list that gives way, not the panel's bottom edge.
#[test]
fn the_footer_stays_visible_while_a_preset_is_being_renamed() {
    let mut h = harness(vec2(1100.0, 420.0), 1.0);
    let panel = crate::panel_docking::probe::panel_rect(&h.ctx, "brushes").expect("the Brushes panel is docked");
    let before = footer_bottom(&h).expect("the footer shows");
    let row = h.state().session.tools.presets[1].name.clone();
    let first_row = h.query_by_label(&row).expect("a preset row").rect();
    let name = h.state().session.tools.presets[0].name.clone();
    h.state_mut().ui.brushes_panel.renaming = Some(crate::brush_panel::Renaming { name: name.clone(), text: name, ..Default::default() });
    h.run_steps(3);
    let footer = footer_bottom(&h).unwrap_or_else(|| panic!("the footer is drawn while renaming"));
    assert!(footer <= panel.max.y + 0.5, "the footer ends at {footer}, the panel at {}", panel.max.y);
    assert!(footer <= before + 0.5, "opening a rename takes room out of the list, not out of the footer: {before} → {footer}");
    assert!(h.query_by_label("OK").is_some(), "the rename bar is open");
    assert_ne!(h.query_by_label(&row).map(|n| n.rect()), Some(first_row), "and the list moved down by it");
}
