//! A press activates the canvas or tile it reaches, and only that: a menu, popup or panel over
//! the canvas keeps its own press (#11010).

use std::cell::Cell;
use std::rc::Rc;

use crate::PhotocraftApp;
use egui::{Modifiers, vec2};
use egui_kittest::Harness;
use serde_json::json;

/// A popup (as a menu is drawn) at `rect` over the canvas while it is `Some`, and whether its
/// item was clicked.
#[derive(Clone, Default)]
struct Cover {
    rect: Rc<Cell<Option<egui::Rect>>>,
    clicked: Rc<Cell<bool>>,
}

/// Two documents, "Left" and "Right" (active), shown with `arrange` (`None`: tabs).
fn harness(cover: &Cover, arrange: Option<&str>) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300, "name": "Left"})).unwrap();
    app.run("file.new", json!({"width": 400, "height": 300, "name": "Right"})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = false;
    let c = cover.clone();
    let mut h = Harness::builder().with_size(vec2(1000.0, 700.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
            if let Some(r) = c.rect.get() {
                egui::Area::new(egui::Id::unique("menu-over-the-canvas")).order(egui::Order::Foreground).fixed_pos(r.min).show(&ctx, |ui| {
                    if ui.add_sized(r.size(), egui::Button::new("Menu item")).clicked() {
                        c.clicked.set(true);
                    }
                });
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    if let Some(arrange) = arrange {
        let ctx = h.ctx.clone();
        crate::menus::invoke(h.state_mut(), &ctx, &format!("window.arrange.{arrange}"), json!({})).unwrap();
        h.run_steps(6);
    }
    h
}

fn click(h: &mut Harness<'static, PhotocraftApp>, p: egui::Pos2) {
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(2);
    }
}

/// A click on the popup over `at`: the popup gets it. Then the popup goes.
fn click_cover(h: &mut Harness<'static, PhotocraftApp>, cover: &Cover, at: egui::Pos2) {
    cover.rect.set(Some(egui::Rect::from_center_size(at, vec2(160.0, 24.0))));
    cover.clicked.set(false);
    h.run_steps(2);
    click(h, at);
    assert!(cover.clicked.get(), "the popup's item took the click");
    cover.rect.set(None);
    h.run_steps(2);
}

fn active(h: &Harness<'static, PhotocraftApp>) -> String {
    h.state().session.active().map(|s| s.doc.name.clone()).unwrap_or_default()
}

fn index_of(h: &Harness<'static, PhotocraftApp>, name: &str) -> usize {
    h.state().session.documents().iter().position(|d| d.doc.name == name).unwrap()
}

fn tile_of(h: &Harness<'static, PhotocraftApp>, name: &str) -> egui::Rect {
    let doc = index_of(h, name);
    crate::canvas::arranged_cells(h.state(), h.state().last_canvas_rect).unwrap().into_iter().find(|(d, _)| *d == doc).unwrap().1
}

/// Window › Arrange: a menu item over the other tile is the menu's click, so the active document
/// stays and the command it runs goes to the document being worked on. A click on the tile
/// itself switches.
#[test]
fn a_menu_over_another_tile_keeps_the_active_document() {
    let cover = Cover::default();
    let mut h = harness(&cover, Some("twoUpVertical"));
    assert_eq!(active(&h), "Right");
    let tile = tile_of(&h, "Left");
    click_cover(&mut h, &cover, tile.center());
    assert_eq!(active(&h), "Right", "the click was the menu's");
    click(&mut h, tile.center());
    assert_eq!(active(&h), "Left", "a click on the tile itself activates it");
}

/// The main canvas showing a docked document while the active one is in its own window: a menu
/// over it keeps the active document, a click on the canvas takes it back.
#[test]
fn a_menu_over_the_main_canvas_keeps_a_floating_document_active() {
    let cover = Cover::default();
    let mut h = harness(&cover, None);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "window.arrange.floatInWindow", json!({})).unwrap();
    h.run_steps(4);
    assert_eq!(active(&h), "Right", "the floating document is active");
    let canvas = h.state().last_canvas_rect;
    click_cover(&mut h, &cover, canvas.center());
    assert_eq!(active(&h), "Right", "the click was the menu's");
    click(&mut h, canvas.center());
    assert_eq!(active(&h), "Left", "a click on the main canvas activates its document");
}

/// A tool letter held while the canvas is pressed makes the tool temporary (#2672); a press on
/// a menu over the canvas doesn't count as using the canvas.
#[test]
fn a_menu_over_the_canvas_does_not_use_a_held_tool() {
    let cover = Cover::default();
    let mut h = harness(&cover, None);
    let spring = crate::spring_tools::Spring {
        previous: crate::state::Tool::Move,
        tool: crate::state::Tool::Brush,
        keys: vec!['b'],
        since: h.ctx.input(|i| i.time),
        used: false,
        released: false,
    };
    h.state_mut().ui.spring = Some(spring);
    let canvas = h.state().last_canvas_rect;
    click_cover(&mut h, &cover, canvas.center());
    assert!(h.state().ui.spring.as_ref().is_some_and(|s| !s.used), "the menu's click isn't a canvas press");
    h.event(egui::Event::PointerButton { pos: canvas.center(), button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    assert!(h.state().ui.spring.as_ref().is_some_and(|s| s.used), "a press on the canvas is");
}
