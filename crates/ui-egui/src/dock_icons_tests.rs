//! Collapse to Icons tests (#4059): the double arrow, the icon strip, flyouts, persistence.

use egui::{Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::*;
use crate::dock::{DockLayout, last_rects};
use crate::theme::ThemeKind;

const THEMES: [ThemeKind; 2] = [ThemeKind::ProMedium, ThemeKind::Studio];

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
    app
}

fn harness(app: PhotocraftApp, size: egui::Vec2, theme: ThemeKind) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(size).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::panels::right_dock(app, ui);
            egui::CentralPanel::default().show(ui, |_| {});
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, theme);
    h.state_mut().ui.theme = theme;
    h.run_steps(4);
    h
}

fn click(h: &mut Harness<'static, PhotocraftApp>, p: Pos2) {
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
}

fn icon(h: &Harness<'static, PhotocraftApp>, g: Group, tab: usize) -> Rect {
    last_icons(&h.ctx).into_iter().find(|(x, i, _)| *x == g && *i == tab).map(|(_, _, r)| r).unwrap_or_else(|| panic!("no icon for {g:?} tab {tab}"))
}

#[test]
fn the_double_arrow_collapses_the_column_to_icons_and_expands_it_again() {
    for theme in THEMES {
        let mut h = harness(app(), vec2(1200.0, 800.0), theme);
        let pro = theme.is_pro();
        assert!(!h.state().ui.dock.iconic);
        assert!(last_icons(&h.ctx).is_empty(), "{theme:?}: an expanded column has no icon strip");
        let wide = last_rects(&h.ctx).first().map(|(_, r)| r.width()).unwrap();
        h.get_by_label("Collapse to Icons").click();
        h.run_steps(3);
        assert!(h.state().ui.dock.iconic, "{theme:?}");
        // One icon per visible panel of every group the column shows, in the layout's order.
        let icons = last_icons(&h.ctx);
        let shown: Vec<Group> = h.state().ui.dock.order().into_iter().filter(|g| icons.iter().any(|(x, _, _)| x == g)).collect();
        let expected: Vec<(Group, usize)> = shown.iter().flat_map(|g| h.state().ui.dock.visible_tabs(*g, pro).into_iter().map(|(i, _)| (*g, i))).collect();
        assert_eq!(icons.iter().map(|(g, i, _)| (*g, *i)).collect::<Vec<_>>(), expected, "{theme:?}");
        assert!(icons.iter().any(|(g, _, _)| *g == Group::Layers), "{theme:?}: Layers has its icon");
        assert!(icons.iter().all(|(_, _, r)| r.width() < wide / 4.0), "{theme:?}: the strip is narrow");
        assert!(last_flyout(&h.ctx).is_none(), "{theme:?}: no flyout until an icon is clicked");
        h.get_by_label("Expand Panels").click();
        h.run_steps(3);
        assert!(!h.state().ui.dock.iconic, "{theme:?}");
        assert!(last_icons(&h.ctx).is_empty() || last_rects(&h.ctx).iter().all(|(_, r)| r.width() > wide - 1.0));
    }
}

#[test]
fn an_icon_opens_its_panel_in_a_flyout_and_a_second_click_closes_it() {
    for theme in THEMES {
        for auto in [false, true] {
            let mut app = app();
            app.session.prefs.edit(|p| p.workspace.auto_collapse_icon_panels = auto);
            app.ui.dock.iconic = true;
            // A collapsed group opens expanded from its icon.
            app.ui.dock.set_collapsed(Group::Layers, true);
            let mut h = harness(app, vec2(1200.0, 800.0), theme);
            let channels = icon(&h, Group::Layers, 1);
            click(&mut h, channels.center());
            assert_eq!(h.state().ui.dock.flyout, Some(Group::Layers), "{theme:?} auto={auto}");
            assert_eq!(h.state().ui.dock_tabs.layers, 1, "{theme:?}: the clicked panel is the one shown");
            assert!(!h.state().ui.dock.is_collapsed(Group::Layers));
            let fly = last_flyout(&h.ctx).unwrap();
            assert!(fly.right() <= channels.left(), "{theme:?}: the flyout opens beside the strip");
            assert!(fly.width() > 200.0 && fly.height() > 100.0, "{fly:?}");
            assert!(h.ctx.content_rect().contains_rect(fly), "{theme:?}: kept on screen");
            // The flyout is the real panel group: the dock draws its tab strip inside it.
            let drawn = last_rects(&h.ctx);
            assert!(drawn.iter().any(|(g, r)| *g == Group::Layers && fly.contains_rect(*r)), "{drawn:?}");
            // A click inside keeps it open.
            click(&mut h, fly.center());
            assert_eq!(h.state().ui.dock.flyout, Some(Group::Layers));
            // Outside (the canvas) closes it only with Auto-Collapse Iconic Panels on.
            click(&mut h, Pos2::new(fly.left() / 2.0, fly.center().y));
            assert_eq!(h.state().ui.dock.flyout.is_none(), auto, "{theme:?} auto={auto}");
            if !auto {
                // Another icon switches to its panel; its own icon again closes it.
                let layers = icon(&h, Group::Layers, 0);
                click(&mut h, layers.center());
                assert_eq!((h.state().ui.dock.flyout, h.state().ui.dock_tabs.layers), (Some(Group::Layers), 0));
                click(&mut h, layers.center());
                assert_eq!(h.state().ui.dock.flyout, None, "{theme:?}: the icon toggles its flyout");
                assert!(last_flyout(&h.ctx).is_none());
            }
        }
    }
}

#[test]
fn the_panel_menu_offers_collapse_to_icons_and_expand_panels() {
    let mut h = harness(app(), vec2(1200.0, 800.0), ThemeKind::ProMedium);
    let menu = crate::dock::last_strips(&h.ctx).into_iter().find(|s| s.group == Group::Layers).unwrap().menu;
    click(&mut h, menu.center());
    // The bar's double arrow carries the same label; the menu's is the last one drawn.
    h.get_all_by_label("Collapse to Icons").last().unwrap().click();
    h.run_steps(3);
    assert!(h.state().ui.dock.iconic);
    let layers = icon(&h, Group::Layers, 0);
    click(&mut h, layers.center());
    let menu = crate::dock::last_strips(&h.ctx).into_iter().find(|s| s.group == Group::Layers).unwrap().menu;
    click(&mut h, menu.center());
    h.get_all_by_label("Expand Panels").last().unwrap().click();
    h.run_steps(3);
    assert!(!h.state().ui.dock.iconic);
    assert_eq!(h.state().ui.dock.flyout, None);
}

#[test]
fn the_iconic_state_is_saved_with_the_layout_and_drivable_by_ui_set() {
    // `ui.set {"dock": …}` only accepts fields the layout serialises.
    assert_eq!(serde_json::to_value(DockLayout::default()).unwrap()["iconic"], json!(false));
    let back: DockLayout = serde_json::from_value(json!({"iconic": true})).unwrap();
    assert!(back.iconic);
    // Layouts saved before #4059 load expanded; a wrong type keeps the rest of the layout.
    let old: DockLayout = serde_json::from_value(json!({"collapsed": ["color"]})).unwrap();
    assert!(!old.iconic && old.is_collapsed(Group::Color));
    let mut app = app();
    app.ui.dock.iconic = true;
    let saved = json!({"dock": serde_json::to_value(&app.ui.dock).unwrap()});
    app.ui.dock = DockLayout::default();
    crate::dock::apply(&mut app, &saved);
    assert!(app.ui.dock.iconic, "workspaces and the remembered layout keep the strip");
}

#[test]
fn tiny_windows_and_hidden_panels_do_not_panic() {
    for theme in THEMES {
        for (w, ht) in [(40.0, 30.0), (300.0, 80.0), (2000.0, 120.0)] {
            let mut app = app();
            app.ui.dock.iconic = true;
            app.ui.panels.navigator = true;
            let mut h = harness(app, vec2(w, ht), theme);
            if let Some((_, _, r)) = last_icons(&h.ctx).first().copied() {
                click(&mut h, r.center());
                if let Some(f) = last_flyout(&h.ctx) {
                    assert!(f.is_finite() && f.height() >= 0.0, "{f:?}");
                }
            }
            // Every panel closed while a flyout is open.
            let p = &mut h.state_mut().ui.panels;
            (p.color, p.properties, p.navigator, p.history, p.layers, p.brushes, p.character) = (false, false, false, false, false, false, false);
            h.run_steps(3);
            assert_eq!(h.state().ui.dock.flyout, None);
        }
    }
}

#[test]
fn every_panel_has_an_existing_icon() {
    for g in Group::ALL {
        for pro in [true, false] {
            for name in g.tabs(pro) {
                let icon = tab_icon(name);
                assert_ne!(icon, "panels-top-left", "{name} falls back to the generic icon");
                assert!(crate::icons::exists(icon), "{name}: {icon}");
            }
        }
    }
}

#[test]
fn window_menu_and_the_rail_open_a_panel_in_its_flyout() {
    let mut app = app();
    app.ui.dock.iconic = true;
    let mut h = harness(app, vec2(1200.0, 800.0), ThemeKind::ProMedium);
    let ctx = h.ctx.clone();
    // Window › Channels opens the Layers group's flyout on Channels instead of hiding the panel.
    crate::view_cmds::invoke(h.state_mut(), &ctx, "window.panel.channels", &json!({})).unwrap().unwrap();
    h.run_steps(3);
    assert_eq!((h.state().ui.dock.flyout, h.state().ui.dock_tabs.layers), (Some(Group::Layers), 1));
    assert!(h.state().ui.panels.layers && last_flyout(&h.ctx).is_some());
    // Chosen again while its flyout shows, it closes the panel as in the expanded dock.
    crate::view_cmds::invoke(h.state_mut(), &ctx, "window.panel.channels", &json!({})).unwrap().unwrap();
    h.run_steps(3);
    assert!(!h.state().ui.panels.layers);
    crate::view_cmds::invoke(h.state_mut(), &ctx, "window.panel.layers", &json!({})).unwrap().unwrap();
    h.run_steps(3);
    assert_eq!(h.state().ui.dock.flyout, Some(Group::Layers));
    // The icon rail toggles the flyout and never hides the docked group.
    for open in [false, true, false] {
        crate::dock::rail_click(h.state_mut(), Group::Layers, true);
        h.run_steps(3);
        assert_eq!(h.state().ui.dock.flyout.is_some(), open);
        assert!(h.state().ui.panels.layers);
    }
}
