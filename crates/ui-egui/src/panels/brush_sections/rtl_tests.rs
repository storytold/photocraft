//! Right-to-left twins of the Brush Settings section bodies: the Size row reads from the
//! right, the angle and roundness captions hug the reading-start edge of their 72 px block,
//! and the tip grid's first cell is the right-most. Direction only — the language stays
//! English, like the left-to-right twins.

use egui::accesskit::Role;
use egui::{Pos2, Rect};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable, by};

use super::*;

/// The section body under test: the working brush, the presets its grids show and the last
/// preset picked in a grid.
struct SectionState {
    brush: BrushSettings,
    presets: Vec<BrushPreset>,
    picked: Option<String>,
}

/// One section body in a harness. The direction scope must already be set when this runs:
/// the theme installs the egui direction flag with it.
fn section_harness(section: usize, state: SectionState) -> Harness<'static, SectionState> {
    let mut ready = false;
    let mut h = Harness::builder().with_size(vec2(340.0, 1100.0)).build_ui_state(
        move |ui, state: &mut SectionState| {
            crate::theme::set_direction(ui.ctx(), crate::direction::is_rtl());
            if !ready {
                crate::PhotocraftApp::setup_context(ui.ctx(), theme::ThemeKind::ProMedium);
                ready = true;
                return;
            }
            if let Some(name) = section_body(ui, &mut state.brush, section, &state.presets, None, Tool::Brush) {
                state.picked = Some(name);
            }
        },
        state,
    );
    h.run_steps(3);
    h
}

/// The caption label with this text, not the field it names.
fn caption_rect(h: &Harness<'_, SectionState>, text: &str) -> Rect {
    h.query_all(by().label(text).include_labels()).find(|n| n.accesskit_node().role() == Role::Label).unwrap_or_else(|| panic!("the `{text}` caption")).rect()
}

/// The value field with this accessible name, not the caption.
fn field_rect(h: &Harness<'_, SectionState>, text: &str) -> Rect {
    h.query_all(by().label(text).include_labels()).find(|n| n.accesskit_node().role() != Role::Label).unwrap_or_else(|| panic!("the `{text}` field")).rect()
}

/// The painted box of the number field: the 70×24 rounded rect the number sits in.
fn field_box(h: &Harness<'_, SectionState>, number: Rect) -> Rect {
    h.output()
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Rect(r) => Some(r.rect),
            _ => None,
        })
        .find(|r| (r.width() - 70.0).abs() < 1.0 && (r.height() - 24.0).abs() < 1.0 && r.contains_rect(number))
        .unwrap_or_else(|| panic!("the field's box around {number:?}"))
}

/// A sampled tip, so the Size row offers its "Restore original size" button.
fn sampled_brush() -> BrushSettings {
    BrushSettings { tip: paint::TipShape::Sampled(paint::GrayTile::from_fn(8, 8, |_, _| 1.0)), size: 77.0, ..Default::default() }
}

/// A preset whose tip size is distinct, so the grid's painted size numbers identify it.
fn preset(name: &str, size: f32) -> BrushPreset {
    let brush = BrushSettings { size, ..Default::default() };
    BrushPreset { name: name.to_string(), brush, builtin: true, group: String::new(), folder: Vec::new(), captures: Default::default() }
}

/// A press and a release on the same spot.
fn click_at(h: &mut Harness<'static, SectionState>, pos: Pos2) {
    h.hover_at(pos);
    h.run_steps(1);
    h.drag_at(pos);
    h.run_steps(1);
    h.drop_at(pos);
    h.run_steps(2);
}

/// The painted text of every shape, with the centre of each galley.
fn painted_text(h: &Harness<'_, SectionState>) -> Vec<(String, Pos2)> {
    fn walk(shape: &egui::Shape, out: &mut Vec<(String, Pos2)>) {
        match shape {
            egui::Shape::Text(t) => {
                let centre = t.pos + t.galley.rect.size() / 2.0;
                out.push((t.galley.job.text.clone(), centre));
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    walk(shape, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for shape in &h.output().shapes {
        walk(&shape.shape, &mut out);
    }
    out
}

/// The Size row reads from the right: the caption leads, the px field ends the row and the
/// restore button sits between them — the mirror of the left-to-right `[Size][restore][field]`.
#[test]
fn rtl_size_row_reads_from_the_right() {
    let _rtl = crate::direction::direction_scope(crate::direction::Dir::Rtl);
    let h = section_harness(0, SectionState { brush: sampled_brush(), presets: vec![], picked: None });
    let caption = caption_rect(&h, "Size");
    let field = field_rect(&h, "Size");
    assert!(caption.left() > field.right(), "the Size caption leads from the right: {caption:?}, {field:?}");
    let restore = h.get_by_role_and_label(Role::Button, "Restore original size").rect();
    assert!(
        restore.left() > field.right() && restore.right() < caption.left(),
        "the restore button sits between the field and the caption: {restore:?}, {field:?}, {caption:?}"
    );
}

/// The tip grid's cells flow from the right: the first preset is the right-most cell, and
/// clicking it picks that preset.
#[test]
fn rtl_tip_grid_starts_at_the_right() {
    let _rtl = crate::direction::direction_scope(crate::direction::Dir::Rtl);
    let presets = vec![preset("First", 11.0), preset("Second", 22.0), preset("Third", 33.0)];
    let mut h = section_harness(0, SectionState { brush: sampled_brush(), presets, picked: None });
    let texts = painted_text(&h);
    let number = |text: &str| texts.iter().find(|(s, _)| s == text).map(|(_, p)| *p).unwrap_or_else(|| panic!("the `{text}` size number"));
    let (first, second, third) = (number("11"), number("22"), number("33"));
    assert!(first.x > second.x && second.x > third.x, "the first preset leads from the right: {first:?}, {second:?}, {third:?}");
    click_at(&mut h, first);
    assert_eq!(h.state().picked.as_deref(), Some("First"), "the right-most cell picks the first preset");
}

/// The whole Brush Settings window in a harness; the direction scope must already be set.
fn window_harness(section: usize) -> Harness<'static, crate::PhotocraftApp> {
    let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.ui.panels.brush_settings = true;
    app.ui.brush_tab = 0;
    app.ui.brush_section = section;
    let mut h = Harness::builder().with_size(vec2(1200.0, 900.0)).build_ui_state(
        |ui, app: &mut crate::PhotocraftApp| {
            let ctx = ui.ctx().clone();
            // The real app re-publishes the egui direction flag every frame
            // (i18n::sync_context -> theme::set_direction); do the same here so the
            // twins exercise the flag-on path, not just the thread-local scope.
            crate::theme::set_direction(&ctx, crate::direction::is_rtl());
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::panels::brush_panel::window(app, &ctx);
        },
        app,
    );
    crate::PhotocraftApp::setup_context(&h.ctx, theme::ThemeKind::ALL[0]);
    // Many steps: the runaway height is a per-frame feedback loop, so a few steps never grow it.
    h.run_steps(60);
    h
}

/// The caption label with this text in any harness, not the field it names.
fn any_caption<T>(h: &Harness<'_, T>, text: &str) -> Rect {
    h.query_all(by().label(text).include_labels()).find(|n| n.accesskit_node().role() == Role::Label).unwrap_or_else(|| panic!("the `{text}` caption")).rect()
}

/// Every section's lock stays inside its own list row, in either direction.
fn locks_bind_to_their_rows(h: &Harness<'_, crate::PhotocraftApp>) {
    for (name, _) in crate::panels::brush_panel::SECTIONS.iter().skip(1) {
        let row = h.get_by_label(name).rect();
        let lock = h.get_by_role_and_label(Role::Button, &format!("Lock: {name}")).rect();
        assert!(row.contains(lock.center()), "{name}: the lock sits inside its row: {row:?} {lock:?}");
    }
}

/// Whole-window twins: the section list leads from the reading start (right in RTL, left in
/// LTR), the options pane follows, and the locks bind to their rows.
#[test]
fn window_columns_and_locks_follow_the_direction() {
    let rtl = crate::direction::direction_scope(crate::direction::Dir::Rtl);
    let h = window_harness(0);
    let sections = h.get_by_label("Brush Tip Shape").rect();
    let options = any_caption(&h, "Size");
    assert!(sections.left() > options.left(), "RTL: the sections lead from the right: {sections:?} {options:?}");
    locks_bind_to_their_rows(&h);
    drop(rtl);
    let h = window_harness(0);
    let sections = h.get_by_label("Brush Tip Shape").rect();
    let options = any_caption(&h, "Size");
    assert!(sections.left() < options.left(), "LTR: the sections stay on the left: {sections:?} {options:?}");
    locks_bind_to_their_rows(&h);
}

/// The angle and roundness captions hug the reading-start edge of their 72 px block: in a
/// right-to-left UI the caption ends at the block's right edge, the mirror of the
/// left-to-right row, so it sits right next to its field.
#[test]
fn rtl_angle_captions_hug_their_block() {
    let _rtl = crate::direction::direction_scope(crate::direction::Dir::Rtl);
    let h = section_harness(0, SectionState { brush: sampled_brush(), presets: vec![], picked: None });
    let spacing = h.ctx.style_of(egui::Theme::Dark).spacing.item_spacing.x;
    for text in ["Angle:", "Roundness:"] {
        let caption = caption_rect(&h, text);
        let field = field_rect(&h, text);
        // The block keeps its 72 px reserve and ends one item spacing before the field's box.
        let block_right = field_box(&h, field).left() - spacing;
        assert!((caption.right() - block_right).abs() < 1.5, "the {text} caption ends at its block's right edge: {caption:?}, block right {block_right}");
    }
}

/// Shape Dynamics with the size control on Fade, so its Control row shows the steps field.
fn fade_brush() -> BrushSettings {
    let mut b = BrushSettings::default();
    b.shape_dynamics.size = paint::Dynamic { jitter: 0.3, control: Control::Fade, fade_steps: 40, minimum: 0.2, ..Default::default() };
    b
}

/// The SpinButton showing this value.
fn spin(h: &Harness<'_, SectionState>, value: f64) -> Rect {
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().numeric_value() == Some(value)).unwrap_or_else(|| panic!("the {value} field")).rect()
}

/// The painted 74×24 box around a slider row's SpinButton.
fn spin_box(h: &Harness<'_, SectionState>, number: Rect) -> Rect {
    h.output()
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Rect(r) => Some(r.rect),
            _ => None,
        })
        .find(|r| (r.width() - 74.0).abs() < 1.0 && (r.height() - 24.0).abs() < 1.0 && r.contains_rect(number))
        .unwrap_or_else(|| panic!("the slider field's box around {number:?}"))
}

/// The Control dropdown buttons (not the dim captions that share the label).
fn control_dropdowns(h: &Harness<'_, SectionState>) -> Vec<Rect> {
    h.query_all(by().label("Control:").include_labels()).filter(|n| n.accesskit_node().role() != Role::Label).map(|n| n.rect()).collect()
}

/// RTL twin: the Control row's dropdown group pins to the same trailing edge as the slider
/// rows' value fields — one shared left edge for every control in the section body. The LTR
/// twin keeps the natural packing: captions lead, controls follow them.
#[test]
fn value_fields_share_one_trailing_edge_in_rtl() {
    let _rtl = crate::direction::direction_scope(crate::direction::Dir::Rtl);
    let h = section_harness(1, SectionState { brush: fade_brush(), presets: vec![], picked: None });
    let slider_field = spin(&h, 30.0);
    let slider_box = spin_box(&h, slider_field);
    let steps_field = spin(&h, 40.0);
    let dropdowns = control_dropdowns(&h);
    assert!(!dropdowns.is_empty(), "the Control dropdowns render");
    for d in &dropdowns {
        assert!((d.left() - slider_box.left()).abs() < 0.5, "the Control dropdown pins to the slider field's edge: {d:?} {slider_box:?}");
    }
    assert!(steps_field.left() > slider_box.left(), "the steps field follows its dropdown inside the pinned group: {steps_field:?} {slider_box:?}");

    drop(_rtl);
    let h = section_harness(1, SectionState { brush: fade_brush(), presets: vec![], picked: None });
    let slider_field = spin(&h, 30.0);
    let caption = caption_rect(&h, "Size Jitter");
    assert!(slider_field.left() > caption.left(), "LTR keeps the slider field right of its caption: {slider_field:?} {caption:?}");
    let dropdowns = control_dropdowns(&h);
    for d in &dropdowns {
        assert!(d.left() > caption_rect(&h, "Control:").left(), "LTR keeps the dropdown after its caption: {d:?}");
    }
}

/// The tab button with this label (the window title shares the text but is not a button).
fn tab_rect(h: &Harness<'_, crate::PhotocraftApp>, label: &str) -> Rect {
    h.query_all(by().label(label).include_labels()).find(|n| n.accesskit_node().role() == Role::Button).unwrap_or_else(|| panic!("the `{label}` tab")).rect()
}

/// Window chrome twins: the tabs lead from the reading start and the close button hugs the
/// reading end, in both directions.
#[test]
fn window_chrome_follows_the_direction() {
    let rtl = crate::direction::direction_scope(crate::direction::Dir::Rtl);
    let h = window_harness(0);
    let settings = tab_rect(&h, "Brush Settings");
    let brushes = tab_rect(&h, "Brushes");
    let close = h.get_by_role_and_label(Role::Button, "Close").rect();
    assert!(settings.left() > brushes.left(), "RTL: the Brush Settings tab leads from the right: {settings:?} {brushes:?}");
    assert!(close.left() < brushes.left(), "RTL: the close button hugs the left (reading-end) edge: {close:?} {brushes:?}");
    drop(rtl);
    let h = window_harness(0);
    let settings = tab_rect(&h, "Brush Settings");
    let brushes = tab_rect(&h, "Brushes");
    let close = h.get_by_role_and_label(Role::Button, "Close").rect();
    assert!(settings.left() < brushes.left(), "LTR: the tabs lead from the left: {settings:?} {brushes:?}");
    assert!(close.right() > settings.right(), "LTR: the close button hugs the right edge: {close:?} {settings:?}");
}

/// The floating window's height stays bounded by its content in both directions. A row scoped
/// with `with_layout` inside the vertical window body gets the body's full remaining height,
/// and a center-aligned row fills that height, so the non-resizable window grows toward the
/// screen height and the tab row (the window's visible title) ends up vertically centered.
#[test]
fn window_height_is_bounded_and_the_tab_row_sits_at_the_top() {
    for dir in [crate::direction::Dir::Ltr, crate::direction::Dir::Rtl] {
        let _scope = crate::direction::direction_scope(dir);
        let h = window_harness(0);
        // The twin must run the real app path: the egui direction flag set, not just the
        // thread-local scope (the flag is what makes nested rows mirror and fill).
        assert_eq!(h.ctx.global_style().right_to_left, dir == crate::direction::Dir::Rtl, "{dir:?}: the egui direction flag follows the scope");
        let win = h.ctx.memory(|m| m.area_rect(egui::Id::unique("brush-settings"))).expect("the brush window is shown");
        let tab = tab_rect(&h, "Brush Settings");
        assert!(win.height() <= 700.0, "{dir:?}: the window height is bounded by its content: {win:?}");
        assert!(win.width() <= 700.0, "{dir:?}: the window width is bounded by its content: {win:?}");
        assert!(tab.top() - win.top() < 60.0, "{dir:?}: the tab row (the window title) sits at the top: win {win:?}, tab {tab:?}");
        assert!(tab.bottom() < win.center().y, "{dir:?}: the tab row is not vertically centered in the window: win {win:?}, tab {tab:?}");
    }
}
