//! Menus on small displays (#138): every row of the longest menus is reachable by keyboard, wheel
//! and the scroll arrows, and the keyboard drives submenus and runs commands.

use super::*;
use crate::PhotocraftApp;
use egui_kittest::{Harness, kittest::Queryable};
use serde_json::json;

/// Window size in pixels and UI scale: the small displays of #138.
const DISPLAYS: [(f32, f32, f32); 4] = [(1280.0, 720.0, 1.0), (1024.0, 600.0, 1.0), (1280.0, 720.0, 1.5), (1024.0, 600.0, 1.5)];
/// The tallest menus.
const LONGEST: [&str; 4] = ["Filter", "Layer", "Image", "Edit"];

/// #3737: on Wayland egui knows the monitor's size but never where the window is (no
/// `inner_rect`). A window taller than the monitor less its taskbar or panel runs under it, so
/// Layer, View and Window (and every other long menu) must end above it and scroll to their last
/// row, at 1× and 1.5×.
#[test]
fn reported_menus_stay_above_the_panel_when_the_window_position_is_unknown() {
    for scale in [1.0, 1.5] {
        for top in ["Layer", "View", "Window", "Filter", "Image", "Edit"] {
            let mut h = harness((1366.0 * scale, 740.0 * scale, scale));
            let v = h.input_mut().viewports.entry(egui::ViewportId::ROOT).or_default();
            v.monitor_size = Some(egui::vec2(1366.0, 768.0));
            v.inner_rect = None;
            v.outer_rect = None;
            v.maximized = Some(false);
            h.run_steps(3);
            let bottom = 768.0 - crate::work_area::TASKBAR;
            let what = format!("{scale}x {top}");
            open(&mut h, top);
            for r in popups(&h) {
                assert!(r.bottom() <= bottom + 0.5, "{what}: popup {r:?} runs under the panel (from {bottom})");
            }
            check_bar_clear(&h, &what);
            let last = rows(&h, 0).len() - 1;
            let in_view = |h: &Harness<'static, PhotocraftApp>| {
                let nav = Nav::current(&h.ctx);
                nav.views[0].expand(0.5).contains_rect(nav.rows[0][last].rect) && nav.views[0].bottom() <= bottom + 0.5
            };
            let view = Nav::current(&h.ctx).views[0];
            h.hover_at(view.center());
            for _ in 0..80 {
                if in_view(&h) {
                    break;
                }
                h.event(egui::Event::MouseWheel {
                    source: egui::MouseWheelSource::Unknown,
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, -60.0),
                    modifiers: egui::Modifiers::NONE,
                    phase: egui::TouchPhase::Move,
                });
                h.run_steps(2);
            }
            assert!(in_view(&h), "{what}: the wheel reaches the last row, above the panel");
        }
    }
}

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app.sync_views();
    app
}

fn harness((w, h, scale): (f32, f32, f32)) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(w / scale, h / scale)).with_pixels_per_point(scale).build_ui_state(
        |ui, app| {
            crate::menus::menu_bar(app, ui);
        },
        app(),
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.ctx.all_styles_mut(|s| s.scroll_animation = egui::style::ScrollAnimation::none());
    h.run_steps(3);
    h
}

fn open(h: &mut Harness<'static, PhotocraftApp>, top: &str) {
    h.get_by_label(top).click();
    h.run_steps(4);
    assert!(Nav::current(&h.ctx).rows.first().is_some_and(|r| !r.is_empty()), "{top} did not open");
}

/// Is row `i` of `level` entirely inside its level's visible (scrolled) area, on screen?
fn visible(h: &Harness<'static, PhotocraftApp>, level: usize, i: usize) -> bool {
    let nav = Nav::current(&h.ctx);
    let (Some(row), Some(view)) = (nav.rows.get(level).and_then(|r| r.get(i)), nav.views.get(level)) else { return false };
    view.expand(0.5).contains_rect(row.rect) && h.ctx.content_rect().expand(0.5).contains_rect(*view)
}

fn rows(h: &Harness<'static, PhotocraftApp>, level: usize) -> Vec<Row> {
    Nav::current(&h.ctx).rows.get(level).cloned().unwrap_or_default()
}

fn key(h: &mut Harness<'static, PhotocraftApp>, k: egui::Key) {
    h.key_press(k);
    h.run_steps(3);
}

#[test]
fn every_row_of_the_longest_menus_is_reachable_with_the_arrow_keys() {
    for display in DISPLAYS {
        for top in LONGEST {
            let mut h = harness(display);
            open(&mut h, top);
            let all = rows(&h, 0);
            let enabled: Vec<usize> = all.iter().enumerate().filter(|(_, r)| r.enabled).map(|(i, _)| i).collect();
            assert!(enabled.len() > 5, "{top}: {} enabled rows", enabled.len());
            let mut seen = Vec::new();
            for _ in 0..enabled.len() {
                key(&mut h, egui::Key::ArrowDown);
                let i = Nav::current(&h.ctx).highlighted(0).expect("a highlighted row");
                assert!(
                    visible(&h, 0, i),
                    "{display:?} {top}: highlighted row {i} ({:?}) is scrolled out of view",
                    rows(&h, 0).get(i).map(|r| r.command.clone())
                );
                seen.push(i);
            }
            seen.sort_unstable();
            assert_eq!(seen, enabled, "{display:?} {top}: ↓ visits every enabled row once");
            // ↓ wraps to the first row (scrolling back up); ↑ wraps to the last.
            key(&mut h, egui::Key::ArrowDown);
            assert_eq!(Nav::current(&h.ctx).highlighted(0), enabled.first().copied());
            assert!(visible(&h, 0, enabled[0]));
            key(&mut h, egui::Key::ArrowUp);
            let last = *enabled.last().unwrap();
            assert_eq!(Nav::current(&h.ctx).highlighted(0), Some(last));
            assert!(visible(&h, 0, last), "{display:?} {top}: the last row is reachable");
        }
    }
}

#[test]
fn small_displays_overflow_and_the_menu_stays_in_the_window() {
    // 1024 × 600 at 150 %: 683 × 400 points, far shorter than the Filter menu.
    let mut h = harness((1024.0, 600.0, 1.5));
    open(&mut h, "Filter");
    let all = rows(&h, 0);
    let last = all.len() - 1;
    assert!(!visible(&h, 0, last), "the Filter menu must overflow at 683 × 400 pt for this test to mean anything");
    let nav = Nav::current(&h.ctx);
    let screen = h.ctx.content_rect();
    assert!(nav.views[0].bottom() <= screen.bottom() && nav.views[0].top() >= screen.top());
    let bar = h.get_by_label("Filter").rect();
    assert!(nav.views[0].top() >= bar.bottom(), "the menu hangs below the menu bar instead of sliding over it: {:?} vs {bar:?}", nav.views[0]);
    // Scroll arrows show when the rows overflow.
    assert!(h.query_by_label("Scroll menu down").is_some() && h.query_by_label("Scroll menu up").is_some());
}

#[test]
fn the_mouse_wheel_scrolls_a_long_menu() {
    for display in DISPLAYS {
        let mut h = harness(display);
        open(&mut h, "Filter");
        let last = rows(&h, 0).len() - 1;
        let view = Nav::current(&h.ctx).views[0];
        h.hover_at(view.center());
        h.run_steps(2);
        for _ in 0..60 {
            if visible(&h, 0, last) {
                break;
            }
            h.event(egui::Event::MouseWheel {
                source: egui::MouseWheelSource::Unknown,
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -60.0),
                modifiers: egui::Modifiers::NONE,
                phase: egui::TouchPhase::Move,
            });
            h.run_steps(2);
        }
        assert!(visible(&h, 0, last), "{display:?}: the wheel reaches the last Filter row");
        assert!(rows(&h, 0).len() == last + 1, "the menu stays open while scrolling");
    }
}

#[test]
fn resting_on_the_scroll_arrows_scrolls() {
    let mut h = harness((1024.0, 600.0, 1.5));
    open(&mut h, "Filter");
    let last = rows(&h, 0).len() - 1;
    assert!(visible(&h, 0, 0) && !visible(&h, 0, last));
    let view = Nav::current(&h.ctx).views[0];
    h.hover_at(egui::pos2(view.center().x, view.bottom() + ARROW / 2.0));
    for _ in 0..240 {
        h.run_steps(1);
        if visible(&h, 0, last) {
            break;
        }
    }
    assert!(visible(&h, 0, last), "resting on ▼ scrolls to the end");
    assert!(!visible(&h, 0, 0));
    let view = Nav::current(&h.ctx).views[0];
    h.hover_at(egui::pos2(view.center().x, view.top() - ARROW / 2.0));
    for _ in 0..240 {
        h.run_steps(1);
        if visible(&h, 0, 0) {
            break;
        }
    }
    assert!(visible(&h, 0, 0), "resting on ▲ scrolls back to the top");
}

#[test]
fn arrows_open_and_close_submenus_and_enter_runs_a_command() {
    let mut h = harness((1024.0, 600.0, 1.5));
    open(&mut h, "Filter");
    // Walk down to the first submenu (Blur, Distort, …) and open it with →.
    for _ in 0..rows(&h, 0).len() {
        key(&mut h, egui::Key::ArrowDown);
        let nav = Nav::current(&h.ctx);
        if nav.highlighted(0).and_then(|i| nav.rows[0].get(i)).is_some_and(|r| r.command.is_none()) {
            break;
        }
    }
    key(&mut h, egui::Key::ArrowRight);
    h.run_steps(2);
    let nav = Nav::current(&h.ctx);
    assert_eq!(nav.level, 1, "→ moves into the submenu");
    assert!(nav.rows.get(1).is_some_and(|r| !r.is_empty()), "the submenu is open");
    let first = nav.rows[1].iter().position(|r| r.enabled);
    assert_eq!(nav.highlighted(1), first, "its first enabled row is highlighted");
    assert!(visible(&h, 1, first.unwrap()));
    key(&mut h, egui::Key::ArrowDown);
    assert_eq!(Nav::current(&h.ctx).level, 1);
    // ← closes it again.
    key(&mut h, egui::Key::ArrowLeft);
    h.run_steps(2);
    let nav = Nav::current(&h.ctx);
    assert_eq!((nav.level, nav.rows.len()), (0, 1), "← closes the submenu");
    // ↩ runs the highlighted command and closes the menu: Select › All.
    key(&mut h, egui::Key::Escape);
    open(&mut h, "Select");
    for _ in 0..rows(&h, 0).len() {
        let nav = Nav::current(&h.ctx);
        if nav.highlighted(0).and_then(|i| nav.rows[0].get(i)).is_some_and(|r| r.command.as_deref() == Some("select.all")) {
            break;
        }
        key(&mut h, egui::Key::ArrowDown);
    }
    assert!(h.state().session.active().unwrap().doc.selection.is_none());
    key(&mut h, egui::Key::Enter);
    assert!(h.state().session.active().unwrap().doc.selection.is_some(), "↩ ran Select › All");
    assert!(!is_open(&h.ctx), "and closed the menu");
}

#[test]
fn left_and_right_switch_menus_and_shortcuts_wait_while_one_is_open() {
    let mut h = harness((1280.0, 720.0, 1.0));
    assert!(!is_open(&h.ctx));
    open(&mut h, "File");
    assert!(is_open(&h.ctx), "shortcuts leave the keys to an open menu");
    key(&mut h, egui::Key::ArrowRight);
    h.run_steps(2);
    assert!(h.query_by_label_contains("Open…").is_none(), "→ leaves File");
    assert!(h.query_by_label_contains("Undo").is_some(), "→ opens Edit");
    key(&mut h, egui::Key::ArrowLeft);
    h.run_steps(2);
    assert!(h.query_by_label_contains("Open…").is_some(), "← back to File");
    key(&mut h, egui::Key::Escape);
    h.run_steps(2);
    assert!(h.query_by_label_contains("Open…").is_none(), "Esc closes the menu");
    assert!(!is_open(&h.ctx), "shortcuts work again");
}

/// In Help › Search, ← / → move the field's cursor instead of switching to Window / File, and
/// Space types a space instead of running the lit row.
#[test]
fn help_search_keeps_the_arrows_and_space_that_edit_it() {
    let mut h = harness((1280.0, 720.0, 1.0));
    open(&mut h, "Help");
    let field = egui::Id::unique("help-menu-search").with("field");
    assert_eq!(h.ctx.memory(|m| m.focused()), Some(field), "the search field has the keyboard");
    let text = |h: &Harness<'static, PhotocraftApp>| h.ctx.data(|d| d.get_temp::<String>(egui::Id::unique("help-menu-search"))).unwrap_or_default();
    let typing = |h: &mut Harness<'static, PhotocraftApp>, s: &str| {
        h.event(egui::Event::Text(s.to_string()));
        h.run_steps(3);
    };
    typing(&mut h, "levls");
    key(&mut h, egui::Key::ArrowLeft);
    assert!(h.query_by_label_contains("About PhotoCraft").is_some(), "← keeps Help open");
    key(&mut h, egui::Key::ArrowLeft);
    typing(&mut h, "e");
    assert_eq!(text(&h), "levels", "← moved the cursor");
    // The cursor sits at `leve|ls`: two → take it to the end.
    key(&mut h, egui::Key::ArrowRight);
    assert!(h.query_by_label_contains("About PhotoCraft").is_some(), "→ keeps Help open");
    key(&mut h, egui::Key::ArrowRight);
    // The pointer resting on a result lights it; a space typed then is text, not a click.
    // "levels" finds both Image › Adjustments › Levels… and the Levels adjustment layer.
    let levels = h.get_all_by_label_contains("› Levels").next().expect("a Levels result").rect().center();
    h.hover_at(levels);
    h.run_steps(3);
    assert!(Nav::current(&h.ctx).highlighted(0).is_some(), "a result is lit");
    h.event(egui::Event::Key { key: egui::Key::Space, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE });
    h.event(egui::Event::Text(" ".into()));
    h.event(egui::Event::Key { key: egui::Key::Space, physical_key: None, pressed: false, repeat: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(3);
    assert!(is_open(&h.ctx) && h.query_by_label_contains("About PhotoCraft").is_some(), "Space keeps Help open");
    assert!(h.state().ui.dialogs.is_empty(), "Space ran no command: {:?}", h.state().ui.dialogs);
    assert_eq!(text(&h), "levels ");
}

/// A level's visible height (its view) and the full height of its rows: equal means no scrolling.
fn fits(h: &Harness<'static, PhotocraftApp>, level: usize) -> (f32, f32) {
    let nav = Nav::current(&h.ctx);
    (nav.views.get(level).map_or(0.0, |v| v.height()), nav.contents.get(level).copied().unwrap_or(0.0))
}

/// The on-screen rect of the popup holding `level` (the smallest area around its view).
fn popup(h: &Harness<'static, PhotocraftApp>, level: usize) -> egui::Rect {
    let Some(view) = Nav::current(&h.ctx).views.get(level).copied() else { return egui::Rect::NOTHING };
    h.ctx
        .memory(|m| m.areas().visible_layer_ids())
        .into_iter()
        .filter_map(|layer| h.ctx.memory(|m| m.area_rect(layer.id)))
        .filter(|r| r.contains(view.center()) && r.width() < 600.0)
        .min_by(|a, b| a.area().total_cmp(&b.area()))
        .unwrap_or(egui::Rect::NOTHING)
}

/// The popup is as tall as its rows plus the menu frame: no blank space, no clipped rows.
fn hugs_rows(h: &Harness<'static, PhotocraftApp>, level: usize) -> Result<(), String> {
    let (_, content) = fits(h, level);
    let frame = h.ctx.global_style().spacing.menu_margin.sum().y + 2.0;
    let area = popup(h, level);
    ((area.height() - content - frame).abs() < 2.0).then_some(()).ok_or(format!("popup {} pt tall for {content} pt of rows + {frame} frame", area.height()))
}

/// Open the `n`th enabled submenu of the open top-level menu with the keyboard.
fn open_submenu(h: &mut Harness<'static, PhotocraftApp>, n: usize) -> bool {
    let subs: Vec<usize> = rows(h, 0).iter().enumerate().filter(|(_, r)| r.enabled && r.command.is_none()).map(|(i, _)| i).collect();
    let Some(&target) = subs.get(n) else { return false };
    for _ in 0..rows(h, 0).len() {
        if Nav::current(&h.ctx).highlighted(0) == Some(target) {
            break;
        }
        key(h, egui::Key::ArrowDown);
    }
    key(h, egui::Key::ArrowRight);
    h.run_steps(3);
    Nav::current(&h.ctx).rows.get(1).is_some_and(|r| !r.is_empty())
}

#[test]
fn menus_are_as_tall_as_their_rows_on_a_tall_window() {
    // #235: every menu opened at egui's 400 pt default area height and scrolled, even at 1440 × 900.
    let mut checked = 0;
    for top in crate::menus::TOP_MENUS {
        let mut h = harness((1440.0, 900.0, 1.0));
        open(&mut h, top);
        let (view, content) = fits(&h, 0);
        if content > h.ctx.content_rect().height() - 60.0 {
            continue; // Taller than the window itself: scrolling is right (see the short-window tests).
        }
        checked += 1;
        assert!((view - content).abs() < 1.0, "{top}: the menu is {view} pt tall for {content} pt of rows");
        if let Err(e) = hugs_rows(&h, 0) {
            panic!("{top}: {e}");
        }
        assert!(h.query_by_label("Scroll menu down").is_none(), "{top}: no scroll arrows when the rows fit");
        assert!((0..rows(&h, 0).len()).all(|i| visible(&h, 0, i)), "{top}: every row is on screen without scrolling");
    }
    assert!(checked >= 6, "most menus fit a 900 px window: {checked}");
    // The File menu is taller than the old 400 pt cap and fits at 1440 × 900.
    let mut h = harness((1440.0, 900.0, 1.0));
    open(&mut h, "File");
    let (view, content) = fits(&h, 0);
    assert!(content > 420.0 && (view - content).abs() < 1.0, "File: {view} pt view for {content} pt of rows");
    assert_eq!(hugs_rows(&h, 0), Ok(()));
}

#[test]
fn submenus_are_as_tall_as_their_rows_on_a_tall_window() {
    let mut checked = 0;
    for top in ["Filter", "Image", "Layer"] {
        for n in 0..3 {
            let mut h = harness((1440.0, 900.0, 1.0));
            open(&mut h, top);
            if !open_submenu(&mut h, n) {
                continue;
            }
            checked += 1;
            let (view, content) = fits(&h, 1);
            assert!(content > 0.0 && (view - content).abs() < 1.0, "{top} submenu {n}: {view} pt view for {content} pt of rows");
            if let Err(e) = hugs_rows(&h, 1) {
                panic!("{top} submenu {n}: {e}");
            }
        }
    }
    assert!(checked >= 6, "opened {checked} submenus");
}

#[test]
fn a_short_window_still_scrolls_the_file_menu_to_its_last_row() {
    let mut h = harness((1000.0, 500.0, 1.0));
    open(&mut h, "File");
    let (view, content) = fits(&h, 0);
    assert!(view < content - 1.0, "File must overflow a 500 px window: {view} vs {content}");
    let nav = Nav::current(&h.ctx);
    assert!(nav.views[0].bottom() <= h.ctx.content_rect().bottom(), "the menu stays in the window");
    assert!(h.query_by_label("Scroll menu down").is_some());
    let last = rows(&h, 0).len() - 1;
    assert!(!visible(&h, 0, last));
    h.hover_at(nav.views[0].center());
    h.run_steps(2);
    for _ in 0..60 {
        if visible(&h, 0, last) {
            break;
        }
        h.event(egui::Event::MouseWheel {
            source: egui::MouseWheelSource::Unknown,
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -60.0),
            modifiers: egui::Modifiers::NONE,
            phase: egui::TouchPhase::Move,
        });
        h.run_steps(2);
    }
    assert!(visible(&h, 0, last), "the wheel reaches File's last row");
}

/// Every popup on screen (menus and submenus).
fn popups(h: &Harness<'static, PhotocraftApp>) -> Vec<egui::Rect> {
    h.ctx
        .memory(|m| m.areas().visible_layer_ids())
        .into_iter()
        .filter(|l| l.order == egui::Order::Foreground)
        .filter_map(|layer| h.ctx.memory(|m| m.area_rect(layer.id)))
        .collect()
}

/// No popup covers the menu bar or runs off the window, and every menu title stays visible.
fn check_bar_clear(h: &Harness<'static, PhotocraftApp>, what: &str) {
    let bar = Nav::current(&h.ctx).bar_bottom.expect("the menu bar's bottom");
    let screen = h.ctx.content_rect();
    for r in popups(h) {
        assert!(r.top() >= bar - 0.5, "{what}: popup {r:?} covers the menu bar (bottom {bar})");
        assert!(r.bottom() <= screen.bottom() + 0.5, "{what}: popup {r:?} runs off the window {screen:?}");
    }
    for title in crate::menus::TOP_MENUS {
        // The title is the topmost node of that name (a submenu row can share it).
        let t = h.query_all_by_label(title).map(|n| n.rect()).min_by(|a, b| a.top().total_cmp(&b.top())).expect("menu title");
        assert!(popups(h).iter().all(|r| !r.intersects(t.shrink(1.0))), "{what}: {title} is covered");
    }
}

#[test]
fn no_menu_or_submenu_covers_the_menu_bar() {
    // #319: on a 1280 × 720 display (the reporter's, at 1× and 2×, less the title bar) tall menus,
    // then tall submenus (Image › Adjustments), slid up over the menu bar and hid its titles.
    let displays = [(1280.0, 703.0, 1.0), (2560.0, 1406.0, 2.0), (1366.0, 768.0, 1.0)];
    let mut checked = 0;
    for display in displays {
        for top in LONGEST {
            let mut h = harness(display);
            open(&mut h, top);
            check_bar_clear(&h, &format!("{display:?} {top}"));
            // Hover each submenu row in view, top to bottom: rows low in the menu open their
            // submenus upward.
            let view = Nav::current(&h.ctx).views.first().copied().unwrap_or(egui::Rect::NOTHING);
            let subs: Vec<egui::Rect> = rows(&h, 0).iter().filter(|r| r.enabled && r.command.is_none() && view.contains_rect(r.rect)).map(|r| r.rect).collect();
            for (i, row) in subs.iter().enumerate() {
                h.hover_at(row.center() + egui::vec2(-20.0, 0.0));
                h.run_steps(6);
                if Nav::current(&h.ctx).rows.get(1).is_none_or(Vec::is_empty) {
                    continue;
                }
                checked += 1;
                check_bar_clear(&h, &format!("{display:?} {top} submenu {i}"));
            }
        }
    }
    assert!(checked > 30, "checked {checked} submenus");
}

#[test]
fn level_room_keeps_every_level_below_the_bar() {
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 703.0));
    let (bar, frame) = (Some(30.0), 14.0);
    let below = 703.0 - EDGE - (30.0 + EDGE) - frame;
    assert_eq!(level_room(screen, bar, 1, None, frame), below);
    // A submenu from a row near the top opens downward with the room under it…
    let row = egui::Rect::from_min_size(egui::pos2(100.0, 80.0), egui::vec2(200.0, 30.0));
    assert_eq!(level_room(screen, bar, 2, Some(row), frame), 703.0 - EDGE - 80.0 - frame);
    // …and from a row near the bottom, upward to the bar.
    let low = egui::Rect::from_min_size(egui::pos2(100.0, 600.0), egui::vec2(200.0, 30.0));
    assert_eq!(level_room(screen, bar, 2, Some(low), frame), 630.0 - (30.0 + EDGE) - frame);
    // Never more than the space under the bar, never less than the scroll arrows need.
    let odd = egui::Rect::from_min_size(egui::pos2(100.0, -50.0), egui::vec2(200.0, 30.0));
    assert!(level_room(screen, bar, 2, Some(odd), frame) <= below);
    assert_eq!(level_room(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(100.0, 40.0)), bar, 1, None, frame), 4.0 * ARROW);
    assert_eq!(level_room(screen, Some(f32::NAN), 1, None, frame), level_room(screen, None, 1, None, frame));
    assert_eq!(level_room(egui::Rect::NOTHING, bar, 2, Some(row), frame), 4.0 * ARROW);
    // A window running under the taskbar: a top-level menu leaves a scrolling submenu's room above
    // it, so the submenu of its last row (which egui opens downward) stays above the taskbar too.
    let visible = egui::Rect::from_min_max(screen.min, egui::pos2(screen.right(), 600.0));
    let top = level_room_in(screen, visible, bar, 1, None, None, frame);
    assert_eq!(top, 600.0 - EDGE - (30.0 + EDGE) - (4.0 * ARROW + frame) - frame);
    let last_row = egui::Rect::from_min_size(egui::pos2(100.0, 30.0 + EDGE + top - 24.0), egui::vec2(200.0, 24.0));
    // A 200 pt submenu fits the window downward, so egui opens it that way: it scrolls in what is left.
    let room = level_room_in(screen, visible, bar, 2, Some(last_row), Some(200.0), frame);
    assert!(room >= 4.0 * ARROW && last_row.top() - (frame - 2.0) / 2.0 + room + frame <= 600.0 - EDGE + 0.5, "room {room}");
}

/// #315: a 1366 × 768 pt display with a Windows taskbar. `window` is the window's content size and
/// `top` where its content starts on the desktop (below the title bar), in points; the taskbar
/// covers the monitor's bottom [`crate::work_area::TASKBAR`] points. `scale` is the UI scale.
fn taskbar_harness(window: (f32, f32), top: f32, scale: f32) -> Harness<'static, PhotocraftApp> {
    let mut h = harness((window.0 * scale, window.1 * scale, scale));
    let v = h.input_mut().viewports.entry(egui::ViewportId::ROOT).or_default();
    v.monitor_size = Some(egui::vec2(1366.0, 768.0));
    v.inner_rect = Some(egui::Rect::from_min_size(egui::pos2(0.0, top), egui::vec2(window.0, window.1)));
    v.maximized = Some(false);
    h.run_steps(3);
    h
}

/// The visible bottom of a taskbar harness: the monitor's bottom less the taskbar, in the window.
fn taskbar_bottom(h: &Harness<'static, PhotocraftApp>) -> f32 {
    crate::work_area::visible_rect(&h.ctx).bottom()
}

#[test]
fn menus_stay_above_the_taskbar_and_scroll_to_their_last_row() {
    // The default 1440 × 900 window on a 1366 × 768 display runs past the screen; a 1366 × 737
    // window below a 31 pt title bar reaches under the taskbar. At 1× and 2× (a 2732 × 1536 px
    // display at 200%).
    let mut scrolled = 0;
    for (window, top, scale) in [((1440.0, 900.0), 0.0, 1.0), ((1366.0, 737.0), 31.0, 1.0), ((1366.0, 737.0), 31.0, 2.0)] {
        for menu in LONGEST {
            let mut h = taskbar_harness(window, top, scale);
            let bottom = taskbar_bottom(&h);
            let what = format!("{window:?}@{scale}x {menu}");
            assert!((bottom - (768.0 - crate::work_area::TASKBAR - top)).abs() < 1.0, "{what}: visible bottom {bottom}");
            open(&mut h, menu);
            for r in popups(&h) {
                assert!(r.bottom() <= bottom + 0.5, "{what}: popup {r:?} runs under the taskbar (from {bottom})");
            }
            let last = rows(&h, 0).len() - 1;
            let in_view = |h: &Harness<'static, PhotocraftApp>, i: usize| {
                let nav = Nav::current(&h.ctx);
                nav.views[0].expand(0.5).contains_rect(nav.rows[0][i].rect) && nav.views[0].bottom() <= bottom + 0.5
            };
            // A menu that fits above the taskbar needs no scrolling.
            let Some(down) = h.query_by_label("Scroll menu down").map(|n| n.rect()) else {
                assert!(in_view(&h, last), "{what}: fits, so its last row is in view");
                continue;
            };
            scrolled += 1;
            // The ▼ arrow is above the taskbar, and resting on it reaches the last row.
            assert!(down.bottom() <= bottom, "{what}: ▼ at {down:?}");
            assert!(!in_view(&h, last));
            h.hover_at(down.center());
            for _ in 0..400 {
                h.run_steps(1);
                if in_view(&h, last) {
                    break;
                }
            }
            assert!(in_view(&h, last), "{what}: ▼ scrolls to the last row, above the taskbar");
            // And the wheel scrolls back up.
            let view = Nav::current(&h.ctx).views[0];
            h.hover_at(view.center());
            for _ in 0..60 {
                h.event(egui::Event::MouseWheel {
                    source: egui::MouseWheelSource::Unknown,
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, 120.0),
                    modifiers: egui::Modifiers::NONE,
                    phase: egui::TouchPhase::Move,
                });
                h.run_steps(1);
                if in_view(&h, 0) {
                    break;
                }
            }
            assert!(in_view(&h, 0), "{what}: the wheel scrolls back to the first row");
        }
    }
    assert!(scrolled >= 6, "the long menus overflow above the taskbar: {scrolled}");
}

#[test]
fn submenus_stay_above_the_taskbar() {
    for menu in ["Image", "Layer", "Filter"] {
        let mut h = taskbar_harness((1440.0, 900.0), 0.0, 1.0);
        let bottom = taskbar_bottom(&h);
        open(&mut h, menu);
        let view = Nav::current(&h.ctx).views.first().copied().unwrap_or(egui::Rect::NOTHING);
        let subs: Vec<egui::Rect> = rows(&h, 0).iter().filter(|r| r.enabled && r.command.is_none() && view.contains_rect(r.rect)).map(|r| r.rect).collect();
        assert!(!subs.is_empty());
        for (i, row) in subs.iter().enumerate() {
            h.hover_at(row.center() + egui::vec2(-20.0, 0.0));
            h.run_steps(6);
            for r in popups(&h) {
                assert!(r.bottom() <= bottom + 0.5, "{menu} submenu {i}: popup {r:?} runs under the taskbar (from {bottom})");
            }
            check_bar_clear(&h, &format!("{menu} submenu {i}"));
        }
    }
}

/// #3737 on X11 (Linux Mint, 1920 × 1080 px): the window manager maximized the window to the whole
/// monitor without keeping it clear of the bottom panel, so the window's position is known but the
/// panel covers its bottom. At 1× and at a 150% UI scale (1280 × 720 pt).
fn maximized_under_a_panel(scale: f32) -> Harness<'static, PhotocraftApp> {
    let size = egui::vec2(1920.0, 1080.0) / scale;
    let mut h = harness((1920.0, 1080.0, scale));
    let v = h.input_mut().viewports.entry(egui::ViewportId::ROOT).or_default();
    v.monitor_size = Some(size);
    v.inner_rect = Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), size));
    v.maximized = Some(true);
    h.run_steps(3);
    h
}

#[test]
fn menus_of_a_maximized_window_stay_above_a_panel_it_runs_under() {
    let mut scrolled = 0;
    for scale in [1.0_f32, 1.5] {
        for menu in LONGEST.iter().copied().chain(["View", "Window"]) {
            let mut h = maximized_under_a_panel(scale);
            let bottom = taskbar_bottom(&h);
            let what = format!("{scale}x {menu}");
            assert!((bottom - (1080.0 / scale - crate::work_area::TASKBAR)).abs() < 1.0, "{what}: visible bottom {bottom}");
            open(&mut h, menu);
            for r in popups(&h) {
                assert!(r.bottom() <= bottom + 0.5, "{what}: popup {r:?} runs under the panel (from {bottom})");
            }
            let last = rows(&h, 0).len() - 1;
            let in_view = |h: &Harness<'static, PhotocraftApp>| {
                let nav = Nav::current(&h.ctx);
                nav.views[0].expand(0.5).contains_rect(nav.rows[0][last].rect) && nav.views[0].bottom() <= bottom + 0.5
            };
            let Some(down) = h.query_by_label("Scroll menu down").map(|n| n.rect()) else {
                assert!(in_view(&h), "{what}: fits, so its last row is in view");
                continue;
            };
            scrolled += 1;
            assert!(down.bottom() <= bottom, "{what}: ▼ at {down:?} is under the panel");
            h.hover_at(down.center());
            for _ in 0..400 {
                h.run_steps(1);
                if in_view(&h) {
                    break;
                }
            }
            assert!(in_view(&h), "{what}: ▼ scrolls to the last row, above the panel");
        }
    }
    assert!(scrolled >= 3, "at 150% the long menus overflow above the panel: {scrolled}");
}

#[test]
fn a_maximized_window_or_an_unknown_monitor_uses_the_whole_window() {
    // Maximized inside the work area (the window manager kept it clear of the taskbar).
    let mut h = taskbar_harness((1366.0, 720.0), 0.0, 1.0);
    h.input_mut().viewports.entry(egui::ViewportId::ROOT).or_default().maximized = Some(true);
    h.run_steps(2);
    assert_eq!(crate::work_area::visible_rect(&h.ctx), h.ctx.content_rect());
    let h = harness((1366.0, 768.0, 1.0));
    assert_eq!(crate::work_area::visible_rect(&h.ctx), h.ctx.content_rect());
}

/// One gesture: press a menu title (it opens on the press, not the release), drag down to an
/// item and release on it to run it. A plain click still opens the menu and leaves it open.
#[test]
fn press_drag_release_runs_a_menu_item() {
    use egui::{Modifiers, PointerButton};
    let mut h = harness((1280.0, 720.0, 1.0));
    let rulers = h.state().ui.extras.rulers;
    let title = h.get_by_label("View").rect().center();
    h.event(egui::Event::PointerMoved(title));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: title, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert!(Nav::current(&h.ctx).rows.first().is_some_and(|r| !r.is_empty()), "the menu opens on the press");
    let item = h.get_by_label_contains("Rulers").rect().center();
    for k in 1..=8 {
        h.event(egui::Event::PointerMoved(title + (item - title) * (k as f32 / 8.0)));
        h.run_steps(1);
    }
    h.event(egui::Event::PointerButton { pos: item, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    assert_eq!(h.state().ui.extras.rulers, !rulers, "releasing on Rulers ran it");
    assert!(Nav::current(&h.ctx).rows.first().is_none_or(|r| r.is_empty()), "and closed the menu");
    // A click (press and release on the title) opens the menu and leaves it open.
    h.get_by_label("View").click();
    h.run_steps(4);
    assert!(Nav::current(&h.ctx).rows.first().is_some_and(|r| !r.is_empty()), "a click leaves it open");
}

/// Windows and Linux menus too (#775): one press-drag-release gesture can move across titles and
/// into a submenu. Press File, drag over to View, down to Proof Setup, right into its submenu and
/// release on Working CMYK.
#[test]
fn press_drag_release_reaches_other_menus_and_submenus() {
    use egui::{Modifiers, PointerButton};
    let mut h = harness((1280.0, 720.0, 1.0));
    let mut at = h.get_by_label("File").rect().center();
    h.event(egui::Event::PointerMoved(at));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(2);
    let mut drag_to = |h: &mut Harness<'static, PhotocraftApp>, to: egui::Pos2| {
        for k in 1..=8 {
            h.event(egui::Event::PointerMoved(at + (to - at) * (k as f32 / 8.0)));
            h.run_steps(1);
        }
        h.run_steps(2);
        at = to;
    };
    let view = h.get_by_label("View").rect().center();
    drag_to(&mut h, view);
    assert!(h.query_by_label_contains("Proof Setup").is_some(), "dragging over View switched to its menu");
    let proof = h.get_by_label_contains("Proof Setup").rect();
    drag_to(&mut h, egui::pos2(proof.left() + 30.0, proof.center().y));
    drag_to(&mut h, egui::pos2(proof.right() - 6.0, proof.center().y));
    let item = h.get_by_label_contains("Working CMYK").rect();
    drag_to(&mut h, egui::pos2(item.left() + 30.0, proof.center().y));
    drag_to(&mut h, item.center());
    h.event(egui::Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    let proofing = crate::menus::menu_items(h.state()).into_iter().find(|i| i.id == "view.proofColors").and_then(|i| i.checked);
    assert_eq!(proofing, Some(true), "releasing on Working CMYK ran it");
    assert!(Nav::current(&h.ctx).rows.first().is_none_or(|r| r.is_empty()), "and closed the menus");
}

/// Press-and-hold a menu title and drag through the items: the highlight follows the
/// pointer the whole way, down and back up, without lagging or dropping (#8887).
#[test]
fn press_drag_highlight_follows_the_pointer() {
    use egui::{Modifiers, PointerButton};
    let mut h = harness((1280.0, 720.0, 1.0));
    let title = h.get_by_label("View").rect().center();
    h.event(egui::Event::PointerMoved(title));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: title, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(2);
    let rows = Nav::current(&h.ctx).rows.first().cloned().unwrap_or_default();
    assert!(rows.len() > 4, "the View menu has rows to drag through");
    // Drag down through the first rows, one at a time, still holding the button.
    for i in 1..=4 {
        let row = rows.get(i).expect("row exists");
        h.event(egui::Event::PointerMoved(row.rect.center()));
        h.run_steps(1);
        assert_eq!(Nav::current(&h.ctx).highlighted(0), Some(i), "holding and dragging onto row {i} highlights it");
    }
    // Back up the way a user overshooting and correcting does.
    for i in (1..=3).rev() {
        let row = rows.get(i).unwrap();
        h.event(egui::Event::PointerMoved(row.rect.center()));
        h.run_steps(1);
        assert_eq!(Nav::current(&h.ctx).highlighted(0), Some(i), "dragging back up to row {i} highlights it");
    }
    h.event(egui::Event::PointerButton { pos: rows.get(1).unwrap().rect.center(), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

/// A menu bar whose frames are `dt` seconds apart, as with a real pointer sending moves.
fn harness_dt(dt: f32) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1280.0, 720.0)).with_step_dt(dt).build_ui_state(
        |ui, app| {
            crate::menus::menu_bar(app, ui);
        },
        app(),
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.ctx.all_styles_mut(|s| s.scroll_animation = egui::style::ScrollAnimation::none());
    h.run_steps(3);
    h
}

/// Which of Image › Mode and Image › Adjustments has its submenu open.
fn image_submenu(h: &Harness<'static, PhotocraftApp>) -> &'static str {
    match (h.query_by_label_contains("16 Bits/Channel").is_some(), h.query_by_label_contains("Brightness/Contrast").is_some()) {
        (true, false) => "Mode",
        (false, true) => "Adjustments",
        (false, false) => "none",
        (true, true) => "both",
    }
}

/// Open Image › Mode with the pointer resting on it, at 120 frames a second; returns where the
/// pointer is, the Adjustments row below it, and Mode's 16 Bits/Channel row.
fn rest_on_mode(h: &mut Harness<'static, PhotocraftApp>) -> (egui::Pos2, egui::Rect, egui::Rect) {
    open(h, "Image");
    let mode = h.get_by_label_contains("Mode").rect();
    let from = egui::pos2(mode.right() - 40.0, mode.center().y);
    for k in 0..=12 {
        h.event(egui::Event::PointerMoved(from - egui::vec2(30.0 * (1.0 - k as f32 / 12.0), 0.0)));
        h.run_steps(1);
    }
    h.run_steps(30);
    assert_eq!(image_submenu(h), "Mode", "resting on Mode opens it");
    (from, h.get_by_label_contains("Adjustments").rect(), h.get_by_label_contains("16 Bits/Channel").rect())
}

/// Move the pointer along `path` (0…1) in `frames` frames; `pause` holds it still for that many
/// frames at that fraction of the way.
fn glide(h: &mut Harness<'static, PhotocraftApp>, frames: usize, pause: Option<(f32, usize)>, mut path: impl FnMut(f32) -> egui::Pos2) {
    let mut paused = false;
    for k in 0..=frames {
        let t = k as f32 / frames as f32;
        h.event(egui::Event::PointerMoved(path(t)));
        h.run_steps(1);
        if let Some((_, n)) = pause.filter(|(at, _)| !paused && t >= *at) {
            paused = true;
            h.run_steps(n);
        }
    }
}

/// A person's path from `a` to `b`: it bows, down first and then across, by `bow`.
fn bowed(a: egui::Pos2, b: egui::Pos2, bow: f32) -> impl Fn(f32) -> egui::Pos2 {
    let c = egui::pos2(a.x + (b.x - a.x) * (0.5 - bow), a.y + (b.y - a.y) * (0.5 + bow));
    move |t| {
        let u = 1.0 - t;
        egui::pos2(u * u * a.x + 2.0 * u * t * c.x + t * t * b.x, u * u * a.y + 2.0 * u * t * c.y + t * t * b.y)
    }
}

/// #1218: heading from Image › Mode for a row low in its submenu, the pointer crosses the
/// Adjustments row. On a diagonal, a bowed path or with a short pause on the way, Mode's submenu
/// stays open and Adjustments' doesn't open; arriving, Mode is the highlighted row.
#[test]
fn a_submenu_stays_open_while_the_pointer_heads_for_it() {
    struct Path {
        name: &'static str,
        /// Frames at 120 a second.
        frames: usize,
        /// Hold still this many frames at this fraction of the way.
        pause: Option<(f32, usize)>,
        bow: f32,
    }
    let paths = [
        Path { name: "straight", frames: 36, pause: None, bow: 0.0 },
        Path { name: "hesitate", frames: 36, pause: Some((0.3, 18)), bow: 0.0 },
        Path { name: "bowed", frames: 42, pause: None, bow: 0.2 },
        Path { name: "slow and bowed", frames: 96, pause: None, bow: 0.2 },
    ];
    let mut wrong = Vec::new();
    for Path { name, frames, pause, bow } in paths {
        let mut h = harness_dt(1.0 / 120.0);
        let (from, adjustments, target) = rest_on_mode(&mut h);
        let to = egui::pos2(target.left() + 40.0, target.center().y);
        let path = bowed(from, to, bow);
        let mode_row = rows(&h, 0).iter().position(|r| r.command.is_none() && r.rect.contains(from)).expect("the Mode row");
        let mut crossed = false;
        glide(&mut h, frames, pause, |t| {
            let p = path(t);
            crossed |= adjustments.contains(p);
            p
        });
        assert!(crossed, "{name}: the path crosses the Adjustments row");
        // Once Mode's submenu closes on the way, the pointer arrives over Adjustments' instead.
        let open = image_submenu(&h);
        if open != "Mode" {
            wrong.push(format!("{name}: arrived with {open} open, not Mode"));
        }
        let row = Nav::current(&h.ctx).highlighted(0);
        if row != Some(mode_row) {
            wrong.push(format!("{name}: row {row:?} is highlighted, not Mode"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// Resting on the row crossed, or moving away from the submenu onto it, still opens its submenu,
/// and the highlight moves to it (only it is lit).
#[test]
fn resting_on_another_row_or_moving_away_switches_submenus() {
    let only_adjustments_lit = |h: &Harness<'static, PhotocraftApp>, adjustments: egui::Rect, what: &str| {
        let row = rows(h, 0).iter().position(|r| r.rect.contains(adjustments.center()));
        assert_eq!(Nav::current(&h.ctx).highlighted(0), row, "{what}: Adjustments is the highlighted row, not Mode");
    };
    let mut h = harness_dt(1.0 / 120.0);
    let (from, adjustments, _) = rest_on_mode(&mut h);
    // Head for the submenu, but stop on Adjustments on the way.
    let stop = egui::pos2(from.x + 15.0, adjustments.center().y);
    glide(&mut h, 12, None, |t| from + (stop - from) * t);
    assert_eq!(image_submenu(&h), "Mode", "just arrived on Adjustments, still heading for Mode's submenu");
    h.run_steps((super::AIM_REST * 120.0) as usize + 12);
    assert_eq!(image_submenu(&h), "Adjustments", "resting on Adjustments opens it");
    only_adjustments_lit(&h, adjustments, "resting");

    // Moving away from Mode's submenu switches (as soon as egui does): straight down, or down and
    // to the left.
    for dx in [0.0, -60.0] {
        let mut h = harness_dt(1.0 / 120.0);
        let (from, adjustments, _) = rest_on_mode(&mut h);
        let to = egui::pos2(from.x + dx, adjustments.center().y);
        glide(&mut h, 10, None, |t| from + (to - from) * t);
        h.run_steps(30);
        assert_eq!(image_submenu(&h), "Adjustments", "moving away ({dx}) from Mode's submenu opens Adjustments'");
        only_adjustments_lit(&h, adjustments, "moving away");
    }
}

#[test]
fn heading_is_the_triangle_to_the_submenus_near_edge() {
    let sub = egui::Rect::from_min_max(egui::pos2(200.0, 100.0), egui::pos2(400.0, 400.0));
    let from = egui::pos2(150.0, 110.0);
    assert!(heading(from, from, sub));
    assert!(heading(from, egui::pos2(170.0, 160.0), sub), "down and right, toward the submenu");
    assert!(heading(from, egui::pos2(190.0, 105.0), sub), "straight across");
    assert!(!heading(from, egui::pos2(150.0, 160.0), sub), "straight down");
    assert!(!heading(from, egui::pos2(120.0, 140.0), sub), "away to the left");
    assert!(!heading(from, egui::pos2(170.0, 60.0), sub), "up past the submenu's top");
    // A submenu that opened to the left.
    let left = egui::Rect::from_min_max(egui::pos2(0.0, 100.0), egui::pos2(100.0, 400.0));
    assert!(heading(egui::pos2(150.0, 110.0), egui::pos2(130.0, 160.0), left));
    assert!(!heading(egui::pos2(150.0, 110.0), egui::pos2(170.0, 160.0), left));
    assert!(!heading(from, from, egui::Rect::NOTHING));
}

/// #5896, #6225: a menu scrolled down (to reach Edit › Preferences or Window › Theme) reopens
/// scrolled, with the rows above its view lying over the menu bar. egui opens a submenu when the
/// pointer is over its row even where the row is scrolled out of sight, so a hidden submenu row
/// under the title opened on the press and the click on the title then closed the whole menu.
/// Rows out of view must not open their submenus: the menu stays open.
#[test]
fn a_scrolled_menu_reopens_and_stays_open_with_a_hidden_submenu_row_under_its_title() {
    use egui::{Modifiers, PointerButton};
    for top in ["Edit", "Window"] {
        let mut h = harness((1280.0, 420.0, 1.0));
        open(&mut h, top);
        let title = h.get_by_label(top).rect().center();
        let view = Nav::current(&h.ctx).views[0];
        assert!(Nav::current(&h.ctx).contents[0] > view.height() + 1.0, "{top} must overflow for this test to mean anything");
        // Scroll to the end, then back up until a submenu row is out of view right under the title.
        h.hover_at(egui::pos2(view.center().x, view.bottom() - 2.0));
        let wheel = |h: &mut Harness<'static, PhotocraftApp>, dy: f32| {
            h.event(egui::Event::MouseWheel {
                source: egui::MouseWheelSource::Unknown,
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, dy),
                modifiers: Modifiers::NONE,
                phase: egui::TouchPhase::Move,
            });
            h.run_steps(1);
        };
        for _ in 0..60 {
            wheel(&mut h, -60.0);
        }
        let under = |h: &Harness<'static, PhotocraftApp>| {
            rows(h, 0).iter().any(|r| r.command.is_none() && r.enabled && r.rect.x_range().contains(title.x) && r.rect.y_range().contains(title.y))
        };
        for _ in 0..400 {
            if under(&h) {
                break;
            }
            wheel(&mut h, 8.0);
        }
        assert!(under(&h), "{top}: a submenu row scrolled under the title");
        key(&mut h, egui::Key::Escape);
        assert!(!is_open(&h.ctx), "{top}: Esc closed it");
        // Click the title again: press, release, as a mouse does.
        h.hover_at(title);
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: title, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: title, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        // Per frame after the click: is the menu open, and how many levels are drawn?
        let frames: Vec<(bool, usize)> = (0..6)
            .map(|_| {
                h.run_steps(1);
                let nav = Nav::current(&h.ctx);
                (nav.rows.first().is_some_and(|r| !r.is_empty()), nav.rows.len())
            })
            .collect();
        assert!(frames.iter().all(|f| f.0), "{top}: the menu closed again right after the click: {frames:?}");
        assert!(frames.iter().all(|f| f.1 == 1), "{top}: a submenu scrolled out of view opened under the title: {frames:?}");
    }
}

/// Frames (at 120 a second, from when the pointer stops) until Image › Adjustments' submenu is
/// open and its row the highlighted one, after the pointer steps from Image › Mode onto it along
/// `path` (0…1) in `frames` frames; `None` if that takes over a second.
fn frames_to_switch(h: &mut Harness<'static, PhotocraftApp>, frames: usize, path: impl Fn(f32) -> egui::Pos2) -> Option<usize> {
    let adjustments = h.get_by_label_contains("Adjustments").rect();
    glide(h, frames, None, path);
    let row = rows(h, 0).iter().position(|r| r.rect.contains(adjustments.center()));
    (0..=120).find(|&k| {
        if k > 0 {
            h.run_steps(1);
        }
        image_submenu(h) == "Adjustments" && Nav::current(&h.ctx).highlighted(0) == row
    })
}

/// A step straight down from Mode onto Adjustments, beside Mode's submenu, switches submenus as
/// soon as the pointer stops. egui's heading check counted a pointer moving straight down beside a
/// submenu as moving toward it and kept Mode's open (and lit) for another 0.1 s.
#[test]
fn a_step_straight_down_switches_submenus_at_once() {
    let mut h = harness_dt(1.0 / 120.0);
    let (from, adjustments, _) = rest_on_mode(&mut h);
    let to = egui::pos2(from.x, adjustments.center().y);
    let waited = frames_to_switch(&mut h, 6, |t| from + (to - from) * t);
    assert!(waited.is_some_and(|k| k <= 2), "switched {waited:?} frames after the pointer stopped");
}

/// The same after sliding along Mode toward its arrow first: egui judges the direction over the
/// last 0.1 s, so the step down still read as heading for Mode's submenu.
#[test]
fn a_step_down_after_sliding_toward_the_arrow_switches_at_once() {
    let mut h = harness_dt(1.0 / 120.0);
    let (from, adjustments, _) = rest_on_mode(&mut h);
    let slid = from + egui::vec2(20.0, 0.0);
    glide(&mut h, 6, None, |t| from + (slid - from) * t);
    let to = egui::pos2(slid.x, adjustments.center().y);
    let waited = frames_to_switch(&mut h, 3, |t| slid + (to - slid) * t);
    assert!(waited.is_some_and(|k| k <= 2), "switched {waited:?} frames after the pointer stopped");
}

/// Stopping on Adjustments while still heading for Mode's submenu keeps it for `AIM_REST` from
/// when the pointer stopped, not longer: resting was timed from egui's smoothed velocity, which
/// still reads "moving" 0.1 s after the pointer stops (0.39 s in all).
#[test]
fn resting_while_heading_for_a_submenu_switches_after_aim_rest() {
    let mut h = harness_dt(1.0 / 120.0);
    let (from, adjustments, _) = rest_on_mode(&mut h);
    let to = egui::pos2(from.x + 10.0, adjustments.center().y);
    let waited = frames_to_switch(&mut h, 6, |t| from + (to - from) * t);
    let rest = (super::AIM_REST * 120.0).round() as usize;
    assert!(waited.is_some_and(|k| k + 2 >= rest && k <= rest + 2), "switched {waited:?} frames after the pointer stopped, AIM_REST is {rest}");
}

/// While a submenu is held, the highlight is on the row under the pointer, the submenu stays open
/// beside it, and the submenu's own highlight is gone.
#[test]
fn the_highlight_follows_the_pointer_while_a_submenu_is_held() {
    let mut h = harness_dt(1.0 / 120.0);
    let (from, adjustments, target) = rest_on_mode(&mut h);
    let to = egui::pos2(target.left() + 40.0, target.center().y);
    let row = rows(&h, 0).iter().position(|r| r.rect.contains(adjustments.center()));
    let mut seen = false;
    for k in 0..=36 {
        let p = from + (to - from) * (k as f32 / 36.0);
        move_pointer(&mut h, p);
        seen |= adjustments.contains(p) && Nav::current(&h.ctx).highlighted(0) == row && image_submenu(&h) == "Mode";
    }
    assert!(seen, "Adjustments was lit with Mode's submenu still open");
}

/// The row of the top-level menu whose submenu is open.
fn open_submenu_row(h: &Harness<'static, PhotocraftApp>) -> Option<usize> {
    let nav = Nav::current(&h.ctx);
    let open = MenuState::from_id(&h.ctx, *nav.roots.first()?, |s| s.open_item)?;
    nav.rows.first()?.iter().position(|r| SubMenu::id_from_widget_id(r.widget) == open)
}

/// Inching straight up or down, a pixel a frame, between two submenu rows with no separator
/// between them (Window › Arrange and Workspace): the submenu switches as the pointer reaches the
/// other row, and that row, once highlighted, stays highlighted. egui counted a pointer moving
/// straight up or down beside a submenu as heading for it, so the old submenu stayed open, its row
/// lit, for the whole step down (and 0.1 s after), and a step up lit the new row, then the old one
/// for a few frames, then the new one again.
#[test]
fn inching_between_touching_submenu_rows_switches_at_once() {
    for (old, new) in [(1, 0), (0, 1)] {
        let mut h = harness_dt(1.0 / 120.0);
        open(&mut h, "Window");
        let all = rows(&h, 0);
        let (from, to) = (all[old].rect, all[new].rect);
        assert!(from.bottom() == to.top() || to.bottom() == from.top(), "Arrange and Workspace touch");
        let x = from.right() - 30.0;
        let rest = egui::pos2(x, from.center().y);
        glide(&mut h, 12, None, |t| rest - egui::vec2(30.0 * (1.0 - t), 0.0));
        h.run_steps(60);
        assert_eq!(open_submenu_row(&h), Some(old), "resting on row {old} opens its submenu");
        let mut late = Vec::new();
        let mut lit_since = None;
        let steps = (to.center().y - from.center().y).abs() as usize;
        for s in 1..=steps {
            let y = rest.y + (to.center().y - rest.y).signum() * s as f32;
            h.event(egui::Event::PointerMoved(egui::pos2(x, y)));
            h.run_steps(1);
            let lit = Nav::current(&h.ctx).highlighted(0) == Some(new);
            if lit {
                lit_since.get_or_insert(s);
            } else if lit_since.is_some() {
                late.push(format!("row {new} went dark again at step {s}"));
            }
            // A frame to follow the pointer, past the rows' shared edge.
            if to.contains(egui::pos2(x, y - (to.center().y - rest.y).signum() * 2.0)) && open_submenu_row(&h) != Some(new) {
                late.push(format!("step {s}: the pointer is on row {new} but row {:?}'s submenu is open", open_submenu_row(&h)));
            }
        }
        assert!(lit_since.is_some(), "row {new} was never highlighted");
        assert!(late.is_empty(), "{old} → {new}: {late:#?}");
    }
}

/// The fill painted behind a row of `rect`, if any.
fn row_fill(h: &Harness<'static, PhotocraftApp>, rect: egui::Rect) -> Option<egui::Color32> {
    fn walk(shape: &egui::Shape, rect: egui::Rect, out: &mut Option<egui::Color32>) {
        match shape {
            egui::Shape::Rect(r) if r.fill.a() > 0 && rect.x_range().contains(r.rect.center().x) && (r.rect.center().y - rect.center().y).abs() < 3.0 && (r.rect.height() - rect.height()).abs() < 6.0 && r.rect.width() > rect.width() * 0.6 => {
                *out = Some(r.fill);
            }
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, rect, out)),
            _ => {}
        }
    }
    let mut fill = None;
    for clipped in &h.output().shapes {
        walk(&clipped.shape, rect, &mut fill);
    }
    fill
}

/// Inching from a submenu row onto a separator or onto the next submenu row, the row left stays
/// highlighted or goes dark: it isn't drawn for a frame in the open-submenu style (gray), and two
/// rows are never highlighted at once. The row was drawn before its submenu closed in the same
/// frame, and the pointer on two touching rows' shared edge counted as on both.
#[test]
fn leaving_a_submenu_row_never_flashes_the_open_submenu_style() {
    // (menu, row left, row toward, stop halfway into the separator between them)
    let cases = [("Image", 0, 1, true), ("Image", 1, 0, true), ("Window", 0, 1, false), ("Window", 1, 0, false)];
    let mut wrong = Vec::new();
    for (menu, old, new, separator) in cases {
        for per in [1, 3] {
            let mut h = harness_dt(1.0 / 120.0);
            open(&mut h, menu);
            let all = rows(&h, 0);
            let (from, to) = (all[old].rect, all[new].rect);
            let gap = (from.bottom().min(to.bottom()), from.top().max(to.top()));
            assert_eq!(separator, gap.1 - gap.0 > 4.0, "{menu}: rows {old} and {new}");
            let x = from.right() - 30.0;
            let rest = egui::pos2(x, from.center().y);
            glide(&mut h, 12, None, |t| rest - egui::vec2(30.0 * (1.0 - t), 0.0));
            h.run_steps(60);
            let accent = crate::theme::Tokens::get(&h.ctx).accent;
            let end = if separator { (gap.0 + gap.1) / 2.0 } else { to.center().y };
            let steps = (end - rest.y).abs() as usize;
            for s in 1..=steps + 8 {
                let y = rest.y + (end - rest.y).signum() * s.min(steps) as f32;
                h.event(egui::Event::PointerMoved(egui::pos2(x, y)));
                for _ in 0..per {
                    h.run_steps(1);
                    let (left, toward) = (row_fill(&h, from), row_fill(&h, to));
                    if left.is_some_and(|c| c != accent) {
                        wrong.push(format!("{menu} {old}→{new} 1px/{per} frames, y {y}: row {old} drawn {left:?}, not highlighted or dark"));
                    }
                    if left == Some(accent) && toward == Some(accent) {
                        wrong.push(format!("{menu} {old}→{new} 1px/{per} frames, y {y}: both rows highlighted"));
                    }
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// A quick move down and toward the open submenu, onto the next submenu row (Window › Arrange to
/// Workspace): the submenu stays open while the pointer heads for it, and the highlight moves to
/// the row under the pointer once, without a frame of neither or both rows lit.
#[test]
fn heading_for_a_submenu_hands_the_highlight_over_once() {
    let mut wrong = Vec::new();
    for (frames, dx) in [(2, 26.0), (3, 30.0), (4, 28.0), (6, 34.0)] {
        let mut h = harness_dt(1.0 / 120.0);
        open(&mut h, "Window");
        let all = rows(&h, 0);
        let (from, to) = (all[0].rect, all[1].rect);
        let rest = egui::pos2(from.right() - 40.0, from.center().y);
        glide(&mut h, 12, None, |t| rest - egui::vec2(30.0 * (1.0 - t), 0.0));
        h.run_steps(60);
        let accent = crate::theme::Tokens::get(&h.ctx).accent;
        let end = egui::pos2(rest.x + dx, to.center().y);
        let mut lit = String::new();
        let mut kept = 0;
        for k in 1..=frames + 60 {
            if k <= frames {
                h.event(egui::Event::PointerMoved(rest + (end - rest) * (k as f32 / frames as f32)));
            }
            h.run_steps(1);
            kept += usize::from(k >= frames && open_submenu_row(&h) == Some(0));
            lit.push(match (row_fill(&h, from) == Some(accent), row_fill(&h, to) == Some(accent)) {
                (true, false) => 'A',
                (false, true) => 'W',
                (true, true) => '2',
                (false, false) => '.',
            });
        }
        // Arrange lit, then Workspace lit: at most one handover, nothing else.
        let handovers = lit.as_bytes().windows(2).filter(|w| w[0] != w[1]).count();
        if lit.contains(['2', '.']) || !lit.ends_with('W') || handovers > 1 {
            wrong.push(format!("{frames} frames, {dx:+} px: {lit}"));
        }
        if kept + 2 < (super::AIM_REST * 120.0) as usize {
            wrong.push(format!("{frames} frames, {dx:+} px: Arrange's submenu was kept {kept} frames while heading for it"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// Run frames only when the last one asked for another (as the app does: egui draws on demand),
/// until none is asked for within 2 s; each wait is spent in `step_dt` steps.
fn run_until_idle(h: &mut Harness<'static, PhotocraftApp>) {
    let mut waited = std::time::Duration::ZERO;
    while waited < std::time::Duration::from_secs(2) {
        let delay = h.output().viewport_output.get(&egui::ViewportId::ROOT).map_or(std::time::Duration::MAX, |v| v.repaint_delay);
        if delay > std::time::Duration::from_secs(2) {
            return;
        }
        let steps = ((delay.as_secs_f32() * 120.0).ceil() as usize).max(1);
        h.run_steps(steps);
        waited += std::time::Duration::from_secs_f32(steps as f32 / 120.0);
    }
}

/// Heading for a submenu, the pointer stops on a command row instead (Image › Mode to Auto Tone,
/// Window › Arrange to Brushes): once the hold lets the submenu go, it disappears and the row
/// under the pointer is highlighted, without the pointer moving again. The submenu was closed
/// after its frame was drawn, and with the pointer resting nothing asked for the next frame, so
/// it stayed on screen until the mouse moved.
#[test]
fn a_submenu_let_go_while_the_pointer_rests_disappears() {
    for (menu, nth_command) in [("Image", 1), ("Image", 3), ("Window", 2), ("Window", 4)] {
        let mut h = harness_dt(1.0 / 120.0);
        open(&mut h, menu);
        let all = rows(&h, 0);
        let src = all.iter().position(|r| r.command.is_none() && r.enabled).expect("a submenu row");
        let (dest, _) = all.iter().enumerate().skip(src + 1).filter(|(_, r)| r.command.is_some() && r.enabled).nth(nth_command - 1).expect("a command row below it");
        let (from, to) = (all[src].rect, all[dest].rect);
        let rest = egui::pos2(from.right() - 60.0, from.center().y);
        glide(&mut h, 12, None, |t| rest - egui::vec2(30.0 * (1.0 - t), 0.0));
        h.run_steps(60);
        assert_eq!(open_submenu_row(&h), Some(src), "{menu}: resting on row {src} opens its submenu");
        let end = egui::pos2(rest.x + 40.0, to.center().y);
        glide(&mut h, 4, None, |t| rest + (end - rest) * t);
        assert_eq!(open_submenu_row(&h), Some(src), "{menu}: heading for it keeps the submenu open");
        run_until_idle(&mut h);
        assert!(rows(&h, 1).is_empty(), "{menu}: row {src}'s submenu is still drawn with the pointer resting on row {dest}");
        assert_eq!(Nav::current(&h.ctx).highlighted(0), Some(dest), "{menu}: the row under the pointer is highlighted");
    }
}

/// Into a submenu, then out of it straight onto another row of the menu, not over the row it hangs
/// from: the highlight goes from that row to the other in one frame, without either being gray or
/// both lit. The pointer heading for the submenu was judged from where it had left the row on the
/// way in, so landing below and to the right of that held the submenu a frame or two; and the
/// pointer on the menu's border closed it with no row there to take the highlight.
#[test]
fn leaving_a_submenu_for_another_row_hands_the_highlight_over() {
    // (menu, row landed on, submenu row the pointer leaves from, frames to get there)
    let cases = [("Window", 1, 0.5, 4), ("Window", 3, 2.5, 4), ("Window", 1, 2.5, 8), ("Image", 2, 4.5, 6)];
    let mut wrong = Vec::new();
    for (menu, land_on, sub_row, frames) in cases {
        let mut h = harness_dt(1.0 / 120.0);
        open(&mut h, menu);
        let all = rows(&h, 0);
        let src = all.iter().position(|r| r.command.is_none() && r.enabled).expect("a submenu row");
        let (row, dest) = (all[src].rect, all[land_on].rect);
        let rest = egui::pos2(row.right() - 40.0, row.center().y);
        glide(&mut h, 12, None, |t| rest - egui::vec2(30.0 * (1.0 - t), 0.0));
        h.run_steps(60);
        let sub = Nav::current(&h.ctx).views[1];
        let inside = egui::pos2(sub.left() + 40.0, sub.top() + 22.0 * sub_row + 8.0);
        glide(&mut h, 10, None, |t| rest + (inside - rest) * t);
        h.run_steps(40);
        let accent = crate::theme::Tokens::get(&h.ctx).accent;
        let out = egui::pos2(dest.right() - 30.0, dest.center().y);
        let mut seen = String::new();
        for k in 1..=frames + 30 {
            if k <= frames {
                h.event(egui::Event::PointerMoved(inside + (out - inside) * (k as f32 / frames as f32)));
            }
            h.run_steps(1);
            let (left, landed) = (row_fill(&h, row), row_fill(&h, dest));
            seen.push(match (left, landed == Some(accent)) {
                (Some(c), _) if c == accent => 'L',
                (Some(_), true) => '2',
                (Some(_), false) => 'g',
                (None, true) => 'D',
                (None, false) => '.',
            });
        }
        if seen.contains(['2', 'g', '.']) || !seen.starts_with('L') || !seen.ends_with('D') {
            wrong.push(format!("{menu} row {src} → {land_on}: {seen} (L: row {src} lit, D: row {land_on} lit, 2: both, g: gray)"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// With the pointer in a submenu, the row it hangs from is drawn highlighted, not in egui's gray
/// open-submenu style, so it's easy to see where the submenu came from. Back on that row it stays
/// lit and nothing in the submenu is.
#[test]
fn inside_a_submenu_its_row_stays_highlighted() {
    let mut h = harness_dt(1.0 / 120.0);
    let (from, _, target) = rest_on_mode(&mut h);
    let mode = rows(&h, 0).iter().find(|r| r.rect.contains(from)).expect("the Mode row").rect;
    let accent = crate::theme::Tokens::get(&h.ctx).accent;
    let inside = egui::pos2(target.left() + 40.0, target.center().y);
    glide(&mut h, 20, None, |t| from + (inside - from) * t);
    h.run_steps(4);
    assert_eq!(image_submenu(&h), "Mode");
    assert_eq!(row_fill(&h, mode), Some(accent), "in the submenu");
    assert!(Nav::current(&h.ctx).highlighted(1).is_some(), "the submenu's own row is lit too");
    glide(&mut h, 20, None, |t| inside + (from - inside) * t);
    h.run_steps(4);
    assert_eq!(row_fill(&h, mode), Some(accent), "back on the row");
    assert_eq!(Nav::current(&h.ctx).highlighted(1), None, "nothing in the submenu is lit");
}

/// The keys light a row exactly as the pointer does. egui draws a focused widget in its active
/// style, and the arrow keys move the focus along, so a row reached from the keyboard was gray.
#[test]
fn the_keyboard_highlights_rows_as_the_pointer_does() {
    let mut h = harness_dt(1.0 / 120.0);
    open(&mut h, "Image");
    let all = rows(&h, 0);
    let accent = crate::theme::Tokens::get(&h.ctx).accent;
    for _ in 0..4 {
        key(&mut h, egui::Key::ArrowDown);
        h.run_steps(5);
        let i = Nav::current(&h.ctx).highlighted(0).expect("a highlighted row");
        assert_eq!(row_fill(&h, all[i].rect), Some(accent), "row {i}");
    }
}

/// Moves the pointer to `p` and runs a frame, as the other pointer tests do.
fn move_pointer(h: &mut Harness<'static, PhotocraftApp>, p: egui::Pos2) {
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
}

/// The pointer on a separator takes the highlight off the row it came from, whichever side that
/// is, as in native menus; the next row it reaches takes it. Before, the row stayed lit until the
/// pointer reached another row.
#[test]
fn a_separator_takes_the_highlight_off_whichever_row_was_lit() {
    let mut h = harness((1280.0, 720.0, 1.0));
    let mut checked = 0;
    for top in ["File", "Edit", "Select", "View", "Window"] {
        open(&mut h, top);
        let row_list = rows(&h, 0);
        // Two neighbouring rows, both on screen, with a separator's gap between them.
        let gap = (1..row_list.len()).find(|&i| visible(&h, 0, i - 1) && visible(&h, 0, i) && row_list[i].rect.top() - row_list[i - 1].rect.bottom() > 4.0);
        if let Some(i) = gap {
            let (above, below) = (&row_list[i - 1], &row_list[i]);
            let on_separator = egui::pos2(above.rect.center().x, (above.rect.bottom() + below.rect.top()) / 2.0);
            let lit = |h: &Harness<'static, PhotocraftApp>| Nav::current(&h.ctx).highlighted(0);
            // Down: from the row above, over the separator, onto the row below.
            move_pointer(&mut h, above.rect.center());
            assert_eq!(lit(&h), Some(i - 1), "{top}: the pointer on the row above lights it");
            move_pointer(&mut h, on_separator);
            assert_eq!(lit(&h), None, "{top}: coming from above, the separator takes the highlight off");
            move_pointer(&mut h, below.rect.center());
            assert_eq!(lit(&h), Some(i), "{top}: the row below takes it");
            // And back up: from the row below, over the separator, onto the row above.
            move_pointer(&mut h, on_separator);
            assert_eq!(lit(&h), None, "{top}: coming from below, the separator takes the highlight off");
            move_pointer(&mut h, above.rect.center());
            assert_eq!(lit(&h), Some(i - 1), "{top}: the row above takes it again");
            checked += 1;
        }
        key(&mut h, egui::Key::Escape);
    }
    assert!(checked > 0, "some menu has a separator between two visible rows");
}

/// Inside a submenu (Filter › Blur), inching from one item to the next hands the highlight over
/// in one frame, as in the top-level menus: the item left isn't lit beside the next one. The
/// highlight followed the pointer before the rows were drawn only in top-level menus; a submenu's
/// rows from the frame before were dropped before it was drawn.
#[test]
fn inside_a_submenu_the_highlight_hands_over_in_one_frame() {
    let mut wrong = Vec::new();
    for (from, to) in [(1, 2), (2, 1), (3, 4)] {
        let mut h = harness_dt(1.0 / 120.0);
        open(&mut h, "Filter");
        let blur = h.get_by_label("Blur").rect();
        let rest = egui::pos2(blur.right() - 40.0, blur.center().y);
        glide(&mut h, 12, None, |t| rest - egui::vec2(30.0 * (1.0 - t), 0.0));
        h.run_steps(60);
        let items = rows(&h, 1);
        let (a, b) = (items[from].rect, items[to].rect);
        assert!(items[from].enabled && items[to].enabled && (a.bottom() == b.top() || b.bottom() == a.top()), "Blur's items {from} and {to} touch");
        let start = egui::pos2(a.left() + 40.0, a.center().y);
        glide(&mut h, 10, None, |t| rest + (start - rest) * t);
        h.run_steps(30);
        let accent = crate::theme::Tokens::get(&h.ctx).accent;
        let mut seen = String::new();
        let steps = (b.center().y - a.center().y).abs() as usize;
        for s in 1..=steps + 4 {
            let y = a.center().y + (b.center().y - a.center().y).signum() * s.min(steps) as f32;
            h.event(egui::Event::PointerMoved(egui::pos2(start.x, y)));
            h.run_steps(1);
            seen.push(match (row_fill(&h, a) == Some(accent), row_fill(&h, b) == Some(accent)) {
                (true, false) => 'A',
                (false, true) => 'B',
                (true, true) => '2',
                (false, false) => '.',
            });
        }
        let handovers = seen.as_bytes().windows(2).filter(|w| w[0] != w[1]).count();
        if !seen.starts_with('A') || !seen.ends_with('B') || handovers != 1 {
            wrong.push(format!("Blur items {from} → {to}: {seen}"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// A window tall enough for the whole Window menu, down to Theme (the 1280 × 720 one scrolls it).
fn tall_harness() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1400.0)).with_step_dt(1.0 / 120.0).build_ui_state(
        |ui, app| {
            crate::menus::menu_bar(app, ui);
        },
        app(),
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.ctx.all_styles_mut(|s| s.scroll_animation = egui::style::ScrollAnimation::none());
    h.run_steps(3);
    h
}

/// The row of `level` whose submenu is open.
fn open_submenu_row_at(h: &Harness<'static, PhotocraftApp>, level: usize) -> Option<usize> {
    let nav = Nav::current(&h.ctx);
    let open = MenuState::from_id(&h.ctx, *nav.roots.get(level)?, |s| s.open_item)?;
    nav.rows.get(level)?.iter().position(|r| SubMenu::id_from_widget_id(r.widget) == open)
}

/// Window › Theme open with the pointer resting on its Dark Theme row, so Dark Theme's submenu is
/// open; returns the Dark Theme and Light Theme rows (touching submenu rows, one level down) and
/// where the pointer rests.
fn rest_on_dark_theme(h: &mut Harness<'static, PhotocraftApp>) -> (egui::Rect, egui::Rect, egui::Pos2) {
    open(h, "Window");
    let theme = h.query_all_by_label("Theme").next().expect("Window › Theme").rect();
    let p = egui::pos2(theme.right() - 30.0, theme.center().y);
    glide(h, 12, None, |t| p - egui::vec2(30.0 * (1.0 - t), 0.0));
    h.run_steps(60);
    let themes = rows(h, 1);
    let (dark, light) = (themes[0].rect, themes[1].rect);
    assert!(themes[0].command.is_none() && themes[1].command.is_none() && dark.bottom() == light.top(), "Dark Theme and Light Theme are touching submenu rows");
    let on_dark = egui::pos2(dark.left() + 60.0, dark.center().y);
    glide(h, 10, None, |t| p + (on_dark - p) * t);
    h.run_steps(60);
    assert_eq!(open_submenu_row_at(h, 1), Some(0), "resting on Dark Theme opens its submenu");
    (dark, light, on_dark)
}

/// One level down (Window › Theme › Dark Theme and Light Theme), inching between two touching
/// submenu rows switches submenus as the pointer reaches the other row and hands the highlight
/// over in one frame, as in the top-level menus; both rows were lit for a frame.
#[test]
fn in_a_submenu_touching_submenu_rows_switch_at_once() {
    let mut wrong = Vec::new();
    for down in [true, false] {
        let mut h = tall_harness();
        let (dark, light, on_dark) = rest_on_dark_theme(&mut h);
        let (from, to, x, want) = if down {
            (dark, light, on_dark.x, 1)
        } else {
            let on_light = egui::pos2(on_dark.x, light.center().y);
            glide(&mut h, 4, None, |t| on_dark + (on_light - on_dark) * t);
            h.run_steps(60);
            (light, dark, on_dark.x, 0)
        };
        let accent = crate::theme::Tokens::get(&h.ctx).accent;
        let mut seen = String::new();
        let steps = (to.center().y - from.center().y).abs() as usize;
        for s in 1..=steps + 6 {
            let y = from.center().y + (to.center().y - from.center().y).signum() * s.min(steps) as f32;
            h.event(egui::Event::PointerMoved(egui::pos2(x, y)));
            h.run_steps(1);
            seen.push(match (row_fill(&h, from) == Some(accent), row_fill(&h, to) == Some(accent)) {
                (true, false) => 'A',
                (false, true) => 'B',
                (true, true) => '2',
                (false, false) => '.',
            });
            if to.contains(egui::pos2(x, y - (to.center().y - from.center().y).signum() * 2.0)) && open_submenu_row_at(&h, 1) != Some(want) {
                wrong.push(format!("{}: step {s}, the pointer is on row {want} but row {:?}'s submenu is open", if down { "down" } else { "up" }, open_submenu_row_at(&h, 1)));
            }
        }
        let handovers = seen.as_bytes().windows(2).filter(|w| w[0] != w[1]).count();
        if !seen.starts_with('A') || !seen.ends_with('B') || handovers != 1 {
            wrong.push(format!("{}: {seen}", if down { "Dark → Light" } else { "Light → Dark" }));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// Into a submenu, onto one of its rows, then straight back onto the row it hangs from: the
/// submenu stays open with nothing in it highlighted, and only the row it hangs from is lit, as in
/// native menus. The submenu's row stayed highlighted too.
#[test]
fn back_on_its_row_a_submenu_has_nothing_highlighted() {
    let mut wrong = Vec::new();
    for (menu, sub_row) in [("Window", 0usize), ("Window", 2), ("Image", 1)] {
        let mut h = harness_dt(1.0 / 120.0);
        open(&mut h, menu);
        let all = rows(&h, 0);
        let src = all.iter().position(|r| r.command.is_none() && r.enabled).expect("a submenu row");
        let row = all[src].rect;
        let rest = egui::pos2(row.right() - 40.0, row.center().y);
        glide(&mut h, 12, None, |t| rest - egui::vec2(30.0 * (1.0 - t), 0.0));
        h.run_steps(60);
        // Across into the submenu, onto its row `sub_row`.
        let target = rows(&h, 1).get(sub_row).expect("a row in the submenu").rect;
        let inside = egui::pos2(target.left() + 40.0, target.center().y);
        glide(&mut h, 10, None, |t| rest + (inside - rest) * t);
        h.run_steps(20);
        if Nav::current(&h.ctx).highlighted(1) != Some(sub_row) {
            wrong.push(format!("{menu}: the pointer on submenu row {sub_row} didn't highlight it"));
            continue;
        }
        // Straight back onto the row it hangs from.
        let back = egui::pos2(row.right() - 20.0, row.center().y);
        glide(&mut h, 8, None, |t| inside + (back - inside) * t);
        h.run_steps(20);
        let nav = Nav::current(&h.ctx);
        let accent = crate::theme::Tokens::get(&h.ctx).accent;
        let lit_in_sub: Vec<usize> = rows(&h, 1).iter().enumerate().filter(|(_, r)| row_fill(&h, r.rect) == Some(accent)).map(|(i, _)| i).collect();
        if open_submenu_row(&h) != Some(src) {
            wrong.push(format!("{menu}: back on row {src}, its submenu closed"));
        }
        if nav.highlighted(0) != Some(src) {
            wrong.push(format!("{menu}: back on row {src}, row {:?} is highlighted", nav.highlighted(0)));
        }
        if nav.highlighted(1).is_some() || !lit_in_sub.is_empty() {
            wrong.push(format!("{menu}: back on row {src}, submenu row {:?} still highlighted (painted lit: {lit_in_sub:?})", nav.highlighted(1)));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// One level down too (Window › Theme › Dark Theme): into Dark Theme's submenu, onto one of its
/// rows, then back onto Dark Theme: only Dark Theme is lit and its submenu has nothing highlighted.
/// And out onto Light Theme instead, Light Theme's submenu doesn't open with that row lit.
#[test]
fn back_on_its_row_a_nested_submenu_has_nothing_highlighted() {
    let mut wrong = Vec::new();
    for back_on_dark in [true, false] {
        let mut h = tall_harness();
        let (dark, light, on_dark) = rest_on_dark_theme(&mut h);
        let item = rows(&h, 2).get(1).expect("a row in Dark Theme's submenu").rect;
        let inside = egui::pos2(item.left() + 40.0, item.center().y);
        glide(&mut h, 10, None, |t| on_dark + (inside - on_dark) * t);
        h.run_steps(30);
        assert_eq!(Nav::current(&h.ctx).highlighted(2), Some(1), "the pointer on Dark Theme's row 1 highlights it");
        let dest = if back_on_dark { egui::pos2(dark.right() - 20.0, dark.center().y) } else { egui::pos2(light.right() - 20.0, light.center().y) };
        glide(&mut h, 6, None, |t| inside + (dest - inside) * t);
        h.run_steps(30);
        let accent = crate::theme::Tokens::get(&h.ctx).accent;
        let lit: Vec<usize> = rows(&h, 2).iter().enumerate().filter(|(_, r)| row_fill(&h, r.rect) == Some(accent)).map(|(i, _)| i).collect();
        let (on, want) = if back_on_dark { ("Dark Theme", 0) } else { ("Light Theme", 1) };
        let nav = Nav::current(&h.ctx);
        if open_submenu_row_at(&h, 1) != Some(want) || nav.highlighted(1) != Some(want) {
            wrong.push(format!("on {on}: row {:?} highlighted, row {:?}'s submenu open", nav.highlighted(1), open_submenu_row_at(&h, 1)));
        }
        if nav.highlighted(2).is_some() || !lit.is_empty() {
            wrong.push(format!("on {on}: its submenu has row {:?} highlighted (painted lit: {lit:?})", nav.highlighted(2)));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// The rows of the open levels drawn with any fill, as (level, row).
fn filled_rows(h: &Harness<'static, PhotocraftApp>) -> Vec<(usize, usize)> {
    let mut filled = Vec::new();
    for level in 0..2 {
        for (i, r) in rows(h, level).iter().enumerate() {
            if row_fill(h, r.rect).is_some() {
                filled.push((level, i));
            }
        }
    }
    filled
}

/// The pointer left on a submenu's row, the arrow keys take over: one row is lit, the submenu
/// closes, and no other row opens under the pointer. egui's hover kept the pointer's row lit, and
/// its focus walked a gray row through the submenu beside the menu.
#[test]
fn with_the_pointer_resting_the_arrow_keys_light_one_row() {
    let mut h = harness_dt(1.0 / 120.0);
    rest_on_mode(&mut h);
    for k in [egui::Key::ArrowDown, egui::Key::ArrowDown, egui::Key::ArrowDown, egui::Key::ArrowUp, egui::Key::ArrowUp, egui::Key::ArrowUp] {
        key(&mut h, k);
        h.run_steps(5);
        let hi = Nav::current(&h.ctx).highlighted(0).expect("a highlighted row");
        assert_eq!(filled_rows(&h), vec![(0, hi)], "only the keys' row is lit");
        assert_eq!(image_submenu(&h), "none", "nothing is open with the pointer resting");
        assert_eq!(h.ctx.memory(|m| m.focused()), None, "egui's focus doesn't walk along");
    }
}

/// Moving the pointer gives the menu back to it.
#[test]
fn moving_the_pointer_after_the_arrow_keys_takes_the_menu_back() {
    let mut h = harness_dt(1.0 / 120.0);
    let (from, ..) = rest_on_mode(&mut h);
    key(&mut h, egui::Key::ArrowDown);
    h.run_steps(5);
    assert_eq!(image_submenu(&h), "none");
    glide(&mut h, 6, None, |t| from + egui::vec2(6.0 * t, 0.0));
    h.run_steps(30);
    assert_eq!(image_submenu(&h), "Mode", "the pointer opens the row it is on again");
    assert_eq!(filled_rows(&h), vec![(0, 0)], "and is the one row lit");
}

/// ← out of a submenu with the pointer resting on its row closes it: the row doesn't reopen it.
#[test]
fn left_out_of_a_submenu_stays_closed_with_the_pointer_on_its_row() {
    let mut h = harness_dt(1.0 / 120.0);
    rest_on_mode(&mut h);
    key(&mut h, egui::Key::ArrowRight);
    h.run_steps(5);
    assert_eq!(image_submenu(&h), "Mode");
    key(&mut h, egui::Key::ArrowLeft);
    h.run_steps(30);
    assert_eq!(image_submenu(&h), "none");
    assert_eq!(filled_rows(&h), vec![(0, 0)]);
}

/// A click takes the menu back from the keys too: the pointer resting on a closed submenu row, a
/// click on it did nothing until the mouse moved.
#[test]
fn a_click_after_the_arrow_keys_opens_the_row_under_a_resting_pointer() {
    let mut h = harness_dt(1.0 / 120.0);
    let (from, ..) = rest_on_mode(&mut h);
    key(&mut h, egui::Key::ArrowDown);
    h.run_steps(5);
    assert_eq!(image_submenu(&h), "none");
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        h.run_steps(5);
    }
    assert_eq!(image_submenu(&h), "Mode", "the click opens it");
    assert_eq!(filled_rows(&h), vec![(0, 0)], "and the highlight is on the clicked row");
}

/// Reaching a submenu on a separator, not over a row, the arrows are in that submenu: they stepped
/// the menu above and, doing so, closed the submenu.
#[test]
fn from_a_separator_in_a_submenu_the_arrows_stay_in_it() {
    let mut h = harness_dt(1.0 / 120.0);
    rest_on_mode(&mut h);
    let sub = rows(&h, 1);
    let gap = sub.windows(2).find(|w| w[1].rect.top() - w[0].rect.bottom() > 4.0).expect("a separator in Mode's submenu");
    let on = egui::pos2(gap[0].rect.center().x, (gap[0].rect.bottom() + gap[1].rect.top()) / 2.0);
    h.event(egui::Event::PointerMoved(on));
    h.run_steps(5);
    assert_eq!(image_submenu(&h), "Mode", "the pointer is in the submenu");
    key(&mut h, egui::Key::ArrowDown);
    h.run_steps(5);
    assert_eq!(image_submenu(&h), "Mode", "the submenu stays open");
    assert!(Nav::current(&h.ctx).highlighted(1).is_some(), "and the arrow moved inside it");
}

/// ← / → to the next menu light its title and no other, with the pointer left on the first title.
/// Nothing lit the open menu's title, and egui's focus, moved by the arrow keys too, left a gray
/// title one step behind the open menu.
#[test]
fn switching_menus_with_the_arrow_keys_lights_the_new_title() {
    let mut h = harness_dt(1.0 / 120.0);
    open(&mut h, "Image");
    for (k, title) in [(egui::Key::ArrowLeft, "Edit"), (egui::Key::ArrowLeft, "File"), (egui::Key::ArrowRight, "Edit"), (egui::Key::ArrowRight, "Image")] {
        h.key_press(k);
        h.run_steps(1);
        run_until_idle(&mut h);
        let lit: Vec<_> = ["File", "Edit", "Image", "Layer"].into_iter().filter(|t| row_fill(&h, h.get_by_label(t).rect()).is_some()).collect();
        assert_eq!(lit, vec![title], "the open menu's title, and only that");
    }
}

/// The pointer parked on a submenu's row doesn't stop the keys opening another: egui closes an
/// open submenu with the pointer on another row, so only the parked row's could stay open.
#[test]
fn with_the_pointer_parked_the_keys_open_any_submenu() {
    let mut h = harness_dt(1.0 / 120.0);
    rest_on_mode(&mut h);
    key(&mut h, egui::Key::ArrowDown);
    h.run_steps(5);
    key(&mut h, egui::Key::ArrowRight);
    h.run_steps(30);
    assert_eq!(image_submenu(&h), "Adjustments", "→ opens the row the keys are on");
    let first = Nav::current(&h.ctx).highlighted(1).expect("the submenu's first row is lit");
    key(&mut h, egui::Key::ArrowDown);
    h.run_steps(30);
    assert!(Nav::current(&h.ctx).highlighted(1).is_some_and(|i| i > first), "↓ moves inside it");
    assert_eq!(image_submenu(&h), "Adjustments");
    key(&mut h, egui::Key::ArrowLeft);
    h.run_steps(30);
    assert_eq!(image_submenu(&h), "none", "← closes it");
}

/// With Help's search field empty, ← / → go on to the next menu: the field held them as soon as it
/// had the focus, so the keys could reach Help but never leave it.
#[test]
fn an_empty_help_search_lets_the_arrows_change_menus() {
    let mut h = harness_dt(1.0 / 120.0);
    open(&mut h, "Help");
    let help = Nav::current(&h.ctx).top;
    key(&mut h, egui::Key::ArrowLeft);
    h.run_steps(5);
    assert_ne!(Nav::current(&h.ctx).top, help, "← leaves Help");
    assert_eq!(h.ctx.memory(|m| m.focused()), None, "and the field's focus goes with it");
    key(&mut h, egui::Key::ArrowRight);
    h.run_steps(5);
    assert_eq!(Nav::current(&h.ctx).top, help, "→ comes back");
    key(&mut h, egui::Key::ArrowRight);
    h.run_steps(5);
    assert_ne!(Nav::current(&h.ctx).top, help, "→ leaves it the other way");
}

/// A highlighted row has the arrows, also with text in Help's search field: they stayed with the
/// field while ↓ walked the results.
#[test]
fn a_highlighted_row_in_help_lets_the_arrows_change_menus() {
    let mut h = harness_dt(1.0 / 120.0);
    open(&mut h, "Help");
    let help = Nav::current(&h.ctx).top;
    h.event(egui::Event::Text("lev".into()));
    h.run_steps(3);
    key(&mut h, egui::Key::ArrowLeft);
    h.run_steps(5);
    assert_eq!(Nav::current(&h.ctx).top, help, "with nothing lit, ← moves the field's cursor");
    key(&mut h, egui::Key::ArrowDown);
    h.run_steps(5);
    assert!(Nav::current(&h.ctx).highlighted(0).is_some(), "a result is lit");
    key(&mut h, egui::Key::ArrowLeft);
    h.run_steps(5);
    assert_ne!(Nav::current(&h.ctx).top, help, "← changes menu");
}

/// A separator clears the levels below it too, as a row does: the submenu closed behind it kept its
/// highlight, and a scroll sliding its row back under a resting pointer reopened it with a row lit
/// that the pointer wasn't on.
#[test]
fn a_separator_clears_the_highlight_of_the_submenu_below() {
    let mut h = harness_dt(1.0 / 120.0);
    open(&mut h, "Window");
    let start = rows(&h, 0)[5].rect.center();
    h.event(egui::Event::PointerMoved(start));
    h.run_steps(5);
    let wheel = |h: &mut Harness<'static, PhotocraftApp>, unit, y: f32| {
        h.event(egui::Event::MouseWheel {
            source: egui::MouseWheelSource::Unknown,
            unit,
            delta: egui::vec2(0.0, y),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        });
        h.run_steps(3);
    };
    for _ in 0..40 {
        wheel(&mut h, egui::MouseWheelUnit::Line, -3.0);
    }
    h.run_steps(60);
    // Theme, the last submenu row, and the row below the separator under it.
    let all = rows(&h, 0);
    let i = all.iter().rposition(|r| r.command.is_none()).expect("a submenu row");
    let (row, next) = (all[i].rect, all[i + 1].rect);
    let y = (row.bottom() + next.top()) / 2.0;
    let on_row = row.center();
    glide(&mut h, 8, None, |t| on_row - egui::vec2(20.0 * (1.0 - t), 0.0));
    h.run_steps(40);
    // Into the submenu, at the separator's height, and straight back onto the separator.
    let inside = egui::pos2(rows(&h, 1)[0].rect.center().x, y);
    glide(&mut h, 12, None, |t| on_row + (inside - on_row) * t);
    h.run_steps(10);
    assert!(Nav::current(&h.ctx).highlighted(1).is_some(), "a row of the submenu is lit");
    let on = egui::pos2(row.center().x, y);
    glide(&mut h, 12, None, |t| inside + (on - inside) * t);
    h.run_steps(60);
    assert_eq!(Nav::current(&h.ctx).highlighted(1), None, "nothing stays lit behind the separator");
    wheel(&mut h, egui::MouseWheelUnit::Line, 1.0);
    h.run_steps(20);
    assert_eq!(open_submenu_row(&h), Some(i), "the scroll brings the row back under the pointer");
    assert_eq!(filled_rows(&h).iter().filter(|(level, _)| *level == 1).count(), 0, "with nothing in it lit");
}

/// Changing the search text clears the highlight: the results shift the rows below them, so a
/// highlight on a result landed on a Help row, and ↩ ran that.
#[test]
fn changing_the_search_text_clears_the_highlight() {
    let mut h = harness_dt(1.0 / 120.0);
    open(&mut h, "Help");
    h.event(egui::Event::Text("lev".into()));
    h.run_steps(3);
    key(&mut h, egui::Key::ArrowDown);
    h.run_steps(5);
    assert!(Nav::current(&h.ctx).highlighted(0).is_some(), "a result is lit");
    h.event(egui::Event::Text("zzz".into()));
    h.run_steps(3);
    assert_eq!(Nav::current(&h.ctx).highlighted(0), None, "no row is lit with nothing found");
    key(&mut h, egui::Key::ArrowDown);
    h.run_steps(5);
    assert!(Nav::current(&h.ctx).highlighted(0).is_some(), "↓ still lights a row");
}

/// An accessibility click opens a closed submenu row while the keys have the highlight: only the
/// pointer resting on a row is ignored then, and the row returned before egui saw the click.
#[test]
fn an_accessibility_click_opens_a_submenu_while_the_keys_have_the_highlight() {
    let mut h = harness_dt(1.0 / 120.0);
    rest_on_mode(&mut h);
    key(&mut h, egui::Key::ArrowDown);
    h.run_steps(5);
    assert_eq!(image_submenu(&h), "none");
    h.get_by_label_contains("Adjustments").click_accesskit();
    h.run_steps(10);
    assert_eq!(image_submenu(&h), "Adjustments");
}
