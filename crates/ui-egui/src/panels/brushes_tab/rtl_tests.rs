//! Right-to-left twins of the Brushes tab's rows: the rows read from the right, the view toggles
//! and the footer buttons hug the reading end, the grid's columns accumulate from the right, and
//! a drop on a grid cell's left half lands after it. Direction only — the language (and so every
//! label) stays English, like the LTR twins.

use egui::{Rect, accesskit::Role, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

use super::*;

/// The docked Brushes tab drawn in a right-to-left UI (`direction_scope` holds for the harness,
/// which runs on this thread). Only the brush panel is drawn, so every query below sees the tab's
/// own widgets.
fn rtl_tab(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let h = Harness::builder().with_size(vec2(1200.0, 900.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::brush_panel::window(app, &ctx);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h
}

/// The Brushes tab of the docked brush settings panel, showing every group.
fn brushes_tab_app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.ui.panels.brush_settings = true;
    app.ui.brush_tab = 1;
    app.ui.brushes_panel.collapsed.clear();
    app
}

/// The tab's Size row: the label leads from the right and the value field follows it. The
/// slider between them paints itself and exposes no accesskit node, so the row's two ends carry
/// the check.
#[test]
fn rtl_size_row_reads_from_the_right() {
    let _rtl = crate::direction::direction_scope(crate::direction::Dir::Rtl);
    let mut h = rtl_tab(brushes_tab_app());
    h.run_steps(4);
    let label = h.query_all_by_label("Size").find(|n| n.accesskit_node().role() == Role::Label).expect("the Size label").rect();
    let field = h.query_all_by_label("Size").find(|n| n.accesskit_node().role() != Role::Label).expect("the Size value field").rect();
    assert!(label.left() > field.right(), "the Size label leads from the right: {label:?}, field {field:?}");
}

/// The search row's Grid/List view toggles hug the reading end: both sit left of the search
/// field, with the Grid toggle (added first) outermost — the mirrored `trailing()` group.
#[test]
fn rtl_search_view_toggles_hug_the_reading_end() {
    let _rtl = crate::direction::direction_scope(crate::direction::Dir::Rtl);
    let mut h = rtl_tab(brushes_tab_app());
    h.run_steps(4);
    let grid = h.get_by_role_and_label(Role::Button, "Grid view").rect();
    let list = h.get_by_role_and_label(Role::Button, "List view").rect();
    let field = h.query_all_by_role(Role::TextInput).map(|n| n.rect()).find(|r| (r.center().y - grid.center().y).abs() < 10.0).expect("the search field");
    assert!(list.right() <= field.left() + 1.0, "the toggles sit left of the field: {list:?}, field {field:?}");
    assert!(grid.left() < list.left(), "the Grid toggle is outermost: {grid:?}, {list:?}");
}

/// The footer's Delete/New/Import buttons hug the reading end: the trash (added first) is
/// outermost, and the whole group sits left of the preset-count label at the reading start.
#[test]
fn rtl_footer_buttons_hug_the_reading_end() {
    let _rtl = crate::direction::direction_scope(crate::direction::Dir::Rtl);
    let mut h = rtl_tab(brushes_tab_app());
    h.run_steps(4);
    let trash = h.get_by_role_and_label(Role::Button, "Delete brush").rect();
    let new = h.get_by_role_and_label(Role::Button, "Create new brush from the current settings").rect();
    let import = h.get_by_role_and_label(Role::Button, "Import Brushes… (.abr or image)").rect();
    assert!(trash.left() < new.left() && new.left() < import.left(), "first-added outermost: {trash:?}, {new:?}, {import:?}");
    let count_text = format!("{} presets", h.state().session.tools.presets.len());
    let count = h.query_all_by_label(&count_text).map(|n| n.rect()).find(|r| r.center().y > trash.center().y - 4.0).expect("the preset-count label");
    assert!(import.right() < count.left(), "the buttons end left of the count: {import:?}, {count:?}");
}

/// The grid's columns accumulate from the right: in the first row the first preset sits at the
/// reading start (its right edge is the row's outermost), mirrored from the LTR tab.
#[test]
fn rtl_grid_cells_accumulate_from_the_right() {
    let _rtl = crate::direction::direction_scope(crate::direction::Dir::Rtl);
    let mut app = brushes_tab_app();
    app.ui.brushes_panel.view = BrushesView::Grid;
    let mut h = rtl_tab(app);
    h.run_steps(4);
    let presets = h.state().session.tools.presets.clone();
    let cells = |h: &Harness<'_, PhotocraftApp>| -> Vec<Rect> {
        h.query_all_by_role(Role::Button)
            .filter(|n| n.accesskit_node().label().is_some_and(|l| presets.iter().any(|p| p.name == l)))
            .map(|n| n.rect())
            .collect()
    };
    let all = cells(&h);
    assert!(all.len() >= 2, "{} grid cells", all.len());
    let top = all.iter().map(|r| r.top()).fold(f32::MAX, f32::min);
    let first_row: Vec<Rect> = all.iter().copied().filter(|r| (r.top() - top).abs() < 0.5).collect();
    assert!(first_row.len() >= 2, "one row of cells: {first_row:?}");
    let rightmost = first_row.iter().map(|r| r.right()).fold(f32::MIN, f32::max);
    let first = h.get_by_role_and_label(Role::Button, &presets[0].name).rect();
    assert!((first.right() - rightmost).abs() < 0.5, "the first preset leads from the right: {first:?} in {first_row:?}");
}

/// The group header still toggles its group when clicked in its mirrored place: the collapsed
/// set keys on the raw group label, which stays "General" under an English UI.
#[test]
fn rtl_group_header_click_still_toggles() {
    let _rtl = crate::direction::direction_scope(crate::direction::Dir::Rtl);
    let mut h = rtl_tab(brushes_tab_app());
    h.run_steps(4);
    assert!(h.state().ui.brushes_panel.collapsed.is_empty(), "every group starts open");
    h.get_by_role_and_label(Role::DisclosureTriangle, "General").click();
    h.run_steps(2);
    assert!(h.state().ui.brushes_panel.collapsed.iter().any(|k| k == "General"), "the click collapsed the group");
    h.get_by_role_and_label(Role::DisclosureTriangle, "General").click();
    h.run_steps(2);
    assert!(h.state().ui.brushes_panel.collapsed.is_empty(), "the second click reopens it");
}

/// The rename bar reads from the right: the name label leads, the text field follows.
#[test]
fn rtl_rename_bar_flips() {
    let _rtl = crate::direction::direction_scope(crate::direction::Dir::Rtl);
    let mut app = brushes_tab_app();
    let name = app.session.tools.presets[0].name.clone();
    app.ui.brushes_panel.renaming = Some(Renaming { group: false, name: name.clone(), folder: Vec::new(), text: String::new() });
    let mut h = rtl_tab(app);
    h.run_steps(4);
    // The field carries the caption's text as its accessible label; the caption itself is a
    // plain label node without one, so it is found by its row next to the field.
    let field = h.query_all_by_label("Brush name").map(|n| n.rect()).next().expect("the name field");
    let label = h.query_all_by_role(Role::Label).map(|n| n.rect()).find(|r| (r.center().y - field.center().y).abs() < 6.0).expect("the name label");
    assert!(label.left() > field.right(), "the label leads from the right: {label:?}, field {field:?}");
}

/// A grid's columns run right to left, so a drop on a cell's left half lands after it — the
/// mirrored twin of the LTR tab's right half.
#[test]
fn rtl_grid_drop_on_the_left_half_lands_after() {
    let _rtl = crate::direction::direction_scope(crate::direction::Dir::Rtl);
    let mut app = brushes_tab_app();
    app.ui.brushes_panel.view = BrushesView::Grid;
    let (dragged, target) = {
        let p = &app.session.tools.presets;
        (p[0].name.clone(), p[1].name.clone())
    };
    let mut h = rtl_tab(app);
    h.run_steps(4);
    let from = h.get_by_role_and_label(Role::Button, &dragged).rect();
    let to = h.get_by_role_and_label(Role::Button, &target).rect();
    h.hover_at(from.center());
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: from.center(), button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.run_steps(1);
    h.event(egui::Event::PointerMoved(from.center() + vec2(0.0, 12.0)));
    h.run_steps(4);
    assert!(egui::DragAndDrop::has_payload_of_type::<BrushDrag>(&h.ctx), "the preset is being dragged");
    // The target's left half: in a mirrored grid that is the far half, so "after" it.
    let land = egui::pos2(to.left() + to.width() * 0.25, to.center().y);
    h.event(egui::Event::PointerMoved(land));
    h.run_steps(2);
    h.event(egui::Event::PointerButton { pos: land, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(4);
    let names: Vec<String> = h.state().session.tools.presets.iter().map(|p| p.name.clone()).collect();
    let d = names.iter().position(|n| *n == dragged).expect("the dragged preset");
    let t = names.iter().position(|n| *n == target).expect("the target preset");
    assert!(d > t, "the dragged preset landed after the target: {names:?}");
}

/// Paint a name in the given span and report the painted rect and the number of text shapes the
/// frame painted: `None` means the span was dropped (the direction-aware label helper refuses a
/// span whose anchor sits inside its limit). The direction scope of the caller is in force while
/// the harness runs.
fn painted_label(x: f32, right: f32) -> Option<(Rect, usize)> {
    let out = std::cell::Cell::new(None);
    let mut h = egui_kittest::Harness::new_ui(|ui| {
        out.set(crate::direction::paint_label_span(ui.painter(), x, 10.0, right, "Soft Round", egui::FontId::proportional(12.0), egui::Color32::WHITE));
    });
    h.run_steps(2);
    let rect = out.get()?;
    let texts = h.output().shapes.iter().filter(|s| matches!(s.shape, egui::Shape::Text(_))).count();
    Some((rect, texts))
}

/// A list row leads with its tip from the right in a right-to-left UI, and its name is painted
/// in the span past the stroke preview (the anchor is the span's right end there).
#[test]
fn rtl_list_row_tip_side_and_name() {
    let _rtl = crate::direction::direction_scope(crate::direction::Dir::Rtl);
    let row = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(300.0, 44.0));
    let tip = list_row_tip_rect(true, row, 1.0);
    assert!(tip.center().x > row.center().x, "the tip leads from the right: {tip:?} in {row:?}");
    let ltr_tip = list_row_tip_rect(false, row, 1.0);
    assert!(ltr_tip.center().x < row.center().x, "the tip leads from the left in LTR: {ltr_tip:?}");
    // The stroke preview follows the tip toward the reading end; the name follows it.
    let w = 120.0;
    let sr = egui::Rect::from_min_size(egui::pos2(tip.left() - 8.0 - w, row.top() + 4.0), egui::vec2(w, 36.0));
    let (x, right) = list_row_name_span(true, row, sr);
    assert!(x > right, "the name's anchor is the span's right end: {x} > {right}");
    assert!(x <= sr.left(), "the name sits past the stroke preview: {x} <= {}", sr.left());
    let painted = painted_label(x, right);
    let (rect, texts) = painted.expect("the name is painted in RTL");
    assert!(texts >= 1, "the frame painted the name's galley: {texts}");
    assert!(rect.right() <= x + 0.5, "the name is right-anchored in the span: {rect:?}");
}

/// The group header mirrors: the name is anchored at the reading start (its right end), the
/// count sits at the row's left end, and the closed disclosure points left.
#[test]
fn rtl_group_header_mirror() {
    let _rtl = crate::direction::direction_scope(crate::direction::Dir::Rtl);
    let row = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(300.0, 24.0));
    let a = group_header_anchors(row, 0);
    assert!(a.name_x > row.center().x, "the name leads from the right: {a:?}");
    assert!((a.count_x - (row.left() + 8.0)).abs() < 0.5, "the count sits at the row's left end: {a:?}");
    assert!(a.chevron_x > a.name_x, "the chevron leads the name: {a:?}");
    assert_eq!(crate::icons::icon_name("chevron-right"), "chevron-left", "the closed disclosure points left");
    let ltr = {
        let _ltr = crate::direction::direction_scope(crate::direction::Dir::Ltr);
        group_header_anchors(row, 0)
    };
    assert!(ltr.name_x < row.center().x, "the name leads from the left in LTR: {ltr:?}");
}

/// A preset card leads with its tip from the right in a right-to-left UI, and both of its
/// names are painted (their span's anchor is its right end).
#[test]
fn rtl_preset_card_tip_side() {
    let _rtl = crate::direction::direction_scope(crate::direction::Dir::Rtl);
    let top = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(200.0, 60.0));
    let cell = card_tip_cell(true, top, 60.0);
    assert!((cell.right() - top.right()).abs() < 0.5, "the tip cell hugs the card's right edge: {cell:?}");
    assert!(cell.left() > top.center().x, "the tip leads from the right: {cell:?}");
    let ltr_cell = card_tip_cell(false, top, 60.0);
    assert!(ltr_cell.left() < top.center().x, "the tip leads from the left in LTR: {ltr_cell:?}");
    let stroke_cell = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(cell.left(), 60.0));
    let (x, right) = card_stroke_name_span(true, stroke_cell, 1.0);
    assert!(x > right, "the stroke-cell name's anchor is the span's right end: {x} > {right}");
    let (_, texts) = painted_label(x, right).expect("the stroke-cell name is painted in RTL");
    assert!(texts >= 1, "the frame painted the stroke-cell name's galley: {texts}");
    let bottom = Rect::from_min_max(egui::pos2(0.0, 60.0), egui::pos2(200.0, 84.0));
    let (bx, bright) = card_bottom_name_span(true, bottom, 1.0);
    assert!(bx > bright, "the bottom name's anchor is the span's right end: {bx} > {bright}");
    let (_, texts) = painted_label(bx, bright).expect("the bottom name is painted in RTL");
    assert!(texts >= 1, "the frame painted the bottom name's galley: {texts}");
}
