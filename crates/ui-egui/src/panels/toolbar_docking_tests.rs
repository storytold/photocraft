//! The toolbar as a dock panel (#12517): it is always a vertical strip (it never docks above, below or beside the
//! canvas, only in dock columns), and its tool flyouts open to the right of the button wherever it is docked and flip
//! to the left only where they would leave the window.

use super::*;
use crate::panels::registry::{CANVAS, TOOLBAR};
use craft::behavior::DockSide;
use craft::behavior::dock::{Action, DockPlacement, PanelPlace, Permissions, Target};
use egui::{PointerButton, Pos2, Rect};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

const SIZE: egui::Vec2 = egui::vec2(1200.0, 800.0);

/// The whole app in a 1200 x 800 window with a small document open.
fn harness() -> Harness<'static, Box<PhotocraftApp>> {
    let mut h = Harness::builder().with_size(SIZE).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, crate::theme::AppTheme::pro());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 64})).expect("new document");
        app
    });
    h.run_steps(4);
    h
}

/// Moves the toolbar's column beside the column holding `anchor`, after it in the dock's column order or before it.
fn dock_toolbar_beside(h: &mut Harness<'static, Box<PhotocraftApp>>, anchor: &str, after: bool) {
    let action = Action::move_column(TOOLBAR.to_owned(), Target::at(anchor.to_owned(), DockPlacement::column(after)));
    crate::panel_docking::apply(h.state_mut(), vec![action]).expect("the toolbar docks beside the column");
    h.run_steps(6);
}

/// The rect of the innermost node named `label`.
fn rect_of(h: &Harness<'static, Box<PhotocraftApp>>, label: &str) -> Option<Rect> {
    h.query_all_by_label(label).last().map(|node| node.rect())
}

/// Right-clicks the middle of `rect`, which opens a tool slot's flyout.
fn right_click(h: &mut Harness<'static, Box<PhotocraftApp>>, rect: Rect) {
    let at: Pos2 = rect.center();
    h.event(egui::Event::PointerMoved(at));
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
    }
    h.run_steps(4);
}

/// The Lasso slot's flyout rows, opened from the toolbar: the slot's button and the Polygonal Lasso row.
fn lasso_flyout(h: &mut Harness<'static, Box<PhotocraftApp>>) -> (Rect, Rect) {
    let button = rect_of(h, "Lasso Tool").expect("the Lasso slot is drawn");
    right_click(h, button);
    let row = rect_of(h, "Polygonal Lasso Tool").expect("the Lasso flyout is open");
    (button, row)
}

/// Docked on the right between the dock's icon column and its panel column, the toolbar still opens its flyouts to the
/// right of the button, over the panels, as the reference application does (they used to open toward the canvas).
#[test]
fn flyouts_open_to_the_right_between_the_end_docks_columns() {
    let mut h = harness();
    dock_toolbar_beside(&mut h, "color", true);
    let toolbar = crate::panel_docking::probe::group_rect(&h, "toolbar");
    let panels = crate::panel_docking::probe::group_rect(&h, "color").expect("the Color group is drawn");
    let (button, row) = lasso_flyout(&mut h);
    assert!(button.right() < panels.left(), "the toolbar sits left of the panel column: {button:?}, {panels:?}, {toolbar:?}");
    assert!(button.center().x > SIZE.x * 0.5, "the toolbar is on the right half of the window: {button:?}");
    assert!(row.left() >= button.right() - 0.5, "the flyout opens to the right of the button: row {row:?}, button {button:?}");
}

/// At the right window edge there is no room to the right, so the flyout flips to the left of the button.
#[test]
fn flyouts_flip_left_at_the_right_window_edge() {
    let mut h = harness();
    dock_toolbar_beside(&mut h, "color", false);
    let (button, row) = lasso_flyout(&mut h);
    assert!(button.right() > SIZE.x - 80.0, "the toolbar is at the right window edge: {button:?}");
    assert!(row.right() <= button.left() + 0.5, "the flyout flips to the left of the button: row {row:?}, button {button:?}");
}

/// Docked at the left edge (the default), the flyout opens to the right.
#[test]
fn flyouts_open_to_the_right_at_the_left_edge() {
    let mut h = harness();
    let (button, row) = lasso_flyout(&mut h);
    assert!(button.left() < 80.0, "the toolbar is at the left window edge: {button:?}");
    assert!(row.left() >= button.right() - 0.5, "the flyout opens to the right of the button: row {row:?}, button {button:?}");
}

/// Where the toolbar is: in a frame of the docked tree (`Node`), in a dock column or floating.
fn toolbar_place(app: &PhotocraftApp) -> Option<PanelPlace> {
    app.ui.docking.layout().place(&TOOLBAR.to_owned())
}

/// The toolbar never docks across the window: a move beside the canvas on any side is refused and changes nothing,
/// while docking it at either edge still works.
#[test]
fn the_toolbar_never_docks_beside_the_canvas() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = app.ctx().clone();
    let before = serde_json::to_value(&app.ui.docking).unwrap();
    for zone in ["top", "bottom", "left", "right"] {
        let params = json!({"panel": "toolbar", "anchor": "canvas", "zone": zone});
        assert!(crate::menus::invoke(&mut app, &ctx, "window.panel.move", params).is_err(), "the toolbar docks {zone} of the canvas");
        assert_eq!(serde_json::to_value(&app.ui.docking).unwrap(), before, "{zone}");
    }
    crate::menus::invoke(&mut app, &ctx, "window.panel.dock", json!({"panel": "toolbar", "column": true, "side": "end"})).unwrap();
    assert!(matches!(toolbar_place(&app), Some(PanelPlace::Dock { .. })), "{:?}", toolbar_place(&app));
}

/// A workspace saved while the toolbar could still dock below the canvas loads with it in a dock column of its own.
#[test]
fn a_saved_toolbar_below_the_canvas_loads_in_a_dock_column() {
    let mut old = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    // Unrestricted permissions, as the release before this change let the toolbar dock there.
    let below = Action::move_column(TOOLBAR.to_owned(), Target::at(CANVAS.to_owned(), DockPlacement::Split(DockSide::Bottom)));
    std::sync::Arc::make_mut(&mut old.ui.docking).apply(below, false, |_: &String| Permissions::ALL).unwrap();
    assert!(matches!(toolbar_place(&old), Some(PanelPlace::Node(_))));
    let saved = crate::panel_docking::snapshot(&old);

    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    crate::panel_docking::load(&mut app, &saved);
    assert!(matches!(toolbar_place(&app), Some(PanelPlace::Dock { .. })), "{:?}", toolbar_place(&app));
    assert!(app.ui.docking.layout().contains(&CANVAS.to_owned()));
}
